//! Persistent user preferences loaded from eframe storage.

use serde::{Deserialize, Serialize};

pub const STORAGE_KEY: &str = "mycad_settings";
pub const DEFAULT_ZOOM_SPEED: f64 = 1.0;
pub const ZOOM_SPEED_MIN: f64 = 0.25;
pub const ZOOM_SPEED_MAX: f64 = 10.0;
pub const ZOOM_SCROLL_BASE: f64 = 1.001;

// ------------------------------------------------------------
// Type: AppSettings
// Purpose: User preferences that survive restart. New fields must
//          carry #[serde(default)] so older storage files still load.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub zoom_speed: f64,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            zoom_speed: DEFAULT_ZOOM_SPEED,
        }
    }
}

impl AppSettings {
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let mut settings: Self = storage
            .and_then(|s| eframe::get_value(s, STORAGE_KEY))
            .unwrap_or_default();
        settings.sanitize();
        settings
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        let mut settings = self.clone();
        settings.sanitize();
        eframe::set_value(storage, STORAGE_KEY, &settings);
    }

    pub fn sanitize(&mut self) {
        self.zoom_speed = sanitize_zoom_speed(self.zoom_speed);
    }

    pub fn reset_zoom_speed(&mut self) {
        self.zoom_speed = DEFAULT_ZOOM_SPEED;
    }
}

pub fn sanitize_zoom_speed(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(ZOOM_SPEED_MIN, ZOOM_SPEED_MAX)
    } else {
        DEFAULT_ZOOM_SPEED
    }
}

// ------------------------------------------------------------
// Function: scroll_to_zoom_factor
// Purpose: Map a smoothed wheel delta to a zoom factor. Speed scales
//          the exponent so 1.0× matches the original 1.001^scroll curve.
// ------------------------------------------------------------
pub fn scroll_to_zoom_factor(scroll_y: f64, zoom_speed: f64) -> f64 {
    ZOOM_SCROLL_BASE.powf(scroll_y * sanitize_zoom_speed(zoom_speed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_zoom_speed_is_one() {
        let settings = AppSettings::default();
        assert!((settings.zoom_speed - 1.0).abs() < 1e-15);
    }

    #[test]
    fn sanitize_clamps_and_replaces_non_finite() {
        assert!((sanitize_zoom_speed(0.1) - ZOOM_SPEED_MIN).abs() < 1e-15);
        assert!((sanitize_zoom_speed(10.0) - 10.0).abs() < 1e-15);
        assert!((sanitize_zoom_speed(20.0) - ZOOM_SPEED_MAX).abs() < 1e-15);
        assert!((sanitize_zoom_speed(f64::NAN) - DEFAULT_ZOOM_SPEED).abs() < 1e-15);
        assert!((sanitize_zoom_speed(f64::INFINITY) - DEFAULT_ZOOM_SPEED).abs() < 1e-15);
    }

    #[test]
    fn ron_round_trip_preserves_zoom_speed() {
        let mut settings = AppSettings {
            zoom_speed: 1.75,
        };
        settings.sanitize();
        let encoded = ron::to_string(&settings).expect("encode");
        let decoded: AppSettings = ron::from_str(&encoded).expect("decode");
        assert!((decoded.zoom_speed - 1.75).abs() < 1e-12);
    }

    #[test]
    fn apply_commits_draft_cancel_restores_applied() {
        let mut applied = AppSettings { zoom_speed: 1.5 };
        let mut draft = applied.clone();
        draft.zoom_speed = 10.0;
        draft.sanitize();
        let canceled = applied.clone();
        assert!((canceled.zoom_speed - 1.5).abs() < 1e-15);
        applied = draft;
        applied.sanitize();
        assert!((applied.zoom_speed - 10.0).abs() < 1e-15);
    }

    #[test]
    fn missing_fields_use_defaults() {
        let decoded: AppSettings = ron::from_str("()").expect("empty settings");
        assert!((decoded.zoom_speed - DEFAULT_ZOOM_SPEED).abs() < 1e-15);
    }

    #[test]
    fn unit_speed_matches_legacy_scroll_curve() {
        let scroll = 80.0;
        let expected = 1.001_f64.powf(scroll);
        let got = scroll_to_zoom_factor(scroll, 1.0);
        assert!((got - expected).abs() < 1e-12);
    }

    #[test]
    fn faster_speed_scales_the_exponent() {
        let scroll = 40.0;
        let slow = scroll_to_zoom_factor(scroll, 1.0);
        let fast = scroll_to_zoom_factor(scroll, 2.0);
        let equivalent = scroll_to_zoom_factor(scroll * 2.0, 1.0);
        assert!((fast - equivalent).abs() < 1e-12);
        assert!(fast > slow);
        let out_fast = scroll_to_zoom_factor(-scroll, 2.0);
        let out_slow = scroll_to_zoom_factor(-scroll, 1.0);
        assert!(out_fast < out_slow);
        assert!((fast * out_fast - 1.0).abs() < 1e-12);
    }

    #[test]
    fn zoom_factors_compose_multiplicatively() {
        let speed = 1.5;
        let a = scroll_to_zoom_factor(25.0, speed);
        let b = scroll_to_zoom_factor(35.0, speed);
        let both = scroll_to_zoom_factor(60.0, speed);
        assert!((a * b - both).abs() < 1e-12);
    }
}
