//! Curve sampling used by tessellation. All math stays in f64 world units.

use cad_core::{ocs_to_wcs, Point2, Point3, PolyVertex};

pub const CIRCLE_SEGMENTS: usize = 32;

pub fn circle_points(center: Point3, radius: f64, extrusion: Point3, segments: usize) -> Vec<Point2> {
    arc_points(center, radius, 0.0, std::f64::consts::TAU, true, extrusion, segments)
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

/// AutoCAD bulge is tan(included_angle / 4). Positive = CCW.
pub fn bulge_arc(p1: Point2, p2: Point2, bulge: f64, segments: usize) -> Vec<Point2> {
    if bulge.abs() < 1e-12 {
        return vec![p1, p2];
    }
    let chord = p1.distance(p2);
    if chord < 1e-15 {
        return vec![p1];
    }
    let included = 4.0 * bulge.atan();
    let radius = chord / (2.0 * (included / 2.0).sin());
    let mx = (p1.x + p2.x) * 0.5;
    let my = (p1.y + p2.y) * 0.5;
    let dx = (p2.x - p1.x) / chord;
    let dy = (p2.y - p1.y) / chord;
    let offset = (chord / 2.0) * (1.0 / bulge - bulge) / 2.0;
    let cx = mx + dy * offset;
    let cy = my - dx * offset;
    let start = (p1.y - cy).atan2(p1.x - cx);
    let end = (p2.y - cy).atan2(p2.x - cx);
    arc_points(
        Point3::new(cx, cy, 0.0),
        radius.abs(),
        start,
        end,
        bulge < 0.0,
        Point3::new(0.0, 0.0, 1.0),
        segments.max(8),
    )
}

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
        let pa = ocs_to_wcs(a.point, extrusion).xy();
        let pb = ocs_to_wcs(b.point, extrusion).xy();
        let mut seg = bulge_arc(pa, pb, a.bulge, 16);
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

fn de_boor(
    degree: usize,
    control: &[Point3],
    knots: &[f64],
    weights: &[f64],
    u: f64,
) -> Point3 {
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

    #[test]
    fn bulge_zero_is_straight() {
        let pts = bulge_arc(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0), 0.0, 16);
        assert_eq!(pts.len(), 2);
    }

    #[test]
    fn semicircle_bulge_is_one() {
        let pts = bulge_arc(Point2::new(0.0, 0.0), Point2::new(2.0, 0.0), 1.0, 32);
        assert!(pts.len() > 4);
        let mid = &pts[pts.len() / 2];
        assert!((mid.y - 1.0).abs() < 0.15);
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
