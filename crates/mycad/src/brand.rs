//! EntoCAD name, window icon, splash card, and settings carried over from MyCad.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Color32, TextureHandle};

/// Name shown in the window, menus, dialogs, and crash report.
pub const APP_NAME: &str = "EntoCAD";

/// App id eframe used before the rename. Settings live under this folder.
const LEGACY_APP_NAME: &str = "MyCad";

/// File eframe writes inside [`eframe::storage_dir`].
const STORAGE_FILE: &str = "app.ron";

const ICON_PNG: &[u8] = include_bytes!("../../../resources/icons/entocad-256.png");
const SPLASH_PNG: &[u8] = include_bytes!("../../../resources/splash/entocad-splash.png");

/// New copies use the first suffix. Older `-MyCad` copies still count, so a
/// second save does not stack another suffix on the name.
pub const SAVED_COPY_SUFFIXES: [&str; 2] = ["-EntoCAD", "-MyCad"];

/// How long the splash stays fully visible after startup.
pub const SPLASH_HOLD: Duration = Duration::from_millis(1200);

/// How long the splash takes to fade once the hold (and any file load) ends.
pub const SPLASH_FADE: Duration = Duration::from_millis(400);

const SPLASH_MAX_WIDTH: f32 = 560.0;
const SPLASH_CARD_PAD: f32 = 28.0;
const SPLASH_CARD_RADIUS: f32 = 16.0;
const EMPTY_LOGO_SIZE: f32 = 96.0;
const EMPTY_LOGO_ALPHA: u8 = 210;
pub const MENU_ICON_SIZE: f32 = 22.0;

const EMPTY_HINT: &str = "File → Open   or   pass a DWG on the command line";

// ------------------------------------------------------------
// Type: SplashState
// Purpose: Keep the startup card fully visible for a short hold, and
//          while a drawing passed on the command line is still loading,
//          then fade it out. A click or a key press ends it immediately.
// ------------------------------------------------------------
pub struct SplashState {
    started: Instant,
    fade_started: Option<Instant>,
}

impl SplashState {
    pub fn start() -> Self {
        Self {
            started: Instant::now(),
            fade_started: None,
        }
    }

    /// Opacity from 1 to 0, or `None` once the card should be removed.
    pub fn opacity(&mut self, loading: bool, dismiss: bool) -> Option<f32> {
        if dismiss {
            return None;
        }
        if loading || self.started.elapsed() < SPLASH_HOLD {
            self.fade_started = None;
            return Some(1.0);
        }
        let fade_started = *self.fade_started.get_or_insert_with(Instant::now);
        let t = fade_started.elapsed().as_secs_f32() / SPLASH_FADE.as_secs_f32();
        if t >= 1.0 {
            None
        } else {
            Some((1.0 - t).clamp(0.0, 1.0))
        }
    }
}

pub fn input_dismisses_splash(input: &egui::InputState) -> bool {
    if input.pointer.any_pressed() || input.pointer.any_click() {
        return true;
    }
    input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key {
                pressed: true,
                repeat: false,
                ..
            }
        )
    })
}

// ------------------------------------------------------------
// Type: BrandTextures
// Purpose: The symbol and the splash lockup, decoded once at startup
//          and uploaded to egui. Missing or unreadable bytes leave the
//          window usable without a picture.
// ------------------------------------------------------------
pub struct BrandTextures {
    pub icon: TextureHandle,
    pub splash: TextureHandle,
}

impl BrandTextures {
    pub fn load(ctx: &egui::Context) -> Option<Self> {
        let icon = load_texture(ctx, "entocad-icon", ICON_PNG)?;
        let splash = load_texture(ctx, "entocad-splash", SPLASH_PNG)?;
        Some(Self { icon, splash })
    }
}

fn load_texture(ctx: &egui::Context, name: &str, png: &[u8]) -> Option<TextureHandle> {
    let image = decode_png(png)?;
    Some(ctx.load_texture(name, image, egui::TextureOptions::LINEAR))
}

fn decode_png(png: &[u8]) -> Option<egui::ColorImage> {
    let image = match image::load_from_memory(png) {
        Ok(image) => image.into_rgba8(),
        Err(err) => {
            eprintln!("{APP_NAME} could not decode a logo: {err}");
            return None;
        }
    };
    let size = [image.width() as usize, image.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(
        size,
        image.as_raw(),
    ))
}

/// Window and taskbar icon. A decode failure opens the window without one.
pub fn window_icon() -> Option<Arc<egui::IconData>> {
    match eframe::icon_data::from_png_bytes(ICON_PNG) {
        Ok(icon) => Some(Arc::new(icon)),
        Err(err) => {
            eprintln!("{APP_NAME} could not load the window icon: {err}");
            None
        }
    }
}

/// Copy `MyCad/data/app.ron` to `EntoCAD` when the new file is not there yet,
/// so a renamed build keeps the saved layout and preferences.
pub fn migrate_legacy_storage() {
    let Some(new_dir) = eframe::storage_dir(APP_NAME) else {
        return;
    };
    let new_file = new_dir.join(STORAGE_FILE);
    if new_file.exists() {
        return;
    }
    let Some(old_file) = eframe::storage_dir(LEGACY_APP_NAME).map(|dir| dir.join(STORAGE_FILE))
    else {
        return;
    };
    if !old_file.is_file() {
        return;
    }
    if let Err(err) = std::fs::create_dir_all(&new_dir) {
        eprintln!("{APP_NAME} could not create its settings folder: {err}");
        return;
    }
    if let Err(err) = std::fs::copy(&old_file, &new_file) {
        eprintln!("{APP_NAME} could not copy settings from {LEGACY_APP_NAME}: {err}");
    }
}

pub fn has_saved_copy_suffix(stem: &str) -> bool {
    SAVED_COPY_SUFFIXES
        .iter()
        .any(|suffix| stem.ends_with(suffix))
}

pub fn saved_copy_stem(stem: &str) -> String {
    if has_saved_copy_suffix(stem) {
        stem.to_string()
    } else {
        format!("{stem}{}", SAVED_COPY_SUFFIXES[0])
    }
}

pub fn show_menu_mark(ui: &mut egui::Ui, icon: &TextureHandle) {
    ui.add(
        egui::Image::new(icon)
            .fit_to_exact_size(egui::vec2(MENU_ICON_SIZE, MENU_ICON_SIZE))
            .alt_text(APP_NAME),
    );
}

pub fn paint_splash(
    painter: &egui::Painter,
    rect: egui::Rect,
    splash: &TextureHandle,
    opacity: f32,
) {
    let opacity = opacity.clamp(0.0, 1.0);
    let alpha = (opacity * 255.0).round() as u8;
    let size = splash.size_vec2();
    let max_width = (rect.width() - 64.0).clamp(120.0, SPLASH_MAX_WIDTH);
    let image_height = max_width * size.y / size.x.max(1.0);
    let card = egui::Rect::from_center_size(
        rect.center(),
        egui::vec2(
            max_width + SPLASH_CARD_PAD * 2.0,
            image_height + SPLASH_CARD_PAD * 2.0,
        ),
    );
    painter.rect_filled(
        card.translate(egui::vec2(0.0, 6.0)),
        SPLASH_CARD_RADIUS,
        Color32::from_black_alpha(alpha / 3),
    );
    painter.rect_filled(
        card,
        SPLASH_CARD_RADIUS,
        Color32::from_rgba_unmultiplied(244, 246, 244, alpha),
    );
    painter.image(
        splash.id(),
        card.shrink(SPLASH_CARD_PAD),
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::from_rgba_unmultiplied(255, 255, 255, alpha),
    );
}

pub fn paint_empty_viewport(painter: &egui::Painter, rect: egui::Rect, icon: &TextureHandle) {
    let logo = EMPTY_LOGO_SIZE
        .min(rect.width() * 0.28)
        .min(rect.height() * 0.28)
        .max(32.0);
    let gap = 16.0;
    let text_height = 20.0;
    let top = rect.center().y - (logo + gap + text_height) * 0.5;
    let logo_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, top + logo * 0.5),
        egui::vec2(logo, logo),
    );
    painter.image(
        icon.id(),
        logo_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::from_rgba_unmultiplied(255, 255, 255, EMPTY_LOGO_ALPHA),
    );
    painter.text(
        egui::pos2(rect.center().x, top + logo + gap + text_height * 0.5),
        egui::Align2::CENTER_CENTER,
        EMPTY_HINT,
        egui::FontId::proportional(16.0),
        Color32::from_rgb(140, 160, 140),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_copy_uses_entocad_and_keeps_an_existing_suffix() {
        assert_eq!(saved_copy_stem("plant"), "plant-EntoCAD");
        assert_eq!(saved_copy_stem("plant-EntoCAD"), "plant-EntoCAD");
        assert_eq!(saved_copy_stem("plant-MyCad"), "plant-MyCad");
        assert!(has_saved_copy_suffix("drawing-MyCad"));
        assert!(!has_saved_copy_suffix("drawing"));
    }

    #[test]
    fn splash_stays_up_while_loading_and_a_click_dismisses_it() {
        let mut splash = SplashState::start();
        assert_eq!(splash.opacity(false, false), Some(1.0));
        assert_eq!(splash.opacity(true, false), Some(1.0));
        assert_eq!(splash.opacity(false, true), None);
    }
}
