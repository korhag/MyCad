//! Native CAD entities. These types must not mention LibreDWG.

use crate::color::{CadColor, Rgb};
use crate::dynamic::InstanceConfiguration;
use crate::geom::{Point2, Point3};
use crate::ids::VertexId;

pub const MAX_HATCH_PATTERN_SEGMENTS: usize = 4000;
/// Parallel lines drawn for one pattern family. A denser family is
/// thinned so the hatch still covers the whole boundary.
pub const MAX_HATCH_PATTERN_LINES: usize = 256;
/// AutoCAD 2025's default HPMAXLINES. A pattern that would need more
/// lines than this is not drawn. The limit lives in AutoCAD's settings,
/// not in the DWG, and can be raised as high as 10_000_000.
pub const HATCH_MAX_LINES: u64 = 100_000;
/// A clipped span that would need more dashes than this is drawn solid.
pub const MAX_HATCH_DASHES_PER_SPAN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DimensionKind {
    #[default]
    Linear,
    Aligned,
    Radius,
    Diameter,
    Angular2Line,
    Angular3Point,
    Ordinate,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DimensionData {
    pub block_name: String,
    pub kind: DimensionKind,
    pub definition: Point3,
    pub text_midpoint: Point3,
    pub extension1: Point3,
    pub extension2: Point3,
    pub rotation: f64,
    pub text: String,
    pub dimstyle: String,
}

impl Default for DimensionData {
    fn default() -> Self {
        Self {
            block_name: String::new(),
            kind: DimensionKind::Linear,
            definition: Point3::default(),
            text_midpoint: Point3::default(),
            extension1: Point3::default(),
            extension2: Point3::default(),
            rotation: 0.0,
            text: String::new(),
            dimstyle: "STANDARD".into(),
        }
    }
}

pub const LINEWEIGHT_BYLAYER: i16 = -1;
pub const LINEWEIGHT_BYBLOCK: i16 = -2;
pub const LINEWEIGHT_DEFAULT: i16 = -3;
/// AutoCAD's usual `$LWDEFAULT`, in hundredths of a millimetre (0.25 mm).
pub const LWDEFAULT_HUNDREDTHS: i16 = 25;

/// On-screen width in pixels for a resolved lineweight.
///
/// `hundredths / 10`, rounded, and at least 1. Zero stays one pixel, which
/// the viewport draws with the hairline line list.
pub fn lineweight_pixels(hundredths: i16) -> i32 {
    if hundredths <= 0 {
        1
    } else {
        ((f64::from(hundredths) / 10.0).round() as i32).max(1)
    }
}

// ------------------------------------------------------------
// Type: EntityId
// Purpose: Stable identity for a drawable entity. Indices into
//          model_space are not durable across insert, erase, or undo.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct EntityId(pub u64);

impl EntityId {
    pub const UNASSIGNED: Self = Self(0);

    pub fn is_assigned(self) -> bool {
        self.0 != 0
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

// ------------------------------------------------------------
// Type: Entity
// Purpose: One drawable object in the native document model.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub id: EntityId,
    pub layer: String,
    pub color: CadColor,
    pub linetype: String,
    pub linetype_scale: f64,
    pub lineweight: i16,
    pub visible: bool,
    /// Painter's order in this entity's space. A higher value is drawn later,
    /// in front. Equal values keep file order.
    pub draw_order: u32,
    pub geometry: Geometry,
}

impl Entity {
    pub fn new(geometry: Geometry) -> Self {
        Self {
            id: EntityId::UNASSIGNED,
            layer: "0".to_string(),
            color: CadColor::ByLayer,
            linetype: "BYLAYER".to_string(),
            linetype_scale: 1.0,
            lineweight: LINEWEIGHT_BYLAYER,
            visible: true,
            draw_order: 0,
            geometry,
        }
    }
}

/// One MULTILEADER leg. `arrow_size` is `None` when the leg uses the entity size.
#[derive(Debug, Clone, PartialEq)]
pub struct LeaderLine {
    pub points: Vec<Point3>,
    pub last_point: Option<Point3>,
    pub dogleg: Option<Point3>,
    pub dogleg_length: f64,
    pub arrow_size: Option<f64>,
}

/// Text or block at the end of a multileader.
#[derive(Debug, Clone, PartialEq)]
pub enum LeaderContent {
    None,
    Text {
        value: String,
        location: Point3,
        height: f64,
        rotation: f64,
        width: f64,
        frame: bool,
    },
    Block {
        name: String,
        location: Point3,
        scale: Point3,
        rotation: f64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    Line {
        start: Point3,
        end: Point3,
    },
    Point {
        position: Point3,
    },
    Circle {
        center: Point3,
        radius: f64,
        extrusion: Point3,
    },
    Arc {
        center: Point3,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
        extrusion: Point3,
    },
    Ellipse {
        center: Point3,
        major_axis: Point3,
        axis_ratio: f64,
        start_param: f64,
        end_param: f64,
        extrusion: Point3,
    },
    LwPolyline {
        vertices: Vec<PolyVertex>,
        closed: bool,
        extrusion: Point3,
        linetype_generation_continuous: bool,
    },
    Polyline {
        vertices: Vec<PolyVertex>,
        closed: bool,
        linetype_generation_continuous: bool,
    },
    Spline {
        degree: u32,
        control_points: Vec<Point3>,
        fit_points: Vec<Point3>,
        knots: Vec<f64>,
        weights: Vec<f64>,
        closed: bool,
    },
    Insert {
        block_name: String,
        insertion: Point3,
        scale: Point3,
        rotation: f64,
        extrusion: Point3,
        attribs: Vec<TextData>,
        column_count: u32,
        row_count: u32,
        column_spacing: f64,
        row_spacing: f64,
        /// Instance parameter values for this reference. Absent for static blocks.
        configuration: Option<InstanceConfiguration>,
        /// AutoCAD XCLIP boundary. Absent when the insert is not clipped.
        clip: Option<SpatialClip>,
    },
    Text(TextData),
    MText(MTextData),
    Hatch(HatchData),
    Dimension(DimensionData),
    Solid {
        corners: [Point3; 4],
        extrusion: Point3,
    },
    Leader {
        vertices: Vec<Point3>,
    },
    MLine {
        vertices: Vec<Point3>,
        /// Unit miter at each vertex. A zero or missing miter uses the left normal.
        miters: Vec<Point3>,
        closed: bool,
        /// Multiplies style offsets. Empty offsets draw the centerline.
        scale: f64,
        /// 0 top, 1 zero, 2 bottom.
        justification: u8,
        offsets: Vec<f64>,
    },
    MultiLeader {
        lines: Vec<LeaderLine>,
        content: LeaderContent,
        arrow_size: f64,
    },
    /// Display wires of a REGION. An empty list draws nothing.
    Region {
        wires: Vec<Vec<Point3>>,
    },
    /// A paper-space viewport. The rectangle lives on the sheet; the view
    /// fields describe the model-space window it shows.
    Viewport(ViewportData),
    Image(RasterFrame),
    Wipeout(RasterFrame),
}

impl LeaderLine {
    /// Stored points, then the landing, then the dogleg.
    pub fn polyline(&self) -> Vec<Point3> {
        let mut points = self.points.clone();
        if let Some(last) = self.last_point {
            push_distinct(&mut points, last);
        }
        if let Some(dir) = self.dogleg {
            if let Some(&end) = points.last() {
                push_distinct(&mut points, end + dogleg_step(dir, self.dogleg_length));
            }
        }
        points
    }

    pub fn arrow_size(&self, entity_size: f64) -> f64 {
        self.arrow_size.unwrap_or(entity_size)
    }
}

impl LeaderContent {
    pub fn location(&self) -> Option<Point3> {
        match self {
            Self::Text { location, .. } | Self::Block { location, .. } => Some(*location),
            Self::None => None,
        }
    }
}

/// One polyline per style offset. An empty offset list is the centerline.
pub fn mline_offset_polylines(
    vertices: &[Point3],
    miters: &[Point3],
    closed: bool,
    scale: f64,
    justification: u8,
    offsets: &[f64],
) -> Vec<Vec<Point3>> {
    if vertices.len() < 2 {
        return Vec::new();
    }
    let scale = if scale.is_finite() { scale } else { 1.0 };
    let close = closed && vertices.len() >= 3;
    let used: Vec<f64> = if offsets.is_empty() {
        vec![0.0]
    } else {
        offsets
            .iter()
            .copied()
            .filter(|value| value.is_finite())
            .collect()
    };
    if used.is_empty() {
        return Vec::new();
    }
    let adjust = if offsets.is_empty() {
        0.0
    } else {
        match justification {
            0 => -used.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            2 => -used.iter().copied().fold(f64::INFINITY, f64::min),
            _ => 0.0,
        }
    };
    let adjust = if adjust.is_finite() { adjust } else { 0.0 };
    let directions: Vec<Point2> = (0..vertices.len())
        .map(|index| mline_miter(vertices, miters, index, close))
        .collect();
    used.into_iter()
        .map(|offset| {
            let distance = (offset + adjust) * scale;
            vertices
                .iter()
                .enumerate()
                .map(|(index, vertex)| {
                    let miter = directions[index];
                    Point3::new(
                        vertex.x + miter.x * distance,
                        vertex.y + miter.y * distance,
                        vertex.z,
                    )
                })
                .collect()
        })
        .collect()
}

fn dogleg_step(dir: Point3, length: f64) -> Point3 {
    let len = dir.length();
    if len < 1e-15 || !length.is_finite() {
        Point3::default()
    } else {
        dir * (length / len)
    }
}

fn push_distinct(points: &mut Vec<Point3>, point: Point3) {
    if !point.is_finite() {
        return;
    }
    if points
        .last()
        .is_some_and(|prev| (*prev - point).length() <= 1e-9)
    {
        return;
    }
    points.push(point);
}

fn unit_xy(vector: Point2) -> Option<Point2> {
    let len = (vector.x * vector.x + vector.y * vector.y).sqrt();
    if len < 1e-12 {
        None
    } else {
        Some(Point2::new(vector.x / len, vector.y / len))
    }
}

fn left_normal(dir: Point2) -> Point2 {
    unit_xy(Point2::new(-dir.y, dir.x)).unwrap_or(Point2::new(0.0, 1.0))
}

fn mline_miter(vertices: &[Point3], miters: &[Point3], index: usize, closed: bool) -> Point2 {
    if let Some(miter) = miters.get(index) {
        if let Some(unit) = unit_xy(miter.xy()) {
            return unit;
        }
    }
    let here = vertices[index].xy();
    let next = if index + 1 < vertices.len() {
        index + 1
    } else if closed {
        0
    } else {
        index
    };
    let prev = if index > 0 {
        index - 1
    } else if closed {
        vertices.len() - 1
    } else {
        index
    };
    let dir = if next != index {
        vertices[next].xy() - here
    } else if prev != index {
        here - vertices[prev].xy()
    } else {
        Point2::new(1.0, 0.0)
    };
    left_normal(dir)
}

// ------------------------------------------------------------
// Type: ViewportData
// Purpose: One VIEWPORT entity. Paper size and sheet name live on
//          `PaperLayout`, not here.
// ------------------------------------------------------------
/// Row-major 4×3. `x' = m[0]*x + m[1]*y + m[2]*z + m[3]`, then the next row for y.
pub const IDENTITY_4X3: [f64; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

// ------------------------------------------------------------
// Type: SpatialClip
// Purpose: XCLIP boundary stored on an INSERT. Points are in clip
//          space. `inverse_transform` maps them into the block.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub struct SpatialClip {
    pub boundary: Vec<Point2>,
    pub origin: Point3,
    pub extrusion: Point3,
    pub inverse_transform: [f64; 12],
    pub transform: [f64; 12],
    /// The filter's own frame flag, used when the drawing has no XCLIPFRAME.
    pub frame_on: bool,
    pub front_clip: Option<f64>,
    pub back_clip: Option<f64>,
}

impl SpatialClip {
    pub fn new(boundary: Vec<Point2>) -> Self {
        Self {
            boundary,
            origin: Point3::default(),
            extrusion: Point3::new(0.0, 0.0, 1.0),
            inverse_transform: IDENTITY_4X3,
            transform: IDENTITY_4X3,
            frame_on: true,
            front_clip: None,
            back_clip: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.boundary.len() >= 3 && self.boundary.iter().all(|point| point.is_finite())
    }

    /// Boundary in the block's coordinate system.
    pub fn block_boundary(&self) -> Vec<Point2> {
        self.boundary
            .iter()
            .map(|point| apply_4x3(&self.inverse_transform, point.x, point.y))
            .filter(|point| point.is_finite())
            .collect()
    }
}

fn apply_4x3(matrix: &[f64; 12], x: f64, y: f64) -> Point2 {
    Point2::new(
        matrix[0] * x + matrix[1] * y + matrix[3],
        matrix[4] * x + matrix[5] * y + matrix[7],
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct RasterFrame {
    pub corner: Point3,
    pub u_vector: Point3,
    pub v_vector: Point3,
    pub size: Point2,
    pub clip: Vec<Point2>,
    pub path: String,
}

impl Default for RasterFrame {
    fn default() -> Self {
        Self {
            corner: Point3::default(),
            u_vector: Point3::from_xy(1.0, 0.0),
            v_vector: Point3::from_xy(0.0, 1.0),
            size: Point2::new(1.0, 1.0),
            clip: Vec::new(),
            path: String::new(),
        }
    }
}

impl RasterFrame {
    pub fn corners(&self) -> [Point3; 4] {
        let u = Point3::new(
            self.u_vector.x * self.size.x,
            self.u_vector.y * self.size.x,
            self.u_vector.z * self.size.x,
        );
        let v = Point3::new(
            self.v_vector.x * self.size.y,
            self.v_vector.y * self.size.y,
            self.v_vector.z * self.size.y,
        );
        [
            self.corner,
            Point3::new(
                self.corner.x + u.x,
                self.corner.y + u.y,
                self.corner.z + u.z,
            ),
            Point3::new(
                self.corner.x + u.x + v.x,
                self.corner.y + u.y + v.y,
                self.corner.z + u.z + v.z,
            ),
            Point3::new(
                self.corner.x + v.x,
                self.corner.y + v.y,
                self.corner.z + v.z,
            ),
        ]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewportData {
    pub center: Point3,
    pub width: f64,
    pub height: f64,
    pub view_center: Point2,
    pub view_height: f64,
    pub view_target: Point3,
    pub view_direction: Point3,
    pub twist: f64,
    pub lens_length: f64,
    pub front_z: f64,
    pub back_z: f64,
    pub snap_angle: f64,
    pub circle_zoom: i32,
    pub status: i32,
    pub id: i32,
    pub status_flag: i32,
}

impl Default for ViewportData {
    fn default() -> Self {
        Self {
            center: Point3::default(),
            width: 100.0,
            height: 100.0,
            view_center: Point2::default(),
            view_height: 100.0,
            view_target: Point3::default(),
            view_direction: Point3::new(0.0, 0.0, 1.0),
            twist: 0.0,
            lens_length: 50.0,
            front_z: 0.0,
            back_z: 0.0,
            snap_angle: 0.0,
            circle_zoom: 100,
            status: 1,
            id: 1,
            status_flag: 0,
        }
    }
}

impl ViewportData {
    pub fn corners(&self) -> [Point3; 4] {
        let half_width = self.width.abs() * 0.5;
        let half_height = self.height.abs() * 0.5;
        let center = self.center;
        [
            Point3::new(center.x - half_width, center.y - half_height, center.z),
            Point3::new(center.x + half_width, center.y - half_height, center.z),
            Point3::new(center.x + half_width, center.y + half_height, center.z),
            Point3::new(center.x - half_width, center.y + half_height, center.z),
        ]
    }
}

impl Geometry {
    // --------------------------------------------------------
    // Method: type_name
    // Purpose: Stable, user-facing name for inspectors and diagnostics.
    // --------------------------------------------------------
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Line { .. } => "Line",
            Self::Point { .. } => "Point",
            Self::Circle { .. } => "Circle",
            Self::Arc { .. } => "Arc",
            Self::Ellipse { .. } => "Ellipse",
            Self::LwPolyline { .. } => "Polyline",
            Self::Polyline { .. } => "Polyline",
            Self::Spline { .. } => "Spline",
            Self::Insert { .. } => "Block",
            Self::Text(_) => "Text",
            Self::MText(_) => "MText",
            Self::Hatch(_) => "Hatch",
            Self::Dimension(_) => "Dimension",
            Self::Solid { .. } => "Solid",
            Self::Leader { .. } => "Leader",
            Self::MLine { .. } => "MLine",
            Self::MultiLeader { .. } => "MultiLeader",
            Self::Region { .. } => "Region",
            Self::Viewport(_) => "Viewport",
            Self::Image(_) => "Image",
            Self::Wipeout(_) => "Wipeout",
        }
    }

    pub fn insert_block_name(&self) -> Option<&str> {
        match self {
            Self::Insert { block_name, .. } => Some(block_name),
            Self::Dimension(data) => Some(&data.block_name),
            _ => None,
        }
    }

    pub fn insert_block_name_mut(&mut self) -> Option<&mut String> {
        match self {
            Self::Insert { block_name, .. } => Some(block_name),
            _ => None,
        }
    }

    pub fn insert_configuration(&self) -> Option<&InstanceConfiguration> {
        match self {
            Self::Insert { configuration, .. } => configuration.as_ref(),
            _ => None,
        }
    }

    pub fn insert_configuration_mut(&mut self) -> Option<&mut Option<InstanceConfiguration>> {
        match self {
            Self::Insert { configuration, .. } => Some(configuration),
            _ => None,
        }
    }

    pub fn set_insert_configuration(&mut self, value: Option<InstanceConfiguration>) {
        if let Self::Insert { configuration, .. } = self {
            *configuration = value;
        }
    }

    pub fn polyline_vertices(&self) -> Option<&[PolyVertex]> {
        match self {
            Self::LwPolyline { vertices, .. } | Self::Polyline { vertices, .. } => Some(vertices),
            _ => None,
        }
    }

    pub fn polyline_vertices_mut(&mut self) -> Option<&mut Vec<PolyVertex>> {
        match self {
            Self::LwPolyline { vertices, .. } | Self::Polyline { vertices, .. } => Some(vertices),
            _ => None,
        }
    }

    pub fn polyline_has_curves(&self) -> bool {
        self.polyline_vertices()
            .is_some_and(|vertices| vertices.iter().any(|vertex| vertex.bulge.abs() > 1e-12))
    }
}

/// Widths at or below this are drawn as a centerline.
pub const POLYLINE_WIDTH_MIN: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolyVertex {
    pub point: Point3,
    pub bulge: f64,
    pub vertex_id: VertexId,
    /// Width at the start of the segment that leaves this vertex (DXF group 40).
    pub start_width: f64,
    /// Width at the end of that segment (DXF group 41).
    pub end_width: f64,
}

impl PolyVertex {
    pub fn new(point: Point3, bulge: f64) -> Self {
        Self {
            point,
            bulge,
            vertex_id: VertexId::UNASSIGNED,
            start_width: 0.0,
            end_width: 0.0,
        }
    }

    pub fn is_wide(self) -> bool {
        self.start_width > POLYLINE_WIDTH_MIN || self.end_width > POLYLINE_WIDTH_MIN
    }
}

pub fn polyline_is_wide(vertices: &[PolyVertex]) -> bool {
    vertices.iter().any(|vertex| vertex.is_wide())
}

// ------------------------------------------------------------
// Enum: TextHAlign / TextVAlign
// Purpose: DXF group 72 / 73. Left+Baseline uses the insertion
//          point; any other combination uses the alignment point.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextHAlign {
    #[default]
    Left = 0,
    Center = 1,
    Right = 2,
    Aligned = 3,
    Middle = 4,
    Fit = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextVAlign {
    #[default]
    Baseline = 0,
    Bottom = 1,
    Middle = 2,
    Top = 3,
}

impl TextHAlign {
    pub fn from_dxf(code: i16) -> Self {
        match code {
            1 => Self::Center,
            2 => Self::Right,
            3 => Self::Aligned,
            4 => Self::Middle,
            5 => Self::Fit,
            _ => Self::Left,
        }
    }

    pub fn to_dxf(self) -> i16 {
        self as i16
    }

    pub fn uses_alignment_point(self) -> bool {
        !matches!(self, Self::Left)
    }
}

impl TextVAlign {
    pub fn from_dxf(code: i16) -> Self {
        match code {
            1 => Self::Bottom,
            2 => Self::Middle,
            3 => Self::Top,
            _ => Self::Baseline,
        }
    }

    pub fn to_dxf(self) -> i16 {
        self as i16
    }

    pub fn uses_alignment_point(self) -> bool {
        !matches!(self, Self::Baseline)
    }
}

pub const ATTRIB_INVISIBLE: i16 = 1;
pub const ATTRIB_CONSTANT: i16 = 2;
pub const ATTRIB_VERIFY: i16 = 4;
pub const ATTRIB_PRESET: i16 = 8;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AttributeInfo {
    pub tag: String,
    pub prompt: String,
    pub flags: i16,
}

impl AttributeInfo {
    pub fn invisible(&self) -> bool {
        self.flags & ATTRIB_INVISIBLE != 0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextData {
    pub insertion: Point3,
    pub height: f64,
    pub rotation: f64,
    pub value: String,
    pub extrusion: Point3,
    pub is_attrib_def: bool,
    pub halign: TextHAlign,
    pub valign: TextVAlign,
    pub alignment: Point3,
    pub width_factor: f64,
    pub oblique: f64,
    /// STYLE table name. Empty means STANDARD.
    pub style: String,
    pub attribute: Option<AttributeInfo>,
}

impl Default for TextData {
    fn default() -> Self {
        Self {
            insertion: Point3::default(),
            height: 1.0,
            rotation: 0.0,
            value: String::new(),
            extrusion: default_extrusion(),
            is_attrib_def: false,
            halign: TextHAlign::Left,
            valign: TextVAlign::Baseline,
            alignment: Point3::default(),
            width_factor: 1.0,
            oblique: 0.0,
            style: "STANDARD".to_string(),
            attribute: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MTextData {
    pub insertion: Point3,
    pub height: f64,
    pub rotation: f64,
    pub width: f64,
    pub value: String,
    pub extrusion: Point3,
    /// AutoCAD group 71. 1 is top-left, 9 is bottom-right.
    pub attachment: i16,
    pub line_spacing: f64,
    pub style: String,
}

impl Default for MTextData {
    fn default() -> Self {
        Self {
            insertion: Point3::default(),
            height: 1.0,
            rotation: 0.0,
            width: 0.0,
            value: String::new(),
            extrusion: default_extrusion(),
            attachment: 1,
            line_spacing: 1.0,
            style: "STANDARD".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HatchData {
    pub extrusion: Point3,
    pub elevation: f64,
    pub solid_fill: bool,
    pub pattern_name: String,
    pub pattern_scale: f64,
    pub pattern_angle: f64,
    pub pattern_type: i16,
    pub double: bool,
    /// 0 = normal (odd parity), 1 = outermost, 2 = ignore holes.
    pub style: i16,
    pub gradient: Option<HatchGradient>,
    pub paths: Vec<HatchPath>,
    pub pattern_lines: Vec<HatchPatternLine>,
}

impl Default for HatchData {
    fn default() -> Self {
        Self {
            extrusion: default_extrusion(),
            elevation: 0.0,
            solid_fill: true,
            pattern_name: "SOLID".into(),
            pattern_scale: 1.0,
            pattern_angle: 0.0,
            pattern_type: 1,
            double: false,
            style: 0,
            gradient: None,
            paths: Vec::new(),
            pattern_lines: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HatchGradient {
    pub name: String,
    pub angle: f64,
    pub shift: f64,
    pub single_color: bool,
    pub tint: f64,
    pub stops: Vec<GradientStop>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub shift: f64,
    pub color: Rgb,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HatchPath {
    Polyline {
        vertices: Vec<PolyVertex>,
        closed: bool,
    },
    Edges(Vec<HatchEdge>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum HatchEdge {
    Line {
        start: Point3,
        end: Point3,
    },
    Arc {
        center: Point3,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
        is_ccw: bool,
    },
    Ellipse {
        center: Point3,
        major_endpoint: Point3,
        axis_ratio: f64,
        start_angle: f64,
        end_angle: f64,
        is_ccw: bool,
    },
    Spline {
        degree: u32,
        periodic: bool,
        knots: Vec<f64>,
        weights: Vec<f64>,
        control_points: Vec<Point3>,
        fit_points: Vec<Point3>,
    },
}

impl HatchEdge {
    /// A cubic spline through `control_points` when the file omitted the knot data.
    pub fn spline(control_points: Vec<Point3>) -> Self {
        Self::Spline {
            degree: 3,
            periodic: false,
            knots: Vec::new(),
            weights: Vec::new(),
            control_points,
            fit_points: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HatchPatternLine {
    pub angle: f64,
    pub base: Point3,
    pub offset: Point3,
    pub dashes: Vec<f64>,
}

pub fn default_extrusion() -> Point3 {
    Point3::new(0.0, 0.0, 1.0)
}
