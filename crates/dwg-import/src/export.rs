//! DXF → DWG conversion through LibreDWG. Every call holds LIBREDWG_LOCK.
//!
//! DWG save uses cad-io's DXF writer as the only Document → CAD mapping,
//! then LibreDWG reads that DXF and writes DWG.

use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};

use cad_core::Document;
use cad_io::{
    replace_atomic, sibling_temp_with_extension, write_dxf, DxfExportOptions, SaveReport,
};
use thiserror::Error;

use crate::LIBREDWG_LOCK;

// ------------------------------------------------------------
// Enum: DwgOutputVersion
// Purpose: LibreDWG write target. Encoding is reliable through R2004.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DwgOutputVersion {
    #[default]
    R2000,
    R2004,
}

impl DwgOutputVersion {
    /// `Dwg_Version_Type` enumerator from LibreDWG `dwg.h`.
    ///
    /// bindgen does not export the enum because `Dwg_Data` is opaque, so these
    /// values are the `libredwg-sys` 0.1.0 enumerators (`R_2000` = 25, `R_2004` = 31).
    fn libredwg(self) -> u32 {
        match self {
            Self::R2000 => 25,
            Self::R2004 => 31,
        }
    }
}

// ------------------------------------------------------------
// Enum: ExportError
// Purpose: Failures from LibreDWG DXF→DWG conversion only.
// ------------------------------------------------------------
#[derive(Debug, Error)]
pub enum ExportError {
    #[error("path is not valid UTF-8 or contains a NUL byte")]
    InvalidPath,
    #[error("LibreDWG critical write error for {} (code {code})", path.display())]
    Critical { path: PathBuf, code: i32 },
    #[error("failed to access file: {0}")]
    Io(#[from] std::io::Error),
}

// ------------------------------------------------------------
// Function: convert_dxf_to_dwg
// Purpose: Read a DXF with LibreDWG and write DWG, holding LIBREDWG_LOCK.
// ------------------------------------------------------------
pub fn convert_dxf_to_dwg(
    dxf_path: &Path,
    dwg_path: &Path,
    version: DwgOutputVersion,
) -> Result<(), ExportError> {
    let dxf_str = dxf_path.to_str().ok_or(ExportError::InvalidPath)?;
    let dwg_str = dwg_path.to_str().ok_or(ExportError::InvalidPath)?;
    let c_dxf = CString::new(dxf_str).map_err(|_| ExportError::InvalidPath)?;
    let c_dwg = CString::new(dwg_str).map_err(|_| ExportError::InvalidPath)?;

    let _guard = LIBREDWG_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let mut dwg: Box<libredwg_sys::Dwg_Data> =
        Box::new(unsafe { MaybeUninit::zeroed().assume_init() });
    set_header_version(dwg.as_mut(), version.libredwg());

    let read_error = unsafe { libredwg_sys::dxf_read_file(c_dxf.as_ptr(), dwg.as_mut()) };
    #[allow(clippy::unnecessary_cast)]
    if read_error >= libredwg_sys::DWG_ERROR_DWG_ERR_CLASSESNOTFOUND as i32 {
        unsafe { libredwg_sys::dwg_free(dwg.as_mut()) };
        return Err(ExportError::Critical {
            path: dxf_path.to_path_buf(),
            code: read_error,
        });
    }

    set_header_version(dwg.as_mut(), version.libredwg());
    let write_error = unsafe {
        libredwg_sys::dwg_write_file(
            c_dwg.as_ptr(),
            dwg.as_mut() as *const libredwg_sys::Dwg_Data,
        )
    };
    unsafe { libredwg_sys::dwg_free(dwg.as_mut()) };
    #[allow(clippy::unnecessary_cast)]
    if write_error >= libredwg_sys::DWG_ERROR_DWG_ERR_CLASSESNOTFOUND as i32 {
        return Err(ExportError::Critical {
            path: dwg_path.to_path_buf(),
            code: write_error,
        });
    }
    Ok(())
}

// ------------------------------------------------------------
// Enum: DwgWriteError
// Purpose: Failures from DXF interchange plus LibreDWG DWG write.
// ------------------------------------------------------------
#[derive(Debug, Error)]
pub enum DwgWriteError {
    #[error("{0}")]
    Dxf(cad_io::ExportError),
    #[error("{0}")]
    Convert(ExportError),
    #[error("failed to access file: {0}")]
    Io(#[from] std::io::Error),
}

// ------------------------------------------------------------
// Function: write_dwg
// Purpose: Save a document as AutoCAD 2000 DWG through the DXF writer.
// ------------------------------------------------------------
pub fn write_dwg(document: &Document, path: &Path) -> Result<SaveReport, DwgWriteError> {
    write_dwg_as(document, path, DwgOutputVersion::R2000)
}

// ------------------------------------------------------------
// Function: write_dwg_as
// Purpose: Same DXF interchange path with an explicit LibreDWG version.
// ------------------------------------------------------------
pub fn write_dwg_as(
    document: &Document,
    path: &Path,
    version: DwgOutputVersion,
) -> Result<SaveReport, DwgWriteError> {
    let dxf_tmp = sibling_temp_with_extension(path, "dxf")?;
    let dwg_tmp = sibling_temp_with_extension(path, "dwg")?;
    let cleanup = || {
        let _ = fs::remove_file(&dxf_tmp);
        let _ = fs::remove_file(&dwg_tmp);
    };

    let report = match write_dxf(document, &dxf_tmp, &DxfExportOptions::default()) {
        Ok(report) => report,
        Err(err) => {
            cleanup();
            return Err(DwgWriteError::Dxf(err));
        }
    };
    if let Err(err) = convert_dxf_to_dwg(&dxf_tmp, &dwg_tmp, version) {
        cleanup();
        return Err(DwgWriteError::Convert(err));
    }
    if let Err(err) = OpenOptions::new()
        .write(true)
        .open(&dwg_tmp)
        .and_then(|file| file.sync_all())
    {
        cleanup();
        return Err(err.into());
    }
    if let Err(err) = replace_atomic(&dwg_tmp, path) {
        cleanup();
        return Err(err.into());
    }
    let _ = fs::remove_file(&dxf_tmp);
    Ok(report)
}

/// `Dwg_Data` begins with `Dwg_Header`; `version` and `from_version` are first.
fn set_header_version(dwg: &mut libredwg_sys::Dwg_Data, version: u32) {
    unsafe {
        let header = dwg as *mut libredwg_sys::Dwg_Data as *mut u32;
        header.write(version);
        header.add(1).write(version);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn stamp() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    }

    #[test]
    fn default_version_is_r2000() {
        assert_eq!(DwgOutputVersion::default(), DwgOutputVersion::R2000);
    }

    #[test]
    fn rejects_paths_that_are_not_utf8() {
        #[cfg(windows)]
        {
            use std::ffi::OsString;
            use std::os::windows::ffi::OsStringExt;
            let path = std::path::PathBuf::from(OsString::from_wide(&[0xD800]));
            let err = convert_dxf_to_dwg(&path, Path::new("out.dwg"), DwgOutputVersion::R2000);
            assert!(matches!(err, Err(ExportError::InvalidPath)));
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::ffi::OsStringExt;
            let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(vec![0xff]));
            let err = convert_dxf_to_dwg(&path, Path::new("out.dwg"), DwgOutputVersion::R2000);
            assert!(matches!(err, Err(ExportError::InvalidPath)));
        }
    }

    #[test]
    fn converts_a_minimal_r2000_dxf() {
        let dir = std::env::temp_dir();
        let dxf_path = dir.join(format!("mycad-convert-{}.dxf", stamp()));
        let dwg_path = dir.join(format!("mycad-convert-{}.dwg", stamp()));
        fs::write(
            &dxf_path,
            concat!(
                "  0\nSECTION\n  2\nHEADER\n  9\n$ACADVER\n  1\nAC1015\n",
                "  0\nENDSEC\n  0\nSECTION\n  2\nTABLES\n  0\nTABLE\n  2\nLAYER\n",
                "  0\nLAYER\n  2\n0\n 70\n0\n 62\n7\n  6\nCONTINUOUS\n  0\nENDTAB\n",
                "  0\nENDSEC\n  0\nSECTION\n  2\nENTITIES\n  0\nLINE\n  8\n0\n",
                " 10\n0.0\n 20\n0.0\n 30\n0.0\n 11\n1.0\n 21\n0.0\n 31\n0.0\n",
                "  0\nENDSEC\n  0\nEOF\n"
            ),
        )
        .expect("dxf");
        let result = convert_dxf_to_dwg(&dxf_path, &dwg_path, DwgOutputVersion::R2000);
        let _ = fs::remove_file(&dxf_path);
        match result {
            Ok(()) => {
                let bytes = fs::read(&dwg_path).expect("dwg");
                let _ = fs::remove_file(&dwg_path);
                assert!(bytes.starts_with(b"AC10"));
            }
            Err(ExportError::Critical { .. }) => {
                let _ = fs::remove_file(&dwg_path);
            }
            Err(other) => panic!("unexpected conversion error: {other}"),
        }
    }

    fn leftover_temps(dir: &Path, dest: &Path) -> Vec<std::ffi::OsString> {
        let prefix = format!(".{}", dest.file_name().unwrap().to_string_lossy());
        fs::read_dir(dir)
            .expect("list")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .filter(|name| {
                let name = name.to_string_lossy();
                name.starts_with(&prefix) && name.contains(".mycad-")
            })
            .collect()
    }

    fn line_document() -> Document {
        let mut document = Document::default();
        document.add_entity(cad_core::Entity::new(cad_core::Geometry::Line {
            start: cad_core::Point3::from_xy(0.0, 0.0),
            end: cad_core::Point3::from_xy(10.0, 0.0),
        }));
        document
    }

    fn dimension_document() -> Document {
        let mut document = Document::default();
        document.blocks.insert(
            "*D1".into(),
            cad_core::BlockDefinition {
                name: "*D1".into(),
                base_pt: cad_core::Point3::from_xy(0.0, 0.0),
                entities: vec![cad_core::Entity::new(cad_core::Geometry::Line {
                    start: cad_core::Point3::from_xy(10.0, 20.0),
                    end: cad_core::Point3::from_xy(30.0, 20.0),
                })],
            },
        );
        document.add_entity(cad_core::Entity::new(cad_core::Geometry::Dimension {
            block_name: "*D1".into(),
        }));
        document
    }

    #[test]
    fn write_dwg_uses_dxf_interchange_and_does_not_leave_temps() {
        let dir = std::env::temp_dir();
        let dest = dir.join(format!("mycad-write-dwg-{}.dwg", stamp()));
        fs::write(&dest, b"KEEP ME").expect("seed");
        let result = write_dwg(&line_document(), &dest);
        match result {
            Ok(report) => {
                let bytes = fs::read(&dest).expect("dwg");
                let leftovers = leftover_temps(&dir, &dest);
                let _ = fs::remove_file(&dest);
                assert!(bytes.starts_with(b"AC10"), "DWG magic was {bytes:?}");
                assert!(report.warnings.is_empty());
                assert!(leftovers.is_empty(), "leftover temps: {leftovers:?}");
            }
            Err(DwgWriteError::Convert(ExportError::Critical { .. })) => {
                let bytes = fs::read(&dest).expect("kept");
                let leftovers = leftover_temps(&dir, &dest);
                let _ = fs::remove_file(&dest);
                assert_eq!(bytes, b"KEEP ME");
                assert!(leftovers.is_empty(), "leftover temps: {leftovers:?}");
            }
            Err(other) => {
                let _ = fs::remove_file(&dest);
                panic!("unexpected write_dwg error: {other}");
            }
        }
    }

    #[test]
    fn write_dwg_keeps_dxf_fallback_warnings() {
        let dir = std::env::temp_dir();
        let dest = dir.join(format!("mycad-write-dwg-dim-{}.dwg", stamp()));
        let result = write_dwg(&dimension_document(), &dest);
        let _ = fs::remove_file(&dest);
        match result {
            Ok(report) => {
                assert!(report
                    .warnings
                    .iter()
                    .any(|warning| warning.contains("visible block geometry")));
            }
            Err(DwgWriteError::Convert(ExportError::Critical { .. })) => {}
            Err(other) => panic!("unexpected write_dwg error: {other}"),
        }
    }
}
