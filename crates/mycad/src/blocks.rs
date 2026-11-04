//! Dockable Blocks tree: definition hierarchy, counts, and inline rename.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

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
use crate::library_preview::{paint_thumbnail, preview_for_asset, LibraryPreview};

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
    library_restored: bool,
    library_favorites_only: bool,
    library_selected: Option<PathBuf>,
    library_description: String,
    thumbnails: HashMap<PathBuf, Result<LibraryPreview, String>>,
    thumbnail_job: Option<ThumbnailJob>,
    placement: Option<LibraryPlacement>,
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
            library_restored: false,
            library_favorites_only: false,
            library_selected: None,
            library_description: String::new(),
            thumbnails: HashMap::new(),
            thumbnail_job: None,
            placement: None,
        }
    }
}

impl BlocksPanel {
    pub(crate) fn cancel_library_insert_for_open(&mut self) {
        self.placement = None;
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
struct ThumbnailJob {
    path: PathBuf,
    receiver: std::sync::mpsc::Receiver<Result<LibraryPreview, String>>,
}

#[derive(Debug)]
struct LibraryPlacement {
    document: Document,
    edits: Vec<crate::history::Edit>,
    inserted: EntityId,
    name: String,
    preview: LibraryPreview,
    source: Weak<Document>,
    generation: u64,
    space: EntitySpace,
    point: cad_core::Point2,
    rotation: f64,
}

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
        preview: LibraryPreview,
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
    let preview = preview_for_asset(&asset)?;
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
        preview,
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
            app.settings.library_browser.remember_folder(folder.clone());
            app.blocks_panel.library_folder = Some(folder);
            app.blocks_panel.library_files = files;
            app.blocks_panel.thumbnails.clear();
            app.blocks_panel.thumbnail_job = None;
            app.blocks_panel.library_selected = None;
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
            preview,
        }) => {
            if !library_authoring_available(app) {
                app.blocks_panel.error = Some("Finish the current block setup or leave Test mode before inserting a library block.".into());
                return;
            }
            app.cancel_active_command();
            let (Some((source, generation)), Some(space)) = (job.source, job.space) else {
                app.blocks_panel.error =
                    Some("The insertion source is no longer available.".into());
                return;
            };
            let point = app.block_edit.local_from_world().apply(app.camera.center);
            app.status = format!("Place {name}: click a snap point; R rotates 90°; Esc cancels");
            app.blocks_panel.placement = Some(LibraryPlacement {
                document,
                edits,
                inserted,
                name,
                preview,
                source,
                generation,
                space,
                point,
                rotation: 0.0,
            });
        }
        Err(err) => app.blocks_panel.error = Some(err),
    }
}

pub(crate) fn library_placement_active(app: &MyCadApp) -> bool {
    app.blocks_panel.placement.is_some()
}

pub(crate) fn library_source_pending(app: &MyCadApp) -> bool {
    library_placement_active(app)
        || app
            .blocks_panel
            .library_job
            .as_ref()
            .is_some_and(|job| job.source.is_some())
}

pub(crate) fn update_library_placement(app: &mut MyCadApp, point: cad_core::Point2) {
    if point.is_finite() {
        if let Some(placement) = &mut app.blocks_panel.placement {
            placement.point = point;
        }
    }
}

pub(crate) fn rotate_library_placement(app: &mut MyCadApp, degrees: f64) {
    if degrees.is_finite() {
        if let Some(placement) = &mut app.blocks_panel.placement {
            placement.rotation =
                (placement.rotation + degrees.to_radians()).rem_euclid(std::f64::consts::TAU);
        }
    }
}

pub(crate) fn cancel_library_placement(app: &mut MyCadApp) -> bool {
    let pending = app
        .blocks_panel
        .library_job
        .as_ref()
        .is_some_and(|job| job.source.is_some());
    if pending {
        app.blocks_panel.library_job = None;
    }
    let active = app.blocks_panel.placement.take().is_some();
    if pending || active {
        app.status = "Library placement canceled".into();
    }
    pending || active
}

fn placement_is_current(app: &MyCadApp, placement: &LibraryPlacement) -> bool {
    app.document.as_ref().is_some_and(|current| {
        placement.source.ptr_eq(&Arc::downgrade(current))
            && placement.generation == current.content_generation()
    }) && placement.space == app.block_edit.active_space()
        && library_authoring_available(app)
}

/// Commit the worker-prepared transaction without cloning the drawing on click.
pub(crate) fn commit_library_placement(app: &mut MyCadApp) -> bool {
    let Some(mut placement) = app.blocks_panel.placement.take() else {
        return false;
    };
    if !placement_is_current(app, &placement) {
        app.blocks_panel.error =
            Some("The drawing changed during placement. Insert the block again.".into());
        return true;
    }
    if app.io_busy() || app.loading_path.is_some() {
        app.status = "Wait for the current file operation before placing the block".into();
        app.blocks_panel.placement = Some(placement);
        return true;
    }
    let Some(entity) = placement.document.entity_by_id(placement.inserted) else {
        app.blocks_panel.error = Some("The prepared block is unavailable. Insert it again.".into());
        return true;
    };
    let mut entity = entity.clone();
    if let Geometry::Insert {
        insertion,
        rotation,
        ..
    } = &mut entity.geometry
    {
        insertion.x = placement.point.x;
        insertion.y = placement.point.y;
        *rotation = placement.rotation;
    }
    if placement
        .document
        .replace_entity_in(&placement.space, placement.inserted, entity.clone())
        .is_none()
    {
        app.blocks_panel.error = Some("The placement destination no longer exists.".into());
        return true;
    }
    if let Some(crate::history::Edit::InsertEntity {
        entity: recorded, ..
    }) = placement.edits.last_mut()
    {
        *recorded = entity;
    } else {
        app.blocks_panel.error = Some("The prepared insertion has no undo record.".into());
        return true;
    }
    placement.document.mark_extents_stale();
    app.block_edit.refresh_dirty(&placement.document);
    app.document = Some(Arc::new(placement.document));
    app.history.begin();
    for edit in placement.edits {
        app.history.record(edit);
    }
    app.history.commit_open();
    app.selection.replace(placement.inserted);
    app.blocks_panel.selected = Some(placement.name.clone());
    app.schedule_full_refresh();
    app.status = format!("Placed {} from library", placement.name);
    true
}

pub(crate) fn paint_library_placement(
    app: &MyCadApp,
    painter: &egui::Painter,
    world_to_screen: impl Fn(cad_core::Point2) -> egui::Pos2,
) {
    let Some(placement) = &app.blocks_panel.placement else {
        return;
    };
    if !placement_is_current(app, placement) {
        return;
    }
    let transform = app
        .block_edit
        .world_from_local()
        .then(Transform2::translate(placement.point.x, placement.point.y))
        .then(Transform2::rotate(placement.rotation));
    for line in &placement.preview.lines {
        let start = world_to_screen(transform.apply(line[0]));
        let end = world_to_screen(transform.apply(line[1]));
        if start.is_finite() && end.is_finite() {
            painter.line_segment([start, end], egui::Stroke::new(1.5_f32, ACCENT));
        }
    }
    let origin = world_to_screen(app.block_edit.world_from_local().apply(placement.point));
    if origin.is_finite() {
        painter.circle_stroke(origin, 5.0, egui::Stroke::new(1.0_f32, ACCENT));
    }
}

fn poll_thumbnail(app: &mut MyCadApp) {
    let Some(job) = &app.blocks_panel.thumbnail_job else {
        return;
    };
    let result = match job.receiver.try_recv() {
        Ok(result) => result,
        Err(std::sync::mpsc::TryRecvError::Empty) => return,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            Err("Preview worker ended unexpectedly".into())
        }
    };
    let Some(job) = app.blocks_panel.thumbnail_job.take() else {
        return;
    };
    if app.blocks_panel.library_files.contains(&job.path) {
        // Keep the transient preview cache bounded even in a large library.
        if app.blocks_panel.thumbnails.len() >= 128 {
            if let Some(old) = app.blocks_panel.thumbnails.keys().next().cloned() {
                app.blocks_panel.thumbnails.remove(&old);
            }
        }
        app.blocks_panel.thumbnails.insert(job.path, result);
    }
}

fn read_library_asset(path: &Path) -> Result<cad_io::BlockAsset, String> {
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Choose a regular library block file.".into());
    }
    if metadata.len() > MAX_LIBRARY_FILE_BYTES {
        return Err("The library block exceeds the 64 MB size limit.".into());
    }
    cad_io::read_mycadblock(path).map_err(|error| error.to_string())
}

fn start_thumbnail(app: &mut MyCadApp, ctx: &egui::Context, path: PathBuf) {
    if app.blocks_panel.thumbnail_job.is_some() {
        return;
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    let worker_path = path.clone();
    match std::thread::Builder::new()
        .name("mycad-library-thumbnail".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                preview_for_asset(&read_library_asset(&worker_path)?)
            }))
            .unwrap_or_else(|_| Err("Could not render this library preview".into()));
            let _ = sender.send(result);
            ctx.request_repaint();
        }) {
        Ok(_) => app.blocks_panel.thumbnail_job = Some(ThumbnailJob { path, receiver }),
        Err(error) => {
            app.blocks_panel
                .thumbnails
                .insert(path, Err(error.to_string()));
        }
    }
}

fn library_authoring_available(app: &MyCadApp) -> bool {
    matches!(app.block_edit.ui, crate::block_edit::BlockUi::None)
        && app.dynamic_authoring.as_ref().is_none_or(|state| {
            state.mode == crate::dynamic_block::AuthoringMode::Author
                && state.pick == crate::dynamic_block::AuthoringPick::Idle
                && state.wizard.is_none()
                && state.attach.is_none()
                && state.size_wizard.is_none()
                && !state.action_inspector.has_draft()
        })
}

fn show_library(ui: &mut Ui, app: &mut MyCadApp) {
    poll_thumbnail(app);
    if !app.blocks_panel.library_restored {
        app.blocks_panel.library_restored = true;
        if let Some(folder) = app.settings.library_browser.selected_folder.clone() {
            start_library_job(app, ui.ctx(), None, move || list_library(folder));
        }
    }
    egui::CollapsingHeader::new("Block library").default_open(true).show(ui, |ui| {
        ui.weak("Choose a block, then snap and click to place it.");
        ui.weak("Blocks use the current drawing's units.");
        let busy = app.blocks_panel.library_job.is_some();
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Open library…").clicked() {
                    if let Some(folder) = rfd::FileDialog::new().set_title("Choose a block library folder").pick_folder() {
                        start_library_job(app, ui.ctx(), None, move || list_library(folder));
                    }
                }
                if ui.button("New library…").clicked() {
                    if let Some(parent) = rfd::FileDialog::new().set_title("Choose where to create the library").pick_folder() {
                        app.blocks_panel.new_library = Some((parent, "My library".into()));
                    }
                }
                if let Some(folder) = app.blocks_panel.library_folder.clone() {
                    if ui.button("Refresh").clicked() {
                        start_library_job(app, ui.ctx(), None, move || list_library(folder));
                    }
                }
            });
            let mut open_recent = None;
            egui::ComboBox::from_id_salt("recent-library-folders")
                .selected_text("Recent libraries")
                .show_ui(ui, |ui| {
                    for folder in &app.settings.library_browser.recent_folders {
                        if ui.selectable_label(app.blocks_panel.library_folder.as_ref() == Some(folder), folder.display().to_string()).clicked() {
                            open_recent = Some(folder.clone());
                        }
                    }
                    if app.settings.library_browser.recent_folders.is_empty() { ui.weak("No recent libraries"); }
                });
            if let Some(folder) = open_recent {
                start_library_job(app, ui.ctx(), None, move || list_library(folder));
            }
            let mut create = None;
            let mut cancel = false;
            if let Some((parent, name)) = &mut app.blocks_panel.new_library {
                ui.label(format!("Create in {}", parent.display()));
                ui.text_edit_singleline(name);
                ui.horizontal(|ui| {
                    if ui.add_enabled(valid_library_name(name), egui::Button::new("Create folder")).clicked() { create = Some(parent.join(name.trim())); }
                    cancel = ui.button("Cancel").clicked();
                });
                ui.weak("Use a folder name without path separators.");
            }
            if cancel { app.blocks_panel.new_library = None; }
            if let Some(folder) = create {
                app.blocks_panel.new_library = None;
                start_library_job(app, ui.ctx(), None, move || {
                    std::fs::create_dir(&folder).map_err(|err| err.to_string())?;
                    list_library(folder)
                });
            }
            if let Some(folder) = app.blocks_panel.library_folder.clone() {
                ui.label(folder.file_name().unwrap_or_default().to_string_lossy()).on_hover_text(folder.display().to_string());
                let name = app.block_edit.current().map(|frame| frame.block_name.clone()).or_else(|| app.blocks_panel.selected.clone());
                let enabled = app.document.is_some() && name.is_some() && !app.io_busy() && library_authoring_available(app);
                let label = name.as_ref().map(|name| format!("Save {name} to library")).unwrap_or_else(|| "Save selected block to library".into());
                if ui.add_enabled(enabled, egui::Button::new(label)).on_hover_text("Choose a block in the drawing tree or edit a block. Existing assets are kept.").clicked() {
                    if let (Some(source), Some(name)) = (app.document.clone(), name) {
                        start_library_job(app, ui.ctx(), None, move || {
                            save_library_block(&source, &name, &folder)?;
                            drop(source);
                            list_library(folder)
                        });
                    }
                }
            }
        });
        if let Some(placement) = app.blocks_panel.placement.as_mut() {
            ui.separator();
            ui.strong(format!("Placing {}", placement.name));
            ui.label("Click a snap point. R: rotate 90°. Esc: cancel.");
            let mut angle = placement.rotation.to_degrees();
            ui.horizontal(|ui| {
                ui.label("Rotation");
                if ui.add(egui::DragValue::new(&mut angle).suffix("°").speed(1.0)).changed() && angle.is_finite() {
                    placement.rotation = angle.to_radians().rem_euclid(std::f64::consts::TAU);
                }
                if ui.button("+90°").clicked() { placement.rotation = (placement.rotation + std::f64::consts::FRAC_PI_2).rem_euclid(std::f64::consts::TAU); }
            });
            if placement.preview.simplified { ui.weak("Simplified placement preview; all geometry will be inserted."); }
            if ui.button("Cancel placement").clicked() { cancel_library_placement(app); }
        }
        ui.separator();
        ui.add(TextEdit::singleline(&mut app.blocks_panel.library_search).hint_text("Find a name or description"));
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut app.blocks_panel.library_favorites_only, "Favorites only");
            ui.checkbox(&mut app.settings.library_preview.show_thumbnails, "Previews");
        });
        let query = app.blocks_panel.library_search.trim().to_lowercase();
        let favorites: HashSet<_> = app.settings.library_browser.favorites.iter().collect();
        let files: Vec<_> = app.blocks_panel.library_files.iter().filter(|path| {
            (!app.blocks_panel.library_favorites_only || favorites.contains(path))
                && library_item_matches(path, app.settings.library_browser.descriptions.get(*path).map(String::as_str).unwrap_or_default(), &query)
        }).cloned().collect();
        let mut insert = None;
        let mut thumbnail = None;
        let mut selected = None;
        let mut toggle_favorite = None;
        let available = app.document.is_some() && !app.io_busy() && app.loading_path.is_none() && !busy && library_authoring_available(app);
        let previews = app.settings.library_preview.show_thumbnails;
        let row_height = if previews { 66.0 } else { 32.0 };
        egui::ScrollArea::vertical().id_salt("library-files").max_height(330.0)
            .show_rows(ui, row_height, files.len(), |ui, range| {
                for index in range {
                    let Some(path) = files.get(index) else { continue };
                    ui.push_id(path, |ui| {
                        ui.horizontal(|ui| {
                            if previews {
                                let cached = app.blocks_panel.thumbnails.get(path);
                                paint_thumbnail(ui, cached.and_then(|value| value.as_ref().ok()));
                                if cached.is_none() && thumbnail.is_none() && app.settings.library_preview.generate_missing { thumbnail = Some(path.clone()); }
                            }
                            ui.vertical(|ui| {
                                let name = path.file_stem().unwrap_or_default().to_string_lossy();
                                if ui.selectable_label(app.blocks_panel.library_selected.as_ref() == Some(path), name).on_hover_text(path.display().to_string()).clicked() { selected = Some(path.clone()); }
                                ui.horizontal(|ui| {
                                    let favorite = favorites.contains(path);
                                    if ui.small_button(if favorite { "Unfavorite" } else { "Favorite" }).clicked() { toggle_favorite = Some(path.clone()); }
                                    if ui.add_enabled(available, egui::Button::new("Place")).clicked() { insert = Some(path.clone()); }
                                });
                            });
                        });
                    });
                }
            });
        if files.is_empty() { ui.weak("No matching blocks. Save a block here or open another library."); }
        if let Some(path) = toggle_favorite {
            let favorites = &mut app.settings.library_browser.favorites;
            if favorites.contains(&path) { favorites.retain(|item| item != &path); }
            else if favorites.len() < MAX_LIBRARY_ITEMS { favorites.push(path); }
        }
        if let Some(path) = selected {
            app.blocks_panel.library_description = app.settings.library_browser.descriptions.get(&path).cloned().unwrap_or_default();
            app.blocks_panel.library_selected = Some(path);
        }
        if let Some(path) = app.blocks_panel.library_selected.clone() {
            ui.separator();
            ui.label("Personal description");
            ui.add(TextEdit::multiline(&mut app.blocks_panel.library_description).desired_rows(2).char_limit(500).hint_text("Describe this block for search"));
            if ui.button("Save description").clicked() {
                let value = app.blocks_panel.library_description.trim().to_owned();
                if value.is_empty() { app.settings.library_browser.descriptions.remove(&path); }
                else { app.settings.library_browser.descriptions.insert(path.clone(), value); }
                app.status = "Library description saved in your preferences".into();
            }
            ui.weak("Personal descriptions travel with exported preferences, not with the block file.");
            if let Some(Err(error)) = app.blocks_panel.thumbnails.get(&path) { ui.colored_label(CYCLE_COLOR, error); }
            if ui.button("Generate preview").clicked() { thumbnail = Some(path); }
        }
        if let Some(path) = thumbnail {
            if app.blocks_panel.thumbnail_job.is_none() && !busy { start_thumbnail(app, ui.ctx(), path); }
        }
        if let Some(path) = insert {
            cancel_library_placement(app);
            app.cancel_active_command();
            if let Some(source) = app.document.clone() {
                let space = app.block_edit.active_space();
                let snapshot = Arc::clone(&source);
                start_library_job(app, ui.ctx(), Some(source), move || {
                    let document = snapshot.as_ref().clone();
                    drop(snapshot);
                    prepare_library_insert(document, read_library_asset(&path)?, space, cad_core::Point2::default())
                });
            }
        }
        if let Some(error) = &app.blocks_panel.error { ui.colored_label(CYCLE_COLOR, error); }
        if busy { ui.horizontal(|ui| { ui.spinner(); ui.label("Working on library…"); }); }
        if busy || app.blocks_panel.thumbnail_job.is_some() { ui.ctx().request_repaint_after(std::time::Duration::from_millis(50)); }
    });
}

fn library_item_matches(path: &Path, description: &str, query: &str) -> bool {
    path.file_stem()
        .is_some_and(|name| name.to_string_lossy().to_lowercase().contains(query))
        || description.to_lowercase().contains(query)
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

    fn staged_placement_app() -> (MyCadApp, Arc<Document>, EntityId) {
        let mut host = Document::default();
        let selected = host
            .add_entity(Entity::new(Geometry::Line {
                start: Point3::new(50.0, 0.0, 0.0),
                end: Point3::new(60.0, 0.0, 0.0),
            }))
            .id;
        let source = Arc::new(host);
        let mut app = MyCadApp::for_test();
        app.document = Some(Arc::clone(&source));
        app.selection.replace(selected);
        let asset = cad_io::BlockAsset {
            blocks: vec![BlockDefinition::plain(
                "Door",
                Point3::default(),
                vec![Entity::new(Geometry::Line {
                    start: Point3::default(),
                    end: Point3::new(10.0, 0.0, 0.0),
                })],
            )],
            layers: Vec::new(),
            linetypes: Vec::new(),
            text_styles: Vec::new(),
        };
        let prepared = prepare_library_insert(
            source.as_ref().clone(),
            asset,
            EntitySpace::ModelSpace,
            cad_core::Point2::default(),
        )
        .unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        app.blocks_panel.library_job = Some(LibraryJob {
            receiver,
            source: Some((Arc::downgrade(&source), source.content_generation())),
            space: Some(EntitySpace::ModelSpace),
        });
        sender.send(Ok(prepared)).unwrap();
        poll_library_job(&mut app);
        (app, source, selected)
    }

    #[test]
    fn library_placement_is_a_preview_until_click_and_commits_one_undoable_transaction() {
        let (mut app, source, selected) = staged_placement_app();
        assert!(library_placement_active(&app));
        assert!(Arc::ptr_eq(app.document.as_ref().unwrap(), &source));
        assert_eq!(app.selection.ids(), &[selected]);
        assert_eq!(app.history.undo_len(), 0);
        assert!(source.blocks.is_empty());
        update_library_placement(&mut app, cad_core::Point2::new(25.0, 50.0));
        rotate_library_placement(&mut app, 90.0);
        assert!(commit_library_placement(&mut app));
        assert!(!library_placement_active(&app));
        assert_eq!(app.history.undo_len(), 1);
        let document = Arc::get_mut(app.document.as_mut().unwrap()).unwrap();
        let inserted = document.model_space.last().unwrap();
        let Geometry::Insert {
            insertion,
            rotation,
            ..
        } = &inserted.geometry
        else {
            panic!("insert")
        };
        assert_eq!(*insertion, Point3::new(25.0, 50.0, 0.0));
        assert_eq!(*rotation, std::f64::consts::FRAC_PI_2);
        assert!(app.history.undo(document));
        assert_eq!(document.model_space.len(), 1);
        assert_eq!(document.model_space[0].id, selected);
        assert!(document.blocks.is_empty());
        assert!(app.history.redo(document));
        assert_eq!(document.model_space.len(), 2);
        let Geometry::Insert {
            insertion,
            rotation,
            ..
        } = &document.model_space[1].geometry
        else {
            panic!("insert")
        };
        assert_eq!(*insertion, Point3::new(25.0, 50.0, 0.0));
        assert_eq!(*rotation, std::f64::consts::FRAC_PI_2);
    }

    #[test]
    fn cancel_library_placement_preserves_document_selection_and_history() {
        let (mut app, source, selected) = staged_placement_app();
        update_library_placement(&mut app, cad_core::Point2::new(3.0, 4.0));
        assert!(cancel_library_placement(&mut app));
        assert!(!commit_library_placement(&mut app));
        assert!(Arc::ptr_eq(app.document.as_ref().unwrap(), &source));
        assert_eq!(app.selection.ids(), &[selected]);
        assert_eq!(app.history.undo_len(), 0);
        assert!(source.blocks.is_empty());
    }

    #[test]
    fn changed_drawing_rejects_a_staged_library_placement() {
        let (mut app, _, selected) = staged_placement_app();
        let replacement = Arc::new(Document::default());
        app.document = Some(Arc::clone(&replacement));
        assert!(commit_library_placement(&mut app));
        assert!(Arc::ptr_eq(app.document.as_ref().unwrap(), &replacement));
        assert_eq!(app.selection.ids(), &[selected]);
        assert_eq!(app.history.undo_len(), 0);
        assert!(app
            .blocks_panel
            .error
            .as_deref()
            .unwrap()
            .contains("changed during placement"));
    }

    #[test]
    fn library_search_includes_personal_descriptions() {
        let path = Path::new("Door.mycadblock");
        assert!(library_item_matches(path, "Fire rated steel", "steel"));
        assert!(library_item_matches(path, "Fire rated steel", "door"));
        assert!(!library_item_matches(path, "Fire rated steel", "window"));
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
                preview: LibraryPreview::default(),
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
                preview: LibraryPreview::default(),
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
