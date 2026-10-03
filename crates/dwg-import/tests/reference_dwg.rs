use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn reference_dwg() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let samples = root.join("samples/KD-1413-260825 Assir Poultry Internal Logistics.dwg");
    if samples.is_file() {
        return samples;
    }
    root.join("test-data/KD-1413-260825 Assir Poultry Internal Logistics.dwg")
}

#[test]
fn reference_dwg_imports_without_panic() {
    let path = reference_dwg();
    assert!(
        path.is_file(),
        "missing acceptance drawing at {}",
        path.display()
    );
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
    assert!(doc.ltscale > 0.0 && doc.ltscale.is_finite());
    assert!(
        doc.layers.contains_key(&doc.current_layer),
        "current layer should exist in the layer table"
    );
    assert!(
        !doc.layers
            .get(&doc.current_layer)
            .expect("current layer")
            .frozen,
        "current layer must not be frozen"
    );
    assert!(
        doc.model_space
            .iter()
            .chain(doc.blocks.values().flat_map(|block| block.entities.iter()))
            .all(|entity| entity.id.is_assigned()),
        "imported entities should receive stable IDs"
    );
    assert_eq!(
        doc.diagnostics.unsupported_total(),
        0,
        "reference drawing should import with no unsupported entities: {:?}",
        doc.diagnostics.unsupported_counts
    );
    let status = doc
        .diagnostics
        .warnings
        .iter()
        .find(|warning| warning.contains("LibreDWG read reported"));
    let status = status.expect("named LibreDWG status warning");
    assert!(
        status.contains("unhandled class"),
        "status 68 should name the unhandled-class bit: {status}"
    );
    assert!(
        status.contains("value out of bounds"),
        "status 68 should name the bounds bit: {status}"
    );
    assert!(
        !doc.diagnostics.has_save_loss(),
        "class notes must not open the save dialog: {:?}",
        doc.diagnostics.lossy_notes
    );
    assert!(
        !doc.diagnostics.unhandled_classes.is_empty(),
        "expected classes LibreDWG could not read"
    );
    assert!(
        !doc.diagnostics.unhandled_classes.contains_key("SUN"),
        "SUN decoded and must not be listed as unread: {:?}",
        doc.diagnostics.unhandled_classes
    );
    assert!(!doc.linetypes.is_empty(), "expected LTYPE table entries");
    let has = |name: &str| doc.linetypes.keys().any(|k| k.eq_ignore_ascii_case(name));
    assert!(has("CONTINUOUS"), "missing CONTINUOUS linetype");
    let patterned = doc.linetypes.values().any(|lt| !lt.is_continuous());
    assert!(patterned, "expected at least one non-continuous linetype");
    let non_continuous_layers = doc
        .layers
        .values()
        .filter(|l| !l.linetype.eq_ignore_ascii_case("CONTINUOUS"))
        .count();
    assert!(
        non_continuous_layers > 0,
        "layer linetype handles were not resolved (all layers CONTINUOUS)"
    );
}

#[test]
fn reference_complex_linetypes_roundtrip_dxf() {
    let path = reference_dwg();
    assert!(path.is_file(), "missing acceptance drawing");
    let original = dwg_import::import_dwg(&path).expect("import reference");
    assert_eq!(original.diagnostics.unsupported_total(), 0);
    let mut small = cad_core::Document::default();
    small.ensure_layer_zero();
    for name in ["FENCELINE1", "AMZIGZAG"] {
        let source = original
            .linetypes
            .values()
            .find(|linetype| linetype.name.eq_ignore_ascii_case(name))
            .unwrap_or_else(|| panic!("missing {name}"));
        assert!(
            source.shapes.iter().any(|shape| shape.is_some()),
            "{name} lost its complex dash on import: {source:?}"
        );
        small.linetypes.insert(source.name.clone(), source.clone());
    }
    for style in original.text_styles.values() {
        small.text_styles.insert(style.name.clone(), style.clone());
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let dxf = std::env::temp_dir().join(format!("mycad-ltype-{stamp}.dxf"));
    cad_io::write_dxf(&small, &dxf, &cad_io::DxfExportOptions::default()).expect("write dxf");
    let back = dwg_import::import_dxf(&dxf).expect("reimport dxf");
    let _ = fs::remove_file(&dxf);
    assert_linetype_shapes(&small, &back, "dxf");
    let dwg = std::env::temp_dir().join(format!("mycad-ltype-{stamp}.dwg"));
    dwg_import::write_dwg(&small, &dwg).expect("write dwg");
    let from_dwg = dwg_import::import_dwg(&dwg).expect("reimport dwg");
    let _ = fs::remove_file(&dwg);
    assert_linetype_shapes(&small, &from_dwg, "dwg");
    let fence = from_dwg
        .linetypes
        .values()
        .find(|linetype| linetype.name.eq_ignore_ascii_case("FENCELINE1"))
        .expect("FENCELINE1");
    assert!(
        fence.shapes.iter().flatten().any(|shape| shape
            .shape_file
            .to_ascii_lowercase()
            .ends_with("ltypeshp.shx")),
        "FENCELINE1 lost its shape file: {fence:?}"
    );
    let zigzag = from_dwg
        .linetypes
        .values()
        .find(|linetype| linetype.name.eq_ignore_ascii_case("AMZIGZAG"))
        .expect("AMZIGZAG");
    assert!(
        zigzag.shapes.iter().flatten().any(|shape| {
            !shape.text.is_empty()
                || shape
                    .shape_file
                    .to_ascii_lowercase()
                    .ends_with("genltshp.shx")
        }),
        "AMZIGZAG lost its text or shape file: {zigzag:?}"
    );
}

fn assert_linetype_shapes(expected: &cad_core::Document, actual: &cad_core::Document, label: &str) {
    let mismatches =
        cad_core::compare_documents(expected, actual, cad_core::CompareTol::ROUND_TRIP);
    let linetype_misses: Vec<_> = mismatches
        .iter()
        .filter(|miss| miss.path.contains("linetypes."))
        .collect();
    assert!(
        linetype_misses.is_empty(),
        "complex linetypes did not round-trip through {label}: {linetype_misses:?}\nimport warnings: {:?}",
        actual.diagnostics.warnings
    );
}

#[test]
fn reference_dwg_save_reimports() {
    let path = reference_dwg();
    assert!(
        path.is_file(),
        "missing acceptance drawing at {}",
        path.display()
    );
    let original = dwg_import::import_dwg(&path).expect("LibreDWG should read the reference DWG");
    let unsupported = format!("{:?}", original.diagnostics.unsupported_counts);
    let layouts = original.layouts.len();
    assert!(
        original.diagnostics.entity_total() > 0,
        "reference drawing imported no entities\nunsupported: {unsupported}\nlayouts: {layouts}"
    );

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let dir = std::env::temp_dir();
    let dxf = dir.join(format!("mycad-reference-{stamp}.dxf"));
    let dwg = dir.join(format!("mycad-reference-{stamp}.dwg"));
    let dxf_report = match cad_io::write_dxf(&original, &dxf, &cad_io::DxfExportOptions::default())
    {
        Ok(report) => report,
        Err(err) => {
            let _ = fs::remove_file(&dxf);
            panic!("DXF save of the reference drawing failed: {err}\nunsupported: {unsupported}\nlayouts: {layouts}");
        }
    };
    let from_dxf = match dwg_import::import_dxf(&dxf) {
        Ok(document) => document,
        Err(err) => {
            let _ = fs::remove_file(&dxf);
            panic!("reimport of the saved DXF failed: {err}\nwarnings: {:?}\nunsupported: {unsupported}\nlayouts: {layouts}", dxf_report.warnings);
        }
    };
    assert!(
        !from_dxf.model_space.is_empty(),
        "saved DXF reimported with an empty model space\nwarnings: {:?}\nunsupported: {unsupported}\nlayouts: {layouts}",
        dxf_report.warnings
    );

    let dwg_report = match dwg_import::write_dwg(&original, &dwg) {
        Ok(report) => report,
        Err(err) => {
            let _ = fs::remove_file(&dxf);
            let _ = fs::remove_file(&dwg);
            panic!("DWG save of the reference drawing failed: {err}\nunsupported: {unsupported}\nlayouts: {layouts}");
        }
    };
    let bytes = fs::read(&dwg).unwrap_or_default();
    assert!(
        bytes.starts_with(b"AC10"),
        "saved DWG magic was {bytes:?}\nwarnings: {:?}",
        dwg_report.warnings
    );
    let from_dwg = match dwg_import::import_dwg(&dwg) {
        Ok(document) => document,
        Err(err) => {
            let _ = fs::remove_file(&dxf);
            let _ = fs::remove_file(&dwg);
            panic!("reimport of the saved DWG failed: {err}\nwarnings: {:?}\nunsupported: {unsupported}\nlayouts: {layouts}", dwg_report.warnings);
        }
    };
    let _ = fs::remove_file(&dxf);
    let _ = fs::remove_file(&dwg);
    assert!(
        !from_dwg.model_space.is_empty(),
        "saved DWG reimported with an empty model space\nwarnings: {:?}\nunsupported: {unsupported}\nlayouts: {layouts}",
        dwg_report.warnings
    );
    assert!(
        from_dwg.diagnostics.entity_total() > 0,
        "saved DWG reimported no entities"
    );
}

#[test]
fn oda_file_converter_audits_export() {
    let Ok(converter) = std::env::var("MYCAD_ODA_FILE_CONVERTER") else {
        eprintln!("MYCAD_ODA_FILE_CONVERTER is not set; skipping ODA audit");
        return;
    };
    let converter = PathBuf::from(converter.trim());
    if !converter.is_file() {
        eprintln!(
            "ODA File Converter was not found at {}; skipping ODA audit",
            converter.display()
        );
        return;
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mycad-oda-{stamp}"));
    let dxf_in = root.join("dxf-in");
    let dwg_in = root.join("dwg-in");
    let dxf_out = root.join("dxf-out");
    let dwg_out = root.join("dwg-out");
    fs::create_dir_all(&dxf_in).expect("dxf in");
    fs::create_dir_all(&dwg_in).expect("dwg in");
    fs::create_dir_all(&dxf_out).expect("dxf out");
    fs::create_dir_all(&dwg_out).expect("dwg out");

    let document = cad_core::primitives_document();
    let dxf_path = dxf_in.join("primitives.dxf");
    cad_io::write_dxf(&document, &dxf_path, &cad_io::DxfExportOptions::default())
        .expect("write dxf");
    let dwg_path = dwg_in.join("primitives.dwg");
    dwg_import::write_dwg(&document, &dwg_path).expect("write dwg");
    let sheet = cad_core::autocad_features_document();
    cad_io::write_dxf(
        &sheet,
        &dxf_in.join("layouts.dxf"),
        &cad_io::DxfExportOptions::default(),
    )
    .expect("write layout dxf");
    dwg_import::write_dwg(&sheet, &dwg_in.join("layouts.dwg")).expect("write layout dwg");

    audit_with_oda(&converter, &dxf_in, &dxf_out, "DWG", "*.dxf");
    audit_with_oda(&converter, &dwg_in, &dwg_out, "DWG", "*.dwg");
    let reimported =
        dwg_import::import_dwg(&dwg_in.join("layouts.dwg")).expect("reimport layout dwg");
    let fence = reimported
        .linetypes
        .values()
        .find(|linetype| linetype.name.eq_ignore_ascii_case("FENCELINE1"))
        .expect("FENCELINE1");
    assert!(
        fence.shapes.iter().flatten().any(|shape| shape
            .shape_file
            .to_ascii_lowercase()
            .ends_with("ltypeshp.shx")),
        "ODA-audited layout DWG lost the fence shape file: {fence:?}"
    );
    assert!(
        reimported
            .linetypes
            .values()
            .find(|linetype| linetype.name.eq_ignore_ascii_case("AMZIGZAG"))
            .and_then(|linetype| linetype
                .shapes
                .iter()
                .flatten()
                .find(|shape| shape.text == "Z"))
            .is_some(),
        "ODA-audited layout DWG lost the zigzag text: {:?}",
        reimported.linetypes.get("AMZIGZAG")
    );
    let _ = fs::remove_dir_all(&root);
}

fn audit_with_oda(converter: &Path, input: &Path, output: &Path, format: &str, filter: &str) {
    let status = Command::new(converter)
        .arg(input)
        .arg(output)
        .arg("ACAD2018")
        .arg(format)
        .arg("0")
        .arg("1")
        .arg(filter)
        .status()
        .unwrap_or_else(|err| panic!("failed to start ODA File Converter: {err}"));
    assert!(
        status.success(),
        "ODA File Converter failed for {} ({status})",
        input.display()
    );
    let mut errors = Vec::new();
    for entry in fs::read_dir(output).expect("list ODA output") {
        let entry = entry.expect("entry");
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.to_ascii_lowercase().ends_with(".err") {
            let body = fs::read_to_string(entry.path()).unwrap_or_default();
            errors.push(format!("{name}: {body}"));
        }
    }
    assert!(
        errors.is_empty(),
        "ODA audit reported errors for {}:\n{}",
        input.display(),
        errors.join("\n")
    );
    let produced = fs::read_dir(output)
        .expect("list")
        .filter_map(|entry| entry.ok())
        .any(|entry| {
            entry
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("dwg"))
        });
    assert!(
        produced,
        "ODA File Converter wrote no DWG for {}",
        input.display()
    );
}
