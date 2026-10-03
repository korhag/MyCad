//! Canonical primitive drawing used by save/import round-trip tests.

use crate::color::CadColor;
use crate::document::{BlockDefinition, Document, DrawingUnits, Layer};
use crate::entity::{
    default_extrusion, AttributeInfo, DimensionData, Entity, Geometry, HatchData, HatchPath,
    HatchPatternLine, MTextData, PolyVertex, TextData, ViewportData, ATTRIB_INVISIBLE,
};
use crate::geom::{Point2, Point3};
use crate::linetype::{LineType, LineTypeShape};
use crate::PaperLayout;
use crate::TextStyle;

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
            ..Layer::default()
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
            ..Layer::default()
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
        ..Default::default()
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
        ..Default::default()
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
        ..HatchData::default()
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

// ------------------------------------------------------------
// Function: autocad_features_document
// Purpose: One drawing that grows with each AutoCAD fidelity release.
//          Round-trip and ODA audit both start from this document.
// ------------------------------------------------------------
pub fn autocad_features_document() -> Document {
    let mut document = Document::default();
    document.units = DrawingUnits::Millimeters;
    document.text_styles.insert(
        "NOTES".into(),
        TextStyle {
            name: "NOTES".into(),
            font_file: "romans.shx".into(),
            width_factor: 0.8,
            ..TextStyle::standard()
        },
    );
    document.layers.insert(
        "NOPLOT".into(),
        Layer {
            name: "NOPLOT".into(),
            locked: true,
            plot: false,
            color: CadColor::Rgb {
                r: 18,
                g: 52,
                b: 86,
            },
            lineweight: 50,
            ..Layer::default()
        },
    );

    let mut kept = Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 0.0),
        end: Point3::from_xy(20.0, 0.0),
    });
    kept.layer = "NOPLOT".into();
    kept.lineweight = 25;
    document.add_entity(kept);

    document.add_entity(Entity::new(Geometry::Text(TextData {
        insertion: Point3::from_xy(1.0, 2.0),
        height: 3.0,
        value: "Styled".into(),
        style: "NOTES".into(),
        ..TextData::default()
    })));

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

    document.add_entity(Entity::new(Geometry::Hatch(HatchData {
        solid_fill: false,
        pattern_name: "ANSI31".into(),
        pattern_scale: 2.0,
        pattern_angle: 0.0,
        pattern_type: 1,
        paths: vec![HatchPath::Polyline {
            vertices: vec![
                PolyVertex {
                    point: Point3::from_xy(0.0, 10.0),
                    bulge: 0.0,
                    vertex_id: Default::default(),
                },
                PolyVertex {
                    point: Point3::from_xy(10.0, 10.0),
                    bulge: 0.0,
                    vertex_id: Default::default(),
                },
                PolyVertex {
                    point: Point3::from_xy(10.0, 20.0),
                    bulge: 0.0,
                    vertex_id: Default::default(),
                },
                PolyVertex {
                    point: Point3::from_xy(0.0, 20.0),
                    bulge: 0.0,
                    vertex_id: Default::default(),
                },
            ],
            closed: true,
        }],
        pattern_lines: vec![HatchPatternLine {
            angle: 0.0,
            base: Point3::from_xy(0.0, 0.0),
            offset: Point3::from_xy(0.0, 5.0),
            dashes: Vec::new(),
        }],
        ..HatchData::default()
    })));

    document.blocks.insert(
        "*D1".into(),
        BlockDefinition {
            name: "*D1".into(),
            entities: vec![Entity::new(Geometry::Line {
                start: Point3::from_xy(0.0, 30.0),
                end: Point3::from_xy(40.0, 30.0),
            })],
            ..BlockDefinition::default()
        },
    );
    document.add_entity(Entity::new(Geometry::Dimension(DimensionData {
        block_name: "*D1".into(),
        definition: Point3::from_xy(0.0, 30.0),
        text_midpoint: Point3::from_xy(20.0, 35.0),
        extension1: Point3::from_xy(0.0, 30.0),
        extension2: Point3::from_xy(40.0, 30.0),
        text: "40".into(),
        ..DimensionData::default()
    })));

    document.blocks.insert(
        "SITE".into(),
        BlockDefinition {
            name: "SITE".into(),
            xref_path: "site.dwg".into(),
            xref_overlay: true,
            ..BlockDefinition::default()
        },
    );

    document
        .layouts
        .push(PaperLayout::sheet("Layout1", "*PAPER_SPACE", 1));
    document.blocks.insert(
        "*PAPER_SPACE".into(),
        BlockDefinition {
            name: "*PAPER_SPACE".into(),
            entities: vec![Entity::new(Geometry::Viewport(ViewportData {
                center: Point3::from_xy(105.0, 148.0),
                width: 180.0,
                height: 120.0,
                view_center: Point2::new(0.0, 0.0),
                view_height: 100.0,
                ..ViewportData::default()
            }))],
            ..BlockDefinition::default()
        },
    );

    document.linetypes.insert(
        "FENCELINE1".into(),
        LineType {
            name: "FENCELINE1".into(),
            dashes: vec![0.25, -0.1, 0.0, -0.1],
            shapes: vec![
                None,
                None,
                Some(LineTypeShape {
                    flag: 4,
                    shapecode: 130,
                    scale: 0.1,
                    x_offset: -0.05,
                    style: String::new(),
                    shape_file: "ltypeshp.shx".into(),
                    ..LineTypeShape::default()
                }),
                None,
            ],
        },
    );
    document.linetypes.insert(
        "AMZIGZAG".into(),
        LineType {
            name: "AMZIGZAG".into(),
            dashes: vec![0.5, -0.2],
            shapes: vec![
                Some(LineTypeShape {
                    flag: 2,
                    text: "Z".into(),
                    scale: 1.0,
                    rotation: 0.3,
                    y_offset: 0.05,
                    style: "STANDARD".into(),
                    ..LineTypeShape::default()
                }),
                None,
            ],
        },
    );
    let mut fence = Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 50.0),
        end: Point3::from_xy(40.0, 50.0),
    });
    fence.linetype = "FENCELINE1".into();
    document.add_entity(fence);
    let mut zigzag = Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 55.0),
        end: Point3::from_xy(40.0, 55.0),
    });
    zigzag.linetype = "AMZIGZAG".into();
    document.add_entity(zigzag);

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
