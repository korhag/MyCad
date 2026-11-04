//! Records the running binary's revision and embeds its Windows icon.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    record_build_revision();
    #[cfg(windows)]
    embed_windows_icon();
}

fn record_build_revision() {
    use std::path::PathBuf;
    use std::process::Command;

    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let git = |args: &[&str]| -> Option<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&manifest_dir)
            .output()
            .ok()?;
        output.status.success().then_some(())?;
        Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
    };

    // HEAD alone does not change on a pull of the current branch. Track its
    // loose ref and packed-refs too, including linked worktree Git directories.
    for relative in ["HEAD", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--git-path", relative]) {
            let path = manifest_dir.join(path);
            if path.exists() {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git(&["rev-parse", "--git-path", &reference]) {
            let mut path = manifest_dir.join(path);
            // Packed branches have no loose ref yet. Watch its nearest
            // existing directory so a pull creating that ref triggers a build.
            while !path.exists() && path.pop() {}
            if path.exists() {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
    let revision = git(&["rev-parse", "--verify", "HEAD"])
        .filter(|value| {
            matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .unwrap_or_default();
    // Archives and machines without Git still build and compare release versions.
    println!("cargo:rustc-env=ENTOCAD_BUILD_REVISION={revision}");
}

#[cfg(windows)]
fn embed_windows_icon() {
    use std::env;
    use std::path::PathBuf;

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let icon = PathBuf::from(&manifest_dir).join("../../resources/icons/entocad.ico");
    println!("cargo:rerun-if-changed={}", icon.display());

    if env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("windows") {
        return;
    }

    let icon = icon.canonicalize().unwrap_or(icon);
    let icon = icon
        .to_str()
        .expect("EntoCAD icon path is not valid Unicode");
    let mut res = winresource::WindowsResource::new();
    res.set_icon(icon);
    res.set("ProductName", "EntoCAD");
    res.set("FileDescription", "EntoCAD");
    if let Err(err) = res.compile() {
        panic!("failed to embed the EntoCAD icon: {err}");
    }
}
