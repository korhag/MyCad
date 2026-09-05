//! Application chrome: menus, viewport interaction, loading, workspace.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use cad_core::{
    create_block_from_entities, duplicate_block_definition, identity_insert, insert_instance_ids,
    make_unique_block, membership_matrix, purge_unused_user_blocks, reference_radius,
    transfer_entity, transform_entity_matrix, transform_geometry, validate_block_rename,
    validate_entities, would_create_block_cycle, BlockTreeIndex, Document, Entity, EntityId,
    EntitySpace, EntityTransform, Extents2, Geometry, MeasureIndex, MeasureRole, MeasurementResult,
    Point2, Point3, SnapFeature, SnapIndex, Transform2, TransformError, GEOM_TOLERANCE,
};
use cad_io::{
    export_pdf, write_dxf, CadFileFormat, DxfExportOptions, PdfExportOptions, PdfOrientation,
    PdfPaperSize, PdfPlotArea, PdfPlotStyle, SaveReport, PDF_MARGIN_MM, PDF_STROKE_WEIGHTS,
};
use cad_render::{
    tessellate_document, tessellate_document_for_block_edit, CadFrame, CadGpu, DisplayList,
    GpuUpload, OverlayBatches, SelectBoxMode,
};
use cad_viewport::Camera2;
use dwg_import::{write_dwg, DwgWriteError, ExportError as DwgExportError, ImportError};
use eframe::egui::{self, PointerButton, Rect, Ui};
use egui_phosphor::regular::FLOPPY_DISK;

use crate::block_edit::{
    self, insert_is_editable, BlockEditSession, BlockUi, CreateBlockDialog, CreateDialogResult,
    LeaveChoice, LeaveIntent, ToolbarAction,
};
use crate::blocks::BlocksPanel;
use crate::commands::{
    AngleState, AreaState, CommandKind, CommandOutput, CommandState, ModifyKind,
};
use crate::context_menu::{self, ContextAction, ContextKind, MenuResult, ViewportMenu};
use crate::drafting::DraftingState;
use crate::dynamic_input::{DynamicInput, DynamicKeyResult, DynamicLayout, LiveValues};
use crate::history::{Edit, History};
use crate::input::{InputAction, InputMap};
use crate::measurement::{self, CardAction, MeasurementOverlay};
use crate::selection::{box_pick_entities_into, pick_entity, Selection, SelectionOp};
use crate::settings::{scroll_to_zoom_factor, AppSettings, RgbColor};
use crate::settings_ui::{self, CaptureTarget, SettingsAction, SettingsTab};
use crate::theme;
use crate::workspace::{self, WorkspaceTab};

const SELECTION_OVERLAY_COLOR: [f32; 4] = [255.0 / 255.0, 196.0 / 255.0, 72.0 / 255.0, 1.0];
const SAVE_TOOLTIP: &str = "Save\nCtrl+S";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PdfPlotAreaKind {
    Extents,
    Window,
}

impl PdfPlotAreaKind {
    fn label(self) -> &'static str {
        match self {
            Self::Extents => "Extents",
            Self::Window => "Window",
        }
    }
}

struct PdfPlotDialogState {
    options: PdfExportOptions,
    area: PdfPlotAreaKind,
    window: Option<Extents2>,
}

struct PlotWindowPick {
    options: PdfExportOptions,
    last_window: Option<Extents2>,
    first: Option<Point2>,
}

enum PdfPlotUi {
    Closed,
    Dialog(PdfPlotDialogState),
    PickWindow(PlotWindowPick),
}

impl PdfPlotUi {
    fn is_closed(&self) -> bool {
        matches!(self, Self::Closed)
    }

    fn is_picking(&self) -> bool {
        matches!(self, Self::PickWindow(_))
    }
}

enum LoadMsg {
    Success {
        document: Box<Document>,
        display: Box<DisplayList>,
        snaps: Box<SnapIndex>,
        measures: Box<MeasureIndex>,
    },
    Failure {
        path: PathBuf,
        message: String,
    },
}

struct BoxSelectDrag {
    button: PointerButton,
    start: egui::Pos2,
    current: egui::Pos2,
    op: SelectionOp,
    candidates: Vec<cad_core::EntityId>,
}

enum PendingDiscard {
    OpenDialog,
    Open(PathBuf),
    Quit,
}

struct KeyChord {
    f3: bool,
    f8: bool,
    line: bool,
    polyline: bool,
    circle: bool,
    arc: bool,
    rectangle: bool,
    distance: bool,
    enter: bool,
    escape: bool,
    undo: bool,
    redo: bool,
    open: bool,
    save: bool,
    save_as: bool,
}

// ------------------------------------------------------------
// Type: MyCadApp
// Purpose: Native GUI shell around the CAD document and wgpu viewport.
// ------------------------------------------------------------
pub struct MyCadApp {
    pub(crate) camera: Camera2,
    pub(crate) document: Option<Document>,
    pub(crate) display: Arc<DisplayList>,
    display_generation: u64,
    gpu_upload: GpuUpload,
    load_rx: Option<Receiver<LoadMsg>>,
    pub(crate) loading_path: Option<PathBuf>,
    status: String,
    pub(crate) error: Option<String>,
    pub(crate) settings: AppSettings,
    pub(crate) settings_draft: AppSettings,
    pub(crate) show_settings: bool,
    pub(crate) settings_message: Option<String>,
    pub(crate) imported_dock: bool,
    pub(crate) capture: Option<CaptureTarget>,
    pub(crate) settings_tab: SettingsTab,
    pub(crate) dock_state: egui_dock::DockState<WorkspaceTab>,
    pub(crate) selection: Selection,
    pub(crate) drafting: DraftingState,
    command: CommandState,
    last_command: Option<CommandKind>,
    dynamic_input: DynamicInput,
    context_menu: Option<ViewportMenu>,
    history: History,
    snaps: Arc<SnapIndex>,
    measures: Arc<MeasureIndex>,
    measurement: Option<MeasurementOverlay>,
    measure_card_hovered: bool,
    viewport_height: f64,
    cursor_world: Option<Point2>,
    pub(crate) input_consumed_escape: bool,
    pending_open: Option<PathBuf>,
    pending_discard: Option<PendingDiscard>,
    pending_lossy_save: bool,
    /// True after MyCAD wrote `document.source_path` this session.
    source_written_by_mycad: bool,
    pdf_plot: PdfPlotUi,
    last_pointer: Option<egui::Pos2>,
    box_select: Option<BoxSelectDrag>,
    command_snaps: Vec<SnapFeature>,
    pub(crate) block_edit: BlockEditSession,
    pub(crate) blocks_panel: BlocksPanel,
    pub(crate) block_tree: BlockTreeIndex,
}

impl MyCadApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_path: Option<PathBuf>) -> Self {
        theme::apply(&cc.egui_ctx);
        if let Some(render_state) = &cc.wgpu_render_state {
            let mut renderer = render_state.renderer.write();
            renderer.callback_resources.insert(CadGpu::new(
                &render_state.device,
                render_state.target_format,
            ));
        }
        let settings = AppSettings::load(cc.storage);
        let mut app = Self::from_settings(settings, initial_path);
        workspace::sanitize_dock_state(&mut app.dock_state);
        if let Some(path) = app.pending_open.take() {
            app.start_load(path);
        }
        app
    }

    fn from_settings(settings: AppSettings, initial_path: Option<PathBuf>) -> Self {
        let drafting_preferences = settings.drafting;
        Self {
            camera: Camera2::default(),
            document: None,
            display: Arc::new(DisplayList::default()),
            display_generation: 0,
            gpu_upload: GpuUpload::Full,
            load_rx: None,
            loading_path: None,
            status: "Ready".to_string(),
            error: None,
            settings_draft: settings.clone(),
            dock_state: settings.dock_state(),
            settings,
            show_settings: false,
            settings_message: None,
            imported_dock: false,
            capture: None,
            settings_tab: SettingsTab::Viewport,
            selection: Selection::default(),
            drafting: DraftingState::new(drafting_preferences),
            command: CommandState::Idle,
            last_command: None,
            dynamic_input: DynamicInput::default(),
            context_menu: None,
            history: History::default(),
            snaps: Arc::new(SnapIndex::default()),
            measures: Arc::new(MeasureIndex::default()),
            measurement: None,
            measure_card_hovered: false,
            viewport_height: 600.0,
            cursor_world: None,
            input_consumed_escape: false,
            pending_open: initial_path,
            pending_discard: None,
            pending_lossy_save: false,
            source_written_by_mycad: false,
            pdf_plot: PdfPlotUi::Closed,
            last_pointer: None,
            box_select: None,
            command_snaps: Vec::new(),
            block_edit: BlockEditSession::default(),
            blocks_panel: BlocksPanel::default(),
            block_tree: BlockTreeIndex::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        let mut app = Self::from_settings(AppSettings::default(), None);
        workspace::sanitize_dock_state(&mut app.dock_state);
        app
    }

    fn start_load_now(&mut self, path: PathBuf) {
        if self.load_rx.is_some() {
            self.pending_open = Some(path);
            return;
        }
        self.error = None;
        self.selection.clear();
        self.box_select = None;
        self.command.cancel();
        self.measurement = None;
        self.dynamic_input.set_layout(DynamicLayout::Hidden);
        self.context_menu = None;
        self.drafting.clear_acquisition();
        self.block_edit.clear();
        self.history.commit_open();
        self.loading_path = Some(path.clone());
        self.status = format!("Loading {}…", file_name(&path));
        let (tx, rx) = mpsc::channel();
        self.load_rx = Some(rx);
        thread::Builder::new()
            .name("mycad-dwg-import".into())
            .spawn(move || match dwg_import::import_dwg(&path) {
                Ok(mut document) => {
                    let prepare = Instant::now();
                    let display = tessellate_document(&document);
                    let snaps = SnapIndex::build(&document);
                    let measures = MeasureIndex::build(&document);
                    document.diagnostics.render_prepare_time = prepare.elapsed();
                    let _ = tx.send(LoadMsg::Success {
                        document: Box::new(document),
                        display: Box::new(display),
                        snaps: Box::new(snaps),
                        measures: Box::new(measures),
                    });
                }
                Err(err) => {
                    let _ = tx.send(LoadMsg::Failure {
                        path,
                        message: format_import_error(&err),
                    });
                }
            })
            .expect("import thread");
    }

    fn poll_load(&mut self, viewport: Rect) {
        let Some(rx) = self.load_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(LoadMsg::Success {
                document,
                display,
                snaps,
                measures,
            }) => {
                let document = *document;
                let display = *display;
                if let Some(extents) = document
                    .diagnostics
                    .extents
                    .or_else(|| document.compute_extents())
                {
                    self.camera.zoom_extents(
                        extents,
                        viewport.width() as f64,
                        viewport.height() as f64,
                    );
                }
                self.status = format!(
                    "Loaded {}  •  {} entities  •  {} unsupported",
                    document.file_name(),
                    document.diagnostics.entity_total(),
                    document.diagnostics.unsupported_total()
                );
                self.selection.clear();
                self.box_select = None;
                self.command.cancel();
                self.measurement = None;
                self.drafting.clear_acquisition();
                self.history.clear();
                self.source_written_by_mycad = false;
                self.document = Some(document);
                self.rebuild_block_tree();
                self.display = Arc::new(display);
                self.snaps = Arc::new(*snaps);
                self.measures = Arc::new(*measures);
                self.gpu_upload = GpuUpload::Full;
                self.display_generation = self.display_generation.wrapping_add(1);
                self.loading_path = None;
                self.load_rx = None;
                self.error = None;
            }
            Ok(LoadMsg::Failure { path, message }) => {
                self.status = format!("Failed to open {}", file_name(&path));
                self.error = Some(message);
                self.selection.clear();
                self.box_select = None;
                self.command.cancel();
                self.drafting.clear_acquisition();
                self.loading_path = None;
                self.load_rx = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.status = "Loader thread ended unexpectedly".into();
                self.loading_path = None;
                self.load_rx = None;
            }
        }
        if self.load_rx.is_none() {
            if let Some(next) = self.pending_open.take() {
                self.start_load(next);
            }
        }
    }

    fn open_dialog(&mut self) {
        if !self.close_block_edit_for_document_action(LeaveIntent::OpenDrawing) {
            return;
        }
        if self.is_dirty() {
            self.pending_discard = Some(PendingDiscard::OpenDialog);
            return;
        }
        self.open_dialog_now();
    }

    fn start_load(&mut self, path: PathBuf) {
        if !self.close_block_edit_for_document_action(LeaveIntent::OpenDrawing) {
            self.pending_open = Some(path);
            return;
        }
        if self.is_dirty() {
            self.pending_discard = Some(PendingDiscard::Open(path));
            return;
        }
        self.start_load_now(path);
    }

    fn close_block_edit_for_document_action(&mut self, intent: LeaveIntent) -> bool {
        while self.block_edit.is_active() {
            if self.block_edit.current_is_dirty() {
                self.block_edit.ui = BlockUi::LeaveDirty {
                    name: self
                        .block_edit
                        .current()
                        .map(|frame| frame.block_name.clone())
                        .unwrap_or_default(),
                    intent,
                };
                return false;
            }
            self.block_edit.pop();
            self.selection.clear();
            self.refresh_derived();
        }
        true
    }

    fn save_drawing(&mut self) -> bool {
        if self.document.is_none() {
            self.status = "No drawing is open".into();
            return false;
        }
        if self.block_edit.current_is_dirty() {
            self.block_edit.ui = BlockUi::SaveDrawing;
            return false;
        }
        if self.request_lossy_save_warning() {
            return false;
        }
        if let Some(path) = self
            .document
            .as_ref()
            .and_then(|document| in_place_cad_path(document, self.source_written_by_mycad))
            .map(Path::to_path_buf)
        {
            self.write_cad_to(&path)
        } else {
            self.save_as_drawing_now()
        }
    }

    fn save_as_drawing(&mut self) -> bool {
        if self.document.is_none() {
            self.status = "No drawing is open".into();
            return false;
        }
        if self.block_edit.current_is_dirty() {
            self.block_edit.ui = BlockUi::SaveDrawing;
            return false;
        }
        if self.request_lossy_save_warning() {
            return false;
        }
        self.save_as_drawing_now()
    }

    fn request_lossy_save_warning(&mut self) -> bool {
        if !self.document.as_ref().is_some_and(needs_lossy_save_warning) {
            return false;
        }
        self.pending_lossy_save = true;
        true
    }

    fn save_as_drawing_now(&mut self) -> bool {
        let Some(document) = self.document.as_ref() else {
            self.status = "No drawing is open".into();
            return false;
        };
        let prefer_dwg = document
            .source_path
            .as_deref()
            .is_some_and(|path| CadFileFormat::from_path(path) == Some(CadFileFormat::Dwg));
        let mut dialog = rfd::FileDialog::new();
        if prefer_dwg {
            dialog = dialog
                .add_filter("DWG Drawing - AutoCAD 2000", &["dwg", "DWG"])
                .add_filter("DXF Drawing", &["dxf", "DXF"]);
        } else {
            dialog = dialog
                .add_filter("DXF Drawing", &["dxf", "DXF"])
                .add_filter("DWG Drawing - AutoCAD 2000", &["dwg", "DWG"]);
        }
        dialog = dialog.add_filter("All files", &["*"]);
        dialog = dialog.set_file_name(suggested_save_name(document, prefer_dwg));
        let Some(path) = dialog.save_file() else {
            return false;
        };
        self.write_cad_to(&with_save_extension(path, prefer_dwg))
    }

    fn write_cad_to(&mut self, path: &Path) -> bool {
        match CadFileFormat::from_path(path) {
            Some(CadFileFormat::Dwg) => self.write_dwg_to(path),
            _ => self.write_dxf_to(path),
        }
    }

    fn write_dxf_to(&mut self, path: &Path) -> bool {
        let Some(document) = self.document.as_ref() else {
            self.status = "No drawing is open".into();
            return false;
        };
        match write_dxf(document, path, &DxfExportOptions::default()) {
            Ok(report) => self.finish_cad_save(path, report),
            Err(err) => {
                self.status = format_save_failed(&err);
                self.error = None;
                false
            }
        }
    }

    fn write_dwg_to(&mut self, path: &Path) -> bool {
        let Some(document) = self.document.as_ref() else {
            self.status = "No drawing is open".into();
            return false;
        };
        match write_dwg(document, path) {
            Ok(report) => self.finish_cad_save(path, report),
            Err(err) => {
                self.status = format_save_failed(format_dwg_write_error(&err));
                self.error = None;
                false
            }
        }
    }

    fn finish_cad_save(&mut self, path: &Path, report: SaveReport) -> bool {
        if let Some(document) = self.document.as_mut() {
            document.source_path = Some(path.to_path_buf());
        }
        self.source_written_by_mycad = true;
        self.history.mark_clean();
        self.error = None;
        self.status = format_save_status(path, &report);
        true
    }

    fn export_pdf_dialog(&mut self) {
        if self.document.is_none() {
            self.status = "No drawing is open".into();
            return;
        }
        self.pdf_plot = PdfPlotUi::Dialog(PdfPlotDialogState {
            options: PdfExportOptions::default(),
            area: PdfPlotAreaKind::Extents,
            window: None,
        });
    }

    fn write_pdf_export(&mut self, options: PdfExportOptions) {
        let Some(document) = self.document.as_ref() else {
            self.status = "No drawing is open".into();
            return;
        };
        let mut dialog = rfd::FileDialog::new()
            .add_filter("PDF documents", &["pdf", "PDF"])
            .add_filter("All files", &["*"]);
        dialog = dialog.set_file_name(suggested_pdf_name(document));
        let Some(path) = dialog.save_file() else {
            return;
        };
        let path = with_pdf_extension(path);
        match export_pdf(document, &path, &options) {
            Ok(report) => {
                self.error = None;
                self.status = format_pdf_export_status(&path, &report);
            }
            Err(err) => {
                self.status = format_save_failed(&err);
                self.error = None;
            }
        }
    }

    fn open_dialog_now(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("DWG drawings", &["dwg", "DWG"])
            .add_filter("All files", &["*"])
            .pick_file()
        {
            self.start_load_now(path);
        }
    }

    fn open_settings(&mut self) {
        self.settings_draft = self.settings.clone();
        self.settings_draft.set_dock_state(&self.dock_state);
        self.settings_message = None;
        self.imported_dock = false;
        self.capture = None;
        self.show_settings = true;
    }

    fn apply_settings(&mut self, storage: Option<&mut (dyn eframe::Storage + 'static)>) {
        self.settings_draft.sanitize();
        if self.imported_dock {
            self.dock_state = self.settings_draft.dock_state();
            workspace::sanitize_dock_state(&mut self.dock_state);
        } else {
            self.settings_draft.set_dock_state(&self.dock_state);
        }
        self.settings = self.settings_draft.clone();
        self.drafting.preferences = self.settings.drafting;
        if let Some(storage) = storage {
            self.settings.save(storage);
        }
        self.imported_dock = false;
        self.capture = None;
        self.show_settings = false;
    }

    fn cancel_settings(&mut self) {
        self.settings_draft = self.settings.clone();
        self.imported_dock = false;
        self.capture = None;
        self.box_select = None;
        self.settings_message = None;
        self.show_settings = false;
    }

    pub(crate) fn command_kind(&self) -> CommandKind {
        self.command.kind()
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub(crate) fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    pub(crate) fn start_line_command(&mut self) {
        self.start_kind(CommandKind::Line);
    }

    pub(crate) fn start_polyline_command(&mut self) {
        self.start_kind(CommandKind::Polyline);
    }

    pub(crate) fn start_circle_command(&mut self) {
        self.start_kind(CommandKind::Circle);
    }

    pub(crate) fn start_arc_command(&mut self) {
        self.start_kind(CommandKind::Arc);
    }

    pub(crate) fn start_rectangle_command(&mut self) {
        self.start_kind(CommandKind::Rectangle);
    }

    pub(crate) fn start_distance_command(&mut self) {
        self.start_kind(CommandKind::Distance);
    }

    pub(crate) fn start_angle_command(&mut self) {
        self.start_kind(CommandKind::Angle);
    }

    pub(crate) fn start_radius_command(&mut self) {
        self.start_kind(CommandKind::Radius);
    }

    pub(crate) fn start_area_command(&mut self) {
        self.start_kind(CommandKind::Area);
    }

    pub(crate) fn start_move_command(&mut self) {
        self.start_kind(CommandKind::Move);
    }

    pub(crate) fn start_copy_command(&mut self) {
        self.start_kind(CommandKind::Copy);
    }

    pub(crate) fn start_rotate_command(&mut self) {
        self.start_kind(CommandKind::Rotate);
    }

    pub(crate) fn start_mirror_command(&mut self) {
        self.start_kind(CommandKind::Mirror);
    }

    pub(crate) fn start_scale_command(&mut self) {
        self.start_kind(CommandKind::Scale);
    }

    pub(crate) fn start_erase_command(&mut self) {
        self.start_kind(CommandKind::Erase);
    }

    // Clicking a different CAD tool cancels the current interaction with
    // the same cleanup as Esc, then starts the new tool immediately.
    fn start_kind(&mut self, kind: CommandKind) {
        if kind == CommandKind::Idle || self.command.kind() == kind {
            return;
        }
        if self.command.is_active() {
            self.cancel_command();
        }
        self.box_select = None;
        self.context_menu = None;
        self.measurement = None;
        if kind.is_measure() {
            self.finish_active_transaction();
        } else {
            self.history.commit_open();
            self.history.begin();
        }
        match kind {
            CommandKind::Line => self.command.start_line(),
            CommandKind::Polyline => self.command.start_polyline(),
            CommandKind::Circle => self.command.start_circle(),
            CommandKind::Arc => self.command.start_arc(),
            CommandKind::Rectangle => self.command.start_rectangle(),
            CommandKind::Distance => self.command.start_distance(),
            CommandKind::Angle => self.command.start_angle(),
            CommandKind::Radius => self.command.start_radius(),
            CommandKind::Area => self.command.start_area(),
            CommandKind::Move
            | CommandKind::Copy
            | CommandKind::Rotate
            | CommandKind::Mirror
            | CommandKind::Scale => {
                let selected = self.selection.ids().to_vec();
                let modify = kind.modify_kind().expect("modify kind");
                if !selected.is_empty() {
                    if let Err(err) = self.validate_modify_selection(&selected) {
                        self.history.commit_open();
                        self.status = err.to_string();
                        return;
                    }
                }
                self.command.start_modify(modify, selected.clone());
                if !selected.is_empty() {
                    self.set_modify_reference_radius();
                }
            }
            CommandKind::Erase => {
                if !self.selection.is_empty() {
                    self.erase_selected();
                    return;
                }
                self.command.start_modify(ModifyKind::Erase, Vec::new());
            }
            CommandKind::Idle => {}
        }
        self.sync_dynamic_layout();
        self.drafting.clear_acquisition();
        if kind.is_measure() {
            self.try_immediate_measurement();
        }
        if self.command.is_active() {
            self.status = self.command.prompt().into();
        }
    }

    fn sync_dynamic_layout(&mut self) {
        let layout = self.command.dynamic_layout();
        if self.dynamic_input.layout() != layout {
            self.dynamic_input.set_layout(layout);
        }
    }

    fn finish_command(&mut self) {
        if self.command.is_erase_picking() {
            self.idle_after_command("Ready");
            return;
        }
        if self.command.is_selecting_objects() {
            self.confirm_modify_selection();
            return;
        }
        match self.command.finish_measurement() {
            Some(Ok(result)) => {
                self.complete_measurement(result);
                return;
            }
            Some(Err(err)) => {
                self.status = err.message().into();
                return;
            }
            None => {}
        }
        if matches!(self.command.kind(), CommandKind::Polyline) && !self.command.can_finish() {
            return;
        }
        if let Some(geometry) = self.command.finish_geometry() {
            let kind = self.command.kind();
            self.commit_geometry(geometry);
            self.remember_completed(kind);
        }
        if self.command.kind().is_modify() {
            return;
        }
        self.idle_after_command("Ready");
    }

    fn close_command(&mut self) {
        let Some(geometry) = self.command.close_geometry() else {
            return;
        };
        let kind = self.command.kind();
        self.commit_geometry(geometry);
        self.remember_completed(kind);
        self.idle_after_command("Ready");
    }

    fn cancel_command(&mut self) {
        self.idle_after_command("Command canceled");
    }

    fn idle_after_command(&mut self, status: &str) {
        self.finish_active_transaction();
        self.command.finish();
        self.dynamic_input.set_layout(DynamicLayout::Hidden);
        self.drafting.clear_acquisition();
        self.command_snaps.clear();
        self.box_select = None;
        self.status = status.into();
    }

    fn undo_last_in_command(&mut self) {
        match self.command.kind() {
            CommandKind::Line => {
                let _ = self.command.undo_last();
            }
            CommandKind::Polyline => {
                let _ = self.command.undo_last();
            }
            CommandKind::Area => {
                let _ = self.command.undo_last();
            }
            kind if kind.is_modify() => {
                let _ = self.command.undo_last();
            }
            _ => {}
        }
        self.sync_dynamic_layout();
        self.dynamic_input.reset_values();
        self.drafting.command_base_point = self.command.base_point();
    }

    fn back_in_command(&mut self) {
        if self.command.back() {
            self.sync_dynamic_layout();
            self.dynamic_input.reset_values();
            self.drafting.command_base_point = self.command.base_point();
            self.status = self.command.prompt().into();
        }
    }

    fn accept_command_point(&mut self, point: Point2) {
        let kind = self.command.kind();
        let output = if kind.is_modify() {
            self.command.accept_modify_point(
                point,
                self.dynamic_input.typed_angle_deg(),
                self.dynamic_input.typed_factor(),
            )
        } else {
            self.command.accept_point(point)
        };
        match output {
            CommandOutput::Geometry(geometry) => {
                let stays_active = self.command.is_active();
                self.commit_geometry(geometry);
                self.remember_completed(kind);
                if stays_active {
                    self.sync_dynamic_layout();
                    self.dynamic_input.reset_values();
                    if kind == CommandKind::Line {
                        self.drafting.clear_acquisition();
                        self.command_snaps.clear();
                    }
                    self.status = self.command.prompt().into();
                } else {
                    self.finish_active_transaction();
                    self.dynamic_input.set_layout(DynamicLayout::Hidden);
                    self.drafting.clear_acquisition();
                }
            }
            CommandOutput::Distance(report) => {
                self.complete_measurement(MeasurementResult::Distance(report));
            }
            CommandOutput::Measurement(result) => {
                self.complete_measurement(result);
            }
            CommandOutput::Modify { transform, copies } => {
                self.commit_modify(transform, copies);
            }
            CommandOutput::Rejected(message) => {
                self.status = message.into();
            }
            CommandOutput::None => {
                self.sync_dynamic_layout();
                self.dynamic_input.reset_values();
                self.status = self.command.prompt().into();
                if matches!(self.command.kind(), CommandKind::Scale) {
                    self.set_modify_reference_radius();
                }
            }
        }
        self.drafting.command_base_point = self.command.base_point();
    }

    fn finish_active_transaction(&mut self) {
        self.history.commit_open();
    }

    pub(crate) fn undo(&mut self) {
        self.finish_active_transaction();
        self.command.cancel();
        self.measurement = None;
        self.dynamic_input.set_layout(DynamicLayout::Hidden);
        self.context_menu = None;
        self.drafting.clear_acquisition();
        let (undone, rename_only, rename_edits) = {
            let Some(document) = self.document.as_mut() else {
                return;
            };
            let peek = self.history.peek_undo();
            let rename_only = peek.is_some_and(|tx| tx.is_rename_only());
            let rename_edits = peek.map(|tx| tx.rename_edits()).unwrap_or_default();
            let undone = if self.block_edit.is_active() {
                self.history
                    .undo_beyond(document, self.block_edit.undo_mark())
            } else {
                self.history.undo(document)
            };
            (undone, rename_only, rename_edits)
        };
        if !undone {
            return;
        }
        if let Some(document) = self.document.as_ref() {
            self.block_edit.refresh_dirty(document);
        }
        if rename_only {
            for (before, after) in rename_edits.into_iter().rev() {
                self.sync_after_block_rename(&after, &before);
            }
        } else {
            self.refresh_derived();
        }
        self.status = "Undo".into();
    }

    pub(crate) fn redo(&mut self) {
        self.finish_active_transaction();
        self.command.cancel();
        self.measurement = None;
        self.dynamic_input.set_layout(DynamicLayout::Hidden);
        self.context_menu = None;
        self.drafting.clear_acquisition();
        let (redone, rename_only, rename_edits) = {
            let Some(document) = self.document.as_mut() else {
                return;
            };
            let peek = self.history.peek_redo();
            let rename_only = peek.is_some_and(|tx| tx.is_rename_only());
            let rename_edits = peek.map(|tx| tx.rename_edits()).unwrap_or_default();
            let redone = self.history.redo(document);
            (redone, rename_only, rename_edits)
        };
        if !redone {
            return;
        }
        if let Some(document) = self.document.as_ref() {
            self.block_edit.refresh_dirty(document);
        }
        if rename_only {
            for (before, after) in rename_edits {
                self.sync_after_block_rename(&before, &after);
            }
        } else {
            self.refresh_derived();
        }
        self.status = "Redo".into();
    }

    pub(crate) fn set_current_layer(&mut self, name: &str) {
        let changed = self
            .ensure_document()
            .is_some_and(|document| document.set_current_layer(name));
        if changed {
            self.status = format!("Current layer: {name}");
        } else {
            self.status = format!("Layer {name} cannot be current");
        }
    }

    pub(crate) fn set_selected_layer_current(&mut self) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let Some(layer) = self.selection.shared_layer(document) else {
            return;
        };
        self.set_current_layer(&layer);
    }

    fn ensure_document(&mut self) -> Option<&mut Document> {
        Some(self.document.get_or_insert_with(Document::default))
    }

    fn refresh_derived(&mut self) {
        self.rebuild_block_tree();
        let Some(document) = self.document.as_mut() else {
            self.display = Arc::new(DisplayList::default());
            self.snaps = Arc::new(SnapIndex::default());
            self.measures = Arc::new(MeasureIndex::default());
            self.measurement = None;
            self.gpu_upload = GpuUpload::Full;
            self.display_generation = self.display_generation.wrapping_add(1);
            self.selection.clear();
            return;
        };
        document.diagnostics.extents = document.compute_extents();
        self.display = Arc::new(if let Some(view) = self.block_edit.tess_view() {
            tessellate_document_for_block_edit(document, &view)
        } else {
            tessellate_document(document)
        });
        self.snaps = Arc::new(SnapIndex::build(document));
        self.measures = Arc::new(MeasureIndex::build(document));
        self.measurement = None;
        self.gpu_upload = GpuUpload::Full;
        self.display_generation = self.display_generation.wrapping_add(1);
        self.selection.retain_valid(document);
    }

    fn rebuild_block_tree(&mut self) {
        self.block_tree = self
            .document
            .as_ref()
            .map(BlockTreeIndex::build)
            .unwrap_or_default();
    }

    fn sync_after_block_rename(&mut self, from: &str, to: &str) {
        self.block_tree.rename(from, to);
        self.block_edit.on_block_renamed(from, to);
        if self
            .blocks_panel
            .selected
            .as_ref()
            .is_some_and(|name| name.eq_ignore_ascii_case(from))
        {
            self.blocks_panel.selected = Some(to.to_string());
        }
        self.blocks_panel.retarget_rename(from, to);
        self.blocks_panel.error = None;
    }

    fn commit_geometry(&mut self, geometry: Geometry) {
        let type_name = geometry.type_name();
        let space = self.block_edit.active_space();
        let to_local = self.block_edit.local_from_world();
        let geometry = match transform_geometry(&geometry, to_local) {
            Ok(geometry) => geometry,
            Err(err) => {
                self.status = err.to_string();
                return;
            }
        };
        let (edit, entity) = {
            let Some(document) = self.ensure_document() else {
                return;
            };
            let entity = document.new_entity(geometry);
            let Some(entity) = document.add_entity_to(&space, entity) else {
                self.status = "Cannot add geometry to this block".into();
                return;
            };
            if space.is_model() {
                document
                    .diagnostics
                    .bump_entity(entity.geometry.type_name());
                document.diagnostics.object_count =
                    document.diagnostics.object_count.saturating_add(1);
            }
            document.expand_extents_for(&entity);
            let index = document.entity_index_in(&space, entity.id).unwrap_or(0);
            (
                Edit::InsertEntity {
                    space: space.clone(),
                    index,
                    entity: entity.clone(),
                },
                entity,
            )
        };
        self.history.record(edit);
        self.block_edit
            .refresh_dirty(self.document.as_ref().unwrap());
        if self.block_edit.is_active() {
            self.refresh_derived();
            self.status = format!("{type_name} added");
            return;
        }
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let display = Arc::make_mut(&mut self.display);
        let appended = display.append_entity(document, &entity);
        Arc::make_mut(&mut self.snaps).append_entity(document, &entity);
        Arc::make_mut(&mut self.measures).append_entity(document, &entity);
        self.measurement = None;
        self.gpu_upload = match appended {
            Some(range) => GpuUpload::Append {
                line_start: range.line_start,
                fill_start: range.fill_start,
            },
            None => GpuUpload::Full,
        };
        self.display_generation = self.display_generation.wrapping_add(1);
        self.status = format!("{type_name} added");
    }

    fn remember_completed(&mut self, kind: CommandKind) {
        if !kind.is_idle() {
            self.last_command = Some(kind);
        }
    }

    fn validate_modify_selection(&self, ids: &[cad_core::EntityId]) -> Result<(), TransformError> {
        let document = self
            .document
            .as_ref()
            .ok_or(TransformError::Invalid("No drawing is open"))?;
        let entities: Vec<_> = ids
            .iter()
            .filter_map(|id| document.entity_by_id(*id))
            .collect();
        if entities.is_empty() {
            return Err(TransformError::Invalid("Select objects first"));
        }
        validate_entities(entities)
    }

    fn set_modify_reference_radius(&mut self) {
        let Some(base) = self.command.base_point().or(self.drafting.current_point) else {
            return;
        };
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let entities: Vec<_> = self
            .command
            .modify_targets()
            .iter()
            .filter_map(|id| document.entity_by_id(*id))
            .collect();
        let radius = reference_radius(entities, base);
        self.command.set_scale_reference(radius);
    }

    fn confirm_modify_selection(&mut self) {
        if !self.command.is_selecting_objects() {
            return;
        }
        let ids = self.editable_selection_ids();
        if ids.is_empty() {
            self.status = if self.selection.is_empty() {
                "Select objects first"
            } else {
                "Cannot modify reference objects. Use Add to Block, or select objects in the active block."
            }
            .into();
            return;
        }
        if self.command.kind() == CommandKind::Erase {
            self.erase_selected();
            return;
        }
        if let Err(err) = self.validate_modify_selection(&ids) {
            self.cancel_command();
            self.status = err.to_string();
            return;
        }
        let radius = {
            let document = self.document.as_ref();
            document
                .map(|document| {
                    let entities: Vec<_> = ids
                        .iter()
                        .filter_map(|id| document.entity_by_id(*id))
                        .collect();
                    reference_radius(entities, Point2::new(0.0, 0.0))
                })
                .unwrap_or(1.0)
        };
        self.command.confirm_modify_selection(ids, radius);
        self.sync_dynamic_layout();
        self.status = self.command.prompt().into();
    }

    fn commit_modify(&mut self, transform: EntityTransform, copies: bool) {
        let kind = self.command.kind();
        let targets = self.editable_ids(self.command.modify_targets());
        let Some(document) = self.document.as_ref() else {
            self.cancel_command();
            self.status = "No drawing is open".into();
            return;
        };
        let mut originals = Vec::new();
        for id in &targets {
            let Some((space, _)) = document.find_entity_location(*id) else {
                continue;
            };
            let Some(entity) = document.entity_by_id(*id).cloned() else {
                continue;
            };
            originals.push((space, entity));
        }
        if originals.is_empty() || originals.len() != targets.len() {
            self.cancel_command();
            self.status = "Select objects first".into();
            return;
        }
        let entities: Vec<_> = originals.iter().map(|(_, entity)| entity.clone()).collect();
        if let Err(err) = validate_entities(&entities) {
            self.cancel_command();
            self.status = err.to_string();
            return;
        }
        let world_matrix = match transform.to_matrix() {
            Ok(matrix) => matrix,
            Err(TransformError::NoOp) => {
                self.cancel_command();
                self.status = "Nothing to transform".into();
                return;
            }
            Err(err) => {
                self.cancel_command();
                self.status = err.to_string();
                return;
            }
        };
        let world_from_local = self.block_edit.world_from_local();
        let local_from_world = self.block_edit.local_from_world();
        let local_matrix = local_from_world.then(world_matrix).then(world_from_local);
        let mut transformed = Vec::new();
        for (_, entity) in &originals {
            match transform_entity_matrix(entity, local_matrix) {
                Ok(after) => transformed.push(after),
                Err(TransformError::NoOp) => {
                    self.cancel_command();
                    self.status = "Nothing to transform".into();
                    return;
                }
                Err(err) => {
                    self.cancel_command();
                    self.status = err.to_string();
                    return;
                }
            }
        }
        {
            let Some(document) = self.document.as_mut() else {
                return;
            };
            if copies {
                let mut created = Vec::new();
                for ((space, _), entity) in originals.iter().zip(transformed) {
                    let mut entity = entity;
                    entity.id = EntityId::UNASSIGNED;
                    let Some(entity) = document.add_entity_to(space, entity) else {
                        continue;
                    };
                    if space.is_model() {
                        document
                            .diagnostics
                            .bump_entity(entity.geometry.type_name());
                        document.diagnostics.object_count =
                            document.diagnostics.object_count.saturating_add(1);
                    }
                    let index = document.entity_index_in(space, entity.id).unwrap_or(0);
                    self.history.record(Edit::InsertEntity {
                        space: space.clone(),
                        index,
                        entity: entity.clone(),
                    });
                    created.push(entity.id);
                }
                self.selection.replace_all(created);
            } else {
                for ((space, before), after) in originals.iter().zip(transformed) {
                    let Some(index) = document.entity_index_in(space, before.id) else {
                        continue;
                    };
                    let _ = document.replace_entity_in(space, before.id, after.clone());
                    self.history.record(Edit::ReplaceEntity {
                        space: space.clone(),
                        index,
                        before: before.clone(),
                        after,
                    });
                }
                self.selection.replace_all(targets);
            }
        }
        if let Some(document) = self.document.as_ref() {
            self.block_edit.refresh_dirty(document);
        }
        self.remember_completed(kind);
        self.finish_active_transaction();
        self.command.finish();
        self.dynamic_input.set_layout(DynamicLayout::Hidden);
        self.drafting.clear_acquisition();
        self.refresh_derived();
        self.status = format!("{} complete", kind.label());
    }

    pub(crate) fn erase_selected(&mut self) {
        if !self.command.is_idle() && !self.command.is_erase_picking() {
            return;
        }
        let ids = self.editable_selection_ids();
        let stay = self.command.is_erase_picking();
        if ids.is_empty() {
            if !self.selection.is_empty() {
                self.status =
                    "Cannot erase reference objects. Use Add to Block, or select objects in the active block."
                        .into();
            }
            return;
        }
        self.erase_ids(&ids, stay);
    }

    fn erase_ids(&mut self, ids: &[cad_core::EntityId], stay_in_erase: bool) {
        if ids.is_empty() {
            return;
        }
        let ids = self.editable_ids(ids);
        if ids.is_empty() {
            return;
        }
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let mut removals: Vec<(EntitySpace, usize, Entity)> = ids
            .iter()
            .filter_map(|id| {
                let (space, index) = document.find_entity_location(*id)?;
                let entity = document.entity_by_id(*id).cloned()?;
                Some((space, index, entity))
            })
            .collect();
        if removals.is_empty() {
            return;
        }
        removals.sort_by_key(|(_, index, _)| std::cmp::Reverse(*index));
        self.history.commit_open();
        self.history.begin();
        {
            let Some(document) = self.document.as_mut() else {
                return;
            };
            for (space, index, entity) in removals {
                let edit = Edit::RemoveEntity {
                    space,
                    index,
                    entity,
                };
                edit.apply(document);
                self.history.record(edit);
            }
        }
        if let Some(document) = self.document.as_ref() {
            self.block_edit.refresh_dirty(document);
        }
        self.remember_completed(CommandKind::Erase);
        self.finish_active_transaction();
        self.selection.remove_all(ids.iter().copied());
        if stay_in_erase && self.command.is_erase_picking() {
            self.dynamic_input.set_layout(DynamicLayout::Hidden);
            self.drafting.clear_acquisition();
            self.refresh_derived();
            self.status = self.command.prompt().into();
            return;
        }
        self.command.finish();
        self.dynamic_input.set_layout(DynamicLayout::Hidden);
        self.drafting.clear_acquisition();
        self.selection.clear();
        self.refresh_derived();
        self.status = "Erase complete".into();
    }

    fn complete_measurement(&mut self, result: MeasurementResult) {
        let kind = self.command.kind();
        self.measurement = Some(MeasurementOverlay::final_result(result));
        self.command.finish();
        self.dynamic_input.set_layout(DynamicLayout::Hidden);
        self.drafting.clear_acquisition();
        self.remember_completed(kind);
        self.status = self.command.prompt().into();
    }

    fn try_immediate_measurement(&mut self) {
        match self.immediate_measurement() {
            Some(Ok(result)) => self.complete_measurement(result),
            Some(Err(message)) => self.status = message.into(),
            None => {}
        }
    }

    fn immediate_measurement(&self) -> Option<Result<MeasurementResult, &'static str>> {
        let document = self.document.as_ref()?;
        let ids = self.selection.ids();
        match self.command.kind() {
            CommandKind::Angle if ids.len() == 2 => {
                let a = document.entity_by_id(ids[0]).and_then(line_segment)?;
                let b = document.entity_by_id(ids[1]).and_then(line_segment)?;
                Some(
                    cad_core::AngleMeasurement::from_segments(a.0, a.1, b.0, b.1)
                        .map(MeasurementResult::Angle)
                        .ok_or("Directions are coincident or zero-length"),
                )
            }
            CommandKind::Radius if ids.len() == 1 => {
                radius_from_entity(document.entity_by_id(ids[0])).map(|result| {
                    result
                        .map(MeasurementResult::Radius)
                        .map_err(cad_core::MeasureError::message)
                })
            }
            CommandKind::Area if ids.len() == 1 => area_from_entity(document.entity_by_id(ids[0]))
                .map(|result| {
                    result
                        .map(MeasurementResult::Area)
                        .map_err(cad_core::MeasureError::message)
                }),
            _ => None,
        }
    }

    fn accept_measure_click(&mut self, point: Point2) {
        let aperture = measurement::world_aperture(&self.camera, self.viewport_height);
        let kind = self.command.kind();
        let angle_three = matches!(
            self.command,
            CommandState::Angle(AngleState::ThreePoint { .. })
        );
        let area_points = matches!(self.command, CommandState::Area(AreaState::Points { .. }));
        match kind {
            CommandKind::Distance => self.accept_command_point(point),
            CommandKind::Angle if angle_three => self.accept_command_point(point),
            CommandKind::Angle => {
                let segment = self
                    .measures
                    .pick(point, aperture, Some(MeasureRole::Straight))
                    .and_then(cad_core::straight_of);
                if let Some((start, end)) = segment {
                    let output = self.command.accept_straight_segment(start, end);
                    self.apply_command_output(output);
                } else {
                    let output = self.command.begin_three_point_angle(point);
                    self.apply_command_output(output);
                }
            }
            CommandKind::Radius => {
                let measured = self
                    .measures
                    .pick(point, aperture, Some(MeasureRole::Curve))
                    .map(|hit| cad_core::radius_from_primitive(hit, point));
                match measured {
                    Some(Ok(radius)) => {
                        self.complete_measurement(MeasurementResult::Radius(radius))
                    }
                    Some(Err(err)) => self.status = err.message().into(),
                    None => self.status = cad_core::MeasureError::Unsupported.message().into(),
                }
            }
            CommandKind::Area if area_points => self.accept_command_point(point),
            CommandKind::Area => {
                let measured = self
                    .measures
                    .pick(point, aperture, Some(MeasureRole::Closed))
                    .map(cad_core::area_from_primitive);
                match measured {
                    Some(Ok(area)) => self.complete_measurement(MeasurementResult::Area(area)),
                    Some(Err(err)) => self.status = err.message().into(),
                    None => {
                        let output = self.command.begin_area_points(point);
                        self.apply_command_output(output);
                    }
                }
            }
            _ => self.accept_command_point(point),
        }
        self.drafting.command_base_point = self.command.base_point();
    }

    fn apply_command_output(&mut self, output: CommandOutput) {
        match output {
            CommandOutput::Geometry(geometry) => {
                let stays_active = self.command.is_active();
                self.commit_geometry(geometry);
                if stays_active {
                    self.sync_dynamic_layout();
                    self.dynamic_input.reset_values();
                } else {
                    self.finish_active_transaction();
                    self.dynamic_input.set_layout(DynamicLayout::Hidden);
                    self.drafting.clear_acquisition();
                }
            }
            CommandOutput::Distance(report) => {
                self.complete_measurement(MeasurementResult::Distance(report));
            }
            CommandOutput::Measurement(result) => self.complete_measurement(result),
            CommandOutput::Rejected(message) => self.status = message.into(),
            CommandOutput::Modify { transform, copies } => self.commit_modify(transform, copies),
            CommandOutput::None => {
                self.sync_dynamic_layout();
                self.dynamic_input.reset_values();
                self.status = self.command.prompt().into();
            }
        }
    }

    fn toggle_ortho(&mut self) {
        self.drafting.preferences.ortho_enabled = !self.drafting.preferences.ortho_enabled;
        self.settings.drafting = self.drafting.preferences;
        if self.show_settings {
            self.settings_draft.drafting = self.drafting.preferences;
        }
    }

    fn toggle_osnap(&mut self) {
        self.drafting.preferences.osnap_enabled = !self.drafting.preferences.osnap_enabled;
        self.settings.drafting = self.drafting.preferences;
        if self.show_settings {
            self.settings_draft.drafting = self.drafting.preferences;
        }
        if !self.drafting.preferences.osnap_enabled {
            self.drafting.acquired_snap = None;
        }
    }

    fn editable_ids(&self, ids: &[EntityId]) -> Vec<EntityId> {
        let active = self.block_edit.active_space();
        let Some(document) = self.document.as_ref() else {
            return Vec::new();
        };
        ids.iter()
            .copied()
            .filter(|id| {
                document
                    .find_entity_location(*id)
                    .is_some_and(|(space, _)| space == active)
            })
            .collect()
    }

    fn editable_selection_ids(&self) -> Vec<EntityId> {
        self.editable_ids(self.selection.ids())
    }

    fn reference_selection_ids(&self) -> Vec<EntityId> {
        let active = self.block_edit.active_space();
        let Some(document) = self.document.as_ref() else {
            return Vec::new();
        };
        self.selection
            .ids()
            .iter()
            .copied()
            .filter(|id| {
                document
                    .find_entity_location(*id)
                    .is_some_and(|(space, _)| space != active)
            })
            .collect()
    }

    fn can_create_block(&self) -> bool {
        if self.command.is_active() || self.selection.is_empty() {
            return false;
        }
        let active = self.block_edit.active_space();
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        self.selection.ids().iter().all(|id| {
            document
                .find_entity_location(*id)
                .is_some_and(|(space, _)| space == active)
        })
    }

    fn can_edit_selected_block(&self) -> bool {
        if self.command.is_active() || self.selection.len() != 1 {
            return false;
        }
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        document
            .entity_by_id(self.selection.ids()[0])
            .is_some_and(insert_is_editable)
    }

    fn can_add_to_block(&self) -> bool {
        self.block_edit.is_active()
            && !self.command.is_active()
            && !self.reference_selection_ids().is_empty()
    }

    fn can_remove_from_block(&self) -> bool {
        self.block_edit.is_active()
            && !self.command.is_active()
            && !self.editable_selection_ids().is_empty()
    }

    fn can_make_unique(&self) -> bool {
        self.can_edit_selected_block()
    }

    fn open_create_block_dialog(&mut self) {
        if !self.can_create_block() {
            self.status = "Select objects in the current space to create a block".into();
            return;
        }
        let space = self.block_edit.active_space();
        let ids = self.selection.ids().to_vec();
        let world_from_local = self.block_edit.world_from_local();
        let base = self
            .document
            .as_ref()
            .and_then(|document| document.entities_extents(&space, &ids))
            .map(|extents| world_from_local.apply(extents.center()))
            .unwrap_or(Point2::new(0.0, 0.0));
        self.block_edit.ui = BlockUi::Create(CreateBlockDialog {
            name: String::new(),
            base_x: format!("{:.4}", base.x),
            base_y: format!("{:.4}", base.y),
            replace: true,
            ids,
            space,
            error: None,
        });
    }

    fn commit_create_block(&mut self, mut dialog: CreateBlockDialog) {
        let Ok(x) = dialog.base_x.trim().parse::<f64>() else {
            dialog.error = Some("Base point X is not a number".into());
            self.block_edit.ui = BlockUi::Create(dialog);
            return;
        };
        let Ok(y) = dialog.base_y.trim().parse::<f64>() else {
            dialog.error = Some("Base point Y is not a number".into());
            self.block_edit.ui = BlockUi::Create(dialog);
            return;
        };
        let world = Point2::new(x, y);
        let local = self.block_edit.local_from_world().apply(world);
        self.history.begin();
        let created = {
            let Some(document) = self.document.as_mut() else {
                return;
            };
            create_block_from_entities(
                document,
                &dialog.space,
                &dialog.ids,
                &dialog.name,
                local,
                dialog.replace,
            )
        };
        match created {
            Ok(result) => {
                let name = result.name.clone();
                for (space, index, entity) in result.removed {
                    self.history.record(Edit::RemoveEntity {
                        space,
                        index,
                        entity,
                    });
                }
                self.history.record(Edit::ReplaceBlockDefinition {
                    name: result.name,
                    before: None,
                    after: Some(result.definition),
                });
                let insert_id = result.insert.as_ref().map(|(_, _, entity)| entity.id);
                if let Some((space, index, entity)) = result.insert {
                    self.history.record(Edit::InsertEntity {
                        space,
                        index,
                        entity,
                    });
                }
                self.history.commit_open();
                self.block_edit.ui = BlockUi::None;
                if let Some(id) = insert_id {
                    self.selection.replace(id);
                } else {
                    self.selection.clear();
                }
                if let Some(document) = self.document.as_ref() {
                    self.block_edit.refresh_dirty(document);
                }
                self.refresh_derived();
                self.status = format!("Block {name} created");
            }
            Err(err) => {
                self.history.commit_open();
                dialog.error = Some(err.to_string());
                self.block_edit.ui = BlockUi::Create(dialog);
            }
        }
    }

    fn enter_block_edit(&mut self, instance_id: EntityId) {
        let entered = {
            let Some(document) = self.document.as_ref() else {
                return;
            };
            match self.block_edit.enter(document, &self.history, instance_id) {
                Ok(()) => {
                    let name = self
                        .block_edit
                        .current()
                        .map(|frame| frame.block_name.clone())
                        .unwrap_or_default();
                    let refs = cad_core::count_block_references(document, &name);
                    Ok(format!("Editing: {name}    •    {refs} references"))
                }
                Err(err) => Err(err),
            }
        };
        match entered {
            Ok(status) => {
                self.selection.clear();
                self.command.cancel();
                self.refresh_derived();
                self.status = status;
            }
            Err(err) => self.status = err,
        }
    }

    fn try_enter_block_from_hit(&mut self, hit: Option<EntityId>) -> bool {
        let Some(id) = hit else {
            return false;
        };
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let Some(entity) = document.entity_by_id(id) else {
            return false;
        };
        if !insert_is_editable(entity) {
            return false;
        }
        if self.block_edit.current_is_dirty() {
            self.block_edit.ui = BlockUi::LeaveDirty {
                name: self
                    .block_edit
                    .current()
                    .map(|frame| frame.block_name.clone())
                    .unwrap_or_default(),
                intent: LeaveIntent::EnterNested { instance_id: id },
            };
            return true;
        }
        self.enter_block_edit(id);
        true
    }

    fn save_active_block(&mut self) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        match self.block_edit.save_current(document, &mut self.history) {
            Ok(()) => {
                self.status = format!(
                    "Block {} saved",
                    self.block_edit
                        .current()
                        .map(|frame| frame.block_name.as_str())
                        .unwrap_or("block")
                );
            }
            Err(err) => self.status = err,
        }
    }

    fn close_block_level(&mut self, discard: bool) {
        if discard {
            if let Some(document) = self.document.as_mut() {
                self.block_edit.discard_current(document, &mut self.history);
            }
        } else if self.block_edit.current_is_dirty() {
            self.save_active_block();
        }
        self.block_edit.pop();
        self.selection.clear();
        self.refresh_derived();
        if let Some(frame) = self.block_edit.current() {
            let refs = self
                .document
                .as_ref()
                .map(|document| cad_core::count_block_references(document, &frame.block_name))
                .unwrap_or(0);
            self.status = format!("Editing: {}    •    {refs} references", frame.block_name);
        } else {
            self.status = "Block edit closed".into();
        }
    }

    fn leave_to_breadcrumb(&mut self, index: usize) {
        while self.block_edit.stack.len() > index {
            if self.block_edit.current_is_dirty() {
                self.block_edit.ui = BlockUi::LeaveDirty {
                    name: self
                        .block_edit
                        .current()
                        .map(|frame| frame.block_name.clone())
                        .unwrap_or_default(),
                    intent: LeaveIntent::CloseTo(index),
                };
                return;
            }
            self.block_edit.pop();
        }
        self.selection.clear();
        self.refresh_derived();
    }

    fn request_leave_block(&mut self, intent: LeaveIntent) {
        if self.block_edit.request_leave(intent.clone()) {
            self.apply_leave_intent(intent);
        }
    }

    fn apply_leave_intent(&mut self, intent: LeaveIntent) {
        match intent {
            LeaveIntent::CloseOne => self.close_block_level(false),
            LeaveIntent::CloseTo(index) => self.leave_to_breadcrumb(index),
            LeaveIntent::EnterNested { instance_id } => self.enter_block_edit(instance_id),
            LeaveIntent::OpenDrawing => {
                if let Some(path) = self.pending_open.take() {
                    self.start_load(path);
                } else {
                    self.open_dialog();
                }
            }
            LeaveIntent::Quit => {
                if self.close_block_edit_for_document_action(LeaveIntent::Quit) {
                    self.pending_discard = Some(PendingDiscard::Quit);
                }
            }
        }
    }

    fn add_selected_to_block(&mut self) {
        let ids = self.reference_selection_ids();
        if ids.is_empty() {
            self.status = "Select reference objects to add".into();
            return;
        }
        let Some(frame) = self.block_edit.current() else {
            return;
        };
        let name = frame.block_name.clone();
        let references = self
            .document
            .as_ref()
            .map(|document| cad_core::count_block_references(document, &name))
            .unwrap_or(0);
        if references > 1 {
            self.block_edit.ui = BlockUi::AddConfirm {
                name,
                references,
                ids,
            };
            return;
        }
        self.commit_add_to_block(&ids);
    }

    fn commit_add_to_block(&mut self, ids: &[EntityId]) {
        let dest = self.block_edit.active_space();
        let dest_world = self.block_edit.world_from_local();
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let mut plans = Vec::new();
        for id in ids {
            let Some((source, _)) = document.find_entity_location(*id) else {
                continue;
            };
            let source_world = self.block_edit.space_world_from_local(document, &source);
            match membership_matrix(source_world, dest_world) {
                Ok(matrix) => plans.push((*id, matrix)),
                Err(err) => {
                    self.status = err.to_string();
                    return;
                }
            }
        }
        self.history.begin();
        let mut error = None;
        {
            let Some(document) = self.document.as_mut() else {
                return;
            };
            for (id, matrix) in plans {
                match transfer_entity(document, id, &dest, matrix) {
                    Ok(result) => {
                        self.history.record(Edit::RemoveEntity {
                            space: result.source,
                            index: result.source_index,
                            entity: result.before,
                        });
                        self.history.record(Edit::InsertEntity {
                            space: result.dest,
                            index: result.dest_index,
                            entity: result.after,
                        });
                    }
                    Err(err) => {
                        error = Some(err);
                        break;
                    }
                }
            }
        }
        self.history.commit_open();
        if let Some(err) = error {
            self.status = err.to_string();
            self.refresh_derived();
            return;
        }
        if let Some(document) = self.document.as_ref() {
            self.block_edit.refresh_dirty(document);
        }
        self.refresh_derived();
        self.status = "Added to block".into();
    }

    fn remove_selected_from_block(&mut self) {
        let ids = self.editable_selection_ids();
        if ids.is_empty() {
            self.status = "Select objects in the active block to remove".into();
            return;
        }
        let dest = self
            .block_edit
            .current()
            .map(|frame| frame.parent_space.clone())
            .unwrap_or(EntitySpace::ModelSpace);
        let dest_world = self
            .document
            .as_ref()
            .map(|document| self.block_edit.space_world_from_local(document, &dest))
            .unwrap_or_else(Transform2::identity);
        let source_world = self.block_edit.world_from_local();
        let matrix = match membership_matrix(source_world, dest_world) {
            Ok(matrix) => matrix,
            Err(err) => {
                self.status = err.to_string();
                return;
            }
        };
        self.history.begin();
        let mut error = None;
        {
            let Some(document) = self.document.as_mut() else {
                return;
            };
            for id in ids {
                match transfer_entity(document, id, &dest, matrix) {
                    Ok(result) => {
                        self.history.record(Edit::RemoveEntity {
                            space: result.source,
                            index: result.source_index,
                            entity: result.before,
                        });
                        self.history.record(Edit::InsertEntity {
                            space: result.dest,
                            index: result.dest_index,
                            entity: result.after,
                        });
                    }
                    Err(err) => {
                        error = Some(err);
                        break;
                    }
                }
            }
        }
        self.history.commit_open();
        if let Some(err) = error {
            self.status = err.to_string();
            self.refresh_derived();
            return;
        }
        if let Some(document) = self.document.as_ref() {
            self.block_edit.refresh_dirty(document);
        }
        self.refresh_derived();
        self.status = "Removed from block".into();
    }

    fn make_selected_unique(&mut self) {
        if self.selection.len() != 1 {
            self.status = "Select one block reference".into();
            return;
        }
        let id = self.selection.ids()[0];
        self.history.begin();
        let made = {
            let Some(document) = self.document.as_mut() else {
                return;
            };
            make_unique_block(document, id)
        };
        match made {
            Ok(result) => {
                let name = result.new_name.clone();
                self.history.record(Edit::ReplaceBlockDefinition {
                    name: result.new_name,
                    before: None,
                    after: Some(result.definition),
                });
                self.history.record(Edit::ReplaceEntity {
                    space: result.insert_space,
                    index: result.insert_index,
                    before: result.insert_before,
                    after: result.insert_after,
                });
                self.history.commit_open();
                self.refresh_derived();
                self.status = format!("Block {name} created");
            }
            Err(err) => {
                self.history.commit_open();
                self.status = err.to_string();
            }
        }
    }

    pub(crate) fn insert_named_block(&mut self, name: &str) {
        let space = self.block_edit.active_space();
        let local = self.block_edit.local_from_world().apply(self.camera.center);
        if let EntitySpace::Block(dest) = &space {
            let Some(document) = self.document.as_ref() else {
                return;
            };
            if would_create_block_cycle(document, dest, name) {
                self.blocks_panel.error = Some("Cannot create a circular block reference".into());
                self.status = "Cannot create a circular block reference".into();
                return;
            }
        }
        self.history.begin();
        let inserted = {
            let Some(document) = self.ensure_document() else {
                return;
            };
            if document.block_by_name(name).is_none() {
                None
            } else {
                let entity = document.new_entity(identity_insert(
                    name.to_string(),
                    Point3::from_xy(local.x, local.y),
                ));
                document.add_entity_to(&space, entity)
            }
        };
        let Some(entity) = inserted else {
            self.history.commit_open();
            self.blocks_panel.error = Some("Block definition was not found".into());
            return;
        };
        let index = self
            .document
            .as_ref()
            .and_then(|document| document.entity_index_in(&space, entity.id))
            .unwrap_or(0);
        self.history.record(Edit::InsertEntity {
            space,
            index,
            entity: entity.clone(),
        });
        self.history.commit_open();
        self.selection.replace(entity.id);
        if let Some(document) = self.document.as_ref() {
            self.block_edit.refresh_dirty(document);
        }
        self.refresh_derived();
        self.blocks_panel.error = None;
        self.status = format!("Inserted {name}");
    }

    pub(crate) fn edit_named_block(&mut self, name: &str) {
        let selected = self.selection.ids().first().copied();
        let ids = self
            .document
            .as_ref()
            .map(|document| insert_instance_ids(document, name))
            .unwrap_or_default();
        let id = selected
            .filter(|id| ids.contains(id))
            .or_else(|| ids.first().copied());
        let Some(id) = id else {
            self.blocks_panel.error = Some("Insert the block first to edit it in place.".into());
            self.status = "Insert the block first to edit it in place.".into();
            return;
        };
        self.blocks_panel.error = None;
        self.try_enter_block_from_hit(Some(id));
    }

    pub(crate) fn edit_named_block_from_tree(&mut self, name: &str) {
        let ids = self
            .document
            .as_ref()
            .map(|document| insert_instance_ids(document, name))
            .unwrap_or_default();
        match ids.as_slice() {
            [] => {
                self.blocks_panel.error =
                    Some("Insert the block first to edit it in place.".into());
                self.status = "Insert the block first to edit it in place.".into();
            }
            [id] => {
                self.blocks_panel.error = None;
                self.try_enter_block_from_hit(Some(*id));
            }
            ids => {
                let message = format!("{} references; double-click an instance", ids.len());
                self.blocks_panel.error = Some(message.clone());
                self.status = message;
            }
        }
    }

    pub(crate) fn make_named_block_unique(&mut self, name: &str) {
        let ids = self
            .document
            .as_ref()
            .map(|document| insert_instance_ids(document, name))
            .unwrap_or_default();
        match ids.as_slice() {
            [] => {
                self.status = "Insert the block first to make it unique.".into();
            }
            [id] => {
                self.selection.replace(*id);
                self.make_selected_unique();
            }
            ids => {
                self.status = format!(
                    "{} references; select one instance to make unique",
                    ids.len()
                );
            }
        }
    }

    pub(crate) fn duplicate_named_block(&mut self, name: &str) {
        self.history.begin();
        let duplicated = {
            let Some(document) = self.document.as_mut() else {
                return;
            };
            duplicate_block_definition(document, name)
        };
        match duplicated {
            Ok(definition) => {
                let new_name = definition.name.clone();
                self.history.record(Edit::ReplaceBlockDefinition {
                    name: new_name.clone(),
                    before: None,
                    after: Some(definition),
                });
                self.history.commit_open();
                self.blocks_panel.selected = Some(new_name.clone());
                self.blocks_panel.error = None;
                self.refresh_derived();
                self.status = format!("Duplicated as {new_name}");
            }
            Err(err) => {
                self.history.commit_open();
                self.blocks_panel.error = Some(err.to_string());
                self.status = err.to_string();
            }
        }
    }

    pub(crate) fn rename_named_block(&mut self, from: &str, to: &str) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        match validate_block_rename(document, from, to) {
            Ok(None) => {
                self.blocks_panel.error = None;
                return;
            }
            Ok(Some(_)) => {}
            Err(err) => {
                self.blocks_panel.error = Some(err.to_string());
                self.status = err.to_string();
                return;
            }
        }
        let before = document.block_key(from).unwrap_or_else(|| from.to_string());
        self.history.begin();
        let renamed = {
            let Some(document) = self.document.as_mut() else {
                return;
            };
            document.rename_block(&before, to)
        };
        match renamed {
            Ok(()) => {
                let after = self
                    .document
                    .as_ref()
                    .and_then(|document| document.block_key(to))
                    .unwrap_or_else(|| to.trim().to_string());
                self.history.record(Edit::RenameBlock {
                    before: before.clone(),
                    after: after.clone(),
                });
                self.history.commit_open();
                self.sync_after_block_rename(&before, &after);
                self.status = format!("Renamed to {after}");
            }
            Err(err) => {
                self.history.commit_open();
                self.blocks_panel.error = Some(err.to_string());
                self.status = err.to_string();
            }
        }
    }

    pub(crate) fn purge_unused_blocks(&mut self) {
        let skip: Vec<String> = self
            .block_edit
            .stack
            .iter()
            .map(|frame| frame.block_name.clone())
            .collect();
        self.history.begin();
        let removed = {
            let Some(document) = self.document.as_mut() else {
                return;
            };
            let mut removed = purge_unused_user_blocks(document);
            removed.retain(|definition| {
                let keep = skip
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(&definition.name));
                if keep {
                    document.replace_block_definition(definition.clone());
                    false
                } else {
                    true
                }
            });
            removed
        };
        if removed.is_empty() {
            self.history.commit_open();
            self.status = "No unused blocks to purge".into();
            return;
        }
        let count = removed.len();
        for definition in removed {
            self.history.record(Edit::ReplaceBlockDefinition {
                name: definition.name.clone(),
                before: Some(definition),
                after: None,
            });
        }
        self.history.commit_open();
        self.blocks_panel.error = None;
        self.refresh_derived();
        self.status = format!("Purged {count} unused block(s)");
    }

    fn apply_toolbar_action(&mut self, action: ToolbarAction) {
        match action {
            ToolbarAction::None => {}
            ToolbarAction::AddToBlock => self.add_selected_to_block(),
            ToolbarAction::RemoveFromBlock => self.remove_selected_from_block(),
            ToolbarAction::Save => self.save_active_block(),
            ToolbarAction::SaveAndClose => {
                if self.block_edit.current_is_dirty() {
                    self.save_active_block();
                }
                self.close_block_level(false);
            }
            ToolbarAction::DiscardAndClose => {
                if self.block_edit.current_is_dirty() {
                    self.close_block_level(true);
                } else {
                    self.close_block_level(false);
                }
            }
            ToolbarAction::Breadcrumb(index) => self.leave_to_breadcrumb(index),
        }
    }

    fn show_block_ui(&mut self, ctx: &egui::Context) {
        match std::mem::replace(&mut self.block_edit.ui, BlockUi::None) {
            BlockUi::None => {}
            BlockUi::Create(mut dialog) => match block_edit::show_create_dialog(ctx, &mut dialog) {
                CreateDialogResult::Open => self.block_edit.ui = BlockUi::Create(dialog),
                CreateDialogResult::Cancel => {}
                CreateDialogResult::PickPoint => {
                    self.status = "Specify base point".into();
                    self.block_edit.ui = BlockUi::PickBase(dialog);
                }
                CreateDialogResult::Create => self.commit_create_block(dialog),
            },
            BlockUi::PickBase(dialog) => {
                self.block_edit.ui = BlockUi::PickBase(dialog);
            }
            BlockUi::LeaveDirty { name, intent } => {
                match block_edit::show_leave_dialog(ctx, &name) {
                    Some(LeaveChoice::Save) => {
                        self.save_active_block();
                        self.apply_leave_intent(intent);
                    }
                    Some(LeaveChoice::Discard) => {
                        if let Some(document) = self.document.as_mut() {
                            self.block_edit.discard_current(document, &mut self.history);
                        }
                        self.apply_leave_intent(intent);
                    }
                    Some(LeaveChoice::Cancel) | None => {
                        self.block_edit.ui = BlockUi::LeaveDirty { name, intent };
                    }
                }
            }
            BlockUi::AddConfirm {
                name,
                references,
                ids,
            } => match block_edit::show_add_confirm_dialog(ctx, &name, references) {
                Some(true) => self.commit_add_to_block(&ids),
                Some(false) => {}
                None => {
                    self.block_edit.ui = BlockUi::AddConfirm {
                        name,
                        references,
                        ids,
                    };
                }
            },
            BlockUi::SaveDrawing => match block_edit::show_save_drawing_dialog(ctx) {
                Some(true) => {
                    self.save_active_block();
                    let _ = self.save_drawing();
                }
                Some(false) => {}
                None => self.block_edit.ui = BlockUi::SaveDrawing,
            },
        }
    }

    pub(crate) fn show_viewport(&mut self, ui: &mut Ui) {
        let toolbar_action = if self.block_edit.is_active() {
            let can_add = self.can_add_to_block();
            let can_remove = self.can_remove_from_block();
            self.document
                .as_ref()
                .map(|document| {
                    block_edit::show_toolbar(ui, &self.block_edit, document, can_add, can_remove)
                })
                .unwrap_or(ToolbarAction::None)
        } else {
            ToolbarAction::None
        };
        if toolbar_action != ToolbarAction::None {
            self.apply_toolbar_action(toolbar_action);
        }
        let rect = ui.available_rect_before_wrap();
        self.viewport_height = rect.height() as f64;
        self.poll_load(rect);
        let units = self
            .document
            .as_ref()
            .map(|document| document.units)
            .unwrap_or_default();
        let card = self
            .measurement
            .as_ref()
            .filter(|overlay| !overlay.live)
            .cloned();
        if let Some(overlay) = card {
            let (action, hovered) = measurement::show_card(ui.ctx(), rect, &overlay, units);
            self.measure_card_hovered = hovered;
            match action {
                CardAction::Close => self.measurement = None,
                CardAction::Copy => self.status = "Copied measurement".into(),
                CardAction::None => {}
            }
        } else if self.measurement.is_none() {
            self.measure_card_hovered = false;
        }
        ui.advance_cursor_after_rect(rect);
        let response = ui.interact(
            rect,
            ui.id().with("cad-viewport"),
            egui::Sense::click_and_drag(),
        );
        if self.capture.is_none() {
            handle_viewport_input(self, ui, &response, rect);
        }

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(10, 14, 12));
        if self.document.is_none() && self.loading_path.is_none() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "File → Open   or   pass a DWG on the command line",
                egui::FontId::proportional(16.0),
                egui::Color32::from_rgb(140, 160, 140),
            );
        }

        let aspect = (rect.width() as f64 / rect.height().max(1.0) as f64).max(1e-6);
        let selection_overlay = self.display.overlay_batches(self.selection.ids());
        let mut preview_overlay = OverlayBatches::default();
        let mut preview_color = RgbColor::WINDOW.to_gpu();
        let mut preview_model = Transform2::identity_mat4();
        let mut box_rect = None;
        if let Some(drag) = &self.box_select {
            let colors = if self.show_settings {
                &self.settings_draft.display
            } else {
                &self.settings.display
            };
            let start = cad_core::Point2::new(drag.start.x as f64, drag.start.y as f64);
            let current = cad_core::Point2::new(drag.current.x as f64, drag.current.y as f64);
            let color = match SelectBoxMode::from_screen_drag(start, current) {
                SelectBoxMode::Window => colors.window_selection,
                SelectBoxMode::Crossing => colors.crossing_selection,
            };
            preview_overlay = self.display.overlay_batches(&drag.candidates);
            preview_color = color.to_gpu();
            box_rect = Some((drag.start, drag.current, color));
        } else if let Some(world) = self.command.preview_transform(
            self.drafting.current_point,
            self.dynamic_input.typed_angle_deg(),
            self.dynamic_input.typed_factor(),
        ) {
            if let Ok(matrix) = world.to_matrix() {
                preview_overlay = self.display.overlay_batches(self.command.modify_targets());
                preview_color = [0.55, 0.95, 0.85, 1.0];
                preview_model = matrix.to_local_origin(self.display.origin).to_mat4();
            }
        }
        painter.add(egui_wgpu::Callback::new_paint_callback(
            rect,
            CadFrame {
                camera: self.camera,
                origin: self.display.origin,
                generation: self.display_generation,
                upload: self.gpu_upload,
                display: Arc::clone(&self.display),
                aspect,
                selection: selection_overlay,
                selection_color: SELECTION_OVERLAY_COLOR,
                preview: preview_overlay,
                preview_color,
                preview_model,
            },
        ));
        if let Some((start, current, color)) = box_rect {
            workspace::paint_box_select_rect(
                &painter,
                start,
                current,
                color.to_color32(),
                color.to_fill(),
            );
        }
        if let Some((start, current)) = self.plot_window_screen_rect(rect) {
            workspace::paint_box_select_rect(
                &painter,
                start,
                current,
                egui::Color32::from_rgb(80, 180, 220),
                egui::Color32::from_rgba_unmultiplied(80, 180, 220, 40),
            );
        }
        let start = self.command.start_vertex();
        let start_marker = start.filter(|start| {
            self.drafting.preferences.osnap_enabled
                && self.drafting.preferences.running_snaps.endpoint
                && !self
                    .command
                    .base_point()
                    .is_some_and(|base| start.distance(base) <= GEOM_TOLERANCE)
        });
        let acquired_snap = self
            .command
            .requests_point()
            .then_some(self.drafting.acquired_snap)
            .flatten();
        let close_hint = self.command.can_close()
            && start.is_some_and(|start| {
                acquired_snap.is_some_and(|snap| snap.point.distance(start) <= GEOM_TOLERANCE)
            });
        crate::drafting::paint_overlay(
            &painter,
            rect,
            self.camera,
            self.command.preview(self.drafting.current_point),
            acquired_snap,
            start_marker,
            close_hint,
        );
        if let Some([start, end]) = self
            .command
            .preview_mirror_axis(self.drafting.current_point)
        {
            crate::drafting::paint_world_axis(&painter, rect, self.camera, start, end);
        }
        let units = self
            .document
            .as_ref()
            .map(|document| document.units)
            .unwrap_or_default();
        if let Some(result) = self.command.live_measurement(self.drafting.current_point) {
            let overlay = MeasurementOverlay::live(result);
            measurement::paint(&painter, rect, self.camera, &overlay, units);
            if let Some(cursor) = self.last_pointer.filter(|pos| rect.contains(*pos)) {
                let text = overlay.result.format(units);
                measurement::live_cursor_label(&painter, rect, cursor, &text.primary);
            }
        } else if let Some(overlay) = &self.measurement {
            measurement::paint(&painter, rect, self.camera, overlay, units);
        }
        if let Some(cursor) = self.last_pointer.filter(|pos| rect.contains(*pos)) {
            let mut live =
                LiveValues::from_points(self.command.base_point(), self.drafting.current_point);
            if let Some(EntityTransform::UniformScale { factor, .. }) =
                self.command.preview_transform(
                    self.drafting.current_point,
                    self.dynamic_input.typed_angle_deg(),
                    self.dynamic_input.typed_factor(),
                )
            {
                live = live.with_factor(factor);
            }
            self.dynamic_input.paint(&painter, rect, cursor, live);
        }
    }

    fn zoom_extents(&mut self, width: f64, height: f64) {
        if let Some(doc) = &self.document {
            if let Some(extents) = doc.diagnostics.extents {
                self.camera.zoom_extents(extents, width, height);
            }
        }
    }

    fn window_title(&self) -> String {
        match &self.document {
            Some(document) => {
                let mark = if self.is_dirty() { "*" } else { "" };
                format!("MyCad — {}{mark}", document.file_name())
            }
            None if self.is_dirty() => "MyCad — (untitled)*".into(),
            None => "MyCad".into(),
        }
    }

    fn status_file_label(&self) -> String {
        let name = self
            .document
            .as_ref()
            .map(|document| document.file_name())
            .or_else(|| self.loading_path.as_ref().map(|path| file_name(path)))
            .unwrap_or_else(|| "(no file)".into());
        if self.is_dirty() {
            format!("{name}*")
        } else {
            name
        }
    }

    fn open_context_menu(&mut self, pos: egui::Pos2, origin: Point2, size: Point2) {
        if let Some(kind) = context_menu::kind_for_state(&self.command) {
            self.context_menu = Some(ViewportMenu::new(pos, kind));
            return;
        }
        let hit = pick_entity(
            &self.display,
            &self.camera,
            Point2::new(pos.x as f64, pos.y as f64),
            origin,
            size,
        );
        if let Some(id) = hit {
            if !self.selection.contains(id) {
                self.selection.replace(id);
            }
            self.context_menu = Some(ViewportMenu::new(pos, ContextKind::Entity));
        } else {
            self.context_menu = Some(ViewportMenu::new(pos, ContextKind::Empty));
        }
    }

    fn show_viewport_menu(&mut self, ctx: &egui::Context) {
        let Some(mut menu) = self.context_menu.take() else {
            return;
        };
        let can_set_layer = self.document.as_ref().is_some_and(|document| {
            self.selection
                .shared_layer(document)
                .is_some_and(|layer| document.layer_can_be_current(&layer))
        });
        let result = context_menu::show(
            ctx,
            &mut menu,
            self.command.can_finish()
                || (self.command.is_selecting_objects() && !self.selection.is_empty()),
            self.command.can_close(),
            self.command.can_undo_last(),
            self.command.can_back(),
            can_set_layer,
            self.last_command.is_some() && !self.command.is_active(),
            self.selection.len(),
            self.last_command,
            context_menu::BlockMenuState {
                can_create: self.can_create_block(),
                can_edit: self.can_edit_selected_block(),
                can_add: self.can_add_to_block(),
                can_remove: self.can_remove_from_block(),
                can_make_unique: self.can_make_unique(),
            },
        );
        match result {
            MenuResult::StayOpen => self.context_menu = Some(menu),
            MenuResult::Dismissed => {}
            MenuResult::Action(action) => {
                let size = ctx.available_rect();
                self.apply_context_action(action, size.width() as f64, size.height() as f64);
            }
        }
    }

    fn apply_context_action(&mut self, action: ContextAction, width: f64, height: f64) {
        match action {
            ContextAction::Finish => self.finish_command(),
            ContextAction::UndoLast => self.undo_last_in_command(),
            ContextAction::Close => self.close_command(),
            ContextAction::Back => self.back_in_command(),
            ContextAction::Cancel => self.cancel_command(),
            ContextAction::Properties => {
                workspace::ensure_tab(&mut self.dock_state, WorkspaceTab::Properties);
            }
            ContextAction::SetCurrentLayerFromObject => self.set_selected_layer_current(),
            ContextAction::Deselect => self.selection.clear(),
            ContextAction::RepeatLast => {
                if let Some(kind) = self.last_command {
                    self.start_kind(kind);
                }
            }
            ContextAction::ZoomExtents => self.zoom_extents(width, height),
            ContextAction::Move => self.start_move_command(),
            ContextAction::Copy => self.start_copy_command(),
            ContextAction::Rotate => self.start_rotate_command(),
            ContextAction::Mirror => self.start_mirror_command(),
            ContextAction::Scale => self.start_scale_command(),
            ContextAction::Erase => self.start_erase_command(),
            ContextAction::CreateBlock => self.open_create_block_dialog(),
            ContextAction::EditBlock => {
                if let Some(id) = self.selection.ids().first().copied() {
                    self.enter_block_edit(id);
                }
            }
            ContextAction::AddToBlock => self.add_selected_to_block(),
            ContextAction::RemoveFromBlock => self.remove_selected_from_block(),
            ContextAction::MakeUnique => self.make_selected_unique(),
        }
    }

    fn show_pdf_plot_dialog(&mut self, ctx: &egui::Context) {
        if !matches!(self.pdf_plot, PdfPlotUi::Dialog(_)) {
            return;
        }
        let PdfPlotUi::Dialog(mut state) = std::mem::replace(&mut self.pdf_plot, PdfPlotUi::Closed)
        else {
            return;
        };
        let mut export = false;
        let mut cancel = false;
        let mut pick_window = false;
        egui::Window::new("Export PDF")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Paper");
                egui::ComboBox::from_id_salt("pdf-paper")
                    .selected_text(state.options.paper.label())
                    .show_ui(ui, |ui| {
                        for paper in PdfPaperSize::ALL {
                            ui.selectable_value(&mut state.options.paper, paper, paper.label());
                        }
                    });
                ui.add_space(6.0);
                ui.label("Orientation");
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut state.options.orientation,
                        PdfOrientation::Portrait,
                        "Portrait",
                    );
                    ui.selectable_value(
                        &mut state.options.orientation,
                        PdfOrientation::Landscape,
                        "Landscape",
                    );
                });
                ui.add_space(6.0);
                ui.label("Plot Area");
                egui::ComboBox::from_id_salt("pdf-plot-area")
                    .selected_text(state.area.label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut state.area,
                            PdfPlotAreaKind::Extents,
                            PdfPlotAreaKind::Extents.label(),
                        );
                        ui.selectable_value(
                            &mut state.area,
                            PdfPlotAreaKind::Window,
                            PdfPlotAreaKind::Window.label(),
                        );
                    });
                if state.area == PdfPlotAreaKind::Window {
                    ui.horizontal(|ui| {
                        if ui.button("Pick Window").clicked() {
                            pick_window = true;
                        }
                        if let Some(window) = state.window {
                            ui.label(format!(
                                "{:.3},{:.3} → {:.3},{:.3}",
                                window.min.x, window.min.y, window.max.x, window.max.y
                            ));
                        }
                    });
                }
                ui.add_space(6.0);
                ui.label("Scale");
                ui.label("Fit to page");
                ui.add_space(6.0);
                ui.checkbox(&mut state.options.center_plot, "Center Plot");
                ui.add_space(6.0);
                ui.label("Style");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut state.options.style, PdfPlotStyle::Color, "Color");
                    ui.selectable_value(
                        &mut state.options.style,
                        PdfPlotStyle::Monochrome,
                        "Monochrome",
                    );
                });
                ui.add_space(6.0);
                ui.label("Margins");
                ui.horizontal(|ui| {
                    for millimetres in PDF_MARGIN_MM {
                        ui.selectable_value(
                            &mut state.options.margin_mm,
                            millimetres,
                            format!("{millimetres:.0} mm"),
                        );
                    }
                });
                ui.add_space(6.0);
                ui.label("Line Thickness");
                ui.horizontal(|ui| {
                    for (weight, label) in PDF_STROKE_WEIGHTS {
                        ui.selectable_value(&mut state.options.stroke_pt, weight, label);
                    }
                });
                ui.add_space(6.0);
                ui.label("Preview");
                paint_pdf_preview(ui, &state);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let can_export =
                        state.area != PdfPlotAreaKind::Window || state.window.is_some();
                    if ui
                        .add_enabled(can_export, egui::Button::new("Export PDF"))
                        .clicked()
                    {
                        export = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
                if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
        if pick_window {
            self.status = "Specify first corner of plot window".into();
            self.pdf_plot = PdfPlotUi::PickWindow(PlotWindowPick {
                options: state.options,
                last_window: state.window,
                first: None,
            });
            return;
        }
        if cancel {
            self.pdf_plot = PdfPlotUi::Closed;
        } else if export {
            let mut options = state.options;
            options.plot_area = match state.area {
                PdfPlotAreaKind::Extents => PdfPlotArea::Extents,
                PdfPlotAreaKind::Window => match state.window {
                    Some(window) => PdfPlotArea::Window(window),
                    None => {
                        self.pdf_plot = PdfPlotUi::Dialog(state);
                        return;
                    }
                },
            };
            self.pdf_plot = PdfPlotUi::Closed;
            self.write_pdf_export(options);
        } else {
            self.pdf_plot = PdfPlotUi::Dialog(state);
        }
    }

    fn cancel_plot_window_pick(&mut self) {
        if let PdfPlotUi::PickWindow(pick) =
            std::mem::replace(&mut self.pdf_plot, PdfPlotUi::Closed)
        {
            self.status = "Export PDF".into();
            self.pdf_plot = PdfPlotUi::Dialog(PdfPlotDialogState {
                options: pick.options,
                area: PdfPlotAreaKind::Window,
                window: pick.last_window,
            });
        }
    }

    fn finish_plot_window_pick(&mut self, first: Point2, second: Point2) {
        let PdfPlotUi::PickWindow(pick) = std::mem::replace(&mut self.pdf_plot, PdfPlotUi::Closed)
        else {
            return;
        };
        let window = Extents2::from_corners(first, second);
        self.status = "Export PDF".into();
        self.pdf_plot = PdfPlotUi::Dialog(PdfPlotDialogState {
            options: pick.options,
            area: PdfPlotAreaKind::Window,
            window: Some(window),
        });
    }

    fn plot_window_screen_rect(&self, viewport: Rect) -> Option<(egui::Pos2, egui::Pos2)> {
        let PdfPlotUi::PickWindow(pick) = &self.pdf_plot else {
            return None;
        };
        let first = pick.first?;
        let current = self.cursor_world?;
        let origin = Point2::new(viewport.min.x as f64, viewport.min.y as f64);
        let size = Point2::new(viewport.width() as f64, viewport.height() as f64);
        let a = self.camera.world_to_screen(first, origin, size);
        let b = self.camera.world_to_screen(current, origin, size);
        Some((
            egui::pos2(a.x as f32, a.y as f32),
            egui::pos2(b.x as f32, b.y as f32),
        ))
    }

    fn show_lossy_save_dialog(&mut self, ctx: &egui::Context) {
        if !self.pending_lossy_save {
            return;
        }
        let count = self
            .document
            .as_ref()
            .map(|document| document.diagnostics.unsupported_total())
            .unwrap_or(0);
        let mut save_copy = false;
        let mut cancel = false;
        egui::Window::new("Unsupported content")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(lossy_save_message(count));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save a Copy").clicked() {
                        save_copy = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            self.pending_lossy_save = false;
        } else if save_copy {
            self.pending_lossy_save = false;
            if self.save_as_drawing_now() && self.pending_discard.is_some() {
                self.continue_pending_discard(ctx);
            }
        }
    }

    fn show_discard_dialog(&mut self, ctx: &egui::Context) {
        if self.pending_lossy_save {
            return;
        }
        if matches!(self.pending_discard, Some(PendingDiscard::Quit))
            && !self.is_dirty()
            && !self.block_edit.is_active()
        {
            self.continue_pending_discard(ctx);
            return;
        }
        let Some(pending) = self.pending_discard.as_ref() else {
            return;
        };
        let title = match pending {
            PendingDiscard::Quit => "Quit without saving?",
            PendingDiscard::OpenDialog | PendingDiscard::Open(_) => "Discard unsaved changes?",
        };
        let file_label = self
            .document
            .as_ref()
            .map(|document| document.file_name())
            .unwrap_or_else(|| "(untitled)".into());
        let mut save = false;
        let mut discard = false;
        let mut cancel = false;
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("Save changes to {file_label}?"));
                ui.label(title);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        save = true;
                    }
                    if ui.button("Don't Save").clicked() {
                        discard = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            self.pending_discard = None;
        } else if save {
            if self.save_drawing() {
                self.continue_pending_discard(ctx);
            }
        } else if discard {
            self.history.clear();
            self.continue_pending_discard(ctx);
        }
    }

    fn continue_pending_discard(&mut self, ctx: &egui::Context) {
        let pending = self.pending_discard.take();
        match pending {
            Some(PendingDiscard::OpenDialog) => self.open_dialog_now(),
            Some(PendingDiscard::Open(path)) => self.start_load_now(path),
            Some(PendingDiscard::Quit) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            None => {}
        }
    }
}

impl eframe::App for MyCadApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.input_consumed_escape = false;
        if ctx.input(|input| input.viewport().close_requested())
            && (self.is_dirty() || self.block_edit.is_active())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !self.close_block_edit_for_document_action(LeaveIntent::Quit) {
                // wait for save/discard
            } else {
                self.pending_discard = Some(PendingDiscard::Quit);
            }
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.window_title()));
        if self.pdf_plot.is_picking() && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.cancel_plot_window_pick();
            self.input_consumed_escape = true;
        }
        if self.capture.is_none()
            && !self.show_settings
            && self.pending_discard.is_none()
            && !self.pending_lossy_save
            && self.pdf_plot.is_closed()
        {
            let text_editing = crate::input::text_field_has_focus(ctx);
            if crate::input::erase_hotkey_allowed(
                self.command.is_active() && !self.command.is_erase_picking(),
                self.dynamic_input.is_active(),
                text_editing,
            ) && ctx.input(|input| {
                self.settings
                    .bindings
                    .key_pressed(InputAction::EraseSelection, input)
            }) {
                self.erase_selected();
            }
        }
        if self.capture.is_none()
            && !self.show_settings
            && self.pending_discard.is_none()
            && !self.pending_lossy_save
            && self.pdf_plot.is_closed()
            && !crate::input::text_field_has_focus(ctx)
        {
            let keys = ctx.input(|input| {
                let ctrl = input.modifiers.ctrl || input.modifiers.command;
                let plain = !ctrl && !input.modifiers.alt && !input.modifiers.shift;
                KeyChord {
                    f3: input.key_pressed(egui::Key::F3),
                    f8: input.key_pressed(egui::Key::F8),
                    line: plain && input.key_pressed(egui::Key::L),
                    polyline: plain && input.key_pressed(egui::Key::P),
                    circle: plain && input.key_pressed(egui::Key::C),
                    arc: plain && input.key_pressed(egui::Key::A),
                    rectangle: plain && input.key_pressed(egui::Key::R),
                    distance: plain && input.key_pressed(egui::Key::D),
                    enter: input.key_pressed(egui::Key::Enter),
                    escape: input.key_pressed(egui::Key::Escape),
                    undo: ctrl && !input.modifiers.shift && input.key_pressed(egui::Key::Z),
                    redo: ctrl
                        && (input.key_pressed(egui::Key::Y)
                            || (input.modifiers.shift && input.key_pressed(egui::Key::Z))),
                    open: ctrl && !input.modifiers.shift && input.key_pressed(egui::Key::O),
                    save: ctrl && !input.modifiers.shift && input.key_pressed(egui::Key::S),
                    save_as: ctrl && input.modifiers.shift && input.key_pressed(egui::Key::S),
                }
            });
            if keys.f3 {
                self.toggle_osnap();
            }
            if keys.f8 {
                self.toggle_ortho();
            }
            if keys.undo {
                self.undo();
            } else if keys.redo {
                self.redo();
            } else if keys.open {
                self.open_dialog();
            } else if keys.save {
                let _ = self.save_drawing();
            } else if keys.save_as {
                let _ = self.save_as_drawing();
            } else if keys.escape && self.context_menu.take().is_some() {
                self.input_consumed_escape = true;
            } else if keys.escape && self.blocks_panel.is_renaming() {
                self.blocks_panel.cancel_rename();
                self.input_consumed_escape = true;
            } else if keys.escape && matches!(self.block_edit.ui, BlockUi::PickBase(_)) {
                if let BlockUi::PickBase(dialog) =
                    std::mem::replace(&mut self.block_edit.ui, BlockUi::None)
                {
                    self.block_edit.ui = BlockUi::Create(dialog);
                }
                self.input_consumed_escape = true;
            } else if keys.escape && self.command.is_active() {
                self.cancel_command();
                self.input_consumed_escape = true;
            } else if keys.escape && self.measurement.take().is_some() {
                self.input_consumed_escape = true;
            } else if keys.escape
                && self.block_edit.is_active()
                && matches!(self.block_edit.ui, BlockUi::None)
            {
                self.request_leave_block(LeaveIntent::CloseOne);
                self.input_consumed_escape = true;
            } else if self.context_menu.is_none() {
                let live =
                    LiveValues::from_points(self.command.base_point(), self.drafting.current_point);
                let finish_empty = matches!(
                    self.command.kind(),
                    CommandKind::Line | CommandKind::Polyline
                ) || self.command.is_selecting_objects();
                let numeric =
                    ctx.input_mut(|input| self.dynamic_input.consume(input, live, finish_empty));
                match numeric {
                    DynamicKeyResult::Submit => {
                        if let Some(point) = self.drafting.current_point {
                            self.accept_command_point(point);
                        }
                    }
                    DynamicKeyResult::FinishEmpty => {
                        if self.command.can_finish() {
                            self.finish_command();
                        }
                    }
                    DynamicKeyResult::Invalid(message) => {
                        self.status = message.into();
                    }
                    DynamicKeyResult::Handled => {}
                    DynamicKeyResult::None => {
                        if keys.enter && self.command.is_active() {
                            self.finish_command();
                        } else if keys.enter
                            && !self.command.is_active()
                            && self.last_command.is_some_and(CommandKind::is_measure)
                        {
                            if let Some(kind) = self.last_command {
                                self.start_kind(kind);
                            }
                        } else if keys.line {
                            self.start_line_command();
                        } else if keys.polyline {
                            self.start_polyline_command();
                        } else if keys.circle {
                            if self.command.can_close() {
                                self.close_command();
                            } else {
                                self.start_circle_command();
                            }
                        } else if keys.arc {
                            self.start_arc_command();
                        } else if keys.rectangle {
                            self.start_rectangle_command();
                        } else if keys.distance {
                            self.start_distance_command();
                        }
                    }
                }
            }
        }
        if self.load_rx.is_some() || self.capture.is_some() {
            ctx.request_repaint();
        }

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.heading("MyCad");
                let has_drawing = self.document.is_some();
                if save_quick_access_button(ui, has_drawing).clicked() {
                    let _ = self.save_drawing();
                }
                ui.separator();
                ui.menu_button("File", |ui| {
                    if ui.button("Open…    Ctrl+O").clicked() {
                        ui.close();
                        self.open_dialog();
                    }
                    if ui
                        .add_enabled(has_drawing, egui::Button::new("Save    Ctrl+S"))
                        .clicked()
                    {
                        ui.close();
                        let _ = self.save_drawing();
                    }
                    if ui
                        .add_enabled(has_drawing, egui::Button::new("Save As…    Ctrl+Shift+S"))
                        .clicked()
                    {
                        ui.close();
                        let _ = self.save_as_drawing();
                    }
                    ui.separator();
                    ui.menu_button("Export", |ui| {
                        if ui
                            .add_enabled(has_drawing, egui::Button::new("PDF…"))
                            .clicked()
                        {
                            ui.close();
                            self.export_pdf_dialog();
                        }
                    });
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ui.close();
                        if self.is_dirty() {
                            self.pending_discard = Some(PendingDiscard::Quit);
                        } else {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui
                        .add_enabled(self.can_undo(), egui::Button::new("Undo    Ctrl+Z"))
                        .clicked()
                    {
                        ui.close();
                        self.undo();
                    }
                    if ui
                        .add_enabled(self.can_redo(), egui::Button::new("Redo    Ctrl+Y"))
                        .clicked()
                    {
                        ui.close();
                        self.redo();
                    }
                });
                ui.menu_button("Draw", |ui| {
                    if ui.button("Line    L").clicked() {
                        ui.close();
                        self.start_line_command();
                    }
                    if ui.button("Polyline    P").clicked() {
                        ui.close();
                        self.start_polyline_command();
                    }
                    if ui.button("Circle    C").clicked() {
                        ui.close();
                        self.start_circle_command();
                    }
                    if ui.button("Arc    A").clicked() {
                        ui.close();
                        self.start_arc_command();
                    }
                    if ui.button("Rectangle    R").clicked() {
                        ui.close();
                        self.start_rectangle_command();
                    }
                });
                ui.menu_button("Modify", |ui| {
                    if ui.button("Move").clicked() {
                        ui.close();
                        self.start_move_command();
                    }
                    if ui.button("Copy").clicked() {
                        ui.close();
                        self.start_copy_command();
                    }
                    if ui.button("Rotate").clicked() {
                        ui.close();
                        self.start_rotate_command();
                    }
                    if ui.button("Mirror").clicked() {
                        ui.close();
                        self.start_mirror_command();
                    }
                    if ui.button("Scale").clicked() {
                        ui.close();
                        self.start_scale_command();
                    }
                    ui.separator();
                    if ui.button("Erase    Del").clicked() {
                        ui.close();
                        self.start_erase_command();
                    }
                });
                ui.menu_button("Measure", |ui| {
                    if ui.button("Distance    D").clicked() {
                        ui.close();
                        self.start_distance_command();
                    }
                    if ui.button("Angle").clicked() {
                        ui.close();
                        self.start_angle_command();
                    }
                    if ui.button("Radius").clicked() {
                        ui.close();
                        self.start_radius_command();
                    }
                    if ui.button("Area").clicked() {
                        ui.close();
                        self.start_area_command();
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui.button("Zoom Extents").clicked() {
                        ui.close();
                        let size = ctx.available_rect();
                        self.zoom_extents(size.width() as f64, size.height() as f64);
                    }
                    if ui.button("Show Home").clicked() {
                        ui.close();
                        workspace::ensure_tab(&mut self.dock_state, WorkspaceTab::Home);
                    }
                    if ui.button("Show Properties").clicked() {
                        ui.close();
                        workspace::ensure_tab(&mut self.dock_state, WorkspaceTab::Properties);
                    }
                    if ui.button("Show Diagnostics").clicked() {
                        ui.close();
                        workspace::ensure_tab(&mut self.dock_state, WorkspaceTab::Diagnostics);
                    }
                    if ui.button("Show Blocks").clicked() {
                        ui.close();
                        workspace::ensure_tab(&mut self.dock_state, WorkspaceTab::Blocks);
                    }
                    if ui.button("Reset layout").clicked() {
                        ui.close();
                        self.dock_state = workspace::default_dock_state();
                        self.settings.compact_home_height_applied = false;
                    }
                });
                ui.menu_button("Settings", |ui| {
                    if ui.button("Preferences…").clicked() {
                        ui.close();
                        self.open_settings();
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !self.selection.is_empty() {
                        ui.label(format!("{} selected", self.selection.len()));
                        ui.separator();
                    }
                    if self.loading_path.is_some() {
                        ui.spinner();
                        ui.label("Importing…");
                    }
                });
            });
            ui.add_space(2.0);
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.monospace(self.status_file_label());
                ui.separator();
                ui.label(&self.status).on_hover_text(self.command.prompt());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .selectable_label(self.drafting.preferences.osnap_enabled, "OSNAP  F3")
                        .clicked()
                    {
                        self.toggle_osnap();
                    }
                    if ui
                        .selectable_label(self.drafting.preferences.ortho_enabled, "ORTHO  F8")
                        .clicked()
                    {
                        self.toggle_ortho();
                    }
                    ui.separator();
                    let coordinate = self
                        .cursor_world
                        .map(|point| format!("X {:>10.3}  Y {:>10.3}", point.x, point.y))
                        .unwrap_or_else(|| "X          —  Y          —".into());
                    ui.monospace(coordinate);
                    if let Some(err) = &self.error {
                        ui.separator();
                        ui.colored_label(egui::Color32::from_rgb(220, 120, 90), err);
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            workspace::show_workspace(ui, self);
        });

        self.show_viewport_menu(ctx);

        self.show_pdf_plot_dialog(ctx);
        self.show_lossy_save_dialog(ctx);
        self.show_discard_dialog(ctx);
        self.show_block_ui(ctx);

        match settings_ui::show(ctx, self) {
            SettingsAction::Apply => self.apply_settings(frame.storage_mut()),
            SettingsAction::Cancel => self.cancel_settings(),
            SettingsAction::None => {}
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.set_dock_state(&self.dock_state);
        self.settings.save(storage);
    }
}

fn paint_pdf_preview(ui: &mut Ui, state: &PdfPlotDialogState) {
    let (page_w, page_h) = state.options.page_size_pt();
    let preview_h = 88.0_f32;
    let preview_w = preview_h * (page_w / page_h.max(1.0)) as f32;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(preview_w, preview_h), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(248, 248, 248));
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(80, 80, 80)),
        egui::StrokeKind::Inside,
    );
    let mx = (state.options.margin_pt() / page_w) as f32 * rect.width();
    let my = (state.options.margin_pt() / page_h) as f32 * rect.height();
    let inner = Rect::from_min_max(rect.min + egui::vec2(mx, my), rect.max - egui::vec2(mx, my));
    if inner.width() < 2.0 || inner.height() < 2.0 {
        return;
    }
    let (world_w, world_h) = match (state.area, state.window) {
        (PdfPlotAreaKind::Window, Some(window)) => {
            (window.width().max(1e-6), window.height().max(1e-6))
        }
        _ => (1.0, 1.0),
    };
    let scale = (inner.width() as f64 / world_w).min(inner.height() as f64 / world_h);
    let mapped_w = (world_w * scale) as f32;
    let mapped_h = (world_h * scale) as f32;
    let ox = if state.options.center_plot {
        inner.center().x - mapped_w * 0.5
    } else {
        inner.min.x
    };
    let oy = if state.options.center_plot {
        inner.center().y - mapped_h * 0.5
    } else {
        inner.max.y - mapped_h
    };
    let plot = Rect::from_min_size(egui::pos2(ox, oy), egui::vec2(mapped_w, mapped_h));
    painter.rect_filled(plot, 0.0, egui::Color32::from_rgb(210, 230, 210));
}

fn handle_plot_window_pick(
    app: &mut MyCadApp,
    ui: &Ui,
    response: &egui::Response,
    origin: Point2,
    size: Point2,
) {
    if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        app.cancel_plot_window_pick();
        app.input_consumed_escape = true;
        return;
    }
    if !response.clicked_by(PointerButton::Primary) {
        return;
    }
    let Some(screen) = response
        .interact_pointer_pos()
        .or(ui.input(|i| i.pointer.latest_pos()))
        .filter(|pos| rect_contains_pos(origin, size, *pos))
    else {
        return;
    };
    let world =
        app.camera
            .screen_to_world(Point2::new(screen.x as f64, screen.y as f64), origin, size);
    let first = match &app.pdf_plot {
        PdfPlotUi::PickWindow(pick) => pick.first,
        _ => None,
    };
    if let Some(first) = first {
        app.finish_plot_window_pick(first, world);
        return;
    }
    if let PdfPlotUi::PickWindow(pick) = &mut app.pdf_plot {
        pick.first = Some(world);
        app.status = "Specify opposite corner".into();
    }
}

fn rect_contains_pos(origin: Point2, size: Point2, pos: egui::Pos2) -> bool {
    let x = pos.x as f64;
    let y = pos.y as f64;
    x >= origin.x && y >= origin.y && x <= origin.x + size.x && y <= origin.y + size.y
}

fn handle_viewport_input(app: &mut MyCadApp, ui: &Ui, response: &egui::Response, rect: Rect) {
    let origin = Point2::new(rect.min.x as f64, rect.min.y as f64);
    let size = Point2::new(rect.width() as f64, rect.height() as f64);
    let bindings = app.settings.bindings.clone();
    let modifiers = ui.input(|i| i.modifiers);
    let typing = crate::input::text_field_has_focus(ui.ctx());

    let pointer_pos = if response.hovered() || response.dragged() {
        ui.input(|input| input.pointer.latest_pos())
    } else {
        response.interact_pointer_pos()
    };
    if let Some(screen) = pointer_pos.filter(|position| rect.contains(*position)) {
        let raw =
            app.camera
                .screen_to_world(Point2::new(screen.x as f64, screen.y as f64), origin, size);
        app.cursor_world = Some(raw);
        if app.command.requests_point() {
            if app.command.uses_osnap() {
                let base = app.command.base_point();
                app.command.write_snap_features(&mut app.command_snaps);
                let resolved = app.drafting.resolve_point(
                    raw,
                    base,
                    modifiers.shift,
                    &app.camera,
                    size.y,
                    &app.snaps,
                    &app.command_snaps,
                );
                let constrained = app.dynamic_input.constrain(base, resolved);
                if let Some(snap) = app.drafting.acquired_snap {
                    if snap.point.distance(constrained) > GEOM_TOLERANCE {
                        app.drafting.acquired_snap = None;
                    }
                }
                app.drafting.current_point = Some(constrained);
                app.cursor_world = Some(constrained);
            } else {
                app.drafting.acquired_snap = None;
                app.drafting.current_point = Some(raw);
                app.cursor_world = Some(raw);
            }
        }
    }

    if app.context_menu.is_some() {
        app.last_pointer = ui.input(|i| i.pointer.latest_pos());
        return;
    }

    if app.pdf_plot.is_picking() {
        handle_plot_window_pick(app, ui, response, origin, size);
        app.last_pointer = ui.input(|i| i.pointer.latest_pos());
        return;
    }

    if let BlockUi::PickBase(mut dialog) = std::mem::replace(&mut app.block_edit.ui, BlockUi::None)
    {
        if response.clicked_by(PointerButton::Primary) {
            if let Some(point) = app.cursor_world.or_else(|| {
                ui.input(|i| i.pointer.latest_pos()).map(|pos| {
                    app.camera.screen_to_world(
                        Point2::new(pos.x as f64, pos.y as f64),
                        origin,
                        size,
                    )
                })
            }) {
                dialog.base_x = format!("{:.4}", point.x);
                dialog.base_y = format!("{:.4}", point.y);
                app.block_edit.ui = BlockUi::Create(dialog);
                app.status = "Base point picked".into();
                app.last_pointer = ui.input(|i| i.pointer.latest_pos());
                return;
            }
        }
        app.block_edit.ui = BlockUi::PickBase(dialog);
        app.last_pointer = ui.input(|i| i.pointer.latest_pos());
        if response.clicked_by(PointerButton::Primary) {
            return;
        }
    }

    if !typing
        && !app.input_consumed_escape
        && ui.input(|i| bindings.key_pressed(InputAction::SelectClear, i))
        && app.box_select.take().is_none()
    {
        app.selection.clear();
    }
    if !typing && ui.input(|i| bindings.key_pressed(InputAction::ZoomExtents, i)) {
        app.zoom_extents(size.x, size.y);
    }

    let box_was_active = app.box_select.is_some();
    if !app.command.requests_point() {
        update_box_select(app, ui, response, origin, size, modifiers);
    }
    let box_active = box_was_active || app.box_select.is_some();

    let mut panning = false;
    if !box_active {
        for button in [
            PointerButton::Primary,
            PointerButton::Middle,
            PointerButton::Secondary,
        ] {
            if response.dragged_by(button) && bindings.dragged(InputAction::Pan, button, modifiers)
            {
                panning = true;
                if let (Some(prev), Some(now)) =
                    (app.last_pointer, ui.input(|i| i.pointer.latest_pos()))
                {
                    let delta = app.camera.pan_screen(
                        Point2::new(prev.x as f64, prev.y as f64),
                        Point2::new(now.x as f64, now.y as f64),
                        origin,
                        size,
                    );
                    app.camera.pan_world(delta);
                }
            }
        }
    }

    let mut opened_menu = false;
    if !panning && !box_active {
        for button in [
            PointerButton::Primary,
            PointerButton::Middle,
            PointerButton::Secondary,
        ] {
            if response.clicked_by(button)
                && bindings.clicked(InputAction::ContextMenu, button, modifiers)
            {
                let pos = response
                    .interact_pointer_pos()
                    .or(app.last_pointer)
                    .unwrap_or(rect.center());
                app.open_context_menu(pos, origin, size);
                opened_menu = true;
                break;
            }
        }
    }

    if !opened_menu && !app.measure_card_hovered && app.command.requests_point() {
        if response.clicked_by(PointerButton::Primary) {
            if let Some(point) = app.drafting.current_point {
                if app.command.kind().is_measure() {
                    app.accept_measure_click(point);
                } else {
                    app.accept_command_point(point);
                }
            }
        }
    } else if !opened_menu && !box_active && !app.measure_card_hovered {
        for button in [
            PointerButton::Primary,
            PointerButton::Middle,
            PointerButton::Secondary,
        ] {
            if response.double_clicked_by(button)
                && bindings.double_clicked(InputAction::ZoomExtents, button, modifiers)
            {
                let hit = response.interact_pointer_pos().and_then(|pos| {
                    pick_entity(
                        &app.display,
                        &app.camera,
                        Point2::new(pos.x as f64, pos.y as f64),
                        origin,
                        size,
                    )
                });
                if !app.command.is_active() && app.try_enter_block_from_hit(hit) {
                    continue;
                }
                app.zoom_extents(size.x, size.y);
                continue;
            }
            if !response.clicked_by(button) {
                continue;
            }
            let world_hit = response.interact_pointer_pos().map(|pos| {
                pick_entity(
                    &app.display,
                    &app.camera,
                    Point2::new(pos.x as f64, pos.y as f64),
                    origin,
                    size,
                )
            });
            let hit = world_hit.flatten();
            if app.command.is_erase_picking()
                && (bindings.clicked(InputAction::SelectReplace, button, modifiers)
                    || bindings.clicked(InputAction::SelectAdd, button, modifiers)
                    || bindings.clicked(InputAction::SelectRemove, button, modifiers))
            {
                if let Some(id) = hit {
                    app.erase_ids(&[id], true);
                }
            } else if let Some(op) = selection_op_for_click(&bindings, button, modifiers) {
                app.selection.apply_click(hit, op);
            }
        }
    }

    app.last_pointer = ui.input(|i| i.pointer.latest_pos());

    if !box_active && response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.0 {
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                let world = app.camera.screen_to_world(
                    Point2::new(pos.x as f64, pos.y as f64),
                    origin,
                    size,
                );
                let factor = scroll_to_zoom_factor(scroll as f64, app.settings.zoom_speed);
                app.camera.zoom_at(world, factor);
            }
        }
    }
}

fn update_box_select(
    app: &mut MyCadApp,
    ui: &Ui,
    response: &egui::Response,
    origin: Point2,
    size: Point2,
    modifiers: egui::Modifiers,
) {
    let bindings = app.settings.bindings.clone();
    let mut started_this_frame = false;
    if app.box_select.is_none() {
        for button in [
            PointerButton::Primary,
            PointerButton::Middle,
            PointerButton::Secondary,
        ] {
            // Only the viewport widget may start a marquee. A global
            // press_origin-in-rect check also fires while resizing dock
            // splitters, whose hit rect overlaps the moving viewport.
            let dragging = response.drag_started_by(button) || response.dragged_by(button);
            if !dragging {
                continue;
            }
            if bindings.dragged(InputAction::Pan, button, modifiers) {
                continue;
            }
            let Some(op) = selection_op_for_pointer(&bindings, button, modifiers) else {
                continue;
            };
            let Some(start) = ui
                .input(|i| i.pointer.press_origin())
                .or(response.interact_pointer_pos())
                .or(app.last_pointer)
            else {
                continue;
            };
            let current = ui.input(|i| i.pointer.latest_pos()).unwrap_or(start);
            app.box_select = Some(BoxSelectDrag {
                button,
                start,
                current,
                op,
                candidates: Vec::new(),
            });
            started_this_frame = true;
            break;
        }
    }

    let (moved, start, current, button, mut candidates) = {
        let Some(drag) = app.box_select.as_mut() else {
            return;
        };
        let mut moved = started_this_frame;
        if let Some(pos) = ui
            .input(|i| i.pointer.latest_pos())
            .or(response.interact_pointer_pos())
        {
            if pos != drag.current {
                drag.current = pos;
                moved = true;
            }
        }
        let start = Point2::new(drag.start.x as f64, drag.start.y as f64);
        let current = Point2::new(drag.current.x as f64, drag.current.y as f64);
        let button = drag.button;
        let candidates = if moved {
            std::mem::take(&mut drag.candidates)
        } else {
            Vec::new()
        };
        (moved, start, current, button, candidates)
    };
    if moved {
        box_pick_entities_into(
            &app.display,
            &app.camera,
            start,
            current,
            origin,
            size,
            &mut candidates,
        );
        if let Some(drag) = app.box_select.as_mut() {
            drag.candidates = candidates;
        }
    }
    let released =
        response.drag_stopped_by(button) || ui.input(|i| i.pointer.button_released(button));
    if released && !started_this_frame {
        if let Some(drag) = app.box_select.take() {
            if app.command.is_erase_picking() {
                app.erase_ids(&drag.candidates, true);
            } else {
                app.selection.commit_box(&drag.candidates, drag.op);
            }
        }
    }
}

fn selection_op_for_click(
    bindings: &InputMap,
    button: PointerButton,
    modifiers: egui::Modifiers,
) -> Option<SelectionOp> {
    if bindings.clicked(InputAction::SelectRemove, button, modifiers) {
        Some(SelectionOp::Remove)
    } else if bindings.clicked(InputAction::SelectAdd, button, modifiers) {
        Some(SelectionOp::Add)
    } else if bindings.clicked(InputAction::SelectReplace, button, modifiers) {
        Some(SelectionOp::Replace)
    } else {
        None
    }
}

fn selection_op_for_pointer(
    bindings: &InputMap,
    button: PointerButton,
    modifiers: egui::Modifiers,
) -> Option<SelectionOp> {
    if bindings.selects_with_pointer(InputAction::SelectRemove, button, modifiers) {
        Some(SelectionOp::Remove)
    } else if bindings.selects_with_pointer(InputAction::SelectAdd, button, modifiers) {
        Some(SelectionOp::Add)
    } else if bindings.selects_with_pointer(InputAction::SelectReplace, button, modifiers) {
        Some(SelectionOp::Replace)
    } else {
        None
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn suggested_save_name(document: &Document, prefer_dwg: bool) -> String {
    if prefer_dwg {
        drawing_copy_name(document, "dwg")
    } else {
        drawing_stem_name(document, "dxf")
    }
}

fn suggested_pdf_name(document: &Document) -> String {
    drawing_stem_name(document, "pdf")
}

fn drawing_stem(document: &Document) -> String {
    let name = document.file_name();
    Path::new(&name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| *stem != "(untitled)")
        .unwrap_or("drawing")
        .to_string()
}

fn drawing_stem_name(document: &Document, extension: &str) -> String {
    format!("{}.{extension}", drawing_stem(document))
}

fn drawing_copy_name(document: &Document, extension: &str) -> String {
    let stem = drawing_stem(document);
    let stem = if stem.ends_with("-MyCad") {
        stem
    } else {
        format!("{stem}-MyCad")
    };
    format!("{stem}.{extension}")
}

fn with_save_extension(path: PathBuf, prefer_dwg: bool) -> PathBuf {
    match CadFileFormat::from_path(&path) {
        Some(_) => path,
        None => with_extension(path, if prefer_dwg { "dwg" } else { "dxf" }),
    }
}

fn with_pdf_extension(path: PathBuf) -> PathBuf {
    with_extension(path, "pdf")
}

fn with_extension(path: PathBuf, extension: &str) -> PathBuf {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case(extension) => path,
        _ => path.with_extension(extension),
    }
}

fn in_place_cad_path(document: &Document, written_by_mycad: bool) -> Option<&Path> {
    let path = document.source_path.as_deref()?;
    match CadFileFormat::from_path(path) {
        Some(CadFileFormat::Dxf) => Some(path),
        Some(CadFileFormat::Dwg) if written_by_mycad || is_mycad_saved_dwg(path) => Some(path),
        _ => None,
    }
}

fn is_mycad_saved_dwg(path: &Path) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.ends_with("-MyCad"))
}

fn needs_lossy_save_warning(document: &Document) -> bool {
    document.diagnostics.unsupported_total() > 0
}

fn lossy_save_message(count: u64) -> String {
    let noun = if count == 1 { "entity" } else { "entities" };
    format!(
        "This drawing contains {count} {noun} that MyCad cannot fully preserve. Saving as DWG/DXF may change or remove unsupported content."
    )
}

fn format_save_status(path: &Path, report: &SaveReport) -> String {
    let name = file_name(path);
    if CadFileFormat::from_path(path) == Some(CadFileFormat::Dwg) && !report.warnings.is_empty() {
        let count = report.warnings.len();
        let noun = if count == 1 { "warning" } else { "warnings" };
        format!("DWG saved with {count} compatibility {noun}")
    } else {
        format!("Saved {name}")
    }
}

fn format_pdf_export_status(path: &Path, _report: &SaveReport) -> String {
    format!("Exported {}", file_name(path))
}

fn format_save_failed(err: impl std::fmt::Display) -> String {
    format!("Save failed: {err}")
}

fn save_quick_access_button(ui: &mut egui::Ui, enabled: bool) -> egui::Response {
    let icon = egui::RichText::new(FLOPPY_DISK).size(16.0);
    ui.add_enabled(
        enabled,
        egui::Button::new(icon)
            .frame(false)
            .min_size(egui::vec2(22.0, 18.0)),
    )
    .on_hover_text(SAVE_TOOLTIP)
    .on_disabled_hover_text(SAVE_TOOLTIP)
}

fn format_import_error(err: &ImportError) -> String {
    match err {
        ImportError::Critical(code) => format!(
            "LibreDWG could not read this DWG (critical error {code}). The file may use features LibreDWG cannot decode."
        ),
        ImportError::InvalidPath => "The file path is not valid UTF-8.".into(),
        ImportError::Io(e) => format!("Could not open file: {e}"),
    }
}

fn format_dwg_write_error(err: &DwgWriteError) -> String {
    match err {
        DwgWriteError::Dxf(err) => err.to_string(),
        DwgWriteError::Convert(DwgExportError::Critical { path, code }) => {
            format!(
                "LibreDWG could not write {} (critical error {code}).",
                file_name(path)
            )
        }
        DwgWriteError::Convert(DwgExportError::InvalidPath) => {
            "The file path is not valid UTF-8.".into()
        }
        DwgWriteError::Convert(DwgExportError::Io(err)) | DwgWriteError::Io(err) => {
            format!("Could not write file: {err}")
        }
    }
}

fn line_segment(entity: &Entity) -> Option<(Point2, Point2)> {
    match entity.geometry {
        Geometry::Line { start, end } => Some((start.xy(), end.xy())),
        _ => None,
    }
}

fn radius_from_entity(
    entity: Option<&Entity>,
) -> Option<Result<cad_core::RadiusMeasurement, cad_core::MeasureError>> {
    let entity = entity?;
    match &entity.geometry {
        Geometry::Circle { center, radius, .. } => {
            let toward = Point2::new(center.x + *radius, center.y);
            Some(
                cad_core::RadiusMeasurement::circle(center.xy(), *radius, toward)
                    .ok_or(cad_core::MeasureError::InvalidGeometry),
            )
        }
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            ..
        } => {
            let toward = Point2::new(center.x + *radius, center.y);
            Some(
                cad_core::RadiusMeasurement::arc(
                    center.xy(),
                    *radius,
                    *start_angle,
                    *end_angle,
                    toward,
                )
                .ok_or(cad_core::MeasureError::InvalidGeometry),
            )
        }
        _ => None,
    }
}

fn area_from_entity(
    entity: Option<&Entity>,
) -> Option<Result<cad_core::AreaMeasurement, cad_core::MeasureError>> {
    let entity = entity?;
    match &entity.geometry {
        Geometry::Circle { center, radius, .. } => Some(
            cad_core::AreaMeasurement::from_circle(center.xy(), *radius)
                .ok_or(cad_core::MeasureError::InvalidGeometry),
        ),
        Geometry::LwPolyline {
            vertices, closed, ..
        }
        | Geometry::Polyline {
            vertices, closed, ..
        } => Some(cad_core::AreaMeasurement::from_polyline(vertices, *closed)),
        _ => None,
    }
}

#[cfg(test)]
mod save_as_tests {
    use super::*;

    #[test]
    fn dwg_source_suggests_a_mycad_copy_name() {
        let mut document = Document::default();
        document.source_path = Some(PathBuf::from("plant.dwg"));
        assert_eq!(suggested_save_name(&document, true), "plant-MyCad.dwg");
        assert_eq!(suggested_save_name(&document, false), "plant.dxf");
    }

    #[test]
    fn existing_mycad_suffix_is_not_repeated() {
        let mut document = Document::default();
        document.source_path = Some(PathBuf::from("plant-MyCad.dwg"));
        assert_eq!(suggested_save_name(&document, true), "plant-MyCad.dwg");
    }

    #[test]
    fn untitled_drawing_uses_a_generic_dxf_name() {
        let document = Document::default();
        assert_eq!(suggested_save_name(&document, false), "drawing.dxf");
        assert_eq!(suggested_save_name(&document, true), "drawing-MyCad.dwg");
    }

    #[test]
    fn save_extension_follows_chosen_format() {
        assert_eq!(
            with_save_extension(PathBuf::from("out.dwg"), false),
            PathBuf::from("out.dwg")
        );
        assert_eq!(
            with_save_extension(PathBuf::from("out.DXF"), true),
            PathBuf::from("out.DXF")
        );
        assert_eq!(
            with_save_extension(PathBuf::from("out"), false),
            PathBuf::from("out.dxf")
        );
        assert_eq!(
            with_save_extension(PathBuf::from("out"), true),
            PathBuf::from("out.dwg")
        );
    }

    #[test]
    fn dwg_source_cannot_save_in_place() {
        let mut document = Document::default();
        document.source_path = Some(PathBuf::from("plant.dwg"));
        assert!(in_place_cad_path(&document, false).is_none());
    }

    #[test]
    fn mycad_dwg_copy_can_save_in_place() {
        let mut document = Document::default();
        document.source_path = Some(PathBuf::from("plant-MyCad.dwg"));
        assert_eq!(
            in_place_cad_path(&document, false),
            Some(Path::new("plant-MyCad.dwg"))
        );
    }

    #[test]
    fn session_written_dwg_can_save_in_place() {
        let mut document = Document::default();
        document.source_path = Some(PathBuf::from("custom.dwg"));
        assert_eq!(
            in_place_cad_path(&document, true),
            Some(Path::new("custom.dwg"))
        );
    }

    #[test]
    fn saved_dxf_can_save_in_place() {
        let mut document = Document::default();
        document.source_path = Some(PathBuf::from("plant.dxf"));
        assert_eq!(
            in_place_cad_path(&document, false),
            Some(Path::new("plant.dxf"))
        );
    }

    #[test]
    fn pdf_source_cannot_save_in_place() {
        let mut document = Document::default();
        document.source_path = Some(PathBuf::from("sheet.pdf"));
        assert!(in_place_cad_path(&document, false).is_none());
    }

    #[test]
    fn pdf_name_does_not_reuse_dxf_extension() {
        let mut document = Document::default();
        document.source_path = Some(PathBuf::from("plant.dxf"));
        assert_eq!(suggested_pdf_name(&document), "plant.pdf");
    }

    #[test]
    fn export_status_does_not_say_saved() {
        let report = SaveReport {
            warnings: Vec::new(),
            entities_written: 2,
        };
        assert_eq!(
            format_pdf_export_status(Path::new("Plant.pdf"), &report),
            "Exported Plant.pdf"
        );
    }

    #[test]
    fn save_status_uses_the_file_name() {
        let report = SaveReport {
            warnings: Vec::new(),
            entities_written: 4,
        };
        assert_eq!(
            format_save_status(Path::new("Plant.dxf"), &report),
            "Saved Plant.dxf"
        );
        assert_eq!(
            format_save_status(Path::new("Plant.dwg"), &report),
            "Saved Plant.dwg"
        );
    }

    #[test]
    fn dwg_status_reports_compatibility_warnings() {
        let report = SaveReport {
            warnings: vec![
                "DIMENSION fallback".into(),
                "MLINE exploded".into(),
                "HATCH spline edge".into(),
            ],
            entities_written: 12,
        };
        assert_eq!(
            format_save_status(Path::new("Plant.dwg"), &report),
            "DWG saved with 3 compatibility warnings"
        );
        let one = SaveReport {
            warnings: vec!["DIMENSION fallback".into()],
            entities_written: 2,
        };
        assert_eq!(
            format_save_status(Path::new("Plant.dwg"), &one),
            "DWG saved with 1 compatibility warning"
        );
        assert_eq!(
            format_save_status(Path::new("Plant.dxf"), &report),
            "Saved Plant.dxf"
        );
    }

    #[test]
    fn save_failed_status_prefixes_the_error() {
        assert_eq!(format_save_failed("disk full"), "Save failed: disk full");
    }

    #[test]
    fn save_quick_access_tooltip_is_save_and_shortcut() {
        assert_eq!(SAVE_TOOLTIP, "Save\nCtrl+S");
    }

    #[test]
    fn unsupported_entities_require_a_lossy_save_warning() {
        let mut document = Document::default();
        assert!(!needs_lossy_save_warning(&document));
        document.diagnostics.bump_unsupported("PROXY");
        document.diagnostics.bump_unsupported("PROXY");
        document.diagnostics.bump_unsupported("XREF");
        assert_eq!(document.diagnostics.unsupported_total(), 3);
        assert!(needs_lossy_save_warning(&document));
    }

    #[test]
    fn lossy_save_message_uses_singular_and_plural() {
        assert_eq!(
            lossy_save_message(1),
            "This drawing contains 1 entity that MyCad cannot fully preserve. Saving as DWG/DXF may change or remove unsupported content."
        );
        assert_eq!(
            lossy_save_message(14),
            "This drawing contains 14 entities that MyCad cannot fully preserve. Saving as DWG/DXF may change or remove unsupported content."
        );
    }
}

#[cfg(test)]
mod command_switch_tests {
    use super::*;

    fn model_count(app: &MyCadApp) -> usize {
        app.document
            .as_ref()
            .map(|document| document.model_space.len())
            .unwrap_or(0)
    }

    fn assert_no_stale_interaction(app: &MyCadApp) {
        assert!(
            app.command.preview(Some(Point2::new(25.0, 17.0))).is_none(),
            "rubber-band preview should not survive a tool switch"
        );
        assert!(
            !app.dynamic_input.is_active(),
            "dynamic input should reset with the new command"
        );
        assert!(app.drafting.acquired_snap.is_none());
        assert!(app.drafting.command_base_point.is_none());
        assert!(app.command_snaps.is_empty());
        assert!(app.measurement.is_none());
        assert!(app.box_select.is_none());
    }

    #[test]
    fn idle_starts_line() {
        let mut app = MyCadApp::for_test();
        assert!(!app.command.is_active());
        app.start_line_command();
        assert_eq!(app.command.kind(), CommandKind::Line);
        assert!(app.command.is_active());
        assert_no_stale_interaction(&app);
    }

    #[test]
    fn line_switches_to_circle_without_escape() {
        let mut app = MyCadApp::for_test();
        app.start_line_command();
        app.start_circle_command();
        assert_eq!(app.command.kind(), CommandKind::Circle);
        assert_no_stale_interaction(&app);
    }

    #[test]
    fn unfinished_circle_is_discarded_when_switching_to_rectangle() {
        let mut app = MyCadApp::for_test();
        app.start_circle_command();
        app.accept_command_point(Point2::new(0.0, 0.0));
        assert!(app.dynamic_input.is_active());
        assert!(app.command.preview(Some(Point2::new(4.0, 0.0))).is_some());
        app.start_rectangle_command();
        assert_eq!(app.command.kind(), CommandKind::Rectangle);
        assert_eq!(model_count(&app), 0);
        assert_no_stale_interaction(&app);
    }

    #[test]
    fn unfinished_rectangle_is_discarded_when_switching_to_arc() {
        let mut app = MyCadApp::for_test();
        app.start_rectangle_command();
        app.accept_command_point(Point2::new(1.0, 1.0));
        assert!(app.dynamic_input.is_active());
        app.start_arc_command();
        assert_eq!(app.command.kind(), CommandKind::Arc);
        assert_eq!(model_count(&app), 0);
        assert_no_stale_interaction(&app);
    }

    #[test]
    fn committed_line_geometry_survives_a_switch_to_distance() {
        let mut app = MyCadApp::for_test();
        app.start_line_command();
        app.accept_command_point(Point2::new(0.0, 0.0));
        app.accept_command_point(Point2::new(10.0, 0.0));
        assert_eq!(model_count(&app), 1);
        app.command.write_snap_features(&mut app.command_snaps);
        assert!(app.command_snaps.is_empty());
        assert_eq!(app.command.base_point(), None);
        app.start_distance_command();
        assert_eq!(app.command.kind(), CommandKind::Distance);
        assert_eq!(model_count(&app), 1);
        assert!(app.can_undo());
        assert_no_stale_interaction(&app);
    }

    #[test]
    fn distance_switches_to_line() {
        let mut app = MyCadApp::for_test();
        app.start_distance_command();
        app.accept_command_point(Point2::new(0.0, 0.0));
        app.start_line_command();
        assert_eq!(app.command.kind(), CommandKind::Line);
        assert_eq!(model_count(&app), 0);
        assert_no_stale_interaction(&app);
    }

    #[test]
    fn move_switches_to_copy() {
        let mut app = MyCadApp::for_test();
        app.start_move_command();
        assert_eq!(app.command.kind(), CommandKind::Move);
        app.start_copy_command();
        assert_eq!(app.command.kind(), CommandKind::Copy);
        assert_no_stale_interaction(&app);
    }

    #[test]
    fn context_menu_copy_switches_from_move() {
        let mut app = MyCadApp::for_test();
        app.start_move_command();
        app.apply_context_action(ContextAction::Copy, 800.0, 600.0);
        assert_eq!(app.command.kind(), CommandKind::Copy);
        assert_no_stale_interaction(&app);
    }

    #[test]
    fn clicking_the_active_tool_does_not_restart_it() {
        let mut app = MyCadApp::for_test();
        app.start_line_command();
        app.accept_command_point(Point2::new(3.0, 4.0));
        assert_eq!(app.command.base_point(), Some(Point2::new(3.0, 4.0)));
        assert!(app.dynamic_input.is_active());
        app.start_line_command();
        assert_eq!(app.command.kind(), CommandKind::Line);
        assert_eq!(app.command.base_point(), Some(Point2::new(3.0, 4.0)));
        assert!(app.dynamic_input.is_active());
        assert_eq!(model_count(&app), 0);
    }

    #[test]
    fn escape_still_returns_to_idle() {
        let mut app = MyCadApp::for_test();
        app.start_line_command();
        app.accept_command_point(Point2::new(1.0, 1.0));
        app.cancel_command();
        assert!(app.command.is_idle());
        assert!(!app.command.is_active());
        assert_no_stale_interaction(&app);
        assert_eq!(app.status, "Command canceled");
    }

    #[test]
    fn one_click_tool_chain_matches_cad_ux() {
        let mut app = MyCadApp::for_test();
        let chain = [
            CommandKind::Line,
            CommandKind::Circle,
            CommandKind::Arc,
            CommandKind::Polyline,
            CommandKind::Rectangle,
            CommandKind::Distance,
            CommandKind::Move,
        ];
        for kind in chain {
            app.start_kind(kind);
            assert_eq!(app.command.kind(), kind);
            assert_no_stale_interaction(&app);
        }
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use cad_core::{Entity, EntityId, Geometry, Point3};

    fn model_count(app: &MyCadApp) -> usize {
        app.document
            .as_ref()
            .map(|document| document.model_space.len())
            .unwrap_or(0)
    }

    fn add_line(app: &mut MyCadApp, x0: f64, y0: f64, x1: f64, y1: f64) -> EntityId {
        let document = app.document.get_or_insert_with(Document::default);
        let entity = document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(x0, y0),
            end: Point3::from_xy(x1, y1),
        }));
        let id = entity.id;
        app.refresh_derived();
        id
    }

    #[test]
    fn line_second_click_commits_then_waits_for_a_new_first_point() {
        let mut app = MyCadApp::for_test();
        app.start_line_command();
        assert_eq!(app.status, "LINE • Specify first point");
        app.accept_command_point(Point2::new(0.0, 0.0));
        assert_eq!(model_count(&app), 0);
        assert_eq!(app.status, "LINE • Specify second point");
        app.accept_command_point(Point2::new(10.0, 0.0));
        assert_eq!(model_count(&app), 1);
        assert_eq!(app.command.kind(), CommandKind::Line);
        assert_eq!(app.command.base_point(), None);
        assert!(!app.dynamic_input.is_active());
        assert_eq!(app.status, "LINE • Specify first point");
        app.accept_command_point(Point2::new(2.0, 3.0));
        assert_eq!(model_count(&app), 1);
        app.accept_command_point(Point2::new(8.0, 3.0));
        assert_eq!(model_count(&app), 2);
        match &app.document.as_ref().unwrap().model_space[1].geometry {
            Geometry::Line { start, end } => {
                assert_eq!(start.xy(), Point2::new(2.0, 3.0));
                assert_eq!(end.xy(), Point2::new(8.0, 3.0));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn preselection_erase_and_delete_remove_immediately() {
        let mut app = MyCadApp::for_test();
        let a = add_line(&mut app, 0.0, 0.0, 1.0, 0.0);
        let b = add_line(&mut app, 0.0, 1.0, 1.0, 1.0);
        let c = add_line(&mut app, 0.0, 2.0, 1.0, 2.0);
        app.selection.replace_all([a, b, c]);
        app.erase_selected();
        assert_eq!(model_count(&app), 0);
        assert!(app.selection.is_empty());
        assert!(app.command.is_idle());
        app.undo();
        assert_eq!(model_count(&app), 3);
        assert!(app.document.as_ref().unwrap().entity_by_id(a).is_some());
        assert!(app.document.as_ref().unwrap().entity_by_id(b).is_some());
        assert!(app.document.as_ref().unwrap().entity_by_id(c).is_some());
        app.redo();
        assert_eq!(model_count(&app), 0);

        let d = add_line(&mut app, 4.0, 0.0, 5.0, 0.0);
        let e = add_line(&mut app, 4.0, 1.0, 5.0, 1.0);
        app.selection.replace_all([d, e]);
        app.start_erase_command();
        assert_eq!(model_count(&app), 0);
        assert!(app.command.is_idle());
        app.undo();
        assert_eq!(model_count(&app), 2);
    }

    #[test]
    fn command_first_erase_click_and_window_stay_active() {
        let mut app = MyCadApp::for_test();
        let a = add_line(&mut app, 0.0, 0.0, 1.0, 0.0);
        let b = add_line(&mut app, 0.0, 2.0, 1.0, 2.0);
        let c = add_line(&mut app, 0.0, 4.0, 1.0, 4.0);
        app.start_erase_command();
        assert!(app.command.is_erase_picking());
        assert_eq!(app.status, "ERASE • Click objects to erase • Esc to finish");
        app.erase_ids(&[a], true);
        assert_eq!(model_count(&app), 2);
        assert!(app.command.is_erase_picking());
        assert!(!app.selection.contains(a));
        app.erase_ids(&[b, c], true);
        assert_eq!(model_count(&app), 0);
        assert!(app.command.is_erase_picking());
        app.undo();
        assert_eq!(model_count(&app), 2);
        app.undo();
        assert_eq!(model_count(&app), 3);
        app.cancel_command();
        assert!(app.command.is_idle());
    }

    #[test]
    fn delete_hotkey_is_blocked_while_editing_or_drawing() {
        assert!(crate::input::erase_hotkey_allowed(false, false, false));
        assert!(!crate::input::erase_hotkey_allowed(true, false, false));
        assert!(!crate::input::erase_hotkey_allowed(false, true, false));
        assert!(!crate::input::erase_hotkey_allowed(false, false, true));
        let mut app = MyCadApp::for_test();
        let id = add_line(&mut app, 0.0, 0.0, 2.0, 0.0);
        app.selection.replace(id);
        app.start_line_command();
        app.erase_selected();
        assert_eq!(model_count(&app), 1);
        assert!(app.selection.contains(id));
        app.cancel_command();
        app.erase_selected();
        assert_eq!(model_count(&app), 0);
    }

    #[test]
    fn pointer_selection_ops_match_replace_add_remove() {
        let map = InputMap::standard();
        let none = egui::Modifiers::default();
        let mut shift = egui::Modifiers::default();
        shift.shift = true;
        let mut ctrl = egui::Modifiers::default();
        ctrl.ctrl = true;
        ctrl.command = true;
        assert_eq!(
            selection_op_for_click(&map, PointerButton::Primary, none),
            Some(SelectionOp::Replace)
        );
        assert_eq!(
            selection_op_for_click(&map, PointerButton::Primary, shift),
            Some(SelectionOp::Add)
        );
        assert_eq!(
            selection_op_for_click(&map, PointerButton::Primary, ctrl),
            Some(SelectionOp::Remove)
        );
        assert_eq!(
            selection_op_for_pointer(&map, PointerButton::Primary, shift),
            Some(SelectionOp::Add)
        );
        assert_eq!(
            selection_op_for_pointer(&map, PointerButton::Primary, ctrl),
            Some(SelectionOp::Remove)
        );
    }
}

#[cfg(test)]
mod plot_window_tests {
    use super::*;

    #[test]
    fn plot_window_pick_stores_normalized_window_from_either_direction() {
        let mut app = MyCadApp::for_test();
        app.pdf_plot = PdfPlotUi::PickWindow(PlotWindowPick {
            options: PdfExportOptions::default(),
            last_window: None,
            first: Some(Point2::new(5.0, 1.0)),
        });
        app.finish_plot_window_pick(Point2::new(5.0, 1.0), Point2::new(2.0, -1.0));
        let PdfPlotUi::Dialog(state) = &app.pdf_plot else {
            panic!("expected dialog after pick");
        };
        assert_eq!(state.area, PdfPlotAreaKind::Window);
        let window = state.window.expect("window");
        assert!((window.min.x - 2.0).abs() < 1e-12);
        assert!((window.min.y + 1.0).abs() < 1e-12);
        assert!((window.max.x - 5.0).abs() < 1e-12);
        assert!((window.max.y - 1.0).abs() < 1e-12);
    }

    #[test]
    fn escape_cancels_only_window_picking() {
        let mut app = MyCadApp::for_test();
        let mut options = PdfExportOptions::default();
        options.paper = PdfPaperSize::A3;
        app.pdf_plot = PdfPlotUi::PickWindow(PlotWindowPick {
            options,
            last_window: Some(Extents2::from_corners(
                Point2::new(0.0, 0.0),
                Point2::new(1.0, 1.0),
            )),
            first: Some(Point2::new(3.0, 3.0)),
        });
        app.cancel_plot_window_pick();
        let PdfPlotUi::Dialog(state) = &app.pdf_plot else {
            panic!("expected dialog after cancel");
        };
        assert_eq!(state.area, PdfPlotAreaKind::Window);
        assert_eq!(state.options.paper, PdfPaperSize::A3);
        assert!(state.window.is_some());
    }
}
