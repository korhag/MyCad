//! Point-in-polygon tests shared by hatch style and fill triangulation.
//!
//! A ring contains another when every vertex is inside or on the boundary
//! and at least one is strictly inside. Touching outlines are not nested.

use crate::geom::Point2;

// ------------------------------------------------------------
// Function: point_in_polygon
// Purpose: Even-odd test. Points exactly on an edge are not inside.
// ------------------------------------------------------------
pub fn point_in_polygon(p: Point2, poly: &[Point2]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let pi = poly[i];
        let pj = poly[j];
        if ((pi.y > p.y) != (pj.y > p.y))
            && (p.x < (pj.x - pi.x) * (p.y - pi.y) / (pj.y - pi.y + 1e-30) + pi.x)
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

// ------------------------------------------------------------
// Function: contour_contains
// Purpose: True when `inner` sits in `outer`, allowing shared edges.
// ------------------------------------------------------------
pub fn contour_contains(outer: &[Point2], inner: &[Point2]) -> bool {
    let mut strictly_inside = 0usize;
    for point in inner {
        if point_in_polygon(*point, outer) {
            strictly_inside += 1;
        } else if !point_on_boundary(*point, outer) {
            return false;
        }
    }
    strictly_inside > 0
}

fn point_on_boundary(p: Point2, poly: &[Point2]) -> bool {
    if poly.is_empty() {
        return false;
    }
    (0..poly.len()).any(|i| point_near_segment(p, poly[i], poly[(i + 1) % poly.len()]))
}

fn point_near_segment(p: Point2, a: Point2, b: Point2) -> bool {
    let ab = b - a;
    let len2 = ab.x * ab.x + ab.y * ab.y;
    if len2 < 1e-18 {
        return p.distance(a) < 1e-8;
    }
    let t = ((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / len2;
    let t = t.clamp(0.0, 1.0);
    let proj = Point2::new(a.x + ab.x * t, a.y + ab.y * t);
    p.distance(proj) < 1e-7
}

// ------------------------------------------------------------
// Function: signed_area
// Purpose: Shoelace area of a ring. Positive is counter-clockwise.
//          The first point is joined back to the last.
// ------------------------------------------------------------
// ------------------------------------------------------------
// Function: clip_polygon
// Purpose: Sutherland–Hodgman. The clip ring is treated as convex
//          and counter-clockwise. A clockwise ring is reversed.
// ------------------------------------------------------------
pub fn clip_polygon(subject: &[Point2], clip: &[Point2]) -> Vec<Point2> {
    if subject.len() < 3 || clip.len() < 3 {
        return Vec::new();
    }
    let clip = ccw_ring(clip);
    let mut output = subject.to_vec();
    for index in 0..clip.len() {
        let edge_start = clip[index];
        let edge_end = clip[(index + 1) % clip.len()];
        let input = std::mem::take(&mut output);
        if input.is_empty() {
            break;
        }
        let mut previous = input[input.len() - 1];
        for &current in &input {
            let current_inside = left_of(edge_start, edge_end, current);
            let previous_inside = left_of(edge_start, edge_end, previous);
            if current_inside {
                if !previous_inside {
                    if let Some(hit) = segment_hit(previous, current, edge_start, edge_end) {
                        output.push(hit);
                    }
                }
                output.push(current);
            } else if previous_inside {
                if let Some(hit) = segment_hit(previous, current, edge_start, edge_end) {
                    output.push(hit);
                }
            }
            previous = current;
        }
    }
    output
}

pub fn intersect_polygons(first: &[Point2], second: &[Point2]) -> Vec<Point2> {
    clip_polygon(first, second)
}

// ------------------------------------------------------------
// Function: clip_open_polyline
// Purpose: Pieces of an open polyline that lie inside the clip.
// ------------------------------------------------------------
pub fn clip_open_polyline(points: &[Point2], clip: &[Point2]) -> Vec<Vec<Point2>> {
    if clip.len() < 3 || points.len() < 2 {
        return Vec::new();
    }
    let mut chains = Vec::new();
    let mut current = Vec::new();
    for pair in points.windows(2) {
        for (start, end) in clip_segment(pair[0], pair[1], clip) {
            if current
                .last()
                .is_some_and(|last: &Point2| last.distance(start) <= 1e-8)
            {
                current.push(end);
                continue;
            }
            if current.len() >= 2 {
                chains.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
            current.push(start);
            current.push(end);
        }
    }
    if current.len() >= 2 {
        chains.push(current);
    }
    chains
}

fn clip_segment(start: Point2, end: Point2, clip: &[Point2]) -> Vec<(Point2, Point2)> {
    let mut parameters = vec![0.0, 1.0];
    for index in 0..clip.len() {
        let edge_end = clip[(index + 1) % clip.len()];
        let Some(parameter) = segment_parameter(start, end, clip[index], edge_end) else {
            continue;
        };
        let Some(along_edge) = clip_edge_parameter(start, end, clip[index], edge_end) else {
            continue;
        };
        if (0.0..=1.0).contains(&parameter) && (0.0..=1.0).contains(&along_edge) {
            parameters.push(parameter);
        }
    }
    parameters.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    parameters.dedup_by(|left, right| (*left - *right).abs() < 1e-9);
    let mut spans = Vec::new();
    for pair in parameters.windows(2) {
        if pair[1] - pair[0] < 1e-9 {
            continue;
        }
        let mid = lerp(start, end, (pair[0] + pair[1]) * 0.5);
        if point_in_polygon(mid, clip) || point_on_boundary(mid, clip) {
            spans.push((lerp(start, end, pair[0]), lerp(start, end, pair[1])));
        }
    }
    spans
}

fn ccw_ring(points: &[Point2]) -> Vec<Point2> {
    if signed_area(points) < 0.0 {
        points.iter().rev().copied().collect()
    } else {
        points.to_vec()
    }
}

fn left_of(start: Point2, end: Point2, point: Point2) -> bool {
    let cross = (end.x - start.x) * (point.y - start.y) - (end.y - start.y) * (point.x - start.x);
    cross >= -1e-8
}

fn segment_hit(a: Point2, b: Point2, c: Point2, d: Point2) -> Option<Point2> {
    let parameter = segment_parameter(a, b, c, d)?;
    (0.0..=1.0)
        .contains(&parameter)
        .then(|| lerp(a, b, parameter))
}

fn segment_parameter(a: Point2, b: Point2, c: Point2, d: Point2) -> Option<f64> {
    let ab = b - a;
    let cd = d - c;
    let denom = ab.x * cd.y - ab.y * cd.x;
    if denom.abs() < 1e-12 {
        return None;
    }
    let ac = c - a;
    Some((ac.x * cd.y - ac.y * cd.x) / denom)
}

fn clip_edge_parameter(a: Point2, b: Point2, c: Point2, d: Point2) -> Option<f64> {
    let ab = b - a;
    let cd = d - c;
    let denom = ab.x * cd.y - ab.y * cd.x;
    if denom.abs() < 1e-12 {
        return None;
    }
    let ac = c - a;
    Some((ac.x * ab.y - ac.y * ab.x) / denom)
}

fn lerp(start: Point2, end: Point2, t: f64) -> Point2 {
    Point2::new(
        start.x + (end.x - start.x) * t,
        start.y + (end.y - start.y) * t,
    )
}

pub fn signed_area(pts: &[Point2]) -> f64 {
    let mut area = 0.0;
    for i in 0..pts.len() {
        let a = pts[i];
        let b = pts[(i + 1) % pts.len()];
        area += a.x * b.y - b.x * a.y;
    }
    area * 0.5
}

// ------------------------------------------------------------
// Function: contour_depths
// Purpose: How many other contours contain each contour. Even depth
//          is an outer ring; odd depth is a hole.
// ------------------------------------------------------------
pub fn contour_depths(contours: &[Vec<Point2>]) -> Vec<usize> {
    (0..contours.len())
        .map(|index| {
            contours
                .iter()
                .enumerate()
                .filter(|(other, ring)| *other != index && contour_contains(ring, &contours[index]))
                .count()
        })
        .collect()
}
