use std::path::PathBuf;

fn reference_dwg() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-data/KD-1413-260825 Assir Poultry Internal Logistics.dwg")
}

#[test]
fn reference_dwg_imports_without_panic() {
    let path = reference_dwg();
    assert!(path.is_file(), "missing acceptance drawing at {}", path.display());
    let doc = dwg_import::import_dwg(&path).expect("LibreDWG should read the reference DWG");
    assert!(
        !doc.model_space.is_empty() || !doc.blocks.is_empty(),
        "expected model-space entities or blocks"
    );
    assert!(
        doc.diagnostics.extents.is_some(),
        "expected drawing extents after import"
    );
    assert!(
        doc.diagnostics.entity_total() > 0 || doc.diagnostics.unsupported_total() > 0,
        "import produced no entity accounting"
    );
}
