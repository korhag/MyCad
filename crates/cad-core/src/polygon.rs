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
