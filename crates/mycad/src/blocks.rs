//! Dockable Blocks tree: definition hierarchy, counts, and inline rename.

use std::collections::HashSet;

use cad_core::{
    insert_instance_ids_in_space, is_user_editable_block_name, validate_block_rename,
    BlockTreeIndex, Document, EntityId, EntitySpace, Extents2, Geometry, Transform2,
};
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
#[derive(Debug)]
pub struct BlocksPanel {
    pub selected: Option<String>,
    pub error: Option<String>,
    expanded: HashSet<String>,
    search: String,
    renaming: Option<InlineRename>,
    reveal: Option<String>,
    library_folder: Option<std::path::PathBuf>,
    library_files: Vec<std::path::PathBuf>,
    library_search: String,
    new_library: Option<(std::path::PathBuf, String)>,
    library_job: Option<LibraryJob>,
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
            reveal: None,
            library_folder: None,
            library_files: Vec::new(),
            library_search: String::new(),
            new_library: None,
            library_job: None,
        }
    }
}

impl BlocksPanel {
    pub(crate) fn cancel_library_insert_for_open(&mut self) {
        if self
            .library_job
            .as_ref()
            .is_some_and(|job| job.source.is_some())
        {
            self.library_job = None;
            self.error =
                Some("Block insertion canceled because another drawing is opening.".into());
        }
    }

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
        if self
            .reveal
            .as_ref()
            .is_some_and(|name| name.eq_ignore_ascii_case(from))
        {
            self.reveal = Some(to.to_string());
        }
    }

    pub(crate) fn reveal_definition(&mut self, name: String, tree: &BlockTreeIndex) {
        if self.renaming.is_some() {
            return;
        }
        self.selected = Some(name.clone());
        self.expand(MODEL_KEY);
        expand_paths_to(tree, &name, MODEL_KEY, &mut Vec::new(), &mut self.expanded);
        self.reveal = Some(name);
    }

    fn take_reveal_if(&mut self, name: &str) -> bool {
        if self
            .reveal
            .as_ref()
            .is_some_and(|pending| pending.eq_ignore_ascii_case(name))
        {
            self.reveal = None;
            true
        } else {
            false
        }
    }
}

const MAX_LIBRARY_ITEMS: usize = 2_000;
const MAX_LIBRARY_SCAN_ENTRIES: usize = 20_000;
const MAX_LIBRARY_FILE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug)]
struct LibraryJob {
    receiver: std::sync::mpsc::Receiver<Result<LibraryResult, String>>,
    source: Option<(std::sync::Weak<Document>, u64)>,
    space: Option<EntitySpace>,
}

#[derive(Debug)]
enum LibraryResult {
    Listed {
        folder: std::path::PathBuf,
        files: Vec<std::path::PathBuf>,
    },
    Imported {
        document: Document,
        edits: Vec<crate::history::Edit>,
        inserted: EntityId,
        name: String,
    },
}

fn list_library(folder: std::path::PathBuf) -> Result<LibraryResult, String> {
    let entries = std::fs::read_dir(&folder).map_err(|err| err.to_string())?;
    let mut files = Vec::new();
    for (index, entry) in entries.enumerate() {
        if index >= MAX_LIBRARY_SCAN_ENTRIES {
            return Err("This folder has too many files. Choose a smaller library folder.".into());
        }
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mycadblock"))
            && path.is_file()
        {
            files.push(path);
            if files.len() > MAX_LIBRARY_ITEMS {
                return Err(format!("This folder contains more than {MAX_LIBRARY_ITEMS} blocks. Split it into smaller libraries."));
            }
        }
    }
    files.sort_by_key(|path| path.file_name().map(|s| s.to_string_lossy().to_lowercase()));
    Ok(LibraryResult::Listed { folder, files })
}

fn asset_file_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(|c| c == '.' || c == ' ');
    if cleaned.is_empty() {
        "Block".into()
    } else {
        format!("Block-{cleaned}")
    }
}

fn save_library_block(
    document: &Document,
    name: &str,
    folder: &std::path::Path,
) -> Result<(), String> {
    let stem = asset_file_stem(name);
    for number in 0..10_000 {
        let file = if number == 0 {
            format!("{stem}.mycadblock")
        } else {
            format!("{stem}-{number}.mycadblock")
        };
        let path = folder.join(file);
        // Reserve a new filename: never overwrite a user's existing library asset.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => drop(file),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err.to_string()),
        }
        if let Err(err) = cad_io::write_mycadblock(document, name, &path) {
            let _ = std::fs::remove_file(&path);
            return Err(err.to_string());
        }
        return Ok(());
    }
    Err("Too many copies of this block in the library.".into())
}

fn prepare_library_insert(
    mut document: Document,
    asset: cad_io::BlockAsset,
    space: EntitySpace,
    point: cad_core::Point2,
) -> Result<LibraryResult, String> {
    if asset.blocks.is_empty() {
        return Err("The library file contains no blocks.".into());
    }
    let source_layers: HashSet<_> = document.layers.keys().cloned().collect();
    let source_linetypes: HashSet<_> = document.linetypes.keys().cloned().collect();
    let source_styles: HashSet<_> = document.text_styles.keys().cloned().collect();
    let source_blocks: HashSet<_> = document.blocks.keys().cloned().collect();
    let name = cad_io::import_block_asset(&mut document, asset).map_err(|err| err.to_string())?;
    if let EntitySpace::Block(dest) = &space {
        if cad_core::would_create_block_cycle(&document, dest, &name) {
            return Err("This block would create a circular reference.".into());
        }
    }
    let mut edits = Vec::new();
    // The importer preserves existing tables and gives conflicts new names.
    // Record dependencies before the blocks so undo removes them last.
    for (name, layer) in &document.layers {
        if !source_layers.contains(name) {
            edits.push(crate::history::Edit::ReplaceLayer {
                name: name.clone(),
                before: None,
                after: Some(layer.clone()),
            });
        }
    }
    for (name, linetype) in &document.linetypes {
        if !source_linetypes.contains(name) {
            edits.push(crate::history::Edit::ReplaceLineType {
                name: name.clone(),
                before: None,
                after: Some(linetype.clone()),
            });
        }
    }
    for (name, style) in &document.text_styles {
        if !source_styles.contains(name) {
            edits.push(crate::history::Edit::ReplaceTextStyle {
                name: name.clone(),
                before: None,
                after: Some(style.clone()),
            });
        }
    }
    for definition in document.blocks.values() {
        if !source_blocks.contains(&definition.name) {
            edits.push(crate::history::Edit::ReplaceBlockDefinition {
                name: definition.name.clone(),
                before: None,
                after: Some(definition.clone()),
            });
        }
    }
    let entity = document.new_entity(Geometry::Insert {
        block_name: name.clone(),
        insertion: cad_core::Point3::from_xy(point.x, point.y),
        scale: cad_core::Point3::new(1.0, 1.0, 1.0),
        rotation: 0.0,
        extrusion: cad_core::Point3::new(0.0, 0.0, 1.0),
        attribs: Vec::new(),
        column_count: 1,
        row_count: 1,
        column_spacing: 0.0,
        row_spacing: 0.0,
        configuration: None,
        clip: None,
    });
    let inserted = document
        .add_entity_to(&space, entity)
        .ok_or("Cannot insert into the current space")?;
    let index = document
        .entity_index_in(&space, inserted.id)
        .ok_or("Inserted block was not indexed")?;
    edits.push(crate::history::Edit::InsertEntity {
        space,
        index,
        entity: inserted.clone(),
    });
    Ok(LibraryResult::Imported {
        document,
        edits,
        inserted: inserted.id,
        name,
    })
}

fn start_library_job(
    app: &mut MyCadApp,
    ctx: &egui::Context,
    source: Option<std::sync::Arc<Document>>,
    task: impl FnOnce() -> Result<LibraryResult, String> + Send + 'static,
) {
    if app.blocks_panel.library_job.is_some() {
        return;
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    match std::thread::Builder::new()
        .name("mycad-library".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task))
                .unwrap_or_else(|_| Err("The library operation failed unexpectedly.".into()));
            let _ = sender.send(result);
            ctx.request_repaint();
        }) {
        Ok(_) => {
            let space = source.as_ref().map(|_| app.block_edit.active_space());
            let source = source.as_ref().map(|source| {
                (
                    std::sync::Arc::downgrade(source),
                    source.content_generation(),
                )
            });
            app.blocks_panel.library_job = Some(LibraryJob {
                receiver,
                source,
                space,
            });
            app.blocks_panel.error = None;
        }
        Err(err) => {
            app.blocks_panel.error = Some(format!("Could not start library operation: {err}"))
        }
    }
}

pub(crate) fn poll_library_job(app: &mut MyCadApp) {
    let Some(job) = &app.blocks_panel.library_job else {
        return;
    };
    let result = match job.receiver.try_recv() {
        Ok(result) => result,
        Err(std::sync::mpsc::TryRecvError::Empty) => return,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            Err("Library worker ended unexpectedly.".into())
        }
    };
    let Some(job) = app.blocks_panel.library_job.take() else {
        return;
    };
    if job.source.as_ref().is_some_and(|(source, generation)| {
        !app.document.as_ref().is_some_and(|current| {
            source.ptr_eq(&std::sync::Arc::downgrade(current))
                && *generation == current.content_generation()
        })
    }) || job
        .space
        .as_ref()
        .is_some_and(|space| space != &app.block_edit.active_space())
    {
        app.blocks_panel.error =
            Some("The drawing changed while the block was loading. Insert it again.".into());
        return;
    }
    match result {
        Ok(LibraryResult::Listed { folder, files }) => {
            app.blocks_panel.library_folder = Some(folder);
            app.blocks_panel.library_files = files;
            app.status = format!(
                "Library ready: {} blocks",
                app.blocks_panel.library_files.len()
            );
        }
        Ok(LibraryResult::Imported {
            document,
            edits,
            inserted,
            name,
        }) => {
            app.block_edit.refresh_dirty(&document);
            app.document = Some(std::sync::Arc::new(document));
            app.history.begin();
            for edit in edits {
                app.history.record(edit);
            }
            app.history.commit_open();
            app.selection.replace(inserted);
            app.blocks_panel.selected = Some(name.clone());
            app.schedule_full_refresh();
            app.status =
                format!("Inserted {name} from library at the center of the view; Move to place it");
        }
        Err(err) => app.blocks_panel.error = Some(err),
    }
}

fn show_library(ui: &mut Ui, app: &mut MyCadApp) {
    egui::CollapsingHeader::new("Block library")
        .default_open(true)
        .show(ui, |ui| {
            ui.weak("Keep reusable blocks in a folder. Dynamic parameters are included.");
            ui.weak("Blocks use the current drawing’s units.");
            ui.weak("For DWG/DXF libraries, use File → Open, then save the blocks you need here.");
            let busy = app.blocks_panel.library_job.is_some();
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Open library…").clicked() {
                        if let Some(folder) = rfd::FileDialog::new()
                            .set_title("Choose a block library folder")
                            .pick_folder()
                        {
                            start_library_job(app, ui.ctx(), None, move || list_library(folder));
                        }
                    }
                    if ui.button("New library…").clicked() {
                        if let Some(parent) = rfd::FileDialog::new()
                            .set_title("Choose where to create the library")
                            .pick_folder()
                        {
                            app.blocks_panel.new_library = Some((parent, "My library".into()));
                        }
                    }
                    if let Some(folder) = app.blocks_panel.library_folder.clone() {
                        if ui.button("Refresh").clicked() {
                            start_library_job(app, ui.ctx(), None, move || list_library(folder));
                        }
                    }
                });
                let mut create = None;
                let mut cancel = false;
                if let Some((parent, name)) = &mut app.blocks_panel.new_library {
                    ui.label(format!("Create in {}", parent.display()));
                    ui.text_edit_singleline(name);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(valid_library_name(name), egui::Button::new("Create folder")).clicked() {
                            create = Some(parent.join(name.trim()));
                        }
                        cancel = ui.button("Cancel").clicked();
                    });
                    ui.weak("Use a folder name without / or \\.");
                }
                if cancel {
                    app.blocks_panel.new_library = None;
                }
                if let Some(folder) = create {
                    app.blocks_panel.new_library = None;
                    start_library_job(app, ui.ctx(), None, move || {
                        std::fs::create_dir(&folder).map_err(|err| err.to_string())?;
                        list_library(folder)
                    });
                }
                if let Some(folder) = app.blocks_panel.library_folder.clone() {
                    ui.label(folder.display().to_string());
                    ui.weak("Library folder stays selected for this session.");
                    let name = app.block_edit.current()
                        .map(|frame| frame.block_name.clone())
                        .or_else(|| app.blocks_panel.selected.clone());
                    let enabled = app.document.is_some() && name.is_some() && !app.io_busy();
                    let label = name.as_ref()
                        .map(|name| format!("Save {name} to library"))
                        .unwrap_or_else(|| "Save selected block to library".into());
                    if ui.add_enabled(enabled, egui::Button::new(label))
                        .on_hover_text("Choose a block in the drawing tree, or edit a block first. Existing library files are kept.")
                        .clicked()
                    {
                        if let (Some(source), Some(name)) = (app.document.clone(), name) {
                            start_library_job(app, ui.ctx(), None, move || {
                                save_library_block(&source, &name, &folder)?;
                                drop(source);
                                list_library(folder)
                            });
                        }
                    }
                }
                ui.add(TextEdit::singleline(&mut app.blocks_panel.library_search)
                    .hint_text("Find a library block"));
                let query = app.blocks_panel.library_search.trim().to_lowercase();
                let mut insert = None;
                let files: Vec<_> = app.blocks_panel.library_files.iter()
                    .filter(|path| path.file_stem().is_some_and(|name| {
                        name.to_string_lossy().to_lowercase().contains(&query)
                    }))
                    .collect();
                if files.is_empty() {
                    ui.weak("No matching blocks. Save a block here or open another library.");
                }
                egui::ScrollArea::vertical()
                    .id_salt("library-files")
                    .max_height(180.0)
                    .show_rows(ui, 24.0, files.len(), |ui, range| {
                        for index in range {
                            let path = files[index];
                            ui.horizontal(|ui| {
                                ui.label(path.file_stem().unwrap_or_default().to_string_lossy());
                                if ui.add_enabled(app.document.is_some() && !app.io_busy() && app.loading_path.is_none(), egui::Button::new("Insert")).clicked() {
                                    insert = Some(path.clone());
                                }
                            });
                        }
                    });
                if let (Some(path), Some(source)) = (insert, app.document.clone()) {
                    let space = app.block_edit.active_space();
                    let point = app.block_edit.local_from_world().apply(app.camera.center);
                    let snapshot = std::sync::Arc::clone(&source);
                    start_library_job(app, ui.ctx(), Some(source), move || {
                        // Copy on the worker, then release the shared drawing before file I/O.
                        let document = snapshot.as_ref().clone();
                        drop(snapshot);
                        if std::fs::metadata(&path).map_err(|err| err.to_string())?.len() > MAX_LIBRARY_FILE_BYTES {
                            return Err("The library block exceeds the 64 MB size limit.".into());
                        }
                        let asset = cad_io::read_mycadblock(&path).map_err(|err| err.to_string())?;
                        prepare_library_insert(document, asset, space, point)
                    });
                }
            });
            if let Some(error) = &app.blocks_panel.error {
                ui.colored_label(CYCLE_COLOR, error);
            }
            if busy {
                ui.spinner();
                ui.label("Working on library…");
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
            }
        });
}

fn valid_library_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.ends_with('.')
        && !name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
}

enum TreeAction {
    Select(Option<String>),
    ZoomTo(String),
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
    expand_reveal_path(app);
    ui.heading("BLOCKS");
    show_library(ui, app);
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

    // Move the tree out for the frame so drawing it does not clone every name.
    let index = std::mem::take(&mut app.block_tree);
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
    app.block_tree = index;

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

fn expand_reveal_path(app: &mut MyCadApp) {
    let Some(target) = app.blocks_panel.reveal.clone() else {
        return;
    };
    app.blocks_panel.expand(MODEL_KEY);
    expand_paths_to(
        &app.block_tree,
        &target,
        MODEL_KEY,
        &mut Vec::new(),
        &mut app.blocks_panel.expanded,
    );
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
            let hint = if editable {
                "F2 or pencil to rename; double-click to zoom"
            } else {
                "Double-click to zoom"
            };
            let response = response.on_hover_text(hint);
            if app.blocks_panel.take_reveal_if(name) {
                response.scroll_to_me(Some(egui::Align::Center));
            }
            if response.double_clicked() {
                *action = Some(TreeAction::ZoomTo(name.to_string()));
            } else if response.clicked() {
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
                let pencil = pencil.on_hover_text("Rename (F2)");
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
        Some(TreeAction::Select(None)) => {
            app.blocks_panel.selected = None;
            app.blocks_panel.error = None;
        }
        Some(TreeAction::Select(Some(name))) => app.select_named_block_instances(&name),
        Some(TreeAction::ZoomTo(name)) => app.zoom_to_named_block(&name),
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

fn first_insert_definition_name(document: &Document, ids: &[EntityId]) -> Option<String> {
    ids.iter().find_map(|id| {
        document.entity_by_id(*id).and_then(|entity| {
            matches!(entity.geometry, Geometry::Insert { .. })
                .then(|| entity.geometry.insert_block_name().map(str::to_string))
                .flatten()
        })
    })
}

pub(crate) fn reveal_selected_insert(app: &mut MyCadApp) {
    if app.blocks_panel.is_renaming() {
        return;
    }
    let Some(document) = app.document.as_ref() else {
        return;
    };
    let Some(name) = first_insert_definition_name(document, app.selection.ids()) else {
        return;
    };
    app.blocks_panel.reveal_definition(name, &app.block_tree);
}

pub(crate) fn world_extents_of_named_inserts(
    document: &Document,
    name: &str,
    space: &EntitySpace,
    world_from_local: Transform2,
) -> Option<Extents2> {
    let ids = insert_instance_ids_in_space(document, name, space);
    document
        .entities_extents(space, &ids)
        .map(|local| local.transformed(world_from_local))
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
        validate_block_rename, BlockDefinition, BlockError, Document, Entity, EntitySpace,
        Geometry, Point3, Transform2,
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
            configuration: None,
            clip: None,
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

    fn library_test_directory() -> std::path::PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("mycad-library-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn saving_library_assets_keeps_existing_files_and_dynamic_parameters() {
        let folder = library_test_directory();
        let mut document = Document::default();
        let id = document.allocate_parameter_id();
        let mut block = BlockDefinition::plain("Door", Point3::default(), Vec::new());
        block.dynamic = Some(cad_core::DynamicDefinition {
            parameters: vec![cad_core::ParameterDef::number(
                id,
                "Width",
                cad_core::NumericParameter::length(100.0),
            )],
            ..Default::default()
        });
        document.replace_block_definition(block);
        save_library_block(&document, "Door", &folder).unwrap();
        let original = folder.join("Block-Door.mycadblock");
        let first_bytes = std::fs::read(&original).unwrap();
        save_library_block(&document, "Door", &folder).unwrap();
        assert_eq!(std::fs::read(&original).unwrap(), first_bytes);
        let LibraryResult::Listed { files, .. } = list_library(folder.clone()).unwrap() else {
            panic!("list result")
        };
        assert_eq!(files.len(), 2);
        let asset = cad_io::read_mycadblock(&files[0]).unwrap();
        let dynamic = asset.blocks.last().unwrap().dynamic.as_ref().unwrap();
        assert_eq!(dynamic.parameters[0].name, "Width");
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn library_insert_renames_nested_conflicts_and_undo_removes_the_whole_import() {
        let mut source = Document::default();
        source.layers.insert(
            "Fixtures".into(),
            cad_core::Layer {
                name: "Fixtures".into(),
                color: cad_core::CadColor::Aci(5),
                linetype: "FixtureDash".into(),
                ..Default::default()
            },
        );
        source.linetypes.insert(
            "FixtureDash".into(),
            cad_core::LineType {
                name: "FixtureDash".into(),
                dashes: vec![5.0, -3.0],
                shapes: Vec::new(),
            },
        );
        let mut style = cad_core::TextStyle::standard();
        style.name = "Labels".into();
        source.text_styles.insert(style.name.clone(), style);
        let mut text = Entity::new(Geometry::Text(cad_core::TextData {
            value: "Door".into(),
            style: "Labels".into(),
            ..Default::default()
        }));
        text.layer = "Fixtures".into();
        source.replace_block_definition(BlockDefinition::plain(
            "Child",
            Point3::default(),
            vec![text],
        ));
        source.replace_block_definition(BlockDefinition::plain(
            "Root",
            Point3::default(),
            vec![insert("Child")],
        ));
        let folder = library_test_directory();
        let file = folder.join("Root.mycadblock");
        cad_io::write_mycadblock(&source, "Root", &file).unwrap();
        let asset = cad_io::read_mycadblock(&file).unwrap();
        let mut host = source.clone();
        host.layers.get_mut("Fixtures").unwrap().color = cad_core::CadColor::Aci(1);
        host.linetypes.get_mut("FixtureDash").unwrap().dashes = vec![2.0, -1.0];
        host.text_styles.get_mut("Labels").unwrap().width_factor = 1.5;
        let LibraryResult::Imported {
            mut document,
            edits,
            name,
            ..
        } = prepare_library_insert(
            host.clone(),
            asset,
            EntitySpace::ModelSpace,
            cad_core::Point2::new(25.0, 50.0),
        )
        .unwrap()
        else {
            panic!("import result")
        };
        assert_eq!(name, "Root_001");
        assert_eq!(
            document.block_by_name(&name).unwrap().entities[0]
                .geometry
                .insert_block_name(),
            Some("Child_001")
        );
        assert_eq!(
            document.block_by_name("Root").unwrap().entities[0]
                .geometry
                .insert_block_name(),
            Some("Child")
        );
        assert_eq!(document.model_space.len(), 1);
        assert_eq!(document.layers.len(), host.layers.len() + 1);
        assert_eq!(document.linetypes.len(), host.linetypes.len() + 1);
        assert_eq!(document.text_styles.len(), host.text_styles.len() + 1);
        let mut history = crate::history::History::default();
        history.begin();
        for edit in edits {
            history.record(edit);
        }
        history.commit_open();
        assert_eq!(history.undo_len(), 1);
        assert!(history.undo(&mut document));
        assert_eq!(document.blocks.len(), host.blocks.len());
        assert_eq!(document.layers.len(), host.layers.len());
        assert_eq!(document.linetypes, host.linetypes);
        assert_eq!(document.text_styles, host.text_styles);
        assert_eq!(
            document.layers["Fixtures"].color,
            cad_core::CadColor::Aci(1)
        );
        assert!(document.model_space.is_empty());
        assert!(history.redo(&mut document));
        assert_eq!(document.model_space.len(), 1);
        assert!(document.block_by_name("Child_001").is_some());
        assert_eq!(document.layers.len(), host.layers.len() + 1);
        assert_eq!(document.linetypes.len(), host.linetypes.len() + 1);
        assert_eq!(document.text_styles.len(), host.text_styles.len() + 1);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn library_names_and_asset_paths_cannot_escape_the_folder() {
        for bad in ["", "..", "../outside", "a/b", "a\\b", "name."] {
            assert!(!valid_library_name(bad), "{bad}");
        }
        assert!(valid_library_name("Conveyors"));
        let filename = asset_file_stem("../Door:90");
        assert!(!filename.contains('/'));
        assert!(!filename.contains(':'));
        assert!(!filename.starts_with('.'));
    }

    #[test]
    fn a_completed_library_import_is_discarded_after_the_drawing_changes() {
        let mut app = MyCadApp::for_test();
        let original = std::sync::Arc::new(Document::default());
        app.document = Some(std::sync::Arc::clone(&original));
        let (sender, receiver) = std::sync::mpsc::channel();
        app.blocks_panel.library_job = Some(LibraryJob {
            receiver,
            source: Some((
                std::sync::Arc::downgrade(&original),
                original.content_generation(),
            )),
            space: Some(EntitySpace::ModelSpace),
        });
        drop(original);
        assert_eq!(
            std::sync::Arc::strong_count(app.document.as_ref().unwrap()),
            1
        );
        let mut loaded = Document::default();
        let inserted = loaded.add_entity(insert("Library block")).id;
        sender
            .send(Ok(LibraryResult::Imported {
                document: loaded,
                edits: Vec::new(),
                inserted,
                name: "Library block".into(),
            }))
            .unwrap();
        let replacement = std::sync::Arc::new(Document::default());
        app.document = Some(std::sync::Arc::clone(&replacement));
        poll_library_job(&mut app);
        assert!(std::sync::Arc::ptr_eq(
            app.document.as_ref().unwrap(),
            &replacement
        ));
        assert!(app
            .blocks_panel
            .error
            .as_ref()
            .unwrap()
            .contains("drawing changed"));
        assert!(!app.history.can_undo());
    }

    #[test]
    fn a_completed_library_import_is_discarded_after_entering_a_block() {
        let mut app = MyCadApp::for_test();
        let mut document = Document::default();
        document.replace_block_definition(BlockDefinition::plain(
            "Door",
            Point3::default(),
            Vec::new(),
        ));
        let inserted = document.add_entity(insert("Door")).id;
        let source = std::sync::Arc::new(document);
        app.document = Some(std::sync::Arc::clone(&source));
        let (sender, receiver) = std::sync::mpsc::channel();
        app.blocks_panel.library_job = Some(LibraryJob {
            receiver,
            source: Some((
                std::sync::Arc::downgrade(&source),
                source.content_generation(),
            )),
            space: Some(EntitySpace::ModelSpace),
        });
        sender
            .send(Ok(LibraryResult::Imported {
                document: Document::default(),
                edits: Vec::new(),
                inserted,
                name: "Unwanted".into(),
            }))
            .unwrap();
        app.block_edit
            .enter(&source, &app.history, inserted)
            .unwrap();
        poll_library_job(&mut app);
        assert!(std::sync::Arc::ptr_eq(
            app.document.as_ref().unwrap(),
            &source
        ));
        assert!(app
            .blocks_panel
            .error
            .as_ref()
            .unwrap()
            .contains("drawing changed"));
        assert!(!app.history.can_undo());
    }

    #[test]
    fn opening_another_drawing_cancels_a_pending_library_insert() {
        let mut app = MyCadApp::for_test();
        let source = std::sync::Arc::new(Document::default());
        app.document = Some(std::sync::Arc::clone(&source));
        let (sender, receiver) = std::sync::mpsc::channel();
        app.blocks_panel.library_job = Some(LibraryJob {
            receiver,
            source: Some((
                std::sync::Arc::downgrade(&source),
                source.content_generation(),
            )),
            space: Some(EntitySpace::ModelSpace),
        });
        app.blocks_panel.cancel_library_insert_for_open();
        assert!(sender.send(Err("late result".into())).is_err());
        poll_library_job(&mut app);
        assert!(app.blocks_panel.library_job.is_none());
        assert!(std::sync::Arc::ptr_eq(
            app.document.as_ref().unwrap(),
            &source
        ));
        assert!(!app.history.can_undo());
    }

    #[test]
    fn tree_walk_stops_on_cycles_and_groups_counts() {
        let mut document = Document::default();
        document.replace_block_definition(BlockDefinition {
            name: "Motor".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![insert("Machine")],
            ..Default::default()
        });
        document.replace_block_definition(BlockDefinition {
            name: "Machine".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![insert("Motor"), insert("Motor")],
            ..Default::default()
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
            ..Default::default()
        });
        document.replace_block_definition(BlockDefinition {
            name: "Machine".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![insert("Motor")],
            ..Default::default()
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
            ..Default::default()
        });
        document.replace_block_definition(BlockDefinition {
            name: "Machine".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![insert("Motor")],
            ..Default::default()
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
            ..Default::default()
        });
        document.replace_block_definition(BlockDefinition {
            name: "Drive".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: Vec::new(),
            ..Default::default()
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

    #[test]
    fn first_insert_name_skips_plain_geometry() {
        let mut document = Document::default();
        document.replace_block_definition(BlockDefinition {
            name: "Door".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: Vec::new(),
            ..Default::default()
        });
        let line = document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(1.0, 0.0),
        }));
        let insert = document.add_entity(insert("Door"));
        assert_eq!(
            first_insert_definition_name(&document, &[line.id, insert.id]).as_deref(),
            Some("Door")
        );
        assert_eq!(first_insert_definition_name(&document, &[line.id]), None);
    }

    #[test]
    fn world_extents_use_current_space_inserts() {
        let mut document = Document::default();
        document.replace_block_definition(BlockDefinition {
            name: "Leaf".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![Entity::new(Geometry::Line {
                start: Point3::from_xy(0.0, 0.0),
                end: Point3::from_xy(2.0, 0.0),
            })],
            ..Default::default()
        });
        document.replace_block_definition(BlockDefinition {
            name: "Holder".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![insert("Leaf")],
            ..Default::default()
        });
        document.add_entity(insert("Leaf"));
        let model = world_extents_of_named_inserts(
            &document,
            "Leaf",
            &EntitySpace::ModelSpace,
            Transform2::identity(),
        )
        .expect("model extents");
        assert!(model.width() > 0.0);
        let nested = world_extents_of_named_inserts(
            &document,
            "Leaf",
            &EntitySpace::Block("Holder".into()),
            Transform2::translate(100.0, 0.0),
        )
        .expect("nested extents");
        assert!((nested.min.x - 100.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_and_select_actions_are_not_edit() {
        assert!(!matches!(
            TreeAction::Select(Some("Door".into())),
            TreeAction::Edit(_)
        ));
        assert!(!matches!(
            TreeAction::ZoomTo("Door".into()),
            TreeAction::Edit(_)
        ));
    }
}
