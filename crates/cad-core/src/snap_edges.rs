//! Object snaps that are solved from nearby edges instead of stored points.
//!
//! Center, intersection, tangent, perpendicular, and nearest depend on the
//! cursor (and, for tangent and perpendicular, the command base). They are
//! not stored in [`crate::SnapIndex`].

use crate::geom::Point2;
use crate::intersect::curve_from;
use crate::measure_index::MeasurePrimitive;
use crate::snap::{SnapFeature, SnapKind};

const DEDUP_EPS: f64 = 1e-6;
const MAX_EDGE_CURVES: usize = 64;
/// Intersection tests actually run after the bounding-box reject.
pub const MAX_INTERSECTION_PAIRS: usize = 256;

// ------------------------------------------------------------
// Function: edge_snaps
// Purpose: Append snaps acquired by hovering edges in `primitives`.
//          The cursor must lie within `aperture` of an edge. The
//          resulting point (a center, a tangent, a foot) may be
//          farther away than the aperture.
// ------------------------------------------------------------
pub fn edge_snaps<'a>(
    primitives: impl IntoIterator<Item = &'a MeasurePrimitive>,
    cursor: Point2,
    base: Option<Point2>,
    aperture: f64,
    out: &mut Vec<SnapFeature>,
) {
    out.clear();
    if aperture <= 0.0 || !cursor.is_finite() {
        return;
    }
    let mut curves = Vec::new();
    for primitive in primitives.into_iter().take(MAX_EDGE_CURVES) {
        if let Some(curve) = curve_from(primitive) {
            curves.push(curve);
        }
    }

    for curve in &curves {
        if curve.distance_to(cursor) > aperture {
            continue;
        }
        if let Some(center) = curve.center() {
            emit(out, center, SnapKind::Center);
        }
        if let Some(nearest) = curve.nearest(cursor) {
            if cursor.distance(nearest) <= aperture {
                emit(out, nearest, SnapKind::Nearest);
            }
        }
        if let Some(base) = base {
            for point in curve.perpendiculars(base) {
                emit(out, point, SnapKind::Perpendicular);
            }
            for point in curve.tangents(base) {
                emit(out, point, SnapKind::Tangent);
            }
        }
    }

    let mut hits = Vec::new();
    let mut pairs = 0usize;
    for left in 0..curves.len() {
        for right in (left + 1)..curves.len() {
            if pairs >= MAX_INTERSECTION_PAIRS {
                break;
            }
            if !curves[left].bounds().intersects(curves[right].bounds()) {
                continue;
            }
            pairs += 1;
            curves[left].collect_hits(&curves[right], false, &mut hits);
            for hit in &hits {
                if cursor.distance(hit.point) <= aperture {
                    emit(out, hit.point, SnapKind::Intersection);
                }
            }
        }
        if pairs >= MAX_INTERSECTION_PAIRS {
            break;
        }
    }
}

fn emit(out: &mut Vec<SnapFeature>, point: Point2, kind: SnapKind) {
    if !point.is_finite() {
        return;
    }
    if out
        .iter()
        .any(|feature| feature.kind == kind && feature.point.distance(point) <= DEDUP_EPS)
    {
        return;
    }
    out.push(SnapFeature { point, kind });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extents::Extents2;
    use crate::measure_index::MeasureGeom;
    use crate::transform::Transform2;
    use crate::EntityId;

    fn near(point: Point2, x: f64, y: f64) -> bool {
        point.distance(Point2::new(x, y)) < 1e-5
    }

    fn has(features: &[SnapFeature], kind: SnapKind, x: f64, y: f64) -> bool {
        features
            .iter()
            .any(|feature| feature.kind == kind && near(feature.point, x, y))
    }

    fn segment(start: Point2, end: Point2) -> MeasurePrimitive {
        MeasurePrimitive {
            owner: EntityId::UNASSIGNED,
            nested: Vec::new(),
            geom: MeasureGeom::Straight { start, end },
            bounds: Extents2::from_corners(start, end),
        }
    }

    fn circle(center: Point2, radius: f64, uniform: bool) -> MeasurePrimitive {
        MeasurePrimitive {
            owner: EntityId::UNASSIGNED,
            nested: Vec::new(),
            geom: MeasureGeom::Circle {
                local_center: center,
                local_radius: radius,
                to_world: Transform2::identity(),
                uniform,
            },
            bounds: Extents2::from_corners(
                Point2::new(center.x - radius, center.y - radius),
                Point2::new(center.x + radius, center.y + radius),
            ),
        }
    }

    fn arc(start_angle: f64, end_angle: f64) -> MeasurePrimitive {
        MeasurePrimitive {
            owner: EntityId::UNASSIGNED,
            nested: Vec::new(),
            geom: MeasureGeom::Arc {
                local_center: Point2::new(0.0, 0.0),
                local_radius: 5.0,
                start_angle,
                end_angle,
                to_world: Transform2::identity(),
                uniform: true,
            },
            bounds: Extents2::from_corners(Point2::new(-5.0, -5.0), Point2::new(5.0, 5.0)),
        }
    }

    #[test]
    fn line_line_intersection_is_the_crossing() {
        let primitives = [
            segment(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0)),
            segment(Point2::new(4.0, -5.0), Point2::new(4.0, 5.0)),
        ];
        let mut found = Vec::new();
        edge_snaps(&primitives, Point2::new(4.05, 0.02), None, 0.2, &mut found);
        assert!(has(&found, SnapKind::Intersection, 4.0, 0.0));
    }

    #[test]
    fn line_circle_and_circle_circle_intersections() {
        let line = segment(Point2::new(-2.0, 0.0), Point2::new(2.0, 0.0));
        let first = circle(Point2::new(0.0, 0.0), 1.0, true);
        let mut found = Vec::new();
        edge_snaps(
            &[line, first.clone()],
            Point2::new(1.0, 0.0),
            None,
            0.2,
            &mut found,
        );
        assert!(has(&found, SnapKind::Intersection, 1.0, 0.0));
        assert!(!has(&found, SnapKind::Intersection, -1.0, 0.0));

        let second = circle(Point2::new(6.0, 0.0), 5.0, true);
        let wide = circle(Point2::new(0.0, 0.0), 5.0, true);
        edge_snaps(
            &[wide, second],
            Point2::new(3.0, 4.0),
            None,
            0.3,
            &mut found,
        );
        assert!(has(&found, SnapKind::Intersection, 3.0, 4.0));
    }

    #[test]
    fn arc_intersection_outside_the_sweep_is_rejected() {
        let below = segment(Point2::new(-6.0, -1.0), Point2::new(6.0, -1.0));
        let above = segment(Point2::new(-6.0, 1.0), Point2::new(6.0, 1.0));
        let quarter = arc(0.0, std::f64::consts::FRAC_PI_2);
        let mut found = Vec::new();
        edge_snaps(
            &[quarter.clone(), below],
            Point2::new(4.9, -1.0),
            None,
            0.4,
            &mut found,
        );
        assert!(found
            .iter()
            .all(|feature| feature.kind != SnapKind::Intersection));
        edge_snaps(
            &[quarter, above],
            Point2::new(4.9, 1.0),
            None,
            0.4,
            &mut found,
        );
        assert!(found
            .iter()
            .any(|feature| feature.kind == SnapKind::Intersection && feature.point.y > 0.0));
    }

    #[test]
    fn external_point_has_two_tangent_points() {
        let primitives = [circle(Point2::new(0.0, 0.0), 5.0, true)];
        let mut found = Vec::new();
        edge_snaps(
            &primitives,
            Point2::new(5.0, 0.1),
            Some(Point2::new(10.0, 0.0)),
            0.4,
            &mut found,
        );
        let tangents: Vec<_> = found
            .iter()
            .filter(|feature| feature.kind == SnapKind::Tangent)
            .collect();
        assert_eq!(tangents.len(), 2);
        for feature in tangents {
            let radius = feature.point;
            let toward = feature.point - Point2::new(10.0, 0.0);
            let dot = radius.x * toward.x + radius.y * toward.y;
            assert!(
                dot.abs() < 1e-5,
                "tangent should be perpendicular to the radius"
            );
            assert!((feature.point.distance(Point2::new(0.0, 0.0)) - 5.0).abs() < 1e-5);
        }
    }

    #[test]
    fn perpendicular_foot_lands_on_the_segment() {
        let primitives = [segment(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0))];
        let mut found = Vec::new();
        edge_snaps(
            &primitives,
            Point2::new(5.0, 0.05),
            Some(Point2::new(5.0, 4.0)),
            0.2,
            &mut found,
        );
        assert!(has(&found, SnapKind::Perpendicular, 5.0, 0.0));
    }

    #[test]
    fn hovering_a_concentric_rim_yields_the_shared_center() {
        let primitives = [
            circle(Point2::new(0.0, 0.0), 5.0, true),
            circle(Point2::new(0.0, 0.0), 10.0, true),
        ];
        let mut found = Vec::new();
        edge_snaps(&primitives, Point2::new(10.0, 0.0), None, 0.5, &mut found);
        assert!(has(&found, SnapKind::Center, 0.0, 0.0));
        assert_eq!(
            found
                .iter()
                .filter(|feature| feature.kind == SnapKind::Center)
                .count(),
            1
        );
    }

    #[test]
    fn non_uniform_circle_is_skipped() {
        let primitives = [circle(Point2::new(0.0, 0.0), 10.0, false)];
        let mut found = Vec::new();
        edge_snaps(&primitives, Point2::new(10.0, 0.0), None, 0.5, &mut found);
        assert!(found.is_empty());
    }
}
