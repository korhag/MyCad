//! Convert a live LibreDWG `Dwg_Data` into a cad-core `Document`.
//! LibreDWG types never leave this module.

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::path::Path;

use cad_core::{
    default_extrusion, normalize_linetype_name, ocs_to_wcs, BlockDefinition, CadColor, Document,
    DrawingUnits, Entity, EntityId, Geometry, HatchData, HatchEdge, HatchPath, HatchPatternLine,
    ImportDiagnostics, Layer, LineType, MTextData, Point3, PolyVertex, TextData,
};

use crate::dynapi::{
    get_array_field, get_common_field, get_field, get_header_field, get_utf8_field, object_dxfname,
    object_fixedtype, read_raw_array, resolve_handle_name, Point2D, Point3D, SplineControlPoint,
};
use crate::ltype::{linetype_from_flags, parse_ltype_dashes, parse_ltype_dashes_r11, LtypeDash};

const LWPOLYLINE_CLOSED_BIT1: u16 = 1;
const LWPOLYLINE_CLOSED_BIT512: u16 = 512;
const HATCH_PATH_POLYLINE: u32 = 0x02;

pub unsafe fn convert_document(
    dwg: *mut libredwg_sys::Dwg_Data,
    path: &Path,
    mut diagnostics: ImportDiagnostics,
) -> Document {
    diagnostics.object_count = unsafe { libredwg_sys::dwg_get_num_objects(dwg) } as u64;
    if let Some(version) = get_header_field::<i32>(dwg, "version") {
        if diagnostics.dwg_version.is_empty() || diagnostics.dwg_version == "unknown" {
            diagnostics.dwg_version = crate::version_label(version);
        }
    }

    let mut layers = BTreeMap::new();
    let mut linetypes = BTreeMap::new();
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
        } else if fixedtype == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LTYPE {
            let ptr = unsafe { libredwg_sys::uncad_object_object_ptr(obj) };
            if let Some(lt) = convert_ltype(ptr, &mut diagnostics) {
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
    document.blocks = blocks;
    document.model_space = model_space;
    document.diagnostics = diagnostics;
    document.ltscale = ltscale;
    document.units = units;
    document.ensure_layer_zero();
    document.apply_current_layer(clayer.as_deref());
    document.assign_missing_ids();
    document.diagnostics.extents = document.compute_extents();
    if document
        .model_space
        .iter()
        .chain(document.blocks.values().flat_map(|b| b.entities.iter()))
        .any(|e| matches!(e.geometry, Geometry::Hatch(ref h) if !h.solid_fill && !h.pattern_lines.is_empty()))
    {
        document.diagnostics.warnings.push(
            "Hatch pattern fills are drawn as boundaries in this milestone (solid hatches are filled)."
                .into(),
        );
    }
    document
}

fn is_model_space(name: &str) -> bool {
    name.eq_ignore_ascii_case("*MODEL_SPACE")
}

fn is_paper_space(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper == "*PAPER_SPACE" || upper.starts_with("*PAPER_SPACE")
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
                if !is_model_space(&name) && !is_paper_space(&name) {
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
            .is_some_and(|name| !is_model_space(name) && !is_paper_space(name))
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
    let aci = color.map(|c| resolve_layer_aci(c)).unwrap_or(7);
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
        color: CadColor::from_aci_index(aci),
        linetype,
    })
}

fn resolve_layer_aci(color: libredwg_sys::Dwg_Color) -> i16 {
    if color.index == 256
        && color.method == libredwg_sys::DWG_COLOR_METHOD_DWG_COLOR_METHOD_TRUECOLOR
    {
        (color.rgb & 0xff) as i16
    } else if color.index == 0 {
        7
    } else {
        color.index
    }
}

fn convert_ltype(object_ptr: *mut c_void, diagnostics: &mut ImportDiagnostics) -> Option<LineType> {
    let raw_name = get_utf8_field(object_ptr, "LTYPE", "name")?;
    let num = get_field::<u8>(object_ptr, "LTYPE", "numdashes").unwrap_or(0);
    let ptr =
        get_field::<*const LtypeDash>(object_ptr, "LTYPE", "dashes").unwrap_or(std::ptr::null());
    let parsed = if !ptr.is_null() && num > 0 {
        let slice = unsafe { std::slice::from_raw_parts(ptr, num as usize) };
        let elements: Vec<(f64, u16)> = slice.iter().map(|d| (d.length, d.shape_flag)).collect();
        parse_ltype_dashes(&raw_name, &elements)
    } else {
        let r11 = get_field::<[f64; 12]>(object_ptr, "LTYPE", "dashes_r11").unwrap_or([0.0; 12]);
        let pattern_len = get_field::<f64>(object_ptr, "LTYPE", "pattern_len");
        parse_ltype_dashes_r11(&raw_name, &r11, pattern_len)
    };
    diagnostics.warnings.extend(parsed.warnings);
    Some(LineType {
        name: parsed.name,
        dashes: parsed.dashes,
    })
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

unsafe fn owned_entities(
    dwg: *mut libredwg_sys::Dwg_Data,
    block_obj: *mut libredwg_sys::Dwg_Object,
    diagnostics: &mut ImportDiagnostics,
) -> Vec<Entity> {
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

    let geometry = match convert_geometry(dwg, obj, entity_ptr, fixedtype) {
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
    _dwg: *mut libredwg_sys::Dwg_Data,
    obj: *mut libredwg_sys::Dwg_Object,
    entity_ptr: *mut c_void,
    fixedtype: libredwg_sys::DWG_OBJECT_TYPE,
) -> Option<Geometry> {
    Some(match fixedtype {
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LINE => {
            let extrusion = extrusion_of(entity_ptr, "LINE");
            Geometry::Line {
                start: ocs_to_wcs(pt_field(entity_ptr, "LINE", "start")?, extrusion),
                end: ocs_to_wcs(pt_field(entity_ptr, "LINE", "end")?, extrusion),
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
            let closed = flag & LWPOLYLINE_CLOSED_BIT1 != 0 || flag & LWPOLYLINE_CLOSED_BIT512 != 0;
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
            let mut attribs = Vec::new();
            let mut sub = unsafe { libredwg_sys::get_first_owned_subentity(obj) };
            while !sub.is_null() {
                if object_fixedtype(sub) == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTRIB {
                    let ap = unsafe { libredwg_sys::uncad_object_entity_ptr(sub) };
                    if let Some(text) = attrib_text(ap) {
                        attribs.push(text);
                    }
                }
                sub = unsafe { libredwg_sys::get_next_owned_subentity(obj, sub) };
            }
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
                ),
                row_count: insert_array_count(
                    get_field::<u16>(entity_ptr, dxf, "num_rows")
                        .map(|n| n as u32)
                        .or_else(|| get_field::<u32>(entity_ptr, dxf, "num_rows")),
                ),
                column_spacing: get_field::<f64>(entity_ptr, dxf, "col_spacing").unwrap_or(0.0),
                row_spacing: get_field::<f64>(entity_ptr, dxf, "row_spacing").unwrap_or(0.0),
                configuration: None,
            }
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_TEXT => Geometry::Text(TextData {
            insertion: pt_field(entity_ptr, "TEXT", "ins_pt")?,
            height: get_field::<f64>(entity_ptr, "TEXT", "height").unwrap_or(1.0),
            rotation: get_field::<f64>(entity_ptr, "TEXT", "rotation").unwrap_or(0.0),
            value: get_utf8_field(entity_ptr, "TEXT", "text_value").unwrap_or_default(),
            extrusion: extrusion_of(entity_ptr, "TEXT"),
            is_attrib_def: false,
        }),
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTRIB => Geometry::Text(attrib_text(entity_ptr)?),
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_ATTDEF => Geometry::Text(TextData {
            insertion: pt_field(entity_ptr, "ATTDEF", "ins_pt")?,
            height: get_field::<f64>(entity_ptr, "ATTDEF", "height").unwrap_or(1.0),
            rotation: get_field::<f64>(entity_ptr, "ATTDEF", "rotation").unwrap_or(0.0),
            value: get_utf8_field(entity_ptr, "ATTDEF", "default_value").unwrap_or_default(),
            extrusion: extrusion_of(entity_ptr, "ATTDEF"),
            is_attrib_def: true,
        }),
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
                corners: [
                    pt_field(entity_ptr, dxf, "corner1")?,
                    pt_field(entity_ptr, dxf, "corner2")?,
                    pt_field(entity_ptr, dxf, "corner3")?,
                    pt_field(entity_ptr, dxf, "corner4")?,
                ],
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
            Geometry::Dimension {
                block_name: get_field::<*mut libredwg_sys::Dwg_Object_Ref>(
                    entity_ptr, dxfname, "block",
                )
                .and_then(resolve_block_name)
                .unwrap_or_default(),
            }
        }
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_LEADER => Geometry::Leader {
            vertices: get_array_field::<u32, Point3D>(entity_ptr, "LEADER", "num_points", "points")
                .into_iter()
                .map(pt3)
                .collect(),
        },
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
        libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE__3DFACE => Geometry::Solid {
            corners: [
                pt3(get_field::<Point3D>(entity_ptr, "3DFACE", "corner1")?),
                pt3(get_field::<Point3D>(entity_ptr, "3DFACE", "corner2")?),
                pt3(get_field::<Point3D>(entity_ptr, "3DFACE", "corner3")?),
                pt3(get_field::<Point3D>(entity_ptr, "3DFACE", "corner4")?),
            ],
            extrusion: default_extrusion(),
        },
        _ => return None,
    })
}

fn attrib_text(entity_ptr: *mut c_void) -> Option<TextData> {
    Some(TextData {
        insertion: pt_field(entity_ptr, "ATTRIB", "ins_pt")?,
        height: get_field::<f64>(entity_ptr, "ATTRIB", "height").unwrap_or(1.0),
        rotation: get_field::<f64>(entity_ptr, "ATTRIB", "rotation").unwrap_or(0.0),
        value: get_utf8_field(entity_ptr, "ATTRIB", "text_value").unwrap_or_default(),
        extrusion: extrusion_of(entity_ptr, "ATTRIB"),
        is_attrib_def: false,
    })
}

fn convert_hatch(entity_ptr: *mut c_void) -> Option<Geometry> {
    let solid_fill = get_field::<u8>(entity_ptr, "HATCH", "is_solid_fill").unwrap_or(0) != 0;
    let paths: Vec<libredwg_sys::Dwg_HATCH_Path> =
        get_array_field::<u32, _>(entity_ptr, "HATCH", "num_paths", "paths");
    let deflines: Vec<libredwg_sys::Dwg_HATCH_DefLine> =
        get_array_field::<u16, _>(entity_ptr, "HATCH", "num_deflines", "deflines");
    Some(Geometry::Hatch(HatchData {
        extrusion: extrusion_of(entity_ptr, "HATCH"),
        elevation: get_field::<f64>(entity_ptr, "HATCH", "elevation").unwrap_or(0.0),
        solid_fill,
        paths: paths.iter().map(convert_hatch_path).collect(),
        pattern_lines: deflines.iter().map(convert_hatch_defline).collect(),
    }))
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

fn convert_hatch_edge(seg: &libredwg_sys::Dwg_HATCH_PathSeg) -> Option<HatchEdge> {
    let p2 = |p: libredwg_sys::BITCODE_2RD| Point3::from_xy(p.x, p.y);
    Some(match seg.curve_type {
        1 => HatchEdge::Line {
            start: p2(seg.first_endpoint),
            end: p2(seg.second_endpoint),
        },
        2 => HatchEdge::Arc {
            center: p2(seg.center),
            radius: seg.radius,
            start_angle: seg.start_angle,
            end_angle: seg.end_angle,
            is_ccw: seg.is_ccw != 0,
        },
        3 => HatchEdge::Ellipse {
            center: p2(seg.center),
            major_endpoint: p2(seg.endpoint),
            axis_ratio: seg.minor_major_ratio,
            start_angle: seg.start_angle,
            end_angle: seg.end_angle,
            is_ccw: seg.is_ccw != 0,
        },
        4 => {
            let control_points =
                unsafe { read_raw_array(seg.control_points, seg.num_control_points) }
                    .into_iter()
                    .map(|cp| Point3::from_xy(cp.point.x, cp.point.y))
                    .collect();
            HatchEdge::Spline { control_points }
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

fn insert_array_count(value: Option<u32>) -> u32 {
    value.filter(|count| *count > 0).unwrap_or(1)
}

fn extrusion_of(entity_ptr: *mut c_void, dxfname: &str) -> Point3 {
    get_field::<Point3D>(entity_ptr, dxfname, "extrusion")
        .map(pt3)
        .unwrap_or_else(default_extrusion)
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
            },
        );
        document.apply_current_layer(Some("FROZEN"));
        assert_eq!(document.current_layer, "0");
        document.apply_current_layer(Some("missing"));
        assert_eq!(document.current_layer, "0");
        document.apply_current_layer(None);
        assert_eq!(document.current_layer, "0");
    }
}
