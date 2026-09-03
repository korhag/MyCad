//! Dockable Home toolbar: History, Draw, Modify, Measure, and Layer.

use cad_core::CadColor;
use eframe::egui::{self, Color32, FontId, Pos2, RichText, Sense, Ui, Vec2};
use egui_phosphor::regular::{
    ANGLE, ARROWS_CLOCKWISE, ARROWS_OUT, ARROWS_OUT_CARDINAL, ARROW_CLOCKWISE,
    ARROW_COUNTER_CLOCKWISE, CIRCLE, COPY, ERASER, FLIP_HORIZONTAL, LINE_SEGMENT, LINE_SEGMENTS,
    PATH, POLYGON, PUSH_PIN, RECTANGLE, RULER, STACK,
};

use crate::app::MyCadApp;

pub const TILE: Vec2 = Vec2::new(38.0, 30.0);
const CAPTION_H: f32 = 10.0;
pub const RIBBON_BODY_HEIGHT: f32 = CAPTION_H + TILE.y;
pub const RIBBON_BODY_MARGIN: i8 = 1;

pub fn show(ui: &mut Ui, app: &mut MyCadApp) {
    ui.spacing_mut().item_spacing = Vec2::new(2.0, 0.0);
    ui.spacing_mut().button_padding = Vec2::new(2.0, 1.0);
    ui.set_height(RIBBON_BODY_HEIGHT);
    egui::ScrollArea::horizontal()
        .id_salt("home-toolbar")
        .auto_shrink([false, true])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .show(ui, |ui| {
            ui.set_height(RIBBON_BODY_HEIGHT);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(2.0, 0.0);
                history_group(ui, app);
                compact_separator(ui);
                draw_group(ui, app);
                compact_separator(ui);
                modify_group(ui);
                compact_separator(ui);
                measure_group(ui, app);
                compact_separator(ui);
                layer_group(ui, app);
            });
        });
}

fn history_group(ui: &mut Ui, app: &mut MyCadApp) {
    group_column(ui, "History", |ui| {
        if icon_tile(
            ui,
            ARROW_COUNTER_CLOCKWISE,
            "Undo",
            app.can_undo(),
            "Ctrl+Z",
        )
        .clicked()
        {
            app.undo();
        }
        if icon_tile(ui, ARROW_CLOCKWISE, "Redo", app.can_redo(), "Ctrl+Y").clicked() {
            app.redo();
        }
    });
}

fn draw_group(ui: &mut Ui, app: &mut MyCadApp) {
    group_column(ui, "Draw", |ui| {
        let idle = !app.command_is_active();
        if icon_tile(ui, LINE_SEGMENT, "Line", idle, "L").clicked() {
            app.start_line_command();
        }
        later_tile(ui, LINE_SEGMENTS, "Polyline", "v0.4.0");
        later_tile(ui, CIRCLE, "Circle", "v0.4.0");
        later_tile(ui, PATH, "Arc", "v0.4.0");
        later_tile(ui, RECTANGLE, "Rectangle", "v0.4.0");
    });
}

fn modify_group(ui: &mut Ui) {
    group_column(ui, "Modify", |ui| {
        later_tile(ui, ARROWS_OUT_CARDINAL, "Move", "v0.5.0");
        later_tile(ui, COPY, "Copy", "v0.5.0");
        later_tile(ui, ARROWS_CLOCKWISE, "Rotate", "v0.5.0");
        later_tile(ui, FLIP_HORIZONTAL, "Mirror", "v0.5.0");
        later_tile(ui, ARROWS_OUT, "Scale", "v0.5.0");
        later_tile(ui, ERASER, "Erase", "v0.5.0");
    });
}

fn measure_group(ui: &mut Ui, app: &mut MyCadApp) {
    group_column(ui, "Measure", |ui| {
        let idle = !app.command_is_active();
        if icon_tile(ui, RULER, "Distance", idle, "DI").clicked() {
            app.start_distance_command();
        }
        later_tile(ui, ANGLE, "Angle", "v0.6.0");
        later_tile(ui, CIRCLE, "Radius", "v0.6.0");
        later_tile(ui, POLYGON, "Area", "v0.6.0");
    });
}

fn layer_group(ui: &mut Ui, app: &mut MyCadApp) {
    group_column(ui, "Layer", |ui| {
        let Some(document) = app.document.as_ref() else {
            ui.label(RichText::new("No drawing").small().weak());
            return;
        };
        let current = document.current_layer.clone();
        let layers: Vec<_> = document
            .layers
            .values()
            .map(|layer| (layer.name.clone(), layer.frozen, layer.color))
            .collect();
        let current_color = layers
            .iter()
            .find(|(name, _, _)| *name == current)
            .map(|(_, _, color)| *color)
            .unwrap_or(CadColor::Aci(7));
        let can_set = app
            .selection
            .shared_layer(document)
            .is_some_and(|layer| document.layer_can_be_current(&layer));
        paint_swatch(ui, current_color);
        ui.label(RichText::new(STACK).size(14.0));
        let mut chosen = None;
        egui::ComboBox::from_id_salt("home-current-layer")
            .selected_text(current.clone())
            .width(96.0)
            .height(TILE.y)
            .show_ui(ui, |ui| {
                for (name, frozen, color) in &layers {
                    ui.horizontal(|ui| {
                        paint_swatch(ui, *color);
                        let label = if *frozen {
                            format!("{name} (frozen)")
                        } else {
                            name.clone()
                        };
                        if ui
                            .add_enabled(
                                !*frozen,
                                egui::Button::selectable(*name == current, label),
                            )
                            .clicked()
                        {
                            chosen = Some(name.clone());
                        }
                    });
                }
            });
        if let Some(name) = chosen {
            app.set_current_layer(&name);
        }
        if icon_tile(
            ui,
            PUSH_PIN,
            "Current",
            can_set,
            "Use the selected object's layer",
        )
        .clicked()
        {
            app.set_selected_layer_current();
        }
    });
}

fn later_tile(ui: &mut Ui, icon: &str, label: &str, release: &str) {
    let _ = icon_tile(ui, icon, label, false, &format!("Coming in {release}"));
}

fn group_column(ui: &mut Ui, title: &str, add_contents: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        ui.set_height(RIBBON_BODY_HEIGHT);
        ui.label(RichText::new(title).size(9.0).weak());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(1.0, 0.0);
            add_contents(ui);
        });
    });
}

fn compact_separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(6.0, RIBBON_BODY_HEIGHT), Sense::hover());
    let x = rect.center().x;
    ui.painter().vline(
        x,
        rect.y_range().shrink(3.0),
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
}

fn icon_tile(ui: &mut Ui, icon: &str, label: &str, enabled: bool, hint: &str) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(TILE, sense);
    let response = response.on_hover_text(hint);
    let style = ui.style();
    let visuals = if !enabled {
        &style.visuals.widgets.noninteractive
    } else if response.is_pointer_button_down_on() {
        &style.visuals.widgets.active
    } else if response.hovered() {
        &style.visuals.widgets.hovered
    } else {
        &style.visuals.widgets.inactive
    };
    ui.painter().rect_filled(rect, 1.0, visuals.bg_fill);
    ui.painter()
        .rect_stroke(rect, 1.0, visuals.bg_stroke, egui::StrokeKind::Inside);
    let icon_color = if enabled {
        visuals.fg_stroke.color
    } else {
        style.visuals.weak_text_color()
    };
    let label_color = if enabled {
        style.visuals.text_color()
    } else {
        style.visuals.weak_text_color()
    };
    ui.painter().text(
        Pos2::new(rect.center().x, rect.top() + 11.0),
        egui::Align2::CENTER_CENTER,
        icon,
        FontId::proportional(14.0),
        icon_color,
    );
    ui.painter().text(
        Pos2::new(rect.center().x, rect.bottom() - 7.0),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(9.0),
        label_color,
    );
    response
}

fn paint_swatch(ui: &mut Ui, color: CadColor) {
    let rgb = color.resolve(color, CadColor::Aci(7));
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 1.0, Color32::from_rgb(rgb.r, rgb.g, rgb.b));
}
