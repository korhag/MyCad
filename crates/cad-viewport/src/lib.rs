//! Double-precision 2D camera: pan, zoom-around-cursor, zoom extents.

use cad_core::{Extents2, Point2};

// ------------------------------------------------------------
// Type: Camera2
// Purpose: Maps world XY to a pixel viewport while preserving aspect.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy)]
pub struct Camera2 {
    pub center: Point2,
    /// World units visible along the viewport height.
    pub view_height: f64,
}

impl Default for Camera2 {
    fn default() -> Self {
        Self {
            center: Point2::new(0.0, 0.0),
            view_height: 100.0,
        }
    }
}

impl Camera2 {
    pub fn zoom_extents(&mut self, extents: Extents2, viewport_width: f64, viewport_height: f64) {
        let extents = extents.padded(0.05).expanded_to_square_if_degenerate();
        let aspect = if viewport_height.abs() < 1e-9 {
            1.0
        } else {
            viewport_width / viewport_height
        };
        let world_w = extents.width();
        let world_h = extents.height();
        let fitted_h = if world_w / aspect > world_h {
            world_w / aspect
        } else {
            world_h
        };
        self.center = extents.center();
        self.view_height = fitted_h.max(1e-9);
    }

    pub fn view_width(&self, aspect: f64) -> f64 {
        self.view_height * aspect.max(1e-9)
    }

    pub fn pixels_per_world(&self, viewport_height: f64) -> f64 {
        viewport_height / self.view_height.max(1e-15)
    }

    pub fn screen_to_world(
        &self,
        screen: Point2,
        viewport_origin: Point2,
        viewport_size: Point2,
    ) -> Point2 {
        let aspect = viewport_size.x / viewport_size.y.max(1e-15);
        let nx = (screen.x - viewport_origin.x) / viewport_size.x.max(1e-15);
        let ny = (screen.y - viewport_origin.y) / viewport_size.y.max(1e-15);
        // Screen Y grows downward; CAD Y grows upward.
        Point2::new(
            self.center.x + (nx - 0.5) * self.view_width(aspect),
            self.center.y - (ny - 0.5) * self.view_height,
        )
    }

    pub fn world_to_screen(
        &self,
        world: Point2,
        viewport_origin: Point2,
        viewport_size: Point2,
    ) -> Point2 {
        let aspect = viewport_size.x / viewport_size.y.max(1e-15);
        let nx = 0.5 + (world.x - self.center.x) / self.view_width(aspect);
        let ny = 0.5 - (world.y - self.center.y) / self.view_height;
        Point2::new(
            viewport_origin.x + nx * viewport_size.x,
            viewport_origin.y + ny * viewport_size.y,
        )
    }

    /// Zoom by `factor` (>1 zooms in) keeping `cursor_world` fixed.
    pub fn zoom_at(&mut self, cursor_world: Point2, factor: f64) {
        let factor = factor.clamp(1e-6, 1e6);
        let new_height = (self.view_height / factor).clamp(1e-12, 1e16);
        let t = 1.0 - (new_height / self.view_height);
        self.center = Point2::new(
            self.center.x + (cursor_world.x - self.center.x) * t,
            self.center.y + (cursor_world.y - self.center.y) * t,
        );
        self.view_height = new_height;
    }

    pub fn pan_world(&mut self, delta: Point2) {
        self.center = Point2::new(self.center.x - delta.x, self.center.y - delta.y);
    }

    pub fn pan_screen(
        &self,
        start: Point2,
        end: Point2,
        viewport_origin: Point2,
        viewport_size: Point2,
    ) -> Point2 {
        let a = self.screen_to_world(start, viewport_origin, viewport_size);
        let b = self.screen_to_world(end, viewport_origin, viewport_size);
        Point2::new(b.x - a.x, b.y - a.y)
    }

    /// Orthographic matrix mapping world (minus `origin`) into NDC, Y-up.
    pub fn view_proj_f32(&self, origin: Point2, aspect: f64) -> [[f32; 4]; 4] {
        let half_h = (self.view_height * 0.5).max(1e-15);
        let half_w = half_h * aspect.max(1e-15);
        let cx = (self.center.x - origin.x) as f32;
        let cy = (self.center.y - origin.y) as f32;
        let sx = (1.0 / half_w) as f32;
        let sy = (1.0 / half_h) as f32;
        [
            [sx, 0.0, 0.0, 0.0],
            [0.0, sy, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [-cx * sx, -cy * sy, 0.0, 1.0],
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp() -> (Point2, Point2) {
        (Point2::new(0.0, 0.0), Point2::new(800.0, 600.0))
    }

    #[test]
    fn zoom_extents_fits_drawing_and_preserves_aspect() {
        let mut cam = Camera2::default();
        let extents = Extents2 {
            min: Point2::new(0.0, 0.0),
            max: Point2::new(200.0, 100.0),
        };
        cam.zoom_extents(extents, 800.0, 600.0);
        let (origin, size) = vp();
        let min_s = cam.world_to_screen(extents.min, origin, size);
        let max_s = cam.world_to_screen(extents.max, origin, size);
        assert!(min_s.x >= -1.0 && max_s.x <= 801.0);
        assert!(min_s.y <= 601.0 && max_s.y >= -1.0);
        let aspect_screen = (max_s.x - min_s.x).abs() / (max_s.y - min_s.y).abs();
        let aspect_world = extents.width() / extents.height();
        assert!((aspect_screen - aspect_world).abs() < 0.05);
    }

    #[test]
    fn zoom_around_cursor_keeps_world_point_stable() {
        let mut cam = Camera2 {
            center: Point2::new(10.0, 20.0),
            view_height: 100.0,
        };
        let (origin, size) = vp();
        let cursor_screen = Point2::new(200.0, 150.0);
        let world_before = cam.screen_to_world(cursor_screen, origin, size);
        cam.zoom_at(world_before, 2.0);
        let world_after = cam.screen_to_world(cursor_screen, origin, size);
        assert!((world_before.x - world_after.x).abs() < 1e-9);
        assert!((world_before.y - world_after.y).abs() < 1e-9);
        assert!((cam.view_height - 50.0).abs() < 1e-9);
    }

    #[test]
    fn pan_moves_center_opposite_to_pointer() {
        let mut cam = Camera2 {
            center: Point2::new(0.0, 0.0),
            view_height: 100.0,
        };
        let (origin, size) = vp();
        let delta = cam.pan_screen(Point2::new(0.0, 0.0), Point2::new(80.0, 0.0), origin, size);
        cam.pan_world(delta);
        assert!(cam.center.x < 0.0);
        assert!(cam.center.y.abs() < 1e-9);
    }

    #[test]
    fn screen_roundtrip() {
        let cam = Camera2 {
            center: Point2::new(12.5, -7.0),
            view_height: 40.0,
        };
        let (origin, size) = vp();
        let world = Point2::new(15.0, -3.0);
        let screen = cam.world_to_screen(world, origin, size);
        let back = cam.screen_to_world(screen, origin, size);
        assert!((world.x - back.x).abs() < 1e-9);
        assert!((world.y - back.y).abs() < 1e-9);
    }
}
