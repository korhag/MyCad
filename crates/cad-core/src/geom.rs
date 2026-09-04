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

pub const GEOM_TOLERANCE: f64 = 1e-12;

// ------------------------------------------------------------
// Type: ThreePointArc
// Purpose: Circle parameters for an arc that starts at A, passes
//          through B, and ends at C. Angles are ordered so a
//          counterclockwise sweep from start_angle to end_angle
//          traces that curve (clockwise input swaps the stored
//          endpoints so tessellation can stay CCW).
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreePointArc {
    pub center: Point2,
    pub radius: f64,
    pub start_angle: f64,
    pub end_angle: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArcFromPointsError {
    DuplicatePoints,
    Collinear,
}

impl ArcFromPointsError {
    pub fn message(self) -> &'static str {
        match self {
            Self::DuplicatePoints => "Points are too close to define an arc",
            Self::Collinear => "Points are collinear; cannot create an arc",
        }
    }
}

// ------------------------------------------------------------
// Function: arc_from_three_points
// Purpose: Fit a circular arc through three world-XY points using
//          double-precision circumcenter math.
// ------------------------------------------------------------
pub fn arc_from_three_points(
    start: Point2,
    mid: Point2,
    end: Point2,
) -> Result<ThreePointArc, ArcFromPointsError> {
    if !start.is_finite() || !mid.is_finite() || !end.is_finite() {
        return Err(ArcFromPointsError::DuplicatePoints);
    }
    let ab = start.distance(mid);
    let bc = mid.distance(end);
    let ac = start.distance(end);
    if ab <= GEOM_TOLERANCE || bc <= GEOM_TOLERANCE || ac <= GEOM_TOLERANCE {
        return Err(ArcFromPointsError::DuplicatePoints);
    }
    let scale = ab.max(bc).max(ac);
    let cross = (mid.x - start.x) * (end.y - start.y) - (mid.y - start.y) * (end.x - start.x);
    if cross.abs() <= GEOM_TOLERANCE * scale * scale {
        return Err(ArcFromPointsError::Collinear);
    }
    let d =
        2.0 * (start.x * (mid.y - end.y) + mid.x * (end.y - start.y) + end.x * (start.y - mid.y));
    if !d.is_finite() || d.abs() <= GEOM_TOLERANCE * scale * scale {
        return Err(ArcFromPointsError::Collinear);
    }
    let start_sq = start.x * start.x + start.y * start.y;
    let mid_sq = mid.x * mid.x + mid.y * mid.y;
    let end_sq = end.x * end.x + end.y * end.y;
    let center = Point2::new(
        (start_sq * (mid.y - end.y) + mid_sq * (end.y - start.y) + end_sq * (start.y - mid.y)) / d,
        (start_sq * (end.x - mid.x) + mid_sq * (start.x - end.x) + end_sq * (mid.x - start.x)) / d,
    );
    if !center.is_finite() {
        return Err(ArcFromPointsError::Collinear);
    }
    let radius = center.distance(start);
    if !radius.is_finite() || radius <= GEOM_TOLERANCE {
        return Err(ArcFromPointsError::Collinear);
    }
    let start_angle = (start.y - center.y).atan2(start.x - center.x);
    let mid_angle = (mid.y - center.y).atan2(mid.x - center.x);
    let end_angle = (end.y - center.y).atan2(end.x - center.x);
    let (start_angle, end_angle) = if ccw_contains(start_angle, end_angle, mid_angle) {
        (start_angle, end_angle)
    } else {
        (end_angle, start_angle)
    };
    Ok(ThreePointArc {
        center,
        radius,
        start_angle,
        end_angle,
    })
}

fn wrap_tau(angle: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let wrapped = angle % tau;
    if wrapped < 0.0 {
        wrapped + tau
    } else {
        wrapped
    }
}

fn ccw_delta(from: f64, to: f64) -> f64 {
    wrap_tau(to - from)
}

fn ccw_contains(start: f64, end: f64, mid: f64) -> bool {
    let span = ccw_delta(start, end);
    let to_mid = ccw_delta(start, mid);
    to_mid <= span + 1e-10
}

impl ThreePointArc {
    pub fn point_at_angle(self, angle: f64) -> Point2 {
        Point2::new(
            self.center.x + self.radius * angle.cos(),
            self.center.y + self.radius * angle.sin(),
        )
    }

    pub fn contains_point(self, point: Point2, tolerance: f64) -> bool {
        if (point.distance(self.center) - self.radius).abs() > tolerance.max(GEOM_TOLERANCE) {
            return false;
        }
        let angle = (point.y - self.center.y).atan2(point.x - self.center.x);
        ccw_contains(self.start_angle, self.end_angle, angle)
    }
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

    #[test]
    fn three_point_arc_passes_through_ccw_and_cw_midpoints() {
        let start = Point2::new(1.0, 0.0);
        let ccw_mid = Point2::new(0.0, 1.0);
        let cw_mid = Point2::new(0.0, -1.0);
        let end = Point2::new(-1.0, 0.0);
        let ccw = arc_from_three_points(start, ccw_mid, end).expect("ccw arc");
        let cw = arc_from_three_points(start, cw_mid, end).expect("cw arc");
        assert!((ccw.center.x).abs() < 1e-9);
        assert!((ccw.center.y).abs() < 1e-9);
        assert!((ccw.radius - 1.0).abs() < 1e-9);
        assert!((cw.center.x).abs() < 1e-9);
        assert!((cw.center.y).abs() < 1e-9);
        assert!((cw.radius - 1.0).abs() < 1e-9);
        assert!(ccw.contains_point(start, 1e-8));
        assert!(ccw.contains_point(ccw_mid, 1e-8));
        assert!(ccw.contains_point(end, 1e-8));
        assert!(!ccw.contains_point(cw_mid, 1e-8));
        assert!(cw.contains_point(start, 1e-8));
        assert!(cw.contains_point(cw_mid, 1e-8));
        assert!(cw.contains_point(end, 1e-8));
        assert!(!cw.contains_point(ccw_mid, 1e-8));
    }

    #[test]
    fn three_point_arc_rejects_duplicates_and_collinear_points() {
        let a = Point2::new(0.0, 0.0);
        let b = Point2::new(1.0, 0.0);
        let c = Point2::new(2.0, 0.0);
        assert_eq!(
            arc_from_three_points(a, a, b),
            Err(ArcFromPointsError::DuplicatePoints)
        );
        assert_eq!(
            arc_from_three_points(a, b, c),
            Err(ArcFromPointsError::Collinear)
        );
    }
}
