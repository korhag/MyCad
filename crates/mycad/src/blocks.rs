//! Dockable Blocks tree: definition hierarchy, counts, and inline rename.

use std::collections::HashSet;

use cad_core::{is_user_editable_block_name, validate_block_rename, BlockTreeIndex};
use eframe::egui::{self, text::CCursorRange, Color32, RichText, TextEdit, Ui};
use egui::text::CCursor;
use egui_phosphor::regular::{
    CARET_DOWN, CARET_RIGHT, CHECK, MAGNIFYING_GLASS, PENCIL_SIMPLE, WARNING,
};

use crate::app::MyCadApp;

const ACCENT: Color32 = Color32::from_rgb(120, 210, 180);
const COUNT_COLOR: Color32 = Color32::from_rgb(140, 158, 154);
const CYCLE_COLOR: Color32 = Color32::from_rgb(220, 120, 90);
const MODEL_KEY: &str = "";

// ------------------------------------------------------------
// Type: InlineRename
// Purpose: In-row TextEdit state for a single block definition.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
struct InlineRename {
    name: String,
    draft: String,
    select_all: bool,
}

// ------------------------------------------------------------
// Type: BlocksPanel
// Purpose: Selection, expansion, search, and rename draft for the tree.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct BlocksPanel {
    pub selected: Option<String>,
    pub error: Option<String>,
    expanded: HashSet<String>,
    search: String,
    renaming: Option<InlineRename>,
}

impl Default for BlocksPanel {
    fn default() -> Self {
        let mut expanded = HashSet::new();
        expanded.insert(MODEL_KEY.to_string());
        Self {
            selected: None,
            error: None,
            expanded,
            search: String::new(),
            renaming: None,
        }
    }
}

impl BlocksPanel {
    fn is_expanded(&self, key: &str) -> bool {
        self.expanded.contains(&key.to_ascii_lowercase())
    }

    fn toggle(&mut self, key: &str) {
        let key = key.to_ascii_lowercase();
        if !self.expanded.remove(&key) {
            self.expanded.insert(key);
        }
    }

    fn expand(&mut self, key: &str) {
        self.expanded.insert(key.to_ascii_lowercase());
    }

    pub(crate) fn is_renaming(&self) -> bool {
        self.renaming.is_some()
    }

    pub(crate) fn cancel_rename(&mut self) {
        self.renaming = None;
    }

    pub(crate) fn retarget_rename(&mut self, from: &str, to: &str) {
        if let Some(renaming) = &mut self.renaming {
            if renaming.name.eq_ignore_ascii_case(from) {
                renaming.name = to.to_string();
            }
        }
    }
}

enum TreeAction {
    Select(Option<String>),
    Toggle(String),
    StartRename(String),
    CommitRename,
    CancelRename,
    Edit(String),
    MakeUnique(String),
    Insert(String),
    Duplicate(String),
}

pub fn show(ui: &mut Ui, app: &mut MyCadApp) {
    expand_active_edit_path(app);
    ui.heading("BLOCKS");
    ui.separator();
    if app.document.is_none() {
        ui.weak("Open a drawing to browse blocks.");
        return;
    }

    ui.horizontal(|ui| {
        ui.label(RichText::new(MAGNIFYING_GLASS).size(14.0));
        ui.add(
            TextEdit::singleline(&mut app.blocks_panel.search)
                .hint_text("Search blocks")
                .desired_width(f32::INFINITY),
        );
    });
    ui.add_space(4.0);

    if ui.input(|input| input.key_pressed(egui::Key::F2)) {
        if let Some(name) = app.blocks_panel.selected.clone() {
            if is_user_editable_block_name(&name) {
                start_rename(app, name);
            }
        }
    }

    let index = app.block_tree.clone();
    let query = app.blocks_panel.search.trim().to_ascii_lowercase();
    let active = app
        .block_edit
        .current()
        .map(|frame| frame.block_name.clone());
    let mut action = None;
    let mut ancestors = Vec::new();
    egui::ScrollArea::vertical()
        .id_salt("blocks-tree")
        .show(ui, |ui| {
            show_model_row(ui, app, &index, &query, &mut action);
            if app.blocks_panel.is_expanded(MODEL_KEY) {
                show_children(
                    ui,
                    app,
                    &index,
                    MODEL_KEY,
                    &query,
                    active.as_deref(),
                    1,
                    &mut ancestors,
                    &mut action,
                );
            }
        });

    if index.model_children().is_empty() && query.is_empty() {
        ui.weak("No blocks in model space.");
    }

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("Purge Unused").clicked() {
            app.purge_unused_blocks();
        }
    });
    if let Some(err) = &app.blocks_panel.error {
        ui.colored_label(CYCLE_COLOR, err);
    }

    apply_action(app, action);
}

fn expand_active_edit_path(app: &mut MyCadApp) {
    let Some(target) = app
        .block_edit
        .current()
        .map(|frame| frame.block_name.clone())
    else {
        return;
    };
    app.blocks_panel.expand(MODEL_KEY);
    for frame in &app.block_edit.stack {
        app.blocks_panel.expand(&frame.block_name);
    }
    let mut ancestors = Vec::new();
    expand_paths_to(
        &app.block_tree,
        &target,
        MODEL_KEY,
        &mut ancestors,
        &mut app.blocks_panel.expanded,
    );
}

fn expand_paths_to(
    index: &BlockTreeIndex,
    target: &str,
    parent: &str,
    ancestors: &mut Vec<String>,
    expanded: &mut HashSet<String>,
) -> bool {
    if !parent.is_empty()
        && ancestors
            .iter()
            .any(|name| name.eq_ignore_ascii_case(parent))
    {
        return false;
    }
    let mut found = parent.eq_ignore_ascii_case(target) && !parent.is_empty();
    if !parent.is_empty() {
        ancestors.push(parent.to_string());
    }
    for child in index.children_of(parent) {
        if expand_paths_to(index, target, &child.name, ancestors, expanded) {
            expanded.insert(parent.to_ascii_lowercase());
            found = true;
        }
    }
    if !parent.is_empty() {
        ancestors.pop();
    }
    found
}

fn show_model_row(
    ui: &mut Ui,
    app: &MyCadApp,
    index: &BlockTreeIndex,
    query: &str,
    action: &mut Option<TreeAction>,
) {
    if !query.is_empty() && !node_matches(index, "Model", MODEL_KEY, query, &mut Vec::new()) {
        return;
    }
    let expanded = app.blocks_panel.is_expanded(MODEL_KEY);
    let selected = app.blocks_panel.selected.is_none();
    ui.horizontal(|ui| {
        if chevron(ui, !index.model_children().is_empty(), expanded).clicked() {
            *action = Some(TreeAction::Toggle(MODEL_KEY.into()));
        }
        let label = RichText::new("Model").strong();
        if ui.selectable_label(selected, label).clicked() {
            *action = Some(TreeAction::Select(None));
        }
    });
}

fn show_children(
    ui: &mut Ui,
    app: &mut MyCadApp,
    index: &BlockTreeIndex,
    parent: &str,
    query: &str,
    active: Option<&str>,
    depth: usize,
    ancestors: &mut Vec<String>,
    action: &mut Option<TreeAction>,
) {
    for child in index.children_of(parent) {
        let cyclic = ancestors
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&child.name));
        if !query.is_empty() {
            let mut path = ancestors.clone();
            if !node_matches(index, &child.name, &child.name, query, &mut path) {
                continue;
            }
        }
        show_block_row(
            ui,
            app,
            index,
            &child.name,
            child.count,
            cyclic,
            query,
            active,
            depth,
            ancestors,
            action,
        );
    }
}

fn show_block_row(
    ui: &mut Ui,
    app: &mut MyCadApp,
    index: &BlockTreeIndex,
    name: &str,
    count: usize,
    cyclic: bool,
    query: &str,
    active: Option<&str>,
    depth: usize,
    ancestors: &mut Vec<String>,
    action: &mut Option<TreeAction>,
) {
    let children = index.children_of(name);
    let expanded = app.blocks_panel.is_expanded(name);
    let selected = app
        .blocks_panel
        .selected
        .as_ref()
        .is_some_and(|sel| sel.eq_ignore_ascii_case(name));
    let is_active = active.is_some_and(|current| current.eq_ignore_ascii_case(name));
    let renaming = app
        .blocks_panel
        .renaming
        .as_ref()
        .is_some_and(|state| state.name.eq_ignore_ascii_case(name));
    let editable = is_user_editable_block_name(name);

    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 12.0);
        if cyclic {
            ui.add_space(16.0);
        } else if chevron(ui, !children.is_empty(), expanded).clicked() {
            *action = Some(TreeAction::Toggle(name.to_string()));
        }
        if renaming {
            show_rename_editor(ui, app, name, action);
        } else {
            let mut label = RichText::new(name);
            if is_active {
                label = label.strong().color(ACCENT);
            } else if selected {
                label = label.strong();
            }
            let response = ui.selectable_label(selected && !is_active, label);
            if response.clicked() {
                *action = Some(TreeAction::Select(Some(name.to_string())));
            }
            response.context_menu(|ui| {
                if ui
                    .add_enabled(editable, egui::Button::new("Rename"))
                    .clicked()
                {
                    *action = Some(TreeAction::StartRename(name.to_string()));
                    ui.close();
                }
                if ui.button("Edit Block").clicked() {
                    *action = Some(TreeAction::Edit(name.to_string()));
                    ui.close();
                }
                if ui.button("Make Unique").clicked() {
                    *action = Some(TreeAction::MakeUnique(name.to_string()));
                    ui.close();
                }
                if ui.button("Duplicate").clicked() {
                    *action = Some(TreeAction::Duplicate(name.to_string()));
                    ui.close();
                }
                if ui.button("Insert").clicked() {
                    *action = Some(TreeAction::Insert(name.to_string()));
                    ui.close();
                }
            });
            if count > 1 {
                ui.label(
                    RichText::new(format!("×{count}"))
                        .small()
                        .color(COUNT_COLOR),
                );
            }
            if cyclic {
                ui.label(RichText::new("cycle").small().color(CYCLE_COLOR))
                    .on_hover_text("Circular block reference");
                ui.label(RichText::new(WARNING).color(CYCLE_COLOR));
            }
            if editable {
                let pencil =
                    ui.add(egui::Button::new(RichText::new(PENCIL_SIMPLE).size(13.0)).frame(false));
                if pencil.clicked() {
                    *action = Some(TreeAction::StartRename(name.to_string()));
                }
            }
        }
    });

    if !cyclic && expanded {
        ancestors.push(name.to_string());
        show_children(
            ui,
            app,
            index,
            name,
            query,
            active,
            depth + 1,
            ancestors,
            action,
        );
        ancestors.pop();
    }
}

fn show_rename_editor(
    ui: &mut Ui,
    app: &mut MyCadApp,
    name: &str,
    action: &mut Option<TreeAction>,
) {
    let Some(document) = app.document.as_ref() else {
        return;
    };
    let validation =
        app.blocks_panel.renaming.as_ref().and_then(|state| {
            match validate_block_rename(document, &state.name, &state.draft) {
                Ok(_) => None,
                Err(err) => Some(err.to_string()),
            }
        });
    let select_all = app
        .blocks_panel
        .renaming
        .as_ref()
        .is_some_and(|state| state.select_all);
    let Some(state) = app.blocks_panel.renaming.as_mut() else {
        return;
    };
    if !state.name.eq_ignore_ascii_case(name) {
        return;
    }
    let output = TextEdit::singleline(&mut state.draft)
        .desired_width(140.0)
        .clip_text(true)
        .show(ui);
    if select_all {
        output.response.request_focus();
        let len = state.draft.chars().count();
        let mut edit_state = output.state;
        edit_state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(len))));
        edit_state.store(ui.ctx(), output.response.id);
        state.select_all = false;
    }
    let enter =
        output.response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    let escape = ui.input(|input| input.key_pressed(egui::Key::Escape));
    if ui
        .add_enabled(
            validation.is_none(),
            egui::Button::new(RichText::new(CHECK).size(13.0)).frame(false),
        )
        .clicked()
        || enter
    {
        *action = Some(TreeAction::CommitRename);
    } else if escape {
        *action = Some(TreeAction::CancelRename);
        app.input_consumed_escape = true;
    } else if output.response.lost_focus() && validation.is_none() {
        *action = Some(TreeAction::CommitRename);
    }
    if let Some(message) = validation {
        output
            .response
            .on_hover_text(RichText::new(&message).color(CYCLE_COLOR));
        ui.colored_label(CYCLE_COLOR, RichText::new(message).small());
    }
}

fn chevron(ui: &mut Ui, has_children: bool, expanded: bool) -> egui::Response {
    let icon = if !has_children {
        " "
    } else if expanded {
        CARET_DOWN
    } else {
        CARET_RIGHT
    };
    ui.add_enabled(
        has_children,
        egui::Button::new(RichText::new(icon).size(12.0)).frame(false),
    )
}

fn start_rename(app: &mut MyCadApp, name: String) {
    app.blocks_panel.selected = Some(name.clone());
    app.blocks_panel.renaming = Some(InlineRename {
        draft: name.clone(),
        name,
        select_all: true,
    });
    app.blocks_panel.error = None;
}

fn apply_action(app: &mut MyCadApp, action: Option<TreeAction>) {
    match action {
        Some(TreeAction::Select(name)) => {
            app.blocks_panel.selected = name;
            app.blocks_panel.error = None;
        }
        Some(TreeAction::Toggle(name)) => app.blocks_panel.toggle(&name),
        Some(TreeAction::StartRename(name)) => start_rename(app, name),
        Some(TreeAction::CommitRename) => commit_inline_rename(app),
        Some(TreeAction::CancelRename) => {
            app.blocks_panel.renaming = None;
            app.input_consumed_escape = true;
        }
        Some(TreeAction::Edit(name)) => app.edit_named_block_from_tree(&name),
        Some(TreeAction::MakeUnique(name)) => app.make_named_block_unique(&name),
        Some(TreeAction::Insert(name)) => app.insert_named_block(&name),
        Some(TreeAction::Duplicate(name)) => app.duplicate_named_block(&name),
        None => {}
    }
}

fn commit_inline_rename(app: &mut MyCadApp) {
    let Some(state) = app.blocks_panel.renaming.clone() else {
        return;
    };
    let Some(document) = app.document.as_ref() else {
        return;
    };
    match validate_block_rename(document, &state.name, &state.draft) {
        Ok(None) => {
            app.blocks_panel.renaming = None;
            app.blocks_panel.error = None;
        }
        Ok(Some(_)) => {
            app.rename_named_block(&state.name, &state.draft);
            if app.blocks_panel.error.is_none() {
                app.blocks_panel.renaming = None;
            }
        }
        Err(err) => {
            app.blocks_panel.error = Some(err.to_string());
        }
    }
}

fn node_matches(
    index: &BlockTreeIndex,
    label: &str,
    key: &str,
    query: &str,
    ancestors: &mut Vec<String>,
) -> bool {
    if query.is_empty() || label.to_ascii_lowercase().contains(query) {
        return true;
    }
    if !key.is_empty() && ancestors.iter().any(|name| name.eq_ignore_ascii_case(key)) {
        return false;
    }
    if !key.is_empty() {
        ancestors.push(key.to_string());
    }
    let found = index
        .children_of(key)
        .iter()
        .any(|child| node_matches(index, &child.name, &child.name, query, ancestors));
    if !key.is_empty() {
        ancestors.pop();
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_core::{
        validate_block_rename, BlockDefinition, BlockError, Document, Entity, Geometry, Point3,
    };

    fn insert(name: &str) -> Entity {
        Entity::new(Geometry::Insert {
            block_name: name.into(),
            insertion: Point3::from_xy(0.0, 0.0),
            scale: Point3::new(1.0, 1.0, 1.0),
            rotation: 0.0,
            extrusion: Point3::new(0.0, 0.0, 1.0),
            attribs: Vec::new(),
            column_count: 1,
            row_count: 1,
            column_spacing: 0.0,
            row_spacing: 0.0,
        })
    }

    fn collect(
        index: &BlockTreeIndex,
        parent: &str,
        ancestors: &mut Vec<String>,
        out: &mut Vec<(String, bool)>,
    ) {
        for child in index.children_of(parent) {
            let cyclic = ancestors
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&child.name));
            out.push((child.name.clone(), cyclic));
            if !cyclic {
                ancestors.push(child.name.clone());
                collect(index, &child.name, ancestors, out);
                ancestors.pop();
            }
        }
    }

    #[test]
    fn tree_walk_stops_on_cycles_and_groups_counts() {
        let mut document = Document::default();
        document.replace_block_definition(BlockDefinition {
            name: "Motor".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![insert("Machine")],
        });
        document.replace_block_definition(BlockDefinition {
            name: "Machine".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![insert("Motor"), insert("Motor")],
        });
        document.add_entity(insert("Machine"));
        let index = BlockTreeIndex::build(&document);
        let mut rows = Vec::new();
        collect(&index, MODEL_KEY, &mut Vec::new(), &mut rows);
        assert_eq!(rows[0].0, "Machine");
        assert!(!rows[0].1);
        assert!(rows.iter().any(|(name, cyclic)| name == "Motor" && !cyclic));
        assert!(rows
            .iter()
            .any(|(name, cyclic)| name == "Machine" && *cyclic));
        assert_eq!(index.children_of("Machine")[0].count, 2);
    }

    #[test]
    fn search_keeps_ancestors_of_a_nested_match() {
        let mut document = Document::default();
        document.replace_block_definition(BlockDefinition {
            name: "Motor".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: Vec::new(),
        });
        document.replace_block_definition(BlockDefinition {
            name: "Machine".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![insert("Motor")],
        });
        document.add_entity(insert("Machine"));
        let index = BlockTreeIndex::build(&document);
        assert!(node_matches(
            &index,
            "Machine",
            "Machine",
            "motor",
            &mut Vec::new()
        ));
        assert!(!node_matches(
            &index,
            "Machine",
            "Machine",
            "zzz",
            &mut Vec::new()
        ));
    }

    #[test]
    fn active_edit_path_expands_ancestors() {
        let mut document = Document::default();
        document.replace_block_definition(BlockDefinition {
            name: "Motor".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: Vec::new(),
        });
        document.replace_block_definition(BlockDefinition {
            name: "Machine".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![insert("Motor")],
        });
        document.add_entity(insert("Machine"));
        let index = BlockTreeIndex::build(&document);
        let mut expanded = HashSet::new();
        expand_paths_to(&index, "Motor", MODEL_KEY, &mut Vec::new(), &mut expanded);
        assert!(expanded.contains(""));
        assert!(expanded.contains("machine"));
    }

    #[test]
    fn validate_messages_cover_empty_duplicate_and_reserved() {
        let mut document = Document::default();
        document.replace_block_definition(BlockDefinition {
            name: "Motor".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: Vec::new(),
        });
        document.replace_block_definition(BlockDefinition {
            name: "Drive".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: Vec::new(),
        });
        assert!(matches!(
            validate_block_rename(&document, "Motor", ""),
            Err(BlockError::InvalidName)
        ));
        assert!(matches!(
            validate_block_rename(&document, "Motor", "drive"),
            Err(BlockError::DuplicateName(_))
        ));
        assert!(matches!(
            validate_block_rename(&document, "Motor", "*MODEL_SPACE"),
            Err(BlockError::StarPrefix)
        ));
        assert!(validate_block_rename(&document, "Motor", "MOTOR")
            .unwrap()
            .is_some());
    }
}
