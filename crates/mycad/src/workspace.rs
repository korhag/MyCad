//! Persistent dockable workspace: Properties, Viewport, Diagnostics.

use cad_render::{stroke_edges, DisplayList, PickKind};
use cad_viewport::Camera2;
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, StrokeKind, Ui};
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};
use serde::{Deserialize, Serialize};

use crate::app::MyCadApp;
use crate::properties;
use crate::selection::Selection;

// ------------------------------------------------------------
// Type: WorkspaceTab
// Purpose: Serializable identity of each dockable panel.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTab {
    Viewport,
    Properties,
    Diagnostics,
}

impl WorkspaceTab {
    pub fn title(self) -> &'static str {
        match self {
            Self::Viewport => "Viewport",
            Self::Properties => "Properties",
            Self::Diagnostics => "Diagnostics",
        }
    }
}

pub fn default_dock_state() -> DockState<WorkspaceTab> {
    let mut state = DockState::new(vec![WorkspaceTab::Viewport]);
    let [_props, viewport] = state.main_surface_mut().split_left(
        NodeIndex::root(),
        0.24,
        vec![WorkspaceTab::Properties],
    );
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
    {
        let mut viewer = WorkspaceViewer { app };
        DockArea::new(&mut dock_state)
            .id(egui::Id::new("mycad-workspace"))
            .style(Style::from_egui(ui.style().as_ref()))
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
}

// ------------------------------------------------------------
// Function: paint_selection_overlay
// Purpose: Constant-pixel highlight from independent pick edges
//          so the accent stays readable at every zoom level.
// ------------------------------------------------------------
pub fn paint_selection_overlay(
    painter: &egui::Painter,
    camera: Camera2,
    rect: Rect,
    display: &DisplayList,
    selection: &Selection,
) {
    paint_entity_highlights(
        painter,
        camera,
        rect,
        display,
        selection.indices(),
        Stroke::new(2.0, Color32::from_rgb(255, 196, 72)),
        Color32::from_rgba_unmultiplied(255, 196, 72, 36),
    );
}

pub fn paint_entity_highlights(
    painter: &egui::Painter,
    camera: Camera2,
    rect: Rect,
    display: &DisplayList,
    indices: &[usize],
    stroke: Stroke,
    fill: Color32,
) {
    if indices.is_empty() {
        return;
    }
    let origin = cad_core::Point2::new(rect.min.x as f64, rect.min.y as f64);
    let size = cad_core::Point2::new(rect.width() as f64, rect.height() as f64);
    let to_screen = |p: cad_core::Point2| {
        let s = camera.world_to_screen(p, origin, size);
        Pos2::new(s.x as f32, s.y as f32)
    };
    for index in indices {
        let Some(pick) = display.pick_for(*index) else {
            continue;
        };
        for primitive in &pick.primitives {
            if let PickKind::Fill = primitive.kind {
                if primitive.points.len() >= 3 {
                    let points: Vec<Pos2> =
                        primitive.points.iter().copied().map(to_screen).collect();
                    painter.add(egui::Shape::convex_polygon(points, fill, Stroke::NONE));
                }
            }
            let closed = match primitive.kind {
                PickKind::Stroke { closed } => closed,
                PickKind::Fill => true,
            };
            for [a, b] in stroke_edges(&primitive.points, closed) {
                painter.line_segment([to_screen(a), to_screen(b)], stroke);
            }
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
mod highlight_tests {
    use cad_core::Geometry;
    use cad_render::{stroke_edges, tessellate_document, PickKind};

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
        for (index, entity) in document.model_space.iter().enumerate() {
            let Geometry::Insert { block_name, .. } = &entity.geometry else {
                continue;
            };
            if !is_logistics(block_name) {
                continue;
            }
            found += 1;
            let pick = display
                .pick_for(index)
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
        }
        assert!(found > 0, "expected at least one LOGISTICS INSERT");
    }
}
