//! Dockable Home toolbar backed by the adaptive ribbon engine.

use cad_core::CadColor;
use eframe::egui::{Color32, Ui};
use egui_phosphor::regular::{
    ANGLE, ARROWS_CLOCKWISE, ARROWS_OUT, ARROWS_OUT_CARDINAL, ARROW_CLOCKWISE,
    ARROW_COUNTER_CLOCKWISE, CIRCLE, COPY, ERASER, FLIP_HORIZONTAL, LINE_SEGMENT, LINE_SEGMENTS,
    PATH, POLYGON, RECTANGLE, RULER,
};

use crate::app::MyCadApp;
use crate::ribbon::{self, LayerChoice, LayerState, RibbonAction, RibbonCommand, RibbonGroup};

const CMD_UNDO: &str = "undo";
const CMD_REDO: &str = "redo";
const CMD_LINE: &str = "line";
const CMD_DISTANCE: &str = "distance";

pub fn show(ui: &mut Ui, app: &mut MyCadApp) {
    let available = ui.available_size();
    let density = ribbon::density_for(ui, available.y);
    let metrics = ribbon::metrics_for(density, available.y);
    let groups = home_groups(app);
    let layer = layer_state(app);
    if let Some(action) = ribbon::show(ui, &groups, Some(&layer), &metrics) {
        dispatch(app, action);
    }
}

fn home_groups(app: &MyCadApp) -> Vec<RibbonGroup> {
    let idle = !app.command_is_active();
    let line_active = app.line_command_is_active();
    let distance_active = app.distance_command_is_active();
    vec![
        RibbonGroup {
            id: "history",
            name: "History",
            commands: vec![
                command(
                    CMD_UNDO,
                    "Undo",
                    "Undo",
                    ARROW_COUNTER_CLOCKWISE,
                    "Ctrl+Z",
                    0,
                    app.can_undo(),
                    false,
                ),
                command(
                    CMD_REDO,
                    "Redo",
                    "Redo",
                    ARROW_CLOCKWISE,
                    "Ctrl+Y",
                    1,
                    app.can_redo(),
                    false,
                ),
            ],
        },
        RibbonGroup {
            id: "draw",
            name: "Draw",
            commands: vec![
                command(
                    CMD_LINE,
                    "Line",
                    "Line",
                    LINE_SEGMENT,
                    "L",
                    0,
                    idle || line_active,
                    line_active,
                ),
                later(LINE_SEGMENTS, "Polyline", "PLine", "v0.4.0", 1),
                later(CIRCLE, "Circle", "Circle", "v0.4.0", 2),
                later(PATH, "Arc", "Arc", "v0.4.0", 3),
                later(RECTANGLE, "Rectangle", "Rect", "v0.4.0", 4),
            ],
        },
        RibbonGroup {
            id: "modify",
            name: "Modify",
            commands: vec![
                later(ARROWS_OUT_CARDINAL, "Move", "Move", "v0.5.0", 0),
                later(COPY, "Copy", "Copy", "v0.5.0", 1),
                later(ARROWS_CLOCKWISE, "Rotate", "Rotate", "v0.5.0", 2),
                later(FLIP_HORIZONTAL, "Mirror", "Mirror", "v0.5.0", 3),
                later(ARROWS_OUT, "Scale", "Scale", "v0.5.0", 4),
                later(ERASER, "Erase", "Erase", "v0.5.0", 5),
            ],
        },
        RibbonGroup {
            id: "measure",
            name: "Measure",
            commands: vec![
                command(
                    CMD_DISTANCE,
                    "Distance",
                    "Dist",
                    RULER,
                    "DI",
                    0,
                    idle || distance_active,
                    distance_active,
                ),
                later(ANGLE, "Angle", "Angle", "v0.6.0", 1),
                later(CIRCLE, "Radius", "Radius", "v0.6.0", 2),
                later(POLYGON, "Area", "Area", "v0.6.0", 3),
            ],
        },
    ]
}

#[allow(clippy::too_many_arguments)]
fn command(
    id: &'static str,
    label: &'static str,
    short_label: &'static str,
    icon: &'static str,
    tooltip: &'static str,
    priority: u8,
    enabled: bool,
    active: bool,
) -> RibbonCommand {
    RibbonCommand {
        id,
        label,
        short_label,
        icon,
        tooltip,
        priority,
        enabled,
        active,
    }
}

fn later(
    icon: &'static str,
    label: &'static str,
    short_label: &'static str,
    release: &'static str,
    priority: u8,
) -> RibbonCommand {
    RibbonCommand {
        id: label,
        label,
        short_label,
        icon,
        tooltip: later_tooltip(release),
        priority,
        enabled: false,
        active: false,
    }
}

fn later_tooltip(release: &'static str) -> &'static str {
    match release {
        "v0.4.0" => "Coming in v0.4.0",
        "v0.5.0" => "Coming in v0.5.0",
        "v0.6.0" => "Coming in v0.6.0",
        _ => "Coming in a later release",
    }
}

fn layer_state(app: &MyCadApp) -> LayerState {
    let Some(document) = app.document.as_ref() else {
        return LayerState {
            current: String::new(),
            current_color: Color32::GRAY,
            layers: Vec::new(),
            can_set_from_selected: false,
            has_document: false,
        };
    };
    let current = document.current_layer.clone();
    let layers: Vec<LayerChoice> = document
        .layers
        .values()
        .map(|layer| LayerChoice {
            name: layer.name.clone(),
            frozen: layer.frozen,
            color: swatch_color(layer.color),
        })
        .collect();
    let current_color = layers
        .iter()
        .find(|layer| layer.name == current)
        .map(|layer| layer.color)
        .unwrap_or(swatch_color(CadColor::Aci(7)));
    let can_set = app
        .selection
        .shared_layer(document)
        .is_some_and(|layer| document.layer_can_be_current(&layer));
    LayerState {
        current,
        current_color,
        layers,
        can_set_from_selected: can_set,
        has_document: true,
    }
}

fn swatch_color(color: CadColor) -> Color32 {
    let rgb = color.resolve(color, CadColor::Aci(7));
    Color32::from_rgb(rgb.r, rgb.g, rgb.b)
}

fn dispatch(app: &mut MyCadApp, action: RibbonAction) {
    match action {
        RibbonAction::Command(CMD_UNDO) => app.undo(),
        RibbonAction::Command(CMD_REDO) => app.redo(),
        RibbonAction::Command(CMD_LINE) => app.start_line_command(),
        RibbonAction::Command(CMD_DISTANCE) => app.start_distance_command(),
        RibbonAction::Command(_) => {}
        RibbonAction::SetLayer(name) => app.set_current_layer(&name),
        RibbonAction::SetSelectedLayerCurrent => app.set_selected_layer_current(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ribbon::{density_from_height, pack_ribbon, RibbonDensity};

    #[test]
    fn home_keeps_line_and_distance_as_highest_draw_measure_priority() {
        let groups = [
            ("Draw", "Line", 0),
            ("Draw", "Polyline", 1),
            ("Modify", "Move", 0),
            ("Measure", "Distance", 0),
            ("Measure", "Area", 3),
        ];
        for (group_name, label, priority) in groups {
            assert!(
                matches_priority(group_name, label, priority),
                "{group_name} {label} should keep priority {priority}"
            );
        }
    }

    fn matches_priority(group_name: &str, label: &str, priority: u8) -> bool {
        // Catalog shape is independent of app state besides enablement.
        let idle = true;
        let group = match group_name {
            "Draw" => RibbonGroup {
                id: "draw",
                name: "Draw",
                commands: vec![
                    command(CMD_LINE, "Line", "Line", LINE_SEGMENT, "L", 0, idle, false),
                    later(LINE_SEGMENTS, "Polyline", "PLine", "v0.4.0", 1),
                ],
            },
            "Modify" => RibbonGroup {
                id: "modify",
                name: "Modify",
                commands: vec![later(ARROWS_OUT_CARDINAL, "Move", "Move", "v0.5.0", 0)],
            },
            "Measure" => RibbonGroup {
                id: "measure",
                name: "Measure",
                commands: vec![
                    command(
                        CMD_DISTANCE,
                        "Distance",
                        "Dist",
                        RULER,
                        "DI",
                        0,
                        idle,
                        false,
                    ),
                    later(POLYGON, "Area", "Area", "v0.6.0", 3),
                ],
            },
            _ => return false,
        };
        group
            .commands
            .iter()
            .any(|command| command.label == label && command.priority == priority)
    }

    #[test]
    fn home_never_uses_icon_above_text_density() {
        assert_ne!(density_from_height(80.0), RibbonDensity::Micro);
        let expanded = crate::ribbon::metrics_for(RibbonDensity::Expanded, 80.0);
        assert!(expanded.show_icons);
        assert!(expanded.icon_text_gap > 0.0);
    }

    #[test]
    fn later_tools_can_collapse_before_line() {
        use crate::ribbon::{MeasuredCommand, MeasuredGroup};
        let groups = vec![MeasuredGroup {
            commands: vec![
                MeasuredCommand {
                    width: 50.0,
                    priority: 0,
                },
                MeasuredCommand {
                    width: 70.0,
                    priority: 4,
                },
            ],
            overflow_width: 44.0,
            caption_width: 0.0,
        }];
        let packed = pack_ribbon(&groups, 100.0, 2.0, 5.0, 1);
        assert_eq!(packed[0].visible, vec![0]);
        assert_eq!(packed[0].overflow, vec![1]);
    }
}
