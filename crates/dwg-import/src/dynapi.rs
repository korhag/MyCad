//! Safe-ish LibreDWG dynapi field accessors.
//!
//! All calls must be serialized by the importer lock; LibreDWG is not
//! reentrant.

use std::ffi::{c_void, CStr, CString};
use std::mem::MaybeUninit;

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Point3D {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Point2D {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplineControlPoint {
    pub parent: *mut c_void,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub w: f64,
}

impl Default for SplineControlPoint {
    fn default() -> Self {
        Self {
            parent: std::ptr::null_mut(),
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 0.0,
        }
    }
}

#[allow(clippy::unnecessary_operation, clippy::identity_op)]
const _: () = {
    ["Size of SplineControlPoint"][std::mem::size_of::<SplineControlPoint>() - 40];
};

fn field_write_size_matches<T>(
    f: &libredwg_sys::Dwg_DYNAPI_field,
    _dxfname: &str,
    _field: &str,
) -> bool {
    let write_size = if f.is_malloc() != 0 {
        std::mem::size_of::<*const c_void>()
    } else {
        f.size as usize
    };
    write_size == std::mem::size_of::<T>()
}

pub fn get_field<T: Copy>(entity: *mut c_void, dxfname: &str, field: &str) -> Option<T> {
    if entity.is_null() {
        return None;
    }
    let c_dxfname = CString::new(dxfname).ok()?;
    let c_field = CString::new(field).ok()?;
    let field_desc =
        unsafe { libredwg_sys::dwg_dynapi_entity_field(c_dxfname.as_ptr(), c_field.as_ptr()) };
    if field_desc.is_null() {
        return None;
    }
    if !field_write_size_matches::<T>(unsafe { &*field_desc }, dxfname, field) {
        return None;
    }
    let mut out = MaybeUninit::<T>::uninit();
    let mut fp: libredwg_sys::Dwg_DYNAPI_field = Default::default();
    let ok = unsafe {
        libredwg_sys::dwg_dynapi_entity_value(
            entity,
            c_dxfname.as_ptr(),
            c_field.as_ptr(),
            out.as_mut_ptr().cast::<c_void>(),
            &mut fp,
        )
    };
    if !ok {
        return None;
    }
    Some(unsafe { out.assume_init() })
}

pub fn get_common_field<T: Copy>(entity: *mut c_void, field: &str) -> Option<T> {
    if entity.is_null() {
        return None;
    }
    let c_field = CString::new(field).ok()?;
    let mut field_desc = unsafe { libredwg_sys::dwg_dynapi_common_entity_field(c_field.as_ptr()) };
    if field_desc.is_null() {
        field_desc = unsafe { libredwg_sys::dwg_dynapi_common_object_field(c_field.as_ptr()) };
    }
    if field_desc.is_null() {
        return None;
    }
    if !field_write_size_matches::<T>(unsafe { &*field_desc }, "<common>", field) {
        return None;
    }
    let mut out = MaybeUninit::<T>::uninit();
    let mut fp: libredwg_sys::Dwg_DYNAPI_field = Default::default();
    let ok = unsafe {
        libredwg_sys::dwg_dynapi_common_value(
            entity,
            c_field.as_ptr(),
            out.as_mut_ptr().cast::<c_void>(),
            &mut fp,
        )
    };
    if !ok {
        return None;
    }
    Some(unsafe { out.assume_init() })
}

pub fn get_header_field<T: Copy>(dwg: *mut libredwg_sys::Dwg_Data, field: &str) -> Option<T> {
    if dwg.is_null() {
        return None;
    }
    let c_field = CString::new(field).ok()?;
    let mut out = MaybeUninit::<T>::uninit();
    let mut fp: libredwg_sys::Dwg_DYNAPI_field = Default::default();
    let ok = unsafe {
        libredwg_sys::dwg_dynapi_header_value(
            dwg,
            c_field.as_ptr(),
            out.as_mut_ptr().cast::<c_void>(),
            &mut fp,
        )
    };
    if !ok {
        return None;
    }
    Some(unsafe { out.assume_init() })
}

pub fn resolve_handle_name(
    dwg: *mut libredwg_sys::Dwg_Data,
    handle: *mut libredwg_sys::Dwg_Object_Ref,
) -> Option<String> {
    if handle.is_null() {
        return None;
    }
    let mut alloced: std::os::raw::c_int = 0;
    let name_ptr = unsafe { libredwg_sys::dwg_dynapi_handle_name(dwg, handle, &mut alloced) };
    if name_ptr.is_null() {
        return None;
    }
    let owned = unsafe { CStr::from_ptr(name_ptr) }
        .to_string_lossy()
        .into_owned();
    if alloced != 0 {
        unsafe { libc::free(name_ptr.cast()) };
    }
    Some(owned)
}

pub fn get_utf8_field(entity: *mut c_void, dxfname: &str, field: &str) -> Option<String> {
    if entity.is_null() {
        return None;
    }
    let c_dxfname = CString::new(dxfname).ok()?;
    let c_field = CString::new(field).ok()?;
    let mut text_ptr: *mut std::os::raw::c_char = std::ptr::null_mut();
    let mut is_new: std::os::raw::c_int = 0;
    let ok = unsafe {
        libredwg_sys::dwg_dynapi_entity_utf8text(
            entity,
            c_dxfname.as_ptr(),
            c_field.as_ptr(),
            &mut text_ptr,
            &mut is_new,
            std::ptr::null_mut(),
        )
    };
    if !ok || text_ptr.is_null() {
        return None;
    }
    let owned = unsafe { CStr::from_ptr(text_ptr) }
        .to_string_lossy()
        .into_owned();
    if is_new != 0 {
        unsafe { libc::free(text_ptr.cast()) };
    }
    Some(owned)
}

pub fn get_array_field<C, T>(
    entity: *mut c_void,
    dxfname: &str,
    count_field: &str,
    array_field: &str,
) -> Vec<T>
where
    C: Copy + TryInto<usize>,
    T: Copy,
{
    let Some(count) = get_field::<C>(entity, dxfname, count_field) else {
        return Vec::new();
    };
    let Ok(count) = count.try_into() else {
        return Vec::new();
    };
    if count == 0 {
        return Vec::new();
    }
    let Some(ptr) = get_field::<*const T>(entity, dxfname, array_field) else {
        return Vec::new();
    };
    if ptr.is_null() {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(ptr, count) }.to_vec()
}

pub unsafe fn read_raw_array<T: Copy>(ptr: *const T, count: u32) -> Vec<T> {
    if ptr.is_null() || count == 0 {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(ptr, count as usize) }.to_vec()
}

pub fn object_dxfname(obj: *mut libredwg_sys::Dwg_Object) -> String {
    let ptr = unsafe { libredwg_sys::dwg_object_get_dxfname(obj) };
    if ptr.is_null() {
        return "UNKNOWN".to_string();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

pub fn object_fixedtype(obj: *mut libredwg_sys::Dwg_Object) -> libredwg_sys::DWG_OBJECT_TYPE {
    unsafe { libredwg_sys::dwg_object_get_fixedtype(obj) as libredwg_sys::DWG_OBJECT_TYPE }
}
