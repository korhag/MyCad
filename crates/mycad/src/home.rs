//! Dockable Home toolbar backed by the adaptive ribbon engine.
//!
//! Save is a compact quick-access control on the menu bar, not a Home group.

use cad_core::CadColor;
use eframe::egui::{Color32, Ui};
use egui_phosphor::regular::{
    ANGLE, ARROWS_CLOCKWISE, ARROWS_OUT, ARROWS_OUT_CARDINAL, ARROW_CLOCKWISE,
    ARROW_COUNTER_CLOCKWISE, CIRCLE, COPY, ERASER, FLIP_HORIZONTAL, LINE_SEGMENT, LINE_SEGMENTS,
    PATH, POLYGON, RECTANGLE, RULER,
};

use crate::app::MyCadApp;
use crate::commands::CommandKind;
use crate::ribbon::{self, LayerChoice, LayerState, RibbonAction, RibbonCommand, RibbonGroup};

const CMD_UNDO: &str = "undo";
const CMD_REDO: &str = "redo";
const CMD_LINE: &str = "line";
const CMD_POLYLINE: &str = "polyline";
const CMD_CIRCLE: &str = "circle";
const CMD_ARC: &str = "arc";
const CMD_RECTANGLE: &str = "rectangle";
const CMD_DISTANCE: &str = "distance";
const CMD_ANGLE: &str = "angle";
const CMD_RADIUS: &str = "radius";
const CMD_AREA: &str = "area";
const CMD_MOVE: &str = "move";
const CMD_COPY: &str = "copy";
const CMD_ROTATE: &str = "rotate";
const CMD_MIRROR: &str = "mirror";
const CMD_SCALE: &str = "scale";
const CMD_ERASE: &str = "erase";

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
    let kind = app.command_kind();
    let line_active = kind == CommandKind::Line;
    let polyline_active = kind == CommandKind::Polyline;
    let circle_active = kind == CommandKind::Circle;
    let arc_active = kind == CommandKind::Arc;
    let rectangle_active = kind == CommandKind::Rectangle;
    let distance_active = kind == CommandKind::Distance;
    let angle_active = kind == CommandKind::Angle;
    let radius_active = kind == CommandKind::Radius;
    let area_active = kind == CommandKind::Area;
    let move_active = kind == CommandKind::Move;
    let copy_active = kind == CommandKind::Copy;
    let rotate_active = kind == CommandKind::Rotate;
    let mirror_active = kind == CommandKind::Mirror;
    let scale_active = kind == CommandKind::Scale;
    let erase_active = kind == CommandKind::Erase;
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
                    true,
                    line_active,
                ),
                command(
                    CMD_POLYLINE,
                    "Polyline",
                    "PLine",
                    LINE_SEGMENTS,
                    "P",
                    1,
                    true,
                    polyline_active,
                ),
                command(
                    CMD_CIRCLE,
                    "Circle",
                    "Circle",
                    CIRCLE,
                    "C",
                    2,
                    true,
                    circle_active,
                ),
                command(
                    CMD_ARC,
                    "Arc",
                    "Arc",
                    PATH,
                    "A",
                    3,
                    true,
                    arc_active,
                ),
                command(
                    CMD_RECTANGLE,
                    "Rectangle",
                    "Rect",
                    RECTANGLE,
                    "R",
                    4,
                    true,
                    rectangle_active,
                ),
            ],
        },
        RibbonGroup {
            id: "modify",
            name: "Modify",
            commands: vec![
                command(
                    CMD_MOVE,
                    "Move",
                    "Move",
                    ARROWS_OUT_CARDINAL,
                    "Move",
                    0,
                    true,
                    move_active,
                ),
                command(
                    CMD_COPY,
                    "Copy",
                    "Copy",
                    COPY,
                    "Copy",
                    1,
                    true,
                    copy_active,
                ),
                command(
                    CMD_ROTATE,
                    "Rotate",
                    "Rotate",
                    ARROWS_CLOCKWISE,
                    "Rotate",
                    2,
                    true,
                    rotate_active,
                ),
                command(
                    CMD_MIRROR,
                    "Mirror",
                    "Mirror",
                    FLIP_HORIZONTAL,
                    "Mirror",
                    3,
                    true,
                    mirror_active,
                ),
                command(
                    CMD_SCALE,
                    "Scale",
                    "Scale",
                    ARROWS_OUT,
                    "Scale",
                    4,
                    true,
                    scale_active,
                ),
                command(
                    CMD_ERASE,
                    "Erase",
                    "Erase",
                    ERASER,
                    "Delete",
                    5,
                    true,
                    erase_active,
                ),
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
                    true,
                    distance_active,
                ),
                command(
                    CMD_ANGLE,
                    "Angle",
                    "Angle",
                    ANGLE,
                    "Angle",
                    1,
                    true,
                    angle_active,
                ),
                command(
                    CMD_RADIUS,
                    "Radius",
                    "Radius",
                    CIRCLE,
                    "Radius",
                    2,
                    true,
                    radius_active,
                ),
                command(
                    CMD_AREA,
                    "Area",
                    "Area",
                    POLYGON,
                    "Area",
                    3,
                    true,
                    area_active,
                ),
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

#[cfg(test)]
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

#[cfg(test)]
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
        RibbonAction::Command(CMD_POLYLINE) => app.start_polyline_command(),
        RibbonAction::Command(CMD_CIRCLE) => app.start_circle_command(),
        RibbonAction::Command(CMD_ARC) => app.start_arc_command(),
        RibbonAction::Command(CMD_RECTANGLE) => app.start_rectangle_command(),
        RibbonAction::Command(CMD_DISTANCE) => app.start_distance_command(),
        RibbonAction::Command(CMD_ANGLE) => app.start_angle_command(),
        RibbonAction::Command(CMD_RADIUS) => app.start_radius_command(),
        RibbonAction::Command(CMD_AREA) => app.start_area_command(),
        RibbonAction::Command(CMD_MOVE) => app.start_move_command(),
        RibbonAction::Command(CMD_COPY) => app.start_copy_command(),
        RibbonAction::Command(CMD_ROTATE) => app.start_rotate_command(),
        RibbonAction::Command(CMD_MIRROR) => app.start_mirror_command(),
        RibbonAction::Command(CMD_SCALE) => app.start_scale_command(),
        RibbonAction::Command(CMD_ERASE) => app.start_erase_command(),
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
                    command(
                        CMD_POLYLINE,
                        "Polyline",
                        "PLine",
                        LINE_SEGMENTS,
                        "P",
                        1,
                        idle,
                        false,
                    ),
                ],
            },
            "Modify" => RibbonGroup {
                id: "modify",
                name: "Modify",
                commands: vec![command(
                    CMD_MOVE,
                    "Move",
                    "Move",
                    ARROWS_OUT_CARDINAL,
                    "Move",
                    0,
                    idle,
                    false,
                )],
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
        let compact = crate::ribbon::metrics_for(RibbonDensity::Compact, 80.0);
        assert!(compact.show_icons);
        assert!(compact.icon_text_gap > 0.0);
        assert_eq!(compact.rows, 1);
    }

    #[test]
    fn home_does_not_grow_a_save_group() {
        assert!(
            !matches_priority("Save", "Save", 0),
            "Save belongs on the menu-bar quick-access control"
        );
    }

    #[test]
    fn draw_modify_measure_stay_clickable_while_another_command_is_active() {
        let mut app = crate::app::MyCadApp::for_test();
        app.start_line_command();
        let groups = home_groups(&app);
        let mut saw_line = false;
        let mut saw_circle = false;
        for group in &groups {
            if matches!(group.id, "draw" | "modify" | "measure") {
                for command in &group.commands {
                    assert!(
                        command.enabled,
                        "{} should stay clickable while LINE is active",
                        command.label
                    );
                }
            }
        }
        for group in &groups {
            for command in &group.commands {
                if command.id == CMD_LINE {
                    saw_line = true;
                    assert!(command.active);
                    assert!(command.enabled);
                }
                if command.id == CMD_CIRCLE {
                    saw_circle = true;
                    assert!(!command.active);
                    assert!(command.enabled);
                }
            }
        }
        assert!(saw_line && saw_circle);
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
