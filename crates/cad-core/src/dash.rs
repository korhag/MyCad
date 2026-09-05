//! Path-length linetype generation.
//!
//! Dash placement is computed on mathematical path distance. Tessellation
//! chords are a display approximation and never restart the pattern.

use crate::curves::CIRCLE_SEGMENTS;
use crate::geom::Point2;
use crate::measure::bulge_circle;

const PATH_EPS: f64 = 1e-12;

#[derive(Debug, Clone, Copy)]
pub struct DashState {
    pub pattern_idx: usize,
    pub remaining: f64,
}

impl DashState {
    pub fn new(pattern: &[f64]) -> Self {
        Self {
            pattern_idx: 0,
            remaining: pattern.first().map(|d| d.abs()).unwrap_or(0.0),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PathSeg {
    Line {
        a: Point2,
        b: Point2,
    },
    Arc {
        center: Point2,
        radius: f64,
        start: f64,
        sweep: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DashInterval {
    pub s0: f64,
    pub s1: f64,
}

impl PathSeg {
    pub fn length(self) -> f64 {
        match self {
            Self::Line { a, b } => a.distance(b),
            Self::Arc { radius, sweep, .. } => radius.abs() * sweep.abs(),
        }
    }

    pub fn point_at(self, s: f64) -> Point2 {
        let len = self.length();
        let t = if len < PATH_EPS {
            0.0
        } else {
            (s / len).clamp(0.0, 1.0)
        };
        match self {
            Self::Line { a, b } => a.lerp(b, t),
            Self::Arc {
                center,
                radius,
                start,
                sweep,
            } => {
                let angle = start + sweep * t;
                Point2::new(
                    center.x + radius * angle.cos(),
                    center.y + radius * angle.sin(),
                )
            }
        }
    }

    pub fn tangent_at(self, s: f64) -> Point2 {
        match self {
            Self::Line { a, b } => normalize(Point2::new(b.x - a.x, b.y - a.y)),
            Self::Arc { start, sweep, .. } => {
                let len = self.length();
                let t = if len < PATH_EPS {
                    0.0
                } else {
                    (s / len).clamp(0.0, 1.0)
                };
                let angle = start + sweep * t;
                let (sin, cos) = angle.sin_cos();
                if sweep >= 0.0 {
                    Point2::new(-sin, cos)
                } else {
                    Point2::new(sin, -cos)
                }
            }
        }
    }

    pub fn from_three_arc_points(start: Point2, mid: Point2, end: Point2) -> Option<Self> {
        let (center, radius, start_ang, sweep) = circumcircle_sweep(start, mid, end)?;
        Some(Self::Arc {
            center,
            radius,
            start: start_ang,
            sweep,
        })
    }

    pub fn full_circle_from_points(samples: &[Point2]) -> Option<Self> {
        if samples.len() < 3 {
            return None;
        }
        let (center, radius, start, _) = circumcircle_sweep(samples[0], samples[1], samples[2])?;
        let mut area = 0.0;
        for i in 0..samples.len() {
            let a = samples[i];
            let b = samples[(i + 1) % samples.len()];
            area += a.x * b.y - a.y * b.x;
        }
        let sweep = if area >= 0.0 {
            std::f64::consts::TAU
        } else {
            -std::f64::consts::TAU
        };
        Some(Self::Arc {
            center,
            radius,
            start,
            sweep,
        })
    }
}

fn circumcircle_sweep(a: Point2, mid: Point2, b: Point2) -> Option<(Point2, f64, f64, f64)> {
    let d = 2.0 * (a.x * (mid.y - b.y) + mid.x * (b.y - a.y) + b.x * (a.y - mid.y));
    if d.abs() < 1e-14 {
        return None;
    }
    let a2 = a.x * a.x + a.y * a.y;
    let m2 = mid.x * mid.x + mid.y * mid.y;
    let b2 = b.x * b.x + b.y * b.y;
    let ux = (a2 * (mid.y - b.y) + m2 * (b.y - a.y) + b2 * (a.y - mid.y)) / d;
    let uy = (a2 * (b.x - mid.x) + m2 * (a.x - b.x) + b2 * (mid.x - a.x)) / d;
    let center = Point2::new(ux, uy);
    let radius = center.distance(a);
    if !radius.is_finite() || radius < PATH_EPS {
        return None;
    }
    let start = (a.y - center.y).atan2(a.x - center.x);
    let mid_ang = (mid.y - center.y).atan2(mid.x - center.x);
    let end = (b.y - center.y).atan2(b.x - center.x);
    let sweep = signed_sweep_through(start, mid_ang, end);
    Some((center, radius, start, sweep))
}

fn signed_sweep_through(start: f64, mid: f64, end: f64) -> f64 {
    let to_mid = wrap_signed(mid - start);
    let mid_to_end = wrap_signed(end - mid);
    let mut sweep = to_mid + mid_to_end;
    if sweep.abs() < 1e-15 {
        sweep = if to_mid >= 0.0 {
            std::f64::consts::TAU
        } else {
            -std::f64::consts::TAU
        };
    }
    sweep
}

fn wrap_signed(delta: f64) -> f64 {
    let mut d = delta;
    while d <= -std::f64::consts::PI {
        d += std::f64::consts::TAU;
    }
    while d > std::f64::consts::PI {
        d -= std::f64::consts::TAU;
    }
    d
}

fn normalize(v: Point2) -> Point2 {
    let len = (v.x * v.x + v.y * v.y).sqrt();
    if len < PATH_EPS {
        Point2::new(1.0, 0.0)
    } else {
        Point2::new(v.x / len, v.y / len)
    }
}

pub fn scaled_pattern(dashes: &[f64], scale: f64) -> Vec<f64> {
    dashes.iter().map(|d| d * scale).collect()
}

pub fn pattern_period(pattern: &[f64]) -> f64 {
    pattern.iter().map(|d| d.abs()).sum()
}

pub fn first_positive_dash(pattern: &[f64]) -> f64 {
    pattern
        .iter()
        .copied()
        .find(|d| *d > PATH_EPS)
        .unwrap_or(0.0)
}

/// Walk `len` of path using `state`. Visible dashes are reported as
/// `[s0, s1)` in the local segment parameter; dots as a single `s`.
pub fn walk_length(
    len: f64,
    pattern: &[f64],
    state: &mut DashState,
    mut on_dash: impl FnMut(f64, f64),
    mut on_dot: impl FnMut(f64),
) {
    if len <= PATH_EPS {
        return;
    }
    if pattern.is_empty() || pattern.iter().all(|d| *d >= 0.0 && d.abs() < 1e-15) {
        on_dash(0.0, len);
        return;
    }
    let has_progress = pattern.iter().any(|d| d.abs() > PATH_EPS);
    let min_advance = pattern
        .iter()
        .map(|d| d.abs())
        .filter(|d| *d > PATH_EPS)
        .fold(f64::INFINITY, f64::min);
    let max_steps = if has_progress {
        ((len / min_advance.max(1e-9)).ceil() as u64)
            .saturating_mul(pattern.len() as u64 + 1)
            .saturating_add(16)
            .max(16)
    } else {
        pattern.len() as u64 + 2
    };

    let mut dist = 0.0;
    let mut steps = 0u64;
    while dist < len - PATH_EPS {
        steps += 1;
        if steps > max_steps {
            break;
        }
        if pattern.is_empty() {
            on_dash(dist, len);
            break;
        }
        let idx = state.pattern_idx % pattern.len();
        let elem = pattern[idx];
        if elem.abs() <= PATH_EPS {
            on_dot(dist);
            state.pattern_idx = state.pattern_idx.wrapping_add(1);
            state.remaining = pattern[state.pattern_idx % pattern.len()].abs();
            if !has_progress {
                break;
            }
            continue;
        }
        if state.remaining <= PATH_EPS {
            state.remaining = elem.abs();
        }
        let take = state.remaining.min(len - dist);
        if elem >= 0.0 {
            on_dash(dist, dist + take);
        }
        dist += take;
        state.remaining -= take;
        if state.remaining <= PATH_EPS {
            state.pattern_idx = state.pattern_idx.wrapping_add(1);
            state.remaining = pattern[state.pattern_idx % pattern.len()].abs();
        }
    }
}

fn merge_intervals(spans: &mut Vec<DashInterval>) {
    if spans.len() < 2 {
        return;
    }
    spans.sort_by(|a, b| a.s0.partial_cmp(&b.s0).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = Vec::with_capacity(spans.len());
    let mut cur = spans[0];
    for next in spans.iter().skip(1) {
        if next.s0 <= cur.s1 + 1e-9 {
            cur.s1 = cur.s1.max(next.s1);
        } else {
            out.push(cur);
            cur = *next;
        }
    }
    out.push(cur);
    *spans = out;
}

fn a_aligned_dashes(len: f64, pattern: &[f64]) -> Vec<DashInterval> {
    let first = first_positive_dash(pattern);
    if first > PATH_EPS && len <= first {
        return vec![DashInterval { s0: 0.0, s1: len }];
    }
    let mut state = DashState::new(pattern);
    let mut dashes = Vec::new();
    walk_length(
        len,
        pattern,
        &mut state,
        |s0, s1| dashes.push(DashInterval { s0, s1 }),
        |_| {},
    );
    if first > PATH_EPS {
        dashes.push(DashInterval {
            s0: 0.0,
            s1: first.min(len),
        });
        dashes.push(DashInterval {
            s0: (len - first).max(0.0),
            s1: len,
        });
        merge_intervals(&mut dashes);
    }
    dashes
}

/// Global-path dash intervals for `segs`. `s` is cumulative path length.
pub fn dash_intervals(segs: &[PathSeg], pattern: &[f64], plinegen: bool) -> Vec<DashInterval> {
    if pattern.is_empty() {
        let total: f64 = segs.iter().map(|s| s.length()).sum();
        return if total > PATH_EPS {
            vec![DashInterval { s0: 0.0, s1: total }]
        } else {
            Vec::new()
        };
    }
    let mut out = Vec::new();
    let mut offset = 0.0;
    let mut state = DashState::new(pattern);
    for seg in segs {
        let len = seg.length();
        if plinegen {
            walk_length(
                len,
                pattern,
                &mut state,
                |s0, s1| {
                    out.push(DashInterval {
                        s0: offset + s0,
                        s1: offset + s1,
                    })
                },
                |_| {},
            );
        } else {
            for span in a_aligned_dashes(len, pattern) {
                out.push(DashInterval {
                    s0: offset + span.s0,
                    s1: offset + span.s1,
                });
            }
            state = DashState::new(pattern);
        }
        offset += len;
    }
    if plinegen {
        merge_intervals(&mut out);
    }
    out
}

pub fn dash_start_points(segs: &[PathSeg], pattern: &[f64], plinegen: bool) -> Vec<Point2> {
    dash_intervals(segs, pattern, plinegen)
        .into_iter()
        .filter_map(|span| point_at_path(segs, span.s0))
        .collect()
}

fn point_at_path(segs: &[PathSeg], mut s: f64) -> Option<Point2> {
    for seg in segs {
        let len = seg.length();
        if s <= len + 1e-9 {
            return Some(seg.point_at(s.min(len)));
        }
        s -= len;
    }
    segs.last().map(|seg| seg.point_at(seg.length()))
}

fn sample_span(seg: PathSeg, s0: f64, s1: f64, chord_segments: usize) -> Vec<Point2> {
    match seg {
        PathSeg::Line { .. } => vec![seg.point_at(s0), seg.point_at(s1)],
        PathSeg::Arc { sweep, .. } => {
            let len = seg.length();
            if len < PATH_EPS {
                return vec![seg.point_at(s0)];
            }
            let frac = ((s1 - s0) / len).abs();
            let n = ((chord_segments as f64) * (sweep.abs() / std::f64::consts::TAU) * frac)
                .ceil()
                .max(1.0) as usize;
            let mut pts = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = i as f64 / n as f64;
                pts.push(seg.point_at(s0 + (s1 - s0) * t));
            }
            pts
        }
    }
}

fn emit_dot(out: &mut Vec<(Point2, Point2)>, seg: PathSeg, s: f64, pattern: &[f64]) {
    let pos = seg.point_at(s);
    let tangent = seg.tangent_at(s);
    let period = pattern_period(pattern);
    let size = if period > PATH_EPS {
        (period * 0.02).clamp(1e-4, period.max(1e-4))
    } else {
        1e-3
    };
    let a = Point2::new(
        pos.x - tangent.x * size * 0.5,
        pos.y - tangent.y * size * 0.5,
    );
    let b = Point2::new(
        pos.x + tangent.x * size * 0.5,
        pos.y + tangent.y * size * 0.5,
    );
    out.push((a, b));
}

/// GPU line pairs for visible dashes and dots. Arc dashes are sampled with
/// `chord_segments` for display only; dash boundaries stay on path length.
pub fn generate_path_dashes(
    segs: &[PathSeg],
    pattern: &[f64],
    plinegen: bool,
    chord_segments: usize,
) -> Vec<(Point2, Point2)> {
    let mut out = Vec::new();
    if segs.is_empty() {
        return out;
    }
    if pattern.is_empty() || pattern.iter().all(|d| *d >= 0.0 && d.abs() < 1e-15) {
        for seg in segs {
            let pts = sample_span(*seg, 0.0, seg.length(), chord_segments.max(CIRCLE_SEGMENTS));
            for w in pts.windows(2) {
                out.push((w[0], w[1]));
            }
        }
        return out;
    }
    let mut state = DashState::new(pattern);
    for seg in segs {
        let len = seg.length();
        if plinegen {
            let mut dashes = Vec::new();
            let mut dots = Vec::new();
            walk_length(
                len,
                pattern,
                &mut state,
                |s0, s1| dashes.push((s0, s1)),
                |s| dots.push(s),
            );
            for (s0, s1) in dashes {
                let pts = sample_span(*seg, s0, s1, chord_segments);
                for w in pts.windows(2) {
                    out.push((w[0], w[1]));
                }
            }
            for s in dots {
                emit_dot(&mut out, *seg, s, pattern);
            }
        } else {
            for span in a_aligned_dashes(len, pattern) {
                let pts = sample_span(*seg, span.s0, span.s1, chord_segments);
                for w in pts.windows(2) {
                    out.push((w[0], w[1]));
                }
            }
            let mut dots_state = DashState::new(pattern);
            walk_length(
                len,
                pattern,
                &mut dots_state,
                |_, _| {},
                |s| {
                    emit_dot(&mut out, *seg, s, pattern);
                },
            );
            state = DashState::new(pattern);
        }
    }
    out
}

pub fn polyline_path_segs(
    vertices: &[crate::PolyVertex],
    closed: bool,
    extrusion: crate::Point3,
    transform: crate::Transform2,
) -> Vec<PathSeg> {
    if vertices.len() < 2 && !(closed && vertices.len() == 1) {
        return Vec::new();
    }
    if vertices.is_empty() {
        return Vec::new();
    }
    let n = vertices.len();
    let count = if closed { n } else { n.saturating_sub(1) };
    let mut segs = Vec::with_capacity(count);
    for i in 0..count {
        let a = vertices[i];
        let b = vertices[(i + 1) % n];
        segs.push(poly_vertex_seg(a, b, extrusion, transform));
    }
    segs
}

fn poly_vertex_seg(
    a: crate::PolyVertex,
    b: crate::PolyVertex,
    extrusion: crate::Point3,
    transform: crate::Transform2,
) -> PathSeg {
    let to_world = |sample: Point2, elevation: f64| {
        transform.apply(
            crate::ocs_to_wcs(crate::Point3::new(sample.x, sample.y, elevation), extrusion).xy(),
        )
    };
    let p1 = Point2::new(a.point.x, a.point.y);
    let p2 = Point2::new(b.point.x, b.point.y);
    if a.bulge.abs() < 1e-12 {
        return PathSeg::Line {
            a: to_world(p1, a.point.z),
            b: to_world(p2, b.point.z),
        };
    }
    if let Some(arc) = bulge_circle(p1, p2, a.bulge) {
        let pt = |t: f64| {
            let angle = arc.start_angle + arc.sweep * t;
            to_world(
                Point2::new(
                    arc.center.x + arc.radius * angle.cos(),
                    arc.center.y + arc.radius * angle.sin(),
                ),
                a.point.z,
            )
        };
        if let Some(seg) = PathSeg::from_three_arc_points(pt(0.0), pt(0.5), pt(1.0)) {
            return seg;
        }
    }
    PathSeg::Line {
        a: to_world(p1, a.point.z),
        b: to_world(p2, b.point.z),
    }
}

pub fn line_chain(pts: &[Point2], closed: bool) -> Vec<PathSeg> {
    if pts.len() < 2 {
        return Vec::new();
    }
    let n = if closed { pts.len() } else { pts.len() - 1 };
    let mut segs = Vec::with_capacity(n);
    for i in 0..n {
        segs.push(PathSeg::Line {
            a: pts[i],
            b: pts[(i + 1) % pts.len()],
        });
    }
    segs
}

pub fn circle_path_segs(pts: &[Point2]) -> Vec<PathSeg> {
    let unique: Vec<Point2> = if pts.len() >= 2 && pts.first() == pts.last() {
        pts[..pts.len() - 1].to_vec()
    } else {
        pts.to_vec()
    };
    PathSeg::full_circle_from_points(&unique)
        .map(|seg| vec![seg])
        .unwrap_or_else(|| line_chain(pts, true))
}

pub fn arc_path_segs(pts: &[Point2]) -> Vec<PathSeg> {
    if pts.len() >= 3 {
        if let Some(seg) =
            PathSeg::from_three_arc_points(pts[0], pts[pts.len() / 2], *pts.last().unwrap())
        {
            return vec![seg];
        }
    }
    line_chain(pts, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Point3, PolyVertex, Transform2};

    const EPS: f64 = 1e-6;

    fn line(len: f64) -> PathSeg {
        PathSeg::Line {
            a: Point2::new(0.0, 0.0),
            b: Point2::new(len, 0.0),
        }
    }

    #[test]
    fn dash_long_line_no_cap() {
        let pattern = [1.0, -1.0];
        let segs = [line(200.0)];
        let spans = dash_intervals(&segs, &pattern, true);
        assert!(
            spans.len() > 64,
            "expected well over 64 dashes, got {}",
            spans.len()
        );
        let last = *spans.last().unwrap();
        assert!(
            last.s1 > 190.0,
            "pattern must continue to the end of the entity, last dash ends at {}",
            last.s1
        );
        assert!((last.s1 - 199.0).abs() < EPS || last.s1 > 198.0);
    }

    #[test]
    fn dash_phase_continuous_two_chords() {
        let pattern = [32.0, -6.0, 4.0, -6.0];
        let one = [line(100.0)];
        let two = [
            PathSeg::Line {
                a: Point2::new(0.0, 0.0),
                b: Point2::new(40.0, 0.0),
            },
            PathSeg::Line {
                a: Point2::new(40.0, 0.0),
                b: Point2::new(100.0, 0.0),
            },
        ];
        let a = dash_intervals(&one, &pattern, true);
        let b = dash_intervals(&two, &pattern, true);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x.s0 - y.s0).abs() < EPS);
            assert!((x.s1 - y.s1).abs() < EPS);
        }
    }

    #[test]
    fn resolution_independent_arc() {
        let pattern = [2.0, -1.0];
        let arc = PathSeg::Arc {
            center: Point2::new(0.0, 0.0),
            radius: 10.0,
            start: 0.0,
            sweep: std::f64::consts::PI,
        };
        let analytic = dash_intervals(&[arc], &pattern, true);
        let starts_8 = generate_path_dashes(&[arc], &pattern, true, 8);
        let starts_64 = generate_path_dashes(&[arc], &pattern, true, 64);
        let first_8: Vec<Point2> = dash_starts_from_pairs(&starts_8);
        let first_64: Vec<Point2> = dash_starts_from_pairs(&starts_64);
        assert_eq!(first_8.len(), first_64.len());
        for (a, b) in first_8.iter().zip(first_64.iter()) {
            assert!(a.distance(*b) < 1e-6, "dash start moved with tessellation");
        }
        assert!(!analytic.is_empty());
        assert!((analytic[0].s0).abs() < EPS);
    }

    fn dash_starts_from_pairs(pairs: &[(Point2, Point2)]) -> Vec<Point2> {
        let mut starts = Vec::new();
        let mut last_end: Option<Point2> = None;
        for (a, b) in pairs {
            let continues = last_end.map(|p| p.distance(*a) < 1e-7).unwrap_or(false);
            if !continues {
                starts.push(*a);
            }
            last_end = Some(*b);
        }
        starts
    }

    #[test]
    fn plinegen_off_resets_at_vertex() {
        let pattern = [12.0, -6.0];
        let segs = [
            PathSeg::Line {
                a: Point2::new(0.0, 0.0),
                b: Point2::new(50.0, 0.0),
            },
            PathSeg::Line {
                a: Point2::new(50.0, 0.0),
                b: Point2::new(50.0, 50.0),
            },
        ];
        let spans = dash_intervals(&segs, &pattern, false);
        assert!(spans.iter().any(|s| (s.s0 - 0.0).abs() < EPS));
        assert!(
            spans.iter().any(|s| (s.s0 - 50.0).abs() < EPS),
            "second vertex must start a dash when PLINEGEN is off: {spans:?}"
        );
    }

    #[test]
    fn plinegen_on_corner_continuous() {
        let pattern = [12.0, -6.0];
        let segs = [
            PathSeg::Line {
                a: Point2::new(0.0, 0.0),
                b: Point2::new(50.0, 0.0),
            },
            PathSeg::Line {
                a: Point2::new(50.0, 0.0),
                b: Point2::new(50.0, 50.0),
            },
        ];
        let one = dash_intervals(&[line(100.0)], &pattern, true);
        let two = dash_intervals(&segs, &pattern, true);
        assert_eq!(one.len(), two.len());
        for (a, b) in one.iter().zip(two.iter()) {
            assert!((a.s0 - b.s0).abs() < EPS);
        }
        assert!(
            !two.iter()
                .any(|s| (s.s0 - 50.0).abs() < EPS && (s.s1 - 50.0 - 12.0).abs() < EPS),
            "PLINEGEN on must not restart a 12-unit dash exactly at the corner"
        );
    }

    #[test]
    fn closed_polyline_plinegen_off() {
        let pattern = [10.0, -5.0];
        let segs = [
            PathSeg::Line {
                a: Point2::new(0.0, 0.0),
                b: Point2::new(40.0, 0.0),
            },
            PathSeg::Line {
                a: Point2::new(40.0, 0.0),
                b: Point2::new(40.0, 40.0),
            },
            PathSeg::Line {
                a: Point2::new(40.0, 40.0),
                b: Point2::new(0.0, 40.0),
            },
            PathSeg::Line {
                a: Point2::new(0.0, 40.0),
                b: Point2::new(0.0, 0.0),
            },
        ];
        let spans = dash_intervals(&segs, &pattern, false);
        for corner in [0.0, 40.0, 80.0, 120.0] {
            assert!(
                spans.iter().any(|s| (s.s0 - corner).abs() < EPS),
                "missing dash start at vertex s={corner}"
            );
        }
    }

    #[test]
    fn dot_pattern_finite() {
        let pattern = [0.0, -6.0];
        let pairs = generate_path_dashes(&[line(100.0)], &pattern, true, 8);
        assert!(!pairs.is_empty());
        assert!(
            pairs.len() < 200,
            "dot pattern must terminate, got {}",
            pairs.len()
        );
        let expected = (0..=16).map(|i| i as f64 * 6.0).collect::<Vec<_>>();
        let starts = dash_starts_from_pairs(&pairs);
        assert!(
            starts.len() >= 10,
            "expected many dots along 100 units, got {}",
            starts.len()
        );
        for s in expected.iter().take(starts.len()) {
            assert!(
                starts.iter().any(|p| (p.x - s).abs() < 0.5),
                "missing dot near s={s}, starts={starts:?}"
            );
        }
    }

    #[test]
    fn zero_only_pattern_guard() {
        let pattern = [0.0, 0.0, 0.0];
        let pairs = generate_path_dashes(&[line(50.0)], &pattern, true, 8);
        assert!(
            pairs.len() <= 4,
            "all-zero pattern must not loop, got {}",
            pairs.len()
        );
    }

    #[test]
    fn scale_chain_period() {
        let dashes = [12.0, -6.0];
        let scale = 2.0 * 0.5;
        let pattern = scaled_pattern(&dashes, scale);
        assert!((pattern_period(&pattern) - 18.0).abs() < EPS);
        let spans = dash_intervals(&[line(36.0)], &pattern, true);
        assert_eq!(spans.len(), 2);
        assert!((spans[0].s1 - spans[0].s0 - 12.0).abs() < EPS);
    }

    #[test]
    fn bulge_segment_uses_arc_length_not_chord() {
        let verts = [
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
        ];
        let segs = polyline_path_segs(
            &verts,
            false,
            Point3::new(0.0, 0.0, 1.0),
            Transform2::identity(),
        );
        assert_eq!(segs.len(), 1);
        let len = segs[0].length();
        assert!(
            (len - std::f64::consts::PI).abs() < 1e-6,
            "semicircle path length {len}, expected π"
        );
    }
}
