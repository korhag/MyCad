//! Embeds the EntoCAD icon in the Windows executable.

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let icon = PathBuf::from(&manifest_dir).join("../../resources/icons/entocad.ico");
    println!("cargo:rerun-if-changed=build.rs");
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
