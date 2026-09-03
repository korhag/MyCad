//! Rebindable mouse and keyboard actions. Bindings are exact about modifiers.

use std::fmt;

use eframe::egui::{self, PointerButton};
use serde::{Deserialize, Serialize};

// ------------------------------------------------------------
// Type: InputAction
// Purpose: User-facing commands that can be rebound in Settings.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputAction {
    SelectReplace,
    SelectToggle,
    SelectClear,
    Pan,
    ZoomExtents,
}

impl InputAction {
    pub const ALL: [Self; 5] = [
        Self::SelectReplace,
        Self::SelectToggle,
        Self::SelectClear,
        Self::Pan,
        Self::ZoomExtents,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::SelectReplace => "Select",
            Self::SelectToggle => "Add / remove from selection",
            Self::SelectClear => "Clear selection",
            Self::Pan => "Pan",
            Self::ZoomExtents => "Zoom extents",
        }
    }

    pub fn group(self) -> &'static str {
        match self {
            Self::SelectReplace | Self::SelectToggle | Self::SelectClear => "Selection",
            Self::Pan | Self::ZoomExtents => "View",
        }
    }

    pub fn prefers_drag(self) -> bool {
        matches!(self, Self::Pan)
    }
}

// ------------------------------------------------------------
// Type: MouseButtonKind
// Purpose: Serializable mouse button identity.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButtonKind {
    Left,
    Middle,
    Right,
}

impl MouseButtonKind {
    pub fn from_egui(button: PointerButton) -> Option<Self> {
        match button {
            PointerButton::Primary => Some(Self::Left),
            PointerButton::Middle => Some(Self::Middle),
            PointerButton::Secondary => Some(Self::Right),
            PointerButton::Extra1 | PointerButton::Extra2 => None,
        }
    }

    #[allow(dead_code)]
    pub fn to_egui(self) -> PointerButton {
        match self {
            Self::Left => PointerButton::Primary,
            Self::Middle => PointerButton::Middle,
            Self::Right => PointerButton::Secondary,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Middle => "Middle",
            Self::Right => "Right",
        }
    }
}

// ------------------------------------------------------------
// Type: Gesture
// Purpose: How the binding is activated.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gesture {
    #[default]
    Click,
    DoubleClick,
    Drag,
    Key,
}

// ------------------------------------------------------------
// Type: Binding
// Purpose: One concrete shortcut. Modifier flags are exact: Ctrl+Click
//          does not also fire a no-modifier Click binding.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Binding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mouse: Option<MouseButtonKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub gesture: Gesture,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub command: bool,
}

impl Default for Binding {
    fn default() -> Self {
        Self {
            mouse: None,
            key: None,
            gesture: Gesture::Click,
            ctrl: false,
            shift: false,
            alt: false,
            command: false,
        }
    }
}

impl Binding {
    pub fn key(name: &str) -> Self {
        Self {
            key: Some(name.to_string()),
            gesture: Gesture::Key,
            ..Self::default()
        }
    }

    pub fn click(button: MouseButtonKind) -> Self {
        Self {
            mouse: Some(button),
            gesture: Gesture::Click,
            ..Self::default()
        }
    }

    pub fn double_click(button: MouseButtonKind) -> Self {
        Self {
            mouse: Some(button),
            gesture: Gesture::DoubleClick,
            ..Self::default()
        }
    }

    pub fn drag(button: MouseButtonKind) -> Self {
        Self {
            mouse: Some(button),
            gesture: Gesture::Drag,
            ..Self::default()
        }
    }

    pub fn with_ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    pub fn with_shift(mut self) -> Self {
        self.shift = true;
        self
    }

    pub fn with_command(mut self) -> Self {
        self.command = true;
        self
    }

    pub fn is_valid(&self) -> bool {
        match self.gesture {
            Gesture::Key => self.key.as_deref().and_then(key_from_name).is_some(),
            Gesture::Click | Gesture::DoubleClick | Gesture::Drag => self.mouse.is_some(),
        }
    }

    pub fn conflicts_with(&self, other: &Self) -> bool {
        self.normalized() == other.normalized()
    }

    fn normalized(&self) -> Self {
        let mut clone = self.clone();
        if let Some(key) = clone.key.as_mut() {
            *key = key.trim().to_ascii_lowercase();
        }
        clone
    }

    pub fn matches_modifiers(&self, modifiers: egui::Modifiers) -> bool {
        if self.shift != modifiers.shift || self.alt != modifiers.alt {
            return false;
        }
        let want_ctrl_family = self.ctrl || self.command;
        if cfg!(target_os = "macos") || modifiers.mac_cmd {
            self.command == modifiers.command && self.ctrl == modifiers.ctrl
        } else {
            want_ctrl_family == (modifiers.ctrl || modifiers.command)
        }
    }

    pub fn matches_click(&self, button: PointerButton, modifiers: egui::Modifiers) -> bool {
        self.gesture == Gesture::Click
            && self.mouse == MouseButtonKind::from_egui(button)
            && self.matches_modifiers(modifiers)
    }

    pub fn matches_double_click(&self, button: PointerButton, modifiers: egui::Modifiers) -> bool {
        self.gesture == Gesture::DoubleClick
            && self.mouse == MouseButtonKind::from_egui(button)
            && self.matches_modifiers(modifiers)
    }

    pub fn matches_drag(&self, button: PointerButton, modifiers: egui::Modifiers) -> bool {
        self.gesture == Gesture::Drag
            && self.mouse == MouseButtonKind::from_egui(button)
            && self.matches_modifiers(modifiers)
    }

    pub fn matches_key(&self, input: &egui::InputState) -> bool {
        if self.gesture != Gesture::Key {
            return false;
        }
        let Some(name) = self.key.as_deref() else {
            return false;
        };
        let Some(key) = key_from_name(name) else {
            return false;
        };
        input.key_pressed(key) && self.matches_modifiers(input.modifiers)
    }
}

impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<&str> = Vec::new();
        if self.ctrl || (self.command && !cfg!(target_os = "macos")) {
            parts.push("Ctrl");
        } else if self.command {
            parts.push("Cmd");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.alt {
            parts.push("Alt");
        }
        let prefix = join_mods(&parts);
        let connector = if prefix.is_empty() { "" } else { "+" };
        match self.gesture {
            Gesture::Key => {
                let key = self
                    .key
                    .as_deref()
                    .map(display_key_name)
                    .unwrap_or_else(|| "?".into());
                write!(f, "{prefix}{connector}{key}")
            }
            Gesture::Click => match self.mouse {
                Some(button) => write!(f, "{prefix}{connector}{} click", button.label()),
                None => write!(f, "(invalid)"),
            },
            Gesture::DoubleClick => match self.mouse {
                Some(button) => write!(f, "{prefix}{connector}{} double-click", button.label()),
                None => write!(f, "(invalid)"),
            },
            Gesture::Drag => match self.mouse {
                Some(button) => write!(f, "{prefix}{connector}{} drag", button.label()),
                None => write!(f, "(invalid)"),
            },
        }
    }
}

fn join_mods(parts: &[&str]) -> String {
    parts.join("+")
}

// ------------------------------------------------------------
// Type: InputMap
// Purpose: All rebindable actions. Missing actions fall back to defaults.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputMap {
    pub select_replace: Vec<Binding>,
    pub select_toggle: Vec<Binding>,
    pub select_clear: Vec<Binding>,
    pub pan: Vec<Binding>,
    pub zoom_extents: Vec<Binding>,
}

impl Default for InputMap {
    fn default() -> Self {
        Self::standard()
    }
}

impl InputMap {
    pub fn standard() -> Self {
        Self {
            select_replace: vec![
                Binding::click(MouseButtonKind::Left),
                Binding::drag(MouseButtonKind::Left),
            ],
            select_toggle: vec![
                Binding::click(MouseButtonKind::Left).with_ctrl(),
                Binding::click(MouseButtonKind::Left).with_shift(),
                Binding::drag(MouseButtonKind::Left).with_ctrl(),
                Binding::drag(MouseButtonKind::Left).with_shift(),
            ],
            select_clear: vec![Binding::key("escape")],
            pan: vec![Binding::drag(MouseButtonKind::Middle)],
            zoom_extents: vec![
                Binding::double_click(MouseButtonKind::Left),
                Binding::key("e").with_command(),
            ],
        }
    }

    fn list(&self, action: InputAction) -> &Vec<Binding> {
        match action {
            InputAction::SelectReplace => &self.select_replace,
            InputAction::SelectToggle => &self.select_toggle,
            InputAction::SelectClear => &self.select_clear,
            InputAction::Pan => &self.pan,
            InputAction::ZoomExtents => &self.zoom_extents,
        }
    }

    fn list_mut(&mut self, action: InputAction) -> &mut Vec<Binding> {
        match action {
            InputAction::SelectReplace => &mut self.select_replace,
            InputAction::SelectToggle => &mut self.select_toggle,
            InputAction::SelectClear => &mut self.select_clear,
            InputAction::Pan => &mut self.pan,
            InputAction::ZoomExtents => &mut self.zoom_extents,
        }
    }

    pub fn sanitize(&mut self) {
        let defaults = Self::standard();
        for action in InputAction::ALL {
            let entries = self.list_mut(action);
            entries.retain(Binding::is_valid);
            if entries.is_empty() {
                *entries = defaults.list(action).clone();
            }
        }
        self.ensure_select_drag_companions();
    }

    // Click-select and box-select share a button. Older saved maps only
    // stored the click, which made left-drag a no-op after the threshold.
    fn ensure_select_drag_companions(&mut self) {
        for action in [InputAction::SelectReplace, InputAction::SelectToggle] {
            let companions: Vec<Binding> = self
                .list(action)
                .iter()
                .filter(|binding| binding.gesture == Gesture::Click && binding.mouse.is_some())
                .map(|click| {
                    let mut drag = click.clone();
                    drag.gesture = Gesture::Drag;
                    drag
                })
                .collect();
            for drag in companions {
                let taken = InputAction::ALL.iter().any(|other| {
                    self.list(*other)
                        .iter()
                        .any(|binding| binding.conflicts_with(&drag))
                });
                if !taken {
                    self.list_mut(action).push(drag);
                }
            }
        }
    }

    pub fn bindings_for(&self, action: InputAction) -> &[Binding] {
        self.list(action)
    }

    pub fn bindings_for_mut(&mut self, action: InputAction) -> &mut Vec<Binding> {
        self.list_mut(action)
    }

    pub fn reset_action(&mut self, action: InputAction) {
        *self.list_mut(action) = Self::standard().list(action).clone();
    }

    pub fn reset_all(&mut self) {
        *self = Self::standard();
    }

    pub fn conflicts(&self) -> Vec<(InputAction, InputAction, Binding)> {
        let items: Vec<(InputAction, Binding)> = InputAction::ALL
            .iter()
            .flat_map(|action| {
                self.list(*action)
                    .iter()
                    .cloned()
                    .map(move |binding| (*action, binding))
            })
            .collect();
        let mut out = Vec::new();
        for (i, (left_action, left)) in items.iter().enumerate() {
            for (right_action, right) in items.iter().skip(i + 1) {
                if left.conflicts_with(right) {
                    out.push((*left_action, *right_action, left.clone()));
                }
            }
        }
        out
    }

    pub fn clicked(
        &self,
        action: InputAction,
        button: PointerButton,
        modifiers: egui::Modifiers,
    ) -> bool {
        self.bindings_for(action)
            .iter()
            .any(|binding| binding.matches_click(button, modifiers))
    }

    pub fn double_clicked(
        &self,
        action: InputAction,
        button: PointerButton,
        modifiers: egui::Modifiers,
    ) -> bool {
        self.bindings_for(action)
            .iter()
            .any(|binding| binding.matches_double_click(button, modifiers))
    }

    pub fn dragged(
        &self,
        action: InputAction,
        button: PointerButton,
        modifiers: egui::Modifiers,
    ) -> bool {
        self.bindings_for(action)
            .iter()
            .any(|binding| binding.matches_drag(button, modifiers))
    }

    pub fn selects_with_pointer(
        &self,
        action: InputAction,
        button: PointerButton,
        modifiers: egui::Modifiers,
    ) -> bool {
        self.clicked(action, button, modifiers) || self.dragged(action, button, modifiers)
    }

    pub fn key_pressed(&self, action: InputAction, input: &egui::InputState) -> bool {
        self.bindings_for(action)
            .iter()
            .any(|binding| binding.matches_key(input))
    }

    #[allow(dead_code)]
    pub fn pointer_button_for_drag(&self, action: InputAction) -> Option<PointerButton> {
        self.bindings_for(action).iter().find_map(|binding| {
            if binding.gesture == Gesture::Drag {
                binding.mouse.map(MouseButtonKind::to_egui)
            } else {
                None
            }
        })
    }
}

pub fn key_from_name(name: &str) -> Option<egui::Key> {
    let name = name.trim().to_ascii_lowercase();
    match name.as_str() {
        "escape" | "esc" => Some(egui::Key::Escape),
        "enter" | "return" => Some(egui::Key::Enter),
        "space" | "spacebar" => Some(egui::Key::Space),
        "tab" => Some(egui::Key::Tab),
        "backspace" => Some(egui::Key::Backspace),
        "delete" | "del" => Some(egui::Key::Delete),
        "insert" => Some(egui::Key::Insert),
        "home" => Some(egui::Key::Home),
        "end" => Some(egui::Key::End),
        "pageup" | "page_up" => Some(egui::Key::PageUp),
        "pagedown" | "page_down" => Some(egui::Key::PageDown),
        "left" | "arrowleft" => Some(egui::Key::ArrowLeft),
        "right" | "arrowright" => Some(egui::Key::ArrowRight),
        "up" | "arrowup" => Some(egui::Key::ArrowUp),
        "down" | "arrowdown" => Some(egui::Key::ArrowDown),
        "f1" => Some(egui::Key::F1),
        "f2" => Some(egui::Key::F2),
        "f3" => Some(egui::Key::F3),
        "f4" => Some(egui::Key::F4),
        "f5" => Some(egui::Key::F5),
        "f6" => Some(egui::Key::F6),
        "f7" => Some(egui::Key::F7),
        "f8" => Some(egui::Key::F8),
        "f9" => Some(egui::Key::F9),
        "f10" => Some(egui::Key::F10),
        "f11" => Some(egui::Key::F11),
        "f12" => Some(egui::Key::F12),
        "0" | "num0" => Some(egui::Key::Num0),
        "1" | "num1" => Some(egui::Key::Num1),
        "2" | "num2" => Some(egui::Key::Num2),
        "3" | "num3" => Some(egui::Key::Num3),
        "4" | "num4" => Some(egui::Key::Num4),
        "5" | "num5" => Some(egui::Key::Num5),
        "6" | "num6" => Some(egui::Key::Num6),
        "7" | "num7" => Some(egui::Key::Num7),
        "8" | "num8" => Some(egui::Key::Num8),
        "9" | "num9" => Some(egui::Key::Num9),
        "a" => Some(egui::Key::A),
        "b" => Some(egui::Key::B),
        "c" => Some(egui::Key::C),
        "d" => Some(egui::Key::D),
        "e" => Some(egui::Key::E),
        "f" => Some(egui::Key::F),
        "g" => Some(egui::Key::G),
        "h" => Some(egui::Key::H),
        "i" => Some(egui::Key::I),
        "j" => Some(egui::Key::J),
        "k" => Some(egui::Key::K),
        "l" => Some(egui::Key::L),
        "m" => Some(egui::Key::M),
        "n" => Some(egui::Key::N),
        "o" => Some(egui::Key::O),
        "p" => Some(egui::Key::P),
        "q" => Some(egui::Key::Q),
        "r" => Some(egui::Key::R),
        "s" => Some(egui::Key::S),
        "t" => Some(egui::Key::T),
        "u" => Some(egui::Key::U),
        "v" => Some(egui::Key::V),
        "w" => Some(egui::Key::W),
        "x" => Some(egui::Key::X),
        "y" => Some(egui::Key::Y),
        "z" => Some(egui::Key::Z),
        _ => None,
    }
}

pub fn key_to_name(key: egui::Key) -> String {
    format!("{key:?}").to_lowercase()
}

pub fn display_key_name(name: &str) -> String {
    match name.trim().to_ascii_lowercase().as_str() {
        "escape" | "esc" => "Esc".into(),
        "enter" | "return" => "Enter".into(),
        "pageup" | "page_up" => "Page Up".into(),
        "pagedown" | "page_down" => "Page Down".into(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => other.to_string(),
            }
        }
    }
}

pub fn capture_binding(input: &egui::InputState, prefer_drag: bool) -> Option<Binding> {
    let modifiers = input.modifiers;
    for key in input.keys_down.iter() {
        if input.key_pressed(*key) {
            let mut binding = Binding::key(&key_to_name(*key));
            apply_modifiers(&mut binding, modifiers);
            return Some(binding);
        }
    }
    for (egui_button, kind) in [
        (PointerButton::Primary, MouseButtonKind::Left),
        (PointerButton::Middle, MouseButtonKind::Middle),
        (PointerButton::Secondary, MouseButtonKind::Right),
    ] {
        if input.pointer.button_double_clicked(egui_button) {
            let mut binding = Binding::double_click(kind);
            apply_modifiers(&mut binding, modifiers);
            return Some(binding);
        }
        if input.pointer.button_clicked(egui_button) {
            let mut binding = if prefer_drag {
                Binding::drag(kind)
            } else {
                Binding::click(kind)
            };
            apply_modifiers(&mut binding, modifiers);
            return Some(binding);
        }
    }
    None
}

fn apply_modifiers(binding: &mut Binding, modifiers: egui::Modifiers) {
    binding.shift = modifiers.shift;
    binding.alt = modifiers.alt;
    if cfg!(target_os = "macos") || modifiers.mac_cmd {
        binding.command = modifiers.command;
        binding.ctrl = modifiers.ctrl;
    } else {
        binding.command = modifiers.command || modifiers.ctrl;
        binding.ctrl = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(ctrl: bool, shift: bool, alt: bool, command: bool) -> egui::Modifiers {
        egui::Modifiers {
            alt,
            ctrl,
            shift,
            mac_cmd: false,
            command,
        }
    }

    #[test]
    fn unmodified_click_does_not_match_ctrl_click() {
        let select = Binding::click(MouseButtonKind::Left);
        let toggle = Binding::click(MouseButtonKind::Left).with_ctrl();
        let none = mods(false, false, false, false);
        let ctrl = mods(true, false, false, true);
        assert!(select.matches_click(PointerButton::Primary, none));
        assert!(!select.matches_click(PointerButton::Primary, ctrl));
        assert!(toggle.matches_click(PointerButton::Primary, ctrl));
        assert!(!toggle.matches_click(PointerButton::Primary, none));
    }

    #[test]
    fn shift_and_ctrl_are_distinct_toggle_bindings() {
        let ctrl = Binding::click(MouseButtonKind::Left).with_ctrl();
        let shift = Binding::click(MouseButtonKind::Left).with_shift();
        assert!(!ctrl.conflicts_with(&shift));
        assert!(ctrl.matches_click(PointerButton::Primary, mods(true, false, false, true)));
        assert!(shift.matches_click(PointerButton::Primary, mods(false, true, false, false)));
        assert!(!ctrl.matches_click(PointerButton::Primary, mods(false, true, false, false)));
    }

    #[test]
    fn standard_map_includes_left_drag_select_without_conflicts() {
        let map = InputMap::standard();
        assert!(map.conflicts().is_empty());
        assert!(map
            .bindings_for(InputAction::SelectReplace)
            .iter()
            .any(|b| b.gesture == Gesture::Drag && b.mouse == Some(MouseButtonKind::Left)));
        assert!(map.dragged(
            InputAction::SelectReplace,
            PointerButton::Primary,
            mods(false, false, false, false)
        ));
        assert!(!map.dragged(
            InputAction::Pan,
            PointerButton::Primary,
            mods(false, false, false, false)
        ));
    }

    #[test]
    fn conflict_detection_finds_duplicate_shortcuts() {
        let mut map = InputMap::standard();
        map.bindings_for_mut(InputAction::SelectClear)
            .push(Binding::click(MouseButtonKind::Left));
        let conflicts = map.conflicts();
        assert!(conflicts
            .iter()
            .any(
                |(a, b, _)| (*a == InputAction::SelectReplace && *b == InputAction::SelectClear)
                    || (*a == InputAction::SelectClear && *b == InputAction::SelectReplace)
            ));
    }

    #[test]
    fn sanitize_restores_missing_actions() {
        let mut map = InputMap {
            select_replace: Vec::new(),
            select_toggle: Vec::new(),
            select_clear: Vec::new(),
            pan: Vec::new(),
            zoom_extents: Vec::new(),
        };
        map.sanitize();
        assert!(!map.bindings_for(InputAction::SelectReplace).is_empty());
        assert!(!map.bindings_for(InputAction::SelectClear).is_empty());
    }

    #[test]
    fn sanitize_adds_drag_companion_for_saved_select_click() {
        let mut map = InputMap {
            select_replace: vec![Binding::click(MouseButtonKind::Left)],
            select_toggle: vec![Binding::click(MouseButtonKind::Left).with_ctrl()],
            select_clear: vec![Binding::key("escape")],
            pan: vec![Binding::drag(MouseButtonKind::Middle)],
            zoom_extents: vec![Binding::double_click(MouseButtonKind::Left)],
        };
        map.sanitize();
        let none = mods(false, false, false, false);
        let ctrl = mods(true, false, false, true);
        assert!(map.dragged(InputAction::SelectReplace, PointerButton::Primary, none));
        assert!(map.dragged(InputAction::SelectToggle, PointerButton::Primary, ctrl));
        assert!(map.selects_with_pointer(InputAction::SelectReplace, PointerButton::Primary, none));
    }

    #[test]
    fn sanitize_does_not_override_existing_pan_drag() {
        let mut map = InputMap {
            select_replace: vec![Binding::click(MouseButtonKind::Left)],
            select_toggle: Vec::new(),
            select_clear: vec![Binding::key("escape")],
            pan: vec![Binding::drag(MouseButtonKind::Left)],
            zoom_extents: vec![Binding::double_click(MouseButtonKind::Left)],
        };
        map.sanitize();
        let none = mods(false, false, false, false);
        assert!(map.dragged(InputAction::Pan, PointerButton::Primary, none));
        assert!(!map.dragged(InputAction::SelectReplace, PointerButton::Primary, none));
    }
}
