//! Affine 2D transforms for INSERT nesting (scale, rotate, translate).

use crate::geom::{is_world_extrusion, ocs_axes, Point2, Point3};

// ------------------------------------------------------------
// Type: Transform2
// Purpose: Column-major 2D affine matrix operating on world XY.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform2 {
    pub m00: f64,
    pub m01: f64,
    pub m10: f64,
    pub m11: f64,
    pub tx: f64,
    pub ty: f64,
}

impl Transform2 {
    pub fn identity() -> Self {
        Self {
            m00: 1.0,
            m01: 0.0,
            m10: 0.0,
            m11: 1.0,
            tx: 0.0,
            ty: 0.0,
        }
    }

    pub fn translate(tx: f64, ty: f64) -> Self {
        Self {
            m00: 1.0,
            m01: 0.0,
            m10: 0.0,
            m11: 1.0,
            tx,
            ty,
        }
    }

    pub fn scale(sx: f64, sy: f64) -> Self {
        Self {
            m00: sx,
            m01: 0.0,
            m10: 0.0,
            m11: sy,
            tx: 0.0,
            ty: 0.0,
        }
    }

    pub fn rotate(radians: f64) -> Self {
        let c = radians.cos();
        let s = radians.sin();
        Self {
            m00: c,
            m01: -s,
            m10: s,
            m11: c,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// INSERT transform: scale, then rotate, then translate (AutoCAD order).
    pub fn insert(insertion: Point3, scale: Point3, rotation: f64) -> Self {
        Self::translate(insertion.x, insertion.y)
            .then(Self::rotate(rotation))
            .then(Self::scale(scale.x, scale.y))
    }

    /// Linear part of AutoCAD OCS → WCS for an extrusion (Z dropped for 2D).
    pub fn ocs_linear(extrusion: Point3) -> Self {
        if is_world_extrusion(extrusion) {
            return Self::identity();
        }
        let (ax, ay) = ocs_axes(extrusion);
        Self {
            m00: ax.x,
            m01: ay.x,
            m10: ax.y,
            m11: ay.y,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// Full AutoCAD INSERT: OCS(extrusion) · T(ins) · R · S · T(-base).
    pub fn block_insert(
        insertion: Point3,
        scale: Point3,
        rotation: f64,
        extrusion: Point3,
        base: Point3,
    ) -> Self {
        Self::ocs_linear(extrusion)
            .then(Self::insert(insertion, scale, rotation))
            .then(Self::translate(-base.x, -base.y))
    }

    pub fn then(self, inner: Self) -> Self {
        Self {
            m00: self.m00 * inner.m00 + self.m01 * inner.m10,
            m01: self.m00 * inner.m01 + self.m01 * inner.m11,
            m10: self.m10 * inner.m00 + self.m11 * inner.m10,
            m11: self.m10 * inner.m01 + self.m11 * inner.m11,
            tx: self.m00 * inner.tx + self.m01 * inner.ty + self.tx,
            ty: self.m10 * inner.tx + self.m11 * inner.ty + self.ty,
        }
    }

    pub fn apply(self, p: Point2) -> Point2 {
        Point2 {
            x: self.m00 * p.x + self.m01 * p.y + self.tx,
            y: self.m10 * p.x + self.m11 * p.y + self.ty,
        }
    }

    pub fn apply3(self, p: Point3) -> Point3 {
        let q = self.apply(p.xy());
        Point3 {
            x: q.x,
            y: q.y,
            z: p.z,
        }
    }

    pub fn rotation_component(self) -> f64 {
        self.m10.atan2(self.m00)
    }

    pub fn scale_x(self) -> f64 {
        (self.m00 * self.m00 + self.m10 * self.m10).sqrt()
    }

    pub fn scale_y(self) -> f64 {
        (self.m01 * self.m01 + self.m11 * self.m11).sqrt()
    }
}

impl Default for Transform2 {
    fn default() -> Self {
        Self::identity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_transform_moves_origin_to_insertion() {
        let t = Transform2::insert(
            Point3::from_xy(100.0, 50.0),
            Point3::new(2.0, 2.0, 1.0),
            0.0,
        );
        let p = t.apply(Point2::new(1.0, 0.0));
        assert!((p.x - 102.0).abs() < 1e-12);
        assert!((p.y - 50.0).abs() < 1e-12);
    }

    #[test]
    fn rotate_90_degrees_ccw() {
        let t = Transform2::rotate(std::f64::consts::FRAC_PI_2);
        let p = t.apply(Point2::new(1.0, 0.0));
        assert!(p.x.abs() < 1e-12);
        assert!((p.y - 1.0).abs() < 1e-12);
    }

    #[test]
    fn nested_transforms_compose() {
        let inner = Transform2::insert(Point3::from_xy(10.0, 0.0), Point3::new(1.0, 1.0, 1.0), 0.0);
        let outer = Transform2::insert(Point3::from_xy(5.0, 5.0), Point3::new(1.0, 1.0, 1.0), 0.0);
        let p = outer.then(inner).apply(Point2::new(1.0, 0.0));
        assert!((p.x - 16.0).abs() < 1e-12);
        assert!((p.y - 5.0).abs() < 1e-12);
    }

    #[test]
    fn block_insert_subtracts_base_point() {
        let t = Transform2::block_insert(
            Point3::from_xy(100.0, 50.0),
            Point3::new(2.0, 2.0, 1.0),
            0.0,
            Point3::new(0.0, 0.0, 1.0),
            Point3::from_xy(10.0, 5.0),
        );
        let p = t.apply(Point2::new(10.0, 5.0));
        assert!((p.x - 100.0).abs() < 1e-12);
        assert!((p.y - 50.0).abs() < 1e-12);
        let q = t.apply(Point2::new(11.0, 5.0));
        assert!((q.x - 102.0).abs() < 1e-12);
        assert!((q.y - 50.0).abs() < 1e-12);
    }

    #[test]
    fn block_insert_negative_x_scale_mirrors() {
        let t = Transform2::block_insert(
            Point3::from_xy(0.0, 0.0),
            Point3::new(-1.0, 1.0, 1.0),
            0.0,
            Point3::new(0.0, 0.0, 1.0),
            Point3::from_xy(0.0, 0.0),
        );
        let p = t.apply(Point2::new(5.0, 3.0));
        assert!((p.x + 5.0).abs() < 1e-12);
        assert!((p.y - 3.0).abs() < 1e-12);
    }

    #[test]
    fn block_insert_negative_z_extrusion_mirrors_x() {
        let t = Transform2::block_insert(
            Point3::from_xy(100.0, 0.0),
            Point3::new(1.0, 1.0, 1.0),
            0.0,
            Point3::new(0.0, 0.0, -1.0),
            Point3::from_xy(0.0, 0.0),
        );
        let p = t.apply(Point2::new(10.0, 4.0));
        assert!((p.x + 110.0).abs() < 1e-9);
        assert!((p.y - 4.0).abs() < 1e-9);
    }
}
