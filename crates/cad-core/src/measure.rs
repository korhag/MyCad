//! Shared geometric measurements used by inspectors and commands.

use crate::entity::PolyVertex;
use crate::geom::Point2;

// ------------------------------------------------------------
// Type: DistanceReport
// Purpose: Two-point distance, deltas, and direction in world XY.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistanceReport {
    pub start: Point2,
    pub end: Point2,
    pub distance: f64,
    pub delta_x: f64,
    pub delta_y: f64,
    pub angle: f64,
}

impl DistanceReport {
    pub fn between(start: Point2, end: Point2) -> Self {
        let delta_x = end.x - start.x;
        let delta_y = end.y - start.y;
        Self {
            start,
            end,
            distance: start.distance(end),
            delta_x,
            delta_y,
            angle: delta_y.atan2(delta_x),
        }
    }
}

pub fn line_length(start: Point2, end: Point2) -> f64 {
    start.distance(end)
}

pub fn polyline_length(vertices: &[PolyVertex], closed: bool) -> f64 {
    if vertices.len() < 2 {
        return 0.0;
    }
    let n = if closed {
        vertices.len()
    } else {
        vertices.len() - 1
    };
    let mut total = 0.0;
    for i in 0..n {
        let a = vertices[i];
        let b = vertices[(i + 1) % vertices.len()];
        total += segment_length(a, b);
    }
    total
}

pub fn segment_length(a: PolyVertex, b: PolyVertex) -> f64 {
    let chord = a.point.xy().distance(b.point.xy());
    if a.bulge.abs() < 1e-12 {
        return chord;
    }
    let theta = 4.0 * a.bulge.atan();
    if theta.abs() < 1e-12 {
        chord
    } else {
        let radius = chord / (2.0 * (theta * 0.5).sin().abs().max(1e-12));
        (radius * theta).abs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Point3;

    fn vertex(x: f64, y: f64, bulge: f64) -> PolyVertex {
        PolyVertex {
            point: Point3::from_xy(x, y),
            bulge,
        }
    }

    #[test]
    fn distance_report_includes_deltas_and_angle() {
        let report = DistanceReport::between(Point2::new(0.0, 0.0), Point2::new(3.0, 4.0));
        assert!((report.distance - 5.0).abs() < 1e-12);
        assert!((report.delta_x - 3.0).abs() < 1e-12);
        assert!((report.delta_y - 4.0).abs() < 1e-12);
        assert!((report.angle - (4.0_f64).atan2(3.0)).abs() < 1e-12);
    }

    #[test]
    fn polyline_length_includes_bulge_arcs() {
        let straight = polyline_length(&[vertex(0.0, 0.0, 0.0), vertex(10.0, 0.0, 0.0)], false);
        assert!((straight - 10.0).abs() < 1e-12);
        let semicircle = polyline_length(&[vertex(0.0, 0.0, 1.0), vertex(2.0, 0.0, 0.0)], false);
        assert!((semicircle - std::f64::consts::PI).abs() < 1e-9);
        let closed = polyline_length(
            &[
                vertex(0.0, 0.0, 0.0),
                vertex(1.0, 0.0, 0.0),
                vertex(1.0, 1.0, 0.0),
                vertex(0.0, 1.0, 0.0),
            ],
            true,
        );
        assert!((closed - 4.0).abs() < 1e-12);
    }
}
