//! Persistent user preferences, including portable JSON import/export.

use serde::{Deserialize, Serialize};

use crate::input::InputMap;
use crate::workspace::{
    decode_dock_layout, default_dock_state, encode_dock_layout, sanitize_dock_state, WorkspaceTab,
};

pub const STORAGE_KEY: &str = "mycad_settings";
pub const SETTINGS_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_ZOOM_SPEED: f64 = 1.0;
pub const ZOOM_SPEED_MIN: f64 = 0.25;
pub const ZOOM_SPEED_MAX: f64 = 10.0;
pub const ZOOM_SCROLL_BASE: f64 = 1.001;
pub const BOX_FILL_ALPHA: u8 = 40;

// ------------------------------------------------------------
// Type: RgbColor
// Purpose: Serializable UI color for display settings.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RgbColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn to_color32(self) -> eframe::egui::Color32 {
        eframe::egui::Color32::from_rgb(self.r, self.g, self.b)
    }

    pub fn to_fill(self) -> eframe::egui::Color32 {
        eframe::egui::Color32::from_rgba_unmultiplied(self.r, self.g, self.b, BOX_FILL_ALPHA)
    }
}

impl Default for RgbColor {
    fn default() -> Self {
        Self::WINDOW
    }
}

impl RgbColor {
    pub const WINDOW: Self = Self::new(64, 128, 255);
    pub const CROSSING: Self = Self::new(64, 200, 96);
}

// ------------------------------------------------------------
// Type: DisplaySettings
// Purpose: Viewport display prefs, including box-selection chrome.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplaySettings {
    pub window_selection: RgbColor,
    pub crossing_selection: RgbColor,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            window_selection: RgbColor::WINDOW,
            crossing_selection: RgbColor::CROSSING,
        }
    }
}

impl DisplaySettings {
    pub fn reset_window(&mut self) {
        self.window_selection = RgbColor::WINDOW;
    }

    pub fn reset_crossing(&mut self) {
        self.crossing_selection = RgbColor::CROSSING;
    }

    pub fn reset_all(&mut self) {
        *self = Self::default();
    }
}

// ------------------------------------------------------------
// Type: AppSettings
// Purpose: User preferences that survive restart and can be copied
//          between machines as JSON. New fields must carry
//          #[serde(default)] so older storage files still load.
// ------------------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub zoom_speed: f64,
    pub bindings: InputMap,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dock_layout: Option<serde_json::Value>,
    pub display: DisplaySettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            zoom_speed: DEFAULT_ZOOM_SPEED,
            bindings: InputMap::standard(),
            dock_layout: Some(encode_dock_layout(&default_dock_state())),
            display: DisplaySettings::default(),
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
        self.bindings.sanitize();
        let state = decode_dock_layout(self.dock_layout.as_ref());
        self.dock_layout = Some(encode_dock_layout(&state));
    }

    pub fn dock_state(&self) -> egui_dock::DockState<WorkspaceTab> {
        decode_dock_layout(self.dock_layout.as_ref())
    }

    pub fn set_dock_state(&mut self, state: &egui_dock::DockState<WorkspaceTab>) {
        let mut state = state.clone();
        sanitize_dock_state(&mut state);
        self.dock_layout = Some(encode_dock_layout(&state));
    }

    pub fn reset_zoom_speed(&mut self) {
        self.zoom_speed = DEFAULT_ZOOM_SPEED;
    }

    pub fn to_portable_json(&self) -> Result<String, String> {
        let mut settings = self.clone();
        settings.sanitize();
        let file = SettingsFile {
            schema_version: SETTINGS_SCHEMA_VERSION,
            settings,
        };
        serde_json::to_string_pretty(&file).map_err(|err| err.to_string())
    }

    pub fn from_portable_json(text: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|err| format!("Invalid JSON: {err}"))?;
        let version = value
            .get("schema_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        if version > SETTINGS_SCHEMA_VERSION {
            return Err(format!(
                "Settings file version {version} is newer than this MyCad build (supports {SETTINGS_SCHEMA_VERSION})."
            ));
        }
        let file: SettingsFile = serde_json::from_value(value)
            .map_err(|err| format!("Could not read settings: {err}"))?;
        let mut settings = file.settings;
        settings.sanitize();
        Ok(settings)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SettingsFile {
    #[serde(default)]
    schema_version: u32,
    #[serde(flatten)]
    settings: AppSettings,
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
    use crate::input::{Binding, InputAction, MouseButtonKind};

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
    fn json_round_trip_preserves_zoom_and_bindings() {
        let mut settings = AppSettings {
            zoom_speed: 1.75,
            ..AppSettings::default()
        };
        settings
            .bindings
            .bindings_for_mut(InputAction::SelectClear)
            .clear();
        settings
            .bindings
            .bindings_for_mut(InputAction::SelectClear)
            .push(Binding::key("space"));
        let json = settings.to_portable_json().expect("encode");
        let decoded = AppSettings::from_portable_json(&json).expect("decode");
        assert!((decoded.zoom_speed - 1.75).abs() < 1e-12);
        assert_eq!(
            decoded.bindings.bindings_for(InputAction::SelectClear)[0]
                .key
                .as_deref(),
            Some("space")
        );
        assert!(json.contains("\"schema_version\": 1"));
        assert!(!json.to_lowercase().contains(".dwg"));
    }

    #[test]
    fn missing_fields_use_defaults() {
        let decoded = AppSettings::from_portable_json("{\"schema_version\":1}").expect("empty");
        assert!((decoded.zoom_speed - DEFAULT_ZOOM_SPEED).abs() < 1e-15);
        assert!(!decoded
            .bindings
            .bindings_for(InputAction::SelectReplace)
            .is_empty());
    }

    #[test]
    fn old_zoom_only_json_still_loads() {
        let decoded = AppSettings::from_portable_json("{\"zoom_speed\":2.5}").expect("legacy");
        assert!((decoded.zoom_speed - 2.5).abs() < 1e-12);
        assert_eq!(
            decoded.bindings.bindings_for(InputAction::SelectReplace)[0].mouse,
            Some(MouseButtonKind::Left)
        );
    }

    #[test]
    fn future_schema_is_rejected() {
        let err = AppSettings::from_portable_json("{\"schema_version\":99,\"zoom_speed\":1.0}")
            .expect_err("future");
        assert!(err.contains("newer"));
    }

    #[test]
    fn invalid_json_is_rejected() {
        let err = AppSettings::from_portable_json("{not json").expect_err("invalid");
        assert!(err.contains("Invalid JSON"));
    }

    #[test]
    fn invalid_dock_layout_resets_to_default() {
        let json = r#"{
            "schema_version": 1,
            "dock_layout": {
                "surfaces": []
            }
        }"#;
        let decoded = AppSettings::from_portable_json(json).expect("sanitize dock");
        let tabs: Vec<_> = decoded
            .dock_state()
            .iter_all_tabs()
            .map(|(_, tab)| *tab)
            .collect();
        assert!(tabs.contains(&WorkspaceTab::Viewport));
        assert!(tabs.contains(&WorkspaceTab::Properties));
    }

    #[test]
    fn apply_commits_draft_cancel_restores_applied() {
        let mut applied = AppSettings {
            zoom_speed: 1.5,
            ..AppSettings::default()
        };
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

    #[test]
    fn ron_round_trip_preserves_zoom_speed() {
        let mut settings = AppSettings {
            zoom_speed: 1.75,
            ..AppSettings::default()
        };
        settings.sanitize();
        let encoded = ron::to_string(&settings).expect("encode");
        let decoded: AppSettings = ron::from_str(&encoded).expect("decode");
        assert!((decoded.zoom_speed - 1.75).abs() < 1e-12);
    }

    #[test]
    fn missing_display_fields_use_defaults() {
        let decoded = AppSettings::from_portable_json("{\"schema_version\":1}").expect("empty");
        assert_eq!(decoded.display.window_selection, RgbColor::WINDOW);
        assert_eq!(decoded.display.crossing_selection, RgbColor::CROSSING);
    }

    #[test]
    fn display_colors_round_trip_in_portable_json() {
        let mut settings = AppSettings::default();
        settings.display.window_selection = RgbColor::new(1, 2, 3);
        settings.display.crossing_selection = RgbColor::new(9, 8, 7);
        let json = settings.to_portable_json().expect("encode");
        let decoded = AppSettings::from_portable_json(&json).expect("decode");
        assert_eq!(decoded.display.window_selection, RgbColor::new(1, 2, 3));
        assert_eq!(decoded.display.crossing_selection, RgbColor::new(9, 8, 7));
    }

    #[test]
    fn old_json_without_display_still_loads() {
        let decoded = AppSettings::from_portable_json("{\"zoom_speed\":2.5}").expect("legacy");
        assert_eq!(decoded.display, DisplaySettings::default());
    }
}
