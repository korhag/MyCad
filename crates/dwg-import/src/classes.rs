//! Classes LibreDWG left as UNKNOWN or PROXY.
//!
//! `dwg_get_class` and `dwg_object_get_type` are not wrapped by libredwg-sys.
//! The symbols are still in the linked LibreDWG library.

use std::ffi::{c_char, c_void};

use cad_core::ImportDiagnostics;

use crate::dynapi::{get_field, object_fixedtype};

// Prefix of `Dwg_Class` in LibreDWG `dwg.h`. Later fields are not read.
#[repr(C)]
#[allow(dead_code)] // field order locks Dwg_Class so dxfname stays at offset 24
struct DwgClassPrefix {
    number: u16,
    proxyflag: u16,
    _align_pointers: u32,
    appname: *mut c_char,
    cppname: *mut c_char,
    dxfname: *mut c_char,
    dxfname_u: *mut c_void,
    is_zombie: u8,
}

const _: () = {
    assert!(std::mem::offset_of!(DwgClassPrefix, dxfname) == 24);
    assert!(std::mem::offset_of!(DwgClassPrefix, is_zombie) == 40);
};

unsafe extern "C" {
    fn dwg_get_num_classes(dwg: *const libredwg_sys::Dwg_Data) -> u32;
    fn dwg_get_class(dwg: *const libredwg_sys::Dwg_Data, index: u32) -> *mut DwgClassPrefix;
    fn dwg_object_get_type(obj: *const libredwg_sys::Dwg_Object) -> i32;
}

const CLASS_NUMBER_BASE: u32 = 500;

pub(crate) fn record_unhandled_classes(
    dwg: *mut libredwg_sys::Dwg_Data,
    diagnostics: &mut ImportDiagnostics,
) {
    let class_count = unsafe { dwg_get_num_classes(dwg) };
    let num_objects = unsafe { libredwg_sys::dwg_get_num_objects(dwg) };
    for index in 0..num_objects {
        let obj = unsafe { libredwg_sys::dwg_get_object(dwg, index) };
        if obj.is_null() {
            continue;
        }
        let Some(class_index) = class_index_of(obj, class_count) else {
            continue;
        };
        let klass = unsafe { dwg_get_class(dwg, class_index) };
        if klass.is_null() {
            continue;
        }
        let name_ptr = unsafe { (*klass).dxfname };
        if let Some(name) = bounded_class_name(name_ptr) {
            diagnostics.record_unhandled_class(&name);
        }
    }
    diagnostics.finish_unhandled_classes();
}

fn class_index_of(obj: *mut libredwg_sys::Dwg_Object, class_count: u32) -> Option<u32> {
    let fixed = object_fixedtype(obj);
    let raw = if fixed == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_UNKNOWN_ENT
        || fixed == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_UNKNOWN_OBJ
    {
        let ty = unsafe { dwg_object_get_type(obj) };
        u32::try_from(ty).ok()?.checked_sub(CLASS_NUMBER_BASE)?
    } else if fixed == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_PROXY_ENTITY
        || fixed == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_PROXY_OBJECT
    {
        proxy_class_index(obj, fixed)?
    } else {
        return None;
    };
    if raw >= class_count {
        None
    } else {
        Some(raw)
    }
}

fn proxy_class_index(
    obj: *mut libredwg_sys::Dwg_Object,
    fixed: libredwg_sys::DWG_OBJECT_TYPE,
) -> Option<u32> {
    let entity = fixed == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_PROXY_ENTITY;
    let ptr = if entity {
        unsafe { libredwg_sys::uncad_object_entity_ptr(obj) }
    } else {
        unsafe { libredwg_sys::uncad_object_object_ptr(obj) }
    };
    if ptr.is_null() {
        return None;
    }
    let dxf = if entity {
        "PROXY_ENTITY"
    } else {
        "PROXY_OBJECT"
    };
    let class_id = get_field::<u32>(ptr, dxf, "class_id")?;
    class_id.checked_sub(CLASS_NUMBER_BASE)
}

fn bounded_class_name(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut bytes = Vec::new();
    for index in 0..64 {
        let byte = unsafe { ptr.add(index).read() } as u8;
        if byte == 0 {
            break;
        }
        if !byte.is_ascii() || !(byte.is_ascii_alphanumeric() || byte == b'_') {
            return None;
        }
        bytes.push(byte);
    }
    if bytes.len() < 2 {
        return None;
    }
    String::from_utf8(bytes).ok()
}
