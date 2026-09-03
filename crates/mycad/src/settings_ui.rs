//! Settings window: viewport speed, rebindable inputs, import/export.

use std::fs;

use eframe::egui::{self, Color32, RichText};

use crate::app::MyCadApp;
use crate::input::{capture_binding, InputAction};
use crate::settings::{
    sanitize_zoom_speed, AppSettings, RgbColor, DEFAULT_ZOOM_SPEED, ZOOM_SPEED_MAX, ZOOM_SPEED_MIN,
};

#[derive(Clone, Copy)]
pub enum SettingsAction {
    None,
    Apply,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureTarget {
    Replace { action: InputAction, index: usize },
    Add { action: InputAction },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Viewport,
    Display,
    Shortcuts,
}

pub fn show(ctx: &egui::Context, app: &mut MyCadApp) -> SettingsAction {
    if !app.show_settings {
        app.capture = None;
        return SettingsAction::None;
    }
    let mut open = true;
    let mut action = SettingsAction::None;
    egui::Window::new("Settings")
        .open(&mut open)
        .resizable(true)
        .collapsible(false)
        .default_width(460.0)
        .show(ctx, |ui| {
            if let Some(target) = app.capture {
                ui.colored_label(
                    Color32::from_rgb(200, 210, 120),
                    "Press a key or click a mouse button…  Esc cancels",
                );
                ui.add_space(6.0);
                let captured = ui.input(|input| {
                    if input.key_pressed(egui::Key::Escape) {
                        return Err(());
                    }
                    Ok(capture_binding(input, target.prefers_drag()))
                });
                match captured {
                    Err(()) => app.capture = None,
                    Ok(Some(binding)) => {
                        match target {
                            CaptureTarget::Replace { action, index } => {
                                let list = app.settings_draft.bindings.bindings_for_mut(action);
                                if index < list.len() {
                                    list[index] = binding;
                                }
                            }
                            CaptureTarget::Add { action } => {
                                app.settings_draft
                                    .bindings
                                    .bindings_for_mut(action)
                                    .push(binding);
                            }
                        }
                        app.capture = None;
                    }
                    Ok(None) => {}
                }
                ui.separator();
            }

            ui.horizontal(|ui| {
                ui.selectable_value(&mut app.settings_tab, SettingsTab::Viewport, "Viewport");
                ui.selectable_value(&mut app.settings_tab, SettingsTab::Display, "Display");
                ui.selectable_value(&mut app.settings_tab, SettingsTab::Shortcuts, "Shortcuts");
            });
            ui.add_space(8.0);

            match app.settings_tab {
                SettingsTab::Viewport => viewport_tab(ui, app),
                SettingsTab::Display => display_tab(ui, app),
                SettingsTab::Shortcuts => shortcuts_tab(ui, app),
            }

            ui.add_space(8.0);
            ui.separator();
            ui.heading("Portable file");
            ui.weak("JSON you can copy between machines. Drawings are not included.");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Export…").clicked() {
                    export_settings(app);
                }
                if ui.button("Import…").clicked() {
                    import_settings(app);
                }
            });
            if let Some(message) = &app.settings_message {
                ui.label(RichText::new(message).small());
            }

            ui.add_space(12.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Apply").clicked() {
                        action = SettingsAction::Apply;
                    }
                    if ui.button("Cancel").clicked() {
                        action = SettingsAction::Cancel;
                    }
                });
            });
        });
    if !open {
        SettingsAction::Cancel
    } else {
        action
    }
}

impl CaptureTarget {
    fn prefers_drag(self) -> bool {
        match self {
            Self::Replace { action, .. } | Self::Add { action } => action.prefers_drag(),
        }
    }
}

fn viewport_tab(ui: &mut egui::Ui, app: &mut MyCadApp) {
    ui.heading("Viewport");
    ui.add_space(6.0);
    ui.label("Zoom speed");
    ui.horizontal(|ui| {
        ui.add(
            egui::Slider::new(
                &mut app.settings_draft.zoom_speed,
                ZOOM_SPEED_MIN..=ZOOM_SPEED_MAX,
            )
            .logarithmic(true)
            .show_value(false),
        );
        ui.add(
            egui::DragValue::new(&mut app.settings_draft.zoom_speed)
                .speed(0.05)
                .range(ZOOM_SPEED_MIN..=ZOOM_SPEED_MAX)
                .suffix("×")
                .max_decimals(2),
        );
    });
    app.settings_draft.zoom_speed = sanitize_zoom_speed(app.settings_draft.zoom_speed);
    ui.weak("1.0× is the original smooth wheel zoom.");
    ui.add_space(8.0);
    if ui
        .add_enabled(
            (app.settings_draft.zoom_speed - DEFAULT_ZOOM_SPEED).abs() > 1e-9,
            egui::Button::new("Reset zoom speed"),
        )
        .clicked()
    {
        app.settings_draft.reset_zoom_speed();
    }
}

fn display_tab(ui: &mut egui::Ui, app: &mut MyCadApp) {
    ui.heading("Selection box");
    ui.weak("Window (left-to-right) selects fully enclosed entities. Crossing (right-to-left) selects anything the box touches.");
    ui.add_space(8.0);
    color_row(
        ui,
        "Window",
        "Fully inside",
        &mut app.settings_draft.display.window_selection,
    );
    if ui.small_button("Reset window color").clicked() {
        app.settings_draft.display.reset_window();
    }
    ui.add_space(8.0);
    color_row(
        ui,
        "Crossing",
        "Anything touched",
        &mut app.settings_draft.display.crossing_selection,
    );
    if ui.small_button("Reset crossing color").clicked() {
        app.settings_draft.display.reset_crossing();
    }
    ui.add_space(8.0);
    if ui.button("Reset display colors").clicked() {
        app.settings_draft.display.reset_all();
    }
}

fn color_row(ui: &mut egui::Ui, title: &str, hint: &str, color: &mut RgbColor) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong());
        ui.weak(hint);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut rgb = [color.r, color.g, color.b];
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                *color = RgbColor::new(rgb[0], rgb[1], rgb[2]);
            }
        });
    });
}

fn shortcuts_tab(ui: &mut egui::Ui, app: &mut MyCadApp) {
    ui.heading("Shortcuts");
    ui.weak("Bindings are exact: Ctrl+Click does not also fire Click.");
    ui.add_space(6.0);
    if ui.button("Reset all shortcuts").clicked() {
        app.settings_draft.bindings.reset_all();
        app.capture = None;
    }
    ui.add_space(8.0);

    let mut current_group = "";
    for action in InputAction::ALL {
        if action.group() != current_group {
            current_group = action.group();
            ui.label(RichText::new(current_group).strong());
            ui.add_space(4.0);
        }
        ui.horizontal(|ui| {
            ui.label(action.label());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Reset").clicked() {
                    app.settings_draft.bindings.reset_action(action);
                }
                if ui.small_button("Add").clicked() {
                    app.capture = Some(CaptureTarget::Add { action });
                }
            });
        });
        let mut remove_at = None;
        let mut recapture = None;
        let bindings = app.settings_draft.bindings.bindings_for(action).to_vec();
        for (index, binding) in bindings.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("  {binding}"))
                        .monospace()
                        .background_color(Color32::from_rgb(42, 52, 44)),
                );
                if ui.small_button("Change").clicked() {
                    recapture = Some(index);
                }
                if bindings.len() > 1 && ui.small_button("Remove").clicked() {
                    remove_at = Some(index);
                }
            });
        }
        if let Some(index) = recapture {
            app.capture = Some(CaptureTarget::Replace { action, index });
        }
        if let Some(index) = remove_at {
            app.settings_draft
                .bindings
                .bindings_for_mut(action)
                .remove(index);
        }
        ui.add_space(6.0);
    }

    let conflicts = app.settings_draft.bindings.conflicts();
    if !conflicts.is_empty() {
        ui.colored_label(Color32::from_rgb(220, 140, 90), "Conflicting shortcuts:");
        for (left, right, binding) in conflicts {
            if left == right {
                ui.weak(format!("• {} is assigned twice as {binding}", left.label()));
            } else {
                ui.weak(format!(
                    "• {} and {} both use {binding}",
                    left.label(),
                    right.label()
                ));
            }
        }
    }
}

fn export_settings(app: &mut MyCadApp) {
    let mut snapshot = app.settings_draft.clone();
    if !app.imported_dock {
        snapshot.set_dock_state(&app.dock_state);
    }
    snapshot.sanitize();
    match snapshot.to_portable_json() {
        Ok(json) => {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("MyCad settings", &["json"])
                .set_file_name("mycad-settings.json")
                .save_file()
            {
                match fs::write(&path, json) {
                    Ok(()) => {
                        app.settings_message = Some(format!("Exported {}", path.display()));
                    }
                    Err(err) => {
                        app.settings_message = Some(format!("Export failed: {err}"));
                    }
                }
            }
        }
        Err(err) => app.settings_message = Some(format!("Export failed: {err}")),
    }
}

fn import_settings(app: &mut MyCadApp) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("MyCad settings", &["json"])
        .add_filter("All files", &["*"])
        .pick_file()
    else {
        return;
    };
    match fs::read_to_string(&path) {
        Ok(text) => match AppSettings::from_portable_json(&text) {
            Ok(settings) => {
                app.settings_draft = settings;
                app.imported_dock = true;
                app.capture = None;
                app.settings_message =
                    Some(format!("Imported {} — click Apply to use", path.display()));
            }
            Err(err) => app.settings_message = Some(err),
        },
        Err(err) => app.settings_message = Some(format!("Import failed: {err}")),
    }
}
