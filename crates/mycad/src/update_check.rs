//! Compare an installed EntoCAD with the workshop copy that installed it.
//!
//! The installer writes `{exe name}.source.json` beside the binary. A
//! `cargo run` build has no such file, so it stays quiet.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateOffer {
    pub current: String,
    pub available: String,
    pub update_command: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    Newer(UpdateOffer),
    Current { version: String },
    Unavailable { message: String },
}

#[derive(Debug, Deserialize)]
struct InstallSource {
    repo: PathBuf,
    update_command: PathBuf,
}

const NOT_INSTALLED: &str =
    "This window was not installed from the workshop, so there is nothing to compare.";

/// Look beside this process for the install record and compare versions.
pub fn check_installed() -> UpdateStatus {
    let Ok(exe) = std::env::current_exe() else {
        return UpdateStatus::Unavailable {
            message: NOT_INSTALLED.into(),
        };
    };
    let Some(source) = source_path_for_exe(&exe) else {
        return UpdateStatus::Unavailable {
            message: NOT_INSTALLED.into(),
        };
    };
    status_from_source(&source, env!("CARGO_PKG_VERSION"))
}

pub fn source_path_for_exe(exe: &Path) -> Option<PathBuf> {
    let stem = exe.file_stem()?.to_str()?;
    let dir = exe.parent()?;
    Some(dir.join(format!("{stem}.source.json")))
}

pub fn status_from_source(source: &Path, current: &str) -> UpdateStatus {
    let text = match std::fs::read_to_string(source) {
        Ok(text) => text.trim_start_matches('\u{feff}').to_string(),
        Err(_) => {
            return UpdateStatus::Unavailable {
                message: NOT_INSTALLED.into(),
            };
        }
    };
    let record: InstallSource = match serde_json::from_str(&text) {
        Ok(record) => record,
        Err(_) => {
            return UpdateStatus::Unavailable {
                message: "The install record beside EntoCAD could not be read.".into(),
            };
        }
    };
    let cargo = record.repo.join("Cargo.toml");
    let available = match std::fs::read_to_string(&cargo) {
        Ok(text) => match workspace_package_version(&text) {
            Some(version) => version,
            None => {
                return UpdateStatus::Unavailable {
                    message: "The workshop Cargo.toml has no package version.".into(),
                };
            }
        },
        Err(_) => {
            return UpdateStatus::Unavailable {
                message: "The workshop folder recorded at install is missing.".into(),
            };
        }
    };
    if !record.update_command.is_file() {
        return UpdateStatus::Unavailable {
            message: "The workshop installer recorded at install is missing.".into(),
        };
    }
    if version_is_newer(&available, current) {
        UpdateStatus::Newer(UpdateOffer {
            current: current.to_string(),
            available,
            update_command: record.update_command,
        })
    } else {
        UpdateStatus::Current {
            version: current.to_string(),
        }
    }
}

pub fn launch_update(command: &Path) -> Result<(), String> {
    let path = command
        .to_str()
        .filter(|path| !path.contains('"'))
        .ok_or_else(|| "The installer path cannot be started.".to_string())?;
    spawn_installer(path).map_err(|err| format!("Could not start the installer: {err}"))
}

#[cfg(windows)]
fn spawn_installer(path: &str) -> std::io::Result<()> {
    Command::new("cmd.exe")
        .arg("/C")
        .arg(format!("start \"EntoCAD update\" \"{path}\""))
        .spawn()?;
    Ok(())
}

#[cfg(not(windows))]
fn spawn_installer(path: &str) -> std::io::Result<()> {
    Command::new("bash").arg(path).spawn()?;
    Ok(())
}

/// `true` when `candidate` is a later `major.minor.patch` than `current`.
pub fn version_is_newer(candidate: &str, current: &str) -> bool {
    version_parts(candidate) > version_parts(current)
}

fn version_parts(version: &str) -> [u64; 3] {
    let mut parts = [0; 3];
    for (index, piece) in version.split('.').take(3).enumerate() {
        let digits: String = piece.chars().take_while(|c| c.is_ascii_digit()).collect();
        parts[index] = digits.parse().unwrap_or(0);
    }
    parts
}

fn workspace_package_version(cargo_toml: &str) -> Option<String> {
    let mut in_package = false;
    for line in cargo_toml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[workspace.package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("version") else {
            continue;
        };
        let rest = rest.trim_start();
        if !rest.starts_with('=') {
            continue;
        }
        let value = rest.trim_start_matches('=').trim();
        let value = value.strip_prefix('"')?.strip_suffix('"')?;
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_patch_or_minor_is_newer() {
        assert!(version_is_newer("0.53.1", "0.53.0"));
        assert!(version_is_newer("0.54.0", "0.53.0"));
        assert!(version_is_newer("0.10.0", "0.9.0"));
        assert!(!version_is_newer("0.53.0", "0.53.0"));
        assert!(!version_is_newer("0.46.1", "0.53.0"));
    }

    #[test]
    fn workspace_version_is_read_from_the_package_table() {
        let text =
            "[workspace]\nversion = \"9.9.9\"\n\n[workspace.package]\nversion = \"0.54.0\"\n";
        assert_eq!(workspace_package_version(text).as_deref(), Some("0.54.0"));
    }

    #[test]
    fn a_newer_workshop_offers_an_update() {
        let dir = std::env::temp_dir().join(format!("entocad-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = dir.join("repo");
        let install = dir.join("install");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(
            repo.join("Cargo.toml"),
            "[workspace.package]\nversion = \"0.54.0\"\n",
        )
        .unwrap();
        let command = install.join("Install EntoCAD.cmd");
        std::fs::write(&command, "echo update\n").unwrap();
        let source = install.join("EntoCAD.source.json");
        let record = serde_json::json!({
            "repo": repo,
            "update_command": command,
        });
        std::fs::write(&source, record.to_string()).unwrap();

        match status_from_source(&source, "0.53.0") {
            UpdateStatus::Newer(offer) => {
                assert_eq!(offer.available, "0.54.0");
                assert_eq!(offer.current, "0.53.0");
            }
            other => panic!("expected an offer, got {other:?}"),
        }
        assert!(matches!(
            status_from_source(&source, "0.54.0"),
            UpdateStatus::Current { .. }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
