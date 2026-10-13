//! Filled bands for wide LWPOLYLINE and POLYLINE entities.
//!
//! Width is measured in the polyline plane. Each segment is a strip from
//! its start width to its end width. Adjacent wide segments meet in a miter
//! so a corner does not leave a square notch. A width of zero stays a
//! centerline and is not returned here.

use crate::curves::{bulge_arc, segments_for_arc, POLYLINE_BULGE_SEGMENTS};
use crate::entity::{PolyVertex, POLYLINE_WIDTH_MIN};
use crate::geom::{ocs_to_wcs, Point2, Point3};
use crate::measure::bulge_circle;

const MITER_LIMIT: f64 = 4.0;

#[derive(Clone, Copy)]
struct Cap {
    left: Point2,
    right: Point2,
}

// ------------------------------------------------------------
// Function: wide_polyline_quads
// Purpose: World-space quads for the wide segments, before an
//          INSERT transform. Each quad is left, right, next right,
//          next left.
// ------------------------------------------------------------
pub fn wide_polyline_quads(
    vertices: &[PolyVertex],
    closed: bool,
    extrusion: Point3,
    chord_tolerance: Option<f64>,
) -> Vec<[Point2; 4]> {
    let count = segment_count(vertices, closed);
    if count == 0 {
        return Vec::new();
    }
    let mut rails: Vec<Option<Vec<Cap>>> = (0..count)
        .map(|index| segment_rail(vertices, index, extrusion, chord_tolerance))
        .collect();
    let joins = count - if closed { 0 } else { 1 };
    for index in 0..joins {
        let next = (index + 1) % count;
        miter_rails(&mut rails, index, next);
    }
    let mut quads = Vec::new();
    for rail in rails.into_iter().flatten() {
        for pair in rail.windows(2) {
            quads.push([pair[0].left, pair[0].right, pair[1].right, pair[1].left]);
        }
    }
    quads
}

fn segment_count(vertices: &[PolyVertex], closed: bool) -> usize {
    if vertices.len() < 2 {
        return 0;
    }
    if closed {
        vertices.len()
    } else {
        vertices.len() - 1
    }
}

fn segment_rail(
    vertices: &[PolyVertex],
    index: usize,
    extrusion: Point3,
    chord_tolerance: Option<f64>,
) -> Option<Vec<Cap>> {
    let start = vertices[index];
    if !start.is_wide() {
        return None;
    }
    let end = vertices[(index + 1) % vertices.len()];
    let from = Point2::new(start.point.x, start.point.y);
    let to = Point2::new(end.point.x, end.point.y);
    if from.distance(to) <= POLYLINE_WIDTH_MIN && start.bulge.abs() <= POLYLINE_WIDTH_MIN {
        return None;
    }
    let samples = bulge_arc(
        from,
        to,
        start.bulge,
        sample_count(from, to, start.bulge, chord_tolerance),
    );
    if samples.len() < 2 {
        return None;
    }
    let last = (samples.len() - 1) as f64;
    let mut caps = Vec::with_capacity(samples.len());
    for (step, point) in samples.iter().enumerate() {
        let t = step as f64 / last;
        let half = (start.start_width + (start.end_width - start.start_width) * t) * 0.5;
        let ahead = if step + 1 < samples.len() {
            samples[step + 1] - *point
        } else {
            *point - samples[step - 1]
        };
        let length = (ahead.x * ahead.x + ahead.y * ahead.y).sqrt();
        if length <= POLYLINE_WIDTH_MIN {
            continue;
        }
        let normal = Point2::new(-ahead.y / length, ahead.x / length);
        let left = *point + normal * half;
        let right = *point - normal * half;
        caps.push(Cap {
            left: to_wcs(left, start.point.z, extrusion),
            right: to_wcs(right, start.point.z, extrusion),
        });
    }
    (caps.len() >= 2).then_some(caps)
}

fn sample_count(from: Point2, to: Point2, bulge: f64, chord_tolerance: Option<f64>) -> usize {
    let Some(tolerance) = chord_tolerance.filter(|value| value.is_finite() && *value > 0.0) else {
        return POLYLINE_BULGE_SEGMENTS;
    };
    let Some(arc) = bulge_circle(from, to, bulge) else {
        return 1;
    };
    segments_for_arc(arc.radius, arc.sweep, tolerance)
}

fn to_wcs(point: Point2, elevation: f64, extrusion: Point3) -> Point2 {
    ocs_to_wcs(Point3::new(point.x, point.y, elevation), extrusion).xy()
}

fn miter_rails(rails: &mut [Option<Vec<Cap>>], left_index: usize, right_index: usize) {
    let (end, before, start, after) = {
        let (Some(incoming), Some(outgoing)) =
            (rails[left_index].as_ref(), rails[right_index].as_ref())
        else {
            return;
        };
        if incoming.len() < 2 || outgoing.len() < 2 {
            return;
        }
        (
            incoming[incoming.len() - 1],
            incoming[incoming.len() - 2],
            outgoing[0],
            outgoing[1],
        )
    };
    let center = Point2::new(
        (end.left.x + end.right.x) * 0.5,
        (end.left.y + end.right.y) * 0.5,
    );
    let half = center.distance(end.left).max(center.distance(start.left));
    let limit = (half * MITER_LIMIT).max(POLYLINE_WIDTH_MIN);
    let Some(left) = limited_hit(before.left, end.left, start.left, after.left, center, limit)
    else {
        return;
    };
    let Some(right) = limited_hit(
        before.right,
        end.right,
        start.right,
        after.right,
        center,
        limit,
    ) else {
        return;
    };
    let joined = Cap { left, right };
    if let Some(incoming) = rails[left_index].as_mut() {
        if let Some(last) = incoming.last_mut() {
            *last = joined;
        }
    }
    if let Some(outgoing) = rails[right_index].as_mut() {
        if let Some(first) = outgoing.first_mut() {
            *first = joined;
        }
    }
}

fn limited_hit(
    a0: Point2,
    a1: Point2,
    b0: Point2,
    b1: Point2,
    center: Point2,
    limit: f64,
) -> Option<Point2> {
    let hit = line_hit(a0, a1, b0, b1)?;
    (hit.distance(center) <= limit).then_some(hit)
}

fn line_hit(a0: Point2, a1: Point2, b0: Point2, b1: Point2) -> Option<Point2> {
    let d1 = a1 - a0;
    let d2 = b1 - b0;
    let cross = d1.x * d2.y - d1.y * d2.x;
    if cross.abs() <= 1e-12 {
        return None;
    }
    let diff = b0 - a0;
    let t = (diff.x * d2.y - diff.y * d2.x) / cross;
    let hit = a0 + d1 * t;
    hit.is_finite().then_some(hit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Point3;

    fn vertex(x: f64, y: f64, width: f64) -> PolyVertex {
        PolyVertex {
            start_width: width,
            end_width: width,
            ..PolyVertex::new(Point3::new(x, y, 0.0), 0.0)
        }
    }

    fn near(points: impl IntoIterator<Item = Point2>, expected: Point2) -> bool {
        points
            .into_iter()
            .any(|point| point.distance(expected) < 1e-4)
    }

    #[test]
    fn straight_width_is_a_centered_band() {
        let quads = wide_polyline_quads(
            &[vertex(0.0, 0.0, 2.0), vertex(10.0, 0.0, 0.0)],
            false,
            Point3::new(0.0, 0.0, 1.0),
            None,
        );
        assert_eq!(quads.len(), 1);
        let quad = quads[0];
        assert!(near(quad, Point2::new(0.0, 1.0)));
        assert!(near(quad, Point2::new(0.0, -1.0)));
        assert!(near(quad, Point2::new(10.0, 1.0)));
        assert!(near(quad, Point2::new(10.0, -1.0)));
    }

    #[test]
    fn corner_miter_fills_the_outside() {
        let quads = wide_polyline_quads(
            &[
                vertex(0.0, 0.0, 2.0),
                vertex(10.0, 0.0, 2.0),
                vertex(10.0, 10.0, 0.0),
            ],
            false,
            Point3::new(0.0, 0.0, 1.0),
            None,
        );
        let points: Vec<Point2> = quads.into_iter().flatten().collect();
        assert!(near(points.iter().copied(), Point2::new(11.0, -1.0)));
        assert!(near(points, Point2::new(9.0, 1.0)));
    }

    #[test]
    fn zero_width_has_no_band() {
        let quads = wide_polyline_quads(
            &[vertex(0.0, 0.0, 0.0), vertex(5.0, 0.0, 0.0)],
            false,
            Point3::new(0.0, 0.0, 1.0),
            None,
        );
        assert!(quads.is_empty());
    }
}
