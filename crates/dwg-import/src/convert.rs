//! Convert a live LibreDWG `Dwg_Data` into a cad-core `Document`.
//! LibreDWG types never leave this module.

use std::collections::{BTreeMap, HashSet};
use std::ffi::{c_void, CStr};
use std::os::raw::c_char;
use std::path::Path;

use cad_core::{
    default_extrusion, is_paper_layout_block, normalize_linetype_name, sorted_paper_layout_blocks,
    AttributeInfo, BlockDefinition, CadColor, DimensionData, DimensionKind, Document, DrawingUnits,
    Entity, EntityId, Geometry, HatchData, HatchEdge, HatchPath, HatchPatternLine,
    ImportDiagnostics, Layer, LineType, LineTypeShape, MTextData, PaperLayout, Point2, Point3,
    PolyVertex, RasterFrame, TextData, TextHAlign, TextStyle, TextVAlign, ViewportData,
    MAX_INSERT_ARRAY_CELLS,
};

use crate::dynapi::{
    embedded_object, get_array_field, get_common_field, get_field, get_header_field,
    get_utf8_field, object_dxfname, object_fixedtype, read_raw_array, resolve_handle_name, Point2D,
    Point3D, SplineControlPoint,
};
use crate::ltype::{
    linetype_from_flags, parse_ltype_dashes_r11, parse_ltype_records, DashRecord, LtypeDash,
};

// DWG LWPOLYLINE flag: bit 512 is closed. Bit 1 means the extrusion was stored.
const LWPOLYLINE_CLOSED_BIT: u16 = 512;
const HATCH_PATH_POLYLINE: u32 = 0x02;

pub unsafe fn convert_document(
    dwg: *mut libredwg_sys::Dwg_Data,
    path: &Path,
    mut diagnostics: ImportDiagnostics,
) -> Document {
    diagnostics.object_count = unsafe { libredwg_sys::dwg_get_num_objects(dwg) } as u64;
    crate::classes::record_unhandled_classes(dwg, &mut diagnostics);
    if let Some(version) = get_header_field::<i32>(dwg, "version") {
        if diagnostics.dwg_version.is_empty() || diagnostics.dwg_version == "unknown" {
            diagnostics.dwg_version = crate::version_label(version);
        }
    }

    let mut layers = BTreeMap::new();
    let mut linetypes = BTreeMap::new();
    let mut text_styles = BTreeMap::new();
    let mut blocks = BTreeMap::new();
    let mut model_space = Vec::new();

    let num_objects = unsafe { libredwg_sys::dwg_get_num_objects(dwg) };
    for i in 0..num_objects {
        let obj = unsafe { libredwg_sys::dwg_get_object(dwg, i) };
        if obj.is_null() {
            continue;
        }
        let fixedtype = object_fixedtype(obj);
        if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LAYER {
            let ptr = unsafe { libredwg_sys::uncad_object_object_ptr(obj) };
            if let Some(layer) = convert_layer(dwg, ptr) {
                layers.insert(layer.name.clone(), layer);
            }
        } else if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_STYLE {
            let ptr = unsafe { libredwg_sys::uncad_object_object_ptr(obj) };
            if let Some(style) = convert_style(ptr) {
                text_styles.insert(style.name.clone(), style);
            }
        } else if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LTYPE {
            let ptr = unsafe { libredwg_sys::uncad_object_object_ptr(obj) };
            if let Some(lt) = convert_ltype(dwg, ptr, &mut diagnostics) {
                linetypes.insert(lt.name.clone(), lt);
            }
        }
    }

    for i in 0..num_objects {
        let obj = unsafe { libredwg_sys::dwg_get_object(dwg, i) };
        if obj.is_null() {
            continue;
        }
        if object_fixedtype(obj) != libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_BLOCK_HEADER {
            continue;
        }
        let object_ptr = unsafe { libredwg_sys::uncad_object_object_ptr(obj) };
        if object_ptr.is_null() {
            continue;
        }
        let Some(name) = block_record_name(object_ptr) else {
            continue;
        };
        let entities = unsafe { owned_entities(dwg, obj, &mut diagnostics) };
        let base_pt = pt_field(object_ptr, "BLOCK_HEADER", "base_pt").unwrap_or(Point3::default());
        if is_model_space(&name) {
            model_space.extend(entities.clone());
        }
        blocks.insert(
            name.clone(),
            BlockDefinition {
                name,
                base_pt,
                entities,
                xref_path: get_utf8_field(object_ptr, "BLOCK_HEADER", "xref_pname")
                    .unwrap_or_default(),
                xref_overlay: get_field::<u8>(object_ptr, "BLOCK_HEADER", "xrefoverlaid")
                    .unwrap_or(0)
                    != 0,
                ..Default::default()
            },
        );
    }

    unsafe {
        fill_named_blocks_from_sequences(dwg, &mut blocks, &mut diagnostics);
    }

    diagnostics.layer_count = layers.len();
    diagnostics.block_count = blocks.len();
    if let Some(p) = get_header_field::<Point3D>(dwg, "EXTMIN") {
        diagnostics
            .warnings
            .push(format!("HEADER EXTMIN {:.6},{:.6},{:.6}", p.x, p.y, p.z));
    }
    if let Some(p) = get_header_field::<Point3D>(dwg, "EXTMAX") {
        diagnostics
            .warnings
            .push(format!("HEADER EXTMAX {:.6},{:.6},{:.6}", p.x, p.y, p.z));
    }

    let ltscale = get_header_field::<f64>(dwg, "LTSCALE")
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(1.0);
    let units = get_header_field::<u16>(dwg, "INSUNITS")
        .map(DrawingUnits::from_insunits)
        .unwrap_or(DrawingUnits::Unspecified);
    let clayer = get_header_field::<*mut libredwg_sys::Dwg_Object_Ref>(dwg, "CLAYER")
        .and_then(|handle| resolve_handle_name(dwg, handle))
        .filter(|name| !name.is_empty());

    let mut document = Document::default();
    document.source_path = Some(path.to_path_buf());
    document.layers = layers;
    document.linetypes = linetypes;
    document.text_styles = text_styles;
    document.blocks = blocks;
    document.layouts = unsafe { import_paper_layouts(dwg) };
    ensure_layouts_for_paper_blocks(&mut document);
    document.model_space = model_space;
    document.diagnostics = diagnostics;
    document.ltscale = ltscale;
    document.units = units;
    document.ensure_layer_zero();
    document.apply_current_layer(clayer.as_deref());
    document.assign_missing_ids();
    document.diagnostics.extents = document.compute_extents();
    document
}

fn is_model_space(name: &str) -> bool {
    name.eq_ignore_ascii_case("*MODEL_SPACE")
}

fn c_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .trim()
        .to_string()
}

unsafe fn import_paper_layouts(dwg: *mut libredwg_sys::Dwg_Data) -> Vec<PaperLayout> {
    let mut layouts = Vec::new();
    let num_objects = unsafe { libredwg_sys::dwg_get_num_objects(dwg) };
    for index in 0..num_objects {
        let obj = unsafe { libredwg_sys::dwg_get_object(dwg, index) };
        if obj.is_null() || object_fixedtype(obj) != libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LAYOUT {
            continue;
        }
        let object_ptr = unsafe { libredwg_sys::uncad_object_object_ptr(obj) };
        if object_ptr.is_null() {
            continue;
        }
        let name = get_utf8_field(object_ptr, "LAYOUT", "layout_name").unwrap_or_default();
        if name.trim().is_empty() || name.eq_ignore_ascii_case("Model") {
            continue;
        }
        let block_name =
            get_field::<*mut libredwg_sys::Dwg_Object_Ref>(object_ptr, "LAYOUT", "block_header")
                .and_then(resolve_block_name)
                .unwrap_or_default();
        if !is_paper_layout_block(&block_name) {
            continue;
        }
        let tab_order = get_field::<u16>(object_ptr, "LAYOUT", "tab_order")
            .map(i32::from)
            .unwrap_or(layouts.len() as i32 + 1);
        let mut layout = PaperLayout::sheet(name, block_name, tab_order);
        if let Some(plot) = embedded_object(object_ptr, "LAYOUT", "plotsettings") {
            if let Some(width) =
                finite_positive(get_field::<f64>(plot, "PLOTSETTINGS", "paper_width"))
            {
                layout.paper_width = width;
            }
            if let Some(height) =
                finite_positive(get_field::<f64>(plot, "PLOTSETTINGS", "paper_height"))
            {
                layout.paper_height = height;
            }
            layout.left_margin =
                finite_non_negative(get_field::<f64>(plot, "PLOTSETTINGS", "left_margin"))
                    .unwrap_or(layout.left_margin);
            layout.bottom_margin =
                finite_non_negative(get_field::<f64>(plot, "PLOTSETTINGS", "bottom_margin"))
                    .unwrap_or(layout.bottom_margin);
            layout.right_margin =
                finite_non_negative(get_field::<f64>(plot, "PLOTSETTINGS", "right_margin"))
                    .unwrap_or(layout.right_margin);
            layout.top_margin =
                finite_non_negative(get_field::<f64>(plot, "PLOTSETTINGS", "top_margin"))
                    .unwrap_or(layout.top_margin);
        }
        layouts.push(layout);
    }
    layouts.sort_by_key(|layout| layout.tab_order);
    layouts
}

fn ensure_layouts_for_paper_blocks(document: &mut Document) {
    let mut next = document
        .layouts
        .iter()
        .map(|layout| layout.tab_order)
        .max()
        .unwrap_or(0);
    for name in sorted_paper_layout_blocks(document) {
        if document
            .layouts
            .iter()
            .any(|layout| layout.block_name.eq_ignore_ascii_case(&name))
        {
            continue;
        }
        next += 1;
        document
            .layouts
            .push(PaperLayout::sheet(format!("Layout{next}"), name, next));
    }
}

fn finite_above_zero(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 1e-9)
}

fn finite_positive(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 1.0)
}

fn finite_non_negative(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value >= 0.0)
}

fn viewport_from(entity_ptr: *mut c_void) -> ViewportData {
    let mut viewport = ViewportData::default();
    if let Some(center) = pt_field(entity_ptr, "VIEWPORT", "center") {
        viewport.center = center;
    }
    if let Some(width) = finite_above_zero(get_field::<f64>(entity_ptr, "VIEWPORT", "width")) {
        viewport.width = width;
    }
    if let Some(height) = finite_above_zero(get_field::<f64>(entity_ptr, "VIEWPORT", "height")) {
        viewport.height = height;
    }
    viewport.view_center = get_field::<Point2D>(entity_ptr, "VIEWPORT", "VIEWCTR")
        .map(|point| Point2::new(point.x, point.y))
        .unwrap_or_else(|| viewport.center.xy());
    if let Some(height) = finite_above_zero(get_field::<f64>(entity_ptr, "VIEWPORT", "VIEWSIZE")) {
        viewport.view_height = height;
    }
    if let Some(target) = pt_field(entity_ptr, "VIEWPORT", "view_target") {
        viewport.view_target = target;
    }
    if let Some(direction) = get_field::<Point3D>(entity_ptr, "VIEWPORT", "VIEWDIR") {
        viewport.view_direction = pt3(direction);
    }
    if let Some(twist) =
        get_field::<f64>(entity_ptr, "VIEWPORT", "VIEWTWIST").filter(|v| v.is_finite())
    {
        viewport.twist = twist;
    }
    if let Some(lens) = finite_above_zero(get_field::<f64>(entity_ptr, "VIEWPORT", "LENSLENGTH")) {
        viewport.lens_length = lens;
    }
    if let Some(front) =
        get_field::<f64>(entity_ptr, "VIEWPORT", "FRONTZ").filter(|v| v.is_finite())
    {
        viewport.front_z = front;
    }
    if let Some(back) = get_field::<f64>(entity_ptr, "VIEWPORT", "BACKZ").filter(|v| v.is_finite())
    {
        viewport.back_z = back;
    }
    if let Some(angle) =
        get_field::<f64>(entity_ptr, "VIEWPORT", "SNAPANG").filter(|v| v.is_finite())
    {
        viewport.snap_angle = angle;
    }
    if let Some(zoom) = get_field::<u16>(entity_ptr, "VIEWPORT", "circle_zoom") {
        viewport.circle_zoom = i32::from(zoom);
    }
    if let Some(status) = get_field::<u16>(entity_ptr, "VIEWPORT", "on_off") {
        viewport.status = i32::from(status);
    }
    if let Some(id) = get_field::<u16>(entity_ptr, "VIEWPORT", "id") {
        viewport.id = i32::from(id);
    }
    if let Some(flag) = get_field::<u32>(entity_ptr, "VIEWPORT", "status_flag") {
        viewport.status_flag = flag as i32;
    }
    viewport
}

unsafe fn fill_named_blocks_from_sequences(
    dwg: *mut libredwg_sys::Dwg_Data,
    blocks: &mut BTreeMap<String, BlockDefinition>,
    diagnostics: &mut ImportDiagnostics,
) {
    let num_objects = unsafe { libredwg_sys::dwg_get_num_objects(dwg) };
    let mut current_name: Option<String> = None;
    let mut collected = Vec::new();
    let mut base_pt = Point3::default();
    for i in 0..num_objects {
        let obj = unsafe { libredwg_sys::dwg_get_object(dwg, i) };
        if obj.is_null() {
            continue;
        }
        let fixedtype = object_fixedtype(obj);
        if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_BLOCK {
            collected.clear();
            let entity_ptr = unsafe { libredwg_sys::uncad_object_entity_ptr(obj) };
            current_name = get_utf8_field(entity_ptr, "BLOCK", "name");
            base_pt = pt_field(entity_ptr, "BLOCK", "base_pt").unwrap_or_default();
        } else if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ENDBLK {
            if let Some(name) = current_name.take() {
                if !is_model_space(&name) {
                    match blocks.get_mut(&name) {
                        Some(block) if block.entities.is_empty() => {
                            block.entities = std::mem::take(&mut collected);
                        }
                        Some(_) => collected.clear(),
                        None => {
                            let key = find_block_key(blocks, &name).unwrap_or_else(|| name.clone());
                            if let Some(block) = blocks.get_mut(&key) {
                                if block.entities.is_empty() {
                                    block.entities = std::mem::take(&mut collected);
                                } else {
                                    collected.clear();
                                }
                            } else {
                                blocks.insert(
                                    name.clone(),
                                    BlockDefinition {
                                        name,
                                        base_pt,
                                        entities: std::mem::take(&mut collected),
                                        ..Default::default()
                                    },
                                );
                            }
                        }
                    }
                } else {
                    collected.clear();
                }
            }
        } else if current_name
            .as_deref()
            .is_some_and(|name| !is_model_space(name))
        {
            let name = current_name.as_deref().unwrap();
            let needs_fill = blocks
                .get(name)
                .or_else(|| find_block_key(blocks, name).and_then(|key| blocks.get(&key)))
                .map(|block| block.entities.is_empty())
                .unwrap_or(true);
            if needs_fill {
                if let Some(entity) = unsafe { convert_one(dwg, obj, diagnostics) } {
                    collected.push(entity);
                }
            }
        }
    }
}

fn find_block_key(blocks: &BTreeMap<String, BlockDefinition>, name: &str) -> Option<String> {
    blocks
        .keys()
        .find(|key| key.eq_ignore_ascii_case(name))
        .cloned()
}

fn convert_layer(dwg: *mut libredwg_sys::Dwg_Data, object_ptr: *mut c_void) -> Option<Layer> {
    let name = get_utf8_field(object_ptr, "LAYER", "name")?;
    let color = get_field::<libredwg_sys::Dwg_Color>(object_ptr, "LAYER", "color");
    let color = color.map(resolve_layer_color).unwrap_or(CadColor::Aci(7));
    let off = get_field::<u8>(object_ptr, "LAYER", "off").unwrap_or(0) != 0;
    let frozen = get_field::<u8>(object_ptr, "LAYER", "frozen").unwrap_or(0) != 0
        || get_field::<u8>(object_ptr, "LAYER", "flag")
            .map(|f| f & 1 != 0)
            .unwrap_or(false);
    let linetype = get_field::<*mut libredwg_sys::Dwg_Object_Ref>(object_ptr, "LAYER", "ltype")
        .and_then(|h| resolve_handle_name(dwg, h))
        .map(|n| normalize_linetype_name(&n))
        .filter(|n| !n.is_empty() && n != "BYLAYER" && n != "BYBLOCK")
        .unwrap_or_else(|| "CONTINUOUS".to_string());
    Some(Layer {
        name,
        visible: !off,
        frozen,
        locked: get_field::<u8>(object_ptr, "LAYER", "locked").unwrap_or(0) != 0,
        plot: get_field::<u8>(object_ptr, "LAYER", "plotflag").unwrap_or(1) != 0,
        color,
        linetype,
        lineweight: layer_lineweight(object_ptr),
    })
}

fn layer_lineweight(object_ptr: *mut c_void) -> i16 {
    get_field::<i8>(object_ptr, "LAYER", "linewt")
        .map(i16::from)
        .or_else(|| get_field::<u8>(object_ptr, "LAYER", "linewt").map(i16::from))
        .map(lineweight_from_index)
        .unwrap_or(cad_core::LINEWEIGHT_DEFAULT)
}

fn entity_lineweight(entity_ptr: *mut c_void) -> i16 {
    get_common_field::<i8>(entity_ptr, "linewt")
        .map(i16::from)
        .or_else(|| get_common_field::<u8>(entity_ptr, "linewt").map(i16::from))
        .map(lineweight_from_index)
        .unwrap_or(cad_core::LINEWEIGHT_BYLAYER)
}

/// LibreDWG stores DXF group 370 as an index into the lineweight table.
/// Values outside that table are already hundredths of a millimetre.
fn lineweight_from_index(raw: i16) -> i16 {
    const TABLE: [i16; 32] = [
        0, 5, 9, 13, 15, 18, 20, 25, 30, 35, 40, 50, 53, 60, 70, 80, 90, 100, 106, 120, 140, 158,
        200, 211, 0, 0, 0, 0, 0, -1, -2, -3,
    ];
    usize::try_from(raw)
        .ok()
        .and_then(|index| TABLE.get(index).copied())
        .unwrap_or(raw)
}

fn resolve_layer_color(color: libredwg_sys::Dwg_Color) -> CadColor {
    let rgb = color.rgb & 0x00ff_ffff;
    // Group 420 is stored as method ACI (0xC2) with the true color in the
    // low 24 bits. A layer that only has group 62 leaves those bits at 0.
    let true_color = rgb != 0
        && (color.method == libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR
            || color.method == libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_ACI);
    if true_color {
        return CadColor::Rgb {
            r: ((rgb >> 16) & 0xff) as u8,
            g: ((rgb >> 8) & 0xff) as u8,
            b: (rgb & 0xff) as u8,
        };
    }
    if color.index == 0 {
        CadColor::Aci(7)
    } else {
        CadColor::from_aci_index(color.index)
    }
}

fn convert_style(object_ptr: *mut c_void) -> Option<TextStyle> {
    let name = get_utf8_field(object_ptr, "STYLE", "name")?;
    if name.trim().is_empty() {
        return None;
    }
    let flag = get_field::<u8>(object_ptr, "STYLE", "flag").unwrap_or(0);
    let shape = get_field::<u8>(object_ptr, "STYLE", "is_shape").unwrap_or(0) != 0;
    if flag & 1 != 0 || shape {
        return None;
    }
    let width = get_field::<f64>(object_ptr, "STYLE", "width_factor")
        .filter(|value| value.is_finite() && *value > 1e-9)
        .unwrap_or(1.0);
    Some(TextStyle {
        name,
        font_file: get_utf8_field(object_ptr, "STYLE", "font_file").unwrap_or_else(|| "txt".into()),
        bigfont_file: get_utf8_field(object_ptr, "STYLE", "bigfont_file").unwrap_or_default(),
        height: get_field::<f64>(object_ptr, "STYLE", "text_size")
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(0.0),
        width_factor: width,
        oblique: get_field::<f64>(object_ptr, "STYLE", "oblique_angle").unwrap_or(0.0),
    })
}

fn resolved_style(dwg: *mut libredwg_sys::Dwg_Data, entity_ptr: *mut c_void, dxf: &str) -> String {
    get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, dxf, "style")
        .and_then(|handle| resolve_handle_name(dwg, handle))
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "STANDARD".to_string())
}

fn convert_ltype(
    dwg: *mut libredwg_sys::Dwg_Data,
    object_ptr: *mut c_void,
    diagnostics: &mut ImportDiagnostics,
) -> Option<LineType> {
    let raw_name = get_utf8_field(object_ptr, "LTYPE", "name")?;
    let num = get_field::<u8>(object_ptr, "LTYPE", "numdashes").unwrap_or(0);
    let ptr =
        get_field::<*const LtypeDash>(object_ptr, "LTYPE", "dashes").unwrap_or(std::ptr::null());
    let parsed = if !ptr.is_null() && num > 0 {
        let slice = unsafe { std::slice::from_raw_parts(ptr, num as usize) };
        let records: Vec<DashRecord> = slice.iter().map(|dash| dash_record(dwg, dash)).collect();
        parse_ltype_records(&raw_name, &records)
    } else {
        let r11 = get_field::<[f64; 12]>(object_ptr, "LTYPE", "dashes_r11").unwrap_or([0.0; 12]);
        let pattern_len = get_field::<f64>(object_ptr, "LTYPE", "pattern_len");
        parse_ltype_dashes_r11(&raw_name, &r11, pattern_len)
    };
    for warning in parsed.warnings {
        diagnostics.note_lossy(warning);
    }
    if parsed.shapes.iter().flatten().any(shape_has_no_font) {
        diagnostics.note_lossy(format!(
            "LTYPE '{}': a complex dash has no style or shape file",
            parsed.name
        ));
    }
    Some(LineType {
        name: parsed.name,
        dashes: parsed.dashes,
        shapes: parsed.shapes,
    })
}

fn dash_record(dwg: *mut libredwg_sys::Dwg_Data, dash: &LtypeDash) -> DashRecord {
    let shape = if dash.shape_flag == 0 {
        None
    } else {
        let (style, shape_file) = shape_style(dwg, dash.style);
        Some(LineTypeShape {
            flag: dash.shape_flag,
            shapecode: dash.complex_shapecode,
            text: c_string(dash.text),
            scale: finite_or(dash.scale, 1.0),
            rotation: finite_or(dash.rotation, 0.0),
            x_offset: finite_or(dash.x_offset, 0.0),
            y_offset: finite_or(dash.y_offset, 0.0),
            style,
            shape_file,
        })
    };
    DashRecord {
        length: dash.length,
        shape,
    }
}

fn shape_has_no_font(shape: &LineTypeShape) -> bool {
    shape.flag != 0 && shape.style.trim().is_empty() && shape.shape_file.trim().is_empty()
}

fn shape_style(
    dwg: *mut libredwg_sys::Dwg_Data,
    style_ref: *mut libredwg_sys::Dwg_Object_Ref,
) -> (String, String) {
    if style_ref.is_null() {
        return (String::new(), String::new());
    }
    let object = unsafe { libredwg_sys::dwg_ref_object(dwg, style_ref) };
    if object.is_null() {
        return (
            resolve_handle_name(dwg, style_ref).unwrap_or_default(),
            String::new(),
        );
    }
    let ptr = unsafe { libredwg_sys::uncad_object_object_ptr(object) };
    if ptr.is_null() {
        return (
            resolve_handle_name(dwg, style_ref).unwrap_or_default(),
            String::new(),
        );
    }
    let flag = get_field::<u8>(ptr, "STYLE", "flag").unwrap_or(0);
    let is_shape = get_field::<u8>(ptr, "STYLE", "is_shape").unwrap_or(0) != 0;
    let font = get_utf8_field(ptr, "STYLE", "font_file").unwrap_or_default();
    if flag & 1 != 0 || is_shape {
        return (String::new(), font);
    }
    let name = get_utf8_field(ptr, "STYLE", "name")
        .filter(|name| !name.trim().is_empty())
        .or_else(|| resolve_handle_name(dwg, style_ref))
        .unwrap_or_default();
    (name, String::new())
}

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

fn block_record_name(block_header_object_ptr: *mut c_void) -> Option<String> {
    let abbreviated = get_utf8_field(block_header_object_ptr, "BLOCK_HEADER", "name");
    if let Some(block_ref) = get_field::<*mut libredwg_sys::Dwg_Object_Ref>(
        block_header_object_ptr,
        "BLOCK_HEADER",
        "block_entity",
    ) {
        if !block_ref.is_null() {
            let block_obj = unsafe { (*block_ref).obj };
            if !block_obj.is_null() {
                let entity_ptr = unsafe { libredwg_sys::uncad_object_entity_ptr(block_obj.cast()) };
                if let Some(full_name) = get_utf8_field(entity_ptr, "BLOCK", "name") {
                    if !full_name.is_empty() {
                        return Some(full_name);
                    }
                }
            }
        }
    }
    abbreviated
}

pub(crate) fn resolve_block_name(
    block_header_ref: *mut libredwg_sys::Dwg_Object_Ref,
) -> Option<String> {
    if block_header_ref.is_null() {
        return None;
    }
    let block_header_obj = unsafe { (*block_header_ref).obj };
    if block_header_obj.is_null() {
        return None;
    }
    let object_ptr = unsafe { libredwg_sys::uncad_object_object_ptr(block_header_obj.cast()) };
    if object_ptr.is_null() {
        return None;
    }
    block_record_name(object_ptr)
}

// LibreDWG `Dwg_Version_Type` enumerator. Same value as `DwgOutputVersion::R2000`.
const LIBREDWG_R_2000: i32 = 25;

unsafe fn owned_entities(
    dwg: *mut libredwg_sys::Dwg_Data,
    block_obj: *mut libredwg_sys::Dwg_Object,
    diagnostics: &mut ImportDiagnostics,
) -> Vec<Entity> {
    // R2000 `get_next_owned_entity` skips ATTDEF. After a DXF read the owned
    // vector still lists every definition, and it contains the iterator's
    // entities. A DWG read is the other way around, so the vector is used only
    // when it covers that iterator.
    let version = get_header_field::<i32>(dwg, "version").unwrap_or(0);
    if version <= LIBREDWG_R_2000 {
        let from_iterator = iterator_entity_objects(block_obj);
        let from_vector = block_header_entities(dwg, block_obj).unwrap_or_default();
        let objects = r2000_owned_objects(from_iterator, from_vector);
        return convert_owned_objects(dwg, &objects, diagnostics);
    }
    let mut entities = Vec::new();
    let mut owned = unsafe { libredwg_sys::get_first_owned_entity(block_obj) };
    while !owned.is_null() {
        if let Some(entity) = unsafe { convert_one(dwg, owned, diagnostics) } {
            entities.push(entity);
        }
        owned = unsafe { libredwg_sys::get_next_owned_entity(block_obj, owned) };
    }
    entities
}

fn block_header_entities(
    dwg: *mut libredwg_sys::Dwg_Data,
    block_obj: *mut libredwg_sys::Dwg_Object,
) -> Option<Vec<*mut libredwg_sys::Dwg_Object>> {
    let header = unsafe { libredwg_sys::uncad_object_object_ptr(block_obj) };
    if header.is_null() {
        return None;
    }
    let count = get_field::<u32>(header, "BLOCK_HEADER", "num_owned")?;
    if count == 0 {
        return None;
    }
    let limit = unsafe { libredwg_sys::dwg_get_num_objects(dwg) } as u32;
    if count > limit {
        return None;
    }
    let handles =
        get_field::<*const *mut libredwg_sys::Dwg_Object_Ref>(header, "BLOCK_HEADER", "entities")?;
    if handles.is_null() {
        return None;
    }
    let handles = unsafe { std::slice::from_raw_parts(handles, count as usize) };
    let mut objects = Vec::with_capacity(handles.len());
    for handle in handles {
        if handle.is_null() {
            continue;
        }
        let mut object = unsafe { (**handle).obj.cast::<libredwg_sys::Dwg_Object>() };
        if object.is_null() {
            object = unsafe { libredwg_sys::dwg_ref_object(dwg, *handle) };
        }
        if object.is_null() || is_linked_subentity(object_fixedtype(object)) {
            continue;
        }
        objects.push(object);
    }
    Some(objects)
}

fn is_linked_subentity(fixedtype: libredwg_sys::DWG_OBJECT_TYPE) -> bool {
    matches!(
        fixedtype,
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTRIB
            | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_2D
            | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_3D
            | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_MESH
            | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_PFACE
            | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_PFACE_FACE
    )
}

fn iterator_entity_objects(
    block_obj: *mut libredwg_sys::Dwg_Object,
) -> Vec<*mut libredwg_sys::Dwg_Object> {
    let mut objects = Vec::new();
    let mut seen = HashSet::new();
    let mut owned = unsafe { libredwg_sys::get_first_owned_entity(block_obj) };
    while !owned.is_null() && seen.insert(owned as usize) {
        objects.push(owned);
        owned = unsafe { libredwg_sys::get_next_owned_entity(block_obj, owned) };
    }
    objects
}

fn r2000_owned_objects(
    from_iterator: Vec<*mut libredwg_sys::Dwg_Object>,
    from_vector: Vec<*mut libredwg_sys::Dwg_Object>,
) -> Vec<*mut libredwg_sys::Dwg_Object> {
    let mut iterator_ids: HashSet<usize> = from_iterator
        .iter()
        .map(|object| *object as usize)
        .collect();
    let vector_ids: HashSet<usize> = from_vector.iter().map(|object| *object as usize).collect();
    let vector_covers_iterator = from_iterator
        .iter()
        .all(|object| vector_ids.contains(&(*object as usize)));
    if !from_vector.is_empty() && vector_covers_iterator && from_vector.len() >= from_iterator.len()
    {
        return from_vector;
    }
    let mut objects = from_iterator;
    for object in from_vector {
        if object_fixedtype(object) == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTDEF
            && iterator_ids.insert(object as usize)
        {
            objects.push(object);
        }
    }
    objects
}

fn convert_owned_objects(
    dwg: *mut libredwg_sys::Dwg_Data,
    objects: &[*mut libredwg_sys::Dwg_Object],
    diagnostics: &mut ImportDiagnostics,
) -> Vec<Entity> {
    let mut entities = Vec::new();
    for object in objects {
        if let Some(entity) = unsafe { convert_one(dwg, *object, diagnostics) } {
            entities.push(entity);
        }
    }
    entities
}

unsafe fn convert_one(
    dwg: *mut libredwg_sys::Dwg_Data,
    obj: *mut libredwg_sys::Dwg_Object,
    diagnostics: &mut ImportDiagnostics,
) -> Option<Entity> {
    let fixedtype = object_fixedtype(obj);
    let entity_ptr = unsafe { libredwg_sys::uncad_object_entity_ptr(obj) };
    if entity_ptr.is_null() {
        return None;
    }
    if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_BLOCK
        || fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ENDBLK
        || fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_SEQEND
        || fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_2D
        || fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_3D
        || fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_MESH
        || fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_PFACE
        || fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_PFACE_FACE
    {
        return None;
    }

    let type_name = object_dxfname(obj);
    diagnostics.bump_entity(&type_name);

    let layer = get_common_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, "layer")
        .and_then(|h| resolve_handle_name(dwg, h))
        .unwrap_or_else(|| "0".to_string());
    let color = entity_color(entity_ptr);
    let linetype = linetype_from_flags(
        get_common_field::<u8>(entity_ptr, "ltype_flags"),
        get_common_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, "ltype")
            .and_then(|h| resolve_handle_name(dwg, h)),
    );
    let linetype_scale = get_common_field::<f64>(entity_ptr, "ltype_scale").unwrap_or(1.0);
    let invisible = get_common_field::<u16>(entity_ptr, "invisible").unwrap_or(0) != 0;

    let geometry = match convert_geometry(dwg, obj, entity_ptr, fixedtype, diagnostics) {
        Some(g) => g,
        None => {
            diagnostics.bump_unsupported(&type_name);
            return None;
        }
    };

    Some(Entity {
        id: EntityId::UNASSIGNED,
        layer,
        color,
        linetype,
        linetype_scale,
        lineweight: entity_lineweight(entity_ptr),
        visible: !invisible,
        geometry,
    })
}

fn entity_color(entity_ptr: *mut c_void) -> CadColor {
    let Some(color) = get_common_field::<libredwg_sys::Dwg_Color>(entity_ptr, "color") else {
        return CadColor::ByLayer;
    };
    if (1..=255).contains(&color.index) {
        return CadColor::from_aci_index(color.index);
    }
    if color.method == libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR {
        let rgb = color.rgb & 0x00ff_ffff;
        return CadColor::Rgb {
            r: ((rgb >> 16) & 0xff) as u8,
            g: ((rgb >> 8) & 0xff) as u8,
            b: (rgb & 0xff) as u8,
        };
    }
    CadColor::from_aci_index(color.index)
}

unsafe fn convert_geometry(
    dwg: *mut libredwg_sys::Dwg_Data,
    obj: *mut libredwg_sys::Dwg_Object,
    entity_ptr: *mut c_void,
    fixedtype: libredwg_sys::DWG_OBJECT_TYPE,
    diagnostics: &mut ImportDiagnostics,
) -> Option<Geometry> {
    Some(match fixedtype {
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LINE => {
            // LINE endpoints are already WCS. Extrusion only affects thickness.
            Geometry::Line {
                start: pt_field(entity_ptr, "LINE", "start")?,
                end: pt_field(entity_ptr, "LINE", "end")?,
            }
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_POINT => Geometry::Point {
            position: Point3::new(
                get_field::<f64>(entity_ptr, "POINT", "x")?,
                get_field::<f64>(entity_ptr, "POINT", "y")?,
                get_field::<f64>(entity_ptr, "POINT", "z").unwrap_or(0.0),
            ),
        },
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_CIRCLE => Geometry::Circle {
            center: pt_field(entity_ptr, "CIRCLE", "center")?,
            radius: get_field::<f64>(entity_ptr, "CIRCLE", "radius")?,
            extrusion: extrusion_of(entity_ptr, "CIRCLE"),
        },
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ARC => Geometry::Arc {
            center: pt_field(entity_ptr, "ARC", "center")?,
            radius: get_field::<f64>(entity_ptr, "ARC", "radius")?,
            start_angle: get_field::<f64>(entity_ptr, "ARC", "start_angle")?,
            end_angle: get_field::<f64>(entity_ptr, "ARC", "end_angle")?,
            extrusion: extrusion_of(entity_ptr, "ARC"),
        },
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ELLIPSE => Geometry::Ellipse {
            center: pt_field(entity_ptr, "ELLIPSE", "center")?,
            major_axis: pt_field(entity_ptr, "ELLIPSE", "sm_axis")?,
            axis_ratio: get_field::<f64>(entity_ptr, "ELLIPSE", "axis_ratio")?,
            start_param: get_field::<f64>(entity_ptr, "ELLIPSE", "start_angle").unwrap_or(0.0),
            end_param: get_field::<f64>(entity_ptr, "ELLIPSE", "end_angle")
                .unwrap_or(std::f64::consts::TAU),
            extrusion: extrusion_of(entity_ptr, "ELLIPSE"),
        },
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LWPOLYLINE => {
            let points: Vec<Point2D> =
                get_array_field::<u32, Point2D>(entity_ptr, "LWPOLYLINE", "num_points", "points");
            let bulges: Vec<f64> =
                get_array_field::<u32, f64>(entity_ptr, "LWPOLYLINE", "num_bulges", "bulges");
            let flag = get_field::<u16>(entity_ptr, "LWPOLYLINE", "flag").unwrap_or(0);
            let closed = lwpolyline_is_closed(flag);
            Geometry::LwPolyline {
                vertices: points
                    .iter()
                    .enumerate()
                    .map(|(i, p)| PolyVertex {
                        point: Point3::from_xy(p.x, p.y),
                        bulge: bulges.get(i).copied().unwrap_or(0.0),
                        vertex_id: Default::default(),
                    })
                    .collect(),
                closed,
                extrusion: extrusion_of(entity_ptr, "LWPOLYLINE"),
                linetype_generation_continuous: flag & 0x80 != 0,
            }
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_POLYLINE_2D => {
            let vertices = unsafe { polyline_2d_vertices(obj, entity_ptr) };
            let flag = get_field::<u16>(entity_ptr, "POLYLINE_2D", "flag").unwrap_or(0);
            Geometry::Polyline {
                vertices,
                closed: flag & 1 != 0,
                linetype_generation_continuous: flag & 0x80 != 0,
            }
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_POLYLINE_3D => {
            let vertices = unsafe { polyline_3d_vertices(obj) };
            let flag = get_field::<u8>(entity_ptr, "POLYLINE_3D", "flag").unwrap_or(0);
            Geometry::Polyline {
                vertices,
                closed: flag & 1 != 0,
                linetype_generation_continuous: false,
            }
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_SPLINE => {
            let fit_points: Vec<Point3D> =
                get_array_field::<u16, Point3D>(entity_ptr, "SPLINE", "num_fit_pts", "fit_pts");
            let ctrl: Vec<SplineControlPoint> = get_array_field::<u32, SplineControlPoint>(
                entity_ptr,
                "SPLINE",
                "num_ctrl_pts",
                "ctrl_pts",
            );
            let knots: Vec<f64> = {
                let from_u32 =
                    get_array_field::<u32, f64>(entity_ptr, "SPLINE", "num_knots", "knots");
                if from_u32.is_empty() {
                    get_array_field::<u16, f64>(entity_ptr, "SPLINE", "num_knots", "knots")
                } else {
                    from_u32
                }
            };
            let degree = get_field::<u8>(entity_ptr, "SPLINE", "degree")
                .map(|d| d as u32)
                .or_else(|| get_field::<u16>(entity_ptr, "SPLINE", "degree").map(|d| d as u32))
                .unwrap_or(3);
            let flag = get_field::<u16>(entity_ptr, "SPLINE", "flag")
                .or_else(|| get_field::<u8>(entity_ptr, "SPLINE", "flag").map(|f| f as u16))
                .unwrap_or(0);
            let rational = get_field::<u8>(entity_ptr, "SPLINE", "rational").unwrap_or(0) != 0
                || get_field::<u8>(entity_ptr, "SPLINE", "weighted").unwrap_or(0) != 0;
            Geometry::Spline {
                degree,
                control_points: ctrl.iter().map(|c| Point3::new(c.x, c.y, c.z)).collect(),
                fit_points: fit_points.iter().copied().map(pt3).collect(),
                knots,
                weights: if rational {
                    ctrl.iter()
                        .map(|c| {
                            if c.w.is_finite() && c.w.abs() > 1e-12 {
                                c.w
                            } else {
                                1.0
                            }
                        })
                        .collect()
                } else {
                    Vec::new()
                },
                closed: flag & 1 != 0,
            }
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_INSERT
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MINSERT => {
            let dxf = if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MINSERT {
                "MINSERT"
            } else {
                "INSERT"
            };
            let block_name =
                get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, dxf, "block_header")
                    .and_then(resolve_block_name)
                    .filter(|name| !name.is_empty())
                    .or_else(|| {
                        get_utf8_field(entity_ptr, dxf, "block_name")
                            .filter(|name| !name.is_empty())
                    })
                    .unwrap_or_default();
            let attribs = insert_attribs(dwg, obj, entity_ptr, dxf);
            Geometry::Insert {
                block_name,
                insertion: pt_field(entity_ptr, dxf, "ins_pt")?,
                scale: pt_field(entity_ptr, dxf, "scale").unwrap_or(Point3::new(1.0, 1.0, 1.0)),
                rotation: get_field::<f64>(entity_ptr, dxf, "rotation").unwrap_or(0.0),
                extrusion: extrusion_of(entity_ptr, dxf),
                attribs,
                column_count: insert_array_count(
                    get_field::<u16>(entity_ptr, dxf, "num_cols")
                        .map(|n| n as u32)
                        .or_else(|| get_field::<u32>(entity_ptr, dxf, "num_cols")),
                    diagnostics,
                ),
                row_count: insert_array_count(
                    get_field::<u16>(entity_ptr, dxf, "num_rows")
                        .map(|n| n as u32)
                        .or_else(|| get_field::<u32>(entity_ptr, dxf, "num_rows")),
                    diagnostics,
                ),
                column_spacing: get_field::<f64>(entity_ptr, dxf, "col_spacing").unwrap_or(0.0),
                row_spacing: get_field::<f64>(entity_ptr, dxf, "row_spacing").unwrap_or(0.0),
                configuration: None,
            }
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_TEXT => {
            let (halign, valign, alignment, width_factor, oblique) =
                text_layout(entity_ptr, "TEXT");
            Geometry::Text(TextData {
                insertion: pt_field(entity_ptr, "TEXT", "ins_pt")?,
                height: get_field::<f64>(entity_ptr, "TEXT", "height").unwrap_or(1.0),
                rotation: get_field::<f64>(entity_ptr, "TEXT", "rotation").unwrap_or(0.0),
                value: get_utf8_field(entity_ptr, "TEXT", "text_value").unwrap_or_default(),
                extrusion: extrusion_of(entity_ptr, "TEXT"),
                is_attrib_def: false,
                halign,
                valign,
                alignment,
                width_factor,
                oblique,
                style: resolved_style(dwg, entity_ptr, "TEXT"),
                ..Default::default()
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTRIB => {
            Geometry::Text(attrib_text(dwg, entity_ptr)?)
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTDEF => {
            let (halign, valign, alignment, width_factor, oblique) =
                text_layout(entity_ptr, "ATTDEF");
            Geometry::Text(TextData {
                insertion: pt_field(entity_ptr, "ATTDEF", "ins_pt")?,
                height: get_field::<f64>(entity_ptr, "ATTDEF", "height").unwrap_or(1.0),
                rotation: get_field::<f64>(entity_ptr, "ATTDEF", "rotation").unwrap_or(0.0),
                value: get_utf8_field(entity_ptr, "ATTDEF", "default_value").unwrap_or_default(),
                extrusion: extrusion_of(entity_ptr, "ATTDEF"),
                is_attrib_def: true,
                halign,
                valign,
                alignment,
                width_factor,
                oblique,
                attribute: attribute_fields(entity_ptr, "ATTDEF", true),
                style: resolved_style(dwg, entity_ptr, "ATTDEF"),
                ..Default::default()
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MTEXT => {
            let x_axis =
                get_field::<Point3D>(entity_ptr, "MTEXT", "x_axis_dir").unwrap_or(Point3D {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                });
            Geometry::MText(MTextData {
                insertion: pt_field(entity_ptr, "MTEXT", "ins_pt")?,
                height: get_field::<f64>(entity_ptr, "MTEXT", "text_height").unwrap_or(1.0),
                rotation: x_axis.y.atan2(x_axis.x),
                width: get_field::<f64>(entity_ptr, "MTEXT", "rect_width").unwrap_or(0.0),
                value: get_utf8_field(entity_ptr, "MTEXT", "text").unwrap_or_default(),
                extrusion: extrusion_of(entity_ptr, "MTEXT"),
                attachment: get_field::<u16>(entity_ptr, "MTEXT", "attachment")
                    .map(|v| v as i16)
                    .filter(|v| (1..=9).contains(v))
                    .unwrap_or(1),
                line_spacing: get_field::<f64>(entity_ptr, "MTEXT", "linespace_factor")
                    .filter(|v| v.is_finite() && *v > 1e-9)
                    .unwrap_or(1.0),
                style: resolved_style(dwg, entity_ptr, "MTEXT"),
                ..Default::default()
            })
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_HATCH => convert_hatch(entity_ptr)?,
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_SOLID
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_TRACE => {
            let dxf = if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_TRACE {
                "TRACE"
            } else {
                "SOLID"
            };
            Geometry::Solid {
                // DWG stores SOLID/TRACE as 1-2-4-3. Keep polygon order 1-2-4-3
                // so the viewport does not draw a bow-tie.
                corners: solid_polygon_corners(
                    pt_field(entity_ptr, dxf, "corner1")?,
                    pt_field(entity_ptr, dxf, "corner2")?,
                    pt_field(entity_ptr, dxf, "corner3")?,
                    pt_field(entity_ptr, dxf, "corner4")?,
                ),
                extrusion: extrusion_of(entity_ptr, dxf),
            }
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ORDINATE
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_LINEAR
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ALIGNED
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG3PT
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG2LN
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_RADIUS
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_DIAMETER
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ARC_DIMENSION => {
            let dxfname = dimension_dxfname(fixedtype);
            Geometry::Dimension(dimension_data(dwg, entity_ptr, dxfname, fixedtype))
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LEADER => Geometry::Leader {
            vertices: get_array_field::<u32, Point3D>(entity_ptr, "LEADER", "num_points", "points")
                .into_iter()
                .map(pt3)
                .collect(),
        },
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VIEWPORT => {
            Geometry::Viewport(viewport_from(entity_ptr))
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MLINE => {
            let verts: Vec<libredwg_sys::Dwg_MLINE_vertex> =
                get_array_field::<u16, _>(entity_ptr, "MLINE", "num_verts", "verts");
            let flags = get_field::<u32>(entity_ptr, "MLINE", "flags")
                .or_else(|| get_field::<u16>(entity_ptr, "MLINE", "flags").map(|f| f as u32))
                .unwrap_or(0);
            Geometry::MLine {
                vertices: verts
                    .iter()
                    .map(|v| Point3::new(v.vertex.x, v.vertex.y, v.vertex.z))
                    .collect(),
                closed: flags & 2 != 0,
            }
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_RAY => Geometry::Line {
            start: pt3(get_field::<Point3D>(entity_ptr, "RAY", "point")?),
            end: {
                let p = pt3(get_field::<Point3D>(entity_ptr, "RAY", "point")?);
                let v = pt3(get_field::<Point3D>(entity_ptr, "RAY", "vector")?);
                p + v * 1_000_000.0
            },
        },
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_XLINE => {
            let p = pt3(get_field::<Point3D>(entity_ptr, "XLINE", "point")?);
            let v = pt3(get_field::<Point3D>(entity_ptr, "XLINE", "vector")?);
            Geometry::Line {
                start: p + v * -1_000_000.0,
                end: p + v * 1_000_000.0,
            }
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE__3DFACE => {
            let corners = [
                pt3(get_field::<Point3D>(entity_ptr, "3DFACE", "corner1")?),
                pt3(get_field::<Point3D>(entity_ptr, "3DFACE", "corner2")?),
                pt3(get_field::<Point3D>(entity_ptr, "3DFACE", "corner3")?),
                pt3(get_field::<Point3D>(entity_ptr, "3DFACE", "corner4")?),
            ];
            Geometry::Polyline {
                vertices: corners
                    .into_iter()
                    .map(|point| PolyVertex {
                        point,
                        bulge: 0.0,
                        vertex_id: Default::default(),
                    })
                    .collect(),
                closed: true,
                linetype_generation_continuous: false,
            }
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_IMAGE => {
            Geometry::Image(raster_frame(dwg, entity_ptr, "IMAGE"))
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_WIPEOUT => {
            Geometry::Wipeout(raster_frame(dwg, entity_ptr, "WIPEOUT"))
        }
        _ => return None,
    })
}

fn raster_frame(
    dwg: *mut libredwg_sys::Dwg_Data,
    entity_ptr: *mut c_void,
    dxf: &str,
) -> RasterFrame {
    let size =
        get_field::<Point2D>(entity_ptr, dxf, "image_size").unwrap_or(Point2D { x: 1.0, y: 1.0 });
    RasterFrame {
        corner: pt_field(entity_ptr, dxf, "pt0").unwrap_or_default(),
        u_vector: pt_field(entity_ptr, dxf, "uvec").unwrap_or(Point3::from_xy(1.0, 0.0)),
        v_vector: pt_field(entity_ptr, dxf, "vvec").unwrap_or(Point3::from_xy(0.0, 1.0)),
        size: Point2::new(size.x, size.y),
        clip: Vec::new(),
        path: image_def_path(dwg, entity_ptr, dxf),
    }
}

fn image_def_path(dwg: *mut libredwg_sys::Dwg_Data, entity_ptr: *mut c_void, dxf: &str) -> String {
    let Some(handle) = get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, dxf, "imagedef")
    else {
        return String::new();
    };
    if handle.is_null() {
        return String::new();
    }
    let object = unsafe { libredwg_sys::dwg_ref_object(dwg, handle) };
    if object.is_null() {
        return String::new();
    }
    let object_ptr = unsafe { libredwg_sys::uncad_object_object_ptr(object) };
    get_utf8_field(object_ptr, "IMAGEDEF", "file_path").unwrap_or_default()
}

fn attrib_text(dwg: *mut libredwg_sys::Dwg_Data, entity_ptr: *mut c_void) -> Option<TextData> {
    let (halign, valign, alignment, width_factor, oblique) = text_layout(entity_ptr, "ATTRIB");
    Some(TextData {
        insertion: pt_field(entity_ptr, "ATTRIB", "ins_pt")?,
        height: get_field::<f64>(entity_ptr, "ATTRIB", "height").unwrap_or(1.0),
        rotation: get_field::<f64>(entity_ptr, "ATTRIB", "rotation").unwrap_or(0.0),
        value: get_utf8_field(entity_ptr, "ATTRIB", "text_value").unwrap_or_default(),
        extrusion: extrusion_of(entity_ptr, "ATTRIB"),
        is_attrib_def: false,
        halign,
        valign,
        alignment,
        width_factor,
        oblique,
        attribute: attribute_fields(entity_ptr, "ATTRIB", false),
        style: resolved_style(dwg, entity_ptr, "ATTRIB"),
        ..Default::default()
    })
}

fn insert_attribs(
    dwg: *mut libredwg_sys::Dwg_Data,
    obj: *mut libredwg_sys::Dwg_Object,
    entity_ptr: *mut c_void,
    dxf: &str,
) -> Vec<TextData> {
    let mut attribs = Vec::new();
    let mut sub = unsafe { libredwg_sys::get_first_owned_subentity(obj) };
    if sub.is_null() {
        if let Some(first) =
            get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, dxf, "first_attrib")
        {
            if !first.is_null() {
                sub = unsafe { libredwg_sys::dwg_ref_object(dwg, first) };
            }
        }
    }
    while !sub.is_null() {
        if object_fixedtype(sub) == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTRIB {
            let ap = unsafe { libredwg_sys::uncad_object_entity_ptr(sub) };
            if let Some(text) = attrib_text(dwg, ap) {
                attribs.push(text);
            }
        }
        let next = unsafe { libredwg_sys::get_next_owned_subentity(obj, sub) };
        if next.is_null() {
            break;
        }
        sub = next;
    }
    if !attribs.is_empty() {
        return attribs;
    }
    let refs: Vec<*mut libredwg_sys::Dwg_Object_Ref> =
        get_array_field::<u32, _>(entity_ptr, dxf, "num_owned", "attribs");
    for href in refs {
        if href.is_null() {
            continue;
        }
        let owned = unsafe { libredwg_sys::dwg_ref_object(dwg, href) };
        if owned.is_null()
            || object_fixedtype(owned) != libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTRIB
        {
            continue;
        }
        let ap = unsafe { libredwg_sys::uncad_object_entity_ptr(owned) };
        if let Some(text) = attrib_text(dwg, ap) {
            attribs.push(text);
        }
    }
    attribs
}

fn attribute_fields(
    entity_ptr: *mut c_void,
    dxf: &str,
    with_prompt: bool,
) -> Option<AttributeInfo> {
    let tag = get_utf8_field(entity_ptr, dxf, "tag").unwrap_or_default();
    if tag.trim().is_empty() {
        return None;
    }
    let flags = get_field::<u8>(entity_ptr, dxf, "flags")
        .map(i16::from)
        .unwrap_or(0);
    let prompt = if with_prompt {
        get_utf8_field(entity_ptr, dxf, "prompt").unwrap_or_default()
    } else {
        String::new()
    };
    Some(AttributeInfo { tag, prompt, flags })
}

fn text_layout(entity_ptr: *mut c_void, dxf: &str) -> (TextHAlign, TextVAlign, Point3, f64, f64) {
    let horiz = get_field::<u16>(entity_ptr, dxf, "horiz_alignment").unwrap_or(0) as i16;
    let vert = get_field::<u16>(entity_ptr, dxf, "vert_alignment").unwrap_or(0) as i16;
    let alignment = pt_field(entity_ptr, dxf, "alignment_pt").unwrap_or_default();
    let width_factor = get_field::<f64>(entity_ptr, dxf, "width_factor")
        .filter(|v| v.is_finite() && *v > 1e-9)
        .unwrap_or(1.0);
    let oblique = get_field::<f64>(entity_ptr, dxf, "oblique_angle").unwrap_or(0.0);
    (
        TextHAlign::from_dxf(horiz),
        TextVAlign::from_dxf(vert),
        alignment,
        width_factor,
        oblique,
    )
}

fn convert_hatch(entity_ptr: *mut c_void) -> Option<Geometry> {
    let solid_fill = get_field::<u8>(entity_ptr, "HATCH", "is_solid_fill").unwrap_or(0) != 0;
    let paths: Vec<libredwg_sys::Dwg_HATCH_Path> =
        get_array_field::<u32, _>(entity_ptr, "HATCH", "num_paths", "paths");
    let deflines: Vec<libredwg_sys::Dwg_HATCH_DefLine> =
        get_array_field::<u16, _>(entity_ptr, "HATCH", "num_deflines", "deflines");
    let pattern_name = get_utf8_field(entity_ptr, "HATCH", "name")
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| {
            if solid_fill {
                "SOLID".into()
            } else {
                "ANSI31".into()
            }
        });
    Some(Geometry::Hatch(HatchData {
        extrusion: extrusion_of(entity_ptr, "HATCH"),
        elevation: get_field::<f64>(entity_ptr, "HATCH", "elevation").unwrap_or(0.0),
        solid_fill,
        pattern_name,
        pattern_scale: get_field::<f64>(entity_ptr, "HATCH", "scale_spacing")
            .filter(|value| value.is_finite() && *value > 1e-9)
            .unwrap_or(1.0),
        pattern_angle: get_field::<f64>(entity_ptr, "HATCH", "angle").unwrap_or(0.0),
        pattern_type: get_field::<i16>(entity_ptr, "HATCH", "pattern_type")
            .unwrap_or(if solid_fill { 1 } else { 0 }),
        double: get_field::<u8>(entity_ptr, "HATCH", "double_flag").unwrap_or(0) != 0,
        style: get_field::<u16>(entity_ptr, "HATCH", "style")
            .map(|style| i16::try_from(style).unwrap_or(0))
            .unwrap_or(0),
        gradient: convert_hatch_gradient(entity_ptr),
        paths: paths.iter().map(convert_hatch_path).collect(),
        pattern_lines: deflines.iter().map(convert_hatch_defline).collect(),
    }))
}

fn convert_hatch_gradient(entity_ptr: *mut c_void) -> Option<cad_core::HatchGradient> {
    let filled = get_field::<u32>(entity_ptr, "HATCH", "is_gradient_fill").unwrap_or(0) != 0;
    if !filled {
        return None;
    }
    let colors: Vec<libredwg_sys::Dwg_HATCH_Color> =
        get_array_field::<u32, _>(entity_ptr, "HATCH", "num_colors", "colors");
    let stops = colors
        .iter()
        .map(|stop| cad_core::GradientStop {
            shift: stop.shift_value,
            color: dwg_color_rgb(stop.color),
        })
        .filter(|stop| stop.shift.is_finite())
        .collect();
    Some(cad_core::HatchGradient {
        name: get_utf8_field(entity_ptr, "HATCH", "gradient_name").unwrap_or_default(),
        angle: get_field::<f64>(entity_ptr, "HATCH", "gradient_angle").unwrap_or(0.0),
        shift: get_field::<f64>(entity_ptr, "HATCH", "gradient_shift").unwrap_or(0.0),
        single_color: get_field::<u32>(entity_ptr, "HATCH", "single_color_gradient").unwrap_or(0)
            != 0,
        tint: get_field::<f64>(entity_ptr, "HATCH", "gradient_tint").unwrap_or(1.0),
        stops,
    })
}

fn dwg_color_rgb(color: libredwg_sys::Dwg_Color) -> cad_core::Rgb {
    resolve_layer_color(color).resolve(cad_core::CadColor::Aci(7), cad_core::CadColor::Aci(7))
}

fn convert_hatch_path(path: &libredwg_sys::Dwg_HATCH_Path) -> HatchPath {
    if path.flag & HATCH_PATH_POLYLINE != 0 {
        let vertices: Vec<libredwg_sys::Dwg_HATCH_PolylinePath> =
            unsafe { read_raw_array(path.polyline_paths, path.num_segs_or_paths) };
        HatchPath::Polyline {
            vertices: vertices
                .iter()
                .map(|v| PolyVertex {
                    point: Point3::from_xy(v.point.x, v.point.y),
                    bulge: v.bulge,
                    vertex_id: Default::default(),
                })
                .collect(),
            closed: path.closed != 0,
        }
    } else {
        let segs = unsafe { read_raw_array(path.segs, path.num_segs_or_paths) };
        HatchPath::Edges(segs.iter().filter_map(convert_hatch_edge).collect())
    }
}

// AutoCAD stores a clockwise hatch arc with both angles mirrored.
// Negating them recovers the real start and end. Counter-clockwise
// edges are already stored as real angles.
fn real_hatch_angles(start: f64, end: f64, is_ccw: bool) -> (f64, f64) {
    if is_ccw {
        (start, end)
    } else {
        (-start, -end)
    }
}

fn convert_hatch_edge(seg: &libredwg_sys::Dwg_HATCH_PathSeg) -> Option<HatchEdge> {
    let p2 = |p: libredwg_sys::BITCODE_2RD| Point3::from_xy(p.x, p.y);
    Some(match seg.curve_type {
        1 => HatchEdge::Line {
            start: p2(seg.first_endpoint),
            end: p2(seg.second_endpoint),
        },
        2 => {
            let is_ccw = seg.is_ccw != 0;
            let (start_angle, end_angle) =
                real_hatch_angles(seg.start_angle, seg.end_angle, is_ccw);
            HatchEdge::Arc {
                center: p2(seg.center),
                radius: seg.radius,
                start_angle,
                end_angle,
                is_ccw,
            }
        }
        3 => {
            let is_ccw = seg.is_ccw != 0;
            let (start_angle, end_angle) =
                real_hatch_angles(seg.start_angle, seg.end_angle, is_ccw);
            HatchEdge::Ellipse {
                center: p2(seg.center),
                major_endpoint: p2(seg.endpoint),
                axis_ratio: seg.minor_major_ratio,
                start_angle,
                end_angle,
                is_ccw,
            }
        }
        4 => {
            let controls = unsafe { read_raw_array(seg.control_points, seg.num_control_points) };
            let knots = unsafe { read_raw_array(seg.knots, seg.num_knots) };
            let fitpts = unsafe { read_raw_array(seg.fitpts, seg.num_fitpts) };
            let degree = if seg.degree == 0 { 3 } else { seg.degree };
            HatchEdge::Spline {
                degree,
                periodic: seg.is_periodic != 0,
                knots,
                weights: if seg.is_rational != 0 {
                    controls.iter().map(|cp| cp.weight).collect()
                } else {
                    Vec::new()
                },
                control_points: controls
                    .iter()
                    .map(|cp| Point3::from_xy(cp.point.x, cp.point.y))
                    .collect(),
                fit_points: fitpts
                    .iter()
                    .map(|point| Point3::from_xy(point.x, point.y))
                    .collect(),
            }
        }
        _ => return None,
    })
}

fn convert_hatch_defline(defline: &libredwg_sys::Dwg_HATCH_DefLine) -> HatchPatternLine {
    let dashes = unsafe { read_raw_array(defline.dashes, defline.num_dashes as u32) };
    HatchPatternLine {
        angle: defline.angle,
        base: Point3::from_xy(defline.pt0.x, defline.pt0.y),
        offset: Point3::from_xy(defline.offset.x, defline.offset.y),
        dashes,
    }
}

fn dimension_data(
    dwg: *mut libredwg_sys::Dwg_Data,
    entity_ptr: *mut c_void,
    dxfname: &str,
    fixedtype: libredwg_sys::DWG_OBJECT_TYPE,
) -> DimensionData {
    let kind = match fixedtype {
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ALIGNED => DimensionKind::Aligned,
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG2LN => DimensionKind::Angular2Line,
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG3PT
        | libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ARC_DIMENSION => DimensionKind::Angular3Point,
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_DIAMETER => DimensionKind::Diameter,
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_RADIUS => DimensionKind::Radius,
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ORDINATE => DimensionKind::Ordinate,
        _ => DimensionKind::Linear,
    };
    DimensionData {
        block_name: get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, dxfname, "block")
            .and_then(resolve_block_name)
            .unwrap_or_default(),
        kind,
        definition: pt_field(entity_ptr, dxfname, "def_pt").unwrap_or_default(),
        text_midpoint: pt_field(entity_ptr, dxfname, "text_midpt").unwrap_or_default(),
        extension1: pt_field(entity_ptr, dxfname, "xline1_pt").unwrap_or_default(),
        extension2: pt_field(entity_ptr, dxfname, "xline2_pt").unwrap_or_default(),
        rotation: get_field::<f64>(entity_ptr, dxfname, "dim_rotation").unwrap_or(0.0),
        text: get_utf8_field(entity_ptr, dxfname, "user_text").unwrap_or_default(),
        dimstyle: get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity_ptr, dxfname, "dimstyle")
            .and_then(|handle| resolve_handle_name(dwg, handle))
            .filter(|name| !name.trim().is_empty())
            .or_else(|| get_utf8_field(entity_ptr, dxfname, "dimstyle"))
            .unwrap_or_else(|| "STANDARD".into()),
    }
}

fn dimension_dxfname(fixedtype: libredwg_sys::DWG_OBJECT_TYPE) -> &'static str {
    match fixedtype {
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ORDINATE => "DIMENSION_ORDINATE",
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_LINEAR => "DIMENSION_LINEAR",
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ALIGNED => "DIMENSION_ALIGNED",
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG3PT => "DIMENSION_ANG3PT",
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_ANG2LN => "DIMENSION_ANG2LN",
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_RADIUS => "DIMENSION_RADIUS",
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_DIMENSION_DIAMETER => "DIMENSION_DIAMETER",
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ARC_DIMENSION => "ARC_DIMENSION",
        _ => "DIMENSION_LINEAR",
    }
}

fn insert_array_count(value: Option<u32>, diagnostics: &mut ImportDiagnostics) -> u32 {
    const MAX_AXIS: u32 = 256;
    let count = value.filter(|count| *count > 0).unwrap_or(1);
    if count > MAX_AXIS || count as u64 > MAX_INSERT_ARRAY_CELLS {
        diagnostics
            .warnings
            .push("INSERT array was reduced so the drawing stays responsive".into());
        count.min(MAX_AXIS)
    } else {
        count
    }
}

// ------------------------------------------------------------
// Function: solid_polygon_corners
// Purpose: DWG SOLID/TRACE corner order is 1-2-4-3. Return the
//          boundary order 1-2-4-3 so a rectangle is not a bow-tie.
// ------------------------------------------------------------
pub(crate) fn solid_polygon_corners(c1: Point3, c2: Point3, c3: Point3, c4: Point3) -> [Point3; 4] {
    [c1, c2, c4, c3]
}

fn extrusion_of(entity_ptr: *mut c_void, dxfname: &str) -> Point3 {
    get_field::<Point3D>(entity_ptr, dxfname, "extrusion")
        .map(pt3)
        .map(extrusion_or_world)
        .unwrap_or_else(default_extrusion)
}

pub(crate) fn lwpolyline_is_closed(flag: u16) -> bool {
    flag & LWPOLYLINE_CLOSED_BIT != 0
}

// LibreDWG leaves the extrusion at (0, 0, 0) when the entity did not store one.
// A zero normal is the world Z axis. Writing it out makes DXF readers flip the entity.
pub(crate) fn extrusion_or_world(extrusion: Point3) -> Point3 {
    if extrusion.length() < 1e-12 {
        default_extrusion()
    } else {
        extrusion
    }
}

fn pt_field(entity_ptr: *mut c_void, dxfname: &str, field: &str) -> Option<Point3> {
    if let Some(p) = get_field::<Point3D>(entity_ptr, dxfname, field) {
        return Some(pt3(p));
    }
    get_field::<Point2D>(entity_ptr, dxfname, field).map(|p| Point3::from_xy(p.x, p.y))
}

fn pt3(p: Point3D) -> Point3 {
    Point3::new(p.x, p.y, p.z)
}

unsafe fn polyline_2d_vertices(
    obj: *mut libredwg_sys::Dwg_Object,
    entity_ptr: *mut c_void,
) -> Vec<PolyVertex> {
    let mut verts = Vec::new();
    let mut sub = unsafe { libredwg_sys::get_first_owned_subentity(obj) };
    while !sub.is_null() {
        if object_fixedtype(sub) == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_2D {
            let vp = unsafe { libredwg_sys::uncad_object_entity_ptr(sub) };
            if let Some(p) = get_field::<Point3D>(vp, "VERTEX_2D", "point") {
                let bulge = get_field::<f64>(vp, "VERTEX_2D", "bulge").unwrap_or(0.0);
                verts.push(PolyVertex {
                    point: pt3(p),
                    bulge,
                    vertex_id: Default::default(),
                });
            }
        }
        sub = unsafe { libredwg_sys::get_next_owned_subentity(obj, sub) };
    }
    if !verts.is_empty() {
        return verts;
    }
    let mut error = 0i32;
    let points_ptr = unsafe { libredwg_sys::dwg_object_polyline_2d_get_points(obj, &mut error) };
    let num_points = unsafe { libredwg_sys::dwg_object_polyline_2d_get_numpoints(obj, &mut error) };
    if !points_ptr.is_null() && num_points > 0 {
        let slice = unsafe {
            std::slice::from_raw_parts(points_ptr.cast::<Point2D>(), num_points as usize)
        };
        verts = slice
            .iter()
            .map(|p| PolyVertex {
                point: Point3::from_xy(p.x, p.y),
                bulge: 0.0,
                vertex_id: Default::default(),
            })
            .collect();
        unsafe { libc::free(points_ptr.cast()) };
    }
    let _ = entity_ptr;
    verts
}

unsafe fn polyline_3d_vertices(obj: *mut libredwg_sys::Dwg_Object) -> Vec<PolyVertex> {
    // LibreDWG's R2000 get_points walk stops before the last vertex. The
    // owned-subentity list includes it.
    let mut verts = Vec::new();
    let mut sub = unsafe { libredwg_sys::get_first_owned_subentity(obj) };
    while !sub.is_null() {
        if object_fixedtype(sub) == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_VERTEX_3D {
            let vertex = unsafe { libredwg_sys::uncad_object_entity_ptr(sub) };
            if let Some(point) = get_field::<Point3D>(vertex, "VERTEX_3D", "point") {
                verts.push(PolyVertex {
                    point: pt3(point),
                    bulge: 0.0,
                    vertex_id: Default::default(),
                });
            }
        }
        sub = unsafe { libredwg_sys::get_next_owned_subentity(obj, sub) };
    }
    if !verts.is_empty() {
        return verts;
    }
    let mut error = 0i32;
    let points_ptr = unsafe { libredwg_sys::dwg_object_polyline_3d_get_points(obj, &mut error) };
    let num_points = unsafe { libredwg_sys::dwg_object_polyline_3d_get_numpoints(obj, &mut error) };
    if points_ptr.is_null() || num_points == 0 {
        return Vec::new();
    }
    let slice =
        unsafe { std::slice::from_raw_parts(points_ptr.cast::<Point3D>(), num_points as usize) };
    let verts = slice
        .iter()
        .map(|p| PolyVertex {
            point: pt3(*p),
            bulge: 0.0,
            vertex_id: Default::default(),
        })
        .collect();
    unsafe { libc::free(points_ptr.cast()) };
    verts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_space_name_is_case_insensitive() {
        assert!(is_model_space("*Model_Space"));
        assert!(is_model_space("*MODEL_SPACE"));
        assert!(!is_model_space("*Paper_Space"));
    }

    #[test]
    fn missing_clayer_falls_back_to_layer_zero() {
        let mut document = Document::default();
        document.layers.insert(
            "FROZEN".into(),
            Layer {
                name: "FROZEN".into(),
                visible: true,
                frozen: true,
                color: CadColor::Aci(1),
                linetype: "CONTINUOUS".into(),
                ..Layer::default()
            },
        );
        document.apply_current_layer(Some("FROZEN"));
        assert_eq!(document.current_layer, "0");
        document.apply_current_layer(Some("missing"));
        assert_eq!(document.current_layer, "0");
        document.apply_current_layer(None);
        assert_eq!(document.current_layer, "0");
    }

    #[test]
    fn lwpolyline_closed_bit_is_512_not_the_extrusion_bit() {
        assert!(!lwpolyline_is_closed(1));
        assert!(lwpolyline_is_closed(512));
        assert!(lwpolyline_is_closed(513));
        assert!(!lwpolyline_is_closed(256));
    }

    #[test]
    fn zero_extrusion_is_the_world_normal() {
        assert_eq!(extrusion_or_world(Point3::default()), default_extrusion());
        assert_eq!(
            extrusion_or_world(Point3::new(0.0, 0.0, -1.0)),
            Point3::new(0.0, 0.0, -1.0)
        );
        assert_eq!(extrusion_or_world(default_extrusion()), default_extrusion());
    }

    #[test]
    fn solid_corners_are_reordered_to_polygon_order() {
        let corners = solid_polygon_corners(
            Point3::from_xy(0.0, 0.0),
            Point3::from_xy(10.0, 0.0),
            Point3::from_xy(0.0, 4.0),
            Point3::from_xy(10.0, 4.0),
        );
        assert_eq!(corners[0], Point3::from_xy(0.0, 0.0));
        assert_eq!(corners[1], Point3::from_xy(10.0, 0.0));
        assert_eq!(corners[2], Point3::from_xy(10.0, 4.0));
        assert_eq!(corners[3], Point3::from_xy(0.0, 4.0));
    }
}
