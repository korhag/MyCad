//! TrueType and OpenType outlines for TEXT and MTEXT.
//!
//! SHX styles and fonts that are not installed keep the Hershey stroke
//! font. File reads happen only while warming a document, which the
//! import worker does before the drawing reaches the UI thread.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use ttf_parser::{Face, GlyphId, OutlineBuilder};

use crate::document::{Document, TextStyle};
use crate::entity::{Entity, Geometry};
use crate::geom::Point2;
use crate::stroke_font::{self, mtext_paragraphs};

// ------------------------------------------------------------
// Enum: FontAvailability
// Purpose: How a style font will be drawn after warming.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontAvailability {
    Stroke,
    TrueType,
    Missing,
}

// ------------------------------------------------------------
// Enum: TextFont
// Purpose: The face used to measure and draw one run.
// ------------------------------------------------------------
#[derive(Clone)]
pub enum TextFont {
    Stroke,
    TrueType(std::sync::Arc<TrueTypeFont>),
}

pub struct TrueTypeFont {
    id: u64,
    bytes: std::sync::Arc<Vec<u8>>,
    face_index: u32,
    cap_units: f32,
}

#[derive(Clone)]
pub struct TextDrawing {
    pub strokes: Vec<[Point2; 2]>,
    pub glyphs: Vec<Vec<Vec<Point2>>>,
}

struct FaceRecord {
    path: PathBuf,
    file_key: String,
    collection_index: u32,
    families: Vec<String>,
    bold: bool,
    italic: bool,
}

struct Library {
    index: Vec<FaceRecord>,
    scanned: bool,
    loaded: HashMap<u64, std::sync::Arc<TrueTypeFont>>,
    by_file: HashMap<String, u64>,
    by_family: HashMap<(String, bool, bool), u64>,
    missing: HashSet<String>,
    next_id: u64,
}

struct CachedGlyph {
    advance: f32,
    contours: Vec<Vec<(f32, f32)>>,
}

enum FontRequest {
    File(String),
    Family {
        name: String,
        bold: bool,
        italic: bool,
    },
}

static LIBRARY: LazyLock<Mutex<Library>> = LazyLock::new(|| {
    Mutex::new(Library {
        index: Vec::new(),
        scanned: false,
        loaded: HashMap::new(),
        by_file: HashMap::new(),
        by_family: HashMap::new(),
        missing: HashSet::new(),
        next_id: 1,
    })
});

static GLYPHS: LazyLock<Mutex<HashMap<(u64, u16), CachedGlyph>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl TextFont {
    // --------------------------------------------------------
    // Method: measure
    // Purpose: Width of one line. `expand_codes` turns TEXT
    //          %% codes into glyphs. MTEXT is already decoded.
    // --------------------------------------------------------
    pub fn measure(&self, text: &str, height: f64, width_factor: f64, expand_codes: bool) -> f64 {
        match self {
            Self::Stroke => stroke_font::measure_styled_width(text, height, width_factor.max(0.0)),
            Self::TrueType(font) => font
                .measure(text, height, width_factor, expand_codes)
                .unwrap_or_else(|| {
                    stroke_font::measure_styled_width(text, height, width_factor.max(0.0))
                }),
        }
    }

    // --------------------------------------------------------
    // Method: draw
    // Purpose: Strokes or filled glyph contours in world space.
    //          A bad font falls back to the stroke font.
    // --------------------------------------------------------
    pub fn draw(
        &self,
        origin: Point2,
        height: f64,
        rotation: f64,
        width_factor: f64,
        oblique: f64,
        text: &str,
        expand_codes: bool,
    ) -> TextDrawing {
        match self {
            Self::Stroke => stroke_drawing(origin, height, rotation, width_factor, oblique, text),
            Self::TrueType(font) => font
                .draw(
                    origin,
                    height,
                    rotation,
                    width_factor,
                    oblique,
                    text,
                    expand_codes,
                )
                .unwrap_or_else(|| {
                    stroke_drawing(origin, height, rotation, width_factor, oblique, text)
                }),
        }
    }

    pub(crate) fn cached_file(font_file: &str) -> Self {
        let Some(lib) = lock_library() else {
            return Self::Stroke;
        };
        if let Some(font) = loaded_file(&lib, font_file) {
            return Self::TrueType(font);
        }
        if let Some(font) = loaded_family(&lib, font_file, false, false) {
            return Self::TrueType(font);
        }
        Self::Stroke
    }

    pub(crate) fn cached_family(family: &str, bold: bool, italic: bool) -> Self {
        let Some(lib) = lock_library() else {
            return Self::Stroke;
        };
        loaded_family(&lib, family, bold, italic)
            .map(Self::TrueType)
            .unwrap_or(Self::Stroke)
    }
}

fn stroke_drawing(
    origin: Point2,
    height: f64,
    rotation: f64,
    width_factor: f64,
    oblique: f64,
    text: &str,
) -> TextDrawing {
    TextDrawing {
        strokes: stroke_font::stroke_text_styled(
            origin,
            height,
            rotation,
            width_factor,
            oblique,
            text,
        ),
        glyphs: Vec::new(),
    }
}

impl TrueTypeFont {
    fn measure(
        &self,
        text: &str,
        height: f64,
        width_factor: f64,
        expand_codes: bool,
    ) -> Option<f64> {
        let text = prepared(text, expand_codes);
        let scale = world_scale(height, self.cap_units)?;
        let factor = width_factor.max(0.0);
        let face = Face::parse(&self.bytes, self.face_index).ok()?;
        let mut glyphs = lock_glyphs()?;
        let mut width = 0.0_f64;
        let mut line = 0.0_f64;
        for ch in text.chars() {
            if ch == '\n' {
                width = width.max(line);
                line = 0.0;
                continue;
            }
            let advance = glyph_advance(&mut glyphs, &face, self.id, self.cap_units, ch);
            line += f64::from(advance) * scale * factor;
        }
        Some(width.max(line))
    }

    fn draw(
        &self,
        origin: Point2,
        height: f64,
        rotation: f64,
        width_factor: f64,
        oblique: f64,
        text: &str,
        expand_codes: bool,
    ) -> Option<TextDrawing> {
        let text = prepared(text, expand_codes);
        let scale = world_scale(height, self.cap_units)?;
        let factor = width_factor.max(0.0);
        let shear = finite_shear(oblique);
        let (sin, cos) = rotation.sin_cos();
        let face = Face::parse(&self.bytes, self.face_index).ok()?;
        let mut glyphs = lock_glyphs()?;
        let mut drawn = TextDrawing {
            strokes: Vec::new(),
            glyphs: Vec::new(),
        };
        let mut cursor = 0.0_f64;
        for ch in text.chars() {
            if ch == '\n' {
                cursor = 0.0;
                continue;
            }
            let (advance_units, contours) =
                glyph_contours(&mut glyphs, &face, self.id, self.cap_units, ch);
            let advance = f64::from(advance_units) * scale * factor;
            if !contours.is_empty() {
                let mut placed = Vec::with_capacity(contours.len());
                for contour in &contours {
                    let mut world = Vec::with_capacity(contour.len());
                    for &(x, y) in contour {
                        let local_x =
                            cursor + (f64::from(x) + f64::from(y) * shear) * scale * factor;
                        let local_y = f64::from(y) * scale;
                        world.push(Point2::new(
                            origin.x + local_x * cos - local_y * sin,
                            origin.y + local_x * sin + local_y * cos,
                        ));
                    }
                    if world.len() >= 3 {
                        placed.push(world);
                    }
                }
                if !placed.is_empty() {
                    drawn.glyphs.push(placed);
                }
            }
            cursor += advance;
        }
        Some(drawn)
    }
}

fn prepared(text: &str, expand_codes: bool) -> String {
    if expand_codes {
        stroke_font::expand_cad_codes(text)
    } else {
        text.to_string()
    }
}

fn world_scale(height: f64, cap_units: f32) -> Option<f64> {
    let height = if height.is_finite() && height > 1e-9 {
        height
    } else {
        return None;
    };
    let cap = f64::from(cap_units);
    if cap > 1e-3 {
        Some(height / cap)
    } else {
        None
    }
}

fn finite_shear(oblique: f64) -> f64 {
    let shear = oblique.tan();
    if shear.is_finite() {
        shear
    } else {
        0.0
    }
}

fn glyph_advance(
    glyphs: &mut HashMap<(u64, u16), CachedGlyph>,
    face: &Face<'_>,
    font_id: u64,
    cap_units: f32,
    ch: char,
) -> f32 {
    glyph_entry(glyphs, face, font_id, cap_units, ch).advance
}

fn glyph_contours(
    glyphs: &mut HashMap<(u64, u16), CachedGlyph>,
    face: &Face<'_>,
    font_id: u64,
    cap_units: f32,
    ch: char,
) -> (f32, Vec<Vec<(f32, f32)>>) {
    let glyph = glyph_entry(glyphs, face, font_id, cap_units, ch);
    (glyph.advance, glyph.contours.clone())
}

fn glyph_entry<'a>(
    glyphs: &'a mut HashMap<(u64, u16), CachedGlyph>,
    face: &Face<'_>,
    font_id: u64,
    cap_units: f32,
    ch: char,
) -> &'a CachedGlyph {
    let glyph_id = face.glyph_index(ch).unwrap_or(GlyphId(0));
    glyphs
        .entry((font_id, glyph_id.0))
        .or_insert_with(|| flatten_glyph(face, glyph_id, cap_units))
}

fn flatten_glyph(face: &Face<'_>, glyph_id: GlyphId, cap_units: f32) -> CachedGlyph {
    let advance = face
        .glyph_hor_advance(glyph_id)
        .map(f32::from)
        .filter(|advance| *advance >= 0.0)
        .unwrap_or(cap_units * 0.5);
    let tolerance = (f32::from(face.units_per_em()) / 256.0).max(1.0);
    let mut builder = GlyphFlattener {
        contours: Vec::new(),
        current: Vec::new(),
        tolerance2: tolerance * tolerance,
    };
    let _ = face.outline_glyph(glyph_id, &mut builder);
    builder.finish_contour();
    CachedGlyph {
        advance,
        contours: builder.contours,
    }
}

struct GlyphFlattener {
    contours: Vec<Vec<(f32, f32)>>,
    current: Vec<(f32, f32)>,
    tolerance2: f32,
}

impl GlyphFlattener {
    fn finish_contour(&mut self) {
        if self.current.len() >= 2 {
            let first = self.current[0];
            let last = *self.current.last().unwrap_or(&first);
            if (first.0 - last.0).abs() < 0.05 && (first.1 - last.1).abs() < 0.05 {
                self.current.pop();
            }
        }
        if self.current.len() >= 3 {
            self.contours.push(std::mem::take(&mut self.current));
        } else {
            self.current.clear();
        }
    }
}

impl OutlineBuilder for GlyphFlattener {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish_contour();
        self.current.push((x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.current.push((x, y));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let start = *self.current.last().unwrap_or(&(x, y));
        flatten_quad(
            start,
            (x1, y1),
            (x, y),
            self.tolerance2,
            &mut self.current,
            0,
        );
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let start = *self.current.last().unwrap_or(&(x, y));
        flatten_cubic(
            start,
            (x1, y1),
            (x2, y2),
            (x, y),
            self.tolerance2,
            &mut self.current,
            0,
        );
    }

    fn close(&mut self) {
        self.finish_contour();
    }
}

fn flatten_quad(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    tolerance2: f32,
    out: &mut Vec<(f32, f32)>,
    depth: u8,
) {
    if depth >= 10 || point_line_distance2(p1, p0, p2) <= tolerance2 {
        out.push(p2);
        return;
    }
    let p01 = midpoint(p0, p1);
    let p12 = midpoint(p1, p2);
    let mid = midpoint(p01, p12);
    flatten_quad(p0, p01, mid, tolerance2, out, depth + 1);
    flatten_quad(mid, p12, p2, tolerance2, out, depth + 1);
}

fn flatten_cubic(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    tolerance2: f32,
    out: &mut Vec<(f32, f32)>,
    depth: u8,
) {
    let far = point_line_distance2(p1, p0, p3).max(point_line_distance2(p2, p0, p3));
    if depth >= 10 || far <= tolerance2 {
        out.push(p3);
        return;
    }
    let p01 = midpoint(p0, p1);
    let p12 = midpoint(p1, p2);
    let p23 = midpoint(p2, p3);
    let p012 = midpoint(p01, p12);
    let p123 = midpoint(p12, p23);
    let mid = midpoint(p012, p123);
    flatten_cubic(p0, p01, p012, mid, tolerance2, out, depth + 1);
    flatten_cubic(mid, p123, p23, p3, tolerance2, out, depth + 1);
}

fn midpoint(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5)
}

fn point_line_distance2(point: (f32, f32), start: (f32, f32), end: (f32, f32)) -> f32 {
    let dx = end.0 - start.0;
    let dy = end.1 - start.1;
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-8 {
        let ex = point.0 - start.0;
        let ey = point.1 - start.1;
        return ex * ex + ey * ey;
    }
    let t = ((point.0 - start.0) * dx + (point.1 - start.1) * dy) / len2;
    let px = start.0 + t * dx;
    let py = start.1 + t * dy;
    let ex = point.0 - px;
    let ey = point.1 - py;
    ex * ex + ey * ey
}

fn lock_library() -> Option<std::sync::MutexGuard<'static, Library>> {
    LIBRARY.lock().ok()
}

fn lock_glyphs() -> Option<std::sync::MutexGuard<'static, HashMap<(u64, u16), CachedGlyph>>> {
    GLYPHS.lock().ok()
}

fn loaded_file(lib: &Library, font_file: &str) -> Option<std::sync::Arc<TrueTypeFont>> {
    let key = file_key(font_file);
    let id = lib.by_file.get(&key)?;
    lib.loaded.get(id).cloned()
}

fn loaded_family(
    lib: &Library,
    family: &str,
    bold: bool,
    italic: bool,
) -> Option<std::sync::Arc<TrueTypeFont>> {
    let name = family_key(family);
    if name.is_empty() {
        return None;
    }
    if let Some(id) = lib.by_family.get(&(name.clone(), bold, italic)) {
        return lib.loaded.get(id).cloned();
    }
    let mut fallback: Option<(u64, i32)> = None;
    for ((stored, stored_bold, stored_italic), id) in &lib.by_family {
        if stored != &name {
            continue;
        }
        let rank = i32::from(*stored_bold == bold) + i32::from(*stored_italic == italic);
        let better = fallback.map(|(_, best)| rank > best).unwrap_or(true);
        if better {
            fallback = Some((*id, rank));
        }
    }
    fallback.and_then(|(id, _)| lib.loaded.get(&id).cloned())
}

// ------------------------------------------------------------
// Function: warm_document_fonts
// Purpose: Load the faces this drawing can draw, on the caller
//          thread. Missing TrueType faces are noted once.
// ------------------------------------------------------------
pub fn warm_document_fonts(document: &mut Document) {
    let Some(notes) = warm_requests(&collect_requests(document)) else {
        return;
    };
    for message in notes {
        if !document
            .diagnostics
            .warnings
            .iter()
            .any(|warning| warning == &message)
        {
            document.diagnostics.warnings.push(message);
        }
    }
}

// ------------------------------------------------------------
// Function: describe_text_style_font
// Purpose: One `--import-only` line for a text style.
// ------------------------------------------------------------
pub fn describe_text_style_font(style: &TextStyle) -> String {
    let status = match font_availability(&style.font_file) {
        FontAvailability::Stroke => "stroke",
        FontAvailability::TrueType => "truetype",
        FontAvailability::Missing => "missing",
    };
    format!(
        "text_style {} font {} {status}",
        style.name, style.font_file
    )
}

#[cfg(test)]
fn warm_font_file(font_file: &str) -> bool {
    let _ = warm_requests(&[FontRequest::File(font_file.to_string())]);
    matches!(TextFont::cached_file(font_file), TextFont::TrueType(_))
}

fn font_availability(font_file: &str) -> FontAvailability {
    let Some(lib) = lock_library() else {
        return FontAvailability::Stroke;
    };
    if loaded_file(&lib, font_file).is_some()
        || loaded_family(&lib, font_file, false, false).is_some()
    {
        return FontAvailability::TrueType;
    }
    if lib.missing.contains(&request_key(font_file)) {
        return FontAvailability::Missing;
    }
    FontAvailability::Stroke
}

fn collect_requests(document: &Document) -> Vec<FontRequest> {
    let mut requests = Vec::new();
    for style in document.text_styles.values() {
        if !style.font_file.trim().is_empty() {
            requests.push(FontRequest::File(style.font_file.clone()));
        }
    }
    each_entity(document, |entity| {
        if let Geometry::MText(text) = &entity.geometry {
            for paragraph in mtext_paragraphs(&text.value, text.height) {
                for run in paragraph.runs {
                    if !run.font.trim().is_empty() {
                        requests.push(FontRequest::Family {
                            name: run.font,
                            bold: run.bold,
                            italic: run.italic,
                        });
                    }
                }
            }
        }
    });
    requests
}

fn each_entity(document: &Document, mut visit: impl FnMut(&Entity)) {
    for entity in &document.model_space {
        visit(entity);
    }
    for block in document.blocks.values() {
        for entity in &block.entities {
            visit(entity);
        }
    }
}

fn warm_requests(requests: &[FontRequest]) -> Option<Vec<String>> {
    ensure_index()?;
    let mut notes = Vec::new();
    let mut lib = lock_library()?;
    for request in requests {
        if let Some(message) = load_request(&mut lib, request) {
            notes.push(message);
        }
    }
    Some(notes)
}

fn load_request(lib: &mut Library, request: &FontRequest) -> Option<String> {
    let (label, from_inline, bold, italic) = match request {
        FontRequest::File(name) => (name.as_str(), false, false, false),
        FontRequest::Family { name, bold, italic } => (name.as_str(), true, *bold, *italic),
    };
    let key = request_key(label);
    if key.is_empty() || lib.missing.contains(&key) {
        return None;
    }
    if request_is_loaded(lib, label, from_inline, bold, italic) {
        return None;
    }
    match choose_face(&lib.index, label, bold, italic) {
        FaceChoice::Stroke => None,
        FaceChoice::Missing => {
            lib.missing.insert(key);
            Some(format!("text font '{label}' not found; using stroke font"))
        }
        FaceChoice::Load(index) => {
            if load_face(lib, index).is_none() {
                lib.missing.insert(key);
                Some(format!("text font '{label}' not found; using stroke font"))
            } else {
                None
            }
        }
    }
}

fn request_is_loaded(
    lib: &Library,
    label: &str,
    from_inline: bool,
    bold: bool,
    italic: bool,
) -> bool {
    if from_inline {
        exact_family_loaded(lib, label, bold, italic)
    } else {
        loaded_file(lib, label).is_some() || exact_family_loaded(lib, label, bold, italic)
    }
}

fn exact_family_loaded(lib: &Library, family: &str, bold: bool, italic: bool) -> bool {
    let name = family_key(family);
    lib.by_family
        .get(&(name, bold, italic))
        .is_some_and(|id| lib.loaded.contains_key(id))
}

enum FaceChoice {
    Stroke,
    Missing,
    Load(usize),
}

fn choose_face(index: &[FaceRecord], label: &str, bold: bool, italic: bool) -> FaceChoice {
    if is_shx(label) {
        return FaceChoice::Stroke;
    }
    if let Some(found) = find_by_file(index, label) {
        return FaceChoice::Load(found);
    }
    if let Some(found) = find_by_family(index, label, bold, italic) {
        return FaceChoice::Load(found);
    }
    // Lowercase names such as `txt` and `isocp` are SHX faces named from
    // an MTEXT \f code. A missing TrueType family or .ttf file is reported.
    if looks_truetype(label) || label.chars().any(|ch| ch.is_uppercase()) {
        FaceChoice::Missing
    } else {
        FaceChoice::Stroke
    }
}

fn load_face(lib: &mut Library, index: usize) -> Option<u64> {
    let (path, file_key, collection_index, families, bold, italic) = {
        let record = lib.index.get(index)?;
        if let Some(id) = lib.by_file.get(&record.file_key).copied() {
            if lib.loaded.contains_key(&id) {
                return Some(id);
            }
        }
        (
            record.path.clone(),
            record.file_key.clone(),
            record.collection_index,
            record.families.clone(),
            record.bold,
            record.italic,
        )
    };
    let bytes = fs::read(&path).ok()?;
    let bytes = std::sync::Arc::new(bytes);
    let face = Face::parse(&bytes, collection_index).ok()?;
    let cap_units = cap_height(&face);
    let id = lib.next_id;
    lib.next_id = lib.next_id.saturating_add(1);
    lib.loaded.insert(
        id,
        std::sync::Arc::new(TrueTypeFont {
            id,
            bytes,
            face_index: collection_index,
            cap_units,
        }),
    );
    lib.by_file.entry(file_key).or_insert(id);
    for family in families {
        lib.by_family.entry((family, bold, italic)).or_insert(id);
    }
    Some(id)
}

fn cap_height(face: &Face<'_>) -> f32 {
    if let Some(height) = face.capital_height() {
        if height > 0 {
            return f32::from(height);
        }
    }
    if let Some(id) = face.glyph_index('H') {
        if let Some(bounds) = face.glyph_bounding_box(id) {
            if bounds.y_max > 0 {
                return f32::from(bounds.y_max);
            }
        }
    }
    let ascender = face.ascender();
    if ascender > 0 {
        f32::from(ascender)
    } else {
        f32::from(face.units_per_em()).max(1.0)
    }
}

fn ensure_index() -> Option<()> {
    {
        let lib = lock_library()?;
        if lib.scanned {
            return Some(());
        }
    }
    let index = scan_fonts();
    let mut lib = lock_library()?;
    if !lib.scanned {
        lib.index = index;
        lib.scanned = true;
    }
    Some(())
}

fn scan_fonts() -> Vec<FaceRecord> {
    let mut records = Vec::new();
    for dir in font_dirs() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !is_font_file(&path) {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            let count = ttf_parser::fonts_in_collection(&bytes).unwrap_or(1).max(1);
            for index in 0..count {
                let Ok(face) = Face::parse(&bytes, index) else {
                    continue;
                };
                let families = family_names(&face);
                if families.is_empty() {
                    continue;
                }
                records.push(FaceRecord {
                    file_key: path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(file_key)
                        .unwrap_or_default(),
                    path: path.clone(),
                    collection_index: index,
                    families,
                    bold: face.is_bold(),
                    italic: face.is_italic(),
                });
            }
        }
    }
    records
}

fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(windir) = std::env::var_os("WINDIR") {
        dirs.push(PathBuf::from(windir).join("Fonts"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(
            PathBuf::from(local)
                .join("Microsoft")
                .join("Windows")
                .join("Fonts"),
        );
    }
    dirs
}

fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "otc"
            )
        })
}

fn family_names(face: &Face<'_>) -> Vec<String> {
    let mut names = Vec::new();
    push_unique(&mut names, pick_name(face, ttf_parser::name_id::FAMILY));
    push_unique(
        &mut names,
        pick_name(face, ttf_parser::name_id::TYPOGRAPHIC_FAMILY),
    );
    let family = pick_name(face, ttf_parser::name_id::FAMILY);
    let subfamily = pick_name(face, ttf_parser::name_id::SUBFAMILY);
    if let (Some(family), Some(subfamily)) = (family, subfamily) {
        if !is_plain_subfamily(&subfamily) {
            push_unique(&mut names, Some(format!("{family} {subfamily}")));
        }
    }
    names.into_iter().map(|name| family_key(&name)).collect()
}

fn push_unique(names: &mut Vec<String>, value: Option<String>) {
    let Some(value) = value else {
        return;
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    if names.iter().any(|name| name.eq_ignore_ascii_case(trimmed)) {
        return;
    }
    names.push(trimmed.to_string());
}

fn is_plain_subfamily(subfamily: &str) -> bool {
    matches!(
        subfamily.to_ascii_lowercase().as_str(),
        "regular" | "bold" | "italic" | "bold italic" | "normal"
    )
}

fn pick_name(face: &Face<'_>, name_id: u16) -> Option<String> {
    let mut fallback = None;
    for name in face.names() {
        if name.name_id != name_id {
            continue;
        }
        let Some(value) = name.to_string() else {
            continue;
        };
        if value.trim().is_empty() {
            continue;
        }
        if name.language_id == 0x0409 {
            return Some(value);
        }
        if fallback.is_none() {
            fallback = Some(value);
        }
    }
    fallback
}

fn find_by_file(index: &[FaceRecord], label: &str) -> Option<usize> {
    let key = file_key(label);
    if let Some(found) = index.iter().position(|face| face.file_key == key) {
        return Some(found);
    }
    for ext in [".ttf", ".otf", ".ttc", ".otc"] {
        let with_ext = format!("{key}{ext}");
        if let Some(found) = index.iter().position(|face| face.file_key == with_ext) {
            return Some(found);
        }
    }
    None
}

fn find_by_family(index: &[FaceRecord], label: &str, bold: bool, italic: bool) -> Option<usize> {
    let name = family_key(label);
    if name.is_empty() {
        return None;
    }
    let mut best: Option<(usize, i32)> = None;
    for (index, face) in index.iter().enumerate() {
        if !face.families.iter().any(|family| family == &name) {
            continue;
        }
        let mut score = 0;
        if face.bold == bold {
            score += 2;
        }
        if face.italic == italic {
            score += 2;
        }
        if best.map(|(_, current)| score > current).unwrap_or(true) {
            best = Some((index, score));
        }
    }
    best.map(|(index, _)| index)
}

fn is_shx(label: &str) -> bool {
    let lower = label.trim().to_ascii_lowercase();
    lower.is_empty() || lower.ends_with(".shx") || lower.ends_with(".shp")
}

fn looks_truetype(label: &str) -> bool {
    let lower = label.trim().to_ascii_lowercase();
    lower.ends_with(".ttf")
        || lower.ends_with(".otf")
        || lower.ends_with(".ttc")
        || lower.ends_with(".otc")
        || label.contains(' ')
}

fn file_key(label: &str) -> String {
    let trimmed = label.trim();
    let name = Path::new(trimmed)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(trimmed);
    name.to_ascii_lowercase()
}

fn family_key(label: &str) -> String {
    label.trim().to_ascii_lowercase()
}

fn request_key(label: &str) -> String {
    label.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stroke_font::measure_styled_width;

    #[test]
    fn missing_truetype_font_measures_like_the_stroke_font() {
        let notes = warm_requests(&[FontRequest::File("definitely-missing.ttf".into())])
            .expect("font library lock");
        assert!(
            notes
                .iter()
                .any(|note| note.contains("definitely-missing.ttf")),
            "{notes:?}"
        );
        let font = TextFont::cached_file("definitely-missing.ttf");
        assert!(matches!(font, TextFont::Stroke));
        assert_eq!(
            font.measure("M", 10.0, 1.0, true),
            measure_styled_width("M", 10.0, 1.0)
        );
    }

    #[test]
    fn arial_m_is_narrower_than_the_stroke_font() {
        if !warm_font_file("arial.ttf") {
            return;
        }
        let font = TextFont::cached_file("arial.ttf");
        let TextFont::TrueType(_) = &font else {
            return;
        };
        let sample = "1:100 SHEET SIZE";
        let arial = font.measure(sample, 10.0, 1.0, true);
        let stroke = measure_styled_width(sample, 10.0, 1.0);
        assert!(
            arial < stroke * 0.9,
            "Arial text should be narrower than Simplex: {arial} vs {stroke}"
        );
        assert!(arial > 2.0, "Arial width {arial}");
        let drawn = font.draw(Point2::new(0.0, 0.0), 10.0, 0.0, 1.0, 0.0, "O", true);
        assert!(
            drawn.glyphs.iter().any(|glyph| glyph.len() >= 2),
            "O needs an outer contour and a hole"
        );
        assert!(drawn.strokes.is_empty());
    }
}
