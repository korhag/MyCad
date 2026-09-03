//! LibreDWG LTYPE dash import. `Dwg_LTYPE_dash` is not exported by
//! libredwg-sys 0.1, so this crate mirrors the C layout privately.

use std::ffi::c_void;
use std::os::raw::c_char;

use cad_core::normalize_linetype_name;

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

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LtypeParseResult {
    pub name: String,
    pub dashes: Vec<f64>,
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
        warnings,
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
