//! Tracks when the view last moved, so pan and zoom draw at one sample
//! and the picture settles to MSAA once the camera stops.

use cad_viewport::Camera2;

/// How long the camera must stay still before the picture settles to MSAA.
pub const MOTION_SETTLE_SECS: f64 = 0.12;

// ------------------------------------------------------------
// Type: ViewMotion
// Purpose: Remember the last camera and viewport size and when they
//          changed. Time is passed in so tests do not need a frame clock.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, Default)]
pub struct ViewMotion {
    key: [u64; 5],
    moved_at: Option<f64>,
}

impl ViewMotion {
    // --------------------------------------------------------
    // Method: update
    // Purpose: Record this frame's view. Returns the seconds left until
    //          the picture settles, or None when the view is still.
    // --------------------------------------------------------
    pub fn update(&mut self, camera: &Camera2, viewport_px: [u32; 2], now: f64) -> Option<f64> {
        let key = [
            camera.center.x.to_bits(),
            camera.center.y.to_bits(),
            camera.view_height.to_bits(),
            u64::from(viewport_px[0]),
            u64::from(viewport_px[1]),
        ];
        if key != self.key {
            self.key = key;
            self.moved_at = Some(now);
        }
        let elapsed = now - self.moved_at?;
        let remaining = MOTION_SETTLE_SECS - elapsed;
        (remaining > 0.0 && elapsed >= 0.0).then_some(remaining)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_core::Point2;

    fn camera(x: f64) -> Camera2 {
        Camera2 {
            center: Point2::new(x, 0.0),
            view_height: 100.0,
        }
    }

    #[test]
    fn a_moving_camera_is_interactive_until_it_settles() {
        let mut motion = ViewMotion::default();
        assert!(motion.update(&camera(0.0), [800, 600], 1.0).is_some());
        assert!(motion.update(&camera(1.0), [800, 600], 1.05).is_some());
        let left = motion
            .update(&camera(1.0), [800, 600], 1.1)
            .expect("still settling");
        assert!((left - (MOTION_SETTLE_SECS - 0.05)).abs() < 1e-9);
        assert!(motion
            .update(&camera(1.0), [800, 600], 1.05 + MOTION_SETTLE_SECS + 1e-6)
            .is_none());
    }

    #[test]
    fn resizing_the_viewport_counts_as_motion() {
        let mut motion = ViewMotion::default();
        motion.update(&camera(0.0), [800, 600], 0.0);
        assert!(motion.update(&camera(0.0), [800, 600], 1.0).is_none());
        assert!(motion.update(&camera(0.0), [820, 600], 1.0).is_some());
    }
}
