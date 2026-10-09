//! Canonical HATCH boundary sampling in world XY.
//!
//! DXF hatch path data is defined in the hatch's OCS. Samples are
//! constructed in that plane using the entity extrusion and elevation,
//! then mapped to WCS so viewport and PDF cannot disagree.

use crate::curves::{
    arc_points, bspline_points, ellipse_arc_points, polyline_points_with_tolerance,
    segments_per_turn, spline_sample_count, CIRCLE_SEGMENTS,
};
use crate::dash::{generate_path_dashes, pattern_period, scaled_pattern, PathSeg};
use crate::entity::{
    default_extrusion, HatchData, HatchEdge, HatchPath, HatchPatternLine, PolyVertex,
    MAX_HATCH_DASHES_PER_SPAN, MAX_HATCH_PATTERN_LINES, MAX_HATCH_PATTERN_SEGMENTS,
};
use crate::geom::{ocs_to_wcs, Point2, Point3};
use crate::transform::Transform2;

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
    hatch_path_points_with_tolerance(path, extrusion, elevation, None)
}

// ------------------------------------------------------------
// Function: hatch_path_points_with_tolerance
// Purpose: Sample one hatch boundary. `Some(tol)` picks arc and
//          bulge density from chord error; `None` keeps the
//          fixed segment counts used by tests and thumbnails.
// ------------------------------------------------------------
pub fn hatch_path_points_with_tolerance(
    path: &HatchPath,
    extrusion: Point3,
    elevation: f64,
    chord_tolerance: Option<f64>,
) -> Vec<Point2> {
    match path {
        HatchPath::Polyline { vertices, closed } => {
            let verts: Vec<PolyVertex> = vertices
                .iter()
                .map(|vertex| PolyVertex {
                    point: ocs_point(vertex.point, elevation),
                    bulge: vertex.bulge,
                    vertex_id: Default::default(),
                })
                .collect();
            polyline_points_with_tolerance(&verts, *closed, extrusion, chord_tolerance)
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
                            chord_tolerance
                                .map(|tol| segments_per_turn(*radius, tol))
                                .unwrap_or(CIRCLE_SEGMENTS),
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
                        let major = Point3::from_xy(major_endpoint.x, major_endpoint.y);
                        let ocs = ellipse_arc_points(
                            Point3::from_xy(center.x, center.y),
                            major,
                            *axis_ratio,
                            *start_angle,
                            *end_angle,
                            *is_ccw,
                            default_extrusion(),
                            chord_tolerance
                                .map(|tol| segments_per_turn(major.length(), tol))
                                .unwrap_or(CIRCLE_SEGMENTS),
                        );
                        let samples: Vec<Point2> = ocs
                            .into_iter()
                            .map(|p| map_ocs(p, elevation, extrusion))
                            .collect();
                        append_edge(&mut pts, samples);
                    }
                    HatchEdge::Spline {
                        degree,
                        knots,
                        weights,
                        control_points,
                        fit_points,
                        ..
                    } => {
                        let samples = sample_hatch_spline(
                            *degree,
                            knots,
                            weights,
                            control_points,
                            fit_points,
                        );
                        let samples: Vec<Point2> = samples
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

fn sample_hatch_spline(
    degree: u32,
    knots: &[f64],
    weights: &[f64],
    control_points: &[Point3],
    fit_points: &[Point3],
) -> Vec<Point2> {
    let degree = if degree == 0 { 3 } else { degree };
    if control_points.len() >= 2 {
        let ocs: Vec<Point3> = control_points
            .iter()
            .map(|p| Point3::from_xy(p.x, p.y))
            .collect();
        return bspline_points(
            degree,
            &ocs,
            knots,
            weights,
            spline_sample_count(ocs.len(), degree),
        );
    }
    fit_points.iter().map(|p| p.xy()).collect()
}

const PATTERN_EPS: f64 = 1e-9;

// ------------------------------------------------------------
// Function: hatch_fill_contours
// Purpose: Keep the loops a hatch style actually fills.
//          Normal keeps every loop, Outer keeps the boundary and
//          its holes, Ignore keeps only the outermost loops.
// ------------------------------------------------------------
pub fn hatch_fill_contours(style: i16, contours: &[Vec<Point2>]) -> Vec<Vec<Point2>> {
    let depths = crate::polygon::contour_depths(contours);
    let keep = |depth: usize| match style {
        2 => depth == 0,
        1 => depth <= 1,
        _ => true,
    };
    contours
        .iter()
        .zip(depths)
        .filter(|(_, depth)| keep(*depth))
        .map(|(contour, _)| contour.clone())
        .collect()
}

// ------------------------------------------------------------
// Struct: GradientRamp
// Purpose: Sample a HATCH gradient. Linear ramps are exact when
//          each triangle vertex is colored. The curved kinds are
//          evaluated per vertex, so the interior of a triangle is
//          a linear blend of those samples.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub struct GradientRamp {
    pub name: String,
    pub angle: f64,
    pub shift: f64,
    pub stops: Vec<(f64, crate::color::Rgb)>,
}

impl GradientRamp {
    pub fn from_gradient(gradient: &crate::entity::HatchGradient) -> Self {
        let mut stops: Vec<(f64, crate::color::Rgb)> = gradient
            .stops
            .iter()
            .filter(|stop| stop.shift.is_finite())
            .map(|stop| (stop.shift, stop.color))
            .collect();
        stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        if stops.len() == 1 && gradient.single_color {
            let tint = if gradient.tint.is_finite() {
                gradient.tint.clamp(0.0, 1.0)
            } else {
                1.0
            };
            let color = stops[0].1;
            let white = crate::color::Rgb {
                r: 255,
                g: 255,
                b: 255,
            };
            stops.push((1.0, lerp_rgb(white, color, tint)));
        }
        if stops.is_empty() {
            stops.push((
                0.0,
                crate::color::Rgb {
                    r: 255,
                    g: 255,
                    b: 255,
                },
            ));
        }
        Self {
            name: gradient.name.clone(),
            angle: if gradient.angle.is_finite() {
                gradient.angle
            } else {
                0.0
            },
            shift: if gradient.shift.is_finite() {
                gradient.shift.clamp(0.0, 1.0)
            } else {
                0.0
            },
            stops,
        }
    }

    pub fn midpoint_color(&self) -> crate::color::Rgb {
        self.color_at(0.5)
    }

    pub fn color_at(&self, t: f64) -> crate::color::Rgb {
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let Some(first) = self.stops.first() else {
            return crate::color::Rgb { r: 0, g: 0, b: 0 };
        };
        if self.stops.len() == 1 || t <= first.0 {
            return first.1;
        }
        let Some(last) = self.stops.last() else {
            return first.1;
        };
        if t >= last.0 {
            return last.1;
        }
        for pair in self.stops.windows(2) {
            let (t0, c0) = pair[0];
            let (t1, c1) = pair[1];
            if t <= t1 {
                let span = t1 - t0;
                let u = if span.abs() < 1e-12 {
                    0.0
                } else {
                    (t - t0) / span
                };
                return lerp_rgb(c0, c1, u);
            }
        }
        last.1
    }

    pub fn parameter(&self, point: Point2, min: Point2, max: Point2) -> f64 {
        let (sin, cos) = self.angle.sin_cos();
        let dir = Point2::new(cos, sin);
        let normal = Point2::new(-sin, cos);
        let center = Point2::new((min.x + max.x) * 0.5, (min.y + max.y) * 0.5);
        let corners = [
            min,
            Point2::new(max.x, min.y),
            max,
            Point2::new(min.x, max.y),
        ];
        let project = |axis: Point2| {
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for corner in corners {
                let d = dot(corner - center, axis);
                lo = lo.min(d);
                hi = hi.max(d);
            }
            (lo, hi)
        };
        let shape = gradient_shape(&self.name);
        let raw = match shape.family {
            GradientFamily::Linear => unit_span(dot(point - center, dir), project(dir)),
            GradientFamily::Cylinder => {
                let span = project(normal);
                let half = span.0.abs().max(span.1.abs()).max(PATTERN_EPS);
                (dot(point - center, normal).abs() / half).clamp(0.0, 1.0)
            }
            GradientFamily::Spherical => {
                let radius = corners
                    .iter()
                    .map(|corner| corner.distance(center))
                    .fold(0.0, f64::max)
                    .max(PATTERN_EPS);
                (point.distance(center) / radius).clamp(0.0, 1.0)
            }
            GradientFamily::Hemispherical => {
                let radius = corners
                    .iter()
                    .map(|corner| corner.distance(center))
                    .fold(0.0, f64::max)
                    .max(PATTERN_EPS);
                let r = (point.distance(center) / radius).clamp(0.0, 1.0);
                (1.0 - (1.0 - r * r).max(0.0).sqrt()).clamp(0.0, 1.0)
            }
            GradientFamily::Curved => {
                let radius = corners
                    .iter()
                    .map(|corner| corner.distance(center))
                    .fold(0.0, f64::max)
                    .max(PATTERN_EPS);
                let r = (point.distance(center) / radius).clamp(0.0, 1.0);
                r * r
            }
        };
        let raw = if shape.inverted { 1.0 - raw } else { raw };
        let compress = 1.0 - self.shift;
        if compress < 1.0 - 1e-9 {
            ((raw - 0.5) * compress + 0.5).clamp(0.0, 1.0)
        } else {
            raw
        }
    }
}

#[derive(Clone, Copy)]
enum GradientFamily {
    Linear,
    Cylinder,
    Spherical,
    Hemispherical,
    Curved,
}

struct GradientShape {
    family: GradientFamily,
    inverted: bool,
}

fn gradient_shape(name: &str) -> GradientShape {
    let upper = name.to_ascii_uppercase();
    let family = if upper.contains("HEMI") {
        GradientFamily::Hemispherical
    } else if upper.contains("SPHER") {
        GradientFamily::Spherical
    } else if upper.contains("CYL") {
        GradientFamily::Cylinder
    } else if upper.contains("CURV") {
        GradientFamily::Curved
    } else {
        GradientFamily::Linear
    };
    GradientShape {
        family,
        inverted: upper.contains("INV"),
    }
}

fn unit_span(value: f64, span: (f64, f64)) -> f64 {
    let width = span.1 - span.0;
    if width.abs() < PATTERN_EPS {
        0.5
    } else {
        ((value - span.0) / width).clamp(0.0, 1.0)
    }
}

fn lerp_rgb(a: crate::color::Rgb, b: crate::color::Rgb, t: f64) -> crate::color::Rgb {
    let mix = |from: u8, to: u8| {
        let v = f64::from(from) + (f64::from(to) - f64::from(from)) * t.clamp(0.0, 1.0);
        v.round().clamp(0.0, 255.0) as u8
    };
    crate::color::Rgb {
        r: mix(a.r, b.r),
        g: mix(a.g, b.g),
        b: mix(a.b, b.b),
    }
}

// ------------------------------------------------------------
// Function: hatch_pattern_segments
// Purpose: Clip each HATCH pattern family to the boundary and
//          return world-space dash segments. `contours` are the
//          boundary samples already mapped into the same space
//          as `transform` (WCS, then INSERT).
// ------------------------------------------------------------
pub fn hatch_pattern_segments(
    hatch: &HatchData,
    contours: &[Vec<Point2>],
    transform: Transform2,
) -> Vec<(Point2, Point2)> {
    let mut out = Vec::new();
    if !transform_is_finite(transform) || contours.is_empty() {
        return out;
    }
    if hatch.pattern_lines.is_empty() {
        let line = fallback_pattern_line(hatch);
        emit_pattern_family(&mut out, hatch, &line, contours, transform);
    } else {
        for line in &hatch.pattern_lines {
            if out.len() >= MAX_HATCH_PATTERN_SEGMENTS {
                break;
            }
            emit_pattern_family(&mut out, hatch, line, contours, transform);
        }
    }
    out
}

fn fallback_pattern_line(hatch: &HatchData) -> HatchPatternLine {
    let angle = if hatch.pattern_angle.is_finite() {
        hatch.pattern_angle
    } else {
        0.0
    };
    let scale = if hatch.pattern_scale.is_finite() && hatch.pattern_scale > 1e-6 {
        hatch.pattern_scale
    } else {
        1.0
    };
    HatchPatternLine {
        angle,
        base: Point3::default(),
        offset: Point3::from_xy(-angle.sin() * scale, angle.cos() * scale),
        dashes: Vec::new(),
    }
}

struct PatternFrame {
    origin: Point2,
    dir: Point2,
    offset: Point2,
    dashes: Vec<f64>,
}

fn emit_pattern_family(
    out: &mut Vec<(Point2, Point2)>,
    hatch: &HatchData,
    line: &HatchPatternLine,
    contours: &[Vec<Point2>],
    transform: Transform2,
) {
    if out.len() >= MAX_HATCH_PATTERN_SEGMENTS {
        return;
    }
    let Some(frame) = pattern_frame(hatch, line, transform) else {
        return;
    };
    let normal = Point2::new(-frame.dir.y, frame.dir.x);
    let spacing = dot(frame.offset, normal);
    let offset_len = hypot(frame.offset);
    if offset_len < PATTERN_EPS || spacing.abs() < offset_len * 1e-9 {
        return;
    }
    let Some((k_min, k_max)) = family_range(contours, frame.origin, normal, spacing) else {
        return;
    };
    let stride = family_stride(k_min, k_max);
    let tol = contour_tolerance(contours);
    let mut drawn = 0_usize;
    let mut k = k_min;
    while k <= k_max + 1e-6 && drawn < MAX_HATCH_PATTERN_LINES {
        drawn += 1;
        if out.len() >= MAX_HATCH_PATTERN_SEGMENTS {
            return;
        }
        let origin = frame.origin + frame.offset * k;
        for (t0, t1) in even_odd_spans(origin, frame.dir, contours, tol) {
            if out.len() >= MAX_HATCH_PATTERN_SEGMENTS {
                return;
            }
            append_span(out, origin, frame.dir, t0, t1, &frame.dashes, tol);
        }
        k += stride;
    }
}

fn pattern_frame(
    hatch: &HatchData,
    line: &HatchPatternLine,
    transform: Transform2,
) -> Option<PatternFrame> {
    if !line.angle.is_finite()
        || !line.base.is_finite()
        || !line.offset.is_finite()
        || line.dashes.iter().any(|dash| !dash.is_finite())
    {
        return None;
    }
    let base = line.base.xy();
    let direction = Point2::new(line.angle.cos(), line.angle.sin());
    let origin = transform.apply(map_ocs(base, hatch.elevation, hatch.extrusion));
    let dir_tip = transform.apply(map_ocs(base + direction, hatch.elevation, hatch.extrusion));
    let offset_tip = transform.apply(map_ocs(
        base + line.offset.xy(),
        hatch.elevation,
        hatch.extrusion,
    ));
    let dir_w = dir_tip - origin;
    let offset = offset_tip - origin;
    let dir_len = hypot(dir_w);
    if !origin.is_finite() || !offset.is_finite() || dir_len < PATTERN_EPS {
        return None;
    }
    Some(PatternFrame {
        origin,
        dir: Point2::new(dir_w.x / dir_len, dir_w.y / dir_len),
        offset,
        dashes: scaled_pattern(&line.dashes, dir_len),
    })
}

fn family_range(
    contours: &[Vec<Point2>],
    origin: Point2,
    normal: Point2,
    spacing: f64,
) -> Option<(f64, f64)> {
    let mut min_k = f64::INFINITY;
    let mut max_k = f64::NEG_INFINITY;
    for contour in contours {
        for point in contour {
            if !point.is_finite() {
                continue;
            }
            let k = dot(*point - origin, normal) / spacing;
            if !k.is_finite() {
                continue;
            }
            min_k = min_k.min(k);
            max_k = max_k.max(k);
        }
    }
    if !min_k.is_finite() || !max_k.is_finite() {
        return None;
    }
    let k_min = (min_k - 1e-7).floor();
    let k_max = (max_k + 1e-7).ceil();
    if !k_min.is_finite() || !k_max.is_finite() || k_max < k_min {
        return None;
    }
    Some((k_min, k_max))
}

fn family_stride(k_min: f64, k_max: f64) -> f64 {
    let count = (k_max - k_min) + 1.0;
    if !count.is_finite() || count <= MAX_HATCH_PATTERN_LINES as f64 {
        1.0
    } else {
        (count / MAX_HATCH_PATTERN_LINES as f64).ceil().max(1.0)
    }
}

fn contour_tolerance(contours: &[Vec<Point2>]) -> f64 {
    let mut min = Point2::new(f64::INFINITY, f64::INFINITY);
    let mut max = Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
    for contour in contours {
        for point in contour {
            if !point.is_finite() {
                continue;
            }
            min.x = min.x.min(point.x);
            min.y = min.y.min(point.y);
            max.x = max.x.max(point.x);
            max.y = max.y.max(point.y);
        }
    }
    let diagonal = (max.x - min.x).hypot(max.y - min.y);
    if diagonal.is_finite() {
        (diagonal * 1e-9).max(PATTERN_EPS)
    } else {
        PATTERN_EPS
    }
}

fn even_odd_spans(
    origin: Point2,
    dir: Point2,
    contours: &[Vec<Point2>],
    tol: f64,
) -> Vec<(f64, f64)> {
    let mut hits = Vec::new();
    for contour in contours {
        let n = contour.len();
        if n < 2 {
            continue;
        }
        let closed = contour[0].distance(contour[n - 1]) <= tol;
        let edges = if closed { n - 1 } else { n };
        for index in 0..edges {
            let a = contour[index];
            let b = contour[(index + 1) % n];
            if !a.is_finite() || !b.is_finite() {
                continue;
            }
            if let Some(t) = line_edge_hit(origin, dir, a, b) {
                hits.push(t);
            }
        }
    }
    hits.retain(|t| t.is_finite());
    hits.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut spans = Vec::new();
    for pair in hits.chunks_exact(2) {
        if pair[1] - pair[0] > tol {
            spans.push((pair[0], pair[1]));
        }
    }
    spans
}

fn line_edge_hit(origin: Point2, dir: Point2, a: Point2, b: Point2) -> Option<f64> {
    // Half-open side test: a vertex on the line belongs to the
    // non-negative side, so a tangent touch does not change parity
    // and a crossing vertex is counted once.
    let da = cross(dir, a - origin);
    let db = cross(dir, b - origin);
    if (da >= 0.0) == (db >= 0.0) {
        return None;
    }
    let edge = b - a;
    let denom = cross(dir, edge);
    if denom.abs() <= PATTERN_EPS {
        return None;
    }
    let t = cross(a - origin, edge) / denom;
    t.is_finite().then_some(t)
}

fn append_span(
    out: &mut Vec<(Point2, Point2)>,
    origin: Point2,
    dir: Point2,
    t0: f64,
    t1: f64,
    dashes: &[f64],
    tol: f64,
) {
    if t1 - t0 <= tol || out.len() >= MAX_HATCH_PATTERN_SEGMENTS {
        return;
    }
    if pattern_is_solid(dashes) || dash_count_exceeds(t1 - t0, dashes) {
        push_clipped(out, origin, dir, t0, t1);
        return;
    }
    let period = pattern_period(dashes);
    if period <= PATTERN_EPS {
        push_clipped(out, origin, dir, t0, t1);
        return;
    }
    let start = (t0 / period).floor() * period;
    if !start.is_finite() || t1 < start {
        push_clipped(out, origin, dir, t0, t1);
        return;
    }
    let segment = PathSeg::Line {
        a: point_at(origin, dir, start),
        b: point_at(origin, dir, t1),
    };
    for (a, b) in generate_path_dashes(&[segment], dashes, true, 1) {
        if out.len() >= MAX_HATCH_PATTERN_SEGMENTS {
            return;
        }
        if let Some((left, right)) = clip_pair(origin, dir, a, b, t0, t1, tol) {
            out.push((left, right));
        }
    }
}

fn pattern_is_solid(pattern: &[f64]) -> bool {
    pattern.is_empty()
        || pattern
            .iter()
            .all(|dash| *dash >= 0.0 && dash.abs() < 1e-15)
}

fn dash_count_exceeds(length: f64, pattern: &[f64]) -> bool {
    let min_advance = pattern
        .iter()
        .map(|dash| dash.abs())
        .filter(|dash| *dash > PATTERN_EPS)
        .fold(f64::INFINITY, f64::min);
    if !min_advance.is_finite() || min_advance <= PATTERN_EPS {
        return false;
    }
    (length / min_advance).ceil() > MAX_HATCH_DASHES_PER_SPAN as f64
}

fn clip_pair(
    origin: Point2,
    dir: Point2,
    a: Point2,
    b: Point2,
    t0: f64,
    t1: f64,
    tol: f64,
) -> Option<(Point2, Point2)> {
    if !a.is_finite() || !b.is_finite() {
        return None;
    }
    let sa = dot(a - origin, dir);
    let sb = dot(b - origin, dir);
    let lo = sa.min(sb).max(t0);
    let hi = sa.max(sb).min(t1);
    if hi - lo <= tol {
        return None;
    }
    let (left, right) = if sa <= sb { (lo, hi) } else { (hi, lo) };
    Some((point_at(origin, dir, left), point_at(origin, dir, right)))
}

fn push_clipped(out: &mut Vec<(Point2, Point2)>, origin: Point2, dir: Point2, t0: f64, t1: f64) {
    if out.len() >= MAX_HATCH_PATTERN_SEGMENTS {
        return;
    }
    let a = point_at(origin, dir, t0);
    let b = point_at(origin, dir, t1);
    if a.is_finite() && b.is_finite() {
        out.push((a, b));
    }
}

fn point_at(origin: Point2, dir: Point2, t: f64) -> Point2 {
    origin + dir * t
}

fn transform_is_finite(transform: Transform2) -> bool {
    transform.m00.is_finite()
        && transform.m01.is_finite()
        && transform.m10.is_finite()
        && transform.m11.is_finite()
        && transform.tx.is_finite()
        && transform.ty.is_finite()
}

fn dot(a: Point2, b: Point2) -> f64 {
    a.x * b.x + a.y * b.y
}

fn cross(a: Point2, b: Point2) -> f64 {
    a.x * b.y - a.y * b.x
}

fn hypot(v: Point2) -> f64 {
    v.x.hypot(v.y)
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
            Point2::new(
                std::f64::consts::FRAC_1_SQRT_2,
                std::f64::consts::FRAC_1_SQRT_2,
            ),
            "midpoint",
        );
        assert!(
            chord_side(pts[0], *pts.last().unwrap(), mid) < 0.0,
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
        assert_point(
            mid,
            Point2::new(
                -std::f64::consts::FRAC_1_SQRT_2,
                -std::f64::consts::FRAC_1_SQRT_2,
            ),
            "midpoint",
        );
        assert!(
            chord_side(pts[0], *pts.last().unwrap(), mid) > 0.0,
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
        let expected = Point2::new(
            2.0 * std::f64::consts::FRAC_1_SQRT_2,
            std::f64::consts::FRAC_1_SQRT_2,
        );
        assert_point(mid, expected, "ellipse midpoint");
        assert!(
            chord_side(pts[0], *pts.last().unwrap(), mid) < 0.0,
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
        assert_point(
            mid,
            Point2::new(
                -2.0 * std::f64::consts::FRAC_1_SQRT_2,
                -std::f64::consts::FRAC_1_SQRT_2,
            ),
            "midpoint",
        );
        assert!(
            chord_side(pts[0], *pts.last().unwrap(), mid) > 0.0,
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
                    vertex_id: Default::default(),
                },
                PolyVertex {
                    point: Point3::from_xy(2.0, 0.0),
                    bulge: 0.0,
                    vertex_id: Default::default(),
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
            ..HatchData::default()
        };
        let pts = hatch_path_points(&hatch.paths[0], hatch.extrusion, hatch.elevation);
        assert_point(pts[0], Point2::new(-1.0, 0.0), "start mirrored");
        assert_point(*pts.last().unwrap(), Point2::new(1.0, 0.0), "end mirrored");
        let mid = sample_mid(&pts);
        assert!(mid.y > 0.0, "CCW semicircle Y is preserved under neg-Z");
    }

    fn square(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Vec<Point2> {
        vec![
            Point2::new(min_x, min_y),
            Point2::new(max_x, min_y),
            Point2::new(max_x, max_y),
            Point2::new(min_x, max_y),
        ]
    }

    #[test]
    fn pattern_line_through_a_vertex_keeps_parity() {
        let square = square(0.0, 0.0, 6.0, 6.0);
        let crossing = even_odd_spans(
            Point2::new(0.0, 0.0),
            {
                let len = 1.0_f64.hypot(0.5);
                Point2::new(1.0 / len, 0.5 / len)
            },
            &[square.clone()],
            1e-8,
        );
        assert_eq!(
            crossing.len(),
            1,
            "a line through a corner still crosses once"
        );
        assert!(crossing[0].1 - crossing[0].0 > 1.0);

        let tangent = even_odd_spans(
            Point2::new(-1.0, 6.0),
            Point2::new(1.0, 0.0),
            &[square.clone()],
            1e-8,
        );
        assert_eq!(
            tangent.len(),
            1,
            "the coincident edge is one span, not a ray"
        );
        let (t0, t1) = tangent[0];
        assert!(
            (t0 - 1.0).abs() < 1e-6 && (t1 - 7.0).abs() < 1e-6,
            "{t0}..{t1}"
        );

        let touch = even_odd_spans(
            Point2::new(-1.0, 6.0),
            Point2::new(1.0, 0.0),
            &[vec![
                Point2::new(1.0, 0.0),
                Point2::new(5.0, 0.0),
                Point2::new(3.0, 6.0),
            ]],
            1e-8,
        );
        assert!(touch.is_empty(), "touching the apex does not fill a span");
    }

    #[test]
    fn pattern_line_between_touching_loops_stays_out_of_the_gap() {
        let left = square(0.0, 0.0, 2.0, 2.0);
        let right = square(3.0, 0.0, 5.0, 2.0);
        let spans = even_odd_spans(
            Point2::new(-1.0, 1.0),
            Point2::new(1.0, 0.0),
            &[left, right],
            1e-8,
        );
        assert_eq!(spans.len(), 2);
        assert!(spans.iter().all(|(t0, t1)| !(*t0 < 3.5 && *t1 > 3.5)));
    }

    #[test]
    fn hatch_style_drops_islands_or_holes() {
        let outer = square(0.0, 0.0, 10.0, 10.0);
        let hole = square(2.0, 2.0, 8.0, 8.0);
        let island = square(4.0, 4.0, 6.0, 6.0);
        let contours = [outer, hole, island];
        assert_eq!(hatch_fill_contours(0, &contours).len(), 3);
        assert_eq!(hatch_fill_contours(1, &contours).len(), 2);
        assert_eq!(hatch_fill_contours(2, &contours).len(), 1);
    }

    #[test]
    fn spline_edge_follows_the_curve_not_the_control_polygon() {
        let path = HatchPath::Edges(vec![HatchEdge::Spline {
            degree: 3,
            periodic: false,
            knots: Vec::new(),
            weights: Vec::new(),
            control_points: vec![
                Point3::from_xy(0.0, 0.0),
                Point3::from_xy(0.0, 1.0),
                Point3::from_xy(1.0, 1.0),
                Point3::from_xy(1.0, 0.0),
            ],
            fit_points: Vec::new(),
        }]);
        let pts = hatch_path_points(&path, world_z(), 0.0);
        let max_y = pts.iter().map(|p| p.y).fold(0.0, f64::max);
        assert!(max_y > 0.4, "the curve rises, got {max_y}");
        assert!(
            max_y < 0.95,
            "a cubic stays inside the control polygon, got {max_y}"
        );
    }
}
