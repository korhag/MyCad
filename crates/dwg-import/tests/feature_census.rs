//! `--import-only` feature census: wide polylines and `$LWDISPLAY`.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use cad_core::{Document, Entity, Geometry, Point3, PolyVertex};
use cad_io::{write_dxf, DxfExportOptions};
use dwg_import::import_dxf;

fn temp_dxf() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    std::env::temp_dir().join(format!("mycad-feature-census-{stamp}.dxf"))
}

#[test]
fn wide_lwpolyline_is_counted_and_lwdisplay_is_off() {
    let path = temp_dxf();
    let dxf = concat!(
        "  0\nSECTION\n  2\nHEADER\n  9\n$ACADVER\n  1\nAC1015\n",
        "  9\n$LWDISPLAY\n290\n     0\n",
        "  0\nENDSEC\n  0\nSECTION\n  2\nTABLES\n  0\nTABLE\n  2\nLAYER\n",
        "  0\nLAYER\n  2\n0\n 70\n     0\n 62\n     7\n  6\nCONTINUOUS\n  0\nENDTAB\n",
        "  0\nENDSEC\n  0\nSECTION\n  2\nENTITIES\n",
        "  0\nLWPOLYLINE\n  8\n0\n 90\n        2\n 70\n     1\n 43\n12.5\n",
        " 10\n0.0\n 20\n0.0\n 10\n10.0\n 20\n0.0\n",
        "  0\nLWPOLYLINE\n  8\n0\n 90\n        2\n 70\n     0\n 43\n0.0\n",
        " 10\n0.0\n 20\n1.0\n 10\n10.0\n 20\n1.0\n",
        "  0\nPOLYLINE\n  8\n0\n 66\n     1\n 70\n     0\n",
        "  0\nVERTEX\n  8\n0\n 10\n0.0\n 20\n2.0\n 30\n0.0\n 40\n2.0\n 41\n4.0\n",
        "  0\nVERTEX\n  8\n0\n 10\n5.0\n 20\n2.0\n 30\n0.0\n 40\n4.0\n 41\n2.0\n",
        "  0\nSEQEND\n  8\n0\n",
        "  0\nENDSEC\n  0\nEOF\n"
    );
    fs::write(&path, dxf).expect("write dxf");
    let document = import_dxf(&path).expect("import dxf");
    let _ = fs::remove_file(&path);
    let counts = &document.diagnostics.feature_counts;

    assert_eq!(counts.get("WIDE_LWPOLYLINE").copied(), Some(1));
    assert_eq!(counts.get("WIDE_POLYLINE_2D").copied(), Some(1));
    assert!(
        (document.diagnostics.max_polyline_width - 12.5).abs() < 1e-6,
        "max width {}",
        document.diagnostics.max_polyline_width
    );
    assert_eq!(document.diagnostics.lwdisplay, Some(false));

    let mut lw = Vec::new();
    let mut poly = Vec::new();
    for entity in &document.model_space {
        match &entity.geometry {
            Geometry::LwPolyline { vertices, .. } => lw.push(width_pairs(vertices)),
            Geometry::Polyline { vertices, .. } => poly.push(width_pairs(vertices)),
            _ => {}
        }
    }
    assert!(
        lw.iter().any(|pairs| pairs
            .iter()
            .all(|(start, end)| { (*start - 12.5).abs() < 1e-6 && (*end - 12.5).abs() < 1e-6 })),
        "constant width was not stored: {lw:?}"
    );
    assert!(
        lw.iter().any(|pairs| pairs
            .iter()
            .all(|(start, end)| *start == 0.0 && *end == 0.0)),
        "zero-width polyline was stored as wide: {lw:?}"
    );
    assert!(
        poly.iter().any(|pairs| {
            pairs.len() >= 2
                && (pairs[0].0 - 2.0).abs() < 1e-6
                && (pairs[0].1 - 4.0).abs() < 1e-6
                && (pairs[1].0 - 4.0).abs() < 1e-6
                && (pairs[1].1 - 2.0).abs() < 1e-6
        }),
        "polyline vertex widths: {poly:?}"
    );
}

#[test]
fn variable_width_round_trips_through_dxf() {
    let mut document = Document::default();
    document.add_entity(Entity::new(Geometry::LwPolyline {
        vertices: vec![
            PolyVertex {
                start_width: 1.0,
                end_width: 3.0,
                ..PolyVertex::new(Point3::from_xy(0.0, 0.0), 0.0)
            },
            PolyVertex {
                start_width: 3.0,
                end_width: 1.0,
                ..PolyVertex::new(Point3::from_xy(10.0, 0.0), 0.0)
            },
        ],
        closed: false,
        extrusion: Point3::new(0.0, 0.0, 1.0),
        linetype_generation_continuous: false,
    }));
    let path = temp_dxf();
    write_dxf(&document, &path, &DxfExportOptions::default()).expect("write dxf");
    let loaded = import_dxf(&path).expect("import dxf");
    let _ = fs::remove_file(&path);
    let Geometry::LwPolyline { vertices, .. } = &loaded.model_space[0].geometry else {
        panic!(
            "expected an LWPOLYLINE, counts {:?}",
            loaded.diagnostics.entity_counts
        );
    };
    assert!(
        (vertices[0].start_width - 1.0).abs() < 1e-6,
        "start {}",
        vertices[0].start_width
    );
    assert!(
        (vertices[0].end_width - 3.0).abs() < 1e-6,
        "end {}",
        vertices[0].end_width
    );
}

/// The sort table's `block_owner` points at a line, not the block record.
/// The table still belongs to model space through its dictionary owner.
#[test]
fn sort_table_owned_only_by_its_dictionary() {
    let path = temp_dxf();
    let dxf = concat!(
        "  0\nSECTION\n  2\nHEADER\n  9\n$ACADVER\n  1\nAC1015\n  9\n$HANDSEED\n  5\n200\n  0\nENDSEC\n",
        "  0\nSECTION\n  2\nTABLES\n",
        "  0\nTABLE\n  2\nLAYER\n  5\n2\n330\n0\n100\nAcDbSymbolTable\n 70\n     1\n",
        "  0\nLAYER\n  5\n10\n330\n2\n100\nAcDbSymbolTableRecord\n100\nAcDbLayerTableRecord\n",
        "  2\n0\n 70\n     0\n 62\n     7\n  6\nCONTINUOUS\n  0\nENDTAB\n",
        "  0\nTABLE\n  2\nBLOCK_RECORD\n  5\n1\n330\n0\n100\nAcDbSymbolTable\n 70\n     1\n",
        "  0\nBLOCK_RECORD\n  5\n1F\n330\n1\n100\nAcDbSymbolTableRecord\n100\nAcDbBlockTableRecord\n",
        "  2\n*MODEL_SPACE\n  0\nENDTAB\n  0\nENDSEC\n",
        "  0\nSECTION\n  2\nBLOCKS\n",
        "  0\nBLOCK\n  5\n20\n330\n1F\n100\nAcDbEntity\n  8\n0\n100\nAcDbBlockBegin\n",
        "  2\n*MODEL_SPACE\n 70\n     0\n 10\n0.0\n 20\n0.0\n 30\n0.0\n  3\n*MODEL_SPACE\n  1\n\n",
        "  0\nENDBLK\n  5\n21\n330\n1F\n100\nAcDbEntity\n  8\n0\n100\nAcDbBlockEnd\n  0\nENDSEC\n",
        "  0\nSECTION\n  2\nENTITIES\n",
        "  0\nLINE\n  5\n22\n330\n1F\n100\nAcDbEntity\n  8\n0\n100\nAcDbLine\n",
        " 10\n0.0\n 20\n0.0\n 30\n0.0\n 11\n1.0\n 21\n0.0\n 31\n0.0\n",
        "  0\nLINE\n  5\n23\n330\n1F\n100\nAcDbEntity\n  8\n0\n100\nAcDbLine\n",
        " 10\n0.0\n 20\n1.0\n 30\n0.0\n 11\n1.0\n 21\n1.0\n 31\n0.0\n",
        "  0\nENDSEC\n  0\nSECTION\n  2\nOBJECTS\n",
        "  0\nDICTIONARY\n  5\nC\n330\n1F\n100\nAcDbDictionary\n  3\nACAD_SORTENTS\n350\nE\n",
        "  0\nSORTENTSTABLE\n  5\nE\n330\nC\n100\nAcDbSortentsTable\n330\n22\n",
        "331\n22\n  5\n100\n331\n23\n  5\n10\n",
        "  0\nENDSEC\n  0\nEOF\n"
    );
    fs::write(&path, dxf).expect("write dxf");
    let document = import_dxf(&path).expect("import dxf");
    let _ = fs::remove_file(&path);
    let counts = &document.diagnostics.feature_counts;
    let mut front = None;
    let mut back = None;
    for entity in &document.model_space {
        let Geometry::Line { start, .. } = &entity.geometry else {
            continue;
        };
        if (start.y - 0.0).abs() < 1e-6 {
            front = Some(entity.draw_order);
        } else if (start.y - 1.0).abs() < 1e-6 {
            back = Some(entity.draw_order);
        }
    }
    assert_eq!(
        counts.get("SORT_NO_OWNER").copied(),
        None,
        "sort owner was not recovered: {counts:?} orders {front:?} {back:?}"
    );
    assert_eq!(counts.get("SORT_APPLIED").copied(), Some(1), "{counts:?}");
    assert!(
        front.is_some_and(|order| back.is_some_and(|behind| order > behind)),
        "the higher sort handle should draw in front: front={front:?} back={back:?}"
    );
}

fn width_pairs(vertices: &[PolyVertex]) -> Vec<(f64, f64)> {
    vertices
        .iter()
        .map(|vertex| (vertex.start_width, vertex.end_width))
        .collect()
}
