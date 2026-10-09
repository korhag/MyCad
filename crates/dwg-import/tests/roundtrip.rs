//! Geometry round-trip through DXF and the existing DWG importer.
//!
//! These tests compare native coordinates, angles, bulge, block transforms,
//! entity counts, layers, linetypes, and extents. A file existing on disk is
//! not enough.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use cad_core::{
    autocad_features_document, compare_documents, primitives_document, AttributeInfo,
    BlockDefinition, CompareTol, Document, DrawingUnits, Entity, Geometry, PaperLayout, Point2,
    Point3, TextData, TextStyle, ViewportData, ATTRIB_INVISIBLE, ATTRIB_PRESET,
};
use cad_io::{write_dxf, DxfExportOptions};
use dwg_import::{convert_dxf_to_dwg, import_dwg, import_dxf, write_dwg, DwgOutputVersion};

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

fn rename_block(document: &mut Document, from: &str, to: &str) {
    if let Some(mut block) = document.blocks.remove(from) {
        block.name = to.to_string();
        document.blocks.insert(to.to_string(), block);
    }
    for entity in &mut document.model_space {
        rewrite_block_ref(&mut entity.geometry, from, to);
    }
    for block in document.blocks.values_mut() {
        for entity in &mut block.entities {
            rewrite_block_ref(&mut entity.geometry, from, to);
        }
    }
}

fn rewrite_block_ref(geometry: &mut Geometry, from: &str, to: &str) {
    match geometry {
        Geometry::Dimension(data) if data.block_name == from => {
            data.block_name = to.to_string();
        }
        Geometry::Insert { block_name, .. } if block_name == from => {
            *block_name = to.to_string();
        }
        _ => {}
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

#[test]
fn varying_z_polyline_survives_dxf_roundtrip() {
    let mut expected = Document::default();
    expected.add_entity(Entity::new(Geometry::Polyline {
        vertices: vec![
            cad_core::PolyVertex {
                point: Point3::new(0.0, 0.0, 1.5),
                bulge: 0.0,
                vertex_id: Default::default(),
            },
            cad_core::PolyVertex {
                point: Point3::new(10.0, 0.0, 4.5),
                bulge: 0.0,
                vertex_id: Default::default(),
            },
        ],
        closed: false,
        linetype_generation_continuous: false,
    }));
    expected.assign_missing_ids();
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
    assert_geometry(&expected, &actual, "3D POLYLINE → DXF → import");
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

fn paper_sheet() -> Document {
    let mut document = Document::default();
    document.add_entity(Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 0.0),
        end: Point3::from_xy(25.0, 0.0),
    }));
    document.layouts.push(PaperLayout {
        name: "Sheet".into(),
        block_name: "*PAPER_SPACE0".into(),
        tab_order: 2,
        paper_width: 420.0,
        paper_height: 297.0,
        left_margin: 10.0,
        bottom_margin: 10.0,
        right_margin: 10.0,
        top_margin: 10.0,
    });
    document.blocks.insert(
        "*PAPER_SPACE0".into(),
        BlockDefinition {
            name: "*PAPER_SPACE0".into(),
            entities: vec![
                Entity::new(Geometry::Line {
                    start: Point3::from_xy(4.0, 8.0),
                    end: Point3::from_xy(9.0, 8.0),
                }),
                Entity::new(Geometry::Viewport(ViewportData {
                    center: Point3::from_xy(100.0, 50.0),
                    width: 180.0,
                    height: 120.0,
                    view_center: Point2::new(10.0, 20.0),
                    view_height: 80.0,
                    ..ViewportData::default()
                })),
            ],
            ..BlockDefinition::default()
        },
    );
    document
}

#[test]
fn paper_space_viewport_survives_dxf_roundtrip() {
    let expected = paper_sheet();
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
    let sheet = actual
        .layouts
        .iter()
        .find(|layout| layout.block_name.eq_ignore_ascii_case("*PAPER_SPACE0"))
        .unwrap_or_else(|| panic!("layouts: {:?}", actual.layouts));
    assert_eq!(sheet.name, "Sheet");
    assert!(
        (sheet.paper_width - 420.0).abs() < 1e-6,
        "{}",
        sheet.paper_width
    );
    assert!((sheet.paper_height - 297.0).abs() < 1e-6);
    assert!((sheet.left_margin - 10.0).abs() < 1e-6);
    let block = actual
        .block_by_name("*PAPER_SPACE0")
        .unwrap_or_else(|| panic!("blocks: {:?}", actual.blocks.keys().collect::<Vec<_>>()));
    assert_eq!(
        block.entities.len(),
        2,
        "{:?}",
        actual.diagnostics.unsupported_counts
    );
    match &block.entities[1].geometry {
        Geometry::Viewport(viewport) => {
            assert!((viewport.center.x - 100.0).abs() < 1e-6);
            assert!((viewport.width - 180.0).abs() < 1e-6);
            assert!((viewport.height - 120.0).abs() < 1e-6);
            assert!((viewport.view_center.x - 10.0).abs() < 1e-6);
            assert!((viewport.view_height - 80.0).abs() < 1e-6);
        }
        other => panic!("{other:?}"),
    }
}

fn attribute_block() -> Document {
    let mut document = Document::default();
    document.units = DrawingUnits::Millimeters;
    let mut definition = Entity::new(Geometry::Text(TextData {
        insertion: Point3::from_xy(0.0, 0.0),
        height: 2.5,
        value: "Door".into(),
        is_attrib_def: true,
        attribute: Some(AttributeInfo {
            tag: "NAME".into(),
            prompt: "Name".into(),
            flags: ATTRIB_INVISIBLE,
        }),
        ..TextData::default()
    }));
    definition.layer = "0".into();
    document.blocks.insert(
        "TAGGED".into(),
        BlockDefinition {
            name: "TAGGED".into(),
            entities: vec![
                Entity::new(Geometry::Line {
                    start: Point3::from_xy(0.0, 0.0),
                    end: Point3::from_xy(10.0, 0.0),
                }),
                definition,
            ],
            ..BlockDefinition::default()
        },
    );
    document.add_entity(Entity::new(Geometry::Insert {
        block_name: "TAGGED".into(),
        insertion: Point3::from_xy(20.0, 5.0),
        scale: Point3::new(1.0, 1.0, 1.0),
        rotation: 0.0,
        extrusion: Point3::new(0.0, 0.0, 1.0),
        attribs: vec![TextData {
            insertion: Point3::from_xy(20.0, 8.0),
            height: 2.5,
            value: "A1".into(),
            attribute: Some(AttributeInfo {
                tag: "NAME".into(),
                prompt: String::new(),
                flags: 0,
            }),
            ..TextData::default()
        }],
        column_count: 1,
        row_count: 1,
        column_spacing: 0.0,
        row_spacing: 0.0,
        configuration: None,
    }));
    document
}

#[test]
fn attributes_survive_dxf_roundtrip() {
    let expected = attribute_block();
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
    assert_geometry(&expected, &actual, "attributes");
}

#[test]
fn several_attdefs_in_one_block_survive_dxf_roundtrip() {
    let mut document = Document::default();
    document.units = DrawingUnits::Millimeters;
    let specs = [
        ("A", 0_i16),
        ("B", ATTRIB_INVISIBLE),
        ("C", ATTRIB_PRESET),
        ("D", 0),
        ("E", ATTRIB_PRESET),
    ];
    let entities = specs
        .iter()
        .enumerate()
        .map(|(index, (tag, flags))| {
            Entity::new(Geometry::Text(TextData {
                insertion: Point3::from_xy(index as f64, 0.0),
                height: 2.5,
                value: (*tag).into(),
                is_attrib_def: true,
                attribute: Some(AttributeInfo {
                    tag: (*tag).into(),
                    prompt: "Prompt".into(),
                    flags: *flags,
                }),
                ..TextData::default()
            }))
        })
        .collect();
    document.blocks.insert(
        "TAGGED".into(),
        BlockDefinition {
            name: "TAGGED".into(),
            entities,
            ..BlockDefinition::default()
        },
    );
    let dxf = temp_path("dxf");
    write_dxf(&document, &dxf, &DxfExportOptions::default()).expect("write DXF");
    let actual = match import_dxf(&dxf) {
        Ok(document) => document,
        Err(err) => {
            cleanup(&[&dxf]);
            panic!("import_dxf failed: {err}");
        }
    };
    let block = actual
        .block_by_name("TAGGED")
        .unwrap_or_else(|| panic!("blocks: {:?}", actual.blocks.keys().collect::<Vec<_>>()));
    let found: Vec<(String, i16)> = block
        .entities
        .iter()
        .filter_map(|entity| match &entity.geometry {
            Geometry::Text(data) if data.is_attrib_def => {
                let tag = data
                    .attribute
                    .as_ref()
                    .map(|info| info.tag.clone())
                    .unwrap_or_default();
                let flags = data.attribute.as_ref().map(|info| info.flags).unwrap_or(0);
                Some((tag, flags))
            }
            _ => None,
        })
        .collect();
    let dwg = temp_path("dwg");
    let from_dwg = write_dwg(&document, &dwg)
        .map_err(|err| err.to_string())
        .and_then(|_| import_dwg(&dwg).map_err(|err| err.to_string()));
    cleanup(&[&dxf, &dwg]);
    assert_eq!(
        found,
        vec![
            ("A".into(), 0),
            ("B".into(), ATTRIB_INVISIBLE),
            ("C".into(), ATTRIB_PRESET),
            ("D".into(), 0),
            ("E".into(), ATTRIB_PRESET),
        ],
        "counts {:?}",
        actual.diagnostics.entity_counts
    );
    let from_dwg = from_dwg.unwrap_or_else(|err| panic!("DWG round-trip failed: {err}"));
    let dwg_tags = attdef_tags(from_dwg.block_by_name("TAGGED"));
    assert_eq!(
        dwg_tags,
        vec!["A", "B", "C", "D", "E"],
        "DWG counts {:?}",
        from_dwg.diagnostics.entity_counts
    );
}

fn attdef_tags(block: Option<&BlockDefinition>) -> Vec<String> {
    block
        .into_iter()
        .flat_map(|block| block.entities.iter())
        .filter_map(|entity| match &entity.geometry {
            Geometry::Text(data) if data.is_attrib_def => Some(
                data.attribute
                    .as_ref()
                    .map(|info| info.tag.clone())
                    .unwrap_or_default(),
            ),
            _ => None,
        })
        .collect()
}

#[test]
fn autocad_features_survive_dxf_roundtrip() {
    let mut expected = autocad_features_document();
    // *D names are saved without the star. AutoCAD rejects them as anonymous
    // blocks, and the reimport comes back under the ordinary name.
    rename_block(&mut expected, "*D1", "D1");
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
    assert_geometry(&expected, &actual, "autocad features");
}

#[test]
fn complex_linetypes_survive_dwg_roundtrip() {
    let expected = autocad_features_document();
    let dwg = temp_path("dwg");
    if let Err(err) = write_dwg(&expected, &dwg) {
        cleanup(&[&dwg]);
        panic!("write_dwg failed: {err}");
    }
    let actual = match import_dwg(&dwg) {
        Ok(document) => document,
        Err(err) => {
            cleanup(&[&dwg]);
            panic!("import_dwg failed: {err}");
        }
    };
    cleanup(&[&dwg]);
    let mismatches = compare_documents(&expected, &actual, CompareTol::ROUND_TRIP);
    let linetype_misses: Vec<_> = mismatches
        .iter()
        .filter(|miss| miss.path.contains("linetypes."))
        .collect();
    assert!(
        linetype_misses.is_empty(),
        "complex linetypes did not survive DWG: {linetype_misses:?}\nwarnings: {:?}",
        actual.diagnostics.warnings
    );
}

#[test]
fn text_style_survives_dxf_roundtrip() {
    let mut expected = Document::default();
    expected.units = DrawingUnits::Millimeters;
    expected.text_styles.insert(
        "NOTES".into(),
        TextStyle {
            name: "NOTES".into(),
            font_file: "romans.shx".into(),
            bigfont_file: String::new(),
            height: 0.0,
            width_factor: 0.8,
            oblique: 0.0,
        },
    );
    expected.add_entity(Entity::new(Geometry::Text(TextData {
        insertion: Point3::from_xy(1.0, 2.0),
        height: 3.0,
        value: "Styled".into(),
        style: "NOTES".into(),
        ..TextData::default()
    })));
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
    assert_geometry(&expected, &actual, "text style");
}
