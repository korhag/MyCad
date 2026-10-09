use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    assert_same_entity_counts(&original, &from_dxf, "DXF", true, &[]);
    // Constant-Z POLYLINE_3D is written as POLYLINE. The fold flag accounts
    // for that. Every other type, including ATTDEF, must match.
    assert_same_entity_counts(&original, &from_dwg, "DWG", true, &[]);
    assert_same_lwpolylines(&original, &from_dxf, "DXF");
    assert_same_lwpolylines(&original, &from_dwg, "DWG");
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

fn assert_same_entity_counts(
    original: &cad_core::Document,
    actual: &cad_core::Document,
    label: &str,
    fold_polyline_3d: bool,
    ignore: &[&str],
) {
    let diffs = count_diffs(
        &original.diagnostics.entity_counts,
        &actual.diagnostics.entity_counts,
        fold_polyline_3d,
        ignore,
    );
    assert!(
        diffs.is_empty(),
        "{label} round-trip changed entity counts:\n{}",
        diffs.join("\n")
    );
}

fn assert_same_lwpolylines(
    original: &cad_core::Document,
    actual: &cad_core::Document,
    label: &str,
) {
    let left = lwpolyline_stats(original);
    let right = lwpolyline_stats(actual);
    assert_eq!(left.count, right.count, "{label} LWPOLYLINE count changed");
    assert_eq!(
        left.closed, right.closed,
        "{label} closed LWPOLYLINE count changed"
    );
    assert_bounds_close(left.stored, right.stored, label, "stored");
    assert_bounds_close(left.world, right.world, label, "world");
}

fn count_diffs(
    original: &BTreeMap<String, u64>,
    actual: &BTreeMap<String, u64>,
    fold_polyline_3d: bool,
    ignore: &[&str],
) -> Vec<String> {
    let mut original = normalized_entity_counts(original, fold_polyline_3d);
    let mut actual = normalized_entity_counts(actual, fold_polyline_3d);
    for name in ignore {
        original.remove(*name);
        actual.remove(*name);
    }
    let mut names: Vec<&String> = original.keys().chain(actual.keys()).collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter_map(|name| {
            let before = original.get(name).copied().unwrap_or(0);
            let after = actual.get(name).copied().unwrap_or(0);
            (before != after).then(|| format!("{name}: {before} -> {after}"))
        })
        .collect()
}

fn normalized_entity_counts(
    counts: &BTreeMap<String, u64>,
    fold_polyline_3d: bool,
) -> BTreeMap<String, u64> {
    let mut normalized = BTreeMap::new();
    for (name, count) in counts {
        let name = if name == "POLYLINE" || (fold_polyline_3d && name == "POLYLINE_3D") {
            "POLYLINE_2D"
        } else {
            name.as_str()
        };
        *normalized.entry(name.to_string()).or_insert(0) += *count;
    }
    normalized
}

struct LwStats {
    count: u64,
    closed: u64,
    stored: Option<Bounds>,
    world: Option<Bounds>,
}

#[derive(Clone, Copy, Debug)]
struct Bounds {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

fn lwpolyline_stats(document: &cad_core::Document) -> LwStats {
    let mut stats = LwStats {
        count: 0,
        closed: 0,
        stored: None,
        world: None,
    };
    for_each_entity(document, |entity| {
        let cad_core::Geometry::LwPolyline {
            vertices,
            closed,
            extrusion,
            ..
        } = &entity.geometry
        else {
            return;
        };
        stats.count += 1;
        if *closed {
            stats.closed += 1;
        }
        for vertex in vertices {
            include_bounds(&mut stats.stored, vertex.point.x, vertex.point.y);
            let world = cad_core::ocs_to_wcs(vertex.point, *extrusion);
            include_bounds(&mut stats.world, world.x, world.y);
        }
    });
    stats
}

fn for_each_entity(document: &cad_core::Document, mut visit: impl FnMut(&cad_core::Entity)) {
    for entity in &document.model_space {
        visit(entity);
    }
    for block in document.blocks.values() {
        for entity in &block.entities {
            visit(entity);
        }
    }
}

fn include_bounds(bounds: &mut Option<Bounds>, x: f64, y: f64) {
    if !x.is_finite() || !y.is_finite() {
        return;
    }
    match bounds {
        Some(current) => {
            current.min_x = current.min_x.min(x);
            current.min_y = current.min_y.min(y);
            current.max_x = current.max_x.max(x);
            current.max_y = current.max_y.max(y);
        }
        None => {
            *bounds = Some(Bounds {
                min_x: x,
                min_y: y,
                max_x: x,
                max_y: y,
            });
        }
    }
}

fn assert_bounds_close(left: Option<Bounds>, right: Option<Bounds>, label: &str, kind: &str) {
    match (left, right) {
        (None, None) => {}
        (Some(left), Some(right)) => {
            for (name, before, after) in [
                ("min x", left.min_x, right.min_x),
                ("min y", left.min_y, right.min_y),
                ("max x", left.max_x, right.max_x),
                ("max y", left.max_y, right.max_y),
            ] {
                let scale = before.abs().max(after.abs()).max(1.0);
                assert!(
                    (before - after).abs() <= scale * 1e-6,
                    "{label} LWPOLYLINE {kind} {name} changed from {before} to {after}"
                );
            }
        }
        _ => panic!("{label} LWPOLYLINE {kind} bounds appeared or disappeared"),
    }
}

// ------------------------------------------------------------
// AutoCAD audit
// Purpose: accoreconsole RECOVER + AUDIT is the pass/fail check for a
//          saved DXF or DWG. The test skips when MYCAD_ACCORECONSOLE is unset.
// ------------------------------------------------------------

const ACCORECONSOLE_ENV: &str = "MYCAD_ACCORECONSOLE";
const AUDIT_TIMEOUT: Duration = Duration::from_secs(600);

struct AuditReport {
    errors_found: u64,
    errors_fixed: u64,
    erased: u64,
    by_type: BTreeMap<String, u64>,
    samples: Vec<String>,
    hard: Vec<String>,
    tail: String,
}

#[test]
fn autocad_audits_export() {
    let Ok(console) = std::env::var(ACCORECONSOLE_ENV) else {
        eprintln!("{ACCORECONSOLE_ENV} is not set; skipping AutoCAD audit");
        return;
    };
    let console = PathBuf::from(console.trim());
    if !console.is_file() {
        eprintln!(
            "AutoCAD console was not found at {}; skipping AutoCAD audit",
            console.display()
        );
        return;
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mycad-acad-{stamp}"));
    fs::create_dir_all(&root).expect("audit dir");
    let baseline = std::env::var("MYCAD_AUDIT_BASELINE").is_ok();
    let mut failures = Vec::new();

    if baseline && audit_wanted("originals") {
        if let Some(path) = almarai_dwg() {
            failures.extend(audit_copy(&console, &root, "almarai-original", &path));
        }
        failures.extend(audit_copy(
            &console,
            &root,
            "kd-original",
            &reference_dwg(),
        ));
    }

    if audit_wanted("primitives") {
        failures.extend(export_and_audit(
            &console,
            &root,
            "primitives",
            &cad_core::primitives_document(),
        ));
    }
    if audit_wanted("layouts") {
        failures.extend(export_and_audit(
            &console,
            &root,
            "layouts",
            &cad_core::autocad_features_document(),
        ));
    }

    if audit_wanted("reference") {
        let kd = dwg_import::import_dwg(&reference_dwg()).expect("import reference");
        failures.extend(export_and_audit(&console, &root, "reference", &kd));
    }
    if audit_wanted("almarai") {
        if let Some(path) = almarai_dwg() {
            let almarai = dwg_import::import_dwg(&path).expect("import almarai");
            failures.extend(export_and_audit(&console, &root, "almarai", &almarai));
            if baseline && audit_wanted("bisect") {
                bisect_almarai(&console, &root, &almarai);
            }
        }
    }

    if baseline {
        eprintln!("baseline failures:\n{}", failures.join("\n"));
        return;
    }
    assert!(
        failures.is_empty(),
        "AutoCAD audit reported errors:\n{}",
        failures.join("\n")
    );
}

#[test]
fn parses_autocad_audit_summary() {
    let text = concat!(
        "Auditing Entities Pass 1\n",
        "AcDbLine(2F)\n",
        "              Invalid layer \"MISSING\"\n",
        "AcDbAttributeDefinition(8E)\n",
        "              Erased\n",
        "Total errors found 1979 fixed 12\n",
        "Erased 21042 objects\n",
        "Total errors found 0 fixed 0\n",
        "Erased 0 objects\n",
    );
    let report = parse_audit(text);
    assert_eq!(report.errors_found, 1979);
    assert_eq!(report.errors_fixed, 12);
    assert_eq!(report.erased, 21042);
    assert_eq!(report.by_type.get("AcDbLine").copied(), Some(1));
    assert_eq!(
        report.by_type.get("AcDbAttributeDefinition").copied(),
        Some(1)
    );
    assert!(report.hard.is_empty());
}

#[test]
fn parses_a_discarded_dxf_as_a_hard_failure() {
    let report = parse_audit("DXF read error on line 34.\nInvalid or incomplete DXF input -- drawing discarded.\n");
    assert!(report.hard.iter().any(|item| item.contains("discarded")));
    assert_eq!(report.errors_found, 0);
}

fn audit_wanted(name: &str) -> bool {
    // The pass/fail test always audits every required drawing. MYCAD_AUDIT_ONLY
    // only narrows a baseline run.
    if std::env::var("MYCAD_AUDIT_BASELINE").is_err() {
        return true;
    }
    let Ok(filter) = std::env::var("MYCAD_AUDIT_ONLY") else {
        return true;
    };
    filter
        .split(',')
        .any(|item| item.trim().eq_ignore_ascii_case(name))
}

fn almarai_dwg() -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../samples/AlMarai-ThirdLabelingLine.dwg");
    path.is_file().then_some(path)
}

fn audit_copy(console: &Path, root: &Path, name: &str, source: &Path) -> Vec<String> {
    let dest = root.join(format!(
        "{name}.{}",
        source.extension().and_then(|ext| ext.to_str()).unwrap_or("dwg")
    ));
    if let Err(err) = fs::copy(source, &dest) {
        return vec![format!("{name}: failed to copy {}: {err}", source.display())];
    }
    expect_clean(console, &dest, name)
}

fn export_and_audit(
    console: &Path,
    root: &Path,
    name: &str,
    document: &cad_core::Document,
) -> Vec<String> {
    let mut failures = Vec::new();
    let dxf = root.join(format!("{name}.dxf"));
    match cad_io::write_dxf(document, &dxf, &cad_io::DxfExportOptions::default()) {
        Ok(_) => failures.extend(expect_clean(console, &dxf, &format!("{name} dxf"))),
        Err(err) => failures.push(format!("{name} dxf write failed: {err}")),
    }
    let dwg = root.join(format!("{name}.dwg"));
    match dwg_import::write_dwg(document, &dwg) {
        Ok(_) => failures.extend(expect_clean(console, &dwg, &format!("{name} dwg"))),
        Err(err) => failures.push(format!("{name} dwg write failed: {err}")),
    }
    failures
}

fn expect_clean(console: &Path, drawing: &Path, label: &str) -> Vec<String> {
    match audit_drawing(console, drawing) {
        Ok(report) => {
            eprintln!(
                "audit {label}: found {} fixed {} erased {} types {:?}{}",
                report.errors_found,
                report.errors_fixed,
                report.erased,
                report.by_type,
                if report.samples.is_empty() {
                    String::new()
                } else {
                    format!("\n  {}", report.samples.join("\n  "))
                }
            );
            let mut problems = Vec::new();
            if report.errors_found > 0 || report.erased > 0 || !report.hard.is_empty() {
                problems.push(format!(
                    "{label}: found {} fixed {} erased {} types {:?} hard {:?}\n{}\n{}",
                    report.errors_found,
                    report.errors_fixed,
                    report.erased,
                    report.by_type,
                    report.hard,
                    report.samples.join("\n"),
                    report.tail
                ));
            }
            problems
        }
        Err(err) => vec![format!("{label}: {err}")],
    }
}

fn audit_drawing(console: &Path, drawing: &Path) -> Result<AuditReport, String> {
    let dir = drawing
        .parent()
        .ok_or_else(|| format!("no directory for {}", drawing.display()))?;
    let script_path = script_for(drawing);
    fs::write(&script_path, audit_script(drawing)).map_err(|err| err.to_string())?;
    let mut text = run_console(console, &script_path, dir)?;
    if console_refused(&text) {
        let acadlt = console.with_file_name("acadlt.exe");
        if acadlt.is_file() {
            text = run_batch(&acadlt, &script_path, dir)?;
        }
    }
    let adt = drawing.with_extension("adt");
    if let Ok(bytes) = fs::read(&adt) {
        text.push('\n');
        text.push_str(&decode_autocad_text(&bytes));
    }
    Ok(parse_audit(&text))
}

fn script_for(drawing: &Path) -> PathBuf {
    let mut name = drawing
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".scr");
    drawing.with_file_name(name)
}

fn audit_script(drawing: &Path) -> String {
    let path = drawing.to_string_lossy().replace('\\', "/");
    format!(
        "FILEDIA\n0\nCMDDIA\n0\nEXPERT\n5\nAUDITCTL\n1\n_.RECOVER\n{path}\n_.AUDIT\n_N\n_.QUIT\n_Y\n"
    )
}

fn console_refused(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    !lower.contains("autocad") && !lower.contains("command:")
}

fn run_console(console: &Path, script: &Path, dir: &Path) -> Result<String, String> {
    run_command(console, &["/s", &script.to_string_lossy()], dir)
}

fn run_batch(acadlt: &Path, script: &Path, dir: &Path) -> Result<String, String> {
    run_command(acadlt, &["/b", &script.to_string_lossy()], dir)
}

fn run_command(program: &Path, args: &[&str], dir: &Path) -> Result<String, String> {
    let child = Command::new(program)
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to start {}: {err}", program.display()))?;
    let pid = child.id();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(AUDIT_TIMEOUT) {
        Ok(Ok(output)) => Ok(format!(
            "{}\n{}",
            decode_autocad_text(&output.stdout),
            decode_autocad_text(&output.stderr)
        )),
        Ok(Err(err)) => Err(format!("{}: {err}", program.display())),
        Err(_) => {
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .status();
            Err(format!(
                "{} timed out after {}s",
                program.display(),
                AUDIT_TIMEOUT.as_secs()
            ))
        }
    }
}

fn decode_autocad_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16_le(&bytes[2..]);
    }
    let sample = bytes.len().min(200);
    let nuls = bytes.iter().take(sample).filter(|byte| **byte == 0).count();
    if sample > 8 && nuls * 3 > sample {
        return decode_utf16_le(bytes);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn decode_utf16_le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

fn parse_audit(text: &str) -> AuditReport {
    let mut report = AuditReport {
        errors_found: 0,
        errors_fixed: 0,
        erased: 0,
        by_type: BTreeMap::new(),
        samples: Vec::new(),
        hard: Vec::new(),
        tail: String::new(),
    };
    for needle in [
        "drawing discarded",
        "DXF read error",
        "FATAL ERROR",
        "Invalid or incomplete",
    ] {
        if text.contains(needle) {
            report.hard.push(needle.to_string());
        }
    }
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let mut current_object: Option<String> = None;
    for line in &lines {
        if let Some(rest) = line.strip_prefix("Total errors found") {
            let nums: Vec<u64> = rest
                .split_whitespace()
                .filter_map(|word| word.parse().ok())
                .collect();
            if nums.len() >= 2 {
                report.errors_found = report.errors_found.max(nums[0]);
                report.errors_fixed = report.errors_fixed.max(nums[1]);
            }
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("erased ") || lower.contains("objects erased") {
            if let Some(count) = lower
                .split_whitespace()
                .find_map(|word| word.parse::<u64>().ok())
            {
                report.erased = report.erased.max(count);
            }
        }
        if let Some(kind) = acad_object_type(line) {
            current_object = Some(kind.clone());
            if line_is_audit_error(line) {
                note_audit_object(&mut report, kind, line);
            }
            continue;
        }
        if line_is_audit_error(line) {
            if let Some(kind) = current_object.clone() {
                note_audit_object(&mut report, kind, line);
            } else if report.samples.len() < 24 {
                report.samples.push((*line).to_string());
            }
        }
    }
    let tail: Vec<&str> = lines.iter().rev().take(12).copied().collect();
    report.tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
    report
}

fn note_audit_object(report: &mut AuditReport, kind: String, line: &str) {
    *report.by_type.entry(kind).or_insert(0) += 1;
    if report.samples.len() < 24 {
        report.samples.push(line.to_string());
    }
}

fn acad_object_type(line: &str) -> Option<String> {
    let start = line.find("AcDb")?;
    let rest = &line[start..];
    let end = rest
        .find(|ch: char| !ch.is_ascii_alphanumeric())
        .unwrap_or(rest.len());
    let name = &rest[..end];
    (name.len() > 4).then(|| name.to_string())
}

fn line_is_audit_error(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if lower.starts_with("total errors found")
        || lower.starts_with("erased ")
        || lower.contains("objects erased")
        || lower.starts_with("pass ")
    {
        return false;
    }
    lower.contains("invalid")
        || lower.contains("erased")
        || lower.contains("error")
        || lower.contains("null")
        || lower.contains("discard")
}

fn bisect_almarai(console: &Path, root: &Path, document: &cad_core::Document) {
    let tables = strip_entities(document);
    let _ = export_and_audit(console, root, "almarai-tables", &tables);
    let mut inserts = document.clone();
    inserts.model_space.retain(|entity| {
        matches!(entity.geometry, cad_core::Geometry::Insert { .. })
    });
    let _ = export_and_audit(console, root, "almarai-inserts", &inserts);

    let mut buckets: BTreeMap<&str, u64> = BTreeMap::new();
    for entity in document
        .model_space
        .iter()
        .chain(document.blocks.values().flat_map(|block| block.entities.iter()))
    {
        *buckets.entry(entity_bucket(entity)).or_insert(0) += 1;
    }
    for (bucket, count) in buckets {
        eprintln!("bisect {bucket}: {count} entities");
        let subset = retain_entities(document, |entity| entity_bucket(entity) == bucket);
        let name = format!("almarai-{}", bucket.to_ascii_lowercase());
        let _ = export_and_audit(console, root, &name, &subset);
    }
}

fn strip_entities(document: &cad_core::Document) -> cad_core::Document {
    retain_entities(document, |_| false)
}

fn retain_entities(
    document: &cad_core::Document,
    keep: impl Fn(&cad_core::Entity) -> bool,
) -> cad_core::Document {
    let mut copy = document.clone();
    copy.model_space.retain(&keep);
    for block in copy.blocks.values_mut() {
        block.entities.retain(&keep);
    }
    copy
}

fn entity_bucket(entity: &cad_core::Entity) -> &'static str {
    match &entity.geometry {
        cad_core::Geometry::Text(data) if data.is_attrib_def => "ATTDEF",
        cad_core::Geometry::Text(_) => "TEXT",
        other => other.type_name(),
    }
}
