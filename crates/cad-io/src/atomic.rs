//! Same-directory temp write, then replace the target. Never truncate first.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_SERIAL: AtomicU64 = AtomicU64::new(1);

// ------------------------------------------------------------
// Function: write_atomic
// Purpose: Persist bytes without destroying an existing target on failure.
// ------------------------------------------------------------
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = sibling_temp_path(path)?;
    if let Err(err) = write_and_sync(&tmp, bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    if let Err(err) = replace_atomic(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

fn write_and_sync(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

// ------------------------------------------------------------
// Function: sibling_temp_path
// Purpose: Unique temp path in the same directory as `target`.
// ------------------------------------------------------------
pub fn sibling_temp_path(target: &Path) -> io::Result<std::path::PathBuf> {
    let Some(name) = target.file_name() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path has no file name",
        ));
    };
    let dir = match target.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    Ok(sibling_temp_in_dir(dir, name))
}

// ------------------------------------------------------------
// Function: sibling_temp_with_extension
// Purpose: Unique temp beside `target` whose name ends with `extension`.
// ------------------------------------------------------------
pub fn sibling_temp_with_extension(
    target: &Path,
    extension: &str,
) -> io::Result<std::path::PathBuf> {
    let Some(name) = target.file_name() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path has no file name",
        ));
    };
    let dir = match target.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    Ok(unique_temp_with_extension(dir, name, extension))
}

// ------------------------------------------------------------
// Function: sibling_temp_in_dir
// Purpose: Unique temp path beside other files in `dir`.
// ------------------------------------------------------------
pub fn sibling_temp_in_dir(dir: &Path, name: &std::ffi::OsStr) -> std::path::PathBuf {
    unique_temp(dir, name)
}

// ------------------------------------------------------------
// Function: replace_atomic
// Purpose: Replace `to` with a completed file at `from` without truncating first.
// ------------------------------------------------------------
pub fn replace_atomic(from: &Path, to: &Path) -> io::Result<()> {
    replace_file(from, to)
}

fn unique_temp(dir: &Path, name: &std::ffi::OsStr) -> std::path::PathBuf {
    unique_temp_with_extension(dir, name, "tmp")
}

fn unique_temp_with_extension(
    dir: &Path,
    name: &std::ffi::OsStr,
    extension: &str,
) -> std::path::PathBuf {
    let serial = TEMP_SERIAL.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let mut file_name = std::ffi::OsString::from(".");
    file_name.push(name);
    file_name.push(format!(
        ".mycad-{}-{}-{serial}.{extension}",
        std::process::id(),
        nanos
    ));
    dir.join(file_name)
}

fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        replace_file_windows(from, to)
    }
    #[cfg(not(windows))]
    {
        fs::rename(from, to)
    }
}

#[cfg(windows)]
fn replace_file_windows(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(
            lp_existing_file_name: *const u16,
            lp_new_file_name: *const u16,
            dw_flags: u32,
        ) -> i32;
    }

    let from_wide: Vec<u16> = from
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let to_wide: Vec<u16> = to
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let ok = unsafe {
        MoveFileExW(
            from_wide.as_ptr(),
            to_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("mycad-atomic-{stamp}-{name}"))
    }

    #[test]
    fn overwrite_replaces_bytes_and_leaves_no_temp() {
        let path = temp_path("drawing.dxf");
        fs::write(&path, b"OLD DRAWING").expect("seed");
        write_atomic(&path, b"NEW DRAWING").expect("replace");
        assert_eq!(fs::read(&path).expect("read"), b"NEW DRAWING");
        let dir = path.parent().expect("dir");
        let prefix = format!(".{}", path.file_name().unwrap().to_string_lossy());
        let leftovers: Vec<_> = fs::read_dir(dir)
            .expect("list")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .filter(|name| {
                let name = name.to_string_lossy();
                name.starts_with(&prefix) && name.contains(".mycad-")
            })
            .collect();
        let _ = fs::remove_file(&path);
        assert!(leftovers.is_empty(), "leftover temps: {leftovers:?}");
    }

    #[test]
    fn interchange_temp_keeps_the_requested_extension() {
        let path = temp_path("plant.dwg");
        let dxf = sibling_temp_with_extension(&path, "dxf").expect("dxf temp");
        let dwg = sibling_temp_with_extension(&path, "dwg").expect("dwg temp");
        assert_eq!(dxf.extension().and_then(|ext| ext.to_str()), Some("dxf"));
        assert_eq!(dwg.extension().and_then(|ext| ext.to_str()), Some("dwg"));
        assert_ne!(dxf, dwg);
    }

    #[test]
    fn failed_create_does_not_turn_a_directory_into_a_file() {
        let as_dir = temp_path("not-a-file");
        fs::create_dir(&as_dir).expect("dir");
        assert!(write_atomic(&as_dir, b"NEW").is_err());
        assert!(as_dir.is_dir());
        let _ = fs::remove_dir(&as_dir);
    }
}
