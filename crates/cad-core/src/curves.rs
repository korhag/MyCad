//! Curve sampling shared by viewport tessellation and vector PDF export.

use crate::geom::{ocs_to_wcs, Point2, Point3};
use crate::entity::PolyVertex;
use crate::measure::bulge_circle;

pub const CIRCLE_SEGMENTS: usize = 32;

pub fn circle_points(
    center: Point3,
    radius: f64,
    extrusion: Point3,
    segments: usize,
) -> Vec<Point2> {
    arc_points(
        center,
        radius,
        0.0,
        std::f64::consts::TAU,
        true,
        extrusion,
        segments,
    )
}

pub fn arc_points(
    center: Point3,
    radius: f64,
    start: f64,
    end: f64,
    ccw: bool,
    extrusion: Point3,
    segments: usize,
) -> Vec<Point2> {
    if radius.abs() < 1e-15 || !center.is_finite() {
        return vec![ocs_to_wcs(center, extrusion).xy()];
    }
    let mut sweep = if ccw { end - start } else { start - end };
    if sweep.abs() < 1e-15 {
        sweep = std::f64::consts::TAU;
    }
    while sweep <= 0.0 {
        sweep += std::f64::consts::TAU;
    }
    while sweep > std::f64::consts::TAU + 1e-12 {
        sweep -= std::f64::consts::TAU;
    }
    let n = ((segments as f64) * (sweep / std::f64::consts::TAU))
        .ceil()
        .max(2.0) as usize;
    let mut pts = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let t = i as f64 / n as f64;
        let angle = if ccw {
            start + sweep * t
        } else {
            start - sweep * t
        };
        let local = Point3::new(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
            center.z,
        );
        pts.push(ocs_to_wcs(local, extrusion).xy());
    }
    pts
}

pub fn ellipse_points(
    center: Point3,
    major_axis: Point3,
    axis_ratio: f64,
    start_param: f64,
    end_param: f64,
    extrusion: Point3,
    segments: usize,
) -> Vec<Point2> {
    let major_len = major_axis.length().max(1e-15);
    let major_dir = major_axis.normalized();
    let minor_dir = extrusion.normalized().cross(major_dir).normalized();
    let minor_len = major_len * axis_ratio.abs().max(1e-15);
    let mut sweep = end_param - start_param;
    if sweep.abs() < 1e-15 {
        sweep = std::f64::consts::TAU;
    }
    while sweep <= 0.0 {
        sweep += std::f64::consts::TAU;
    }
    let n = ((segments as f64) * (sweep / std::f64::consts::TAU))
        .ceil()
        .max(2.0) as usize;
    let mut pts = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let t = start_param + sweep * (i as f64 / n as f64);
        let p = center + major_dir * (major_len * t.cos()) + minor_dir * (minor_len * t.sin());
        // ELLIPSE center and major axis are already WCS; only project to XY.
        pts.push(p.xy());
    }
    pts
}

pub const POLYLINE_BULGE_SEGMENTS: usize = 16;

// ------------------------------------------------------------
// Function: bulge_arc
// Purpose: Sample a bulge-defined arc in the same 2D plane as the
//          supplied vertices. First and last samples equal P1/P2.
// ------------------------------------------------------------
pub fn bulge_arc(p1: Point2, p2: Point2, bulge: f64, segments: usize) -> Vec<Point2> {
    let Some(arc) = bulge_circle(p1, p2, bulge) else {
        if p1.distance(p2) < 1e-15 {
            return vec![p1];
        }
        return vec![p1, p2];
    };
    let n = segments.max(8);
    let mut pts = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let t = i as f64 / n as f64;
        let angle = arc.start_angle + arc.sweep * t;
        pts.push(Point2::new(
            arc.center.x + arc.radius * angle.cos(),
            arc.center.y + arc.radius * angle.sin(),
        ));
    }
    if let Some(first) = pts.first_mut() {
        *first = p1;
    }
    if let Some(last) = pts.last_mut() {
        *last = p2;
    }
    pts
}

fn ocs_xy(point: Point3) -> Point2 {
    Point2::new(point.x, point.y)
}

fn bulge_sample_to_wcs(sample: Point2, elevation: f64, extrusion: Point3) -> Point2 {
    ocs_to_wcs(Point3::new(sample.x, sample.y, elevation), extrusion).xy()
}

// ------------------------------------------------------------
// Function: polyline_points
// Purpose: Tessellate LWPOLYLINE/POLYLINE vertices. Bulge arcs are
//          constructed in OCS so negative-Z extrusion does not
//          reverse handedness, then each sample is mapped to WCS.
// ------------------------------------------------------------
pub fn polyline_points(vertices: &[PolyVertex], closed: bool, extrusion: Point3) -> Vec<Point2> {
    if vertices.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let n = vertices.len();
    let count = if closed { n } else { n.saturating_sub(1) };
    for i in 0..count {
        let a = vertices[i];
        let b = vertices[(i + 1) % n];
        let mut seg = bulge_arc(
            ocs_xy(a.point),
            ocs_xy(b.point),
            a.bulge,
            POLYLINE_BULGE_SEGMENTS,
        );
        for sample in &mut seg {
            *sample = bulge_sample_to_wcs(*sample, a.point.z, extrusion);
        }
        if !out.is_empty() && !seg.is_empty() {
            seg.remove(0);
        }
        out.extend(seg);
    }
    if !closed {
        if let Some(last) = vertices.last() {
            let p = ocs_to_wcs(last.point, extrusion).xy();
            if out.last().map(|q| q.distance(p) > 1e-12).unwrap_or(true) {
                out.push(p);
            }
        }
    }
    out
}

pub fn bspline_points(
    degree: u32,
    control: &[Point3],
    knots: &[f64],
    weights: &[f64],
    samples: usize,
) -> Vec<Point2> {
    if control.len() < 2 {
        return control.iter().map(|p| p.xy()).collect();
    }
    let p = degree.max(1) as usize;
    if control.len() <= p {
        return control.iter().map(|c| c.xy()).collect();
    }
    let knots = if knot_vector_is_usable(knots, control.len(), p) {
        knots.to_vec()
    } else {
        clamped_uniform_knots(control.len(), p)
    };
    let weights = sanitized_spline_weights(weights, control.len());
    let u0 = knots[p];
    let u1 = knots[control.len()];
    if (u1 - u0).abs() < 1e-15 {
        return control.iter().map(|c| c.xy()).collect();
    }
    let n = samples.max(8);
    let mut pts = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let u = u0 + (u1 - u0) * (i as f64 / n as f64);
        pts.push(de_boor(p, control, &knots, &weights, u).xy());
    }
    pts
}

fn sanitized_spline_weights(weights: &[f64], n_ctrl: usize) -> Vec<f64> {
    if weights.len() < n_ctrl {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(n_ctrl);
    for w in weights.iter().take(n_ctrl) {
        if !w.is_finite() || *w <= 1e-12 {
            return Vec::new();
        }
        out.push(*w);
    }
    let min_w = out.iter().copied().fold(f64::INFINITY, f64::min);
    let max_w = out.iter().copied().fold(0.0, f64::max);
    if max_w / min_w < 1.001 {
        Vec::new()
    } else {
        out
    }
}

fn knot_vector_is_usable(knots: &[f64], n_ctrl: usize, degree: usize) -> bool {
    if knots.len() < n_ctrl + degree + 1 {
        return false;
    }
    for pair in knots.windows(2) {
        if !pair[0].is_finite() || !pair[1].is_finite() || pair[1] + 1e-12 < pair[0] {
            return false;
        }
    }
    true
}

fn clamped_uniform_knots(n_ctrl: usize, degree: usize) -> Vec<f64> {
    let n = n_ctrl + degree + 1;
    let mut k = vec![0.0; n];
    let last = (n_ctrl - degree) as f64;
    for (i, item) in k.iter_mut().enumerate() {
        if i <= degree {
            *item = 0.0;
        } else if i >= n_ctrl {
            *item = last;
        } else {
            *item = (i - degree) as f64;
        }
    }
    k
}

fn de_boor(degree: usize, control: &[Point3], knots: &[f64], weights: &[f64], u: f64) -> Point3 {
    let n = control.len() - 1;
    let mut span = degree;
    while span < n && u >= knots[span + 1] {
        span += 1;
    }
    span = span.min(n);
    let mut d = vec![Point3::default(); degree + 1];
    let mut w = vec![1.0; degree + 1];
    for j in 0..=degree {
        let idx = span + j - degree;
        let idx = idx.clamp(0, n);
        let weight = weights.get(idx).copied().unwrap_or(1.0);
        let weight = if weight.is_finite() && weight > 1e-12 {
            weight
        } else {
            1.0
        };
        d[j] = control[idx] * weight;
        w[j] = weight;
    }
    for r in 1..=degree {
        for j in (r..=degree).rev() {
            let i = span + j - degree;
            let denom = knots[i + degree + 1 - r] - knots[i];
            let alpha = if denom.abs() < 1e-15 {
                0.0
            } else {
                (u - knots[i]) / denom
            };
            let alpha = alpha.clamp(0.0, 1.0);
            d[j] = d[j - 1] * (1.0 - alpha) + d[j] * alpha;
            w[j] = w[j - 1] * (1.0 - alpha) + w[j] * alpha;
        }
    }
    if w[degree].abs() > 1e-15 {
        Point3::new(
            d[degree].x / w[degree],
            d[degree].y / w[degree],
            d[degree].z / w[degree],
        )
    } else {
        control[span.min(n)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-9;

    fn p(x: f64, y: f64) -> Point2 {
        Point2::new(x, y)
    }

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

    fn assert_near(got: f64, expected: f64, label: &str) {
        assert!(
            (got - expected).abs() < EPS,
            "{label}: got {got}, expected {expected}"
        );
    }

    fn sample_mid(pts: &[Point2]) -> Point2 {
        pts[pts.len() / 2]
    }

    fn chord_vertices(a: Point2, b: Point2, bulge: f64) -> [PolyVertex; 2] {
        [
            PolyVertex {
                point: Point3::from_xy(a.x, a.y),
                bulge,
            },
            PolyVertex {
                point: Point3::from_xy(b.x, b.y),
                bulge: 0.0,
            },
        ]
    }

    #[test]
    fn bulge_zero_is_straight() {
        let p1 = p(0.0, 0.0);
        let p2 = p(2.0, 0.0);
        let pts = bulge_arc(p1, p2, 0.0, 16);
        assert_eq!(pts.len(), 2);
        assert_point(pts[0], p1, "start");
        assert_point(pts[1], p2, "end");
        assert!(bulge_circle(p1, p2, 0.0).is_none());
    }

    #[test]
    fn positive_unit_bulge_is_lower_semicircle() {
        let p1 = p(0.0, 0.0);
        let p2 = p(2.0, 0.0);
        let arc = bulge_circle(p1, p2, 1.0).expect("arc");
        assert_point(arc.center, p(1.0, 0.0), "center");
        assert_near(arc.radius, 1.0, "radius");
        assert_near(arc.sweep, std::f64::consts::PI, "sweep");
        let pts = bulge_arc(p1, p2, 1.0, 32);
        assert_point(pts[0], p1, "start");
        assert_point(*pts.last().unwrap(), p2, "end");
        assert_point(sample_mid(&pts), p(1.0, -1.0), "midpoint");
        assert!(
            sample_mid(&pts).y < 0.0,
            "positive bulge stays below +X chord"
        );
    }

    #[test]
    fn negative_unit_bulge_is_upper_semicircle() {
        let p1 = p(0.0, 0.0);
        let p2 = p(2.0, 0.0);
        let arc = bulge_circle(p1, p2, -1.0).expect("arc");
        assert_point(arc.center, p(1.0, 0.0), "center");
        assert_near(arc.radius, 1.0, "radius");
        assert_near(arc.sweep, -std::f64::consts::PI, "sweep");
        let pts = bulge_arc(p1, p2, -1.0, 32);
        assert_point(pts[0], p1, "start");
        assert_point(*pts.last().unwrap(), p2, "end");
        assert_point(sample_mid(&pts), p(1.0, 1.0), "midpoint");
        assert!(
            sample_mid(&pts).y > 0.0,
            "negative bulge stays above +X chord"
        );
    }

    #[test]
    fn plus_tan_22_5_is_ccw_quarter_arc() {
        let p1 = p(0.0, 0.0);
        let p2 = p(2.0, 0.0);
        let bulge = (std::f64::consts::PI / 8.0).tan();
        let arc = bulge_circle(p1, p2, bulge).expect("arc");
        assert_near(arc.sweep, std::f64::consts::FRAC_PI_2, "sweep");
        assert_near(arc.radius, std::f64::consts::SQRT_2, "radius");
        assert_point(arc.center, p(1.0, 1.0), "center");
        let pts = bulge_arc(p1, p2, bulge, 32);
        assert_point(pts[0], p1, "start");
        assert_point(*pts.last().unwrap(), p2, "end");
        let mid = sample_mid(&pts);
        assert_point(mid, p(1.0, 1.0 - std::f64::consts::SQRT_2), "midpoint");
        assert!(mid.y < 0.0);
    }

    #[test]
    fn minus_tan_22_5_is_cw_quarter_arc() {
        let p1 = p(0.0, 0.0);
        let p2 = p(2.0, 0.0);
        let bulge = -((std::f64::consts::PI / 8.0).tan());
        let arc = bulge_circle(p1, p2, bulge).expect("arc");
        assert_near(arc.sweep, -std::f64::consts::FRAC_PI_2, "sweep");
        assert_near(arc.radius, std::f64::consts::SQRT_2, "radius");
        assert_point(arc.center, p(1.0, -1.0), "center");
        let pts = bulge_arc(p1, p2, bulge, 32);
        assert_point(pts[0], p1, "start");
        assert_point(*pts.last().unwrap(), p2, "end");
        let mid = sample_mid(&pts);
        assert_point(mid, p(1.0, -1.0 + std::f64::consts::SQRT_2), "midpoint");
        assert!(mid.y > 0.0);
    }

    #[test]
    fn major_arc_bulge_greater_than_one_keeps_signed_sweep() {
        let p1 = p(0.0, 0.0);
        let p2 = p(2.0, 0.0);
        let bulge = (3.0 * std::f64::consts::PI / 8.0).tan();
        assert!(bulge > 1.0);
        let arc = bulge_circle(p1, p2, bulge).expect("arc");
        assert_near(arc.sweep, 3.0 * std::f64::consts::FRAC_PI_2, "sweep");
        assert_near(arc.radius, std::f64::consts::SQRT_2, "radius");
        assert_point(arc.center, p(1.0, -1.0), "center");
        let pts = bulge_arc(p1, p2, bulge, 48);
        assert_point(pts[0], p1, "start");
        assert_point(*pts.last().unwrap(), p2, "end");
        let mid = sample_mid(&pts);
        assert_point(mid, p(1.0, -1.0 - std::f64::consts::SQRT_2), "midpoint");
        assert!(
            mid.y < -1.0,
            "major CCW arc goes through the far lower side"
        );
    }

    #[test]
    fn reversed_endpoints_change_direction_not_only_shape() {
        let p1 = p(2.0, 0.0);
        let p2 = p(0.0, 0.0);
        let pos = bulge_circle(p1, p2, 1.0).expect("pos");
        let neg = bulge_circle(p1, p2, -1.0).expect("neg");
        assert_point(pos.center, p(1.0, 0.0), "pos center");
        assert_point(neg.center, p(1.0, 0.0), "neg center");
        assert_near(pos.sweep, std::f64::consts::PI, "pos sweep");
        assert_near(neg.sweep, -std::f64::consts::PI, "neg sweep");
        let pos_mid = sample_mid(&bulge_arc(p1, p2, 1.0, 32));
        let neg_mid = sample_mid(&bulge_arc(p1, p2, -1.0, 32));
        assert_point(pos_mid, p(1.0, 1.0), "CCW from right to left is upper");
        assert_point(neg_mid, p(1.0, -1.0), "CW from right to left is lower");
        assert_ne!(pos_mid.y.signum(), neg_mid.y.signum());
    }

    #[test]
    fn world_extrusion_polyline_matches_bulge_arc() {
        let verts = chord_vertices(p(0.0, 0.0), p(2.0, 0.0), 1.0);
        let pts = polyline_points(&verts, false, Point3::new(0.0, 0.0, 1.0));
        assert_point(pts[0], p(0.0, 0.0), "start");
        assert_point(*pts.last().unwrap(), p(2.0, 0.0), "end");
        assert_point(sample_mid(&pts), p(1.0, -1.0), "midpoint");
        assert_eq!(pts.len(), POLYLINE_BULGE_SEGMENTS.max(8) + 1);
    }

    #[test]
    fn negative_z_ocs_preserves_bulge_handedness() {
        let verts = chord_vertices(p(0.0, 0.0), p(2.0, 0.0), 1.0);
        let pts = polyline_points(&verts, false, Point3::new(0.0, 0.0, -1.0));
        assert_point(pts[0], p(0.0, 0.0), "start");
        assert_point(*pts.last().unwrap(), p(-2.0, 0.0), "end X is mirrored");
        let mid = sample_mid(&pts);
        assert_point(mid, p(-1.0, -1.0), "Y side matches OCS, X is mirrored");
        assert!(mid.y < 0.0);
    }

    #[test]
    fn closed_polyline_returns_to_start() {
        let verts = [
            PolyVertex {
                point: Point3::from_xy(0.0, 0.0),
                bulge: 0.0,
            },
            PolyVertex {
                point: Point3::from_xy(1.0, 0.0),
                bulge: 0.0,
            },
            PolyVertex {
                point: Point3::from_xy(1.0, 1.0),
                bulge: 0.0,
            },
        ];
        let pts = polyline_points(&verts, true, Point3::new(0.0, 0.0, 1.0));
        assert!((pts.first().unwrap().x - 0.0).abs() < 1e-12);
        assert!((pts.last().unwrap().x - 0.0).abs() < 1e-12);
        assert!((pts.last().unwrap().y - 0.0).abs() < 1e-12 || pts.len() >= 4);
    }

    #[test]
    fn zero_spline_weights_do_not_explode() {
        let control = [
            Point3::from_xy(0.0, 0.0),
            Point3::from_xy(1.0, 0.0),
            Point3::from_xy(1.0, 1.0),
            Point3::from_xy(0.0, 1.0),
        ];
        let pts = bspline_points(3, &control, &[], &[0.0, 0.0, 0.0, 0.0], 16);
        assert!(pts.len() > 2);
        for p in pts {
            assert!(p.is_finite());
            assert!(p.x.abs() < 10.0);
            assert!(p.y.abs() < 10.0);
        }
    }

    #[test]
    fn uniform_weights_do_not_scale_away_from_origin() {
        let control = [
            Point3::from_xy(1000.0, 500.0),
            Point3::from_xy(1001.0, 500.0),
            Point3::from_xy(1001.0, 501.0),
        ];
        let knots = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let pts = bspline_points(2, &control, &knots, &[0.5, 0.5, 0.5], 12);
        for p in pts {
            assert!(p.is_finite());
            assert!((p.x - 1000.0).abs() < 5.0, "x={}", p.x);
            assert!((p.y - 500.0).abs() < 5.0, "y={}", p.y);
        }
    }

    #[test]
    fn quadratic_stays_in_control_bbox() {
        let control = [
            Point3::from_xy(637009.7, 295419.5),
            Point3::from_xy(637009.9, 295419.1),
            Point3::from_xy(637010.0, 295419.1),
        ];
        let pts = bspline_points(2, &control, &[], &[], 16);
        for p in pts {
            assert!(p.x > 637009.0 && p.x < 637011.0, "x={}", p.x);
            assert!(p.y > 295418.0 && p.y < 295421.0, "y={}", p.y);
        }
    }
}
