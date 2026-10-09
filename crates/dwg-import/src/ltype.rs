//! LibreDWG LTYPE dash import. `Dwg_LTYPE_dash` is not exported by
//! libredwg-sys 0.1, so this crate mirrors the C layout privately.

use std::ffi::c_void;
use std::os::raw::c_char;

use cad_core::{normalize_linetype_name, LineTypeShape};

/// Mirrors `Dwg_LTYPE_dash` / subclass `"LTYPE_dash"` in LibreDWG `dwg.h`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct LtypeDash {
    pub parent: *mut c_void,
    pub length: f64,
    pub complex_shapecode: u16,
    pub style: *mut libredwg_sys::Dwg_Object_Ref,
    pub x_offset: f64,
    pub y_offset: f64,
    pub scale: f64,
    pub rotation: f64,
    pub shape_flag: u16,
    pub text: *mut c_char,
}

const _: () = {
    assert!(std::mem::size_of::<LtypeDash>() == 80);
    assert!(std::mem::align_of::<LtypeDash>() >= 8);
};

/// LibreDWG replaces a pre-2007 linetype string area with a blank buffer
/// after it has copied dash text into that area, and it does not turn DXF
/// style flag bit 1 into `is_shape`. Repair both before `dwg_write_file`.
pub(crate) fn restore_linetype_strings_area(dwg: *mut libredwg_sys::Dwg_Data) {
    let count = unsafe { libredwg_sys::dwg_get_num_objects(dwg) };
    for index in 0..count {
        let obj = unsafe { libredwg_sys::dwg_get_object(dwg, index) };
        if obj.is_null() {
            continue;
        }
        let dxfname = crate::dynapi::object_dxfname(obj);
        let ptr = unsafe { libredwg_sys::uncad_object_object_ptr(obj) };
        if ptr.is_null() {
            continue;
        }
        if dxfname == "LTYPE" {
            copy_dash_text(ptr);
            repair_linetype(ptr);
        } else if dxfname == "STYLE" {
            mark_shape_style(ptr);
            set_u8(ptr, "STYLE", "is_xref_ref", 1);
        } else if matches!(
            dxfname.as_str(),
            "LAYER" | "DIMSTYLE" | "APPID" | "VPORT" | "VIEW" | "UCS" | "BLOCK_HEADER"
        ) {
            set_u8(ptr, &dxfname, "is_xref_ref", 1);
        }
    }
}

unsafe extern "C" {
    fn dwg_dynapi_entity_set_value(
        entity: *mut c_void,
        dxfname: *const c_char,
        fieldname: *const c_char,
        value: *const c_void,
        is_utf8: bool,
    ) -> bool;
}

fn mark_shape_style(style: *mut c_void) {
    use crate::dynapi::get_field;
    let flag = get_field::<u8>(style, "STYLE", "flag").unwrap_or(0);
    if flag & 1 == 0 {
        return;
    }
    let Ok(c_dxf) = std::ffi::CString::new("STYLE") else {
        return;
    };
    let Ok(c_field) = std::ffi::CString::new("is_shape") else {
        return;
    };
    let value: u8 = 1;
    unsafe {
        dwg_dynapi_entity_set_value(
            style,
            c_dxf.as_ptr(),
            c_field.as_ptr(),
            &value as *const u8 as *const c_void,
            false,
        );
    }
}

fn set_u8(object: *mut c_void, dxfname: &str, field: &str, value: u8) {
    let Ok(c_dxf) = std::ffi::CString::new(dxfname) else {
        return;
    };
    let Ok(c_field) = std::ffi::CString::new(field) else {
        return;
    };
    unsafe {
        dwg_dynapi_entity_set_value(
            object,
            c_dxf.as_ptr(),
            c_field.as_ptr(),
            &value as *const u8 as *const c_void,
            false,
        );
    }
}

fn set_f64(object: *mut c_void, dxfname: &str, field: &str, value: f64) {
    let Ok(c_dxf) = std::ffi::CString::new(dxfname) else {
        return;
    };
    let Ok(c_field) = std::ffi::CString::new(field) else {
        return;
    };
    unsafe {
        dwg_dynapi_entity_set_value(
            object,
            c_dxf.as_ptr(),
            c_field.as_ptr(),
            &value as *const f64 as *const c_void,
            false,
        );
    }
}

/// R2000 table records store this bit as 1. A zero shifts the fields after
/// the name, and AutoCAD then reads a plain linetype as having a dash whose
/// shape index is 0.
fn repair_linetype(ltype: *mut c_void) {
    use crate::dynapi::{get_field, get_utf8_field};
    set_u8(ltype, "LTYPE", "is_xref_ref", 1);
    let name = get_utf8_field(ltype, "LTYPE", "name").unwrap_or_default();
    let upper = name.to_ascii_uppercase();
    if matches!(upper.as_str(), "CONTINUOUS" | "BYLAYER" | "BYBLOCK") {
        set_u8(ltype, "LTYPE", "numdashes", 0);
        set_f64(ltype, "LTYPE", "pattern_len", 0.0);
        return;
    }
    let num = get_field::<u8>(ltype, "LTYPE", "numdashes").unwrap_or(0);
    let dashes =
        get_field::<*mut LtypeDash>(ltype, "LTYPE", "dashes").unwrap_or(std::ptr::null_mut());
    if num == 0 || dashes.is_null() {
        return;
    }
    for index in 0..num {
        let dash = unsafe { &mut *dashes.add(usize::from(index)) };
        if dash.shape_flag == 0 && dash.scale == 0.0 {
            dash.scale = 1.0;
        }
    }
}

fn copy_dash_text(ltype: *mut c_void) {
    use crate::dynapi::get_field;
    let num = get_field::<u8>(ltype, "LTYPE", "numdashes").unwrap_or(0);
    let dashes =
        get_field::<*mut LtypeDash>(ltype, "LTYPE", "dashes").unwrap_or(std::ptr::null_mut());
    let area = get_field::<*mut u8>(ltype, "LTYPE", "strings_area").unwrap_or(std::ptr::null_mut());
    if num == 0 || dashes.is_null() || area.is_null() {
        return;
    }
    const AREA_BYTES: usize = 256;
    let mut cursor = 0usize;
    for index in 0..num {
        let dash = unsafe { &*dashes.add(usize::from(index)) };
        if dash.shape_flag & 2 == 0 || dash.text.is_null() {
            continue;
        }
        let bytes = unsafe { std::ffi::CStr::from_ptr(dash.text) }.to_bytes();
        if cursor + bytes.len() + 1 > AREA_BYTES {
            break;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), area.add(cursor), bytes.len());
            *area.add(cursor + bytes.len()) = 0;
        }
        cursor += bytes.len() + 1;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DashRecord {
    pub length: f64,
    pub shape: Option<LineTypeShape>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LtypeParseResult {
    pub name: String,
    pub dashes: Vec<f64>,
    pub shapes: Vec<Option<LineTypeShape>>,
    pub warnings: Vec<String>,
}

pub(crate) fn parse_ltype_dashes(name: &str, elements: &[(f64, u16)]) -> LtypeParseResult {
    let name = normalize_linetype_name(name);
    let mut dashes = Vec::with_capacity(elements.len());
    let mut complex_at = Vec::new();
    for (i, (length, shape_flag)) in elements.iter().copied().enumerate() {
        if shape_flag != 0 {
            complex_at.push(i);
        }
        dashes.push(length);
    }
    let mut warnings = Vec::new();
    if !complex_at.is_empty() {
        warnings.push(format!(
            "LTYPE '{name}': complex text/shape dashes at indices {complex_at:?} \
             (shape_flag != 0) are unsupported; using length spacing only"
        ));
    }
    LtypeParseResult {
        name,
        dashes,
        shapes: Vec::new(),
        warnings,
    }
}

pub(crate) fn parse_ltype_records(name: &str, records: &[DashRecord]) -> LtypeParseResult {
    let name = normalize_linetype_name(name);
    let mut dashes = Vec::with_capacity(records.len());
    let mut shapes = Vec::with_capacity(records.len());
    for record in records {
        dashes.push(record.length);
        match &record.shape {
            Some(shape) if shape.flag != 0 => shapes.push(Some(shape.clone())),
            _ => shapes.push(None),
        }
    }
    LtypeParseResult {
        name,
        dashes,
        shapes,
        warnings: Vec::new(),
    }
}

pub(crate) fn parse_ltype_dashes_r11(
    name: &str,
    inline: &[f64; 12],
    pattern_len: Option<f64>,
) -> LtypeParseResult {
    let mut elements = Vec::new();
    if let Some(limit) = pattern_len.filter(|p| p.is_finite() && *p > 1e-15) {
        let mut acc = 0.0;
        for &d in inline {
            if acc >= limit - 1e-12 && !elements.is_empty() {
                break;
            }
            elements.push((d, 0u16));
            acc += d.abs();
        }
    } else {
        let last_nonzero = inline.iter().rposition(|d| d.abs() > 1e-15);
        if let Some(end) = last_nonzero {
            for &d in &inline[..=end] {
                elements.push((d, 0u16));
            }
        }
    }
    parse_ltype_dashes(name, &elements)
}

pub(crate) fn linetype_from_flags(flags: Option<u8>, handle_name: Option<String>) -> String {
    match flags {
        Some(0) => "BYLAYER".into(),
        Some(1) => "BYBLOCK".into(),
        Some(2) => "CONTINUOUS".into(),
        _ => handle_name
            .map(|n| normalize_linetype_name(&n))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "BYLAYER".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn empty_dash(length: f64, shape_flag: u16) -> LtypeDash {
        LtypeDash {
            parent: std::ptr::null_mut(),
            length,
            complex_shapecode: 0,
            style: std::ptr::null_mut(),
            x_offset: 0.0,
            y_offset: 0.0,
            scale: 1.0,
            rotation: 0.0,
            shape_flag,
            text: std::ptr::null_mut(),
        }
    }

    #[test]
    fn ltype_dash_matches_libredwg_subclass_size() {
        let name = CString::new("LTYPE_dash").unwrap();
        let size = unsafe { libredwg_sys::dwg_dynapi_fields_size(name.as_ptr()) };
        assert_eq!(size as usize, std::mem::size_of::<LtypeDash>());
    }

    #[test]
    fn continuous_empty() {
        let parsed = parse_ltype_dashes("CONTINUOUS", &[]);
        assert!(parsed.dashes.is_empty());
        assert!(parsed.warnings.is_empty());
        assert_eq!(parsed.name, "CONTINUOUS");
    }

    #[test]
    fn dashed_pattern() {
        let parsed = parse_ltype_dashes("Dashed", &[(12.0, 0), (-6.0, 0)]);
        assert_eq!(parsed.dashes, vec![12.0, -6.0]);
        assert!(parsed.warnings.is_empty());
        assert_eq!(parsed.name, "DASHED");
    }

    #[test]
    fn center_pattern() {
        let parsed = parse_ltype_dashes("CENTER", &[(32.0, 0), (-6.0, 0), (4.0, 0), (-6.0, 0)]);
        assert_eq!(parsed.dashes, vec![32.0, -6.0, 4.0, -6.0]);
    }

    #[test]
    fn dot_pattern_preserves_zero() {
        let parsed = parse_ltype_dashes("DOT", &[(0.0, 0), (-6.0, 0)]);
        assert_eq!(parsed.dashes, vec![0.0, -6.0]);
    }

    #[test]
    fn complex_dash_warns_and_keeps_length() {
        let parsed = parse_ltype_dashes("GASLINE", &[(0.5, 0), (-0.2, 2), (-0.25, 0)]);
        assert_eq!(parsed.dashes, vec![0.5, -0.2, -0.25]);
        assert_eq!(parsed.warnings.len(), 1);
        assert!(parsed.warnings[0].contains("shape_flag"));
        assert!(parsed.warnings[0].contains("GASLINE"));
    }

    #[test]
    fn complex_record_keeps_shape_fields() {
        let parsed = parse_ltype_records(
            "FENCELINE1",
            &[
                DashRecord {
                    length: 0.25,
                    shape: None,
                },
                DashRecord {
                    length: -0.1,
                    shape: Some(LineTypeShape {
                        flag: 4,
                        shapecode: 130,
                        text: String::new(),
                        scale: 0.1,
                        rotation: 0.4,
                        x_offset: -0.05,
                        y_offset: 0.02,
                        style: "STANDARD".into(),
                        shape_file: String::new(),
                    }),
                },
            ],
        );
        assert!(parsed.warnings.is_empty());
        assert_eq!(parsed.dashes, vec![0.25, -0.1]);
        let shape = parsed.shapes[1].as_ref().expect("shape");
        assert_eq!(shape.flag, 4);
        assert_eq!(shape.shapecode, 130);
        assert_eq!(shape.style, "STANDARD");
    }

    #[test]
    fn r11_inline_stops_at_pattern_len() {
        let inline = [12.0, -6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let parsed = parse_ltype_dashes_r11("DASHED", &inline, Some(18.0));
        assert_eq!(parsed.dashes, vec![12.0, -6.0]);
    }

    #[test]
    fn r11_preserves_interior_dot() {
        let inline = [0.0, -6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let parsed = parse_ltype_dashes_r11("DOT", &inline, Some(6.0));
        assert_eq!(parsed.dashes, vec![0.0, -6.0]);
    }

    #[test]
    fn flags_map_to_semantic_names() {
        assert_eq!(
            linetype_from_flags(Some(0), Some("DASHED".into())),
            "BYLAYER"
        );
        assert_eq!(
            linetype_from_flags(Some(1), Some("CENTER".into())),
            "BYBLOCK"
        );
        assert_eq!(
            linetype_from_flags(Some(2), Some("HIDDEN".into())),
            "CONTINUOUS"
        );
        assert_eq!(
            linetype_from_flags(Some(3), Some("Dashed".into())),
            "DASHED"
        );
        assert_eq!(linetype_from_flags(None, Some("CENTER".into())), "CENTER");
        assert_eq!(linetype_from_flags(None, None), "BYLAYER");
    }

    #[test]
    fn struct_length_field_is_not_reinterpreted_as_raw_f64_array() {
        let dashes = [empty_dash(12.0, 0), empty_dash(-6.0, 0)];
        let as_f64 = unsafe { std::slice::from_raw_parts(dashes.as_ptr() as *const f64, 2) };
        assert_ne!(as_f64[0], 12.0, "parent pointer occupies the first 8 bytes");
        assert_eq!(dashes[0].length, 12.0);
        assert_eq!(dashes[1].length, -6.0);
    }
}
