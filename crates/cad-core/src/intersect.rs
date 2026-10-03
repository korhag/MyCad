//! Shared planar curve intersections for snaps and edit tools.
//!
//! Snaps use bounded hits (both curves must contain the point). Extend
//! uses the same solvers with the edited curve left unbounded, and keeps
//! the boundary curve finite.

use crate::geom::{Point2, GEOM_TOLERANCE};
use crate::measure::{
    angle_on_arc, bulge_circle, infinite_line_intersection, point_on_circle, point_segment_distance,
};
use crate::measure_index::{MeasureGeom, MeasurePrimitive};
use crate::transform::Transform2;

pub(crate) const PARAM_EPS: f64 = 1e-8;

// ------------------------------------------------------------
// Type: CurveHit
// Purpose: One intersection plus the parameter on the curve that
//          was asked. A segment parameter is 0 at the start and 1
//          at the end. An arc or circle parameter is the world angle.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy)]
pub(crate) struct CurveHit {
    pub point: Point2,
    pub param: f64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ArcLimit {
    Full,
    Sweep {
        start: f64,
        sweep: f64,
    },
    Local {
        to_world: Transform2,
        local_center: Point2,
        start: f64,
        end: f64,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct WorldArc {
    pub center: Point2,
    pub radius: f64,
    pub endpoints: Option<(Point2, Point2)>,
    pub limit: ArcLimit,
}

pub(crate) enum Curve {
    Segment { start: Point2, end: Point2 },
    Arc(WorldArc),
}

impl Curve {
    pub(crate) fn segment(start: Point2, end: Point2) -> Option<Self> {
        if start.distance(end) <= GEOM_TOLERANCE {
            return None;
        }
        Some(Self::Segment { start, end })
    }

    pub(crate) fn circle(center: Point2, radius: f64) -> Option<Self> {
        if radius <= GEOM_TOLERANCE {
            return None;
        }
        Some(Self::Arc(WorldArc {
            center,
            radius,
            endpoints: None,
            limit: ArcLimit::Full,
        }))
    }

    pub(crate) fn arc(
        center: Point2,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    ) -> Option<Self> {
        if radius <= GEOM_TOLERANCE {
            return None;
        }
        let start = point_at_angle(center, radius, start_angle);
        let end = point_at_angle(center, radius, end_angle);
        Some(Self::Arc(WorldArc {
            center,
            radius,
            endpoints: Some((start, end)),
            limit: ArcLimit::Local {
                to_world: Transform2::identity(),
                local_center: center,
                start: start_angle,
                end: end_angle,
            },
        }))
    }

    pub(crate) fn bulge(start: Point2, end: Point2, bulge: f64) -> Option<Self> {
        let arc = bulge_circle(start, end, bulge)?;
        if arc.radius <= GEOM_TOLERANCE {
            return None;
        }
        Some(Self::Arc(WorldArc {
            center: arc.center,
            radius: arc.radius,
            endpoints: Some((start, end)),
            limit: ArcLimit::Sweep {
                start: arc.start_angle,
                sweep: arc.sweep,
            },
        }))
    }

    pub(crate) fn distance_to(&self, point: Point2) -> f64 {
        match self {
            Self::Segment { start, end } => point_segment_distance(point, *start, *end),
            Self::Arc(arc) => point.distance(closest_on_arc(point, arc)),
        }
    }

    pub(crate) fn center(&self) -> Option<Point2> {
        match self {
            Self::Arc(arc) => Some(arc.center),
            Self::Segment { .. } => None,
        }
    }

    pub(crate) fn nearest(&self, cursor: Point2) -> Option<Point2> {
        match self {
            Self::Segment { start, end } => Some(closest_on_segment(cursor, *start, *end)),
            Self::Arc(arc) => Some(closest_on_arc(cursor, arc)),
        }
    }

    /// Unclamped position of `point` on this curve. Segment values may
    /// fall outside 0..=1. Arc values are the radial angle.
    pub(crate) fn project_param(&self, point: Point2) -> Option<f64> {
        match self {
            Self::Segment { start, end } => segment_param(*start, *end, point),
            Self::Arc(arc) => {
                if point.distance(arc.center) <= GEOM_TOLERANCE {
                    return None;
                }
                Some((point.y - arc.center.y).atan2(point.x - arc.center.x))
            }
        }
    }

    pub(crate) fn point_at(&self, param: f64) -> Point2 {
        match self {
            Self::Segment { start, end } => *start + (*end - *start) * param,
            Self::Arc(arc) => point_at_angle(arc.center, arc.radius, param),
        }
    }

    pub(crate) fn world_arc(&self) -> Option<&WorldArc> {
        match self {
            Self::Arc(arc) => Some(arc),
            Self::Segment { .. } => None,
        }
    }

    /// Signed travel from the arc start to `end_angle` along the stored sweep.
    /// Full circles report a positive full turn. `None` for segments.
    pub(crate) fn signed_sweep(&self) -> Option<f64> {
        let arc = self.world_arc()?;
        Some(match arc.limit {
            ArcLimit::Full => std::f64::consts::TAU,
            ArcLimit::Sweep { sweep, .. } => sweep,
            ArcLimit::Local { start, end, .. } => ccw_sweep(start, end),
        })
    }

    pub(crate) fn start_angle(&self) -> Option<f64> {
        let arc = self.world_arc()?;
        Some(match arc.limit {
            ArcLimit::Full => 0.0,
            ArcLimit::Sweep { start, .. } => start,
            ArcLimit::Local { start, .. } => start,
        })
    }

    pub(crate) fn perpendiculars(&self, base: Point2) -> Vec<Point2> {
        match self {
            Self::Segment { start, end } => {
                foot_on_segment(base, *start, *end).into_iter().collect()
            }
            Self::Arc(arc) => perpendiculars_on_arc(base, arc),
        }
    }

    pub(crate) fn tangents(&self, base: Point2) -> Vec<Point2> {
        match self {
            Self::Segment { .. } => Vec::new(),
            Self::Arc(arc) => tangent_points(arc.center, arc.radius, base)
                .into_iter()
                .filter(|point| arc.contains(*point))
                .collect(),
        }
    }

    pub(crate) fn intersections(&self, other: &Self) -> Vec<Point2> {
        self.hits(other, false)
            .into_iter()
            .map(|hit| hit.point)
            .collect()
    }

    /// Intersections with `other`. When `extend_self` is set, this curve
    /// may be hit past its ends; `other` stays finite.
    pub(crate) fn hits(&self, other: &Self, extend_self: bool) -> Vec<CurveHit> {
        match (self, other) {
            (Self::Segment { start: a0, end: a1 }, Self::Segment { start: b0, end: b1 }) => {
                segment_segment_hits(*a0, *a1, *b0, *b1, extend_self)
            }
            (Self::Segment { start, end }, Self::Arc(arc)) => {
                segment_arc_hits(*start, *end, arc, extend_self, true)
            }
            (Self::Arc(arc), Self::Segment { start, end }) => {
                segment_arc_hits(*start, *end, arc, false, !extend_self)
                    .into_iter()
                    .map(|hit| CurveHit {
                        point: hit.point,
                        param: arc_param(arc, hit.point),
                    })
                    .collect()
            }
            (Self::Arc(left), Self::Arc(right)) => {
                circle_circle(left.center, left.radius, right.center, right.radius)
                    .into_iter()
                    .filter(|point| {
                        (extend_self || left.contains(*point)) && right.contains(*point)
                    })
                    .map(|point| CurveHit {
                        point,
                        param: arc_param(left, point),
                    })
                    .collect()
            }
        }
    }
}

impl WorldArc {
    pub(crate) fn contains(&self, point: Point2) -> bool {
        match self.limit {
            ArcLimit::Full => true,
            ArcLimit::Sweep { start, sweep } => {
                angle_within_sweep(point, self.center, start, sweep)
            }
            ArcLimit::Local {
                to_world,
                local_center,
                start,
                end,
            } => {
                let Some(inverse) = to_world.try_inverse() else {
                    return false;
                };
                angle_on_arc(inverse.apply(point), local_center, start, end)
            }
        }
    }
}

pub(crate) fn curve_from(primitive: &MeasurePrimitive) -> Option<Curve> {
    match &primitive.geom {
        MeasureGeom::Straight { start, end } => Curve::segment(*start, *end),
        MeasureGeom::Circle {
            local_center,
            local_radius,
            to_world,
            uniform,
        } => {
            if !*uniform {
                return None;
            }
            world_circle(*local_center, *local_radius, *to_world, ArcLimit::Full)
        }
        MeasureGeom::Arc {
            local_center,
            local_radius,
            start_angle,
            end_angle,
            to_world,
            uniform,
        } => {
            if !*uniform {
                return None;
            }
            let start = world_local_point(*local_center, *local_radius, *start_angle, *to_world);
            let end = world_local_point(*local_center, *local_radius, *end_angle, *to_world);
            world_circle(
                *local_center,
                *local_radius,
                *to_world,
                ArcLimit::Local {
                    to_world: *to_world,
                    local_center: *local_center,
                    start: *start_angle,
                    end: *end_angle,
                },
            )
            .map(|curve| {
                let Curve::Arc(mut arc) = curve else {
                    return curve;
                };
                arc.endpoints = Some((start, end));
                Curve::Arc(arc)
            })
        }
        MeasureGeom::Bulge { start, end, bulge } => Curve::bulge(*start, *end, *bulge),
        MeasureGeom::ClosedLoop { .. } => None,
    }
}

fn world_circle(
    local_center: Point2,
    local_radius: f64,
    to_world: Transform2,
    limit: ArcLimit,
) -> Option<Curve> {
    if !to_world.is_uniform_scale() {
        return None;
    }
    let radius = local_radius * to_world.scale_x();
    if radius <= GEOM_TOLERANCE {
        return None;
    }
    Some(Curve::Arc(WorldArc {
        center: to_world.apply(local_center),
        radius,
        endpoints: None,
        limit,
    }))
}

fn world_local_point(center: Point2, radius: f64, angle: f64, to_world: Transform2) -> Point2 {
    to_world.apply(Point2::new(
        center.x + radius * angle.cos(),
        center.y + radius * angle.sin(),
    ))
}

fn closest_on_segment(point: Point2, start: Point2, end: Point2) -> Point2 {
    let delta = end - start;
    let len2 = delta.x * delta.x + delta.y * delta.y;
    if len2 <= GEOM_TOLERANCE {
        return start;
    }
    let t = ((point.x - start.x) * delta.x + (point.y - start.y) * delta.y) / len2;
    start + delta * t.clamp(0.0, 1.0)
}

fn closest_on_arc(point: Point2, arc: &WorldArc) -> Point2 {
    let radial = point_on_circle(arc.center, arc.radius, point);
    if arc.contains(radial) {
        return radial;
    }
    let Some((start, end)) = arc.endpoints else {
        return radial;
    };
    if point.distance(start) <= point.distance(end) {
        start
    } else {
        end
    }
}

fn foot_on_segment(base: Point2, start: Point2, end: Point2) -> Option<Point2> {
    let delta = end - start;
    let len2 = delta.x * delta.x + delta.y * delta.y;
    if len2 <= GEOM_TOLERANCE {
        return None;
    }
    let t = ((base.x - start.x) * delta.x + (base.y - start.y) * delta.y) / len2;
    if !(-PARAM_EPS..=1.0 + PARAM_EPS).contains(&t) {
        return None;
    }
    Some(start + delta * t.clamp(0.0, 1.0))
}

fn perpendiculars_on_arc(base: Point2, arc: &WorldArc) -> Vec<Point2> {
    if base.distance(arc.center) <= GEOM_TOLERANCE {
        return Vec::new();
    }
    let near = point_on_circle(arc.center, arc.radius, base);
    let far = Point2::new(
        arc.center.x - (near.x - arc.center.x),
        arc.center.y - (near.y - arc.center.y),
    );
    [near, far]
        .into_iter()
        .filter(|point| arc.contains(*point))
        .collect()
}

fn tangent_points(center: Point2, radius: f64, from: Point2) -> Vec<Point2> {
    let toward = center - from;
    let dist2 = toward.x * toward.x + toward.y * toward.y;
    let radius2 = radius * radius;
    if dist2 <= radius2 + GEOM_TOLERANCE {
        return Vec::new();
    }
    let a = radius2 / dist2;
    let b = radius * (dist2 - radius2).sqrt() / dist2;
    let perp = Point2::new(-toward.y, toward.x);
    let mid = center - toward * a;
    vec![mid + perp * b, mid - perp * b]
}

fn segment_segment_hits(
    a0: Point2,
    a1: Point2,
    b0: Point2,
    b1: Point2,
    extend_self: bool,
) -> Vec<CurveHit> {
    if extend_self {
        let Some(point) = infinite_line_intersection(a0, a1, b0, b1) else {
            return Vec::new();
        };
        let Some(param) = segment_param(a0, a1, point) else {
            return Vec::new();
        };
        let Some(other) = segment_param(b0, b1, point) else {
            return Vec::new();
        };
        if !(-PARAM_EPS..=1.0 + PARAM_EPS).contains(&other) {
            return Vec::new();
        }
        return vec![CurveHit { point, param }];
    }
    let da = a1 - a0;
    let db = b1 - b0;
    let denom = da.x * db.y - da.y * db.x;
    let scale = da.x.abs() + da.y.abs() + db.x.abs() + db.y.abs();
    if denom.abs() <= 1e-12 * scale.max(1.0) {
        return Vec::new();
    }
    let t = ((b0.x - a0.x) * db.y - (b0.y - a0.y) * db.x) / denom;
    let u = ((b0.x - a0.x) * da.y - (b0.y - a0.y) * da.x) / denom;
    if !(-PARAM_EPS..=1.0 + PARAM_EPS).contains(&t) || !(-PARAM_EPS..=1.0 + PARAM_EPS).contains(&u)
    {
        return Vec::new();
    }
    let param = t.clamp(0.0, 1.0);
    vec![CurveHit {
        point: a0 + da * param,
        param,
    }]
}

fn segment_arc_hits(
    start: Point2,
    end: Point2,
    arc: &WorldArc,
    extend_line: bool,
    require_arc: bool,
) -> Vec<CurveHit> {
    let delta = end - start;
    let from = start - arc.center;
    let a = delta.x * delta.x + delta.y * delta.y;
    if a <= GEOM_TOLERANCE {
        return Vec::new();
    }
    let b = 2.0 * (from.x * delta.x + from.y * delta.y);
    let c = from.x * from.x + from.y * from.y - arc.radius * arc.radius;
    let disc = b * b - 4.0 * a * c;
    if disc < -1e-8 {
        return Vec::new();
    }
    let root = disc.max(0.0).sqrt();
    let mut hits = Vec::new();
    for sign in [-1.0, 1.0] {
        let t = (-b + sign * root) / (2.0 * a);
        if !extend_line && !(-PARAM_EPS..=1.0 + PARAM_EPS).contains(&t) {
            continue;
        }
        if !t.is_finite() {
            continue;
        }
        let param = if extend_line { t } else { t.clamp(0.0, 1.0) };
        let point = start + delta * param;
        if require_arc && !arc.contains(point) {
            continue;
        }
        hits.push(CurveHit { point, param });
    }
    hits
}

fn circle_circle(c1: Point2, r1: f64, c2: Point2, r2: f64) -> Vec<Point2> {
    let delta = c2 - c1;
    let distance = (delta.x * delta.x + delta.y * delta.y).sqrt();
    if distance <= GEOM_TOLERANCE || distance > r1 + r2 + 1e-6 || distance < (r1 - r2).abs() - 1e-6
    {
        return Vec::new();
    }
    let a = (r1 * r1 - r2 * r2 + distance * distance) / (2.0 * distance);
    let height2 = r1 * r1 - a * a;
    if height2 < -1e-8 {
        return Vec::new();
    }
    let height = height2.max(0.0).sqrt();
    let ux = delta.x / distance;
    let uy = delta.y / distance;
    let mid = Point2::new(c1.x + a * ux, c1.y + a * uy);
    if height <= 1e-8 {
        return vec![mid];
    }
    vec![
        Point2::new(mid.x - uy * height, mid.y + ux * height),
        Point2::new(mid.x + uy * height, mid.y - ux * height),
    ]
}

fn angle_within_sweep(point: Point2, center: Point2, start: f64, sweep: f64) -> bool {
    let angle = (point.y - center.y).atan2(point.x - center.x);
    let tau = std::f64::consts::TAU;
    if sweep.abs() >= tau - 1e-6 {
        return true;
    }
    let delta = if sweep >= 0.0 {
        (angle - start).rem_euclid(tau)
    } else {
        (start - angle).rem_euclid(tau)
    };
    delta <= sweep.abs() + 1e-6
}

fn segment_param(start: Point2, end: Point2, point: Point2) -> Option<f64> {
    let delta = end - start;
    let len2 = delta.x * delta.x + delta.y * delta.y;
    if len2 <= GEOM_TOLERANCE {
        return None;
    }
    let t = ((point.x - start.x) * delta.x + (point.y - start.y) * delta.y) / len2;
    t.is_finite().then_some(t)
}

fn arc_param(arc: &WorldArc, point: Point2) -> f64 {
    (point.y - arc.center.y).atan2(point.x - arc.center.x)
}

fn point_at_angle(center: Point2, radius: f64, angle: f64) -> Point2 {
    Point2::new(
        center.x + radius * angle.cos(),
        center.y + radius * angle.sin(),
    )
}

fn ccw_sweep(start: f64, end: f64) -> f64 {
    (end - start).rem_euclid(std::f64::consts::TAU)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbounded_line_hits_past_its_end() {
        let line = Curve::segment(Point2::new(0.0, 0.0), Point2::new(1.0, 0.0)).unwrap();
        let fence = Curve::segment(Point2::new(4.0, -1.0), Point2::new(4.0, 1.0)).unwrap();
        assert!(line.hits(&fence, false).is_empty());
        let hits = line.hits(&fence, true);
        assert_eq!(hits.len(), 1);
        assert!((hits[0].param - 4.0).abs() < 1e-6);
        assert!(hits[0].point.distance(Point2::new(4.0, 0.0)) < 1e-6);
    }
}
