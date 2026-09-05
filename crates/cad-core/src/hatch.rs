//! Canonical HATCH boundary sampling in world XY.
//!
//! DXF hatch path data is defined in the hatch's OCS. Samples are
//! constructed in that plane using the entity extrusion and elevation,
//! then mapped to WCS so viewport and PDF cannot disagree.

use crate::curves::{
    arc_points, bspline_points, ellipse_arc_points, polyline_points, CIRCLE_SEGMENTS,
};
use crate::entity::{default_extrusion, HatchEdge, HatchPath, PolyVertex};
use crate::geom::{ocs_to_wcs, Point2, Point3};

fn map_ocs(point: Point2, elevation: f64, extrusion: Point3) -> Point2 {
    ocs_to_wcs(Point3::new(point.x, point.y, elevation), extrusion).xy()
}

fn ocs_point(point: Point3, elevation: f64) -> Point3 {
    Point3::new(point.x, point.y, elevation)
}

fn append_edge(pts: &mut Vec<Point2>, mut samples: Vec<Point2>) {
    if !pts.is_empty() && !samples.is_empty() {
        samples.remove(0);
    }
    pts.extend(samples);
}

// ------------------------------------------------------------
// Function: hatch_path_points
// Purpose: Sample one hatch boundary into WCS polyline points using
//          the hatch extrusion and elevation. Ellipse group 11 is
//          the major-axis vector relative to the center (DXF).
// ------------------------------------------------------------
pub fn hatch_path_points(path: &HatchPath, extrusion: Point3, elevation: f64) -> Vec<Point2> {
    match path {
        HatchPath::Polyline { vertices, closed } => {
            let verts: Vec<PolyVertex> = vertices
                .iter()
                .map(|vertex| PolyVertex {
                    point: ocs_point(vertex.point, elevation),
                    bulge: vertex.bulge,
                })
                .collect();
            polyline_points(&verts, *closed, extrusion)
        }
        HatchPath::Edges(edges) => {
            let mut pts = Vec::new();
            for edge in edges {
                match edge {
                    HatchEdge::Line { start, end } => {
                        let a = map_ocs(start.xy(), elevation, extrusion);
                        let b = map_ocs(end.xy(), elevation, extrusion);
                        if pts
                            .last()
                            .map(|p: &Point2| p.distance(a) > 1e-9)
                            .unwrap_or(true)
                        {
                            pts.push(a);
                        }
                        pts.push(b);
                    }
                    HatchEdge::Arc {
                        center,
                        radius,
                        start_angle,
                        end_angle,
                        is_ccw,
                    } => {
                        let samples = arc_points(
                            ocs_point(*center, elevation),
                            *radius,
                            *start_angle,
                            *end_angle,
                            *is_ccw,
                            extrusion,
                            CIRCLE_SEGMENTS,
                        );
                        append_edge(&mut pts, samples);
                    }
                    HatchEdge::Ellipse {
                        center,
                        major_endpoint,
                        axis_ratio,
                        start_angle,
                        end_angle,
                        is_ccw,
                    } => {
                        // Group 11 is the major-axis vector in OCS, not a WCS point.
                        let ocs = ellipse_arc_points(
                            Point3::from_xy(center.x, center.y),
                            Point3::from_xy(major_endpoint.x, major_endpoint.y),
                            *axis_ratio,
                            *start_angle,
                            *end_angle,
                            *is_ccw,
                            default_extrusion(),
                            CIRCLE_SEGMENTS,
                        );
                        let samples: Vec<Point2> = ocs
                            .into_iter()
                            .map(|p| map_ocs(p, elevation, extrusion))
                            .collect();
                        append_edge(&mut pts, samples);
                    }
                    HatchEdge::Spline { control_points } => {
                        let ocs: Vec<Point3> = control_points
                            .iter()
                            .map(|p| Point3::from_xy(p.x, p.y))
                            .collect();
                        let samples: Vec<Point2> = bspline_points(3, &ocs, &[], &[], 24)
                            .into_iter()
                            .map(|p| map_ocs(p, elevation, extrusion))
                            .collect();
                        append_edge(&mut pts, samples);
                    }
                }
            }
            pts
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::HatchData;

    const EPS: f64 = 1e-8;

    fn assert_point(got: Point2, expected: Point2, label: &str) {
        assert!(
            (got.x - expected.x).abs() < EPS && (got.y - expected.y).abs() < EPS,
            "{label}: got ({}, {}), expected ({}, {})",
            got.x,
            got.y,
            expected.x,
            expected.y
        );
    }

    fn sample_mid(pts: &[Point2]) -> Point2 {
        pts[pts.len() / 2]
    }

    fn chord_side(start: Point2, end: Point2, mid: Point2) -> f64 {
        (end.x - start.x) * (mid.y - start.y) - (end.y - start.y) * (mid.x - start.x)
    }

    fn world_z() -> Point3 {
        Point3::new(0.0, 0.0, 1.0)
    }

    fn neg_z() -> Point3 {
        Point3::new(0.0, 0.0, -1.0)
    }

    #[test]
    fn hatch_ccw_circular_arc_keeps_start_end_and_short_side() {
        let path = HatchPath::Edges(vec![HatchEdge::Arc {
            center: Point3::from_xy(0.0, 0.0),
            radius: 1.0,
            start_angle: 0.0,
            end_angle: std::f64::consts::FRAC_PI_2,
            is_ccw: true,
        }]);
        let pts = hatch_path_points(&path, world_z(), 0.0);
        assert_point(pts[0], Point2::new(1.0, 0.0), "start");
        assert_point(*pts.last().unwrap(), Point2::new(0.0, 1.0), "end");
        let mid = sample_mid(&pts);
        assert_point(
            mid,
            Point2::new(std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2),
            "midpoint",
        );
        assert!(
            chord_side(pts[0], *pts.last().unwrap(), mid) > 0.0,
            "CCW quarter stays on the +XY side of the chord"
        );
    }

    #[test]
    fn hatch_cw_circular_arc_keeps_start_end_and_long_side() {
        let path = HatchPath::Edges(vec![HatchEdge::Arc {
            center: Point3::from_xy(0.0, 0.0),
            radius: 1.0,
            start_angle: 0.0,
            end_angle: std::f64::consts::FRAC_PI_2,
            is_ccw: false,
        }]);
        let pts = hatch_path_points(&path, world_z(), 0.0);
        assert_point(pts[0], Point2::new(1.0, 0.0), "start");
        assert_point(*pts.last().unwrap(), Point2::new(0.0, 1.0), "end");
        let mid = sample_mid(&pts);
        assert_point(mid, Point2::new(-1.0, 0.0), "midpoint");
        assert!(
            chord_side(pts[0], *pts.last().unwrap(), mid) < 0.0,
            "CW three-quarter arc stays on the opposite side of the chord"
        );
    }

    #[test]
    fn hatch_ccw_elliptic_arc_uses_relative_major_axis() {
        let path = HatchPath::Edges(vec![HatchEdge::Ellipse {
            center: Point3::from_xy(0.0, 0.0),
            major_endpoint: Point3::from_xy(2.0, 0.0),
            axis_ratio: 0.5,
            start_angle: 0.0,
            end_angle: std::f64::consts::FRAC_PI_2,
            is_ccw: true,
        }]);
        let pts = hatch_path_points(&path, world_z(), 0.0);
        assert_point(pts[0], Point2::new(2.0, 0.0), "start");
        assert_point(*pts.last().unwrap(), Point2::new(0.0, 1.0), "end");
        let mid = sample_mid(&pts);
        assert_point(
            mid,
            Point2::new(std::f64::consts::SQRT_2, 0.5 * std::f64::consts::FRAC_1_SQRT_2 * 2.0 / 2.0 + 0.5 * std::f64::consts::FRAC_1_SQRT_2),
            "placeholder",
        );
        let expected = Point2::new(
            2.0 * std::f64::consts::FRAC_1_SQRT_2,
            0.5 * std::f64::consts::FRAC_1_SQRT_2,
        );
        assert_point(mid, expected, "ellipse midpoint");
        assert!(
            chord_side(pts[0], *pts.last().unwrap(), mid) > 0.0,
            "CCW elliptic quarter stays on the +XY side of the chord"
        );
    }

    #[test]
    fn hatch_cw_elliptic_arc_keeps_start_end_and_opposite_side() {
        let path = HatchPath::Edges(vec![HatchEdge::Ellipse {
            center: Point3::from_xy(0.0, 0.0),
            major_endpoint: Point3::from_xy(2.0, 0.0),
            axis_ratio: 0.5,
            start_angle: 0.0,
            end_angle: std::f64::consts::FRAC_PI_2,
            is_ccw: false,
        }]);
        let pts = hatch_path_points(&path, world_z(), 0.0);
        assert_point(pts[0], Point2::new(2.0, 0.0), "start");
        assert_point(*pts.last().unwrap(), Point2::new(0.0, 1.0), "end");
        let mid = sample_mid(&pts);
        assert_point(mid, Point2::new(-2.0, 0.0), "midpoint");
        assert!(
            chord_side(pts[0], *pts.last().unwrap(), mid) < 0.0,
            "CW elliptic three-quarter stays on the opposite side of the chord"
        );
    }

    #[test]
    fn hatch_polyline_uses_hatch_extrusion_not_world_z() {
        let path = HatchPath::Polyline {
            vertices: vec![
                PolyVertex {
                    point: Point3::from_xy(0.0, 0.0),
                    bulge: 1.0,
                },
                PolyVertex {
                    point: Point3::from_xy(2.0, 0.0),
                    bulge: 0.0,
                },
            ],
            closed: false,
        };
        let pts = hatch_path_points(&path, neg_z(), 0.0);
        assert_point(pts[0], Point2::new(0.0, 0.0), "start");
        assert_point(*pts.last().unwrap(), Point2::new(-2.0, 0.0), "end");
        let mid = sample_mid(&pts);
        assert_point(mid, Point2::new(-1.0, -1.0), "midpoint");
        assert!(mid.y < 0.0);
    }

    #[test]
    fn hatch_line_edge_maps_through_negative_z() {
        let path = HatchPath::Edges(vec![HatchEdge::Line {
            start: Point3::from_xy(1.0, 2.0),
            end: Point3::from_xy(3.0, 2.0),
        }]);
        let pts = hatch_path_points(&path, neg_z(), 4.0);
        assert_point(pts[0], Point2::new(-1.0, 2.0), "start");
        assert_point(pts[1], Point2::new(-3.0, 2.0), "end");
    }

    #[test]
    fn hatch_data_paths_share_extrusion() {
        let hatch = HatchData {
            extrusion: neg_z(),
            elevation: 0.0,
            solid_fill: true,
            paths: vec![HatchPath::Edges(vec![HatchEdge::Arc {
                center: Point3::from_xy(0.0, 0.0),
                radius: 1.0,
                start_angle: 0.0,
                end_angle: std::f64::consts::PI,
                is_ccw: true,
            }])],
            pattern_lines: Vec::new(),
        };
        let pts = hatch_path_points(&hatch.paths[0], hatch.extrusion, hatch.elevation);
        assert_point(pts[0], Point2::new(-1.0, 0.0), "start mirrored");
        assert_point(*pts.last().unwrap(), Point2::new(1.0, 0.0), "end mirrored");
        let mid = sample_mid(&pts);
        assert!(mid.y > 0.0, "CCW semicircle Y is preserved under neg-Z");
    }
}
