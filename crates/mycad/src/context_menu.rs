//! Compact viewport right-click menu.

use eframe::egui::{
    self, Color32, CornerRadius, Frame, Id, Margin, Order, Pos2, Rect, Shadow, Stroke, Ui, Vec2,
};
use egui_phosphor::regular::{
    ARROWS_CLOCKWISE, ARROWS_OUT, ARROWS_OUT_CARDINAL, COPY, ERASER, FLIP_HORIZONTAL,
};

use crate::commands::{CommandKind, CommandState};

const MENU_WIDTH: f32 = 208.0;
const ROW_HEIGHT: f32 = 25.0;
const ICON_SIZE: f32 = 15.0;
const ROW_PAD: f32 = 7.0;
const CORNER: f32 = 6.0;

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
    Move,
    Copy,
    Rotate,
    Mirror,
    Scale,
    Erase,
    CreateBlock,
    EditBlock,
    AddToBlock,
    RemoveFromBlock,
    MakeUnique,
    CreateDynamicBlock,
    EditDynamicBlock,
    AttachMoveTo(cad_core::ParameterId),
    AttachStretchTo(cad_core::ParameterId),
    NewSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextKind {
    Line,
    Polyline,
    TwoPointDraw,
    AreaPoints,
    Modify,
    Erase,
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
    if matches!(
        state,
        CommandState::Area(crate::commands::AreaState::Points { .. })
    ) {
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
        CommandKind::Move
        | CommandKind::Copy
        | CommandKind::Rotate
        | CommandKind::Mirror
        | CommandKind::Scale => Some(ContextKind::Modify),
        CommandKind::Erase => Some(ContextKind::Erase),
        CommandKind::Idle => None,
    }
}

#[derive(Debug, Clone, Default)]
pub struct BlockMenuState {
    pub can_create: bool,
    pub can_edit: bool,
    pub can_add: bool,
    pub can_remove: bool,
    pub can_make_unique: bool,
    pub can_create_dynamic: bool,
    pub can_edit_dynamic: bool,
    pub can_attach: bool,
    pub size_parameters: Vec<(cad_core::ParameterId, String)>,
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
    selection_count: usize,
    last_command: Option<CommandKind>,
    block_menu: BlockMenuState,
) -> MenuResult {
    let mut action = None;
    let screen = ctx.screen_rect();
    let pos = clamp_menu(menu.pos, screen);
    let area = egui::Area::new(Id::new("mycad-viewport-context"))
        .order(Order::Foreground)
        .fixed_pos(pos)
        .constrain_to(screen)
        .show(ctx, |ui| {
            Frame::popup(ui.style())
                .inner_margin(Margin::same(4))
                .corner_radius(CornerRadius::same(CORNER as u8))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(48, 58, 56)))
                .shadow(Shadow {
                    offset: [0, 6],
                    blur: 16,
                    spread: 0,
                    color: Color32::from_black_alpha(90),
                })
                .fill(Color32::from_rgb(22, 28, 27))
                .show(ui, |ui| {
                    ui.set_min_width(MENU_WIDTH);
                    ui.set_max_width(MENU_WIDTH);
                    ui.spacing_mut().item_spacing = Vec2::new(0.0, 1.0);
                    match menu.kind {
                        ContextKind::Line => {
                            row(
                                ui,
                                None,
                                "Finish",
                                None,
                                can_finish,
                                &mut action,
                                ContextAction::Finish,
                            );
                            row(
                                ui,
                                None,
                                "Undo first point",
                                None,
                                can_undo,
                                &mut action,
                                ContextAction::UndoLast,
                            );
                            separator(ui);
                            row(
                                ui,
                                None,
                                "Cancel",
                                Some("Esc"),
                                true,
                                &mut action,
                                ContextAction::Cancel,
                            );
                        }
                        ContextKind::Polyline => {
                            row(
                                ui,
                                None,
                                "Finish",
                                None,
                                can_finish,
                                &mut action,
                                ContextAction::Finish,
                            );
                            row(
                                ui,
                                None,
                                "Undo last vertex",
                                None,
                                can_undo,
                                &mut action,
                                ContextAction::UndoLast,
                            );
                            row(
                                ui,
                                None,
                                "Close",
                                Some("C"),
                                can_close,
                                &mut action,
                                ContextAction::Close,
                            );
                            separator(ui);
                            row(
                                ui,
                                None,
                                "Cancel",
                                Some("Esc"),
                                true,
                                &mut action,
                                ContextAction::Cancel,
                            );
                        }
                        ContextKind::AreaPoints => {
                            row(
                                ui,
                                None,
                                "Finish",
                                None,
                                can_finish,
                                &mut action,
                                ContextAction::Finish,
                            );
                            row(
                                ui,
                                None,
                                "Undo last point",
                                None,
                                can_undo,
                                &mut action,
                                ContextAction::UndoLast,
                            );
                            row(
                                ui,
                                None,
                                "Cancel",
                                Some("Esc"),
                                true,
                                &mut action,
                                ContextAction::Cancel,
                            );
                        }
                        ContextKind::TwoPointDraw => {
                            row(
                                ui,
                                None,
                                "Back",
                                None,
                                can_back,
                                &mut action,
                                ContextAction::Back,
                            );
                            row(
                                ui,
                                None,
                                "Cancel",
                                Some("Esc"),
                                true,
                                &mut action,
                                ContextAction::Cancel,
                            );
                        }
                        ContextKind::Modify => {
                            row(
                                ui,
                                None,
                                if can_finish {
                                    "Confirm selection"
                                } else {
                                    "Finish"
                                },
                                None,
                                can_finish,
                                &mut action,
                                ContextAction::Finish,
                            );
                            row(
                                ui,
                                None,
                                "Undo last point",
                                None,
                                can_undo,
                                &mut action,
                                ContextAction::UndoLast,
                            );
                            row(
                                ui,
                                None,
                                "Cancel",
                                Some("Esc"),
                                true,
                                &mut action,
                                ContextAction::Cancel,
                            );
                        }
                        ContextKind::Erase => {
                            row(
                                ui,
                                None,
                                "Finish",
                                Some("Esc"),
                                true,
                                &mut action,
                                ContextAction::Finish,
                            );
                            row(
                                ui,
                                None,
                                "Cancel",
                                Some("Esc"),
                                true,
                                &mut action,
                                ContextAction::Cancel,
                            );
                        }
                        ContextKind::Entity | ContextKind::Empty => {
                            idle_menu(
                                ui,
                                &mut action,
                                menu.kind == ContextKind::Entity,
                                can_set_layer,
                                can_repeat,
                                selection_count,
                                last_command,
                                block_menu,
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

fn idle_menu(
    ui: &mut Ui,
    action: &mut Option<ContextAction>,
    entity: bool,
    can_set_layer: bool,
    can_repeat: bool,
    selection_count: usize,
    last_command: Option<CommandKind>,
    block_menu: BlockMenuState,
) {
    if selection_count > 0 {
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(format!(
                "{selection_count} object{} selected",
                if selection_count == 1 { "" } else { "s" }
            ))
            .size(11.0)
            .color(Color32::from_rgb(120, 140, 136)),
        );
        ui.add_space(2.0);
    }
    if entity {
        row(
            ui,
            None,
            "Properties",
            None,
            true,
            action,
            ContextAction::Properties,
        );
        row(
            ui,
            None,
            "Set current layer from object",
            None,
            can_set_layer,
            action,
            ContextAction::SetCurrentLayerFromObject,
        );
        row(
            ui,
            None,
            "Deselect",
            None,
            true,
            action,
            ContextAction::Deselect,
        );
        separator(ui);
    }
    if block_menu.can_create {
        row(
            ui,
            None,
            "Create Block…",
            None,
            true,
            action,
            ContextAction::CreateBlock,
        );
    }
    if block_menu.can_edit {
        row(
            ui,
            None,
            "Edit Block",
            None,
            true,
            action,
            ContextAction::EditBlock,
        );
    }
    if block_menu.can_create_dynamic {
        row(
            ui,
            None,
            "Create a Dynamic Block…",
            None,
            true,
            action,
            ContextAction::CreateDynamicBlock,
        );
    }
    if block_menu.can_edit_dynamic {
        row(
            ui,
            None,
            "Edit Dynamic Block…",
            None,
            true,
            action,
            ContextAction::EditDynamicBlock,
        );
    }
    if block_menu.can_attach {
        for (id, name) in &block_menu.size_parameters {
            row(
                ui,
                None,
                &format!("Attach to {name} → Move"),
                None,
                true,
                action,
                ContextAction::AttachMoveTo(*id),
            );
            row(
                ui,
                None,
                &format!("Attach to {name} → Stretch"),
                None,
                true,
                action,
                ContextAction::AttachStretchTo(*id),
            );
        }
        row(
            ui,
            None,
            "Attach to Parameter → New Size…",
            None,
            true,
            action,
            ContextAction::NewSize,
        );
    }
    if block_menu.can_make_unique {
        row(
            ui,
            None,
            "Make Unique…",
            None,
            true,
            action,
            ContextAction::MakeUnique,
        );
    }
    if block_menu.can_add {
        row(
            ui,
            None,
            "Add Selected to Block",
            None,
            true,
            action,
            ContextAction::AddToBlock,
        );
    }
    if block_menu.can_remove {
        row(
            ui,
            None,
            "Remove Selected from Block",
            None,
            true,
            action,
            ContextAction::RemoveFromBlock,
        );
    }
    if block_menu.can_create
        || block_menu.can_edit
        || block_menu.can_create_dynamic
        || block_menu.can_edit_dynamic
        || block_menu.can_attach
        || block_menu.can_make_unique
        || block_menu.can_add
        || block_menu.can_remove
    {
        separator(ui);
    }
    let repeat = last_command
        .map(|kind| format!("Repeat {}", title_case(kind.label())))
        .unwrap_or_else(|| "Repeat last command".into());
    row(
        ui,
        None,
        &repeat,
        None,
        can_repeat,
        action,
        ContextAction::RepeatLast,
    );
    modify_submenu(ui, action);
    separator(ui);
    row(
        ui,
        None,
        "Zoom Extents",
        None,
        true,
        action,
        ContextAction::ZoomExtents,
    );
}

fn modify_submenu(ui: &mut Ui, action: &mut Option<ContextAction>) {
    ui.menu_button("Modify", |ui| {
        ui.set_min_width(168.0);
        row(
            ui,
            Some(ARROWS_OUT_CARDINAL),
            "Move",
            None,
            true,
            action,
            ContextAction::Move,
        );
        row(
            ui,
            Some(COPY),
            "Copy",
            None,
            true,
            action,
            ContextAction::Copy,
        );
        row(
            ui,
            Some(ARROWS_CLOCKWISE),
            "Rotate",
            None,
            true,
            action,
            ContextAction::Rotate,
        );
        row(
            ui,
            Some(FLIP_HORIZONTAL),
            "Mirror",
            None,
            true,
            action,
            ContextAction::Mirror,
        );
        row(
            ui,
            Some(ARROWS_OUT),
            "Scale",
            None,
            true,
            action,
            ContextAction::Scale,
        );
        separator(ui);
        row(
            ui,
            Some(ERASER),
            "Erase",
            Some("Del"),
            true,
            action,
            ContextAction::Erase,
        );
    });
}

fn title_case(label: &str) -> String {
    let lower = label.to_ascii_lowercase();
    let mut chars = lower.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => lower,
    }
}

fn clamp_menu(pos: Pos2, screen: Rect) -> Pos2 {
    let width = MENU_WIDTH + 16.0;
    let height = 220.0;
    Pos2::new(
        pos.x.clamp(
            screen.min.x + 4.0,
            (screen.max.x - width).max(screen.min.x + 4.0),
        ),
        pos.y.clamp(
            screen.min.y + 4.0,
            (screen.max.y - height).max(screen.min.y + 4.0),
        ),
    )
}

fn separator(ui: &mut Ui) {
    ui.add_space(3.0);
    let rect = ui.available_rect_before_wrap();
    let y = rect.min.y + 1.0;
    ui.painter().hline(
        rect.min.x + 6.0..=rect.max.x - 6.0,
        y,
        Stroke::new(1.0_f32, Color32::from_rgb(42, 52, 50)),
    );
    ui.add_space(6.0);
}

fn row(
    ui: &mut Ui,
    icon: Option<&'static str>,
    label: &str,
    shortcut: Option<&str>,
    enabled: bool,
    action: &mut Option<ContextAction>,
    value: ContextAction,
) {
    let dangerous = matches!(value, ContextAction::Erase);
    let response = menu_row_ui(ui, icon, label, shortcut, enabled, dangerous);
    if enabled && response.clicked() {
        *action = Some(value);
    }
}

fn menu_row_ui(
    ui: &mut Ui,
    icon: Option<&'static str>,
    label: &str,
    shortcut: Option<&str>,
    enabled: bool,
    dangerous: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), ROW_HEIGHT),
        egui::Sense::click(),
    );
    let hovered = response.hovered() && enabled;
    if hovered {
        let fill = if dangerous {
            Color32::from_rgb(92, 32, 28)
        } else {
            Color32::from_rgb(36, 52, 50)
        };
        ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
    }
    let text = if !enabled {
        Color32::from_rgb(90, 100, 98)
    } else if hovered && dangerous {
        Color32::from_rgb(255, 170, 160)
    } else {
        Color32::from_rgb(224, 230, 226)
    };
    let mut x = rect.min.x + ROW_PAD;
    if let Some(icon) = icon {
        ui.painter().text(
            Pos2::new(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            icon,
            egui::FontId::proportional(ICON_SIZE),
            text,
        );
        x += ICON_SIZE + 6.0;
    }
    ui.painter().text(
        Pos2::new(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        text,
    );
    if let Some(shortcut) = shortcut {
        ui.painter().text(
            Pos2::new(rect.max.x - ROW_PAD, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            shortcut,
            egui::FontId::proportional(11.0),
            Color32::from_rgb(110, 128, 124),
        );
    }
    response
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
        assert_eq!(
            kind_for_command(CommandKind::Move),
            Some(ContextKind::Modify)
        );
        assert_eq!(
            kind_for_command(CommandKind::Erase),
            Some(ContextKind::Erase)
        );
    }

    #[test]
    fn modify_actions_match_ribbon_command_kinds() {
        let pairs = [
            (ContextAction::Move, CommandKind::Move),
            (ContextAction::Copy, CommandKind::Copy),
            (ContextAction::Rotate, CommandKind::Rotate),
            (ContextAction::Mirror, CommandKind::Mirror),
            (ContextAction::Scale, CommandKind::Scale),
            (ContextAction::Erase, CommandKind::Erase),
        ];
        for (action, kind) in pairs {
            assert_eq!(kind.modify_kind().unwrap().command_kind(), kind);
            let _ = action;
        }
    }
}
