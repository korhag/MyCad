//! Compare this running binary with the checkout that installed it.
//!
//! Checks and installer startup perform filesystem/process work and must run
//! on a worker. They never fetch or pull changes from the network.

use std::cmp::Ordering;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use semver::Version;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateOffer {
    pub current: String,
    pub available: String,
    pub available_revision: Option<String>,
    /// Includes the revision, so dismissing a release does not hide later pulls.
    pub update_key: String,
    pub update_command: PathBuf,
    install_prefix: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    install_dir: Option<PathBuf>,
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
    // Old manifests recorded a wrapper that has since moved. Resolve the
    // current platform script from repo instead of executing the stale value.
    #[serde(default, rename = "update_command")]
    _update_command: Option<PathBuf>,
    #[serde(default)]
    protocol_version: Option<u32>,
    #[serde(default)]
    install_prefix: Option<PathBuf>,
    #[serde(default)]
    data_dir: Option<PathBuf>,
    #[serde(default)]
    install_dir: Option<PathBuf>,
}

const NOT_INSTALLED: &str =
    "This copy has no installation record. Run the installer from scripts to enable update checks.";

/// Blocking: look beside this process for its installation record.
pub fn check_installed() -> UpdateStatus {
    let Some(source) = std::env::current_exe()
        .ok()
        .and_then(|exe| source_path_for_exe(&exe))
    else {
        return unavailable(NOT_INSTALLED);
    };
    status_from_source(
        &source,
        env!("CARGO_PKG_VERSION"),
        valid_revision(env!("ENTOCAD_BUILD_REVISION")),
    )
}

fn source_path_for_exe(exe: &Path) -> Option<PathBuf> {
    let mut name = exe.file_stem()?.to_os_string();
    name.push(".source.json");
    Some(exe.parent()?.join(name))
}

fn unavailable(message: impl Into<String>) -> UpdateStatus {
    UpdateStatus::Unavailable {
        message: message.into(),
    }
}

fn status_from_source(
    source: &Path,
    current: &str,
    current_revision: Option<&str>,
) -> UpdateStatus {
    let text = match std::fs::read_to_string(source) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return unavailable(NOT_INSTALLED)
        }
        Err(error) => {
            return unavailable(format!("Could not read the installation record: {error}"))
        }
    };
    let record: InstallSource = match serde_json::from_str(text.trim_start_matches('\u{feff}')) {
        Ok(record) => record,
        Err(_) => return unavailable("The installation record beside EntoCAD is invalid."),
    };
    if record.protocol_version.is_some_and(|version| version != 1) {
        return unavailable("This installation record uses a newer installer protocol. Run the installer from scripts manually.");
    }
    if !record.repo.is_absolute()
        || [
            &record.install_prefix,
            &record.data_dir,
            &record.install_dir,
        ]
        .into_iter()
        .flatten()
        .any(|path| !path.is_absolute())
    {
        return unavailable("The installation record must contain absolute folder paths.");
    }
    let cargo = match std::fs::read_to_string(record.repo.join("Cargo.toml")) {
        Ok(text) => text,
        Err(_) => {
            return unavailable(
                "The source checkout recorded at installation is missing or unreadable.",
            )
        }
    };
    let Some(available) = workspace_package_version(&cargo) else {
        return unavailable("The checkout Cargo.toml has no valid workspace package version.");
    };
    let (Ok(candidate_version), Ok(current_version)) =
        (Version::parse(&available), Version::parse(current))
    else {
        return unavailable("The installed or checkout version is not a valid semantic version.");
    };
    let available_revision = checkout_revision(&record.repo);
    if !update_is_available(
        &candidate_version,
        &current_version,
        available_revision.as_deref(),
        current_revision,
    ) {
        return UpdateStatus::Current {
            version: current.to_owned(),
        };
    }
    let update_command = record.repo.join(platform_installer());
    if !update_command.is_file() {
        return unavailable(format!(
            "The checkout installer is missing: {}. Update the checkout or run its installer manually.",
            update_command.display()
        ));
    }
    let update_key = match &available_revision {
        Some(revision) => format!("{available}@{revision}"),
        None => available.clone(),
    };
    UpdateStatus::Newer(UpdateOffer {
        current: current.to_owned(),
        available,
        available_revision,
        update_key,
        update_command,
        install_prefix: record.install_prefix,
        data_dir: record.data_dir,
        install_dir: record.install_dir,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InstallerPlatform {
    Linux,
    Macos,
    Windows,
}

fn current_platform() -> InstallerPlatform {
    if cfg!(windows) {
        InstallerPlatform::Windows
    } else if cfg!(target_os = "macos") {
        InstallerPlatform::Macos
    } else {
        InstallerPlatform::Linux
    }
}

fn platform_installer() -> &'static str {
    match current_platform() {
        InstallerPlatform::Windows => "scripts/install-windows.ps1",
        InstallerPlatform::Macos => "scripts/install-macos.sh",
        InstallerPlatform::Linux => "scripts/install-linux.sh",
    }
}

fn valid_revision(value: &str) -> Option<&str> {
    (matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then_some(value)
}

fn checkout_revision(repo: &Path) -> Option<String> {
    let mut command = Command::new("git");
    command
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(repo);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW for the background check.
    }
    let output = command.output().ok()?;
    output.status.success().then_some(())?;
    let revision = String::from_utf8(output.stdout).ok()?;
    valid_revision(revision.trim()).map(str::to_owned)
}

fn update_is_available(
    candidate: &Version,
    current: &Version,
    candidate_revision: Option<&str>,
    current_revision: Option<&str>,
) -> bool {
    // Build metadata does not affect SemVer precedence. A commit change is
    // offered only with an identity baked into this running executable; the
    // manifest may already describe a replacement installed by another window.
    match candidate.cmp_precedence(current) {
        Ordering::Greater => true,
        Ordering::Less => false,
        Ordering::Equal => matches!(
            (candidate_revision.and_then(valid_revision), current_revision.and_then(valid_revision)),
            (Some(candidate), Some(current)) if !candidate.eq_ignore_ascii_case(current)
        ),
    }
}

/// Blocking: start an independent installer, then let the UI close cleanly.
/// The installer waits for this process to exit before replacing the binary.
/// The returned file is the persistent installer log, including on failure.
pub fn launch_update(offer: &UpdateOffer, reopen: Option<&Path>) -> Result<PathBuf, String> {
    if !offer.update_command.is_absolute() || !offer.update_command.is_file() {
        return Err("The checkout installer is missing. Check for updates again.".into());
    }
    let reopen = absolute_reopen_path(reopen)?;
    let log_dir = update_log_dir();
    std::fs::create_dir_all(&log_dir)
        .map_err(|error| format!("Could not create the update log folder: {error}"))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("Could not create an update log timestamp: {error}"))?
        .as_nanos();
    let log = log_dir.join(format!("update-{timestamp}-{}.log", std::process::id()));
    let mut log_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&log)
        .map_err(|error| format!("Could not create the update log: {error}"))?;
    writeln!(log_file, "EntoCAD is installing from the local checkout.")
        .map_err(|error| format!("Could not write the update log: {error}"))?;
    let (program, args) = installer_invocation(
        offer,
        reopen.as_deref(),
        &log,
        std::process::id(),
        current_platform(),
    );
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut startup_name = log.as_os_str().to_os_string();
        startup_name.push(".startup.log");
        let startup_path = PathBuf::from(startup_name);
        let startup_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&startup_path)
            .map_err(|error| format!("Could not create the installer startup log: {error}"))?;
        let startup_error_file = startup_file
            .try_clone()
            .map_err(|error| format!("Could not open the installer startup log: {error}"))?;
        writeln!(
            log_file,
            "Installer startup output: {}",
            startup_path.display()
        )
        .map_err(|error| format!("Could not write the update log: {error}"))?;
        command
            .stdout(Stdio::from(startup_file))
            .stderr(Stdio::from(startup_error_file))
            // Separate from the GUI's process group; no terminal or pipe to
            // the exiting application remains attached to the installer.
            .process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // PowerShell owns a visible console and its transcript. A new console
        // survives closure of the GUI without cmd.exe or command interpolation.
        command.creation_flags(0x0000_0010 | 0x0000_0200); // NEW_CONSOLE | NEW_PROCESS_GROUP
    }
    command.spawn().map_err(|error| {
        format!(
            "Could not start the installer: {error}. Update log: {}",
            log.display()
        )
    })?;
    Ok(log)
}

fn absolute_reopen_path(reopen: Option<&Path>) -> Result<Option<PathBuf>, String> {
    // CLI opens may retain a relative source path. Resolve it while the old
    // process still owns that working directory; the installer may change it.
    reopen
        .map(std::path::absolute)
        .transpose()
        .map_err(|error| format!("Could not resolve the drawing path before updating: {error}"))
}

fn installer_invocation(
    offer: &UpdateOffer,
    reopen: Option<&Path>,
    log: &Path,
    process_id: u32,
    platform: InstallerPlatform,
) -> (OsString, Vec<OsString>) {
    let windows = platform == InstallerPlatform::Windows;
    let mut args = Vec::new();
    let program = if windows {
        args.extend(
            [
                "-NoLogo",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ]
            .map(OsString::from),
        );
        OsString::from("powershell.exe")
    } else {
        OsString::from("/bin/bash")
    };
    args.push(offer.update_command.as_os_str().to_owned());
    args.push(OsString::from(if windows {
        "-WaitPid"
    } else {
        "--wait-pid"
    }));
    args.push(OsString::from(process_id.to_string()));
    args.push(OsString::from(if windows { "-Launch" } else { "--launch" }));
    if let Some(path) = reopen {
        args.push(OsString::from(if windows { "-Reopen" } else { "--reopen" }));
        args.push(path.as_os_str().to_owned());
    }
    args.push(OsString::from(if windows {
        "-LogPath"
    } else {
        "--log-file"
    }));
    args.push(log.as_os_str().to_owned());
    let install_paths: &[(&str, &Option<PathBuf>)] = match platform {
        InstallerPlatform::Linux => &[
            ("--prefix", &offer.install_prefix),
            ("--data-dir", &offer.data_dir),
        ],
        InstallerPlatform::Macos => &[("--install-dir", &offer.install_dir)],
        InstallerPlatform::Windows => &[],
    };
    for &(flag, path) in install_paths {
        if let Some(path) = path {
            args.push(OsString::from(flag));
            args.push(path.as_os_str().to_owned());
        }
    }
    (program, args)
}

fn update_log_dir() -> PathBuf {
    let absolute_env = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    if cfg!(windows) {
        absolute_env("LOCALAPPDATA").map(|path| path.join("EntoCAD/logs"))
    } else if cfg!(target_os = "macos") {
        absolute_env("HOME").map(|path| path.join("Library/Logs/EntoCAD"))
    } else {
        absolute_env("XDG_STATE_HOME")
            .map(|path| path.join("entocad"))
            .or_else(|| absolute_env("HOME").map(|path| path.join(".local/state/entocad")))
    }
    .unwrap_or_else(|| std::env::temp_dir().join("entocad-updates"))
}

fn workspace_package_version(cargo_toml: &str) -> Option<String> {
    let mut in_package = false;
    for line in cargo_toml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed.split('#').next()?.trim() == "[workspace.package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        if key.trim() != "version" {
            continue;
        }
        let value = value.trim();
        let quote = value.chars().next()?;
        if !matches!(quote, '\'' | '"') {
            return None;
        }
        let (version, remainder) = value.get(1..)?.split_once(quote)?;
        if !remainder.trim().is_empty() && !remainder.trim().starts_with('#') {
            return None;
        }
        return Version::parse(version).ok().map(|_| version.to_owned());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    const OLD_REVISION: &str = "1111111111111111111111111111111111111111";
    const NEW_REVISION: &str = "2222222222222222222222222222222222222222";

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let sequence = SEQUENCE.fetch_add(1, AtomicOrdering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("entocad-update-{}-{sequence}", std::process::id()));
            std::fs::create_dir_all(path.join("repo/scripts")).unwrap();
            std::fs::create_dir_all(path.join("install")).unwrap();
            std::fs::write(
                path.join("repo/Cargo.toml"),
                "[workspace.package]\nversion = \"0.60.0\"\n",
            )
            .unwrap();
            std::fs::write(
                path.join("repo").join(platform_installer()),
                "installer test fixture",
            )
            .unwrap();
            Self(path)
        }
        fn source(&self) -> PathBuf {
            self.0.join("install/EntoCAD.source.json")
        }
        fn write_record(&self, extra: serde_json::Value) {
            let mut record = serde_json::json!({
                "repo": self.0.join("repo"),
                "update_command": self.0.join("removed/Install EntoCAD.cmd"),
            });
            record
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            std::fs::write(self.source(), format!("\u{feff}{record}")).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn newer(
        candidate: &str,
        current: &str,
        candidate_revision: Option<&str>,
        current_revision: Option<&str>,
    ) -> bool {
        update_is_available(
            &Version::parse(candidate).unwrap(),
            &Version::parse(current).unwrap(),
            candidate_revision,
            current_revision,
        )
    }
    #[test]
    fn semantic_versions_handle_prereleases_and_metadata() {
        assert!(newer("0.60.0", "0.59.9", None, None));
        assert!(newer("0.60.0", "0.60.0-rc.2", None, None));
        assert!(newer("0.60.0-rc.10", "0.60.0-rc.2", None, None));
        assert!(!newer("0.60.0-rc.1", "0.60.0", None, None));
        assert!(!newer("0.60.0+new", "0.60.0+old", None, None));
    }
    #[test]
    fn revision_updates_require_a_known_running_binary_and_never_downgrade_versions() {
        assert!(newer(
            "0.60.0",
            "0.60.0",
            Some(NEW_REVISION),
            Some(OLD_REVISION)
        ));
        assert!(!newer("0.60.0", "0.60.0", Some(NEW_REVISION), None));
        assert!(!newer(
            "0.60.0",
            "0.60.0",
            Some(NEW_REVISION),
            Some("invalid")
        ));
        assert!(!newer(
            "0.60.0",
            "0.60.0",
            Some(NEW_REVISION),
            Some(NEW_REVISION)
        ));
        assert!(!newer(
            "0.59.0",
            "0.60.0",
            Some(NEW_REVISION),
            Some(OLD_REVISION)
        ));
    }
    #[test]
    fn workspace_version_supports_comments_and_rejects_malformed_values() {
        assert_eq!(workspace_package_version("[package]\nversion=\"9.0.0\"\n[workspace.package] # release\nversion = '0.60.0-rc.1+build' # comment\n").as_deref(), Some("0.60.0-rc.1+build"));
        for version in ["nope", "1.2", "01.2.3", "1.2.3-01", "1.2.3+"] {
            assert!(workspace_package_version(&format!(
                "[workspace.package]\nversion=\"{version}\"\n"
            ))
            .is_none());
        }
        assert!(
            workspace_package_version("[workspace.package]\nversion=\"0.60.0\" garbage").is_none()
        );
    }
    #[test]
    fn old_bom_record_migrates_to_canonical_script_and_preserves_custom_install_paths() {
        let fixture = Fixture::new();
        let prefix = fixture.0.join("custom prefix");
        fixture.write_record(serde_json::json!({"install_prefix": prefix}));
        let UpdateStatus::Newer(offer) = status_from_source(&fixture.source(), "0.59.0", None)
        else {
            panic!("expected update")
        };
        assert_eq!(
            offer.update_command,
            fixture.0.join("repo").join(platform_installer())
        );
        assert_eq!(offer.install_prefix, Some(prefix));
        assert_eq!(offer.current, "0.59.0");
        assert_eq!(offer.available, "0.60.0");
    }
    #[test]
    fn missing_malformed_and_future_records_report_unavailable() {
        let fixture = Fixture::new();
        assert!(matches!(
            status_from_source(&fixture.source(), "0.59.0", None),
            UpdateStatus::Unavailable { .. }
        ));
        std::fs::write(fixture.source(), "not json").unwrap();
        assert!(matches!(
            status_from_source(&fixture.source(), "0.59.0", None),
            UpdateStatus::Unavailable { .. }
        ));
        fixture.write_record(serde_json::json!({"protocol_version": 2}));
        assert!(matches!(
            status_from_source(&fixture.source(), "0.59.0", None),
            UpdateStatus::Unavailable { .. }
        ));
        fixture.write_record(serde_json::json!({"repo": "relative"}));
        assert!(matches!(
            status_from_source(&fixture.source(), "0.59.0", None),
            UpdateStatus::Unavailable { .. }
        ));
        fixture.write_record(serde_json::json!({}));
        std::fs::remove_file(fixture.0.join("repo").join(platform_installer())).unwrap();
        assert!(matches!(
            status_from_source(&fixture.source(), "0.59.0", None),
            UpdateStatus::Unavailable { .. }
        ));
    }
    #[test]
    fn installed_metadata_cannot_replace_the_running_binary_version() {
        let fixture = Fixture::new();
        fixture.write_record(serde_json::json!({"protocol_version": 1, "installed_version": "0.60.0", "installed_commit": NEW_REVISION}));
        assert!(matches!(
            status_from_source(&fixture.source(), "0.59.0", None),
            UpdateStatus::Newer(_)
        ));
        assert!(matches!(
            status_from_source(&fixture.source(), "0.60.0", None),
            UpdateStatus::Current { .. }
        ));
        assert!(matches!(
            status_from_source(&fixture.source(), "invalid", None),
            UpdateStatus::Unavailable { .. }
        ));
    }
    #[test]
    fn invocation_keeps_shell_metacharacters_as_literal_arguments() {
        let mut offer = UpdateOffer {
            current: "0.59.0".into(),
            available: "0.60.0".into(),
            available_revision: None,
            update_key: "0.60.0".into(),
            update_command: PathBuf::from("/checkout with spaces/$x; & 'quotes'/install.sh"),
            install_prefix: Some(PathBuf::from("/install folder")),
            data_dir: None,
            install_dir: None,
        };
        let reopen = Path::new("/drawings/a $(touch unsafe) & \"quotes\".dwg");
        let log = Path::new("/logs/install log.txt");
        let (program, args) =
            installer_invocation(&offer, Some(reopen), log, 1234, InstallerPlatform::Linux);
        assert_eq!(program, "/bin/bash");
        let expected: Vec<OsString> = vec![
            offer.update_command.as_os_str(),
            "--wait-pid".as_ref(),
            "1234".as_ref(),
            "--launch".as_ref(),
            "--reopen".as_ref(),
            reopen.as_os_str(),
            "--log-file".as_ref(),
            log.as_os_str(),
            "--prefix".as_ref(),
            "/install folder".as_ref(),
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        assert_eq!(args, expected);
        let (program, args) =
            installer_invocation(&offer, Some(reopen), log, 1234, InstallerPlatform::Windows);
        assert_eq!(program, "powershell.exe");
        assert_eq!(args[4], "-File");
        assert_eq!(args[5], offer.update_command.as_os_str());
        assert_eq!(args[6], "-WaitPid");
        assert_eq!(args[7], "1234");
        assert_eq!(args[8], "-Launch");
        assert_eq!(args[9], "-Reopen");
        assert_eq!(args[10], reopen.as_os_str());
        assert_eq!(args[11], "-LogPath");
        assert_eq!(args[12], log.as_os_str());
        assert_eq!(args.len(), 13);
        offer.install_dir = Some(PathBuf::from("/Applications folder"));
        let (_, args) = installer_invocation(&offer, None, log, 1234, InstallerPlatform::Macos);
        assert!(args.contains(&OsString::from("--install-dir")));
        assert!(args.contains(&OsString::from("/Applications folder")));
        assert!(!args.contains(&OsString::from("--prefix")));
        assert!(!args.contains(&OsString::from("--reopen")));
        let (_, args) = installer_invocation(&offer, None, log, 1234, InstallerPlatform::Linux);
        assert!(!args.contains(&OsString::from("--install-dir")));
    }
    #[test]
    fn relative_cli_drawings_are_reopened_from_an_absolute_path() {
        let relative = Path::new("drawings/a file with spaces.dwg");
        let expected = std::env::current_dir().unwrap().join(relative);
        assert_eq!(
            absolute_reopen_path(Some(relative)).unwrap(),
            Some(expected.clone())
        );
        assert_eq!(
            absolute_reopen_path(Some(&expected)).unwrap(),
            Some(expected)
        );
        assert_eq!(absolute_reopen_path(None).unwrap(), None);
    }

    #[test]
    fn source_record_uses_executable_stem_on_each_platform() {
        assert_eq!(
            source_path_for_exe(Path::new(
                "/Applications/EntoCAD.app/Contents/MacOS/EntoCAD"
            )),
            Some(PathBuf::from(
                "/Applications/EntoCAD.app/Contents/MacOS/EntoCAD.source.json"
            ))
        );
        assert_eq!(
            source_path_for_exe(Path::new("/install/EntoCAD.exe")),
            Some(PathBuf::from("/install/EntoCAD.source.json"))
        );
    }
}
