use crate::{
    box_select, hit_test, tessellate_document, DisplayList, SelectBoxMode,
    DEFAULT_PICK_TOLERANCE_PX,
};
use cad_core::Point2;
use cad_core::{
    BlockDefinition, CadColor, Document, Entity, Extents2, Geometry, Layer, Point3, PolyVertex,
    Transform2,
};
use cad_viewport::Camera2;

fn layer0(document: &mut Document) {
    document.layers.insert(
        "0".into(),
        Layer {
            name: "0".into(),
            visible: true,
            frozen: false,
            color: CadColor::Aci(7),
            linetype: "CONTINUOUS".into(),
        },
    );
}

fn camera_looking_at(center: Point2, view_height: f64) -> Camera2 {
    Camera2 {
        center,
        view_height,
    }
}

fn vp() -> (Point2, Point2) {
    (Point2::new(0.0, 0.0), Point2::new(800.0, 600.0))
}

fn pick_world(document: &Document, world: Point2) -> Option<usize> {
    let list = tessellate_document(document);
    let camera = camera_looking_at(world, 100.0);
    let (origin, size) = vp();
    let screen = camera.world_to_screen(world, origin, size);
    hit_test(
        &list.picks,
        &camera,
        screen,
        origin,
        size,
        DEFAULT_PICK_TOLERANCE_PX,
    )
}

#[test]
fn tessellates_line_into_two_vertices() {
    let mut document = Document::default();
    layer0(&mut document);
    document.model_space.push(Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 0.0),
        end: Point3::from_xy(10.0, 0.0),
    }));
    let list = tessellate_document(&document);
    assert_eq!(list.line_count(), 1);
    assert_eq!(list.picks.len(), 1);
    assert_eq!(list.picks[0].entity_index, 0);
}

#[test]
fn picks_a_line_near_its_midpoint() {
    let mut document = Document::default();
    layer0(&mut document);
    document.model_space.push(Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 0.0),
        end: Point3::from_xy(10.0, 0.0),
    }));
    assert_eq!(pick_world(&document, Point2::new(5.0, 0.0)), Some(0));
    assert_eq!(pick_world(&document, Point2::new(50.0, 50.0)), None);
}

#[test]
fn picks_a_circle_on_the_circumference_not_the_center() {
    let mut document = Document::default();
    layer0(&mut document);
    document.model_space.push(Entity::new(Geometry::Circle {
        center: Point3::from_xy(0.0, 0.0),
        radius: 10.0,
        extrusion: Point3::new(0.0, 0.0, 1.0),
    }));
    assert_eq!(pick_world(&document, Point2::new(10.0, 0.0)), Some(0));
    assert_eq!(pick_world(&document, Point2::new(0.0, 0.0)), None);
}

#[test]
fn picks_a_polyline_along_a_segment() {
    let mut document = Document::default();
    layer0(&mut document);
    document.model_space.push(Entity::new(Geometry::LwPolyline {
        vertices: vec![
            PolyVertex {
                point: Point3::from_xy(0.0, 0.0),
                bulge: 0.0,
            },
            PolyVertex {
                point: Point3::from_xy(8.0, 0.0),
                bulge: 0.0,
            },
            PolyVertex {
                point: Point3::from_xy(8.0, 6.0),
                bulge: 0.0,
            },
        ],
        closed: false,
        extrusion: Point3::new(0.0, 0.0, 1.0),
        linetype_generation_continuous: false,
    }));
    assert_eq!(pick_world(&document, Point2::new(4.0, 0.0)), Some(0));
    assert_eq!(pick_world(&document, Point2::new(8.0, 3.0)), Some(0));
    assert_eq!(pick_world(&document, Point2::new(4.0, 6.0)), None);
}

#[test]
fn picks_filled_solid_from_its_interior() {
    let mut document = Document::default();
    layer0(&mut document);
    document.model_space.push(Entity::new(Geometry::Solid {
        corners: [
            Point3::from_xy(0.0, 0.0),
            Point3::from_xy(10.0, 0.0),
            Point3::from_xy(10.0, 10.0),
            Point3::from_xy(0.0, 10.0),
        ],
        extrusion: Point3::new(0.0, 0.0, 1.0),
    }));
    assert_eq!(pick_world(&document, Point2::new(5.0, 5.0)), Some(0));
    assert_eq!(pick_world(&document, Point2::new(40.0, 40.0)), None);
}

#[test]
fn overlapping_strokes_prefer_the_closer_entity() {
    let mut document = Document::default();
    layer0(&mut document);
    document.model_space.push(Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 0.0),
        end: Point3::from_xy(20.0, 0.0),
    }));
    document.model_space.push(Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 1.0),
        end: Point3::from_xy(20.0, 1.0),
    }));
    assert_eq!(pick_world(&document, Point2::new(10.0, 0.9)), Some(1));
    assert_eq!(pick_world(&document, Point2::new(10.0, 0.1)), Some(0));
}

#[test]
fn overlapping_equal_distance_uses_later_draw_order() {
    let mut document = Document::default();
    layer0(&mut document);
    document.model_space.push(Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 0.0),
        end: Point3::from_xy(20.0, 0.0),
    }));
    document.model_space.push(Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 0.0),
        end: Point3::from_xy(20.0, 0.0),
    }));
    assert_eq!(pick_world(&document, Point2::new(10.0, 0.0)), Some(1));
}

#[test]
fn nested_block_picks_the_parent_insert() {
    let mut document = Document::default();
    layer0(&mut document);
    document.blocks.insert(
        "SYM".into(),
        BlockDefinition {
            name: "SYM".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![Entity::new(Geometry::Line {
                start: Point3::from_xy(0.0, 0.0),
                end: Point3::from_xy(10.0, 0.0),
            })],
        },
    );
    document.model_space.push(Entity::new(Geometry::Insert {
        block_name: "SYM".into(),
        insertion: Point3::from_xy(100.0, 50.0),
        scale: Point3::new(2.0, 2.0, 1.0),
        rotation: 0.0,
        extrusion: Point3::new(0.0, 0.0, 1.0),
        attribs: Vec::new(),
        column_count: 1,
        row_count: 1,
        column_spacing: 0.0,
        row_spacing: 0.0,
    }));
    assert_eq!(pick_world(&document, Point2::new(110.0, 50.0)), Some(0));
    assert_eq!(pick_world(&document, Point2::new(0.0, 0.0)), None);
}

#[test]
fn stroke_on_a_fill_wins_within_the_aperture() {
    let mut document = Document::default();
    layer0(&mut document);
    document.model_space.push(Entity::new(Geometry::Solid {
        corners: [
            Point3::from_xy(0.0, 0.0),
            Point3::from_xy(20.0, 0.0),
            Point3::from_xy(20.0, 20.0),
            Point3::from_xy(0.0, 20.0),
        ],
        extrusion: Point3::new(0.0, 0.0, 1.0),
    }));
    document.model_space.push(Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 10.0),
        end: Point3::from_xy(20.0, 10.0),
    }));
    assert_eq!(pick_world(&document, Point2::new(10.0, 10.0)), Some(1));
}

fn dashed() -> cad_core::LineType {
    cad_core::LineType {
        name: "DASHED".into(),
        dashes: vec![12.0, -6.0],
    }
}

#[test]
fn bylayer_resolves_layer_linetype() {
    let mut document = Document::default();
    layer0(&mut document);
    document.layers.get_mut("0").unwrap().linetype = "DASHED".into();
    document.linetypes.insert("DASHED".into(), dashed());
    document.model_space.push(Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 0.0),
        end: Point3::from_xy(180.0, 0.0),
    }));
    let list = tessellate_document(&document);
    assert!(
        list.line_count() > 2,
        "ByLayer DASHED must emit multiple dashes, got {}",
        list.line_count()
    );
}

#[test]
fn ltscale_scales_pattern() {
    let mut a = Document::default();
    layer0(&mut a);
    a.linetypes.insert("DASHED".into(), dashed());
    let mut entity = Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 0.0),
        end: Point3::from_xy(180.0, 0.0),
    });
    entity.linetype = "DASHED".into();
    a.model_space.push(entity.clone());
    a.ltscale = 1.0;
    let n1 = tessellate_document(&a).line_count();

    let mut b = a.clone();
    b.ltscale = 2.0;
    let n2 = tessellate_document(&b).line_count();
    assert!(
        n2 < n1,
        "larger LTSCALE must produce fewer dashes ({n2} vs {n1})"
    );
}

#[test]
fn nested_byblock_inherits_insert_linetype() {
    let mut document = Document::default();
    layer0(&mut document);
    document.linetypes.insert("DASHED".into(), dashed());
    let mut child = Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 0.0),
        end: Point3::from_xy(180.0, 0.0),
    });
    child.linetype = "BYBLOCK".into();
    document.blocks.insert(
        "SYM".into(),
        BlockDefinition {
            name: "SYM".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![child],
        },
    );
    let mut insert = Entity::new(Geometry::Insert {
        block_name: "SYM".into(),
        insertion: Point3::from_xy(0.0, 0.0),
        scale: Point3::new(1.0, 1.0, 1.0),
        rotation: 0.0,
        extrusion: Point3::new(0.0, 0.0, 1.0),
        attribs: Vec::new(),
        column_count: 1,
        row_count: 1,
        column_spacing: 0.0,
        row_spacing: 0.0,
    });
    insert.linetype = "DASHED".into();
    document.model_space.push(insert);
    let list = tessellate_document(&document);
    assert!(
        list.line_count() > 2,
        "ByBlock child must inherit INSERT DASHED, got {}",
        list.line_count()
    );
}

#[test]
fn unparented_byblock_is_continuous() {
    let mut document = Document::default();
    layer0(&mut document);
    document.linetypes.insert("DASHED".into(), dashed());
    let mut entity = Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 0.0),
        end: Point3::from_xy(180.0, 0.0),
    });
    entity.linetype = "BYBLOCK".into();
    document.model_space.push(entity);
    let list = tessellate_document(&document);
    assert_eq!(list.line_count(), 1);
}

#[test]
fn dashed_polyline_remains_pickable_in_gaps() {
    let mut document = Document::default();
    layer0(&mut document);
    document.layers.get_mut("0").unwrap().linetype = "DASHED".into();
    document.linetypes.insert("DASHED".into(), dashed());
    document.model_space.push(Entity::new(Geometry::LwPolyline {
        vertices: vec![
            PolyVertex {
                point: Point3::from_xy(0.0, 0.0),
                bulge: 0.0,
            },
            PolyVertex {
                point: Point3::from_xy(8.0, 0.0),
                bulge: 0.0,
            },
            PolyVertex {
                point: Point3::from_xy(8.0, 6.0),
                bulge: 0.0,
            },
        ],
        closed: false,
        extrusion: Point3::new(0.0, 0.0, 1.0),
        linetype_generation_continuous: true,
    }));
    assert_eq!(pick_world(&document, Point2::new(4.0, 0.0)), Some(0));
    assert_eq!(pick_world(&document, Point2::new(8.0, 3.0)), Some(0));
}

#[test]
fn explicit_center_uses_imported_definition() {
    let mut document = Document::default();
    layer0(&mut document);
    document.linetypes.insert(
        "CENTER".into(),
        cad_core::LineType {
            name: "CENTER".into(),
            dashes: vec![32.0, -6.0, 4.0, -6.0],
        },
    );
    let mut entity = Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 0.0),
        end: Point3::from_xy(96.0, 0.0),
    });
    entity.linetype = "CENTER".into();
    document.model_space.push(entity);
    let list = tessellate_document(&document);
    assert!(list.line_count() >= 2);
}

fn p(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

fn separated_line_pairs() -> [(Point2, Point2); 3] {
    [
        (p(0.0, 0.0), p(10.0, 0.0)),
        (p(100.0, 100.0), p(110.0, 100.0)),
        (p(-200.0, 50.0), p(-190.0, 50.0)),
    ]
}

fn line_entity(a: Point2, b: Point2) -> Entity {
    Entity::new(Geometry::Line {
        start: Point3::from_xy(a.x, a.y),
        end: Point3::from_xy(b.x, b.y),
    })
}

fn insert_entity(name: &str, insertion: Point3, scale: Point3, rotation: f64) -> Entity {
    Entity::new(Geometry::Insert {
        block_name: name.into(),
        insertion,
        scale,
        rotation,
        extrusion: Point3::new(0.0, 0.0, 1.0),
        attribs: Vec::new(),
        column_count: 1,
        row_count: 1,
        column_spacing: 0.0,
        row_spacing: 0.0,
    })
}

fn separated_block(name: &str) -> BlockDefinition {
    let pairs = separated_line_pairs();
    BlockDefinition {
        name: name.into(),
        base_pt: Point3::from_xy(0.0, 0.0),
        entities: pairs.into_iter().map(|(a, b)| line_entity(a, b)).collect(),
    }
}

fn document_with_insert(insertion: Point3, scale: Point3, rotation: f64) -> Document {
    let mut document = Document::default();
    layer0(&mut document);
    document.blocks.insert("SEP".into(), separated_block("SEP"));
    document
        .model_space
        .push(insert_entity("SEP", insertion, scale, rotation));
    document
}

fn world_line_pairs(list: &DisplayList) -> Vec<[Point2; 2]> {
    let origin = list.origin;
    list.line_vertices
        .chunks_exact(2)
        .map(|pair| {
            [
                Point2::new(
                    origin.x + pair[0].position[0] as f64,
                    origin.y + pair[0].position[1] as f64,
                ),
                Point2::new(
                    origin.x + pair[1].position[0] as f64,
                    origin.y + pair[1].position[1] as f64,
                ),
            ]
        })
        .collect()
}

fn same_segment(a: [Point2; 2], b: [Point2; 2], eps: f64) -> bool {
    (near(a[0], b[0], eps) && near(a[1], b[1], eps))
        || (near(a[0], b[1], eps) && near(a[1], b[0], eps))
}

fn near(a: Point2, b: Point2, eps: f64) -> bool {
    (a.x - b.x).abs() <= eps && (a.y - b.y).abs() <= eps
}

fn contains_segment(edges: &[[Point2; 2]], needle: [Point2; 2], eps: f64) -> bool {
    edges.iter().any(|edge| same_segment(*edge, needle, eps))
}

fn assert_segments_match(pick: &[[Point2; 2]], gpu: &[[Point2; 2]], expected: &[[Point2; 2]]) {
    assert_eq!(pick.len(), expected.len());
    assert_eq!(gpu.len(), expected.len());
    for segment in expected {
        assert!(
            contains_segment(pick, *segment, 1e-9),
            "missing pick edge {segment:?}"
        );
        assert!(
            contains_segment(gpu, *segment, 1e-3),
            "missing display-list edge {segment:?}"
        );
    }
}

fn expected_world_pairs(transform: Transform2) -> Vec<[Point2; 2]> {
    separated_line_pairs()
        .into_iter()
        .map(|(a, b)| [transform.apply(a), transform.apply(b)])
        .collect()
}

fn connector_needles(expected: &[[Point2; 2]]) -> Vec<[Point2; 2]> {
    vec![
        [expected[0][1], expected[1][0]],
        [expected[1][1], expected[2][0]],
    ]
}

fn assert_no_connectors(edges: &[[Point2; 2]], expected: &[[Point2; 2]]) {
    for needle in connector_needles(expected) {
        assert!(
            !contains_segment(edges, needle, 1e-6),
            "false connector {:?}",
            needle
        );
    }
}

fn insert_transform(insertion: Point3, scale: Point3, rotation: f64) -> Transform2 {
    Transform2::block_insert(
        insertion,
        scale,
        rotation,
        Point3::new(0.0, 0.0, 1.0),
        Point3::from_xy(0.0, 0.0),
    )
}

fn assert_selected_insert_matches(document: &Document, transform: Transform2) {
    let list = tessellate_document(document);
    let pick = list.pick_for(0).expect("parent INSERT should be pickable");
    let pick_edges: Vec<_> = pick.stroke_edges().collect();
    let gpu_edges = world_line_pairs(&list);
    let expected = expected_world_pairs(transform);
    assert_no_connectors(&pick_edges, &expected);
    assert_no_connectors(&gpu_edges, &expected);
    assert_segments_match(&pick_edges, &gpu_edges, &expected);
}

#[test]
fn selected_block_keeps_separated_line_topology() {
    let document = document_with_insert(Point3::from_xy(0.0, 0.0), Point3::new(1.0, 1.0, 1.0), 0.0);
    assert_selected_insert_matches(&document, Transform2::identity());
}

#[test]
fn selected_block_translated_matches_display_list() {
    let insertion = Point3::from_xy(40.0, -15.0);
    let document = document_with_insert(insertion, Point3::new(1.0, 1.0, 1.0), 0.0);
    assert_selected_insert_matches(
        &document,
        insert_transform(insertion, Point3::new(1.0, 1.0, 1.0), 0.0),
    );
}

#[test]
fn selected_block_rotated_matches_display_list() {
    let insertion = Point3::from_xy(0.0, 0.0);
    let rotation = std::f64::consts::FRAC_PI_2;
    let document = document_with_insert(insertion, Point3::new(1.0, 1.0, 1.0), rotation);
    assert_selected_insert_matches(
        &document,
        insert_transform(insertion, Point3::new(1.0, 1.0, 1.0), rotation),
    );
}

#[test]
fn selected_block_scaled_matches_display_list() {
    let insertion = Point3::from_xy(5.0, 8.0);
    let scale = Point3::new(2.0, 3.0, 1.0);
    let document = document_with_insert(insertion, scale, 0.0);
    assert_selected_insert_matches(&document, insert_transform(insertion, scale, 0.0));
}

#[test]
fn selected_nested_insert_matches_display_list() {
    let mut document = Document::default();
    layer0(&mut document);
    document.blocks.insert("SEP".into(), separated_block("SEP"));
    document.blocks.insert(
        "OUTER".into(),
        BlockDefinition {
            name: "OUTER".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![insert_entity(
                "SEP",
                Point3::from_xy(5.0, 5.0),
                Point3::new(1.0, 1.0, 1.0),
                0.0,
            )],
        },
    );
    let outer_ins = Point3::from_xy(20.0, -10.0);
    let outer_scale = Point3::new(2.0, 2.0, 1.0);
    let outer_rot = std::f64::consts::FRAC_PI_4;
    document
        .model_space
        .push(insert_entity("OUTER", outer_ins, outer_scale, outer_rot));
    let nested = insert_transform(outer_ins, outer_scale, outer_rot).then(insert_transform(
        Point3::from_xy(5.0, 5.0),
        Point3::new(1.0, 1.0, 1.0),
        0.0,
    ));
    assert_selected_insert_matches(&document, nested);
}

fn box_doc_line(x0: f64, y0: f64, x1: f64, y1: f64) -> Document {
    let mut document = Document::default();
    layer0(&mut document);
    document.model_space.push(Entity::new(Geometry::Line {
        start: Point3::from_xy(x0, y0),
        end: Point3::from_xy(x1, y1),
    }));
    document
}

#[test]
fn window_selects_only_fully_contained_entities() {
    let mut document = Document::default();
    layer0(&mut document);
    document.model_space.push(Entity::new(Geometry::Line {
        start: Point3::from_xy(1.0, 1.0),
        end: Point3::from_xy(2.0, 1.0),
    }));
    document.model_space.push(Entity::new(Geometry::Line {
        start: Point3::from_xy(5.0, 1.0),
        end: Point3::from_xy(20.0, 1.0),
    }));
    let list = tessellate_document(&document);
    let region = Extents2::from_corners(Point2::new(0.0, 0.0), Point2::new(10.0, 10.0));
    assert_eq!(
        box_select(&list.picks, region, SelectBoxMode::Window),
        vec![0]
    );
    assert_eq!(
        box_select(&list.picks, region, SelectBoxMode::Crossing),
        vec![0, 1]
    );
}

#[test]
fn crossing_hits_a_segment_through_the_box() {
    let document = box_doc_line(-10.0, 5.0, 30.0, 5.0);
    let list = tessellate_document(&document);
    let region = Extents2::from_corners(Point2::new(0.0, 0.0), Point2::new(10.0, 10.0));
    assert!(box_select(&list.picks, region, SelectBoxMode::Window).is_empty());
    assert_eq!(
        box_select(&list.picks, region, SelectBoxMode::Crossing),
        vec![0]
    );
}

#[test]
fn crossing_selects_a_fill_that_contains_the_box() {
    let mut document = Document::default();
    layer0(&mut document);
    document.model_space.push(Entity::new(Geometry::Solid {
        corners: [
            Point3::from_xy(0.0, 0.0),
            Point3::from_xy(20.0, 0.0),
            Point3::from_xy(20.0, 20.0),
            Point3::from_xy(0.0, 20.0),
        ],
        extrusion: Point3::new(0.0, 0.0, 1.0),
    }));
    let list = tessellate_document(&document);
    let inside = Extents2::from_corners(Point2::new(8.0, 8.0), Point2::new(12.0, 12.0));
    assert!(box_select(&list.picks, inside, SelectBoxMode::Window).is_empty());
    assert_eq!(
        box_select(&list.picks, inside, SelectBoxMode::Crossing),
        vec![0]
    );
}

#[test]
fn crossing_ignores_empty_gaps_inside_a_block() {
    let mut document = Document::default();
    layer0(&mut document);
    document.blocks.insert(
        "GAP".into(),
        BlockDefinition {
            name: "GAP".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![
                Entity::new(Geometry::Line {
                    start: Point3::from_xy(0.0, 0.0),
                    end: Point3::from_xy(10.0, 0.0),
                }),
                Entity::new(Geometry::Line {
                    start: Point3::from_xy(100.0, 0.0),
                    end: Point3::from_xy(110.0, 0.0),
                }),
            ],
        },
    );
    document.model_space.push(insert_entity(
        "GAP",
        Point3::from_xy(0.0, 0.0),
        Point3::new(1.0, 1.0, 1.0),
        0.0,
    ));
    let list = tessellate_document(&document);
    let gap = Extents2::from_corners(Point2::new(40.0, -5.0), Point2::new(50.0, 5.0));
    assert!(box_select(&list.picks, gap, SelectBoxMode::Crossing).is_empty());
    let around_child = Extents2::from_corners(Point2::new(-1.0, -1.0), Point2::new(11.0, 1.0));
    assert_eq!(
        box_select(&list.picks, around_child, SelectBoxMode::Crossing),
        vec![0]
    );
    assert!(box_select(&list.picks, around_child, SelectBoxMode::Window).is_empty());
}
