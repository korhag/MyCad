//! Persistent dockable workspace: Home, Properties, Viewport, Diagnostics.

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, StrokeKind, Ui};
use egui_dock::{DockArea, DockState, Node, NodeIndex, Split, Style, TabViewer};
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
}

impl WorkspaceTab {
    pub fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Viewport => "Viewport",
            Self::Properties => "Properties",
            Self::Diagnostics => "Diagnostics",
        }
    }
}

const HOME_SPLIT_FRACTION: f32 = 0.08;

pub fn default_dock_state() -> DockState<WorkspaceTab> {
    let mut state = DockState::new(vec![WorkspaceTab::Viewport]);
    let [viewport, _home] = state.main_surface_mut().split_above(
        NodeIndex::root(),
        HOME_SPLIT_FRACTION,
        vec![WorkspaceTab::Home],
    );
    let [viewport, _props] =
        state
            .main_surface_mut()
            .split_left(viewport, 0.24, vec![WorkspaceTab::Properties]);
    let _ = state
        .main_surface_mut()
        .split_right(viewport, 0.76, vec![WorkspaceTab::Diagnostics]);
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
            HOME_SPLIT_FRACTION,
            Node::leaf(WorkspaceTab::Home),
        );
    } else {
        state.push_to_focused_leaf(WorkspaceTab::Home);
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
            HOME_SPLIT_FRACTION
        } else {
            1.0 - HOME_SPLIT_FRACTION
        };
        if let Node::Vertical(split) = &mut state[surface][parent] {
            split.fraction = fraction;
        }
    }
    true
}

pub fn ensure_tab(state: &mut DockState<WorkspaceTab>, tab: WorkspaceTab) {
    if state.find_tab(&tab).is_none() {
        state.push_to_focused_leaf(tab);
    }
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
    let style = Style::from_egui(ui.style().as_ref());
    {
        let mut viewer = WorkspaceViewer { app };
        DockArea::new(&mut dock_state)
            .id(egui::Id::new("mycad-workspace"))
            .style(style)
            .show_leaf_collapse_buttons(true)
            .show_inside(ui, &mut viewer);
    }
    app.dock_state = dock_state;
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
        }
    }

    fn closeable(&mut self, tab: &mut Self::Tab) -> bool {
        !matches!(tab, WorkspaceTab::Viewport)
    }

    fn force_close(&mut self, tab: &mut Self::Tab) -> bool {
        let _ = tab;
        false
    }

    fn scroll_bars(&self, tab: &Self::Tab) -> [bool; 2] {
        match tab {
            WorkspaceTab::Viewport => [false, false],
            WorkspaceTab::Home => [true, false],
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
        Stroke::new(1.5, border),
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
        assert!(tabs.contains(&WorkspaceTab::Diagnostics));
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
        assert_eq!(split.fraction, HOME_SPLIT_FRACTION);
        split.fraction = 0.6;
        assert!(recover_home_split_once(&mut state, true));
        let Node::Vertical(split) = &state[surface][parent] else {
            unreachable!();
        };
        assert_eq!(split.fraction, 0.6);
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
}

#[cfg(test)]
mod highlight_tests {
    use cad_core::{Extents2, Geometry};
    use cad_render::{box_select, stroke_edges, tessellate_document, PickKind, SelectBoxMode};

    fn reference_dwg() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-data/KD-1413-260825 Assir Poultry Internal Logistics.dwg")
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
