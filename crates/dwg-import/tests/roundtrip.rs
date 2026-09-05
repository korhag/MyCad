//! Geometry round-trip through DXF and the existing DWG importer.
//!
//! These tests compare native coordinates, angles, bulge, block transforms,
//! entity counts, layers, linetypes, and extents. A file existing on disk is
//! not enough.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use cad_core::{
    compare_documents, primitives_document, CompareTol, Document, DrawingUnits, Entity, Geometry,
    Point3,
};
use cad_io::{write_dxf, DxfExportOptions};
use dwg_import::{convert_dxf_to_dwg, import_dwg, import_dxf, DwgOutputVersion};

fn stamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos()
}

fn temp_path(ext: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "mycad-roundtrip-{}-{}.{}",
        std::process::id(),
        stamp(),
        ext
    ))
}

fn assert_geometry(expected: &Document, actual: &Document, label: &str) {
    let mismatches = compare_documents(expected, actual, CompareTol::ROUND_TRIP);
    if mismatches.is_empty() {
        return;
    }
    let detail = mismatches
        .iter()
        .map(|item| format!("  {item}"))
        .collect::<Vec<_>>()
        .join("\n");
    panic!(
        "{label}: {} geometry mismatches\n{detail}\nLibreDWG entity_counts: {:?}\nwarnings: {:?}",
        mismatches.len(),
        actual.diagnostics.entity_counts,
        actual.diagnostics.warnings
    );
}

fn cleanup(paths: &[&Path]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

#[test]
fn document_dxf_import_preserves_geometry() {
    let expected = primitives_document();
    let dxf = temp_path("dxf");
    write_dxf(&expected, &dxf, &DxfExportOptions::default()).expect("write DXF");
    let actual = match import_dxf(&dxf) {
        Ok(document) => document,
        Err(err) => {
            cleanup(&[&dxf]);
            panic!("import_dxf failed: {err}");
        }
    };
    cleanup(&[&dxf]);
    assert_geometry(&expected, &actual, "Document → DXF → import");
}

#[test]
fn document_dxf_dwg_import_preserves_geometry() {
    let expected = primitives_document();
    let dxf = temp_path("dxf");
    let dwg = temp_path("dwg");
    write_dxf(&expected, &dxf, &DxfExportOptions::default()).expect("write DXF");
    if let Err(err) = convert_dxf_to_dwg(&dxf, &dwg, DwgOutputVersion::R2000) {
        cleanup(&[&dxf, &dwg]);
        panic!("Document → DXF → DWG failed: {err}");
    }
    let actual = match import_dwg(&dwg) {
        Ok(document) => document,
        Err(err) => {
            cleanup(&[&dxf, &dwg]);
            panic!("import_dwg failed: {err}");
        }
    };
    cleanup(&[&dxf, &dwg]);
    assert_geometry(&expected, &actual, "Document → DXF → DWG → import");
}

fn inches_line_document() -> Document {
    let mut document = Document::default();
    document.units = DrawingUnits::Inches;
    document.add_entity(Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 0.0),
        end: Point3::from_xy(12.0, 0.0),
    }));
    document.assign_missing_ids();
    document
}

#[test]
fn document_dxf_import_preserves_inches() {
    let expected = inches_line_document();
    let dxf = temp_path("dxf");
    write_dxf(&expected, &dxf, &DxfExportOptions::default()).expect("write DXF");
    let actual = match import_dxf(&dxf) {
        Ok(document) => document,
        Err(err) => {
            cleanup(&[&dxf]);
            panic!("import_dxf failed: {err}");
        }
    };
    cleanup(&[&dxf]);
    assert_geometry(&expected, &actual, "inches Document → DXF → import");
}

#[test]
fn document_dxf_dwg_import_preserves_inches() {
    let expected = inches_line_document();
    let dxf = temp_path("dxf");
    let dwg = temp_path("dwg");
    write_dxf(&expected, &dxf, &DxfExportOptions::default()).expect("write DXF");
    if let Err(err) = convert_dxf_to_dwg(&dxf, &dwg, DwgOutputVersion::R2000) {
        cleanup(&[&dxf, &dwg]);
        panic!("Document → DXF → DWG failed: {err}");
    }
    let actual = match import_dwg(&dwg) {
        Ok(document) => document,
        Err(err) => {
            cleanup(&[&dxf, &dwg]);
            panic!("import_dwg failed: {err}");
        }
    };
    cleanup(&[&dxf, &dwg]);
    assert_geometry(&expected, &actual, "inches Document → DXF → DWG → import");
}

#[test]
fn created_block_survives_dxf_roundtrip() {
    use cad_core::{create_block_from_entities, default_extrusion, EntitySpace, Point2};
    let mut expected = Document::default();
    let a = expected.add_entity(Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 0.0),
        end: Point3::from_xy(10.0, 0.0),
    }));
    let b = expected.add_entity(Entity::new(Geometry::Circle {
        center: Point3::from_xy(5.0, 0.0),
        radius: 2.0,
        extrusion: default_extrusion(),
    }));
    create_block_from_entities(
        &mut expected,
        &EntitySpace::ModelSpace,
        &[a.id, b.id],
        "TestBlock",
        Point2::new(5.0, 0.0),
        true,
    )
    .expect("create block");
    let dxf = temp_path("dxf");
    write_dxf(&expected, &dxf, &DxfExportOptions::default()).expect("write DXF");
    let actual = match import_dxf(&dxf) {
        Ok(document) => document,
        Err(err) => {
            cleanup(&[&dxf]);
            panic!("import_dxf failed: {err}");
        }
    };
    cleanup(&[&dxf]);
    assert!(
        actual.block_by_name("TestBlock").is_some(),
        "definition missing"
    );
    assert_eq!(actual.model_space.len(), 1);
    match &actual.model_space[0].geometry {
        Geometry::Insert { block_name, .. } => assert_eq!(block_name, "TestBlock"),
        other => panic!("{other:?}"),
    }
    assert_eq!(actual.block_by_name("TestBlock").unwrap().entities.len(), 2);
}

#[test]
fn nested_block_survives_dxf_roundtrip() {
    use cad_core::{create_block_from_entities, default_extrusion, EntitySpace, Point2};
    let mut expected = Document::default();
    let circle = expected.add_entity(Entity::new(Geometry::Circle {
        center: Point3::from_xy(0.0, 0.0),
        radius: 1.0,
        extrusion: default_extrusion(),
    }));
    create_block_from_entities(
        &mut expected,
        &EntitySpace::ModelSpace,
        &[circle.id],
        "B",
        Point2::new(0.0, 0.0),
        true,
    )
    .expect("create B");
    let line = expected.add_entity(Entity::new(Geometry::Line {
        start: Point3::from_xy(8.0, 0.0),
        end: Point3::from_xy(10.0, 0.0),
    }));
    let b_id = expected.model_space[0].id;
    create_block_from_entities(
        &mut expected,
        &EntitySpace::ModelSpace,
        &[line.id, b_id],
        "A",
        Point2::new(0.0, 0.0),
        true,
    )
    .expect("create A");
    let before = expected.compute_extents();
    let dxf = temp_path("dxf");
    write_dxf(&expected, &dxf, &DxfExportOptions::default()).expect("write DXF");
    let actual = match import_dxf(&dxf) {
        Ok(document) => document,
        Err(err) => {
            cleanup(&[&dxf]);
            panic!("import_dxf failed: {err}");
        }
    };
    cleanup(&[&dxf]);
    assert!(actual.block_by_name("A").is_some());
    assert!(actual.block_by_name("B").is_some());
    assert_eq!(actual.model_space.len(), 1);
    match &actual.model_space[0].geometry {
        Geometry::Insert { block_name, .. } => assert_eq!(block_name, "A"),
        other => panic!("{other:?}"),
    }
    let a = actual.block_by_name("A").unwrap();
    assert!(a
        .entities
        .iter()
        .any(|entity| matches!(entity.geometry, Geometry::Insert { .. })));
    assert_eq!(actual.block_by_name("B").unwrap().entities.len(), 1);
    if let (Some(expected_ext), Some(actual_ext)) = (before, actual.compute_extents()) {
        assert!((expected_ext.min.x - actual_ext.min.x).abs() < 1e-6);
        assert!((expected_ext.max.x - actual_ext.max.x).abs() < 1e-6);
    }
}

#[test]
fn created_block_survives_dxf_dwg_roundtrip() {
    use cad_core::{create_block_from_entities, default_extrusion, EntitySpace, Point2};
    let mut expected = Document::default();
    let a = expected.add_entity(Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 0.0),
        end: Point3::from_xy(10.0, 0.0),
    }));
    let b = expected.add_entity(Entity::new(Geometry::Circle {
        center: Point3::from_xy(5.0, 0.0),
        radius: 2.0,
        extrusion: default_extrusion(),
    }));
    create_block_from_entities(
        &mut expected,
        &EntitySpace::ModelSpace,
        &[a.id, b.id],
        "TestBlock",
        Point2::new(5.0, 0.0),
        true,
    )
    .expect("create block");
    let dxf = temp_path("dxf");
    let dwg = temp_path("dwg");
    write_dxf(&expected, &dxf, &DxfExportOptions::default()).expect("write DXF");
    if let Err(err) = convert_dxf_to_dwg(&dxf, &dwg, DwgOutputVersion::R2000) {
        cleanup(&[&dxf, &dwg]);
        panic!("Document → DXF → DWG failed: {err}");
    }
    let actual = match import_dwg(&dwg) {
        Ok(document) => document,
        Err(err) => {
            cleanup(&[&dxf, &dwg]);
            panic!("import_dwg failed: {err}");
        }
    };
    cleanup(&[&dxf, &dwg]);
    assert!(
        actual.block_by_name("TestBlock").is_some(),
        "definition missing after DWG round-trip"
    );
    assert_eq!(actual.model_space.len(), 1);
    match &actual.model_space[0].geometry {
        Geometry::Insert { block_name, .. } => assert_eq!(block_name, "TestBlock"),
        other => panic!("{other:?}"),
    }
    assert_eq!(actual.block_by_name("TestBlock").unwrap().entities.len(), 2);
}

#[test]
fn renamed_block_survives_dxf_and_dwg_roundtrip() {
    use cad_core::{create_block_from_entities, default_extrusion, EntitySpace, Point2};
    let mut expected = Document::default();
    let a = expected.add_entity(Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 0.0),
        end: Point3::from_xy(10.0, 0.0),
    }));
    let b = expected.add_entity(Entity::new(Geometry::Circle {
        center: Point3::from_xy(5.0, 0.0),
        radius: 2.0,
        extrusion: default_extrusion(),
    }));
    create_block_from_entities(
        &mut expected,
        &EntitySpace::ModelSpace,
        &[a.id, b.id],
        "Motor",
        Point2::new(5.0, 0.0),
        true,
    )
    .expect("create block");
    expected
        .rename_block("Motor", "Motor Drive")
        .expect("rename");
    let dxf = temp_path("dxf");
    write_dxf(&expected, &dxf, &DxfExportOptions::default()).expect("write DXF");
    let from_dxf = match import_dxf(&dxf) {
        Ok(document) => document,
        Err(err) => {
            cleanup(&[&dxf]);
            panic!("import_dxf failed: {err}");
        }
    };
    assert!(from_dxf.block_by_name("Motor Drive").is_some());
    assert!(from_dxf.block_by_name("Motor").is_none());
    match &from_dxf.model_space[0].geometry {
        Geometry::Insert { block_name, .. } => assert_eq!(block_name, "Motor Drive"),
        other => panic!("{other:?}"),
    }

    let dwg = temp_path("dwg");
    if let Err(err) = convert_dxf_to_dwg(&dxf, &dwg, DwgOutputVersion::R2000) {
        cleanup(&[&dxf, &dwg]);
        panic!("Document → DXF → DWG failed: {err}");
    }
    let from_dwg = match import_dwg(&dwg) {
        Ok(document) => document,
        Err(err) => {
            cleanup(&[&dxf, &dwg]);
            panic!("import_dwg failed: {err}");
        }
    };
    cleanup(&[&dxf, &dwg]);
    assert!(from_dwg.block_by_name("Motor Drive").is_some());
    match &from_dwg.model_space[0].geometry {
        Geometry::Insert { block_name, .. } => assert_eq!(block_name, "Motor Drive"),
        other => panic!("{other:?}"),
    }
}
