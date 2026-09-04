//! Compact viewport right-click menu.

use eframe::egui::{self, Area, Frame, Id, Order, Pos2, Ui};

use crate::commands::{CommandKind, CommandState};

// ------------------------------------------------------------
// Enum: ContextAction
// Purpose: One menu item the viewport can run without also
//          accepting a point from the same click.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextAction {
    Finish,
    UndoLast,
    Close,
    Back,
    Cancel,
    Properties,
    SetCurrentLayerFromObject,
    Deselect,
    RepeatLast,
    ZoomExtents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextKind {
    Line,
    Polyline,
        TwoPointDraw,
        AreaPoints,
        Entity,
    Empty,
}

#[derive(Debug, Clone)]
pub struct ViewportMenu {
    pub pos: Pos2,
    pub kind: ContextKind,
    pub just_opened: bool,
}

impl ViewportMenu {
    pub fn new(pos: Pos2, kind: ContextKind) -> Self {
        Self {
            pos,
            kind,
            just_opened: true,
        }
    }
}

pub fn kind_for_state(state: &CommandState) -> Option<ContextKind> {
    if matches!(state, CommandState::Area(crate::commands::AreaState::Points { .. })) {
        return Some(ContextKind::AreaPoints);
    }
    kind_for_command(state.kind())
}

pub fn kind_for_command(kind: CommandKind) -> Option<ContextKind> {
    match kind {
        CommandKind::Line => Some(ContextKind::Line),
        CommandKind::Polyline => Some(ContextKind::Polyline),
        CommandKind::Circle
        | CommandKind::Arc
        | CommandKind::Rectangle
        | CommandKind::Distance
        | CommandKind::Angle
        | CommandKind::Radius
        | CommandKind::Area => Some(ContextKind::TwoPointDraw),
        CommandKind::Idle => None,
    }
}

pub enum MenuResult {
    StayOpen,
    Dismissed,
    Action(ContextAction),
}

pub fn show(
    ctx: &egui::Context,
    menu: &mut ViewportMenu,
    can_finish: bool,
    can_close: bool,
    can_undo: bool,
    can_back: bool,
    can_set_layer: bool,
    can_repeat: bool,
) -> MenuResult {
    let mut action = None;
    let area = Area::new(Id::new("mycad-viewport-context"))
        .order(Order::Foreground)
        .fixed_pos(menu.pos)
        .constrain_to(ctx.screen_rect())
        .show(ctx, |ui| {
            Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(168.0);
                match menu.kind {
                    ContextKind::Line => {
                        item(ui, "Finish", can_finish, &mut action, ContextAction::Finish);
                        item(
                            ui,
                            "Undo last segment",
                            can_undo,
                            &mut action,
                            ContextAction::UndoLast,
                        );
                        item(ui, "Close", can_close, &mut action, ContextAction::Close);
                    }
                    ContextKind::Polyline => {
                        item(ui, "Finish", can_finish, &mut action, ContextAction::Finish);
                        item(
                            ui,
                            "Undo last vertex",
                            can_undo,
                            &mut action,
                            ContextAction::UndoLast,
                        );
                        item(ui, "Close", can_close, &mut action, ContextAction::Close);
                    }
                    ContextKind::AreaPoints => {
                        item(ui, "Finish", can_finish, &mut action, ContextAction::Finish);
                        item(
                            ui,
                            "Undo last point",
                            can_undo,
                            &mut action,
                            ContextAction::UndoLast,
                        );
                        item(ui, "Cancel", true, &mut action, ContextAction::Cancel);
                    }
                    ContextKind::TwoPointDraw => {
                        item(ui, "Back", can_back, &mut action, ContextAction::Back);
                        item(ui, "Cancel", true, &mut action, ContextAction::Cancel);
                    }
                    ContextKind::Entity => {
                        item(
                            ui,
                            "Properties",
                            true,
                            &mut action,
                            ContextAction::Properties,
                        );
                        item(
                            ui,
                            "Set current layer from object",
                            can_set_layer,
                            &mut action,
                            ContextAction::SetCurrentLayerFromObject,
                        );
                        item(ui, "Deselect", true, &mut action, ContextAction::Deselect);
                    }
                    ContextKind::Empty => {
                        item(
                            ui,
                            "Repeat last command",
                            can_repeat,
                            &mut action,
                            ContextAction::RepeatLast,
                        );
                        item(
                            ui,
                            "Zoom Extents",
                            true,
                            &mut action,
                            ContextAction::ZoomExtents,
                        );
                    }
                }
            });
        });
    if let Some(action) = action {
        return MenuResult::Action(action);
    }
    if menu.just_opened {
        menu.just_opened = false;
        MenuResult::StayOpen
    } else if area.response.clicked_elsewhere() {
        MenuResult::Dismissed
    } else {
        MenuResult::StayOpen
    }
}

fn item(
    ui: &mut Ui,
    label: &str,
    enabled: bool,
    action: &mut Option<ContextAction>,
    value: ContextAction,
) {
    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
        *action = Some(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawing_commands_map_to_expected_menus() {
        assert_eq!(kind_for_command(CommandKind::Line), Some(ContextKind::Line));
        assert_eq!(
            kind_for_command(CommandKind::Polyline),
            Some(ContextKind::Polyline)
        );
        assert_eq!(
            kind_for_command(CommandKind::Circle),
            Some(ContextKind::TwoPointDraw)
        );
        assert_eq!(kind_for_command(CommandKind::Idle), None);
        assert_eq!(
            kind_for_command(CommandKind::Distance),
            Some(ContextKind::TwoPointDraw)
        );
    }
}
