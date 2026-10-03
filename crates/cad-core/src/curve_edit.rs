//! Pure trim, extend, offset, and stretch for world-space geometry.
//!
//! These functions do not touch a document. The editor supplies the
//! picked point, the cutting or boundary edges, and writes the result
//! back through the normal history path.

use std::fmt;

use crate::entity::{Geometry, PolyVertex};
use crate::entity_transform::{transform_geometry, EntityTransform, TransformError};
use crate::extents::Extents2;
use crate::geom::{is_world_extrusion, Point2, Point3, GEOM_TOLERANCE};
use crate::ids::VertexId;
use crate::intersect::{Curve, CurveHit};
use crate::measure::{infinite_line_intersection, point_segment_distance};

/// Cutting edges considered for one trim click.
pub const MAX_TRIM_EDGES: usize = 64;
/// Boundary edges considered for one extend click.
pub const MAX_EXTEND_EDGES: usize = 64;
/// Segments drawn in the stretch rubber-band.
pub const MAX_STRETCH_PREVIEW: usize = 256;

const SPLIT_EPS: f64 = 1e-6;
const MITER_LIMIT: f64 = 2.0;
const TAU: f64 = std::f64::consts::TAU;

// ------------------------------------------------------------
// Enum: EditError
// Purpose: Why a trim, extend, offset, or stretch produced nothing.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    Unsupported,
    NoIntersection,
    NothingToExtend,
    RadiusTooSmall,
    BulgedPolyline,
    Degenerate,
    NoOp,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "Cannot edit this object",
            Self::NoIntersection => "Does not intersect a cutting edge",
            Self::NothingToExtend => "Cannot extend to a boundary",
            Self::RadiusTooSmall => "Offset distance is larger than the radius",
            Self::BulgedPolyline => "Offset does not support bulged polylines",
            Self::Degenerate => "Result is too small",
            Self::NoOp => "Nothing to stretch",
        })
    }
}

impl From<TransformError> for EditError {
    fn from(err: TransformError) -> Self {
        match err {
            TransformError::NoOp => Self::NoOp,
            TransformError::Unsupported(_) | TransformError::Invalid(_) => Self::Unsupported,
        }
    }
}

// ------------------------------------------------------------
// Type: TrimResult
// Purpose: The pieces that replace the trimmed object. Empty means
//          the click removed the whole object.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub struct TrimResult {
    pub pieces: Vec<Geometry>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StretchOutcome {
    Unchanged,
    Replaced(Geometry),
}

struct ChainSeg {
    start: Point3,
    end: Point3,
    bulge: f64,
    start_id: VertexId,
    end_id: VertexId,
}

// ------------------------------------------------------------
// Function: trim_geometry
// Purpose: Remove the span of `geometry` that contains `pick`,
//          stopping at the nearest cutting edges on either side.
// ------------------------------------------------------------
pub fn trim_geometry(
    geometry: &Geometry,
    pick: Point2,
    edges: &[Geometry],
) -> Result<TrimResult, EditError> {
    if !pick.is_finite() {
        return Err(EditError::Degenerate);
    }
    let curves = edge_curves(edges, MAX_TRIM_EDGES);
    if curves.is_empty() {
        return Err(EditError::NoIntersection);
    }
    match geometry {
        Geometry::Line { start, end } => trim_line(*start, *end, pick, &curves),
        Geometry::Circle {
            center,
            radius,
            extrusion,
        } => trim_circle(*center, *radius, *extrusion, pick, &curves),
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            extrusion,
        } => trim_arc(
            *center,
            *radius,
            *start_angle,
            *end_angle,
            *extrusion,
            pick,
            &curves,
        ),
        Geometry::LwPolyline { .. } | Geometry::Polyline { .. } => {
            trim_polyline(geometry, pick, &curves)
        }
        _ => Err(EditError::Unsupported),
    }
}

// ------------------------------------------------------------
// Function: extend_geometry
// Purpose: Lengthen the end nearest `pick` until it meets a boundary.
// ------------------------------------------------------------
pub fn extend_geometry(
    geometry: &Geometry,
    pick: Point2,
    edges: &[Geometry],
) -> Result<Geometry, EditError> {
    if !pick.is_finite() {
        return Err(EditError::Degenerate);
    }
    let curves = edge_curves(edges, MAX_EXTEND_EDGES);
    if curves.is_empty() {
        return Err(EditError::NothingToExtend);
    }
    match geometry {
        Geometry::Line { start, end } => extend_line(*start, *end, pick, &curves),
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            extrusion,
        } => extend_arc(
            *center,
            *radius,
            *start_angle,
            *end_angle,
            *extrusion,
            pick,
            &curves,
        ),
        Geometry::LwPolyline { .. } | Geometry::Polyline { .. } => {
            extend_polyline(geometry, pick, &curves)
        }
        _ => Err(EditError::Unsupported),
    }
}

// ------------------------------------------------------------
// Function: offset_geometry
// Purpose: Parallel copy. `distance` is a positive magnitude and
//          `side` chooses which way the copy moves.
// ------------------------------------------------------------
pub fn offset_geometry(
    geometry: &Geometry,
    distance: f64,
    side: Point2,
) -> Result<Geometry, EditError> {
    if !distance.is_finite() || !side.is_finite() || distance <= GEOM_TOLERANCE {
        return Err(EditError::Degenerate);
    }
    match geometry {
        Geometry::Line { start, end } => offset_line(*start, *end, distance, side),
        Geometry::Circle {
            center,
            radius,
            extrusion,
        } => offset_circle(*center, *radius, *extrusion, distance, side),
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            extrusion,
        } => offset_arc(
            *center,
            *radius,
            *start_angle,
            *end_angle,
            *extrusion,
            distance,
            side,
        ),
        Geometry::LwPolyline {
            vertices,
            closed,
            extrusion,
            linetype_generation_continuous,
        } => {
            if vertices.iter().any(|vertex| vertex.bulge.abs() > 1e-9) {
                return Err(EditError::BulgedPolyline);
            }
            let points = offset_vertices(vertices, *closed, distance, side)?;
            Ok(Geometry::LwPolyline {
                vertices: points,
                closed: *closed,
                extrusion: *extrusion,
                linetype_generation_continuous: *linetype_generation_continuous,
            })
        }
        Geometry::Polyline {
            vertices,
            closed,
            linetype_generation_continuous,
        } => {
            if vertices.iter().any(|vertex| vertex.bulge.abs() > 1e-9) {
                return Err(EditError::BulgedPolyline);
            }
            let points = offset_vertices(vertices, *closed, distance, side)?;
            Ok(Geometry::Polyline {
                vertices: points,
                closed: *closed,
                linetype_generation_continuous: *linetype_generation_continuous,
            })
        }
        _ => Err(EditError::Unsupported),
    }
}

// ------------------------------------------------------------
// Function: stretch_geometry
// Purpose: Move line endpoints and polyline vertices that lie in
//          `window`. Any other object moves only when its defining
//          point lies in the window.
// ------------------------------------------------------------
pub fn stretch_geometry(
    geometry: &Geometry,
    window: Extents2,
    dx: f64,
    dy: f64,
) -> Result<StretchOutcome, EditError> {
    if !dx.is_finite() || !dy.is_finite() || !window.is_valid() {
        return Err(EditError::Degenerate);
    }
    match geometry {
        Geometry::Line { start, end } => {
            let (start_in, end_in) = (window.contains(start.xy()), window.contains(end.xy()));
            if !start_in && !end_in {
                return Ok(StretchOutcome::Unchanged);
            }
            if dx.abs() <= GEOM_TOLERANCE && dy.abs() <= GEOM_TOLERANCE {
                return Err(EditError::NoOp);
            }
            Ok(StretchOutcome::Replaced(Geometry::Line {
                start: if start_in {
                    translate3(*start, dx, dy)
                } else {
                    *start
                },
                end: if end_in {
                    translate3(*end, dx, dy)
                } else {
                    *end
                },
            }))
        }
        Geometry::LwPolyline { vertices, .. } | Geometry::Polyline { vertices, .. } => {
            if !vertices
                .iter()
                .any(|vertex| window.contains(vertex.point.xy()))
            {
                return Ok(StretchOutcome::Unchanged);
            }
            if dx.abs() <= GEOM_TOLERANCE && dy.abs() <= GEOM_TOLERANCE {
                return Err(EditError::NoOp);
            }
            let moved: Vec<PolyVertex> = vertices
                .iter()
                .map(|vertex| {
                    if window.contains(vertex.point.xy()) {
                        PolyVertex {
                            point: translate3(vertex.point, dx, dy),
                            ..*vertex
                        }
                    } else {
                        *vertex
                    }
                })
                .collect();
            Ok(StretchOutcome::Replaced(replace_vertices(geometry, moved)))
        }
        other => {
            let Some(anchor) = defining_point(other) else {
                return Ok(StretchOutcome::Unchanged);
            };
            if !window.contains(anchor) {
                return Ok(StretchOutcome::Unchanged);
            }
            if dx.abs() <= GEOM_TOLERANCE && dy.abs() <= GEOM_TOLERANCE {
                return Err(EditError::NoOp);
            }
            let moved =
                transform_geometry(other, EntityTransform::Translate { dx, dy }.to_matrix()?)?;
            Ok(StretchOutcome::Replaced(moved))
        }
    }
}

fn trim_line(
    start: Point3,
    end: Point3,
    pick: Point2,
    edges: &[Curve],
) -> Result<TrimResult, EditError> {
    let curve = Curve::segment(start.xy(), end.xy()).ok_or(EditError::Degenerate)?;
    let pick_t = curve
        .project_param(pick)
        .ok_or(EditError::Degenerate)?
        .clamp(0.0, 1.0);
    let cuts = segment_cuts(&curve, edges);
    pieces_from_span(&cuts, pick_t, 1.0, |from, to| {
        line_between(start, end, from, to)
    })
}

fn trim_circle(
    center: Point3,
    radius: f64,
    extrusion: Point3,
    pick: Point2,
    edges: &[Curve],
) -> Result<TrimResult, EditError> {
    require_world(extrusion)?;
    let curve = Curve::circle(center.xy(), radius).ok_or(EditError::Degenerate)?;
    let pick_angle = curve.project_param(pick).ok_or(EditError::Degenerate)?;
    let mut cuts = Vec::new();
    for edge in edges {
        for hit in curve.hits(edge, false) {
            cuts.push(hit.param.rem_euclid(TAU));
        }
    }
    dedupe_angles(&mut cuts);
    if cuts.len() < 2 {
        return Err(EditError::NoIntersection);
    }
    let Some((gap_start, gap_end)) = closed_gap(&cuts, pick_angle.rem_euclid(TAU), TAU) else {
        return Err(EditError::NoIntersection);
    };
    let kept_sweep = TAU - (gap_end - gap_start);
    if kept_sweep <= SPLIT_EPS || kept_sweep >= TAU - SPLIT_EPS {
        return Err(EditError::Degenerate);
    }
    Ok(TrimResult {
        pieces: vec![arc_geometry(center, radius, gap_end, kept_sweep, extrusion)],
    })
}

fn trim_arc(
    center: Point3,
    radius: f64,
    start_angle: f64,
    end_angle: f64,
    extrusion: Point3,
    pick: Point2,
    edges: &[Curve],
) -> Result<TrimResult, EditError> {
    require_world(extrusion)?;
    let curve =
        Curve::arc(center.xy(), radius, start_angle, end_angle).ok_or(EditError::Degenerate)?;
    let sweep = curve.signed_sweep().ok_or(EditError::Degenerate)?;
    let start = curve.start_angle().ok_or(EditError::Degenerate)?;
    let pick_angle = curve.project_param(pick).ok_or(EditError::Degenerate)?;
    let pick_t = travel(start, pick_angle, true) / sweep;
    if !(0.0..=1.0).contains(&pick_t) {
        return Err(EditError::Degenerate);
    }
    let mut locals = Vec::new();
    for edge in edges {
        for hit in curve.hits(edge, false) {
            let t = travel(start, hit.param, true) / sweep;
            if (SPLIT_EPS..1.0 - SPLIT_EPS).contains(&t) {
                locals.push(t);
            }
        }
    }
    dedupe_params(&mut locals);
    pieces_from_span(&locals, pick_t, 1.0, |from, to| {
        let piece_sweep = sweep * (to - from);
        if piece_sweep.abs() <= SPLIT_EPS {
            return None;
        }
        Some(arc_geometry(
            center,
            radius,
            start + sweep * from,
            piece_sweep,
            extrusion,
        ))
    })
}

fn trim_polyline(
    geometry: &Geometry,
    pick: Point2,
    edges: &[Curve],
) -> Result<TrimResult, EditError> {
    let (vertices, closed) = poly_parts(geometry).ok_or(EditError::Unsupported)?;
    if let Some(extrusion) = poly_extrusion(geometry) {
        require_world(extrusion)?;
    }
    let segs = chain_segments(vertices, closed);
    if segs.is_empty() {
        return Err(EditError::Degenerate);
    }
    let n = segs.len() as f64;
    let (pick_param, _) = project_chain(&segs, pick).ok_or(EditError::Degenerate)?;
    let mut cuts = Vec::new();
    for (index, seg) in segs.iter().enumerate() {
        let Some(curve) = seg_curve(seg) else {
            continue;
        };
        for edge in edges {
            for hit in curve.hits(edge, false) {
                let Some(local) = hit_local(&curve, seg, hit) else {
                    continue;
                };
                if !(SPLIT_EPS..1.0 - SPLIT_EPS).contains(&local) {
                    continue;
                }
                cuts.push(index as f64 + local);
            }
        }
    }
    dedupe_params(&mut cuts);
    if closed {
        if cuts.len() < 2 {
            return Err(EditError::NoIntersection);
        }
        let Some((gap_start, gap_end)) = closed_gap(&cuts, pick_param, n) else {
            return Err(EditError::NoIntersection);
        };
        let from = gap_end;
        let to = gap_start + n;
        let piece = chain_piece(geometry, &segs, from, to)?;
        return Ok(TrimResult {
            pieces: vec![piece],
        });
    }
    pieces_from_span(&cuts, pick_param, n, |from, to| {
        chain_piece(geometry, &segs, from, to).ok()
    })
}

fn extend_line(
    start: Point3,
    end: Point3,
    pick: Point2,
    edges: &[Curve],
) -> Result<Geometry, EditError> {
    let curve = Curve::segment(start.xy(), end.xy()).ok_or(EditError::Degenerate)?;
    let pick_t = curve.project_param(pick).ok_or(EditError::Degenerate)?;
    let toward_start = pick_t < 0.5;
    let mut best: Option<f64> = None;
    for edge in edges {
        for hit in curve.hits(edge, true) {
            if toward_start && hit.param < -SPLIT_EPS {
                best = Some(best.map_or(hit.param, |current| current.max(hit.param)));
            } else if !toward_start && hit.param > 1.0 + SPLIT_EPS {
                best = Some(best.map_or(hit.param, |current| current.min(hit.param)));
            }
        }
    }
    let param = best.ok_or(EditError::NothingToExtend)?;
    let point = curve.point_at(param);
    if toward_start {
        if point.distance(start.xy()) <= GEOM_TOLERANCE {
            return Err(EditError::NothingToExtend);
        }
        Ok(Geometry::Line {
            start: Point3::new(point.x, point.y, start.z),
            end,
        })
    } else {
        if point.distance(end.xy()) <= GEOM_TOLERANCE {
            return Err(EditError::NothingToExtend);
        }
        Ok(Geometry::Line {
            start,
            end: Point3::new(point.x, point.y, end.z),
        })
    }
}

fn extend_arc(
    center: Point3,
    radius: f64,
    start_angle: f64,
    end_angle: f64,
    extrusion: Point3,
    pick: Point2,
    edges: &[Curve],
) -> Result<Geometry, EditError> {
    require_world(extrusion)?;
    let curve =
        Curve::arc(center.xy(), radius, start_angle, end_angle).ok_or(EditError::Degenerate)?;
    let sweep = curve.signed_sweep().ok_or(EditError::Degenerate)?;
    let start_pt = curve.point_at(start_angle);
    let end_pt = curve.point_at(end_angle);
    let extend_start = pick.distance(start_pt) <= pick.distance(end_pt);
    let origin = if extend_start { start_angle } else { end_angle };
    let mut best_travel = f64::MAX;
    let mut best_angle = 0.0;
    let room = TAU - sweep;
    for edge in edges {
        for hit in curve.hits(edge, true) {
            let delta = if extend_start {
                travel(hit.param, origin, true)
            } else {
                travel(origin, hit.param, true)
            };
            if delta <= 1e-4 || delta >= room - 1e-4 {
                continue;
            }
            if delta < best_travel {
                best_travel = delta;
                best_angle = hit.param;
            }
        }
    }
    if !best_travel.is_finite() || best_travel == f64::MAX {
        return Err(EditError::NothingToExtend);
    }
    let new_sweep = sweep + best_travel;
    if new_sweep >= TAU - 1e-4 {
        return Err(EditError::Degenerate);
    }
    let new_start = if extend_start {
        best_angle
    } else {
        start_angle
    };
    Ok(arc_geometry(
        center, radius, new_start, new_sweep, extrusion,
    ))
}

fn extend_polyline(
    geometry: &Geometry,
    pick: Point2,
    edges: &[Curve],
) -> Result<Geometry, EditError> {
    let (vertices, closed) = poly_parts(geometry).ok_or(EditError::Unsupported)?;
    if closed || vertices.len() < 2 {
        return Err(EditError::Unsupported);
    }
    if let Some(extrusion) = poly_extrusion(geometry) {
        require_world(extrusion)?;
    }
    let extend_start = pick.distance(vertices[0].point.xy())
        <= pick.distance(vertices[vertices.len() - 1].point.xy());
    let mut moved = vertices.to_vec();
    if extend_start {
        let (point, bulge) = extend_chain_end(
            vertices[1].point,
            vertices[0].point,
            -vertices[0].bulge,
            edges,
            true,
        )?;
        moved[0].point = point;
        moved[0].bulge = -bulge;
    } else {
        let last = vertices.len() - 1;
        let (point, bulge) = extend_chain_end(
            vertices[last - 1].point,
            vertices[last].point,
            vertices[last - 1].bulge,
            edges,
            false,
        )?;
        moved[last].point = point;
        moved[last - 1].bulge = bulge;
    }
    Ok(replace_vertices(geometry, moved))
}

fn extend_chain_end(
    start: Point3,
    end: Point3,
    bulge: f64,
    edges: &[Curve],
    toward_start_of_segment: bool,
) -> Result<(Point3, f64), EditError> {
    if bulge.abs() <= 1e-9 {
        let extended = extend_line(
            start,
            end,
            if toward_start_of_segment {
                start.xy()
            } else {
                end.xy()
            },
            edges,
        )?;
        let Geometry::Line {
            start: new_start,
            end: new_end,
        } = extended
        else {
            return Err(EditError::Degenerate);
        };
        return Ok(if toward_start_of_segment {
            (new_start, 0.0)
        } else {
            (new_end, 0.0)
        });
    }
    let curve = Curve::bulge(start.xy(), end.xy(), bulge).ok_or(EditError::Degenerate)?;
    let sweep = curve.signed_sweep().ok_or(EditError::Degenerate)?;
    let origin_angle = curve.start_angle().ok_or(EditError::Degenerate)?
        + if toward_start_of_segment { 0.0 } else { sweep };
    let room = TAU - sweep.abs();
    let mut best_travel = f64::MAX;
    let mut best_point = end.xy();
    let ccw = sweep >= 0.0;
    for edge in edges {
        for hit in curve.hits(edge, true) {
            let delta = if toward_start_of_segment {
                travel(hit.param, origin_angle, ccw)
            } else {
                travel(origin_angle, hit.param, ccw)
            };
            if delta <= 1e-4 || delta >= room - 1e-4 {
                continue;
            }
            if delta < best_travel {
                best_travel = delta;
                best_point = hit.point;
            }
        }
    }
    if best_travel == f64::MAX {
        return Err(EditError::NothingToExtend);
    }
    let new_sweep = sweep.signum() * (sweep.abs() + best_travel);
    let moved = if toward_start_of_segment { start } else { end };
    Ok((
        Point3::new(best_point.x, best_point.y, moved.z),
        bulge_for_sweep(new_sweep),
    ))
}

fn offset_line(
    start: Point3,
    end: Point3,
    distance: f64,
    side: Point2,
) -> Result<Geometry, EditError> {
    let normal = left_normal(start.xy(), end.xy()).ok_or(EditError::Degenerate)?;
    let signed = distance * side_sign(normal, start.xy(), side);
    let offset = normal * signed;
    Ok(Geometry::Line {
        start: Point3::new(start.x + offset.x, start.y + offset.y, start.z),
        end: Point3::new(end.x + offset.x, end.y + offset.y, end.z),
    })
}

fn offset_circle(
    center: Point3,
    radius: f64,
    extrusion: Point3,
    distance: f64,
    side: Point2,
) -> Result<Geometry, EditError> {
    require_world(extrusion)?;
    let inside = side.distance(center.xy()) < radius;
    let new_radius = if inside {
        radius - distance
    } else {
        radius + distance
    };
    if new_radius <= GEOM_TOLERANCE {
        return Err(EditError::RadiusTooSmall);
    }
    Ok(Geometry::Circle {
        center,
        radius: new_radius,
        extrusion,
    })
}

fn offset_arc(
    center: Point3,
    radius: f64,
    start_angle: f64,
    end_angle: f64,
    extrusion: Point3,
    distance: f64,
    side: Point2,
) -> Result<Geometry, EditError> {
    require_world(extrusion)?;
    let inside = side.distance(center.xy()) < radius;
    let new_radius = if inside {
        radius - distance
    } else {
        radius + distance
    };
    if new_radius <= GEOM_TOLERANCE {
        return Err(EditError::RadiusTooSmall);
    }
    Ok(Geometry::Arc {
        center,
        radius: new_radius,
        start_angle,
        end_angle,
        extrusion,
    })
}

fn offset_vertices(
    vertices: &[PolyVertex],
    closed: bool,
    distance: f64,
    side: Point2,
) -> Result<Vec<PolyVertex>, EditError> {
    if vertices.len() < 2 || (closed && vertices.len() < 3) {
        return Err(EditError::Degenerate);
    }
    let flat: Vec<Point2> = vertices.iter().map(|vertex| vertex.point.xy()).collect();
    let sign = chain_side_sign(&flat, closed, side).ok_or(EditError::Degenerate)?;
    let signed = distance * sign;
    let count = if closed {
        vertices.len()
    } else {
        vertices.len() - 1
    };
    let mut shifted = Vec::with_capacity(count);
    for index in 0..count {
        let a = vertices[index].point;
        let b = vertices[(index + 1) % vertices.len()].point;
        let normal = left_normal(a.xy(), b.xy()).ok_or(EditError::Degenerate)?;
        let offset = normal * signed;
        shifted.push((
            Point2::new(a.x + offset.x, a.y + offset.y),
            Point2::new(b.x + offset.x, b.y + offset.y),
            a.z,
        ));
    }
    let limit = MITER_LIMIT * distance;
    let mut out = Vec::new();
    if !closed {
        out.push(poly_at(
            shifted[0].0,
            vertices[0].point.z,
            vertices[0].vertex_id,
        ));
    }
    if closed {
        for index in 0..count {
            let prev = if index == 0 { count - 1 } else { index - 1 };
            let z = vertices[index].point.z;
            let id = vertices[index].vertex_id;
            push_join(&mut out, shifted[prev], shifted[index], limit, z, id);
        }
    } else {
        for index in 1..count {
            let z = vertices[index].point.z;
            let id = vertices[index].vertex_id;
            push_join(&mut out, shifted[index - 1], shifted[index], limit, z, id);
        }
    }
    if !closed {
        let last = vertices.len() - 1;
        out.push(poly_at(
            shifted[count - 1].1,
            vertices[last].point.z,
            vertices[last].vertex_id,
        ));
    }
    compact_vertices(&mut out);
    if out.len() < 2 || (closed && out.len() < 3) {
        return Err(EditError::Degenerate);
    }
    Ok(out)
}

fn push_join(
    out: &mut Vec<PolyVertex>,
    prev: (Point2, Point2, f64),
    next: (Point2, Point2, f64),
    limit: f64,
    z: f64,
    id: VertexId,
) {
    if let Some(point) = infinite_line_intersection(prev.0, prev.1, next.0, next.1) {
        if prev.1.distance(point) <= limit && next.0.distance(point) <= limit {
            out.push(poly_at(point, z, id));
            return;
        }
    }
    out.push(poly_at(prev.1, z, id));
    out.push(poly_at(next.0, z, VertexId::UNASSIGNED));
}

fn pieces_from_span<F>(
    cuts: &[f64],
    pick: f64,
    end: f64,
    mut build: F,
) -> Result<TrimResult, EditError>
where
    F: FnMut(f64, f64) -> Option<Geometry>,
{
    if cuts.is_empty() {
        return Err(EditError::NoIntersection);
    }
    let lower = cuts
        .iter()
        .copied()
        .filter(|cut| *cut < pick - SPLIT_EPS)
        .reduce(f64::max);
    let upper = cuts
        .iter()
        .copied()
        .filter(|cut| *cut > pick + SPLIT_EPS)
        .reduce(f64::min);
    let mut pieces = Vec::new();
    match (lower, upper) {
        (Some(from_cut), Some(to_cut)) => {
            if let Some(piece) = build(0.0, from_cut) {
                pieces.push(piece);
            }
            if let Some(piece) = build(to_cut, end) {
                pieces.push(piece);
            }
        }
        (None, Some(to_cut)) => {
            if let Some(piece) = build(to_cut, end) {
                pieces.push(piece);
            }
        }
        (Some(from_cut), None) => {
            if let Some(piece) = build(0.0, from_cut) {
                pieces.push(piece);
            }
        }
        (None, None) => return Err(EditError::NoIntersection),
    }
    if pieces.is_empty() {
        return Err(EditError::Degenerate);
    }
    Ok(TrimResult { pieces })
}

fn chain_piece(
    source: &Geometry,
    segs: &[ChainSeg],
    from: f64,
    to: f64,
) -> Result<Geometry, EditError> {
    if to - from <= SPLIT_EPS {
        return Err(EditError::Degenerate);
    }
    let mut params = vec![from];
    let mut vertex = from.floor() as i64 + 1;
    while (vertex as f64) < to - SPLIT_EPS {
        let param = vertex as f64;
        if param > from + SPLIT_EPS {
            params.push(param);
        }
        vertex += 1;
    }
    if params
        .last()
        .is_some_and(|last| (last - to).abs() > SPLIT_EPS)
    {
        params.push(to);
    }
    let mut vertices = Vec::new();
    for pair in params.windows(2) {
        let (point, id) = point_at_param(segs, pair[0]);
        let bulge = bulge_between(segs, pair[0], pair[1]);
        if vertices
            .last()
            .is_some_and(|last: &PolyVertex| last.point.xy().distance(point.xy()) <= GEOM_TOLERANCE)
        {
            continue;
        }
        vertices.push(PolyVertex {
            point,
            bulge,
            vertex_id: id,
        });
    }
    let Some(end_param) = params.last().copied() else {
        return Err(EditError::Degenerate);
    };
    let (end_point, end_id) = point_at_param(segs, end_param);
    if vertices
        .last()
        .is_none_or(|last: &PolyVertex| last.point.xy().distance(end_point.xy()) > GEOM_TOLERANCE)
    {
        vertices.push(PolyVertex {
            point: end_point,
            bulge: 0.0,
            vertex_id: end_id,
        });
    }
    if vertices.len() < 2 {
        return Err(EditError::Degenerate);
    }
    Ok(replace_vertices_open(source, vertices))
}

fn point_at_param(segs: &[ChainSeg], param: f64) -> (Point3, VertexId) {
    let n = segs.len();
    let nf = n as f64;
    if param > nf + SPLIT_EPS {
        return point_at_param(segs, param - nf);
    }
    if (param - nf).abs() <= SPLIT_EPS {
        let seg = &segs[n - 1];
        return (seg.end, seg.end_id);
    }
    let index = (param.floor().max(0.0) as usize).min(n - 1);
    let t = (param - index as f64).clamp(0.0, 1.0);
    let seg = &segs[index];
    if t <= SPLIT_EPS {
        return (seg.start, seg.start_id);
    }
    if t >= 1.0 - SPLIT_EPS {
        return (seg.end, seg.end_id);
    }
    (point_on_seg(seg, t), VertexId::UNASSIGNED)
}

fn bulge_between(segs: &[ChainSeg], from: f64, to: f64) -> f64 {
    let n = segs.len();
    let mut start = from;
    if start >= n as f64 {
        start -= n as f64;
    }
    let index = (start.floor().max(0.0) as usize) % n;
    let t0 = (start - start.floor()).clamp(0.0, 1.0);
    let t1 = if (to - start).abs() < 1.0 - SPLIT_EPS && to.floor() == start.floor() {
        (to - to.floor()).clamp(0.0, 1.0)
    } else {
        1.0
    };
    partial_bulge(segs[index].bulge, t0, t1.max(t0))
}

fn point_on_seg(seg: &ChainSeg, t: f64) -> Point3 {
    if seg.bulge.abs() <= 1e-9 {
        return Point3::new(
            seg.start.x + (seg.end.x - seg.start.x) * t,
            seg.start.y + (seg.end.y - seg.start.y) * t,
            seg.start.z + (seg.end.z - seg.start.z) * t,
        );
    }
    let Some(curve) = Curve::bulge(seg.start.xy(), seg.end.xy(), seg.bulge) else {
        return seg.start;
    };
    let start = curve.start_angle().unwrap_or(0.0);
    let sweep = curve.signed_sweep().unwrap_or(0.0);
    let point = curve.point_at(start + sweep * t);
    Point3::new(
        point.x,
        point.y,
        seg.start.z + (seg.end.z - seg.start.z) * t,
    )
}

fn partial_bulge(bulge: f64, t0: f64, t1: f64) -> f64 {
    if bulge.abs() <= 1e-9 || t1 - t0 <= SPLIT_EPS {
        return 0.0;
    }
    let sweep = 4.0 * bulge.atan() * (t1 - t0);
    bulge_for_sweep(sweep)
}

fn bulge_for_sweep(sweep: f64) -> f64 {
    (sweep / 4.0).tan()
}

fn local_on_seg(curve: &Curve, seg: &ChainSeg, pick: Point2) -> f64 {
    if seg.bulge.abs() <= 1e-9 {
        return curve.project_param(pick).unwrap_or(0.0).clamp(0.0, 1.0);
    }
    if let Some(angle) = curve.project_param(pick) {
        if let Some(local) = hit_local(
            curve,
            seg,
            CurveHit {
                point: pick,
                param: angle,
            },
        ) {
            return local;
        }
    }
    if pick.distance(seg.start.xy()) <= pick.distance(seg.end.xy()) {
        0.0
    } else {
        1.0
    }
}

fn project_chain(segs: &[ChainSeg], pick: Point2) -> Option<(f64, f64)> {
    let mut best_param = 0.0;
    let mut best_dist = f64::MAX;
    for (index, seg) in segs.iter().enumerate() {
        let curve = seg_curve(seg)?;
        let dist = curve.distance_to(pick);
        if dist < best_dist {
            best_dist = dist;
            let local = local_on_seg(&curve, seg, pick);
            best_param = index as f64 + local;
        }
    }
    (best_dist.is_finite()).then_some((best_param, best_dist))
}

fn seg_curve(seg: &ChainSeg) -> Option<Curve> {
    if seg.bulge.abs() <= 1e-9 {
        Curve::segment(seg.start.xy(), seg.end.xy())
    } else {
        Curve::bulge(seg.start.xy(), seg.end.xy(), seg.bulge)
    }
}

fn hit_local(curve: &Curve, seg: &ChainSeg, hit: CurveHit) -> Option<f64> {
    if seg.bulge.abs() <= 1e-9 {
        return (0.0..=1.0).contains(&hit.param).then_some(hit.param);
    }
    let start = curve.start_angle()?;
    let sweep = curve.signed_sweep()?;
    if sweep.abs() <= GEOM_TOLERANCE {
        return None;
    }
    let ccw = sweep >= 0.0;
    let delta = travel(start, hit.param, ccw);
    if delta > sweep.abs() + 1e-5 {
        return None;
    }
    Some((delta / sweep.abs()).clamp(0.0, 1.0))
}

fn segment_cuts(curve: &Curve, edges: &[Curve]) -> Vec<f64> {
    let mut cuts = Vec::new();
    for edge in edges {
        for hit in curve.hits(edge, false) {
            if (SPLIT_EPS..1.0 - SPLIT_EPS).contains(&hit.param) {
                cuts.push(hit.param);
            }
        }
    }
    dedupe_params(&mut cuts);
    cuts
}

fn chain_segments(vertices: &[PolyVertex], closed: bool) -> Vec<ChainSeg> {
    let count = if closed {
        vertices.len()
    } else {
        vertices.len().saturating_sub(1)
    };
    if vertices.len() < 2 {
        return Vec::new();
    }
    (0..count)
        .map(|index| {
            let next = (index + 1) % vertices.len();
            ChainSeg {
                start: vertices[index].point,
                end: vertices[next].point,
                bulge: vertices[index].bulge,
                start_id: vertices[index].vertex_id,
                end_id: vertices[next].vertex_id,
            }
        })
        .collect()
}

fn edge_curves(edges: &[Geometry], cap: usize) -> Vec<Curve> {
    let mut out = Vec::new();
    for geometry in edges.iter().take(cap) {
        if out.len() >= cap {
            break;
        }
        push_geometry_curves(geometry, &mut out, cap);
    }
    out
}

fn push_geometry_curves(geometry: &Geometry, out: &mut Vec<Curve>, cap: usize) {
    match geometry {
        Geometry::Line { start, end } => {
            if let Some(curve) = Curve::segment(start.xy(), end.xy()) {
                out.push(curve);
            }
        }
        Geometry::Circle {
            center,
            radius,
            extrusion,
        } => {
            if is_world_extrusion(*extrusion) {
                if let Some(curve) = Curve::circle(center.xy(), *radius) {
                    out.push(curve);
                }
            }
        }
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            extrusion,
        } => {
            if is_world_extrusion(*extrusion) {
                if let Some(curve) = Curve::arc(center.xy(), *radius, *start_angle, *end_angle) {
                    out.push(curve);
                }
            }
        }
        Geometry::LwPolyline {
            vertices,
            closed,
            extrusion,
            ..
        } => {
            if !is_world_extrusion(*extrusion) {
                return;
            }
            push_poly_curves(vertices, *closed, out, cap);
        }
        Geometry::Polyline {
            vertices, closed, ..
        } => {
            push_poly_curves(vertices, *closed, out, cap);
        }
        _ => {}
    }
}

fn push_poly_curves(vertices: &[PolyVertex], closed: bool, out: &mut Vec<Curve>, cap: usize) {
    for seg in chain_segments(vertices, closed) {
        if out.len() >= cap {
            break;
        }
        if let Some(curve) = seg_curve(&seg) {
            out.push(curve);
        }
    }
}

fn closed_gap(cuts: &[f64], pick: f64, period: f64) -> Option<(f64, f64)> {
    if cuts.len() < 2 {
        return None;
    }
    let mut ordered = cuts.to_vec();
    ordered.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    dedupe_params(&mut ordered);
    let pick = pick.rem_euclid(period);
    for pair in ordered.windows(2) {
        if pair[0] < pick && pick < pair[1] {
            return Some((pair[0], pair[1]));
        }
    }
    let start = *ordered.last()?;
    let end = ordered[0] + period;
    let pick_u = if pick >= start { pick } else { pick + period };
    (start < pick_u && pick_u < end).then_some((start, end))
}

fn line_between(start: Point3, end: Point3, from: f64, to: f64) -> Option<Geometry> {
    if to - from <= SPLIT_EPS {
        return None;
    }
    let a = lerp3(start, end, from);
    let b = lerp3(start, end, to);
    if a.xy().distance(b.xy()) <= GEOM_TOLERANCE {
        return None;
    }
    Some(Geometry::Line { start: a, end: b })
}

fn arc_geometry(
    center: Point3,
    radius: f64,
    start_angle: f64,
    sweep: f64,
    extrusion: Point3,
) -> Geometry {
    let (start_angle, end_angle) = if sweep >= 0.0 {
        (start_angle, start_angle + sweep)
    } else {
        (start_angle + sweep, start_angle)
    };
    Geometry::Arc {
        center,
        radius,
        start_angle,
        end_angle,
        extrusion,
    }
}

fn defining_point(geometry: &Geometry) -> Option<Point2> {
    match geometry {
        Geometry::Point { position }
        | Geometry::Circle {
            center: position, ..
        }
        | Geometry::Arc {
            center: position, ..
        }
        | Geometry::Ellipse {
            center: position, ..
        }
        | Geometry::Insert {
            insertion: position,
            ..
        } => Some(position.xy()),
        Geometry::Text(text) => Some(text.insertion.xy()),
        Geometry::MText(text) => Some(text.insertion.xy()),
        Geometry::Spline {
            control_points,
            fit_points,
            ..
        } => control_points
            .first()
            .or(fit_points.first())
            .map(|point| point.xy()),
        Geometry::Solid { corners, .. } => Some(corners[0].xy()),
        Geometry::Leader { vertices } | Geometry::MLine { vertices, .. } => {
            vertices.first().map(|point| point.xy())
        }
        _ => None,
    }
}

fn replace_vertices(geometry: &Geometry, vertices: Vec<PolyVertex>) -> Geometry {
    match geometry {
        Geometry::LwPolyline {
            closed,
            extrusion,
            linetype_generation_continuous,
            ..
        } => Geometry::LwPolyline {
            vertices,
            closed: *closed,
            extrusion: *extrusion,
            linetype_generation_continuous: *linetype_generation_continuous,
        },
        Geometry::Polyline {
            closed,
            linetype_generation_continuous,
            ..
        } => Geometry::Polyline {
            vertices,
            closed: *closed,
            linetype_generation_continuous: *linetype_generation_continuous,
        },
        _ => geometry.clone(),
    }
}

fn replace_vertices_open(geometry: &Geometry, vertices: Vec<PolyVertex>) -> Geometry {
    match geometry {
        Geometry::LwPolyline {
            extrusion,
            linetype_generation_continuous,
            ..
        } => Geometry::LwPolyline {
            vertices,
            closed: false,
            extrusion: *extrusion,
            linetype_generation_continuous: *linetype_generation_continuous,
        },
        Geometry::Polyline {
            linetype_generation_continuous,
            ..
        } => Geometry::Polyline {
            vertices,
            closed: false,
            linetype_generation_continuous: *linetype_generation_continuous,
        },
        _ => geometry.clone(),
    }
}

fn poly_parts(geometry: &Geometry) -> Option<(&[PolyVertex], bool)> {
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

fn poly_extrusion(geometry: &Geometry) -> Option<Point3> {
    match geometry {
        Geometry::LwPolyline { extrusion, .. } => Some(*extrusion),
        _ => None,
    }
}

fn require_world(extrusion: Point3) -> Result<(), EditError> {
    if is_world_extrusion(extrusion) {
        Ok(())
    } else {
        Err(EditError::Unsupported)
    }
}

fn left_normal(start: Point2, end: Point2) -> Option<Point2> {
    let delta = end - start;
    let len = (delta.x * delta.x + delta.y * delta.y).sqrt();
    if len <= GEOM_TOLERANCE {
        return None;
    }
    Some(Point2::new(-delta.y / len, delta.x / len))
}

fn side_sign(left: Point2, origin: Point2, side: Point2) -> f64 {
    let to = side - origin;
    if left.x * to.x + left.y * to.y >= 0.0 {
        1.0
    } else {
        -1.0
    }
}

fn chain_side_sign(vertices: &[Point2], closed: bool, side: Point2) -> Option<f64> {
    let count = if closed {
        vertices.len()
    } else {
        vertices.len() - 1
    };
    let mut best = 0;
    let mut best_dist = f64::MAX;
    for index in 0..count {
        let dist = point_segment_distance(
            side,
            vertices[index],
            vertices[(index + 1) % vertices.len()],
        );
        if dist < best_dist {
            best_dist = dist;
            best = index;
        }
    }
    let normal = left_normal(vertices[best], vertices[(best + 1) % vertices.len()])?;
    Some(side_sign(normal, vertices[best], side))
}

fn poly_at(point: Point2, z: f64, vertex_id: VertexId) -> PolyVertex {
    PolyVertex {
        point: Point3::new(point.x, point.y, z),
        bulge: 0.0,
        vertex_id,
    }
}

fn compact_vertices(vertices: &mut Vec<PolyVertex>) {
    let mut compact = Vec::with_capacity(vertices.len());
    for vertex in vertices.drain(..) {
        if compact
            .last()
            .is_some_and(|last: &PolyVertex| last.point.xy().distance(vertex.point.xy()) <= 1e-8)
        {
            continue;
        }
        compact.push(vertex);
    }
    *vertices = compact;
}

fn translate3(point: Point3, dx: f64, dy: f64) -> Point3 {
    Point3::new(point.x + dx, point.y + dy, point.z)
}

fn lerp3(start: Point3, end: Point3, t: f64) -> Point3 {
    Point3::new(
        start.x + (end.x - start.x) * t,
        start.y + (end.y - start.y) * t,
        start.z + (end.z - start.z) * t,
    )
}

fn travel(from: f64, to: f64, ccw: bool) -> f64 {
    if ccw {
        (to - from).rem_euclid(TAU)
    } else {
        (from - to).rem_euclid(TAU)
    }
}

fn dedupe_params(values: &mut Vec<f64>) {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    values.dedup_by(|a, b| (*a - *b).abs() <= SPLIT_EPS);
}

fn dedupe_angles(values: &mut Vec<f64>) {
    dedupe_params(values);
    if values.len() >= 2 && (values[0] + TAU - values[values.len() - 1]).abs() <= SPLIT_EPS {
        values.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::default_extrusion;

    fn near(point: Point2, x: f64, y: f64) -> bool {
        point.distance(Point2::new(x, y)) < 1e-5
    }

    fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> Geometry {
        Geometry::Line {
            start: Point3::new(x0, y0, 0.0),
            end: Point3::new(x1, y1, 0.0),
        }
    }

    fn fence(x: f64) -> Geometry {
        line(x, -10.0, x, 10.0)
    }

    #[test]
    fn trim_line_middle_splits_and_end_shortens() {
        let target = line(0.0, 0.0, 10.0, 0.0);
        let edges = [fence(3.0), fence(7.0)];
        let middle = trim_geometry(&target, Point2::new(5.0, 0.0), &edges).unwrap();
        assert_eq!(middle.pieces.len(), 2);
        match &middle.pieces[0] {
            Geometry::Line { start, end } => {
                assert!(near(start.xy(), 0.0, 0.0));
                assert!(near(end.xy(), 3.0, 0.0));
            }
            _ => panic!("expected a line"),
        }
        match &middle.pieces[1] {
            Geometry::Line { start, end } => {
                assert!(near(start.xy(), 7.0, 0.0));
                assert!(near(end.xy(), 10.0, 0.0));
            }
            _ => panic!("expected a line"),
        }

        let end = trim_geometry(&target, Point2::new(9.0, 0.0), &[fence(7.0)]).unwrap();
        assert_eq!(end.pieces.len(), 1);
        match &end.pieces[0] {
            Geometry::Line { end, .. } => assert!(near(end.xy(), 7.0, 0.0)),
            _ => panic!("expected a line"),
        }
    }

    #[test]
    fn trim_circle_and_arc() {
        let circle = Geometry::Circle {
            center: Point3::new(0.0, 0.0, 0.0),
            radius: 1.0,
            extrusion: default_extrusion(),
        };
        let cutter = line(0.0, -2.0, 0.0, 2.0);
        let trimmed = trim_geometry(&circle, Point2::new(1.0, 0.0), &[cutter]).unwrap();
        assert_eq!(trimmed.pieces.len(), 1);
        match &trimmed.pieces[0] {
            Geometry::Arc {
                radius,
                start_angle,
                end_angle,
                ..
            } => {
                assert!((radius - 1.0).abs() < 1e-6);
                let sweep = (*end_angle - *start_angle).rem_euclid(TAU);
                assert!((sweep - std::f64::consts::PI).abs() < 1e-4);
                let pick = 0.0;
                let delta = (pick - *start_angle).rem_euclid(TAU);
                assert!(delta > sweep + 1e-3 || delta < -1e-3);
            }
            _ => panic!("circle trim should become an arc"),
        }

        let arc = Geometry::Arc {
            center: Point3::new(0.0, 0.0, 0.0),
            radius: 5.0,
            start_angle: 0.0,
            end_angle: std::f64::consts::FRAC_PI_2,
            extrusion: default_extrusion(),
        };
        let ray = line(0.0, 0.0, 10.0, 10.0);
        let shortened = trim_geometry(&arc, Point2::new(0.0, 5.0), &[ray]).unwrap();
        assert_eq!(shortened.pieces.len(), 1);
        match &shortened.pieces[0] {
            Geometry::Arc { end_angle, .. } => {
                assert!((*end_angle - std::f64::consts::FRAC_PI_4).abs() < 1e-4);
            }
            _ => panic!("expected an arc"),
        }
    }

    #[test]
    fn trim_lwpolyline_segment() {
        let poly = Geometry::LwPolyline {
            vertices: vec![
                PolyVertex::new(Point3::new(0.0, 0.0, 0.0), 0.0),
                PolyVertex::new(Point3::new(10.0, 0.0, 0.0), 0.0),
                PolyVertex::new(Point3::new(10.0, 4.0, 0.0), 0.0),
            ],
            closed: false,
            extrusion: default_extrusion(),
            linetype_generation_continuous: false,
        };
        let trimmed =
            trim_geometry(&poly, Point2::new(6.0, 0.0), &[fence(4.0), fence(8.0)]).unwrap();
        assert_eq!(trimmed.pieces.len(), 2);
    }

    #[test]
    fn extend_line_to_a_line_and_an_arc() {
        let target = line(0.0, 0.0, 1.0, 0.0);
        let extended = extend_geometry(&target, Point2::new(1.0, 0.0), &[fence(4.0)]).unwrap();
        match extended {
            Geometry::Line { end, .. } => assert!(near(end.xy(), 4.0, 0.0)),
            _ => panic!("expected a line"),
        }
        let boundary = Geometry::Arc {
            center: Point3::new(5.0, 0.0, 0.0),
            radius: 2.0,
            start_angle: std::f64::consts::PI / 2.0,
            end_angle: 3.0 * std::f64::consts::PI / 2.0,
            extrusion: default_extrusion(),
        };
        let to_arc = extend_geometry(&target, Point2::new(1.0, 0.0), &[boundary]).unwrap();
        match to_arc {
            Geometry::Line { end, .. } => assert!(near(end.xy(), 3.0, 0.0)),
            _ => panic!("expected a line"),
        }
    }

    #[test]
    fn offset_line_circle_and_rectangle() {
        let moved =
            offset_geometry(&line(0.0, 0.0, 10.0, 0.0), 2.0, Point2::new(0.0, 1.0)).unwrap();
        match moved {
            Geometry::Line { start, end } => {
                assert!(near(start.xy(), 0.0, 2.0));
                assert!(near(end.xy(), 10.0, 2.0));
            }
            _ => panic!("expected a line"),
        }
        let circle = Geometry::Circle {
            center: Point3::new(0.0, 0.0, 0.0),
            radius: 5.0,
            extrusion: default_extrusion(),
        };
        match offset_geometry(&circle, 1.0, Point2::new(9.0, 0.0)).unwrap() {
            Geometry::Circle { radius, .. } => assert!((radius - 6.0).abs() < 1e-9),
            _ => panic!("expected a circle"),
        }
        assert_eq!(
            offset_geometry(&circle, 5.0, Point2::new(0.0, 0.0)).unwrap_err(),
            EditError::RadiusTooSmall
        );

        let rect = Geometry::LwPolyline {
            vertices: vec![
                PolyVertex::new(Point3::new(0.0, 0.0, 0.0), 0.0),
                PolyVertex::new(Point3::new(10.0, 0.0, 0.0), 0.0),
                PolyVertex::new(Point3::new(10.0, 5.0, 0.0), 0.0),
                PolyVertex::new(Point3::new(0.0, 5.0, 0.0), 0.0),
            ],
            closed: true,
            extrusion: default_extrusion(),
            linetype_generation_continuous: false,
        };
        let offset = offset_geometry(&rect, 1.0, Point2::new(5.0, -1.0)).unwrap();
        let Geometry::LwPolyline {
            vertices, closed, ..
        } = offset
        else {
            panic!("expected a polyline");
        };
        assert!(closed);
        let has = |x, y| vertices.iter().any(|vertex| near(vertex.point.xy(), x, y));
        assert!(has(11.0, -1.0) && has(11.0, 6.0) && has(-1.0, 6.0) && has(-1.0, -1.0));
    }

    #[test]
    fn stretch_moves_only_the_corner_inside_the_window() {
        let rect = Geometry::LwPolyline {
            vertices: vec![
                PolyVertex::new(Point3::new(0.0, 0.0, 0.0), 0.0),
                PolyVertex::new(Point3::new(10.0, 0.0, 0.0), 0.0),
                PolyVertex::new(Point3::new(10.0, 10.0, 0.0), 0.0),
                PolyVertex::new(Point3::new(0.0, 10.0, 0.0), 0.0),
            ],
            closed: true,
            extrusion: default_extrusion(),
            linetype_generation_continuous: false,
        };
        let window = Extents2::from_corners(Point2::new(9.0, -1.0), Point2::new(11.0, 1.0));
        let StretchOutcome::Replaced(Geometry::LwPolyline { vertices, .. }) =
            stretch_geometry(&rect, window, 5.0, 0.0).unwrap()
        else {
            panic!("expected a stretched polyline");
        };
        assert!(near(vertices[0].point.xy(), 0.0, 0.0));
        assert!(near(vertices[1].point.xy(), 15.0, 0.0));
        assert!(near(vertices[2].point.xy(), 10.0, 10.0));
    }
}
