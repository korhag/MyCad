//! Compare two native documents with CAD tolerances.

use crate::color::{aci_rgb, CadColor};
use crate::document::{BlockDefinition, Document, DrawingUnits, Layer};
use crate::entity::{
    Entity, Geometry, HatchData, HatchEdge, HatchPath, MTextData, PolyVertex, TextData,
};
use crate::extents::Extents2;
use crate::geom::{Point2, Point3};
use crate::linetype::{normalize_linetype_name, LineType};

const TAU: f64 = std::f64::consts::TAU;
const PI: f64 = std::f64::consts::PI;

// ------------------------------------------------------------
// Type: CompareTol
// Purpose: Absolute/relative CAD epsilon for round-trip geometry.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompareTol {
    pub coord: f64,
    pub angle: f64,
    pub bulge: f64,
}

impl CompareTol {
    /// 1e-8 in local units, growing with coordinate magnitude.
    pub const ROUND_TRIP: Self = Self {
        coord: 1e-8,
        angle: 1e-8,
        bulge: 1e-8,
    };
}

impl Default for CompareTol {
    fn default() -> Self {
        Self::ROUND_TRIP
    }
}

// ------------------------------------------------------------
// Type: Mismatch
// Purpose: One failed field in a document comparison.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    pub path: String,
    pub message: String,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

// ------------------------------------------------------------
// Function: compare_documents
// Purpose: Diff units, tables, extents, blocks, and model-space geometry.
// ------------------------------------------------------------
pub fn compare_documents(expected: &Document, actual: &Document, tol: CompareTol) -> Vec<Mismatch> {
    let mut mismatches = Vec::new();
    compare_units(expected.units, actual.units, &mut mismatches);
    if !close(expected.ltscale, actual.ltscale, tol.coord) {
        mismatches.push(Mismatch {
            path: "ltscale".into(),
            message: format!("expected {} got {}", expected.ltscale, actual.ltscale),
        });
    }
    compare_layers(&expected.layers, &actual.layers, &mut mismatches);
    compare_linetypes(&expected.linetypes, &actual.linetypes, tol, &mut mismatches);
    compare_extents(
        expected.compute_extents(),
        actual.compute_extents(),
        tol,
        &mut mismatches,
    );
    compare_blocks(&expected.blocks, &actual.blocks, tol, &mut mismatches);
    compare_entities(
        "model_space",
        &expected.model_space,
        &actual.model_space,
        tol,
        &mut mismatches,
    );
    mismatches
}

fn compare_units(expected: DrawingUnits, actual: DrawingUnits, out: &mut Vec<Mismatch>) {
    if expected.to_insunits() != actual.to_insunits() {
        out.push(Mismatch {
            path: "units".into(),
            message: format!(
                "expected INSUNITS {} got {}",
                expected.to_insunits(),
                actual.to_insunits()
            ),
        });
    }
}

fn compare_layers(
    expected: &std::collections::BTreeMap<String, Layer>,
    actual: &std::collections::BTreeMap<String, Layer>,
    out: &mut Vec<Mismatch>,
) {
    for (name, layer) in expected {
        let Some(got) = find_layer(actual, name) else {
            out.push(Mismatch {
                path: format!("layers.{name}"),
                message: "missing layer".into(),
            });
            continue;
        };
        if layer.visible != got.visible {
            out.push(Mismatch {
                path: format!("layers.{name}.visible"),
                message: format!("expected {} got {}", layer.visible, got.visible),
            });
        }
        if layer.frozen != got.frozen {
            out.push(Mismatch {
                path: format!("layers.{name}.frozen"),
                message: format!("expected {} got {}", layer.frozen, got.frozen),
            });
        }
        if !colors_match(layer.color, got.color) {
            out.push(Mismatch {
                path: format!("layers.{name}.color"),
                message: format!("expected {:?} got {:?}", layer.color, got.color),
            });
        }
        if normalize_linetype_name(&layer.linetype) != normalize_linetype_name(&got.linetype) {
            out.push(Mismatch {
                path: format!("layers.{name}.linetype"),
                message: format!("expected {} got {}", layer.linetype, got.linetype),
            });
        }
    }
}

fn find_layer<'a>(
    layers: &'a std::collections::BTreeMap<String, Layer>,
    name: &str,
) -> Option<&'a Layer> {
    layers.get(name).or_else(|| {
        layers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, layer)| layer)
    })
}

fn compare_linetypes(
    expected: &std::collections::BTreeMap<String, LineType>,
    actual: &std::collections::BTreeMap<String, LineType>,
    tol: CompareTol,
    out: &mut Vec<Mismatch>,
) {
    for (name, linetype) in expected {
        let Some(got) = find_linetype(actual, name) else {
            out.push(Mismatch {
                path: format!("linetypes.{name}"),
                message: "missing linetype".into(),
            });
            continue;
        };
        if linetype.dashes.len() != got.dashes.len() {
            out.push(Mismatch {
                path: format!("linetypes.{name}.dashes"),
                message: format!(
                    "expected {} dashes got {}",
                    linetype.dashes.len(),
                    got.dashes.len()
                ),
            });
            continue;
        }
        for (index, (a, b)) in linetype.dashes.iter().zip(got.dashes.iter()).enumerate() {
            if !close(*a, *b, tol.coord) {
                out.push(Mismatch {
                    path: format!("linetypes.{name}.dashes[{index}]"),
                    message: format!("expected {a} got {b}"),
                });
            }
        }
    }
}

fn find_linetype<'a>(
    linetypes: &'a std::collections::BTreeMap<String, LineType>,
    name: &str,
) -> Option<&'a LineType> {
    let key = normalize_linetype_name(name);
    linetypes.get(&key).or_else(|| {
        linetypes
            .iter()
            .find(|(existing, _)| existing.eq_ignore_ascii_case(&key))
            .map(|(_, linetype)| linetype)
    })
}

fn compare_extents(
    expected: Option<Extents2>,
    actual: Option<Extents2>,
    tol: CompareTol,
    out: &mut Vec<Mismatch>,
) {
    match (expected, actual) {
        (None, None) => {}
        (Some(a), Some(b)) => {
            point2_field("extents.min", a.min, b.min, tol, out);
            point2_field("extents.max", a.max, b.max, tol, out);
        }
        (Some(_), None) => out.push(Mismatch {
            path: "extents".into(),
            message: "expected extents, actual document has none".into(),
        }),
        (None, Some(_)) => out.push(Mismatch {
            path: "extents".into(),
            message: "actual document has extents, expected none".into(),
        }),
    }
}

fn compare_blocks(
    expected: &std::collections::BTreeMap<String, BlockDefinition>,
    actual: &std::collections::BTreeMap<String, BlockDefinition>,
    tol: CompareTol,
    out: &mut Vec<Mismatch>,
) {
    for (name, block) in expected {
        let Some(got) = find_block(actual, name) else {
            out.push(Mismatch {
                path: format!("blocks.{name}"),
                message: "missing block".into(),
            });
            continue;
        };
        point3_field(
            &format!("blocks.{name}.base_pt"),
            block.base_pt,
            got.base_pt,
            tol,
            out,
        );
        compare_entities(
            &format!("blocks.{name}.entities"),
            &block.entities,
            &got.entities,
            tol,
            out,
        );
    }
}

fn find_block<'a>(
    blocks: &'a std::collections::BTreeMap<String, BlockDefinition>,
    name: &str,
) -> Option<&'a BlockDefinition> {
    blocks.get(name).or_else(|| {
        blocks
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, block)| block)
    })
}

fn compare_entities(
    path: &str,
    expected: &[Entity],
    actual: &[Entity],
    tol: CompareTol,
    out: &mut Vec<Mismatch>,
) {
    if expected.len() != actual.len() {
        out.push(Mismatch {
            path: path.into(),
            message: format!(
                "entity count expected {} ({}) got {} ({})",
                expected.len(),
                type_list(expected),
                actual.len(),
                type_list(actual)
            ),
        });
        return;
    }
    for (index, (exp, got)) in expected.iter().zip(actual.iter()).enumerate() {
        let item = format!("{path}[{index}]");
        if !exp.layer.eq_ignore_ascii_case(&got.layer) {
            out.push(Mismatch {
                path: format!("{item}.layer"),
                message: format!("expected {} got {}", exp.layer, got.layer),
            });
        }
        if !colors_match(exp.color, got.color) {
            out.push(Mismatch {
                path: format!("{item}.color"),
                message: format!("expected {:?} got {:?}", exp.color, got.color),
            });
        }
        if normalize_linetype_name(&exp.linetype) != normalize_linetype_name(&got.linetype) {
            out.push(Mismatch {
                path: format!("{item}.linetype"),
                message: format!("expected {} got {}", exp.linetype, got.linetype),
            });
        }
        if !close(exp.linetype_scale, got.linetype_scale, tol.coord) {
            out.push(Mismatch {
                path: format!("{item}.linetype_scale"),
                message: format!("expected {} got {}", exp.linetype_scale, got.linetype_scale),
            });
        }
        if exp.visible != got.visible {
            out.push(Mismatch {
                path: format!("{item}.visible"),
                message: format!("expected {} got {}", exp.visible, got.visible),
            });
        }
        compare_geometry(
            &format!("{item}.geometry"),
            &exp.geometry,
            &got.geometry,
            tol,
            out,
        );
    }
}

fn type_list(entities: &[Entity]) -> String {
    entities
        .iter()
        .map(|entity| entity.geometry.type_name())
        .collect::<Vec<_>>()
        .join(", ")
}

fn compare_geometry(
    path: &str,
    expected: &Geometry,
    actual: &Geometry,
    tol: CompareTol,
    out: &mut Vec<Mismatch>,
) {
    if let (Some(a), Some(b)) = (polyline_of(expected), polyline_of(actual)) {
        if a.1 != b.1 {
            out.push(Mismatch {
                path: format!("{path}.closed"),
                message: format!("expected {} got {}", a.1, b.1),
            });
        }
        vertices_field(&format!("{path}.vertices"), a.0, b.0, tol, out);
        return;
    }
    match (expected, actual) {
        (Geometry::Line { start: s0, end: e0 }, Geometry::Line { start: s1, end: e1 }) => {
            point3_field(&format!("{path}.start"), *s0, *s1, tol, out);
            point3_field(&format!("{path}.end"), *e0, *e1, tol, out);
        }
        (Geometry::Point { position: a }, Geometry::Point { position: b }) => {
            point3_field(path, *a, *b, tol, out);
        }
        (
            Geometry::Circle {
                center: c0,
                radius: r0,
                extrusion: e0,
            },
            Geometry::Circle {
                center: c1,
                radius: r1,
                extrusion: e1,
            },
        ) => {
            point3_field(&format!("{path}.center"), *c0, *c1, tol, out);
            scalar_field(&format!("{path}.radius"), *r0, *r1, tol.coord, out);
            point3_field(&format!("{path}.extrusion"), *e0, *e1, tol, out);
        }
        (
            Geometry::Arc {
                center: c0,
                radius: r0,
                start_angle: s0,
                end_angle: e0,
                extrusion: x0,
            },
            Geometry::Arc {
                center: c1,
                radius: r1,
                start_angle: s1,
                end_angle: e1,
                extrusion: x1,
            },
        ) => {
            point3_field(&format!("{path}.center"), *c0, *c1, tol, out);
            scalar_field(&format!("{path}.radius"), *r0, *r1, tol.coord, out);
            angle_field(&format!("{path}.start_angle"), *s0, *s1, tol.angle, out);
            angle_field(&format!("{path}.end_angle"), *e0, *e1, tol.angle, out);
            point3_field(&format!("{path}.extrusion"), *x0, *x1, tol, out);
        }
        (
            Geometry::Ellipse {
                center: c0,
                major_axis: m0,
                axis_ratio: r0,
                start_param: s0,
                end_param: e0,
                extrusion: x0,
            },
            Geometry::Ellipse {
                center: c1,
                major_axis: m1,
                axis_ratio: r1,
                start_param: s1,
                end_param: e1,
                extrusion: x1,
            },
        ) => {
            point3_field(&format!("{path}.center"), *c0, *c1, tol, out);
            point3_field(&format!("{path}.major_axis"), *m0, *m1, tol, out);
            scalar_field(&format!("{path}.axis_ratio"), *r0, *r1, tol.coord, out);
            angle_field(&format!("{path}.start_param"), *s0, *s1, tol.angle, out);
            angle_field(&format!("{path}.end_param"), *e0, *e1, tol.angle, out);
            point3_field(&format!("{path}.extrusion"), *x0, *x1, tol, out);
        }
        (
            Geometry::Spline {
                degree: d0,
                control_points: c0,
                knots: k0,
                closed: cl0,
                ..
            },
            Geometry::Spline {
                degree: d1,
                control_points: c1,
                knots: k1,
                closed: cl1,
                ..
            },
        ) => {
            if d0 != d1 {
                out.push(Mismatch {
                    path: format!("{path}.degree"),
                    message: format!("expected {d0} got {d1}"),
                });
            }
            if cl0 != cl1 {
                out.push(Mismatch {
                    path: format!("{path}.closed"),
                    message: format!("expected {cl0} got {cl1}"),
                });
            }
            points_field(&format!("{path}.control_points"), c0, c1, tol, out);
            if !k0.is_empty() && !k1.is_empty() {
                if k0.len() != k1.len() {
                    out.push(Mismatch {
                        path: format!("{path}.knots"),
                        message: format!("expected {} knots got {}", k0.len(), k1.len()),
                    });
                } else {
                    for (index, (a, b)) in k0.iter().zip(k1.iter()).enumerate() {
                        scalar_field(&format!("{path}.knots[{index}]"), *a, *b, tol.coord, out);
                    }
                }
            }
        }
        (
            Geometry::Insert {
                block_name: n0,
                insertion: i0,
                scale: s0,
                rotation: r0,
                extrusion: e0,
                column_count: cc0,
                row_count: rc0,
                column_spacing: cs0,
                row_spacing: rs0,
                ..
            },
            Geometry::Insert {
                block_name: n1,
                insertion: i1,
                scale: s1,
                rotation: r1,
                extrusion: e1,
                column_count: cc1,
                row_count: rc1,
                column_spacing: cs1,
                row_spacing: rs1,
                ..
            },
        ) => {
            if !n0.eq_ignore_ascii_case(n1) {
                out.push(Mismatch {
                    path: format!("{path}.block_name"),
                    message: format!("expected {n0} got {n1}"),
                });
            }
            point3_field(&format!("{path}.insertion"), *i0, *i1, tol, out);
            point3_field(&format!("{path}.scale"), *s0, *s1, tol, out);
            angle_field(&format!("{path}.rotation"), *r0, *r1, tol.angle, out);
            point3_field(&format!("{path}.extrusion"), *e0, *e1, tol, out);
            if cc0 != cc1 || rc0 != rc1 {
                out.push(Mismatch {
                    path: format!("{path}.array"),
                    message: format!("expected {cc0}x{rc0} got {cc1}x{rc1}"),
                });
            }
            scalar_field(
                &format!("{path}.column_spacing"),
                *cs0,
                *cs1,
                tol.coord,
                out,
            );
            scalar_field(&format!("{path}.row_spacing"), *rs0, *rs1, tol.coord, out);
        }
        (Geometry::Text(a), Geometry::Text(b)) => compare_text(path, a, b, tol, out),
        (Geometry::MText(a), Geometry::MText(b)) => compare_mtext(path, a, b, tol, out),
        (Geometry::Hatch(a), Geometry::Hatch(b)) => compare_hatch(path, a, b, tol, out),
        (
            Geometry::Solid {
                corners: c0,
                extrusion: e0,
            },
            Geometry::Solid {
                corners: c1,
                extrusion: e1,
            },
        ) => {
            for (index, (a, b)) in c0.iter().zip(c1.iter()).enumerate() {
                point3_field(&format!("{path}.corners[{index}]"), *a, *b, tol, out);
            }
            point3_field(&format!("{path}.extrusion"), *e0, *e1, tol, out);
        }
        (Geometry::Leader { vertices: a }, Geometry::Leader { vertices: b }) => {
            points_field(&format!("{path}.vertices"), a, b, tol, out);
        }
        _ => out.push(Mismatch {
            path: path.into(),
            message: format!(
                "type expected {} got {}",
                expected.type_name(),
                actual.type_name()
            ),
        }),
    }
}

fn polyline_of(geometry: &Geometry) -> Option<(&[PolyVertex], bool)> {
    match geometry {
        Geometry::LwPolyline {
            vertices, closed, ..
        }
        | Geometry::Polyline {
            vertices, closed, ..
        } => Some((vertices, *closed)),
        _ => None,
    }
}

fn compare_text(path: &str, a: &TextData, b: &TextData, tol: CompareTol, out: &mut Vec<Mismatch>) {
    point3_field(
        &format!("{path}.insertion"),
        a.insertion,
        b.insertion,
        tol,
        out,
    );
    scalar_field(
        &format!("{path}.height"),
        a.height,
        b.height,
        tol.coord,
        out,
    );
    angle_field(
        &format!("{path}.rotation"),
        a.rotation,
        b.rotation,
        tol.angle,
        out,
    );
    if a.value != b.value {
        out.push(Mismatch {
            path: format!("{path}.value"),
            message: format!("expected {:?} got {:?}", a.value, b.value),
        });
    }
}

fn compare_mtext(
    path: &str,
    a: &MTextData,
    b: &MTextData,
    tol: CompareTol,
    out: &mut Vec<Mismatch>,
) {
    point3_field(
        &format!("{path}.insertion"),
        a.insertion,
        b.insertion,
        tol,
        out,
    );
    scalar_field(
        &format!("{path}.height"),
        a.height,
        b.height,
        tol.coord,
        out,
    );
    scalar_field(&format!("{path}.width"), a.width, b.width, tol.coord, out);
    angle_field(
        &format!("{path}.rotation"),
        a.rotation,
        b.rotation,
        tol.angle,
        out,
    );
    if a.value != b.value {
        out.push(Mismatch {
            path: format!("{path}.value"),
            message: format!("expected {:?} got {:?}", a.value, b.value),
        });
    }
}

fn compare_hatch(
    path: &str,
    a: &HatchData,
    b: &HatchData,
    tol: CompareTol,
    out: &mut Vec<Mismatch>,
) {
    if a.solid_fill != b.solid_fill {
        out.push(Mismatch {
            path: format!("{path}.solid_fill"),
            message: format!("expected {} got {}", a.solid_fill, b.solid_fill),
        });
    }
    if a.paths.len() != b.paths.len() {
        out.push(Mismatch {
            path: format!("{path}.paths"),
            message: format!("expected {} paths got {}", a.paths.len(), b.paths.len()),
        });
        return;
    }
    for (index, (left, right)) in a.paths.iter().zip(b.paths.iter()).enumerate() {
        compare_hatch_path(&format!("{path}.paths[{index}]"), left, right, tol, out);
    }
}

fn compare_hatch_path(
    path: &str,
    expected: &HatchPath,
    actual: &HatchPath,
    tol: CompareTol,
    out: &mut Vec<Mismatch>,
) {
    match (expected, actual) {
        (
            HatchPath::Polyline {
                vertices: v0,
                closed: c0,
            },
            HatchPath::Polyline {
                vertices: v1,
                closed: c1,
            },
        ) => {
            if c0 != c1 {
                out.push(Mismatch {
                    path: format!("{path}.closed"),
                    message: format!("expected {c0} got {c1}"),
                });
            }
            vertices_field(&format!("{path}.vertices"), v0, v1, tol, out);
        }
        (HatchPath::Edges(a), HatchPath::Edges(b)) => {
            compare_edge_list(path, a, b, tol, out);
        }
        (HatchPath::Polyline { vertices, closed }, HatchPath::Edges(edges))
        | (HatchPath::Edges(edges), HatchPath::Polyline { vertices, closed }) => {
            match polyline_as_line_edges(vertices, *closed) {
                Some(from_poly) => compare_edge_list(path, &from_poly, edges, tol, out),
                None => out.push(Mismatch {
                    path: path.into(),
                    message: "hatch polyline with bulge is not comparable to edge paths".into(),
                }),
            }
        }
    }
}

fn compare_edge_list(
    path: &str,
    expected: &[HatchEdge],
    actual: &[HatchEdge],
    tol: CompareTol,
    out: &mut Vec<Mismatch>,
) {
    if expected.len() != actual.len() {
        out.push(Mismatch {
            path: path.into(),
            message: format!("expected {} edges got {}", expected.len(), actual.len()),
        });
        return;
    }
    for (index, (left, right)) in expected.iter().zip(actual.iter()).enumerate() {
        compare_hatch_edge(&format!("{path}[{index}]"), left, right, tol, out);
    }
}

fn polyline_as_line_edges(vertices: &[PolyVertex], closed: bool) -> Option<Vec<HatchEdge>> {
    if vertices.len() < 2 {
        return None;
    }
    if vertices.iter().any(|vertex| vertex.bulge.abs() > 1e-12) {
        return None;
    }
    let mut edges = Vec::new();
    for pair in vertices.windows(2) {
        edges.push(HatchEdge::Line {
            start: pair[0].point,
            end: pair[1].point,
        });
    }
    if closed {
        let first = vertices.first()?;
        let last = vertices.last()?;
        edges.push(HatchEdge::Line {
            start: last.point,
            end: first.point,
        });
    }
    Some(edges)
}

fn compare_hatch_edge(
    path: &str,
    expected: &HatchEdge,
    actual: &HatchEdge,
    tol: CompareTol,
    out: &mut Vec<Mismatch>,
) {
    match (expected, actual) {
        (HatchEdge::Line { start: s0, end: e0 }, HatchEdge::Line { start: s1, end: e1 }) => {
            point3_field(&format!("{path}.start"), *s0, *s1, tol, out);
            point3_field(&format!("{path}.end"), *e0, *e1, tol, out);
        }
        (
            HatchEdge::Arc {
                center: c0,
                radius: r0,
                start_angle: s0,
                end_angle: e0,
                is_ccw: w0,
            },
            HatchEdge::Arc {
                center: c1,
                radius: r1,
                start_angle: s1,
                end_angle: e1,
                is_ccw: w1,
            },
        ) => {
            point3_field(&format!("{path}.center"), *c0, *c1, tol, out);
            scalar_field(&format!("{path}.radius"), *r0, *r1, tol.coord, out);
            angle_field(&format!("{path}.start_angle"), *s0, *s1, tol.angle, out);
            angle_field(&format!("{path}.end_angle"), *e0, *e1, tol.angle, out);
            if w0 != w1 {
                out.push(Mismatch {
                    path: format!("{path}.is_ccw"),
                    message: format!("expected {w0} got {w1}"),
                });
            }
        }
        _ => out.push(Mismatch {
            path: path.into(),
            message: "hatch edge kinds differ".into(),
        }),
    }
}

fn vertices_field(
    path: &str,
    expected: &[PolyVertex],
    actual: &[PolyVertex],
    tol: CompareTol,
    out: &mut Vec<Mismatch>,
) {
    if expected.len() != actual.len() {
        out.push(Mismatch {
            path: path.into(),
            message: format!("expected {} vertices got {}", expected.len(), actual.len()),
        });
        return;
    }
    for (index, (a, b)) in expected.iter().zip(actual.iter()).enumerate() {
        point3_field(
            &format!("{path}[{index}].point"),
            a.point,
            b.point,
            tol,
            out,
        );
        if !close(a.bulge, b.bulge, tol.bulge) {
            out.push(Mismatch {
                path: format!("{path}[{index}].bulge"),
                message: format!("expected {} got {}", a.bulge, b.bulge),
            });
        }
    }
}

fn points_field(
    path: &str,
    expected: &[Point3],
    actual: &[Point3],
    tol: CompareTol,
    out: &mut Vec<Mismatch>,
) {
    if expected.len() != actual.len() {
        out.push(Mismatch {
            path: path.into(),
            message: format!("expected {} points got {}", expected.len(), actual.len()),
        });
        return;
    }
    for (index, (a, b)) in expected.iter().zip(actual.iter()).enumerate() {
        point3_field(&format!("{path}[{index}]"), *a, *b, tol, out);
    }
}

fn point3_field(
    path: &str,
    expected: Point3,
    actual: Point3,
    tol: CompareTol,
    out: &mut Vec<Mismatch>,
) {
    if !close(expected.x, actual.x, coord_eps(tol, expected.x, actual.x))
        || !close(expected.y, actual.y, coord_eps(tol, expected.y, actual.y))
        || !close(expected.z, actual.z, coord_eps(tol, expected.z, actual.z))
    {
        out.push(Mismatch {
            path: path.into(),
            message: format!(
                "expected ({}, {}, {}) got ({}, {}, {})",
                expected.x, expected.y, expected.z, actual.x, actual.y, actual.z
            ),
        });
    }
}

fn point2_field(
    path: &str,
    expected: Point2,
    actual: Point2,
    tol: CompareTol,
    out: &mut Vec<Mismatch>,
) {
    if !close(expected.x, actual.x, coord_eps(tol, expected.x, actual.x))
        || !close(expected.y, actual.y, coord_eps(tol, expected.y, actual.y))
    {
        out.push(Mismatch {
            path: path.into(),
            message: format!(
                "expected ({}, {}) got ({}, {})",
                expected.x, expected.y, actual.x, actual.y
            ),
        });
    }
}

fn scalar_field(path: &str, expected: f64, actual: f64, tol: f64, out: &mut Vec<Mismatch>) {
    if !close(
        expected,
        actual,
        tol * (1.0 + expected.abs().max(actual.abs())),
    ) {
        out.push(Mismatch {
            path: path.into(),
            message: format!("expected {expected} got {actual}"),
        });
    }
}

fn angle_field(path: &str, expected: f64, actual: f64, tol: f64, out: &mut Vec<Mismatch>) {
    if !angles_close(expected, actual, tol) {
        out.push(Mismatch {
            path: path.into(),
            message: format!("expected {expected} rad got {actual} rad"),
        });
    }
}

fn colors_match(expected: CadColor, actual: CadColor) -> bool {
    if expected == actual {
        return true;
    }
    match (expected, actual) {
        (CadColor::Aci(index), CadColor::Rgb { r, g, b })
        | (CadColor::Rgb { r, g, b }, CadColor::Aci(index)) => {
            let palette = aci_rgb(index);
            palette.r == r && palette.g == g && palette.b == b
        }
        _ => false,
    }
}

fn coord_eps(tol: CompareTol, a: f64, b: f64) -> f64 {
    tol.coord * (1.0 + a.abs().max(b.abs()))
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    if a == b {
        return true;
    }
    (a - b).abs() <= tol
}

fn angles_close(a: f64, b: f64, tol: f64) -> bool {
    let mut delta = (a - b) % TAU;
    if delta > PI {
        delta -= TAU;
    }
    if delta < -PI {
        delta += TAU;
    }
    delta.abs() <= tol
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::entity::{Entity, Geometry};

    #[test]
    fn identical_documents_have_no_mismatches() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(1.0, 2.0),
        }));
        assert!(compare_documents(&document, &document, CompareTol::ROUND_TRIP).is_empty());
    }

    #[test]
    fn moved_line_is_a_coordinate_mismatch() {
        let mut expected = Document::default();
        expected.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(1.0, 0.0),
        }));
        let mut actual = expected.clone();
        if let Geometry::Line { end, .. } = &mut actual.model_space[0].geometry {
            end.x = 2.0;
        }
        let mismatches = compare_documents(&expected, &actual, CompareTol::ROUND_TRIP);
        assert!(mismatches.iter().any(|item| item.path.contains("end")));
    }
}
