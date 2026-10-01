//! Curve sampling shared by viewport tessellation and vector PDF export.

use crate::entity::PolyVertex;
use crate::geom::{ocs_to_wcs, Point2, Point3};
use crate::measure::bulge_circle;

pub const CIRCLE_SEGMENTS: usize = 32;
const MAX_SEGMENTS_PER_TURN: usize = 512;

// ------------------------------------------------------------
// Function: segments_per_turn
// Purpose: How many chords a full circle needs so the sagitta
//          stays within `chord_tolerance`. Clamped to the old
//          32-segment floor and a 512-segment cap.
// ------------------------------------------------------------
pub fn segments_per_turn(radius: f64, chord_tolerance: f64) -> usize {
    let radius = radius.abs();
    if radius < 1e-15 {
        return CIRCLE_SEGMENTS;
    }
    let tol = chord_tolerance.max(1e-12);
    if tol >= radius {
        return CIRCLE_SEGMENTS;
    }
    let step = 2.0 * (1.0 - (tol / radius).min(1.0)).acos();
    if !step.is_finite() || step < 1e-9 {
        return MAX_SEGMENTS_PER_TURN;
    }
    let n = (std::f64::consts::TAU / step).ceil() as usize;
    n.clamp(CIRCLE_SEGMENTS, MAX_SEGMENTS_PER_TURN)
}

// ------------------------------------------------------------
// Function: segments_for_arc
// Purpose: Chord count for one arc of `sweep` radians, using the
//          same per-turn density as `segments_per_turn`.
// ------------------------------------------------------------
pub fn segments_for_arc(radius: f64, sweep: f64, chord_tolerance: f64) -> usize {
    let per_turn = segments_per_turn(radius, chord_tolerance);
    ((per_turn as f64) * (sweep.abs() / std::f64::consts::TAU))
        .ceil()
        .max(2.0) as usize
}

pub fn spline_sample_count(control_len: usize, degree: u32) -> usize {
    let spans = control_len.saturating_sub(degree.max(1) as usize).max(1);
    (spans * 16).clamp(24, 512)
}

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
    ellipse_arc_points(
        center,
        major_axis,
        axis_ratio,
        start_param,
        end_param,
        true,
        extrusion,
        segments,
    )
}

// ------------------------------------------------------------
// Function: ellipse_arc_points
// Purpose: Sample an elliptic arc in the same parameter space as
//          DXF ELLIPSE / HATCH ellipse edges. `ccw` selects the
//          short increasing-parameter sweep versus the opposite
//          clockwise sweep. ELLIPSE entities are WCS; hatch edges
//          should be sampled in OCS then mapped with `ocs_to_wcs`.
// ------------------------------------------------------------
pub fn ellipse_arc_points(
    center: Point3,
    major_axis: Point3,
    axis_ratio: f64,
    start_param: f64,
    end_param: f64,
    ccw: bool,
    extrusion: Point3,
    segments: usize,
) -> Vec<Point2> {
    let major_len = major_axis.length().max(1e-15);
    let major_dir = major_axis.normalized();
    let minor_dir = extrusion.normalized().cross(major_dir).normalized();
    let minor_len = major_len * axis_ratio.abs().max(1e-15);
    let mut sweep = if ccw {
        end_param - start_param
    } else {
        start_param - end_param
    };
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
        let param = if ccw {
            start_param + sweep * t
        } else {
            start_param - sweep * t
        };
        let p =
            center + major_dir * (major_len * param.cos()) + minor_dir * (minor_len * param.sin());
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

fn bulge_segments(p1: Point2, p2: Point2, bulge: f64, chord_tolerance: Option<f64>) -> usize {
    let Some(tol) = chord_tolerance else {
        return POLYLINE_BULGE_SEGMENTS;
    };
    let Some(arc) = bulge_circle(p1, p2, bulge) else {
        return 2;
    };
    segments_for_arc(arc.radius, arc.sweep, tol)
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
    polyline_points_with_tolerance(vertices, closed, extrusion, None)
}

// ------------------------------------------------------------
// Function: polyline_points_with_tolerance
// Purpose: Same as `polyline_points`, but each bulge arc uses
//          `chord_tolerance` instead of a fixed segment count.
//          `None` keeps the historical 16-segment bulges.
// ------------------------------------------------------------
pub fn polyline_points_with_tolerance(
    vertices: &[PolyVertex],
    closed: bool,
    extrusion: Point3,
    chord_tolerance: Option<f64>,
) -> Vec<Point2> {
    if vertices.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let n = vertices.len();
    let count = if closed { n } else { n.saturating_sub(1) };
    for i in 0..count {
        let a = vertices[i];
        let b = vertices[(i + 1) % n];
        let segments = bulge_segments(ocs_xy(a.point), ocs_xy(b.point), a.bulge, chord_tolerance);
        let mut seg = bulge_arc(ocs_xy(a.point), ocs_xy(b.point), a.bulge, segments);
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

// ------------------------------------------------------------
// Function: catmull_rom_fit_points
// Purpose: Draw a fit-point spline as a centripetal Catmull-Rom
//          curve through every fit point, instead of straight chords.
// ------------------------------------------------------------
pub fn catmull_rom_fit_points(points: &[Point3], closed: bool) -> Vec<Point2> {
    if points.len() < 3 {
        return points.iter().map(|p| p.xy()).collect();
    }
    let samples_per_span = 16usize;
    let n = points.len();
    let spans = if closed { n } else { n - 1 };
    let mut out = Vec::with_capacity(spans * samples_per_span + 1);
    for i in 0..spans {
        let p1 = points[i % n].xy();
        let p2 = points[(i + 1) % n].xy();
        let p0 = if !closed && i == 0 {
            p1
        } else {
            points[(i + n - 1) % n].xy()
        };
        let p3 = if !closed && i + 1 == spans {
            p2
        } else {
            points[(i + 2) % n].xy()
        };
        let steps = if i + 1 == spans {
            samples_per_span
        } else {
            samples_per_span - 1
        };
        for step in 0..=steps {
            let t = step as f64 / samples_per_span as f64;
            out.push(catmull_rom_centripetal(p0, p1, p2, p3, t));
        }
    }
    if let Some(first) = out.first_mut() {
        *first = points[0].xy();
    }
    if let Some(last) = out.last_mut() {
        let end = if closed {
            points[0].xy()
        } else {
            points[n - 1].xy()
        };
        *last = end;
    }
    out
}

fn catmull_rom_centripetal(p0: Point2, p1: Point2, p2: Point2, p3: Point2, t: f64) -> Point2 {
    let alpha = 0.5;
    let knot = |a: Point2, b: Point2, ti: f64| ti + a.distance(b).powf(alpha).max(1e-9);
    let t0 = 0.0;
    let t1 = knot(p0, p1, t0);
    let t2 = knot(p1, p2, t1);
    let t3 = knot(p2, p3, t2);
    if (t2 - t1).abs() < 1e-12 {
        return p1.lerp(p2, t.clamp(0.0, 1.0));
    }
    let u = t1 + (t2 - t1) * t.clamp(0.0, 1.0);
    let lerp_at = |a: Point2, b: Point2, ta: f64, tb: f64| {
        if (tb - ta).abs() < 1e-12 {
            b
        } else {
            a * ((tb - u) / (tb - ta)) + b * ((u - ta) / (tb - ta))
        }
    };
    let a1 = lerp_at(p0, p1, t0, t1);
    let a2 = lerp_at(p1, p2, t1, t2);
    let a3 = lerp_at(p2, p3, t2, t3);
    let b1 = lerp_at(a1, a2, t0, t2);
    let b2 = lerp_at(a2, a3, t1, t3);
    lerp_at(b1, b2, t1, t2)
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
                vertex_id: Default::default(),
            },
            PolyVertex {
                point: Point3::from_xy(b.x, b.y),
                bulge: 0.0,
                vertex_id: Default::default(),
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
                vertex_id: Default::default(),
            },
            PolyVertex {
                point: Point3::from_xy(1.0, 0.0),
                bulge: 0.0,
                vertex_id: Default::default(),
            },
            PolyVertex {
                point: Point3::from_xy(1.0, 1.0),
                bulge: 0.0,
                vertex_id: Default::default(),
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

    #[test]
    fn ellipse_arc_cw_and_ccw_keep_start_end_and_side() {
        let center = Point3::from_xy(0.0, 0.0);
        let major = Point3::from_xy(2.0, 0.0);
        let extrusion = Point3::new(0.0, 0.0, 1.0);
        let ccw = ellipse_arc_points(
            center,
            major,
            0.5,
            0.0,
            std::f64::consts::FRAC_PI_2,
            true,
            extrusion,
            32,
        );
        let cw = ellipse_arc_points(
            center,
            major,
            0.5,
            0.0,
            std::f64::consts::FRAC_PI_2,
            false,
            extrusion,
            32,
        );
        assert_point(ccw[0], p(2.0, 0.0), "ccw start");
        assert_point(*ccw.last().unwrap(), p(0.0, 1.0), "ccw end");
        assert_point(cw[0], p(2.0, 0.0), "cw start");
        assert_point(*cw.last().unwrap(), p(0.0, 1.0), "cw end");
        let ccw_mid = sample_mid(&ccw);
        let cw_mid = sample_mid(&cw);
        assert!(ccw_mid.x > 0.0 && ccw_mid.y > 0.0);
        assert!(cw_mid.x < 0.0 && cw_mid.y < 0.0);
        assert_ne!(ccw_mid.y.signum(), cw_mid.y.signum());
    }

    #[test]
    fn arc_chords_stay_inside_the_tolerance() {
        let radius = 250.0;
        let tol = 0.05;
        let n = segments_for_arc(radius, std::f64::consts::TAU, tol);
        let pts = circle_points(
            Point3::from_xy(0.0, 0.0),
            radius,
            Point3::new(0.0, 0.0, 1.0),
            n,
        );
        let mut worst = 0.0_f64;
        for pair in pts.windows(2) {
            let mid = pair[0].lerp(pair[1], 0.5);
            let sagitta = radius - mid.distance(p(0.0, 0.0));
            worst = worst.max(sagitta);
        }
        assert!(
            worst <= tol + 1e-6,
            "sagitta {worst} exceeds tolerance {tol}"
        );
        assert!(n >= CIRCLE_SEGMENTS);
        assert!(n <= 512);
    }

    #[test]
    fn fit_point_spline_passes_through_the_points() {
        let fit = [
            Point3::from_xy(0.0, 0.0),
            Point3::from_xy(1.0, 2.0),
            Point3::from_xy(3.0, 2.0),
            Point3::from_xy(4.0, 0.0),
        ];
        let pts = catmull_rom_fit_points(&fit, false);
        assert!(pts.len() > fit.len());
        for target in &fit {
            let hit = pts.iter().any(|p| p.distance(target.xy()) < 1e-6);
            assert!(hit, "missing fit point ({}, {})", target.x, target.y);
        }
        let mid = pts[pts.len() / 2];
        assert!(mid.y > 0.5, "curve should bow upward, mid y={}", mid.y);
    }
}
