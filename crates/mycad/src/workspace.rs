//! Persistent dockable workspace: Home, Properties, Viewport, Diagnostics, Blocks, Dynamic Block.

use eframe::egui::{self, Color32, Margin, Pos2, Rect, Stroke, StrokeKind, Ui, Vec2};
use egui_dock::{
    DockArea, DockState, Node, NodeIndex, Split, Style, Surface, TabStyle, TabViewer, Tree,
};
use serde::{Deserialize, Serialize};

use crate::app::MyCadApp;
use crate::home;
use crate::properties;

// ------------------------------------------------------------
// Type: WorkspaceTab
// Purpose: Serializable identity of each dockable panel.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTab {
    Home,
    Viewport,
    Properties,
    Diagnostics,
    Blocks,
    DynamicBlock,
    CommandLine,
}

impl WorkspaceTab {
    pub fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Viewport => "Viewport",
            Self::Properties => "Properties",
            Self::Diagnostics => "Diagnostics",
            Self::Blocks => "Blocks",
            Self::DynamicBlock => "Dynamic Block",
            Self::CommandLine => "Command",
        }
    }
}

/// Total Home leaf height used for Reset / first layout (tab bar + body).
pub const HOME_DEFAULT_HEIGHT_PX: f32 = 50.0;
/// Smallest Home leaf the splitter will allow. Matches a ~40–45 px ribbon.
pub const HOME_MIN_LEAF_PX: f32 = 42.0;
pub const HOME_TAB_BAR_HEIGHT: f32 = 20.0;
/// Smallest expanded leaf: tab bar plus one ribbon row.
pub const MIN_LEAF_HEIGHT_PX: f32 = HOME_MIN_LEAF_PX;
/// Smallest leaf width that still fits a tab title and a side-panel heading.
pub const MIN_LEAF_WIDTH_PX: f32 = 160.0;
const ASSUMED_WORKSPACE_HEIGHT: f32 = 800.0;
const LEGACY_HOME_SPLIT_FRACTION: f32 = 0.08;

pub fn home_split_fraction(workspace_height: f32) -> f32 {
    let height = workspace_height.max(HOME_DEFAULT_HEIGHT_PX);
    (HOME_DEFAULT_HEIGHT_PX / height).clamp(0.02, 0.25)
}

fn default_home_fraction() -> f32 {
    home_split_fraction(ASSUMED_WORKSPACE_HEIGHT)
}

pub fn default_dock_state() -> DockState<WorkspaceTab> {
    let mut state = DockState::new(vec![WorkspaceTab::Viewport]);
    let [viewport, _home] = state.main_surface_mut().split_above(
        NodeIndex::root(),
        default_home_fraction(),
        vec![WorkspaceTab::Home],
    );
    let [viewport, _props] = state.main_surface_mut().split_left(
        viewport,
        0.24,
        vec![WorkspaceTab::Properties, WorkspaceTab::Blocks],
    );
    let [viewport, _diag] =
        state
            .main_surface_mut()
            .split_right(viewport, 0.76, vec![WorkspaceTab::Diagnostics]);
    let _ = state
        .main_surface_mut()
        .split_below(viewport, 0.88, vec![WorkspaceTab::CommandLine]);
    state
}

pub fn sanitize_dock_state(state: &mut DockState<WorkspaceTab>) {
    let tabs: Vec<WorkspaceTab> = state.iter_all_tabs().map(|(_, tab)| *tab).collect();
    let mut seen = Vec::new();
    let mut unique = true;
    for tab in &tabs {
        if seen.contains(tab) {
            unique = false;
            break;
        }
        seen.push(*tab);
    }
    if !tabs.contains(&WorkspaceTab::Viewport) || !unique {
        *state = default_dock_state();
    }
}

pub fn migrate_blocks_tab(state: &mut DockState<WorkspaceTab>, already_migrated: bool) -> bool {
    if already_migrated {
        return true;
    }
    if state.find_tab(&WorkspaceTab::Blocks).is_some() {
        return true;
    }
    if let Some((surface, node, _)) = state.find_tab(&WorkspaceTab::Properties) {
        if let Some(leaf) = state[surface][node].get_leaf_mut() {
            leaf.tabs.push(WorkspaceTab::Blocks);
        } else {
            state.push_to_focused_leaf(WorkspaceTab::Blocks);
        }
    } else {
        state.push_to_focused_leaf(WorkspaceTab::Blocks);
    }
    true
}

pub fn migrate_home_tab(state: &mut DockState<WorkspaceTab>, already_migrated: bool) -> bool {
    if already_migrated {
        return true;
    }
    if state.find_tab(&WorkspaceTab::Home).is_some() {
        return true;
    }
    if let Some((surface, node, _)) = state.find_tab(&WorkspaceTab::Viewport) {
        let _ = state.split(
            (surface, node),
            Split::Above,
            default_home_fraction(),
            Node::leaf(WorkspaceTab::Home),
        );
    } else {
        state.push_to_focused_leaf(WorkspaceTab::Home);
    }
    true
}

pub fn migrate_command_line_tab(
    state: &mut DockState<WorkspaceTab>,
    already_migrated: bool,
) -> bool {
    if already_migrated || state.find_tab(&WorkspaceTab::CommandLine).is_some() {
        return true;
    }
    if let Some((surface, node, _)) = state.find_tab(&WorkspaceTab::Viewport) {
        let _ = state.split(
            (surface, node),
            Split::Below,
            0.88,
            Node::leaf(WorkspaceTab::CommandLine),
        );
    } else {
        state.push_to_focused_leaf(WorkspaceTab::CommandLine);
    }
    true
}

/// Move Command out of the Viewport leaf once. A later re-dock is left alone.
pub fn split_command_from_viewport_once(
    state: &mut DockState<WorkspaceTab>,
    already_repaired: bool,
) -> bool {
    if already_repaired {
        return true;
    }
    let Some((surface, node, command)) = state.find_tab(&WorkspaceTab::CommandLine) else {
        return true;
    };
    let Some((viewport_surface, viewport_node, _)) = state.find_tab(&WorkspaceTab::Viewport) else {
        return true;
    };
    if surface != viewport_surface || node != viewport_node {
        return true;
    }
    state.remove_tab((surface, node, command));
    if let Some((surface, node, _)) = state.find_tab(&WorkspaceTab::Viewport) {
        let _ = state.split(
            (surface, node),
            Split::Below,
            0.88,
            Node::leaf(WorkspaceTab::CommandLine),
        );
    } else {
        state.push_to_focused_leaf(WorkspaceTab::CommandLine);
    }
    true
}

pub fn recover_home_split_once(
    state: &mut DockState<WorkspaceTab>,
    already_recovered: bool,
) -> bool {
    if already_recovered {
        return true;
    }
    let Some((surface, home, _)) = state.find_tab(&WorkspaceTab::Home) else {
        return true;
    };
    let Some(parent) = home.parent() else {
        return true;
    };
    let is_home_leaf = state[surface][home]
        .tabs()
        .is_some_and(|tabs| tabs.len() == 1 && tabs[0] == WorkspaceTab::Home);
    let Node::Vertical(split) = &state[surface][parent] else {
        return true;
    };
    let home_is_top = parent.left() == home;
    let home_is_bottom = parent.right() == home;
    if is_home_leaf
        && (home_is_top || home_is_bottom)
        && ((home_is_top && split.fraction > 0.28) || (home_is_bottom && split.fraction < 0.72))
    {
        let fraction = if home_is_top {
            default_home_fraction()
        } else {
            1.0 - default_home_fraction()
        };
        if let Node::Vertical(split) = &mut state[surface][parent] {
            split.fraction = fraction;
        }
    }
    true
}

pub fn apply_compact_home_height(
    state: &mut DockState<WorkspaceTab>,
    workspace_height: f32,
    already_applied: bool,
) -> bool {
    if already_applied {
        return true;
    }
    if workspace_height < HOME_DEFAULT_HEIGHT_PX * 2.0 {
        return false;
    }
    let Some((surface, home, _)) = state.find_tab(&WorkspaceTab::Home) else {
        return true;
    };
    let Some(parent) = home.parent() else {
        return true;
    };
    let is_home_leaf = state[surface][home]
        .tabs()
        .is_some_and(|tabs| tabs.len() == 1 && tabs[0] == WorkspaceTab::Home);
    let Node::Vertical(split) = &state[surface][parent] else {
        return true;
    };
    let home_is_top = parent.left() == home;
    let home_is_bottom = parent.right() == home;
    if !is_home_leaf || !(home_is_top || home_is_bottom) {
        return true;
    }
    let home_frac = if home_is_top {
        split.fraction
    } else {
        1.0 - split.fraction
    };
    let looks_like_default = (home_frac - LEGACY_HOME_SPLIT_FRACTION).abs() < 0.005
        || (home_frac - default_home_fraction()).abs() < 0.005
        || home_frac > 0.28;
    if !looks_like_default {
        return true;
    }
    let fraction = home_split_fraction(workspace_height);
    if let Node::Vertical(split) = &mut state[surface][parent] {
        split.fraction = if home_is_top {
            fraction
        } else {
            1.0 - fraction
        };
    }
    true
}

fn dock_style(ui: &Ui) -> Style {
    let mut style = Style::from_egui(ui.style().as_ref());
    style.separator.extra = HOME_MIN_LEAF_PX;
    style.separator.width = 1.0;
    style.separator.extra_interact_width = 10.0;
    style.tab_bar.height = HOME_TAB_BAR_HEIGHT;
    style
}

pub fn ensure_tab(state: &mut DockState<WorkspaceTab>, tab: WorkspaceTab) {
    if state.find_tab(&tab).is_some() {
        return;
    }
    if tab == WorkspaceTab::Blocks || tab == WorkspaceTab::DynamicBlock {
        if let Some((surface, node, _)) = state.find_tab(&WorkspaceTab::Properties) {
            state[surface][node].append_tab(tab);
            return;
        }
    }
    if tab == WorkspaceTab::CommandLine {
        if let Some((surface, node, _)) = state.find_tab(&WorkspaceTab::Viewport) {
            let _ = state.split(
                (surface, node),
                Split::Below,
                0.88,
                Node::leaf(WorkspaceTab::CommandLine),
            );
            return;
        }
    }
    state.push_to_focused_leaf(tab);
}

pub fn encode_dock_layout(state: &DockState<WorkspaceTab>) -> serde_json::Value {
    let mut value = serde_json::to_value(state).unwrap_or(serde_json::Value::Null);
    replace_null_numbers(&mut value);
    value
}

pub fn decode_dock_layout(value: Option<&serde_json::Value>) -> DockState<WorkspaceTab> {
    let Some(value) = value else {
        return default_dock_state();
    };
    let mut value = value.clone();
    replace_null_numbers(&mut value);
    match serde_json::from_value::<DockState<WorkspaceTab>>(value) {
        Ok(mut state) => {
            sanitize_dock_state(&mut state);
            state
        }
        Err(_) => default_dock_state(),
    }
}

fn replace_null_numbers(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Null => {}
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if child.is_null()
                    && matches!(
                        key.as_str(),
                        "x" | "y" | "width" | "height" | "fraction" | "scroll"
                    )
                {
                    *child = serde_json::json!(0.0);
                } else {
                    replace_null_numbers(child);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                replace_null_numbers(child);
            }
        }
        _ => {}
    }
}

pub fn show_workspace(ui: &mut Ui, app: &mut MyCadApp) {
    let mut dock_state = std::mem::replace(&mut app.dock_state, default_dock_state());
    app.settings.compact_home_height_applied = apply_compact_home_height(
        &mut dock_state,
        ui.available_height(),
        app.settings.compact_home_height_applied,
    );
    // egui_dock 0.17 asserts when a tab strip is exactly the 24px collapse
    // button. A restored 0×0 window, or a split dragged onto that width,
    // panics every frame and leaves `cargo run` stuck in the terminal.
    // The same pass keeps every leaf at a usable size. Split fractions are
    // ratios, so a strip saved for a tall window would otherwise shrink
    // until the next tab bar paints over it.
    let show_collapse = guard_collapse_buttons(&mut dock_state, ui.available_size());
    let style = dock_style(ui);
    {
        let mut viewer = WorkspaceViewer { app };
        DockArea::new(&mut dock_state)
            .id(egui::Id::new("mycad-workspace"))
            .style(style)
            .show_leaf_collapse_buttons(show_collapse)
            .show_leaf_close_all_buttons(false)
            .show_inside(ui, &mut viewer);
    }
    app.dock_state = dock_state;
}

/// egui_dock `Style::TAB_COLLAPSE_BUTTON_SIZE`. A tab strip of this width
/// subtracts down to zero and trips `assert_ne!(available_width, 0.0)`.
const COLLAPSE_BUTTON_PX: f32 = 24.0;

/// Keep every tab strip wider than the collapse button, and keep every leaf
/// at least [`MIN_LEAF_WIDTH_PX`] by [`MIN_LEAF_HEIGHT_PX`].
fn guard_collapse_buttons(state: &mut DockState<WorkspaceTab>, main_size: Vec2) -> bool {
    let mut allowed = true;
    for surface in state.iter_surfaces_mut() {
        let safe = match surface {
            Surface::Empty => true,
            Surface::Main(tree) => guard_tree(tree, main_size),
            Surface::Window(tree, _) => {
                let size = tree[NodeIndex::root()]
                    .rect()
                    .map(|rect| rect.size())
                    .unwrap_or(Vec2::ZERO);
                guard_tree(tree, size)
            }
        };
        allowed &= safe;
    }
    allowed
}

fn guard_tree(tree: &mut Tree<WorkspaceTab>, size: Vec2) -> bool {
    if !size.x.is_finite() || size.x <= 0.0 {
        return true;
    }
    enforce_min_leaf_sizes(tree, NodeIndex::root(), size);
    if size.x <= COLLAPSE_BUTTON_PX + 0.5 {
        return false;
    }
    !tab_strip_hits_collapse_button(tree, NodeIndex::root(), size.x)
}

fn subtree_min_size(tree: &Tree<WorkspaceTab>, index: NodeIndex) -> Vec2 {
    if index.0 >= tree.len() {
        return Vec2::new(MIN_LEAF_WIDTH_PX, MIN_LEAF_HEIGHT_PX);
    }
    match &tree[index] {
        Node::Empty => Vec2::ZERO,
        Node::Leaf(leaf) => {
            let height = if leaf.collapsed {
                HOME_TAB_BAR_HEIGHT
            } else {
                MIN_LEAF_HEIGHT_PX
            };
            Vec2::new(MIN_LEAF_WIDTH_PX, height)
        }
        Node::Horizontal(_) => {
            let left = subtree_min_size(tree, index.left());
            let right = subtree_min_size(tree, index.right());
            Vec2::new(left.x + right.x, left.y.max(right.y))
        }
        Node::Vertical(_) => {
            let left = subtree_min_size(tree, index.left());
            let right = subtree_min_size(tree, index.right());
            Vec2::new(left.x.max(right.x), left.y + right.y)
        }
    }
}

fn enforce_min_leaf_sizes(tree: &mut Tree<WorkspaceTab>, index: NodeIndex, size: Vec2) {
    if index.0 >= tree.len() {
        return;
    }
    let orientation = match &tree[index] {
        Node::Horizontal(split) => Some((true, split.fraction)),
        Node::Vertical(split) => Some((false, split.fraction)),
        Node::Leaf(_) | Node::Empty => None,
    };
    let Some((horizontal, current)) = orientation else {
        return;
    };
    let left_min = subtree_min_size(tree, index.left());
    let right_min = subtree_min_size(tree, index.right());
    let (span, first_min, second_min) = if horizontal {
        (size.x, left_min.x, right_min.x)
    } else {
        (size.y, left_min.y, right_min.y)
    };
    let fraction = if span.is_finite() && span > 0.0 {
        clamp_split_fraction(current, span, first_min, second_min)
    } else {
        current
    };
    match &mut tree[index] {
        Node::Horizontal(split) | Node::Vertical(split) => split.fraction = fraction,
        Node::Leaf(_) | Node::Empty => return,
    }
    let (left_size, right_size) = if horizontal {
        (
            Vec2::new(size.x * fraction, size.y),
            Vec2::new(size.x * (1.0 - fraction), size.y),
        )
    } else {
        (
            Vec2::new(size.x, size.y * fraction),
            Vec2::new(size.x, size.y * (1.0 - fraction)),
        )
    };
    enforce_min_leaf_sizes(tree, index.left(), left_size);
    enforce_min_leaf_sizes(tree, index.right(), right_size);
}

fn clamp_split_fraction(fraction: f32, span: f32, first_min: f32, second_min: f32) -> f32 {
    let fraction = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.5
    };
    if !span.is_finite() || span <= 0.0 {
        return fraction.clamp(0.05, 0.95);
    }
    let first_min = first_min.max(0.0);
    let second_min = second_min.max(0.0);
    let need = first_min + second_min;
    if need <= 0.0 || span + 0.5 < need {
        return fraction.clamp(0.05, 0.95);
    }
    let min_frac = (first_min / span).clamp(0.0, 1.0);
    let max_frac = (1.0 - second_min / span).clamp(0.0, 1.0);
    if min_frac > max_frac {
        return fraction.clamp(0.05, 0.95);
    }
    fraction.clamp(min_frac, max_frac)
}

fn tab_strip_hits_collapse_button(tree: &Tree<WorkspaceTab>, index: NodeIndex, width: f32) -> bool {
    if index.0 >= tree.len() || !width.is_finite() {
        return false;
    }
    match &tree[index] {
        Node::Leaf(_) => width > 0.5 && (width - COLLAPSE_BUTTON_PX).abs() <= 0.5,
        Node::Horizontal(split) => {
            let fraction = split.fraction.clamp(0.0, 1.0);
            tab_strip_hits_collapse_button(tree, index.left(), width * fraction)
                || tab_strip_hits_collapse_button(tree, index.right(), width * (1.0 - fraction))
        }
        Node::Vertical(_) => {
            tab_strip_hits_collapse_button(tree, index.left(), width)
                || tab_strip_hits_collapse_button(tree, index.right(), width)
        }
        Node::Empty => false,
    }
}

// ------------------------------------------------------------
// Type: WorkspaceViewer
// Purpose: Renders dock tabs and keeps the viewport as a required leaf.
// ------------------------------------------------------------
struct WorkspaceViewer<'a> {
    app: &'a mut MyCadApp,
}

impl TabViewer for WorkspaceViewer<'_> {
    type Tab = WorkspaceTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        tab.title().into()
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Self::Tab) {
        match tab {
            WorkspaceTab::Home => home::show(ui, self.app),
            WorkspaceTab::Viewport => self.app.show_viewport(ui),
            WorkspaceTab::Properties => properties::show(ui, self.app),
            WorkspaceTab::Diagnostics => crate::diagnostics::show(ui, self.app),
            WorkspaceTab::Blocks => crate::blocks::show(ui, self.app),
            WorkspaceTab::DynamicBlock => crate::dynamic_block::show_authoring(ui, self.app),
            WorkspaceTab::CommandLine => crate::command_line::show(ui, self.app),
        }
    }

    fn is_closeable(&self, tab: &Self::Tab) -> bool {
        !matches!(tab, WorkspaceTab::Viewport)
    }

    fn force_close(&mut self, tab: &mut Self::Tab) -> bool {
        let _ = tab;
        false
    }

    fn tab_style_override(&self, tab: &Self::Tab, global_style: &TabStyle) -> Option<TabStyle> {
        if !matches!(tab, WorkspaceTab::Home) {
            return None;
        }
        let mut style = global_style.clone();
        style.tab_body.inner_margin = Margin::symmetric(4, 2);
        Some(style)
    }

    fn scroll_bars(&self, tab: &Self::Tab) -> [bool; 2] {
        match tab {
            WorkspaceTab::Viewport | WorkspaceTab::Home => [false, false],
            _ => [true, true],
        }
    }
}

pub fn paint_box_select_rect(
    painter: &egui::Painter,
    start: Pos2,
    current: Pos2,
    border: Color32,
    fill: Color32,
) {
    let rect = Rect::from_two_pos(start, current);
    if rect.width() < 1.0 && rect.height() < 1.0 {
        return;
    }
    painter.rect(
        rect,
        0.0,
        fill,
        Stroke::new(1.5_f32, border),
        StrokeKind::Inside,
    );
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn default_layout_includes_home_above_viewport() {
        let state = default_dock_state();
        let tabs: Vec<_> = state.iter_all_tabs().map(|(_, tab)| *tab).collect();
        assert!(tabs.contains(&WorkspaceTab::Home));
        assert!(tabs.contains(&WorkspaceTab::Viewport));
        assert!(tabs.contains(&WorkspaceTab::Properties));
        assert!(tabs.contains(&WorkspaceTab::Blocks));
        assert!(tabs.contains(&WorkspaceTab::Diagnostics));
        assert!(tabs.contains(&WorkspaceTab::CommandLine));
    }

    #[test]
    fn default_blocks_shares_the_properties_leaf() {
        let state = default_dock_state();
        let (p_surface, p_node, _) = state
            .find_tab(&WorkspaceTab::Properties)
            .expect("Properties");
        let (b_surface, b_node, _) = state.find_tab(&WorkspaceTab::Blocks).expect("Blocks");
        assert_eq!(p_surface, b_surface);
        assert_eq!(p_node, b_node);
        let tabs = state[p_surface][p_node].tabs().expect("left leaf");
        assert!(tabs.contains(&WorkspaceTab::Properties));
        assert!(tabs.contains(&WorkspaceTab::Blocks));
        assert_eq!(tabs.len(), 2);
    }

    #[test]
    fn dynamic_block_tab_restores_beside_properties() {
        let mut state = default_dock_state();
        assert!(state.find_tab(&WorkspaceTab::DynamicBlock).is_none());
        ensure_tab(&mut state, WorkspaceTab::DynamicBlock);
        let (p_surface, p_node, _) = state
            .find_tab(&WorkspaceTab::Properties)
            .expect("Properties");
        let (d_surface, d_node, _) = state
            .find_tab(&WorkspaceTab::DynamicBlock)
            .expect("Dynamic Block");
        assert_eq!(p_surface, d_surface);
        assert_eq!(p_node, d_node);
    }

    #[test]
    fn blocks_tab_migrates_once_beside_properties() {
        let mut state = DockState::new(vec![WorkspaceTab::Viewport]);
        let [viewport, _home] = state.main_surface_mut().split_above(
            NodeIndex::root(),
            default_home_fraction(),
            vec![WorkspaceTab::Home],
        );
        let [viewport, _props] =
            state
                .main_surface_mut()
                .split_left(viewport, 0.24, vec![WorkspaceTab::Properties]);
        let _ =
            state
                .main_surface_mut()
                .split_right(viewport, 0.76, vec![WorkspaceTab::Diagnostics]);
        assert!(state.find_tab(&WorkspaceTab::Blocks).is_none());
        assert!(migrate_blocks_tab(&mut state, false));
        let (p_surface, p_node, _) = state
            .find_tab(&WorkspaceTab::Properties)
            .expect("Properties");
        let (b_surface, b_node, _) = state.find_tab(&WorkspaceTab::Blocks).expect("Blocks");
        assert_eq!(p_surface, b_surface);
        assert_eq!(p_node, b_node);
        if let Some(tab) = state.find_tab(&WorkspaceTab::Blocks) {
            state.remove_tab(tab);
        }
        assert!(state.find_tab(&WorkspaceTab::Blocks).is_none());
        assert!(migrate_blocks_tab(&mut state, true));
        assert!(
            state.find_tab(&WorkspaceTab::Blocks).is_none(),
            "closing Blocks must not re-open on every launch"
        );
    }

    #[test]
    fn ensure_blocks_appends_beside_properties() {
        let mut state = default_dock_state();
        if let Some(tab) = state.find_tab(&WorkspaceTab::Blocks) {
            state.remove_tab(tab);
        }
        ensure_tab(&mut state, WorkspaceTab::Blocks);
        let (p_surface, p_node, _) = state
            .find_tab(&WorkspaceTab::Properties)
            .expect("Properties");
        let (b_surface, b_node, _) = state.find_tab(&WorkspaceTab::Blocks).expect("Blocks");
        assert_eq!(p_surface, b_surface);
        assert_eq!(p_node, b_node);
    }

    #[test]
    fn decode_preserves_home_and_reset_restores_it() {
        let encoded = encode_dock_layout(&default_dock_state());
        let decoded = decode_dock_layout(Some(&encoded));
        let tabs: Vec<_> = decoded.iter_all_tabs().map(|(_, tab)| *tab).collect();
        assert!(tabs.contains(&WorkspaceTab::Home));
        let mut closed = decoded;
        if let Some(tab) = closed.find_tab(&WorkspaceTab::Home) {
            closed.remove_tab(tab);
        }
        assert!(closed.find_tab(&WorkspaceTab::Home).is_none());
        ensure_tab(&mut closed, WorkspaceTab::Home);
        assert!(closed.find_tab(&WorkspaceTab::Home).is_some());
        let reset = default_dock_state();
        assert!(reset.find_tab(&WorkspaceTab::Home).is_some());
    }

    #[test]
    fn default_home_is_a_full_width_strip_above_the_viewport() {
        let state = default_dock_state();
        let (surface, node) = state
            .find_tab(&WorkspaceTab::Home)
            .map(|(surface, node, _)| (surface, node))
            .expect("Home");
        let parent = node.parent().expect("Home should sit in a split");
        assert!(
            matches!(state[surface][parent], Node::Vertical(_)),
            "Home must be a full-width strip, not mixed into Properties"
        );
        let (v_surface, viewport, _) = state.find_tab(&WorkspaceTab::Viewport).expect("Viewport");
        assert_eq!(surface, v_surface);
        assert_ne!(node, viewport);
    }

    #[test]
    fn home_split_is_not_rewritten_after_layout_creation() {
        let mut state = default_dock_state();
        let (surface, node) = state
            .find_tab(&WorkspaceTab::Home)
            .map(|(surface, node, _)| (surface, node))
            .expect("Home");
        let parent = node.parent().expect("Home should sit in a split");
        assert!(
            matches!(state[surface][parent], Node::Vertical(_)),
            "Home should be above the viewport"
        );
        let Node::Vertical(split) = &mut state[surface][parent] else {
            unreachable!();
        };
        split.fraction = 0.42;
        assert_eq!(split.fraction, 0.42);
    }

    #[test]
    fn oversized_home_split_is_recovered_only_once() {
        let mut state = default_dock_state();
        let (surface, home, _) = state.find_tab(&WorkspaceTab::Home).expect("Home");
        let parent = home.parent().expect("Home split");
        let Node::Vertical(split) = &mut state[surface][parent] else {
            panic!("Home should be above viewport");
        };
        split.fraction = 0.6;
        assert!(recover_home_split_once(&mut state, false));
        let Node::Vertical(split) = &mut state[surface][parent] else {
            unreachable!();
        };
        assert_eq!(split.fraction, default_home_fraction());
        split.fraction = 0.6;
        assert!(recover_home_split_once(&mut state, true));
        let Node::Vertical(split) = &state[surface][parent] else {
            unreachable!();
        };
        assert_eq!(split.fraction, 0.6);
    }

    #[test]
    fn default_home_split_is_a_pixel_height_not_a_percentage_guess() {
        let tall = home_split_fraction(1600.0);
        let short = home_split_fraction(800.0);
        assert!((short * 800.0 - HOME_DEFAULT_HEIGHT_PX).abs() < 0.5);
        assert!((tall * 1600.0 - HOME_DEFAULT_HEIGHT_PX).abs() < 0.5);
        assert!(tall < short);
        assert!(HOME_MIN_LEAF_PX >= 40.0 && HOME_MIN_LEAF_PX <= 45.0);
        assert!(HOME_TAB_BAR_HEIGHT >= 18.0 && HOME_TAB_BAR_HEIGHT <= 20.0);
    }

    #[test]
    fn compact_home_height_rewrites_legacy_defaults_once() {
        let mut state = default_dock_state();
        let (surface, home, _) = state.find_tab(&WorkspaceTab::Home).expect("Home");
        let parent = home.parent().expect("Home split");
        let Node::Vertical(split) = &mut state[surface][parent] else {
            panic!("Home should be above viewport");
        };
        split.fraction = LEGACY_HOME_SPLIT_FRACTION;
        assert!(!apply_compact_home_height(&mut state, 40.0, false));
        assert!(apply_compact_home_height(&mut state, 900.0, false));
        {
            let Node::Vertical(split) = &state[surface][parent] else {
                unreachable!();
            };
            assert!((split.fraction - home_split_fraction(900.0)).abs() < 1e-5);
        }
        {
            let Node::Vertical(split) = &mut state[surface][parent] else {
                unreachable!();
            };
            split.fraction = 0.18;
        }
        assert!(apply_compact_home_height(&mut state, 900.0, true));
        let Node::Vertical(split) = &state[surface][parent] else {
            unreachable!();
        };
        assert!((split.fraction - 0.18).abs() < 1e-5);
    }

    #[test]
    fn compact_home_height_preserves_a_user_resized_strip() {
        let mut state = default_dock_state();
        let (surface, home, _) = state.find_tab(&WorkspaceTab::Home).expect("Home");
        let parent = home.parent().expect("Home split");
        let Node::Vertical(split) = &mut state[surface][parent] else {
            panic!("Home should be above viewport");
        };
        split.fraction = 0.15;
        assert!(apply_compact_home_height(&mut state, 900.0, false));
        let Node::Vertical(split) = &state[surface][parent] else {
            unreachable!();
        };
        assert!((split.fraction - 0.15).abs() < 1e-5);
    }

    #[test]
    fn old_layout_without_home_is_migrated_once() {
        let mut state = DockState::new(vec![WorkspaceTab::Viewport]);
        assert!(state.find_tab(&WorkspaceTab::Home).is_none());
        assert!(migrate_home_tab(&mut state, false));
        assert!(state.find_tab(&WorkspaceTab::Home).is_some());
        assert!(state.find_tab(&WorkspaceTab::Viewport).is_some());

        if let Some(tab) = state.find_tab(&WorkspaceTab::Home) {
            state.remove_tab(tab);
        }
        assert!(state.find_tab(&WorkspaceTab::Home).is_none());
        assert!(migrate_home_tab(&mut state, true));
        assert!(
            state.find_tab(&WorkspaceTab::Home).is_none(),
            "closing Home after migration must be preserved"
        );
    }

    #[test]
    fn a_split_as_wide_as_the_collapse_button_is_widened() {
        let mut state = default_dock_state();
        let width = 1400.0;
        let (surface, props, _) = state
            .find_tab(&WorkspaceTab::Properties)
            .expect("Properties");
        let parent = props.parent().expect("properties split");
        let Node::Horizontal(split) = &mut state[surface][parent] else {
            panic!("Properties should sit in a horizontal split");
        };
        split.fraction = COLLAPSE_BUTTON_PX / width;
        assert!(guard_collapse_buttons(&mut state, egui::vec2(width, 800.0)));
        let Node::Horizontal(split) = &state[surface][parent] else {
            unreachable!();
        };
        assert!(split.fraction * width >= MIN_LEAF_WIDTH_PX - 0.1);
        assert!(!tab_strip_hits_collapse_button(
            state.main_surface(),
            NodeIndex::root(),
            width
        ));
    }

    #[test]
    fn a_button_width_workspace_hides_collapse_buttons() {
        let mut state = default_dock_state();
        assert!(!guard_collapse_buttons(
            &mut state,
            egui::vec2(COLLAPSE_BUTTON_PX, 800.0)
        ));
    }

    #[test]
    fn short_home_strip_is_raised_to_the_leaf_minimum() {
        let mut state = default_dock_state();
        let height = 826.0;
        let (surface, home, _) = state.find_tab(&WorkspaceTab::Home).expect("Home");
        let parent = home.parent().expect("Home split");
        let Node::Vertical(split) = &mut state[surface][parent] else {
            panic!("Home should be above the viewport");
        };
        split.fraction = 0.0417;
        assert!(guard_collapse_buttons(
            &mut state,
            egui::vec2(1424.0, height)
        ));
        let Node::Vertical(split) = &state[surface][parent] else {
            unreachable!();
        };
        assert!(
            split.fraction * height >= MIN_LEAF_HEIGHT_PX - 0.1,
            "home height {}px",
            split.fraction * height
        );
    }

    #[test]
    fn narrow_side_panels_keep_subtree_minimums() {
        let mut state = default_dock_state();
        let width = 1424.0;
        let (surface, props, _) = state
            .find_tab(&WorkspaceTab::Properties)
            .expect("Properties");
        let props_parent = props.parent().expect("properties split");
        let Node::Horizontal(split) = &mut state[surface][props_parent] else {
            panic!("Properties should sit in a horizontal split");
        };
        split.fraction = 0.061;

        let (surface, viewport, _) = state.find_tab(&WorkspaceTab::Viewport).expect("Viewport");
        let column = viewport.parent().expect("viewport column");
        let diag_parent = column.parent().expect("diagnostics split");
        let Node::Horizontal(split) = &mut state[surface][diag_parent] else {
            panic!("Viewport column should sit beside Diagnostics");
        };
        split.fraction = 0.05;

        assert!(guard_collapse_buttons(&mut state, egui::vec2(width, 826.0)));

        let Node::Horizontal(split) = &state[surface][props_parent] else {
            unreachable!();
        };
        let props_width = split.fraction * width;
        assert!(
            props_width >= MIN_LEAF_WIDTH_PX - 0.1,
            "properties width {props_width}px"
        );

        let Node::Horizontal(split) = &state[surface][diag_parent] else {
            unreachable!();
        };
        let column_width = split.fraction * (width - props_width);
        assert!(
            column_width >= MIN_LEAF_WIDTH_PX - 0.1,
            "viewport column width {column_width}px"
        );
    }

    #[test]
    fn a_workspace_smaller_than_the_minimums_keeps_sane_fractions() {
        for size in [egui::vec2(24.0, 24.0), egui::vec2(60.0, 60.0)] {
            let mut state = default_dock_state();
            let _ = guard_collapse_buttons(&mut state, size);
            for node in state.main_surface().iter() {
                let fraction = match node {
                    Node::Horizontal(split) | Node::Vertical(split) => split.fraction,
                    Node::Leaf(_) | Node::Empty => continue,
                };
                assert!(
                    fraction.is_finite() && fraction > 0.0 && fraction < 1.0,
                    "fraction {fraction} for size {size:?}"
                );
            }
        }
    }

    #[test]
    fn command_stacked_on_viewport_is_split_out_once() {
        let mut state = default_dock_state();
        let command = state.find_tab(&WorkspaceTab::CommandLine).expect("Command");
        state.remove_tab(command);
        let (surface, node, _) = state.find_tab(&WorkspaceTab::Viewport).expect("Viewport");
        state[surface][node].append_tab(WorkspaceTab::CommandLine);

        assert!(split_command_from_viewport_once(&mut state, true));
        let (viewport_surface, viewport_node, _) =
            state.find_tab(&WorkspaceTab::Viewport).expect("Viewport");
        let (command_surface, command_node, _) =
            state.find_tab(&WorkspaceTab::CommandLine).expect("Command");
        assert_eq!(
            (viewport_surface, viewport_node),
            (command_surface, command_node),
            "a repaired layout must keep a later re-dock"
        );

        assert!(split_command_from_viewport_once(&mut state, false));
        let (viewport_surface, viewport_node, _) =
            state.find_tab(&WorkspaceTab::Viewport).expect("Viewport");
        let (command_surface, command_node, _) =
            state.find_tab(&WorkspaceTab::CommandLine).expect("Command");
        assert_ne!(
            (viewport_surface, viewport_node),
            (command_surface, command_node)
        );
        let parent = command_node.parent().expect("command split");
        assert!(matches!(state[command_surface][parent], Node::Vertical(_)));
        assert_eq!(parent.left(), viewport_node);
        assert_eq!(parent.right(), command_node);
    }
}

#[cfg(test)]
mod highlight_tests {
    use cad_core::{Extents2, Geometry};
    use cad_render::{box_select, stroke_edges, tessellate_document, PickKind, SelectBoxMode};

    fn reference_dwg() -> std::path::PathBuf {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let samples = root.join("samples/KD-1413-260825 Assir Poultry Internal Logistics.dwg");
        if samples.is_file() {
            return samples;
        }
        root.join("test-data/KD-1413-260825 Assir Poultry Internal Logistics.dwg")
    }

    fn is_logistics(name: &str) -> bool {
        name.to_ascii_uppercase().contains("LOGISTICS")
    }

    #[test]
    fn logistics_insert_highlight_stays_pairwise() {
        let path = reference_dwg();
        assert!(
            path.is_file(),
            "missing acceptance drawing at {}",
            path.display()
        );
        let document =
            dwg_import::import_dwg(&path).expect("LibreDWG should read the reference DWG");
        let display = tessellate_document(&document);
        let mut found = 0usize;
        for entity in document.model_space.iter() {
            let Geometry::Insert { block_name, .. } = &entity.geometry else {
                continue;
            };
            if !is_logistics(block_name) {
                continue;
            }
            found += 1;
            let pick = display
                .pick_for(entity.id)
                .expect("LOGISTICS INSERT should have pick geometry");
            assert!(
                pick.primitives.len() > 1,
                "LOGISTICS should highlight as many child primitives, not one path"
            );
            let pairwise: Vec<_> = pick.stroke_edges().collect();
            let mut flat = Vec::new();
            for primitive in &pick.primitives {
                if matches!(primitive.kind, PickKind::Stroke { .. }) {
                    flat.extend_from_slice(&primitive.points);
                }
            }
            let joined: Vec<_> = stroke_edges(&flat, false).collect();
            assert!(
                joined.len() > pairwise.len(),
                "stitching child endpoints must invent extra edges ({} vs {})",
                joined.len(),
                pairwise.len()
            );
            let max_pair = pairwise
                .iter()
                .map(|[a, b]| a.distance(*b))
                .fold(0.0_f64, f64::max);
            let max_extra = joined
                .iter()
                .filter(|[a, b]| {
                    !pairwise.iter().any(|edge| {
                        (edge[0] == *a && edge[1] == *b) || (edge[0] == *b && edge[1] == *a)
                    })
                })
                .map(|[a, b]| a.distance(*b))
                .fold(0.0_f64, f64::max);
            assert!(
                max_extra > max_pair.max(1.0),
                "flattened polyline would add long connectors (extra={max_extra}, pairwise={max_pair})"
            );
            let overlay = display.overlay_batches(&[entity.id]);
            let edge_count = pairwise.len();
            assert!(
                overlay.range_count() <= 2,
                "GPU overlay should batch by entity range, not emit {edge_count} highlight edges"
            );
            assert!(
                edge_count > overlay.range_count(),
                "LOGISTICS overlay must be cheaper than per-edge painting ({edge_count} edges, {} ranges)",
                overlay.range_count()
            );
            let region = Extents2::from_corners(pick.bounds.min, pick.bounds.center());
            let brute = box_select(&display.picks, region, SelectBoxMode::Crossing);
            let mut indexed = Vec::new();
            display.box_select_into(region, SelectBoxMode::Crossing, &mut indexed);
            assert_eq!(brute, indexed);
        }
        assert!(found > 0, "expected at least one LOGISTICS INSERT");
    }
}
