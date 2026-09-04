//! Dockable Home toolbar: History, Draw, Modify, Measure, and Layer.

use cad_core::CadColor;
use eframe::egui::{self, Color32, FontId, Pos2, RichText, Sense, Ui, Vec2};
use egui_phosphor::regular::{
    ANGLE, ARROWS_CLOCKWISE, ARROWS_OUT, ARROWS_OUT_CARDINAL, ARROW_CLOCKWISE,
    ARROW_COUNTER_CLOCKWISE, CIRCLE, COPY, ERASER, FLIP_HORIZONTAL, LINE_SEGMENT, LINE_SEGMENTS,
    PATH, POLYGON, PUSH_PIN, RECTANGLE, RULER, STACK,
};

use crate::app::MyCadApp;

const COMPACT_HEIGHT: f32 = 45.0;
const MIN_SCALE: f32 = 0.7;
const MAX_SCALE: f32 = 2.4;

#[derive(Clone, Copy, Debug, PartialEq)]
struct RibbonLayout {
    compact: bool,
    tile: Vec2,
    icon_size: f32,
    label_size: f32,
    caption_size: f32,
}

fn layout_for_height(height: f32) -> RibbonLayout {
    if height < COMPACT_HEIGHT {
        return RibbonLayout {
            compact: true,
            tile: Vec2::ZERO,
            icon_size: 0.0,
            label_size: 0.0,
            caption_size: 0.0,
        };
    }
    let scale = ((height - 18.0) / 42.0).clamp(MIN_SCALE, MAX_SCALE);
    RibbonLayout {
        compact: false,
        tile: Vec2::new(38.0 * scale, 30.0 * scale),
        icon_size: (14.0 * scale).clamp(11.0, 28.0),
        label_size: (9.0 * scale).clamp(8.0, 14.0),
        caption_size: (9.0 * scale).clamp(8.0, 13.0),
    }
}

fn compact_show(ui: &mut Ui, app: &mut MyCadApp) {
    ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(app.can_undo(), egui::Button::new("Undo"))
            .on_hover_text("Ctrl+Z")
            .clicked()
        {
            app.undo();
        }
        if ui
            .add_enabled(app.can_redo(), egui::Button::new("Redo"))
            .on_hover_text("Ctrl+Y")
            .clicked()
        {
            app.redo();
        }
        let idle = !app.command_is_active();
        if ui
            .add_enabled(idle, egui::Button::new("Line"))
            .on_hover_text("L")
            .clicked()
        {
            app.start_line_command();
        }
        if ui
            .add_enabled(idle, egui::Button::new("Distance"))
            .on_hover_text("DI")
            .clicked()
        {
            app.start_distance_command();
        }
        compact_menu(ui, "Draw", |ui| {
            ui.add_enabled(false, egui::Button::new("Polyline"))
                .on_disabled_hover_text("Coming in v0.4.0");
            ui.add_enabled(false, egui::Button::new("Circle"))
                .on_disabled_hover_text("Coming in v0.4.0");
            ui.add_enabled(false, egui::Button::new("Arc"))
                .on_disabled_hover_text("Coming in v0.4.0");
            ui.add_enabled(false, egui::Button::new("Rectangle"))
                .on_disabled_hover_text("Coming in v0.4.0");
        });
        compact_menu(ui, "Modify", |ui| {
            for label in ["Move", "Copy", "Rotate", "Mirror", "Scale", "Erase"] {
                ui.add_enabled(false, egui::Button::new(label))
                    .on_disabled_hover_text("Coming in v0.5.0");
            }
        });
        compact_menu(ui, "Measure", |ui| {
            for label in ["Angle", "Radius", "Area"] {
                ui.add_enabled(false, egui::Button::new(label))
                    .on_disabled_hover_text("Coming in v0.6.0");
            }
        });
        let mut chosen_layer = None;
        compact_menu(ui, "Layer", |ui| {
            if let Some(document) = app.document.as_ref() {
                let current = document.current_layer.clone();
                for layer in document.layers.values() {
                    if ui
                        .add_enabled(
                            !layer.frozen,
                            egui::Button::selectable(layer.name == current, &layer.name),
                        )
                        .clicked()
                    {
                        chosen_layer = Some(layer.name.clone());
                    }
                }
            } else {
                ui.label("No drawing");
            }
        });
        if let Some(layer) = chosen_layer {
            app.set_current_layer(&layer);
        }
    });
}

fn compact_menu(ui: &mut Ui, label: &str, contents: impl FnOnce(&mut Ui)) {
    ui.menu_button(label, |ui| contents(ui));
}

pub fn show(ui: &mut Ui, app: &mut MyCadApp) {
    let layout = layout_for_height(ui.available_height());
    if layout.compact {
        compact_show(ui, app);
        return;
    }
    ui.spacing_mut().item_spacing = Vec2::new(2.0, 0.0);
    ui.spacing_mut().button_padding = Vec2::new(2.0, 1.0);
    egui::ScrollArea::horizontal()
        .id_salt("home-toolbar")
        .auto_shrink([false, true])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(2.0, 0.0);
                history_group(ui, app, layout);
                compact_separator(ui, layout);
                draw_group(ui, app, layout);
                compact_separator(ui, layout);
                modify_group(ui, layout);
                compact_separator(ui, layout);
                measure_group(ui, app, layout);
                compact_separator(ui, layout);
                layer_group(ui, app, layout);
            });
        });
}

fn history_group(ui: &mut Ui, app: &mut MyCadApp, layout: RibbonLayout) {
    group_column(ui, "History", layout, |ui| {
        if icon_tile(
            ui,
            ARROW_COUNTER_CLOCKWISE,
            "Undo",
            app.can_undo(),
            "Ctrl+Z",
            layout,
        )
        .clicked()
        {
            app.undo();
        }
        if icon_tile(
            ui,
            ARROW_CLOCKWISE,
            "Redo",
            app.can_redo(),
            "Ctrl+Y",
            layout,
        )
        .clicked()
        {
            app.redo();
        }
    });
}

fn draw_group(ui: &mut Ui, app: &mut MyCadApp, layout: RibbonLayout) {
    group_column(ui, "Draw", layout, |ui| {
        let idle = !app.command_is_active();
        if icon_tile(ui, LINE_SEGMENT, "Line", idle, "L", layout).clicked() {
            app.start_line_command();
        }
        later_tile(ui, LINE_SEGMENTS, "Polyline", "v0.4.0", layout);
        later_tile(ui, CIRCLE, "Circle", "v0.4.0", layout);
        later_tile(ui, PATH, "Arc", "v0.4.0", layout);
        later_tile(ui, RECTANGLE, "Rectangle", "v0.4.0", layout);
    });
}

fn modify_group(ui: &mut Ui, layout: RibbonLayout) {
    group_column(ui, "Modify", layout, |ui| {
        later_tile(ui, ARROWS_OUT_CARDINAL, "Move", "v0.5.0", layout);
        later_tile(ui, COPY, "Copy", "v0.5.0", layout);
        later_tile(ui, ARROWS_CLOCKWISE, "Rotate", "v0.5.0", layout);
        later_tile(ui, FLIP_HORIZONTAL, "Mirror", "v0.5.0", layout);
        later_tile(ui, ARROWS_OUT, "Scale", "v0.5.0", layout);
        later_tile(ui, ERASER, "Erase", "v0.5.0", layout);
    });
}

fn measure_group(ui: &mut Ui, app: &mut MyCadApp, layout: RibbonLayout) {
    group_column(ui, "Measure", layout, |ui| {
        let idle = !app.command_is_active();
        if icon_tile(ui, RULER, "Distance", idle, "DI", layout).clicked() {
            app.start_distance_command();
        }
        later_tile(ui, ANGLE, "Angle", "v0.6.0", layout);
        later_tile(ui, CIRCLE, "Radius", "v0.6.0", layout);
        later_tile(ui, POLYGON, "Area", "v0.6.0", layout);
    });
}

fn layer_group(ui: &mut Ui, app: &mut MyCadApp, layout: RibbonLayout) {
    group_column(ui, "Layer", layout, |ui| {
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
        let scale = layout.tile.x / 38.0;
        paint_swatch(ui, current_color, scale);
        ui.label(RichText::new(STACK).size((14.0 * scale).clamp(11.0, 26.0)));
        let mut chosen = None;
        egui::ComboBox::from_id_salt("home-current-layer")
            .selected_text(current.clone())
            .width(96.0 * scale)
            .height(layout.tile.y)
            .show_ui(ui, |ui| {
                for (name, frozen, color) in &layers {
                    ui.horizontal(|ui| {
                        paint_swatch(ui, *color, scale);
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
            layout,
        )
        .clicked()
        {
            app.set_selected_layer_current();
        }
    });
}

fn later_tile(ui: &mut Ui, icon: &str, label: &str, release: &str, layout: RibbonLayout) {
    let _ = icon_tile(
        ui,
        icon,
        label,
        false,
        &format!("Coming in {release}"),
        layout,
    );
}

fn group_column(
    ui: &mut Ui,
    title: &str,
    layout: RibbonLayout,
    add_contents: impl FnOnce(&mut Ui),
) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        ui.label(RichText::new(title).size(layout.caption_size).weak());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(1.0, 0.0);
            add_contents(ui);
        });
    });
}

fn compact_separator(ui: &mut Ui, layout: RibbonLayout) {
    let height = layout.tile.y + layout.caption_size;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(6.0, height), Sense::hover());
    let x = rect.center().x;
    ui.painter().vline(
        x,
        rect.y_range().shrink(3.0),
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
}

fn icon_tile(
    ui: &mut Ui,
    icon: &str,
    label: &str,
    enabled: bool,
    hint: &str,
    layout: RibbonLayout,
) -> egui::Response {
    let tile = layout.tile;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(tile, sense);
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
        FontId::proportional(layout.icon_size),
        icon_color,
    );
    ui.painter().text(
        Pos2::new(rect.center().x, rect.bottom() - 7.0),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(layout.label_size),
        label_color,
    );
    response
}

fn paint_swatch(ui: &mut Ui, color: CadColor, scale: f32) {
    let rgb = color.resolve(color, CadColor::Aci(7));
    let side = (10.0 * scale).clamp(8.0, 20.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 1.0, Color32::from_rgb(rgb.r, rgb.g, rgb.b));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_home_uses_compact_mode() {
        assert!(layout_for_height(44.0).compact);
        assert!(!layout_for_height(45.0).compact);
    }

    #[test]
    fn icon_layout_scales_monotonically_and_is_clamped() {
        let small = layout_for_height(48.0);
        let medium = layout_for_height(80.0);
        let large = layout_for_height(500.0);
        assert!(medium.tile.y > small.tile.y);
        assert!(large.tile.y > medium.tile.y);
        assert!((large.tile.x - 91.2).abs() < 1e-5);
        assert!((large.tile.y - 72.0).abs() < 1e-5);
        assert_eq!(small.icon_size, 11.0);
        assert_eq!(large.icon_size, 28.0);
    }
}
