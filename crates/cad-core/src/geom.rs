//! Double-precision 2D/3D geometry primitives for the CAD document model.

use std::ops::{Add, Mul, Sub};

// ------------------------------------------------------------
// Type: Point2
// Purpose: World-space 2D coordinate pair stored as f64.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point2 {
    pub x: f64,
    pub y: f64,
}

impl Point2 {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn from_point3(p: Point3) -> Self {
        Self { x: p.x, y: p.y }
    }

    pub fn distance(self, other: Self) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }

    pub fn lerp(self, other: Self, t: f64) -> Self {
        Self {
            x: self.x + (other.x - self.x) * t,
            y: self.y + (other.y - self.y) * t,
        }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Add for Point2 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl Sub for Point2 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
        }
    }
}

impl Mul<f64> for Point2 {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self {
        Self {
            x: self.x * rhs,
            y: self.y * rhs,
        }
    }
}

// ------------------------------------------------------------
// Type: Point3
// Purpose: World-space 3D coordinate stored as f64. Z is kept
//          for OCS extrusion and 3D polylines; the viewport is 2D.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point3 {
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn from_xy(x: f64, y: f64) -> Self {
        Self { x, y, z: 0.0 }
    }

    pub fn xy(self) -> Point2 {
        Point2::from_point3(self)
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    pub fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    pub fn normalized(self) -> Self {
        let len = self.length();
        if len < 1e-15 {
            Self {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            }
        } else {
            Self {
                x: self.x / len,
                y: self.y / len,
                z: self.z / len,
            }
        }
    }

    pub fn cross(self, other: Self) -> Self {
        Self {
            x: self.y * other.z - self.z * other.y,
            y: self.z * other.x - self.x * other.z,
            z: self.x * other.y - self.y * other.x,
        }
    }

    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }
}

impl Add for Point3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
        }
    }
}

impl Sub for Point3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
        }
    }
}

impl Mul<f64> for Point3 {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self {
        Self {
            x: self.x * rhs,
            y: self.y * rhs,
            z: self.z * rhs,
        }
    }
}

// ------------------------------------------------------------
// Function: ocs_to_wcs
// Purpose: Convert an Object Coordinate System point to WCS
//          using AutoCAD's Arbitrary Axis Algorithm.
// ------------------------------------------------------------
pub fn ocs_to_wcs(point: Point3, extrusion: Point3) -> Point3 {
    let n = extrusion.normalized();
    let (ax, ay) = ocs_axes(n);
    ax * point.x + ay * point.y + n * point.z
}

pub fn ocs_axes(normal: Point3) -> (Point3, Point3) {
    let n = normal.normalized();
    let ax = if n.x.abs() < 1.0 / 64.0 && n.y.abs() < 1.0 / 64.0 {
        Point3::new(0.0, 1.0, 0.0).cross(n).normalized()
    } else {
        Point3::new(0.0, 0.0, 1.0).cross(n).normalized()
    };
    let ay = n.cross(ax).normalized();
    (ax, ay)
}

pub fn is_world_extrusion(extrusion: Point3) -> bool {
    extrusion.x.abs() < 1e-12 && extrusion.y.abs() < 1e-12 && extrusion.z > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_ocs_when_extrusion_is_z() {
        let p = Point3::new(10.0, 20.0, 0.0);
        let w = ocs_to_wcs(p, Point3::new(0.0, 0.0, 1.0));
        assert!((w.x - 10.0).abs() < 1e-12);
        assert!((w.y - 20.0).abs() < 1e-12);
    }

    #[test]
    fn ocs_flip_when_extrusion_is_negative_z() {
        let p = Point3::new(10.0, 20.0, 0.0);
        let w = ocs_to_wcs(p, Point3::new(0.0, 0.0, -1.0));
        assert!((w.x + 10.0).abs() < 1e-9);
        assert!((w.y - 20.0).abs() < 1e-9);
    }
}
