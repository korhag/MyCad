//! Persistent user preferences, including portable JSON import/export.

use serde::{Deserialize, Serialize};

use crate::drafting::DraftingPreferences;
use crate::input::InputMap;
use crate::workspace::{
    decode_dock_layout, default_dock_state, encode_dock_layout, migrate_blocks_tab,
    migrate_command_line_tab, migrate_home_tab, recover_home_split_once, sanitize_dock_state,
    WorkspaceTab,
};

pub const STORAGE_KEY: &str = "mycad_settings";
pub const SETTINGS_SCHEMA_VERSION: u32 = 6;
pub const DEFAULT_ZOOM_SPEED: f64 = 1.0;
pub const ZOOM_SPEED_MIN: f64 = 0.25;
pub const ZOOM_SPEED_MAX: f64 = 10.0;
pub const ZOOM_SCROLL_BASE: f64 = 1.001;
pub const DEFAULT_WHEEL_ACCEL_STRENGTH: f64 = 1.0;
pub const WHEEL_ACCEL_STRENGTH_MIN: f64 = 0.25;
pub const WHEEL_ACCEL_STRENGTH_MAX: f64 = 3.0;
/// Same-direction clicks closer than this continue one streak.
pub const WHEEL_ACCEL_WINDOW_SECS: f64 = 0.25;
/// Added to the zoom multiplier for each click after the first, at strength 1.
pub const WHEEL_ACCEL_STEP: f64 = 0.35;
pub const WHEEL_ACCEL_MAX_MULTIPLIER: f64 = 5.0;
pub const DEFAULT_VIEWPORT_MSAA: u32 = 4;
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

    pub fn to_gpu(self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            1.0,
        ]
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
// Type: LibraryPreviewSettings
// Purpose: Thumbnail display, generation, and refresh are independent.
//          The library browser is a later phase; these settings are
//          the contract that browser will honor.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryPreviewSettings {
    pub show_thumbnails: bool,
    pub generate_missing: bool,
    pub refresh_policy: ThumbnailRefreshPolicySetting,
}

impl Default for LibraryPreviewSettings {
    fn default() -> Self {
        Self {
            show_thumbnails: true,
            generate_missing: true,
            refresh_policy: ThumbnailRefreshPolicySetting::Ask,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThumbnailRefreshPolicySetting {
    Ask,
    Automatic,
    Manual,
}

impl ThumbnailRefreshPolicySetting {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ask => "Ask",
            Self::Automatic => "Automatic",
            Self::Manual => "Manual",
        }
    }
}

// ------------------------------------------------------------
// Type: WheelAccelerationSettings
// Purpose: Consecutive same-direction wheel clicks zoom further.
//          A single click stays at 1× so precise steps are unchanged.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WheelAccelerationSettings {
    pub enabled: bool,
    pub strength: f64,
}

impl Default for WheelAccelerationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            strength: DEFAULT_WHEEL_ACCEL_STRENGTH,
        }
    }
}

impl WheelAccelerationSettings {
    pub fn sanitize(&mut self) {
        self.strength = sanitize_wheel_accel_strength(self.strength);
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
    pub drafting: DraftingPreferences,
    #[serde(default)]
    pub home_layout_migrated: bool,
    #[serde(default)]
    pub blocks_tab_migrated: bool,
    #[serde(default)]
    pub command_line_migrated: bool,
    #[serde(default)]
    pub responsive_ribbon_recovered: bool,
    #[serde(default)]
    pub compact_home_height_applied: bool,
    #[serde(default)]
    pub library_preview: LibraryPreviewSettings,
    #[serde(default)]
    pub wheel_acceleration: WheelAccelerationSettings,
    /// Offscreen viewport samples. The window itself stays at one sample.
    #[serde(default = "default_viewport_msaa")]
    pub viewport_msaa: u32,
    /// Ease each wheel click instead of jumping to the new zoom.
    #[serde(default = "default_smooth_zoom")]
    pub smooth_zoom: bool,
}

fn default_viewport_msaa() -> u32 {
    DEFAULT_VIEWPORT_MSAA
}

fn default_smooth_zoom() -> bool {
    true
}

pub fn sanitize_viewport_msaa(value: u32) -> u32 {
    match value {
        1 => 1,
        2 => 2,
        _ => DEFAULT_VIEWPORT_MSAA,
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            zoom_speed: DEFAULT_ZOOM_SPEED,
            bindings: InputMap::standard(),
            dock_layout: Some(encode_dock_layout(&default_dock_state())),
            display: DisplaySettings::default(),
            drafting: DraftingPreferences::default(),
            home_layout_migrated: false,
            blocks_tab_migrated: false,
            command_line_migrated: false,
            responsive_ribbon_recovered: false,
            compact_home_height_applied: false,
            library_preview: LibraryPreviewSettings::default(),
            wheel_acceleration: WheelAccelerationSettings::default(),
            viewport_msaa: DEFAULT_VIEWPORT_MSAA,
            smooth_zoom: default_smooth_zoom(),
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
        self.wheel_acceleration.sanitize();
        self.viewport_msaa = sanitize_viewport_msaa(self.viewport_msaa);
        self.bindings.sanitize();
        let mut state = decode_dock_layout(self.dock_layout.as_ref());
        self.home_layout_migrated = migrate_home_tab(&mut state, self.home_layout_migrated);
        self.blocks_tab_migrated = migrate_blocks_tab(&mut state, self.blocks_tab_migrated);
        self.command_line_migrated =
            migrate_command_line_tab(&mut state, self.command_line_migrated);
        self.responsive_ribbon_recovered =
            recover_home_split_once(&mut state, self.responsive_ribbon_recovered);
        self.dock_layout = Some(encode_dock_layout(&state));
        self.drafting.sanitize();
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

    pub fn reset_wheel_acceleration(&mut self) {
        self.wheel_acceleration = WheelAccelerationSettings::default();
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
                "Settings file version {version} is newer than this {} build (supports {SETTINGS_SCHEMA_VERSION}).",
                crate::brand::APP_NAME
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

pub fn sanitize_wheel_accel_strength(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(WHEEL_ACCEL_STRENGTH_MIN, WHEEL_ACCEL_STRENGTH_MAX)
    } else {
        DEFAULT_WHEEL_ACCEL_STRENGTH
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
        assert!(json.contains("\"schema_version\": 6"));
        assert!(!json.to_lowercase().contains(".dwg"));
    }

    #[test]
    fn json_round_trip_preserves_drafting_switches() {
        let mut settings = AppSettings::default();
        settings.drafting.ortho_enabled = true;
        settings.drafting.osnap_enabled = false;
        settings.drafting.running_snaps.midpoint = false;
        let json = settings.to_portable_json().expect("encode");
        let decoded = AppSettings::from_portable_json(&json).expect("decode");
        assert!(decoded.drafting.ortho_enabled);
        assert!(!decoded.drafting.osnap_enabled);
        assert!(!decoded.drafting.running_snaps.midpoint);
    }

    #[test]
    fn missing_fields_use_defaults() {
        let decoded = AppSettings::from_portable_json("{\"schema_version\":1}").expect("empty");
        assert!((decoded.zoom_speed - DEFAULT_ZOOM_SPEED).abs() < 1e-15);
        assert!(decoded.smooth_zoom);
        assert!(!decoded
            .bindings
            .bindings_for(InputAction::SelectReplace)
            .is_empty());
        assert!(!decoded
            .bindings
            .bindings_for(InputAction::ContextMenu)
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
    fn old_select_toggle_settings_migrate_to_add_and_remove() {
        let json = r#"{
            "schema_version": 4,
            "bindings": {
                "select_replace": [{"mouse":"left","gesture":"click","ctrl":false,"shift":false,"alt":false,"command":false}],
                "select_toggle": [
                    {"mouse":"left","gesture":"click","ctrl":true,"shift":false,"alt":false,"command":false},
                    {"mouse":"left","gesture":"click","ctrl":false,"shift":true,"alt":false,"command":false}
                ],
                "select_clear": [{"key":"escape","gesture":"key","ctrl":false,"shift":false,"alt":false,"command":false}],
                "pan": [{"mouse":"middle","gesture":"drag","ctrl":false,"shift":false,"alt":false,"command":false}],
                "zoom_extents": [{"mouse":"left","gesture":"double_click","ctrl":false,"shift":false,"alt":false,"command":false}]
            }
        }"#;
        let decoded = AppSettings::from_portable_json(json).expect("legacy toggle");
        assert!(decoded.bindings.select_toggle.is_empty());
        assert!(!decoded
            .bindings
            .bindings_for(InputAction::SelectAdd)
            .is_empty());
        assert!(!decoded
            .bindings
            .bindings_for(InputAction::SelectRemove)
            .is_empty());
        assert!(decoded.bindings.clicked(
            InputAction::SelectAdd,
            eframe::egui::PointerButton::Primary,
            eframe::egui::Modifiers {
                shift: true,
                ..eframe::egui::Modifiers::default()
            }
        ));
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
        assert!(tabs.contains(&WorkspaceTab::Home));
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

    #[test]
    fn old_layout_without_home_is_migrated_once() {
        let old = egui_dock::DockState::new(vec![WorkspaceTab::Viewport]);
        let mut settings = AppSettings {
            dock_layout: Some(encode_dock_layout(&old)),
            home_layout_migrated: false,
            ..AppSettings::default()
        };
        settings.dock_layout = Some(encode_dock_layout(&old));
        settings.home_layout_migrated = false;
        settings.sanitize();
        assert!(settings.home_layout_migrated);
        assert!(settings
            .dock_state()
            .find_tab(&WorkspaceTab::Home)
            .is_some());

        let mut closed = settings.dock_state();
        if let Some(tab) = closed.find_tab(&WorkspaceTab::Home) {
            closed.remove_tab(tab);
        }
        settings.set_dock_state(&closed);
        settings.sanitize();
        assert!(
            settings
                .dock_state()
                .find_tab(&WorkspaceTab::Home)
                .is_none(),
            "users can still close Home after the one-time migration"
        );
        assert!(settings.home_layout_migrated);
    }

    #[test]
    fn old_layout_gains_blocks_beside_properties_once() {
        let mut old = egui_dock::DockState::new(vec![WorkspaceTab::Viewport]);
        let [viewport, _home] = old.main_surface_mut().split_above(
            egui_dock::NodeIndex::root(),
            crate::workspace::home_split_fraction(800.0),
            vec![WorkspaceTab::Home],
        );
        let [viewport, _props] =
            old.main_surface_mut()
                .split_left(viewport, 0.24, vec![WorkspaceTab::Properties]);
        let _ = old
            .main_surface_mut()
            .split_right(viewport, 0.76, vec![WorkspaceTab::Diagnostics]);
        let mut settings = AppSettings {
            dock_layout: Some(encode_dock_layout(&old)),
            home_layout_migrated: true,
            blocks_tab_migrated: false,
            ..AppSettings::default()
        };
        settings.sanitize();
        assert!(settings.blocks_tab_migrated);
        let state = settings.dock_state();
        let (p_surface, p_node, _) = state
            .find_tab(&WorkspaceTab::Properties)
            .expect("Properties");
        let (b_surface, b_node, _) = state.find_tab(&WorkspaceTab::Blocks).expect("Blocks");
        assert_eq!(p_surface, b_surface);
        assert_eq!(p_node, b_node);

        let mut closed = settings.dock_state();
        if let Some(tab) = closed.find_tab(&WorkspaceTab::Blocks) {
            closed.remove_tab(tab);
        }
        settings.set_dock_state(&closed);
        settings.sanitize();
        assert!(
            settings
                .dock_state()
                .find_tab(&WorkspaceTab::Blocks)
                .is_none(),
            "users can still close Blocks after the one-time migration"
        );
        assert!(settings.blocks_tab_migrated);
    }

    #[test]
    fn library_preview_settings_are_independent() {
        let mut settings = AppSettings::default();
        settings.library_preview.show_thumbnails = false;
        assert!(settings.library_preview.generate_missing);
        let mut core = cad_core::ThumbnailSettings {
            show_thumbnails: settings.library_preview.show_thumbnails,
            generate_missing: settings.library_preview.generate_missing,
            refresh_policy: cad_core::ThumbnailRefreshPolicy::Ask,
        };
        assert!(!core.show_thumbnails);
        assert!(!core.hover_may_decode_source());
        settings.library_preview.generate_missing = false;
        core.generate_missing = false;
        assert!(!core.generation_allowed());
        assert!(!core.hover_may_decode_source());
    }

    #[test]
    fn sanitize_wheel_accel_clamps_and_replaces_non_finite() {
        assert!((sanitize_wheel_accel_strength(0.1) - WHEEL_ACCEL_STRENGTH_MIN).abs() < 1e-15);
        assert!((sanitize_wheel_accel_strength(1.5) - 1.5).abs() < 1e-15);
        assert!((sanitize_wheel_accel_strength(9.0) - WHEEL_ACCEL_STRENGTH_MAX).abs() < 1e-15);
        assert!(
            (sanitize_wheel_accel_strength(f64::NAN) - DEFAULT_WHEEL_ACCEL_STRENGTH).abs() < 1e-15
        );
        assert!(
            (sanitize_wheel_accel_strength(f64::INFINITY) - DEFAULT_WHEEL_ACCEL_STRENGTH).abs()
                < 1e-15
        );
    }

    #[test]
    fn json_round_trip_preserves_wheel_acceleration() {
        let mut settings = AppSettings::default();
        settings.wheel_acceleration.enabled = false;
        settings.wheel_acceleration.strength = 2.25;
        let json = settings.to_portable_json().expect("encode");
        let decoded = AppSettings::from_portable_json(&json).expect("decode");
        assert!(!decoded.wheel_acceleration.enabled);
        assert!((decoded.wheel_acceleration.strength - 2.25).abs() < 1e-12);
    }

    #[test]
    fn old_zoom_only_json_uses_default_wheel_acceleration() {
        let decoded = AppSettings::from_portable_json("{\"zoom_speed\":2.5}").expect("legacy");
        assert_eq!(
            decoded.wheel_acceleration,
            WheelAccelerationSettings::default()
        );
        assert!((decoded.zoom_speed - 2.5).abs() < 1e-12);
    }
}
