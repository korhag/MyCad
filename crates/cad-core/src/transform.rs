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

    pub fn apply_vector(self, v: Point2) -> Point2 {
        Point2 {
            x: self.m00 * v.x + self.m01 * v.y,
            y: self.m10 * v.x + self.m11 * v.y,
        }
    }

    pub fn determinant(self) -> f64 {
        self.m00 * self.m11 - self.m01 * self.m10
    }

    pub fn reverses_orientation(self) -> bool {
        self.determinant() < 0.0
    }

    /// Map a world-space affine into DisplayList local coordinates
    /// (`vertex = world - origin`): `T(-origin) · world · T(origin)`.
    pub fn to_local_origin(self, origin: Point2) -> Self {
        Self::translate(-origin.x, -origin.y)
            .then(self)
            .then(Self::translate(origin.x, origin.y))
    }

    /// Column-major mat4x4 matching `Camera2::view_proj_f32`.
    pub fn to_mat4(self) -> [[f32; 4]; 4] {
        [
            [self.m00 as f32, self.m10 as f32, 0.0, 0.0],
            [self.m01 as f32, self.m11 as f32, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [self.tx as f32, self.ty as f32, 0.0, 1.0],
        ]
    }

    pub fn identity_mat4() -> [[f32; 4]; 4] {
        Self::identity().to_mat4()
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

    pub fn is_uniform_scale(self) -> bool {
        let sx = self.scale_x();
        let sy = self.scale_y();
        (sx - sy).abs() <= crate::geom::GEOM_TOLERANCE * sx.max(sy).max(1.0)
    }

    pub fn uniform_scale(self) -> Option<f64> {
        self.is_uniform_scale().then_some(self.scale_x())
    }

    pub fn try_inverse(self) -> Option<Self> {
        let det = self.m00 * self.m11 - self.m01 * self.m10;
        if !det.is_finite() || det.abs() <= crate::geom::GEOM_TOLERANCE {
            return None;
        }
        Some(Self {
            m00: self.m11 / det,
            m01: -self.m01 / det,
            m10: -self.m10 / det,
            m11: self.m00 / det,
            tx: (self.m01 * self.ty - self.m11 * self.tx) / det,
            ty: (self.m10 * self.tx - self.m00 * self.ty) / det,
        })
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

    #[test]
    fn try_inverse_roundtrips_and_uniform_scale_detects_mirroring() {
        let t = Transform2::insert(
            Point3::from_xy(10.0, -4.0),
            Point3::new(2.0, 3.0, 1.0),
            0.4,
        );
        let inv = t.try_inverse().expect("invertible");
        let p = Point2::new(7.0, 1.5);
        let back = inv.apply(t.apply(p));
        assert!((back.x - p.x).abs() < 1e-9);
        assert!((back.y - p.y).abs() < 1e-9);
        assert!(Transform2::scale(-1.0, 1.0).is_uniform_scale());
        assert!(!Transform2::scale(2.0, 1.0).is_uniform_scale());
    }

    #[test]
    fn then_applies_the_inner_transform_first() {
        let translated = Transform2::translate(10.0, 0.0)
            .then(Transform2::rotate(std::f64::consts::FRAC_PI_2))
            .apply(Point2::new(1.0, 0.0));
        assert!(translated.x.abs() < 1e-12);
        assert!((translated.y - 1.0).abs() < 1e-12);
        let rotated_then_moved = Transform2::rotate(std::f64::consts::FRAC_PI_2)
            .then(Transform2::translate(10.0, 0.0))
            .apply(Point2::new(1.0, 0.0));
        assert!((rotated_then_moved.x + 10.0).abs() < 1e-12);
        assert!((rotated_then_moved.y - 1.0).abs() < 1e-12);
    }

    #[test]
    fn local_origin_preview_does_not_drift_at_large_coordinates() {
        let origin = Point2::new(1.0e8, 2.0e8);
        let world = Transform2::translate(10.0, -4.0);
        let local = world.to_local_origin(origin);
        let stored = Point2::new(3.0, 5.0);
        let preview = local.apply(stored);
        assert!((preview.x - 13.0).abs() < 1e-9);
        assert!((preview.y - 1.0).abs() < 1e-9);
    }
}
