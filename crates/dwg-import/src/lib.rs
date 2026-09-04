//! LibreDWG DWG/DXF import and DXF-interchange DWG save. Converts into cad-core
//! and then frees the C graph.

use std::ffi::CString;
use std::fs::File;
use std::io::Read;
use std::mem::MaybeUninit;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use cad_core::{Document, ImportDiagnostics};
use thiserror::Error;

mod convert;
mod dynapi;
mod export;
mod ltype;

pub use export::{
    convert_dxf_to_dwg, write_dwg, write_dwg_as, DwgOutputVersion, DwgWriteError, ExportError,
};

static LIBREDWG_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("path is not valid UTF-8 or contains a NUL byte")]
    InvalidPath,
    #[error("LibreDWG critical read error (code {0})")]
    Critical(i32),
    #[error("failed to open file: {0}")]
    Io(#[from] std::io::Error),
}

enum CadReadKind {
    Dwg,
    Dxf,
}

// ------------------------------------------------------------
// Function: import_dwg
// Purpose: Read a DWG via LibreDWG and return an isolated CAD document.
// ------------------------------------------------------------
pub fn import_dwg(path: impl AsRef<Path>) -> Result<Document, ImportError> {
    import_cad_file(path.as_ref(), CadReadKind::Dwg)
}

// ------------------------------------------------------------
// Function: import_dxf
// Purpose: Read a DXF via LibreDWG using the same lock and convert path.
// ------------------------------------------------------------
pub fn import_dxf(path: impl AsRef<Path>) -> Result<Document, ImportError> {
    import_cad_file(path.as_ref(), CadReadKind::Dxf)
}

fn import_cad_file(path: &Path, kind: CadReadKind) -> Result<Document, ImportError> {
    let path_str = path.to_str().ok_or(ImportError::InvalidPath)?;
    let c_path = CString::new(path_str).map_err(|_| ImportError::InvalidPath)?;
    let started = Instant::now();
    let magic = match kind {
        CadReadKind::Dwg => read_dwg_magic(path).unwrap_or_default(),
        CadReadKind::Dxf => String::new(),
    };

    let _guard = LIBREDWG_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let mut dwg: Box<libredwg_sys::Dwg_Data> =
        Box::new(unsafe { MaybeUninit::zeroed().assume_init() });
    let error = unsafe {
        match kind {
            CadReadKind::Dwg => libredwg_sys::dwg_read_file(c_path.as_ptr(), dwg.as_mut()),
            CadReadKind::Dxf => libredwg_sys::dxf_read_file(c_path.as_ptr(), dwg.as_mut()),
        }
    };
    #[allow(clippy::unnecessary_cast)]
    if error >= libredwg_sys::DWG_ERROR_DWG_ERR_CLASSESNOTFOUND as i32 {
        unsafe { libredwg_sys::dwg_free(dwg.as_mut()) };
        return Err(ImportError::Critical(error));
    }

    let mut diagnostics = ImportDiagnostics {
        dwg_version: match kind {
            CadReadKind::Dwg if magic.is_empty() => "unknown".into(),
            CadReadKind::Dwg => format!("{magic} ({})", ac_magic_label(&magic)),
            CadReadKind::Dxf => "DXF".into(),
        },
        ..Default::default()
    };
    if error != 0 {
        diagnostics.warnings.push(format!(
            "LibreDWG returned non-zero status {error} (non-critical)"
        ));
    }

    let mut document = unsafe { convert::convert_document(dwg.as_mut(), path, diagnostics) };
    unsafe { libredwg_sys::dwg_free(dwg.as_mut()) };
    document.diagnostics.import_time = started.elapsed();
    Ok(document)
}

fn read_dwg_magic(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut buf = [0u8; 6];
    file.read_exact(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn ac_magic_label(magic: &str) -> &'static str {
    match magic {
        "AC1014" => "R14",
        "AC1015" => "R2000",
        "AC1018" => "R2004",
        "AC1021" => "R2007",
        "AC1024" => "R2010",
        "AC1027" => "R2013",
        "AC1032" => "R2018",
        "AC1036" => "R2024",
        _ => "unknown revision",
    }
}

pub(crate) fn version_label(version: i32) -> String {
    format!("LibreDWG version enum {version}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ac1032_is_r2018() {
        assert_eq!(ac_magic_label("AC1032"), "R2018");
    }
}
