//! Consecutive mouse-wheel clicks zoom further; a pause or a reverse starts over.

use crate::settings::{
    sanitize_wheel_accel_strength, WheelAccelerationSettings, WHEEL_ACCEL_MAX_MULTIPLIER,
    WHEEL_ACCEL_STEP, WHEEL_ACCEL_WINDOW_SECS,
};

// ------------------------------------------------------------
// Type: WheelZoomAccelerator
// Purpose: Count same-direction wheel clicks that arrive close together
//          and turn that streak into a zoom multiplier. The first click
//          of a streak is always 1×. Time is passed in so tests do not
//          need a frame clock.
// ------------------------------------------------------------
#[derive(Debug, Clone, Default)]
pub struct WheelZoomAccelerator {
    streak: u32,
    last_sign: f32,
    last_click_time: Option<f64>,
}

impl WheelZoomAccelerator {
    // --------------------------------------------------------
    // Method: register_clicks
    // Purpose: Extend the streak when `count` clicks share `sign` and
    //          land inside the window. A pause or a direction change
    //          starts a new streak at `count`. Disabled acceleration
    //          forgets the streak so turning it back on starts slow.
    // --------------------------------------------------------
    pub fn register_clicks(
        &mut self,
        sign: f32,
        count: u32,
        now: f64,
        prefs: &WheelAccelerationSettings,
    ) {
        if !prefs.enabled {
            self.streak = 0;
            self.last_sign = 0.0;
            self.last_click_time = None;
            return;
        }
        if count == 0 || sign == 0.0 || !now.is_finite() {
            return;
        }
        let continues = self.last_click_time.is_some_and(|last| {
            self.streak > 0
                && self.last_sign == sign
                && now >= last
                && (now - last) <= WHEEL_ACCEL_WINDOW_SECS
        });
        self.streak = if continues {
            self.streak.saturating_add(count)
        } else {
            count
        };
        self.last_sign = sign;
        self.last_click_time = Some(now);
    }

    // --------------------------------------------------------
    // Method: multiplier
    // Purpose: Zoom scale for the current streak. 1× when acceleration
    //          is off, the streak is empty, or the last click is older
    //          than the window.
    // --------------------------------------------------------
    pub fn multiplier(&self, now: f64, prefs: &WheelAccelerationSettings) -> f64 {
        if !prefs.enabled || self.streak == 0 {
            return 1.0;
        }
        let Some(last) = self.last_click_time else {
            return 1.0;
        };
        if !now.is_finite() || now < last || (now - last) > WHEEL_ACCEL_WINDOW_SECS {
            return 1.0;
        }
        let strength = sanitize_wheel_accel_strength(prefs.strength);
        let steps = f64::from(self.streak - 1);
        let raw = 1.0 + WHEEL_ACCEL_STEP * strength * steps;
        raw.min(WHEEL_ACCEL_MAX_MULTIPLIER)
    }
}

// ------------------------------------------------------------
// Function: wheel_zoom_delta
// Purpose: Pick this frame's wheel delta. Smooth zoom uses egui's
//          eased delta, which spreads a notched click over about
//          0.1 s and keeps repainting until it lands. Off uses the
//          raw delta, so each click zooms at once.
// ------------------------------------------------------------
pub fn wheel_zoom_delta(smooth_zoom: bool, smooth_delta: f32, raw_delta: f32) -> f32 {
    if smooth_zoom {
        smooth_delta
    } else {
        raw_delta
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::DEFAULT_WHEEL_ACCEL_STRENGTH;
    use cad_core::Point2;
    use cad_viewport::Camera2;

    #[test]
    fn smooth_zoom_off_uses_the_raw_click() {
        assert_eq!(wheel_zoom_delta(true, 12.0, 50.0), 12.0);
        assert_eq!(wheel_zoom_delta(false, 12.0, 50.0), 50.0);
    }

    #[test]
    fn eased_steps_land_on_the_instant_zoom_and_stay_anchored() {
        let origin = Point2::new(0.0, 0.0);
        let size = Point2::new(800.0, 600.0);
        let cursor = Point2::new(620.0, 140.0);
        let start = Camera2 {
            center: Point2::new(5.0, -3.0),
            view_height: 80.0,
        };
        let anchor = start.screen_to_world(cursor, origin, size);
        let mut instant = start;
        instant.zoom_at(anchor, 4.0);
        let mut eased = start;
        for step in [0.4, 0.25, 0.2, 0.1, 0.05] {
            let world = eased.screen_to_world(cursor, origin, size);
            eased.zoom_at(world, 4.0_f64.powf(step));
            let held = eased.screen_to_world(cursor, origin, size);
            assert!((held.x - anchor.x).abs() < 1e-9 && (held.y - anchor.y).abs() < 1e-9);
        }
        assert!((eased.view_height - instant.view_height).abs() < 1e-9);
        assert!((eased.center.x - instant.center.x).abs() < 1e-9);
        assert!((eased.center.y - instant.center.y).abs() < 1e-9);
    }

    fn prefs_with_strength(strength: f64) -> WheelAccelerationSettings {
        WheelAccelerationSettings {
            strength,
            ..WheelAccelerationSettings::default()
        }
    }

    #[test]
    fn single_click_is_unit() {
        let mut accel = WheelZoomAccelerator::default();
        let prefs = WheelAccelerationSettings::default();
        accel.register_clicks(1.0, 1, 0.0, &prefs);
        assert!((accel.multiplier(0.0, &prefs) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn fast_clicks_ramp_up() {
        let mut accel = WheelZoomAccelerator::default();
        let prefs = WheelAccelerationSettings::default();
        let mut previous = 0.0;
        for step in 0..6 {
            let now = step as f64 * 0.05;
            accel.register_clicks(1.0, 1, now, &prefs);
            let multiplier = accel.multiplier(now, &prefs);
            assert!(
                multiplier > previous,
                "click {step} multiplier {multiplier} should exceed {previous}"
            );
            let expected = 1.0 + WHEEL_ACCEL_STEP * DEFAULT_WHEEL_ACCEL_STRENGTH * step as f64;
            assert!((multiplier - expected).abs() < 1e-12);
            previous = multiplier;
        }
    }

    #[test]
    fn multiplier_never_exceeds_cap() {
        let mut accel = WheelZoomAccelerator::default();
        let prefs = WheelAccelerationSettings::default();
        accel.register_clicks(1.0, 100, 1.0, &prefs);
        let multiplier = accel.multiplier(1.0, &prefs);
        assert!((multiplier - WHEEL_ACCEL_MAX_MULTIPLIER).abs() < 1e-12);
    }

    #[test]
    fn pause_beyond_window_resets_streak() {
        let mut accel = WheelZoomAccelerator::default();
        let prefs = WheelAccelerationSettings::default();
        accel.register_clicks(1.0, 5, 0.0, &prefs);
        assert!(accel.multiplier(0.1, &prefs) > 1.0);
        let expired = WHEEL_ACCEL_WINDOW_SECS + 0.01;
        assert!((accel.multiplier(expired, &prefs) - 1.0).abs() < 1e-12);
        accel.register_clicks(1.0, 1, expired, &prefs);
        assert!((accel.multiplier(expired, &prefs) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn direction_flip_resets_streak() {
        let mut accel = WheelZoomAccelerator::default();
        let prefs = WheelAccelerationSettings::default();
        accel.register_clicks(1.0, 4, 0.0, &prefs);
        accel.register_clicks(-1.0, 1, 0.05, &prefs);
        assert!((accel.multiplier(0.05, &prefs) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn disabled_acceleration_is_always_unit() {
        let mut accel = WheelZoomAccelerator::default();
        let prefs = WheelAccelerationSettings {
            enabled: false,
            ..WheelAccelerationSettings::default()
        };
        accel.register_clicks(1.0, 10, 0.0, &prefs);
        accel.register_clicks(1.0, 10, 0.05, &prefs);
        assert!((accel.multiplier(0.05, &prefs) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn higher_strength_ramps_faster() {
        let mut slow = WheelZoomAccelerator::default();
        let mut fast = WheelZoomAccelerator::default();
        let slow_prefs = prefs_with_strength(0.5);
        let fast_prefs = prefs_with_strength(2.0);
        for step in 0..4 {
            let now = step as f64 * 0.05;
            slow.register_clicks(1.0, 1, now, &slow_prefs);
            fast.register_clicks(1.0, 1, now, &fast_prefs);
        }
        let now = 3.0 * 0.05;
        let fast_multiplier = fast.multiplier(now, &fast_prefs);
        let slow_multiplier = slow.multiplier(now, &slow_prefs);
        assert!(
            fast_multiplier > slow_multiplier,
            "fast {fast_multiplier} slow {slow_multiplier}"
        );
    }
}
