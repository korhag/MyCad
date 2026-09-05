//! Canonical primitive drawing used by save/import round-trip tests.

use crate::color::CadColor;
use crate::document::{BlockDefinition, Document, DrawingUnits, Layer};
use crate::entity::{
    default_extrusion, Entity, Geometry, HatchData, HatchPath, MTextData, PolyVertex, TextData,
};
use crate::geom::Point3;
use crate::linetype::LineType;

// ------------------------------------------------------------
// Function: primitives_document
// Purpose: One native drawing covering every round-trip primitive.
// ------------------------------------------------------------
pub fn primitives_document() -> Document {
    let mut document = Document::default();
    document.units = DrawingUnits::Millimeters;
    document.ltscale = 2.0;
    document
        .linetypes
        .insert("CONTINUOUS".into(), LineType::builtin("CONTINUOUS"));
    document
        .linetypes
        .insert("DASHED".into(), LineType::builtin("DASHED"));
    document
        .linetypes
        .insert("CENTER".into(), LineType::builtin("CENTER"));
    document.layers.insert(
        "STRUCTURE".into(),
        Layer {
            name: "STRUCTURE".into(),
            visible: true,
            frozen: false,
            color: CadColor::Aci(1),
            linetype: "DASHED".into(),
        },
    );
    document.layers.insert(
        "ANNOTATION".into(),
        Layer {
            name: "ANNOTATION".into(),
            visible: true,
            frozen: false,
            color: CadColor::Aci(4),
            linetype: "CONTINUOUS".into(),
        },
    );
    document.current_layer = "STRUCTURE".into();

    document.blocks.insert(
        "LEAF".into(),
        BlockDefinition {
            name: "LEAF".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![{
                let mut line = Entity::new(Geometry::Line {
                    start: Point3::from_xy(0.0, 0.0),
                    end: Point3::from_xy(10.0, 0.0),
                });
                line.color = CadColor::ByBlock;
                line.layer = "0".into();
                line
            }],
            ..Default::default()
        },
    );
    document.blocks.insert(
        "NESTED".into(),
        BlockDefinition {
            name: "NESTED".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![{
                let mut insert = Entity::new(Geometry::Insert {
                    block_name: "LEAF".into(),
                    insertion: Point3::from_xy(2.0, 3.0),
                    scale: Point3::new(1.0, 1.0, 1.0),
                    rotation: 0.0,
                    extrusion: default_extrusion(),
                    attribs: Vec::new(),
                    column_count: 1,
                    row_count: 1,
                    column_spacing: 0.0,
                    row_spacing: 0.0,
                    configuration: None,
                });
                insert.layer = "0".into();
                insert.color = CadColor::ByLayer;
                insert
            }],
            ..Default::default()
        },
    );

    let mut negative = Entity::new(Geometry::Line {
        start: Point3::from_xy(-1250.5, -800.25),
        end: Point3::from_xy(-10.0, 20.0),
    });
    negative.layer = "STRUCTURE".into();
    negative.color = CadColor::ByLayer;
    negative.linetype = "BYLAYER".into();
    document.add_entity(negative);

    let mut large = Entity::new(Geometry::Line {
        start: Point3::new(1_000_000.0, 2_000_000.0, 12.5),
        end: Point3::new(1_000_010.0, 2_000_000.0, 12.5),
    });
    large.layer = "0".into();
    large.color = CadColor::Aci(3);
    large.linetype = "CENTER".into();
    large.linetype_scale = 0.5;
    document.add_entity(large);

    document.add_entity(Entity::new(Geometry::Arc {
        center: Point3::from_xy(50.0, 60.0),
        radius: 15.0,
        start_angle: 0.25,
        end_angle: 2.1,
        extrusion: default_extrusion(),
    }));
    document.add_entity(Entity::new(Geometry::Circle {
        center: Point3::from_xy(80.0, -40.0),
        radius: 12.5,
        extrusion: default_extrusion(),
    }));
    document.add_entity(Entity::new(Geometry::Ellipse {
        center: Point3::from_xy(200.0, 10.0),
        major_axis: Point3::from_xy(25.0, 0.0),
        axis_ratio: 0.4,
        start_param: 0.0,
        end_param: std::f64::consts::TAU,
        extrusion: default_extrusion(),
    }));

    let mut bulged = Entity::new(Geometry::LwPolyline {
        vertices: vec![
            PolyVertex {
                point: Point3::from_xy(0.0, 100.0),
                bulge: 0.0,
            vertex_id: Default::default(),
        },
            PolyVertex {
                point: Point3::from_xy(40.0, 100.0),
                bulge: 0.5,
            vertex_id: Default::default(),
        },
            PolyVertex {
                point: Point3::from_xy(40.0, 140.0),
                bulge: 0.0,
            vertex_id: Default::default(),
        },
        ],
        closed: false,
        extrusion: default_extrusion(),
        linetype_generation_continuous: true,
    });
    bulged.layer = "STRUCTURE".into();
    document.add_entity(bulged);

    document.add_entity(Entity::new(Geometry::LwPolyline {
        vertices: vec![
            PolyVertex {
                point: Point3::from_xy(300.0, 300.0),
                bulge: 0.0,
            vertex_id: Default::default(),
        },
            PolyVertex {
                point: Point3::from_xy(360.0, 300.0),
                bulge: 0.0,
            vertex_id: Default::default(),
        },
            PolyVertex {
                point: Point3::from_xy(360.0, 340.0),
                bulge: 0.0,
            vertex_id: Default::default(),
        },
            PolyVertex {
                point: Point3::from_xy(300.0, 340.0),
                bulge: 0.0,
            vertex_id: Default::default(),
        },
        ],
        closed: true,
        extrusion: default_extrusion(),
        linetype_generation_continuous: false,
    }));

    let mut text = Entity::new(Geometry::Text(TextData {
        insertion: Point3::from_xy(5.0, 5.0),
        height: 3.5,
        rotation: std::f64::consts::FRAC_PI_6,
        value: "Round-trip".into(),
        extrusion: default_extrusion(),
        is_attrib_def: false,
    }));
    text.layer = "ANNOTATION".into();
    document.add_entity(text);

    let mut mtext = Entity::new(Geometry::MText(MTextData {
        insertion: Point3::from_xy(5.0, 15.0),
        height: 2.5,
        rotation: 0.0,
        width: 40.0,
        value: "MTEXT sample".into(),
        extrusion: default_extrusion(),
    }));
    mtext.layer = "ANNOTATION".into();
    document.add_entity(mtext);

    let mut insert = Entity::new(Geometry::Insert {
        block_name: "NESTED".into(),
        insertion: Point3::from_xy(400.0, -50.0),
        scale: Point3::new(2.0, 2.0, 1.0),
        rotation: std::f64::consts::FRAC_PI_4,
        extrusion: default_extrusion(),
        attribs: Vec::new(),
        column_count: 1,
        row_count: 1,
        column_spacing: 0.0,
        row_spacing: 0.0,
        configuration: None,
    });
    insert.color = CadColor::Aci(6);
    insert.layer = "0".into();
    document.add_entity(insert);

    document.add_entity(Entity::new(Geometry::Solid {
        corners: [
            Point3::from_xy(500.0, 0.0),
            Point3::from_xy(520.0, 0.0),
            Point3::from_xy(520.0, 10.0),
            Point3::from_xy(500.0, 10.0),
        ],
        extrusion: default_extrusion(),
    }));

    document.add_entity(Entity::new(Geometry::Hatch(HatchData {
        extrusion: default_extrusion(),
        elevation: 0.0,
        solid_fill: true,
        paths: vec![HatchPath::Polyline {
            vertices: vec![
                PolyVertex {
                    point: Point3::from_xy(600.0, 0.0),
                    bulge: 0.0,
                vertex_id: Default::default(),
        },
                PolyVertex {
                    point: Point3::from_xy(630.0, 0.0),
                    bulge: 0.0,
                vertex_id: Default::default(),
        },
                PolyVertex {
                    point: Point3::from_xy(630.0, 20.0),
                    bulge: 0.0,
                vertex_id: Default::default(),
        },
                PolyVertex {
                    point: Point3::from_xy(600.0, 20.0),
                    bulge: 0.0,
                vertex_id: Default::default(),
        },
            ],
            closed: true,
        }],
        pattern_lines: Vec::new(),
    })));

    document.add_entity(Entity::new(Geometry::Leader {
        vertices: vec![
            Point3::from_xy(700.0, 0.0),
            Point3::from_xy(720.0, 15.0),
            Point3::from_xy(740.0, 15.0),
        ],
    }));

    document.add_entity(Entity::new(Geometry::Point {
        position: Point3::new(15.0, 25.0, 7.5),
    }));

    document.add_entity(Entity::new(Geometry::Polyline {
        vertices: vec![
            PolyVertex {
                point: Point3::from_xy(800.0, 10.0),
                bulge: 0.0,
            vertex_id: Default::default(),
        },
            PolyVertex {
                point: Point3::from_xy(830.0, 10.0),
                bulge: 0.0,
            vertex_id: Default::default(),
        },
            PolyVertex {
                point: Point3::from_xy(830.0, 40.0),
                bulge: 0.0,
            vertex_id: Default::default(),
        },
        ],
        closed: false,
        linetype_generation_continuous: false,
    }));

    document.add_entity(Entity::new(Geometry::LwPolyline {
        vertices: vec![
            PolyVertex {
                point: Point3::from_xy(0.0, -50.0),
                bulge: 0.0,
            vertex_id: Default::default(),
        },
            PolyVertex {
                point: Point3::from_xy(30.0, -50.0),
                bulge: -0.5,
            vertex_id: Default::default(),
        },
            PolyVertex {
                point: Point3::from_xy(30.0, -20.0),
                bulge: 0.0,
            vertex_id: Default::default(),
        },
        ],
        closed: false,
        extrusion: default_extrusion(),
        linetype_generation_continuous: false,
    }));

    document.add_entity(Entity::new(Geometry::Spline {
        degree: 3,
        control_points: vec![
            Point3::from_xy(0.0, 200.0),
            Point3::from_xy(20.0, 240.0),
            Point3::from_xy(40.0, 160.0),
            Point3::from_xy(60.0, 200.0),
        ],
        fit_points: Vec::new(),
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        weights: vec![1.0, 1.0, 1.0, 1.0],
        closed: false,
    }));

    document.assign_missing_ids();
    document
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_covers_required_primitives() {
        let document = primitives_document();
        let names: Vec<_> = document
            .model_space
            .iter()
            .map(|entity| entity.geometry.type_name())
            .collect();
        for required in [
            "Line", "Point", "Arc", "Circle", "Ellipse", "Polyline", "Spline", "Text", "MText",
            "Block", "Solid", "Hatch", "Leader",
        ] {
            assert!(
                names.contains(&required),
                "fixture missing {required}, have {names:?}"
            );
        }
        assert!(document.layers.contains_key("STRUCTURE"));
        assert!(document.layers.contains_key("ANNOTATION"));
        assert!(document.linetypes.contains_key("DASHED"));
        assert!(document.linetypes.contains_key("CENTER"));
        assert!(document.blocks.contains_key("NESTED"));
        assert!(document.blocks.contains_key("LEAF"));
        assert_eq!(document.units, DrawingUnits::Millimeters);
        assert!(document.model_space.iter().any(|entity| matches!(
            &entity.geometry,
            Geometry::Line { start, .. } if start.x < 0.0
        )));
        assert!(document.model_space.iter().any(|entity| matches!(
            &entity.geometry,
            Geometry::Line { start, .. } if start.x >= 1_000_000.0
        )));
        assert!(document.model_space.iter().any(|entity| matches!(
            &entity.geometry,
            Geometry::LwPolyline { vertices, closed: false, .. }
                if vertices.iter().any(|vertex| vertex.bulge > 1e-9)
        )));
        assert!(document.model_space.iter().any(|entity| matches!(
            &entity.geometry,
            Geometry::LwPolyline { vertices, closed: false, .. }
                if vertices.iter().any(|vertex| vertex.bulge < -1e-9)
        )));
        assert!(document
            .model_space
            .iter()
            .any(|entity| matches!(&entity.geometry, Geometry::LwPolyline { closed: true, .. })));
        assert!(document
            .model_space
            .iter()
            .any(|entity| matches!(&entity.geometry, Geometry::Polyline { .. })));
        assert!(document.model_space.iter().any(|entity| {
            matches!(&entity.geometry, Geometry::Point { position } if position.z.abs() > 1e-9)
        }));
        assert!(document.model_space.iter().any(|entity| matches!(
            &entity.geometry,
            Geometry::Line { start, .. } if start.z.abs() > 1e-9
        )));
        assert!(document
            .model_space
            .iter()
            .any(|entity| (entity.linetype_scale - 0.5).abs() < 1e-12));
        assert!(document
            .model_space
            .iter()
            .any(|entity| { entity.color == CadColor::ByLayer }));
        let leaf_line = &document.blocks["LEAF"].entities[0];
        assert_eq!(leaf_line.color, CadColor::ByBlock);
    }
}
