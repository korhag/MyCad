//! Fill triangulation for concave polygons and even-odd holes.
//!
//! Contours are classified by how many other contours contain them.
//! Even depth is a filled outer ring; odd depth is a hole of the
//! smallest outer ring that contains it.

use cad_core::{contour_contains, Point2};

// ------------------------------------------------------------
// Function: triangulate_polygon
// Purpose: Triangulate one contour, including concave outlines.
// ------------------------------------------------------------
pub fn triangulate_polygon(pts: &[Point2]) -> Vec<[Point2; 3]> {
    triangulate_even_odd(&[pts.to_vec()])
}

// ------------------------------------------------------------
// Function: triangulate_even_odd
// Purpose: Fill the even-odd union of `contours`. Islands inside
//          holes become their own outer rings.
// ------------------------------------------------------------
pub fn triangulate_even_odd(contours: &[Vec<Point2>]) -> Vec<[Point2; 3]> {
    let rings: Vec<Vec<Point2>> = contours.iter().filter_map(|c| clean_ring(c)).collect();
    if rings.is_empty() {
        return Vec::new();
    }
    let depth: Vec<usize> = (0..rings.len())
        .map(|index| {
            rings
                .iter()
                .enumerate()
                .filter(|(other, ring)| *other != index && contour_contains(ring, &rings[index]))
                .count()
        })
        .collect();
    let mut triangles = Vec::new();
    for (index, ring) in rings.iter().enumerate() {
        if depth[index] % 2 != 0 {
            continue;
        }
        let mut holes = Vec::new();
        for (hole_index, hole) in rings.iter().enumerate() {
            if depth[hole_index] != depth[index] + 1 {
                continue;
            }
            if !contour_contains(ring, hole) {
                continue;
            }
            let covered_by_smaller = rings.iter().enumerate().any(|(other, candidate)| {
                other != index
                    && other != hole_index
                    && depth[other] == depth[index]
                    && contour_contains(candidate, hole)
                    && contour_contains(ring, candidate)
                    && ring_area(candidate).abs() < ring_area(ring).abs() - 1e-9
            });
            if !covered_by_smaller {
                holes.push(hole.clone());
            }
        }
        triangles.extend(earcut_group(ring, &holes));
    }
    triangles
}

fn clean_ring(pts: &[Point2]) -> Option<Vec<Point2>> {
    let mut ring: Vec<Point2> = pts.iter().copied().filter(|p| p.is_finite()).collect();
    if ring.len() >= 2 && ring[0].distance(*ring.last().unwrap()) < 1e-9 {
        ring.pop();
    }
    (ring.len() >= 3).then_some(ring)
}

fn ring_area(pts: &[Point2]) -> f64 {
    signed_area(pts)
}

fn signed_area(pts: &[Point2]) -> f64 {
    let mut area = 0.0;
    for i in 0..pts.len() {
        let a = pts[i];
        let b = pts[(i + 1) % pts.len()];
        area += a.x * b.y - b.x * a.y;
    }
    area * 0.5
}

fn with_winding(pts: &[Point2], ccw: bool) -> Vec<Point2> {
    let positive = signed_area(pts) >= 0.0;
    if positive == ccw {
        pts.to_vec()
    } else {
        let mut reversed = pts.to_vec();
        reversed.reverse();
        reversed
    }
}

fn earcut_group(outer: &[Point2], holes: &[Vec<Point2>]) -> Vec<[Point2; 3]> {
    let outer = with_winding(outer, true);
    let holes: Vec<Vec<Point2>> = holes.iter().map(|hole| with_winding(hole, false)).collect();
    let mut flat =
        Vec::with_capacity((outer.len() + holes.iter().map(Vec::len).sum::<usize>()) * 2);
    let mut hole_starts = Vec::with_capacity(holes.len());
    for p in &outer {
        flat.push(p.x);
        flat.push(p.y);
    }
    for hole in &holes {
        hole_starts.push(flat.len() / 2);
        for p in hole {
            flat.push(p.x);
            flat.push(p.y);
        }
    }
    let Ok(indices) = earcutr::earcut(&flat, &hole_starts, 2) else {
        return Vec::new();
    };
    let mut tris = Vec::with_capacity(indices.len() / 3);
    for tri in indices.chunks_exact(3) {
        let at = |index: usize| {
            let i = index * 2;
            Point2::new(flat[i], flat[i + 1])
        };
        let triangle = [at(tri[0]), at(tri[1]), at(tri[2])];
        if signed_area(&triangle).abs() > 1e-12 {
            tris.push(triangle);
        }
    }
    tris
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh_area(tris: &[[Point2; 3]]) -> f64 {
        tris.iter().map(|tri| signed_area(tri).abs()).sum()
    }

    fn p(x: f64, y: f64) -> Point2 {
        Point2::new(x, y)
    }

    #[test]
    fn concave_l_shape_keeps_its_area() {
        let l = vec![
            p(0.0, 0.0),
            p(2.0, 0.0),
            p(2.0, 1.0),
            p(1.0, 1.0),
            p(1.0, 2.0),
            p(0.0, 2.0),
        ];
        let tris = triangulate_polygon(&l);
        assert!(!tris.is_empty());
        assert!((mesh_area(&tris) - 3.0).abs() < 1e-6);
        let spill = tris
            .iter()
            .flatten()
            .any(|v| v.x > 2.0 + 1e-6 || v.y > 2.0 + 1e-6);
        assert!(!spill);
        let outside_notch = tris.iter().any(|tri| {
            let c = Point2::new(
                (tri[0].x + tri[1].x + tri[2].x) / 3.0,
                (tri[0].y + tri[1].y + tri[2].y) / 3.0,
            );
            c.x > 1.0 && c.y > 1.0
        });
        assert!(!outside_notch, "fan would fill the missing corner");
    }

    #[test]
    fn square_with_hole_subtracts_the_island() {
        let outer = vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)];
        let hole = vec![p(3.0, 3.0), p(7.0, 3.0), p(7.0, 7.0), p(3.0, 7.0)];
        let tris = triangulate_even_odd(&[outer, hole]);
        assert!((mesh_area(&tris) - 84.0).abs() < 1e-4);
    }
}
