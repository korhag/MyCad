//! Semantic 2D transforms for native drawing entities.

use std::fmt;

use crate::entity::{
    Entity, Geometry, HatchData, HatchEdge, HatchPath, HatchPatternLine, MTextData, PolyVertex,
    TextData,
};
use crate::geom::{is_world_extrusion, Point2, Point3, GEOM_TOLERANCE};
use crate::transform::Transform2;

const ANGLE_NOOP: f64 = 1e-12;

// ------------------------------------------------------------
// Enum: EntityTransform
// Purpose: Exact planar operations shared by Move, Copy, Rotate,
//          Mirror, and Scale. Internally a Transform2 matrix.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EntityTransform {
    Translate {
        dx: f64,
        dy: f64,
    },
    Rotate {
        base: Point2,
        radians: f64,
    },
    UniformScale {
        base: Point2,
        factor: f64,
    },
    Mirror {
        axis_start: Point2,
        axis_end: Point2,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransformError {
    Invalid(&'static str),
    NoOp,
    Unsupported(Vec<&'static str>),
}

impl fmt::Display for TransformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::NoOp => f.write_str("Nothing to transform"),
            Self::Unsupported(types) => {
                write!(f, "Cannot transform {}", types.join(", "))
            }
        }
    }
}

impl EntityTransform {
    pub fn to_matrix(self) -> Result<Transform2, TransformError> {
        match self {
            Self::Translate { dx, dy } => {
                if !dx.is_finite() || !dy.is_finite() {
                    return Err(TransformError::Invalid("Displacement is not finite"));
                }
                if dx.abs() <= GEOM_TOLERANCE && dy.abs() <= GEOM_TOLERANCE {
                    return Err(TransformError::NoOp);
                }
                Ok(Transform2::translate(dx, dy))
            }
            Self::Rotate { base, radians } => {
                if !base.is_finite() || !radians.is_finite() {
                    return Err(TransformError::Invalid("Rotation is not finite"));
                }
                if is_noop_angle(radians) {
                    return Err(TransformError::NoOp);
                }
                Ok(Transform2::translate(base.x, base.y)
                    .then(Transform2::rotate(radians))
                    .then(Transform2::translate(-base.x, -base.y)))
            }
            Self::UniformScale { base, factor } => {
                if !base.is_finite() || !factor.is_finite() {
                    return Err(TransformError::Invalid("Scale is not finite"));
                }
                if factor <= 0.0 {
                    return Err(TransformError::Invalid(
                        "Scale factor must be greater than zero",
                    ));
                }
                if (factor - 1.0).abs() <= GEOM_TOLERANCE {
                    return Err(TransformError::NoOp);
                }
                Ok(Transform2::translate(base.x, base.y)
                    .then(Transform2::scale(factor, factor))
                    .then(Transform2::translate(-base.x, -base.y)))
            }
            Self::Mirror {
                axis_start,
                axis_end,
            } => {
                if !axis_start.is_finite() || !axis_end.is_finite() {
                    return Err(TransformError::Invalid("Mirror axis is not finite"));
                }
                if axis_start.distance(axis_end) <= GEOM_TOLERANCE {
                    return Err(TransformError::Invalid("Mirror axis is too short"));
                }
                let angle = (axis_end.y - axis_start.y).atan2(axis_end.x - axis_start.x);
                Ok(Transform2::translate(axis_start.x, axis_start.y)
                    .then(Transform2::rotate(angle))
                    .then(Transform2::scale(1.0, -1.0))
                    .then(Transform2::rotate(-angle))
                    .then(Transform2::translate(-axis_start.x, -axis_start.y)))
            }
        }
    }
}

pub fn validate_entities<'a>(
    entities: impl IntoIterator<Item = &'a Entity>,
) -> Result<(), TransformError> {
    let mut unsupported = Vec::new();
    for entity in entities {
        if let Err(TransformError::Unsupported(types)) = geometry_supported(&entity.geometry) {
            for name in types {
                if !unsupported.contains(&name) {
                    unsupported.push(name);
                }
            }
        }
    }
    if unsupported.is_empty() {
        Ok(())
    } else {
        Err(TransformError::Unsupported(unsupported))
    }
}

pub fn transform_entity(entity: &Entity, xform: EntityTransform) -> Result<Entity, TransformError> {
    let matrix = xform.to_matrix()?;
    transform_entity_matrix(entity, matrix)
}

pub fn transform_entity_matrix(
    entity: &Entity,
    matrix: Transform2,
) -> Result<Entity, TransformError> {
    Ok(Entity {
        geometry: transform_geometry(&entity.geometry, matrix)?,
        ..entity.clone()
    })
}

pub fn transform_geometry(
    geometry: &Geometry,
    matrix: Transform2,
) -> Result<Geometry, TransformError> {
    geometry_supported(geometry)?;
    if needs_uniform_scale(geometry) && !matrix.is_uniform_scale() {
        return Err(TransformError::Unsupported(vec![geometry.type_name()]));
    }
    Ok(match geometry {
        Geometry::Line { start, end } => Geometry::Line {
            start: matrix.apply3(*start),
            end: matrix.apply3(*end),
        },
        Geometry::Point { position } => Geometry::Point {
            position: matrix.apply3(*position),
        },
        Geometry::Circle {
            center,
            radius,
            extrusion,
        } => Geometry::Circle {
            center: matrix.apply3(*center),
            radius: radius.abs() * matrix.scale_x(),
            extrusion: *extrusion,
        },
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            extrusion,
        } => transform_arc(
            *center,
            *radius,
            *start_angle,
            *end_angle,
            *extrusion,
            matrix,
        ),
        Geometry::Ellipse {
            center,
            major_axis,
            axis_ratio,
            start_param,
            end_param,
            extrusion,
        } => {
            let (start_param, end_param) = if matrix.reverses_orientation() {
                (*end_param, *start_param)
            } else {
                (*start_param, *end_param)
            };
            Geometry::Ellipse {
                center: matrix.apply3(*center),
                major_axis: apply_vector3(matrix, *major_axis),
                axis_ratio: *axis_ratio,
                start_param,
                end_param,
                extrusion: *extrusion,
            }
        }
        Geometry::LwPolyline {
            vertices,
            closed,
            extrusion,
            linetype_generation_continuous,
        } => Geometry::LwPolyline {
            vertices: transform_poly_vertices(vertices, matrix),
            closed: *closed,
            extrusion: *extrusion,
            linetype_generation_continuous: *linetype_generation_continuous,
        },
        Geometry::Polyline {
            vertices,
            closed,
            linetype_generation_continuous,
        } => Geometry::Polyline {
            vertices: transform_poly_vertices(vertices, matrix),
            closed: *closed,
            linetype_generation_continuous: *linetype_generation_continuous,
        },
        Geometry::Spline {
            degree,
            control_points,
            fit_points,
            knots,
            weights,
            closed,
        } => Geometry::Spline {
            degree: *degree,
            control_points: control_points.iter().map(|p| matrix.apply3(*p)).collect(),
            fit_points: fit_points.iter().map(|p| matrix.apply3(*p)).collect(),
            knots: knots.clone(),
            weights: weights.clone(),
            closed: *closed,
        },
        Geometry::Insert {
            block_name,
            insertion,
            scale,
            rotation,
            extrusion,
            attribs,
            column_count,
            row_count,
            column_spacing,
            row_spacing,
        } => {
            let combined = matrix.then(Transform2::insert(*insertion, *scale, *rotation));
            let (insertion_xy, new_scale, new_rotation) = decompose_insert(combined, scale.z);
            Geometry::Insert {
                block_name: block_name.clone(),
                insertion: Point3::new(insertion_xy.x, insertion_xy.y, insertion.z),
                scale: new_scale,
                rotation: new_rotation,
                extrusion: *extrusion,
                attribs: attribs
                    .iter()
                    .map(|text| transform_text(text, matrix))
                    .collect(),
                column_count: *column_count,
                row_count: *row_count,
                column_spacing: *column_spacing * matrix.scale_x(),
                row_spacing: *row_spacing * matrix.scale_x(),
            }
        }
        Geometry::Text(data) => Geometry::Text(transform_text(data, matrix)),
        Geometry::MText(data) => Geometry::MText(transform_mtext(data, matrix)),
        Geometry::Solid { corners, extrusion } => {
            let mut corners = corners.map(|corner| matrix.apply3(corner));
            if matrix.reverses_orientation() {
                corners.reverse();
            }
            Geometry::Solid {
                corners,
                extrusion: *extrusion,
            }
        }
        Geometry::Leader { vertices } => Geometry::Leader {
            vertices: vertices.iter().map(|p| matrix.apply3(*p)).collect(),
        },
        Geometry::MLine { vertices, closed } => Geometry::MLine {
            vertices: vertices.iter().map(|p| matrix.apply3(*p)).collect(),
            closed: *closed,
        },
        Geometry::Hatch(hatch) => Geometry::Hatch(transform_hatch(hatch, matrix)?),
        Geometry::Dimension { .. } => {
            return Err(TransformError::Unsupported(vec![geometry.type_name()]));
        }
    })
}

pub fn reference_radius<'a>(entities: impl IntoIterator<Item = &'a Entity>, base: Point2) -> f64 {
    let mut farthest = 0.0_f64;
    for entity in entities {
        for point in representative_points(&entity.geometry) {
            farthest = farthest.max(base.distance(point));
        }
    }
    farthest.max(1.0)
}

fn geometry_supported(geometry: &Geometry) -> Result<(), TransformError> {
    match geometry {
        Geometry::Dimension { .. } => Err(TransformError::Unsupported(vec![geometry.type_name()])),
        Geometry::Circle { extrusion, .. }
        | Geometry::Arc { extrusion, .. }
        | Geometry::Ellipse { extrusion, .. }
        | Geometry::LwPolyline { extrusion, .. }
        | Geometry::Insert { extrusion, .. }
        | Geometry::Solid { extrusion, .. }
        | Geometry::Hatch(crate::entity::HatchData { extrusion, .. }) => {
            require_world_ocs(*extrusion, geometry.type_name())
        }
        Geometry::Text(data) => require_world_ocs(data.extrusion, geometry.type_name()),
        Geometry::MText(data) => require_world_ocs(data.extrusion, geometry.type_name()),
        Geometry::Line { .. }
        | Geometry::Point { .. }
        | Geometry::Polyline { .. }
        | Geometry::Spline { .. }
        | Geometry::Leader { .. }
        | Geometry::MLine { .. } => Ok(()),
    }
}

fn needs_uniform_scale(geometry: &Geometry) -> bool {
    matches!(
        geometry,
        Geometry::Circle { .. } | Geometry::Arc { .. } | Geometry::Hatch(_)
    )
}

fn require_world_ocs(extrusion: Point3, type_name: &'static str) -> Result<(), TransformError> {
    if is_world_extrusion(extrusion) {
        Ok(())
    } else {
        Err(TransformError::Unsupported(vec![type_name]))
    }
}

fn transform_poly_vertices(vertices: &[PolyVertex], matrix: Transform2) -> Vec<PolyVertex> {
    let reverse_bulge = matrix.reverses_orientation();
    vertices
        .iter()
        .map(|vertex| PolyVertex {
            point: matrix.apply3(vertex.point),
            bulge: if reverse_bulge {
                -vertex.bulge
            } else {
                vertex.bulge
            },
        })
        .collect()
}

fn transform_arc(
    center: Point3,
    radius: f64,
    start_angle: f64,
    end_angle: f64,
    extrusion: Point3,
    matrix: Transform2,
) -> Geometry {
    let start = Point2::new(
        center.x + radius * start_angle.cos(),
        center.y + radius * start_angle.sin(),
    );
    let end = Point2::new(
        center.x + radius * end_angle.cos(),
        center.y + radius * end_angle.sin(),
    );
    let new_center = matrix.apply3(center);
    let new_start = matrix.apply(start);
    let new_end = matrix.apply(end);
    let start_from = (new_start.y - new_center.y).atan2(new_start.x - new_center.x);
    let end_from = (new_end.y - new_center.y).atan2(new_end.x - new_center.x);
    let (start_angle, end_angle) = if matrix.reverses_orientation() {
        (end_from, start_from)
    } else {
        (start_from, end_from)
    };
    Geometry::Arc {
        center: new_center,
        radius: radius.abs() * matrix.scale_x(),
        start_angle,
        end_angle,
        extrusion,
    }
}

fn transform_text(data: &TextData, matrix: Transform2) -> TextData {
    let axis = Point2::new(data.rotation.cos(), data.rotation.sin());
    let new_axis = matrix.apply_vector(axis);
    TextData {
        insertion: matrix.apply3(data.insertion),
        height: data.height.abs() * matrix.scale_x(),
        rotation: new_axis.y.atan2(new_axis.x),
        value: data.value.clone(),
        extrusion: data.extrusion,
        is_attrib_def: data.is_attrib_def,
    }
}

fn transform_mtext(data: &MTextData, matrix: Transform2) -> MTextData {
    let axis = Point2::new(data.rotation.cos(), data.rotation.sin());
    let new_axis = matrix.apply_vector(axis);
    MTextData {
        insertion: matrix.apply3(data.insertion),
        height: data.height.abs() * matrix.scale_x(),
        rotation: new_axis.y.atan2(new_axis.x),
        width: data.width.abs() * matrix.scale_x(),
        value: data.value.clone(),
        extrusion: data.extrusion,
    }
}

fn transform_hatch(hatch: &HatchData, matrix: Transform2) -> Result<HatchData, TransformError> {
    let mut paths = Vec::with_capacity(hatch.paths.len());
    for path in &hatch.paths {
        paths.push(match path {
            HatchPath::Polyline { vertices, closed } => HatchPath::Polyline {
                vertices: transform_poly_vertices(vertices, matrix),
                closed: *closed,
            },
            HatchPath::Edges(edges) => {
                let mut out = Vec::with_capacity(edges.len());
                for edge in edges {
                    out.push(transform_hatch_edge(edge, matrix)?);
                }
                HatchPath::Edges(out)
            }
        });
    }
    let pattern_lines = hatch
        .pattern_lines
        .iter()
        .map(|line| transform_pattern_line(line, matrix))
        .collect();
    Ok(HatchData {
        extrusion: hatch.extrusion,
        elevation: hatch.elevation,
        solid_fill: hatch.solid_fill,
        paths,
        pattern_lines,
    })
}

fn transform_hatch_edge(edge: &HatchEdge, matrix: Transform2) -> Result<HatchEdge, TransformError> {
    Ok(match edge {
        HatchEdge::Line { start, end } => HatchEdge::Line {
            start: matrix.apply3(*start),
            end: matrix.apply3(*end),
        },
        HatchEdge::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            is_ccw,
        } => match transform_arc(
            *center,
            *radius,
            *start_angle,
            *end_angle,
            crate::entity::default_extrusion(),
            matrix,
        ) {
            Geometry::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                ..
            } => HatchEdge::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                is_ccw: if matrix.reverses_orientation() {
                    !*is_ccw
                } else {
                    *is_ccw
                },
            },
            _ => return Err(TransformError::Unsupported(vec!["Hatch"])),
        },
        HatchEdge::Ellipse {
            center,
            major_endpoint,
            axis_ratio,
            start_angle,
            end_angle,
            is_ccw,
        } => HatchEdge::Ellipse {
            center: matrix.apply3(*center),
            major_endpoint: matrix.apply3(*major_endpoint),
            axis_ratio: *axis_ratio,
            start_angle: *start_angle,
            end_angle: *end_angle,
            is_ccw: if matrix.reverses_orientation() {
                !*is_ccw
            } else {
                *is_ccw
            },
        },
        HatchEdge::Spline { control_points } => HatchEdge::Spline {
            control_points: control_points.iter().map(|p| matrix.apply3(*p)).collect(),
        },
    })
}

fn transform_pattern_line(line: &HatchPatternLine, matrix: Transform2) -> HatchPatternLine {
    let axis = Point2::new(line.angle.cos(), line.angle.sin());
    let new_axis = matrix.apply_vector(axis);
    HatchPatternLine {
        angle: new_axis.y.atan2(new_axis.x),
        base: matrix.apply3(line.base),
        offset: apply_vector3(matrix, line.offset),
        dashes: line.dashes.clone(),
    }
}

fn decompose_insert(combined: Transform2, scale_z: f64) -> (Point2, Point3, f64) {
    let insertion = Point2::new(combined.tx, combined.ty);
    let sx = combined.scale_x().max(GEOM_TOLERANCE);
    let mut sy = combined.scale_y().max(GEOM_TOLERANCE);
    if combined.reverses_orientation() {
        sy = -sy;
    }
    (
        insertion,
        Point3::new(sx, sy, scale_z),
        combined.rotation_component(),
    )
}

fn apply_vector3(matrix: Transform2, v: Point3) -> Point3 {
    let xy = matrix.apply_vector(v.xy());
    Point3 {
        x: xy.x,
        y: xy.y,
        z: v.z,
    }
}

fn representative_points(geometry: &Geometry) -> Vec<Point2> {
    match geometry {
        Geometry::Line { start, end } => vec![start.xy(), end.xy()],
        Geometry::Point { position } => vec![position.xy()],
        Geometry::Circle { center, radius, .. } => {
            vec![center.xy(), Point2::new(center.x + radius, center.y)]
        }
        Geometry::Arc { center, radius, .. } => {
            vec![center.xy(), Point2::new(center.x + radius, center.y)]
        }
        Geometry::Ellipse {
            center, major_axis, ..
        } => vec![
            center.xy(),
            Point2::new(center.x + major_axis.x, center.y + major_axis.y),
        ],
        Geometry::LwPolyline { vertices, .. } | Geometry::Polyline { vertices, .. } => {
            vertices.iter().map(|v| v.point.xy()).collect()
        }
        Geometry::Spline {
            control_points,
            fit_points,
            ..
        } => control_points
            .iter()
            .chain(fit_points)
            .map(|p| p.xy())
            .collect(),
        Geometry::Insert { insertion, .. } => vec![insertion.xy()],
        Geometry::Text(data) => vec![data.insertion.xy()],
        Geometry::MText(data) => vec![data.insertion.xy()],
        Geometry::Solid { corners, .. } => corners.iter().map(|p| p.xy()).collect(),
        Geometry::Leader { vertices } | Geometry::MLine { vertices, .. } => {
            vertices.iter().map(|p| p.xy()).collect()
        }
        Geometry::Hatch(_) | Geometry::Dimension { .. } => Vec::new(),
    }
}

fn is_noop_angle(radians: f64) -> bool {
    let tau = std::f64::consts::TAU;
    let wrapped = ((radians % tau) + tau) % tau;
    wrapped <= ANGLE_NOOP || (tau - wrapped) <= ANGLE_NOOP
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::CadColor;
    use crate::entity::{default_extrusion, EntityId, HatchData};

    fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> Entity {
        Entity::new(Geometry::Line {
            start: Point3::from_xy(x0, y0),
            end: Point3::from_xy(x1, y1),
        })
    }

    fn circle(x: f64, y: f64, radius: f64) -> Entity {
        Entity::new(Geometry::Circle {
            center: Point3::from_xy(x, y),
            radius,
            extrusion: default_extrusion(),
        })
    }

    fn arc(start: f64, end: f64) -> Entity {
        Entity::new(Geometry::Arc {
            center: Point3::from_xy(0.0, 0.0),
            radius: 1.0,
            start_angle: start,
            end_angle: end,
            extrusion: default_extrusion(),
        })
    }

    #[test]
    fn rotate_90_around_non_origin_base() {
        let entity = line(11.0, 0.0, 12.0, 0.0);
        let moved = transform_entity(
            &entity,
            EntityTransform::Rotate {
                base: Point2::new(10.0, 0.0),
                radians: std::f64::consts::FRAC_PI_2,
            },
        )
        .expect("rotate");
        match moved.geometry {
            Geometry::Line { start, end } => {
                assert!((start.x - 10.0).abs() < 1e-12);
                assert!((start.y - 1.0).abs() < 1e-12);
                assert!((end.x - 10.0).abs() < 1e-12);
                assert!((end.y - 2.0).abs() < 1e-12);
                assert!((start.z).abs() < 1e-15);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn uniform_scale_changes_circle_and_arc_radius() {
        let scaled = transform_entity(
            &circle(4.0, 0.0, 2.0),
            EntityTransform::UniformScale {
                base: Point2::new(0.0, 0.0),
                factor: 3.0,
            },
        )
        .expect("scale");
        match scaled.geometry {
            Geometry::Circle { center, radius, .. } => {
                assert!((center.x - 12.0).abs() < 1e-12);
                assert!((radius - 6.0).abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
        let scaled_arc = transform_entity(
            &arc(0.0, std::f64::consts::FRAC_PI_2),
            EntityTransform::UniformScale {
                base: Point2::new(0.0, 0.0),
                factor: 0.5,
            },
        )
        .expect("arc scale");
        match scaled_arc.geometry {
            Geometry::Arc { radius, .. } => assert!((radius - 0.5).abs() < 1e-12),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn mirror_horizontal_vertical_and_angled_axes() {
        let entity = line(5.0, 3.0, 6.0, 3.0);
        let across_x = transform_entity(
            &entity,
            EntityTransform::Mirror {
                axis_start: Point2::new(0.0, 0.0),
                axis_end: Point2::new(1.0, 0.0),
            },
        )
        .expect("horizontal");
        match across_x.geometry {
            Geometry::Line { start, end } => {
                assert!((start.x - 5.0).abs() < 1e-12);
                assert!((start.y + 3.0).abs() < 1e-12);
                assert!((end.x - 6.0).abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
        let across_y = transform_entity(
            &entity,
            EntityTransform::Mirror {
                axis_start: Point2::new(0.0, 0.0),
                axis_end: Point2::new(0.0, 1.0),
            },
        )
        .expect("vertical");
        match across_y.geometry {
            Geometry::Line { start, .. } => {
                assert!((start.x + 5.0).abs() < 1e-12);
                assert!((start.y - 3.0).abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
        let diagonal = transform_entity(
            &line(1.0, 0.0, 2.0, 0.0),
            EntityTransform::Mirror {
                axis_start: Point2::new(0.0, 0.0),
                axis_end: Point2::new(1.0, 1.0),
            },
        )
        .expect("angled");
        match diagonal.geometry {
            Geometry::Line { start, end } => {
                assert!(start.x.abs() < 1e-12);
                assert!((start.y - 1.0).abs() < 1e-12);
                assert!(end.x.abs() < 1e-12);
                assert!((end.y - 2.0).abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn mirrored_polyline_negates_bulge() {
        let entity = Entity::new(Geometry::LwPolyline {
            vertices: vec![
                PolyVertex {
                    point: Point3::from_xy(0.0, 0.0),
                    bulge: 0.5,
                },
                PolyVertex {
                    point: Point3::from_xy(2.0, 0.0),
                    bulge: -0.25,
                },
            ],
            closed: false,
            extrusion: default_extrusion(),
            linetype_generation_continuous: false,
        });
        let mirrored = transform_entity(
            &entity,
            EntityTransform::Mirror {
                axis_start: Point2::new(0.0, 0.0),
                axis_end: Point2::new(1.0, 0.0),
            },
        )
        .expect("mirror");
        match mirrored.geometry {
            Geometry::LwPolyline { vertices, .. } => {
                assert!((vertices[0].bulge + 0.5).abs() < 1e-15);
                assert!((vertices[1].bulge - 0.25).abs() < 1e-15);
                assert!((vertices[0].point.y).abs() < 1e-12);
                assert!((vertices[1].point.y).abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn mirrored_arc_preserves_visible_sweep() {
        let quarter = arc(0.0, std::f64::consts::FRAC_PI_2);
        let mirrored = transform_entity(
            &quarter,
            EntityTransform::Mirror {
                axis_start: Point2::new(0.0, 0.0),
                axis_end: Point2::new(1.0, 0.0),
            },
        )
        .expect("arc mirror");
        match mirrored.geometry {
            Geometry::Arc {
                start_angle,
                end_angle,
                radius,
                ..
            } => {
                assert!((radius - 1.0).abs() < 1e-12);
                let start = Point2::new(start_angle.cos(), start_angle.sin());
                let end = Point2::new(end_angle.cos(), end_angle.sin());
                assert!((start.x).abs() < 1e-12);
                assert!((start.y + 1.0).abs() < 1e-12);
                assert!((end.x - 1.0).abs() < 1e-12);
                assert!(end.y.abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn insert_instance_moves_without_touching_block_name() {
        let entity = Entity::new(Geometry::Insert {
            block_name: "DOOR".into(),
            insertion: Point3::from_xy(10.0, 4.0),
            scale: Point3::new(2.0, 2.0, 1.0),
            rotation: 0.0,
            extrusion: default_extrusion(),
            attribs: Vec::new(),
            column_count: 1,
            row_count: 1,
            column_spacing: 0.0,
            row_spacing: 0.0,
        });
        let moved = transform_entity(&entity, EntityTransform::Translate { dx: 5.0, dy: -1.0 })
            .expect("move insert");
        match moved.geometry {
            Geometry::Insert {
                block_name,
                insertion,
                scale,
                rotation,
                ..
            } => {
                assert_eq!(block_name, "DOOR");
                assert!((insertion.x - 15.0).abs() < 1e-12);
                assert!((insertion.y - 3.0).abs() < 1e-12);
                assert!((scale.x - 2.0).abs() < 1e-12);
                assert!(rotation.abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn mixed_unsupported_selection_lists_types() {
        let ok = line(0.0, 0.0, 1.0, 0.0);
        let hatch = Entity::new(Geometry::Hatch(HatchData {
            extrusion: default_extrusion(),
            elevation: 0.0,
            solid_fill: true,
            paths: Vec::new(),
            pattern_lines: Vec::new(),
        }));
        let dimension = Entity::new(Geometry::Dimension {
            block_name: "*D1".into(),
        });
        let err = validate_entities([&ok, &hatch, &dimension]).unwrap_err();
        assert_eq!(err, TransformError::Unsupported(vec!["Dimension"]));
        assert_eq!(err.to_string(), "Cannot transform Dimension");
    }

    #[test]
    fn zero_mirror_axis_and_invalid_scale_are_rejected() {
        assert!(matches!(
            EntityTransform::Mirror {
                axis_start: Point2::new(1.0, 1.0),
                axis_end: Point2::new(1.0, 1.0),
            }
            .to_matrix(),
            Err(TransformError::Invalid(_))
        ));
        assert!(matches!(
            EntityTransform::UniformScale {
                base: Point2::new(0.0, 0.0),
                factor: 0.0,
            }
            .to_matrix(),
            Err(TransformError::Invalid(_))
        ));
        assert!(matches!(
            EntityTransform::UniformScale {
                base: Point2::new(0.0, 0.0),
                factor: -2.0,
            }
            .to_matrix(),
            Err(TransformError::Invalid(_))
        ));
        assert!(matches!(
            EntityTransform::UniformScale {
                base: Point2::new(0.0, 0.0),
                factor: f64::NAN,
            }
            .to_matrix(),
            Err(TransformError::Invalid(_))
        ));
    }

    #[test]
    fn no_op_transform_is_rejected() {
        assert_eq!(
            EntityTransform::Translate { dx: 0.0, dy: 0.0 }.to_matrix(),
            Err(TransformError::NoOp)
        );
        assert_eq!(
            EntityTransform::Rotate {
                base: Point2::new(1.0, 1.0),
                radians: std::f64::consts::TAU,
            }
            .to_matrix(),
            Err(TransformError::NoOp)
        );
        assert_eq!(
            EntityTransform::UniformScale {
                base: Point2::new(0.0, 0.0),
                factor: 1.0,
            }
            .to_matrix(),
            Err(TransformError::NoOp)
        );
    }

    #[test]
    fn copy_keeps_style_and_caller_assigns_new_ids() {
        let mut source = line(0.0, 0.0, 4.0, 0.0);
        source.id = EntityId(7);
        source.layer = "A-WALL".into();
        source.color = CadColor::Aci(3);
        source.linetype = "DASHED".into();
        source.linetype_scale = 2.0;
        let copy = transform_entity(&source, EntityTransform::Translate { dx: 10.0, dy: 0.0 })
            .expect("copy");
        assert_eq!(copy.id, source.id);
        assert_eq!(copy.layer, "A-WALL");
        assert_eq!(copy.color, CadColor::Aci(3));
        assert_eq!(copy.linetype, "DASHED");
        assert_eq!(copy.linetype_scale, 2.0);
        assert!(copy.visible);
    }

    #[test]
    fn text_stays_readable_after_mirror() {
        let entity = Entity::new(Geometry::Text(TextData {
            insertion: Point3::from_xy(4.0, 2.0),
            height: 2.5,
            rotation: 0.0,
            value: "ROOM".into(),
            extrusion: default_extrusion(),
            is_attrib_def: false,
        }));
        let mirrored = transform_entity(
            &entity,
            EntityTransform::Mirror {
                axis_start: Point2::new(0.0, 0.0),
                axis_end: Point2::new(0.0, 1.0),
            },
        )
        .expect("text");
        match mirrored.geometry {
            Geometry::Text(data) => {
                assert!((data.insertion.x + 4.0).abs() < 1e-12);
                assert!(data.height > 0.0);
                assert!(data.rotation.is_finite());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn insert_transform_does_not_mutate_block_definition() {
        use crate::document::{BlockDefinition, Document};
        let mut document = Document::default();
        document.blocks.insert(
            "DOOR".into(),
            BlockDefinition {
                name: "DOOR".into(),
                base_pt: Point3::from_xy(0.0, 0.0),
                entities: vec![line(0.0, 0.0, 1.0, 0.0)],
            },
        );
        let insert = document.add_entity(Entity::new(Geometry::Insert {
            block_name: "DOOR".into(),
            insertion: Point3::from_xy(10.0, 4.0),
            scale: Point3::new(1.0, 1.0, 1.0),
            rotation: 0.0,
            extrusion: default_extrusion(),
            attribs: Vec::new(),
            column_count: 1,
            row_count: 1,
            column_spacing: 0.0,
            row_spacing: 0.0,
        }));
        let moved = transform_entity(&insert, EntityTransform::Translate { dx: 5.0, dy: 0.0 })
            .expect("move insert");
        let _ = document.replace_model_entity(insert.id, moved);
        let def = document.blocks.get("DOOR").expect("block");
        match &def.entities[0].geometry {
            Geometry::Line { start, end } => {
                assert!((start.x).abs() < 1e-12);
                assert!((end.x - 1.0).abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn non_world_ocs_geometry_is_unsupported() {
        let entity = Entity::new(Geometry::Circle {
            center: Point3::from_xy(0.0, 0.0),
            radius: 1.0,
            extrusion: Point3::new(1.0, 0.0, 0.0),
        });
        let err = validate_entities([&entity]).unwrap_err();
        assert!(matches!(err, TransformError::Unsupported(_)));
    }
}
