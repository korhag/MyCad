//! Axis-aligned world-space extents.

use crate::geom::Point2;

// ------------------------------------------------------------
// Type: Extents2
// Purpose: Inclusive 2D bounding box in double-precision world units.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extents2 {
    pub min: Point2,
    pub max: Point2,
}

impl Extents2 {
    pub fn empty() -> Self {
        Self {
            min: Point2::new(f64::INFINITY, f64::INFINITY),
            max: Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
        }
    }

    pub fn from_points(points: impl IntoIterator<Item = Point2>) -> Option<Self> {
        let mut e = Self::empty();
        let mut any = false;
        for p in points {
            if p.is_finite() {
                e.include(p);
                any = true;
            }
        }
        any.then_some(e)
    }

    pub fn from_corners(a: Point2, b: Point2) -> Self {
        Self {
            min: Point2::new(a.x.min(b.x), a.y.min(b.y)),
            max: Point2::new(a.x.max(b.x), a.y.max(b.y)),
        }
    }

    pub fn include(&mut self, p: Point2) {
        if !p.is_finite() {
            return;
        }
        self.min.x = self.min.x.min(p.x);
        self.min.y = self.min.y.min(p.y);
        self.max.x = self.max.x.max(p.x);
        self.max.y = self.max.y.max(p.y);
    }

    pub fn union(&mut self, other: Self) {
        self.include(other.min);
        self.include(other.max);
    }

    pub fn is_valid(self) -> bool {
        self.min.x.is_finite()
            && self.min.y.is_finite()
            && self.max.x.is_finite()
            && self.max.y.is_finite()
            && self.max.x >= self.min.x
            && self.max.y >= self.min.y
    }

    pub fn contains(self, p: Point2) -> bool {
        self.is_valid()
            && p.x >= self.min.x
            && p.x <= self.max.x
            && p.y >= self.min.y
            && p.y <= self.max.y
    }

    pub fn contains_extents(self, other: Self) -> bool {
        self.is_valid()
            && other.is_valid()
            && other.min.x >= self.min.x
            && other.max.x <= self.max.x
            && other.min.y >= self.min.y
            && other.max.y <= self.max.y
    }

    pub fn intersects(self, other: Self) -> bool {
        self.is_valid()
            && other.is_valid()
            && self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
    }

    pub fn inflated(self, pad: f64) -> Self {
        let pad = pad.max(0.0);
        Self {
            min: Point2::new(self.min.x - pad, self.min.y - pad),
            max: Point2::new(self.max.x + pad, self.max.y + pad),
        }
    }

    pub fn width(self) -> f64 {
        (self.max.x - self.min.x).max(0.0)
    }

    pub fn height(self) -> f64 {
        (self.max.y - self.min.y).max(0.0)
    }

    pub fn center(self) -> Point2 {
        Point2::new(
            (self.min.x + self.max.x) * 0.5,
            (self.min.y + self.max.y) * 0.5,
        )
    }

    pub fn padded(self, fraction: f64) -> Self {
        let dx = self.width().max(self.height()) * fraction;
        let dy = dx;
        Self {
            min: Point2::new(self.min.x - dx, self.min.y - dy),
            max: Point2::new(self.max.x + dx, self.max.y + dy),
        }
    }

    pub fn expanded_to_square_if_degenerate(self) -> Self {
        let mut w = self.width();
        let mut h = self.height();
        if w < 1e-9 && h < 1e-9 {
            w = 1.0;
            h = 1.0;
        } else if w < 1e-9 {
            w = h;
        } else if h < 1e-9 {
            h = w;
        }
        let c = self.center();
        Self {
            min: Point2::new(c.x - w * 0.5, c.y - h * 0.5),
            max: Point2::new(c.x + w * 0.5, c.y + h * 0.5),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extents_from_points() {
        let e = Extents2::from_points([
            Point2::new(-1.0, 2.0),
            Point2::new(3.0, -4.0),
            Point2::new(0.0, 0.0),
        ])
        .unwrap();
        assert_eq!(e.min, Point2::new(-1.0, -4.0));
        assert_eq!(e.max, Point2::new(3.0, 2.0));
        assert!((e.center().x - 1.0).abs() < 1e-12);
    }

    #[test]
    fn ignores_non_finite_points() {
        let e = Extents2::from_points([
            Point2::new(f64::NAN, 0.0),
            Point2::new(1.0, 1.0),
            Point2::new(f64::INFINITY, 2.0),
        ])
        .unwrap();
        assert_eq!(e.min, Point2::new(1.0, 1.0));
        assert_eq!(e.max, Point2::new(1.0, 1.0));
    }

    #[test]
    fn contains_extents_requires_full_inclusion() {
        let outer = Extents2::from_corners(Point2::new(0.0, 0.0), Point2::new(10.0, 10.0));
        let inner = Extents2::from_corners(Point2::new(1.0, 1.0), Point2::new(2.0, 2.0));
        let overlapping = Extents2::from_corners(Point2::new(8.0, 8.0), Point2::new(12.0, 12.0));
        assert!(outer.contains_extents(inner));
        assert!(!outer.contains_extents(overlapping));
        assert!(outer.intersects(overlapping));
        assert!(!inner.intersects(Extents2::from_corners(
            Point2::new(20.0, 20.0),
            Point2::new(21.0, 21.0)
        )));
    }
}
