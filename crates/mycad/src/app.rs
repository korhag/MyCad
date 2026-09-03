//! Application chrome: menus, viewport interaction, loading, workspace.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use cad_core::{DistanceReport, Document, Extents2, Geometry, Point2, Point3, SnapIndex};
use cad_render::{
    tessellate_document, CadFrame, CadGpu, DisplayList, GpuUpload, OverlayBatches, SelectBoxMode,
};
use cad_viewport::Camera2;
use dwg_import::ImportError;
use eframe::egui::{self, PointerButton, Rect, Ui};

use crate::commands::{CommandOutput, CommandState};
use crate::drafting::DraftingState;
use crate::history::{Edit, History};
use crate::input::InputAction;
use crate::selection::{box_pick_entities_into, pick_entity, Selection};
use crate::settings::{scroll_to_zoom_factor, AppSettings, RgbColor};
use crate::settings_ui::{self, CaptureTarget, SettingsAction, SettingsTab};
use crate::theme;
use crate::workspace::{self, WorkspaceTab};

const SELECTION_OVERLAY_COLOR: [f32; 4] = [255.0 / 255.0, 196.0 / 255.0, 72.0 / 255.0, 1.0];

enum LoadMsg {
    Success {
        document: Box<Document>,
        display: Box<DisplayList>,
        snaps: Box<SnapIndex>,
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
    toggle: bool,
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
    distance: bool,
    enter: bool,
    escape: bool,
    undo: bool,
    redo: bool,
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
    history: History,
    snaps: Arc<SnapIndex>,
    cursor_world: Option<Point2>,
    input_consumed_escape: bool,
    pending_open: Option<PathBuf>,
    pending_discard: Option<PendingDiscard>,
    last_pointer: Option<egui::Pos2>,
    box_select: Option<BoxSelectDrag>,
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
        let drafting_preferences = settings.drafting;
        let mut app = Self {
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
            history: History::default(),
            snaps: Arc::new(SnapIndex::default()),
            cursor_world: None,
            input_consumed_escape: false,
            pending_open: initial_path,
            pending_discard: None,
            last_pointer: None,
            box_select: None,
        };
        workspace::sanitize_dock_state(&mut app.dock_state);
        if let Some(path) = app.pending_open.take() {
            app.start_load(path);
        }
        app
    }

    fn start_load(&mut self, path: PathBuf) {
        if self.is_dirty() {
            self.pending_discard = Some(PendingDiscard::Open(path));
            return;
        }
        self.start_load_now(path);
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
        self.drafting.clear_acquisition();
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
                    document.diagnostics.render_prepare_time = prepare.elapsed();
                    let _ = tx.send(LoadMsg::Success {
                        document: Box::new(document),
                        display: Box::new(display),
                        snaps: Box::new(snaps),
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
                self.drafting.clear_acquisition();
                self.history.clear();
                self.document = Some(document);
                self.display = Arc::new(display);
                self.snaps = Arc::new(*snaps);
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
        if self.is_dirty() {
            self.pending_discard = Some(PendingDiscard::OpenDialog);
            return;
        }
        self.open_dialog_now();
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

    pub(crate) fn command_is_active(&self) -> bool {
        self.command.is_active()
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
        self.box_select = None;
        self.history.commit_open();
        self.history.begin();
        self.command.start_line();
        self.drafting.clear_acquisition();
        self.status = "LINE started".into();
    }

    pub(crate) fn start_distance_command(&mut self) {
        self.box_select = None;
        self.finish_active_transaction();
        self.command.start_distance();
        self.drafting.clear_acquisition();
        self.status = "DIST started".into();
    }

    fn finish_command(&mut self) {
        self.finish_active_transaction();
        self.command.finish();
        self.drafting.clear_acquisition();
        self.status = "Ready".into();
    }

    fn cancel_command(&mut self) {
        self.finish_active_transaction();
        self.command.cancel();
        self.drafting.clear_acquisition();
        self.status = "Command canceled".into();
    }

    fn finish_active_transaction(&mut self) {
        self.history.commit_open();
    }

    pub(crate) fn undo(&mut self) {
        self.finish_active_transaction();
        self.command.cancel();
        self.drafting.clear_acquisition();
        let Some(document) = self.document.as_mut() else {
            return;
        };
        if self.history.undo(document) {
            self.refresh_derived();
            self.status = "Undo".into();
        }
    }

    pub(crate) fn redo(&mut self) {
        self.finish_active_transaction();
        self.command.cancel();
        self.drafting.clear_acquisition();
        let Some(document) = self.document.as_mut() else {
            return;
        };
        if self.history.redo(document) {
            self.refresh_derived();
            self.status = "Redo".into();
        }
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
        let Some(document) = self.document.as_mut() else {
            self.display = Arc::new(DisplayList::default());
            self.snaps = Arc::new(SnapIndex::default());
            self.gpu_upload = GpuUpload::Full;
            self.display_generation = self.display_generation.wrapping_add(1);
            self.selection.clear();
            return;
        };
        document.diagnostics.extents = document.compute_extents();
        self.display = Arc::new(tessellate_document(document));
        self.snaps = Arc::new(SnapIndex::build(document));
        self.gpu_upload = GpuUpload::Full;
        self.display_generation = self.display_generation.wrapping_add(1);
        self.selection.retain_valid(document);
    }

    fn commit_line_segment(&mut self, [start, end]: [Point2; 2]) {
        let (edit, entity) = {
            let Some(document) = self.ensure_document() else {
                return;
            };
            let index = document.model_space.len();
            let entity = document.new_entity(Geometry::Line {
                start: Point3::from_xy(start.x, start.y),
                end: Point3::from_xy(end.x, end.y),
            });
            let entity = document.insert_model_entity(index, entity);
            document
                .diagnostics
                .bump_entity(entity.geometry.type_name());
            document.diagnostics.object_count = document.diagnostics.object_count.saturating_add(1);
            if let Some(extents) = document.diagnostics.extents.as_mut() {
                extents.include(start);
                extents.include(end);
            } else {
                document.diagnostics.extents = Extents2::from_points([start, end]);
            }
            (
                Edit::Insert {
                    index,
                    entity: entity.clone(),
                },
                entity,
            )
        };
        self.history.record(edit);
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let display = Arc::make_mut(&mut self.display);
        let appended = display.append_entity(document, &entity);
        Arc::make_mut(&mut self.snaps).append_entity(document, &entity);
        self.gpu_upload = match appended {
            Some(range) => GpuUpload::Append {
                line_start: range.line_start,
                fill_start: range.fill_start,
            },
            None => GpuUpload::Full,
        };
        self.display_generation = self.display_generation.wrapping_add(1);
        self.status = "LINE segment added".into();
    }

    fn report_distance(&mut self, report: DistanceReport) {
        let units = self
            .document
            .as_ref()
            .map(|document| document.units.label())
            .unwrap_or("drawing units");
        self.status = format!(
            "Distance {:.4} {units}  ΔX {:.4}  ΔY {:.4}  Angle {:.2}°",
            report.distance,
            report.delta_x,
            report.delta_y,
            report.angle.to_degrees()
        );
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

    pub(crate) fn show_viewport(&mut self, ui: &mut Ui) {
        let rect = ui.available_rect_before_wrap();
        self.poll_load(rect);
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
        crate::drafting::paint_overlay(
            &painter,
            rect,
            self.camera,
            self.command.preview(self.drafting.current_point),
            self.command
                .requests_point()
                .then_some(self.drafting.acquired_snap)
                .flatten(),
        );
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

    fn show_discard_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending_discard.as_ref() else {
            return;
        };
        let title = match pending {
            PendingDiscard::Quit => "Quit without saving?",
            PendingDiscard::OpenDialog | PendingDiscard::Open(_) => "Discard unsaved changes?",
        };
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("MyCad cannot write DWG files yet. In-memory edits will be lost.");
                ui.label(title);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Discard").clicked() {
                        confirm = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            self.pending_discard = None;
        } else if confirm {
            let pending = self.pending_discard.take();
            self.history.clear();
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
}

impl eframe::App for MyCadApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.input_consumed_escape = false;
        if ctx.input(|input| input.viewport().close_requested()) && self.is_dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_discard = Some(PendingDiscard::Quit);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.window_title()));
        if self.capture.is_none()
            && !self.show_settings
            && self.pending_discard.is_none()
            && !ctx.wants_keyboard_input()
        {
            let keys = ctx.input(|input| {
                let ctrl = input.modifiers.ctrl || input.modifiers.command;
                let plain = !ctrl && !input.modifiers.alt && !input.modifiers.shift;
                KeyChord {
                    f3: input.key_pressed(egui::Key::F3),
                    f8: input.key_pressed(egui::Key::F8),
                    line: plain && input.key_pressed(egui::Key::L),
                    distance: plain && input.key_pressed(egui::Key::D),
                    enter: input.key_pressed(egui::Key::Enter),
                    escape: input.key_pressed(egui::Key::Escape),
                    undo: ctrl && !input.modifiers.shift && input.key_pressed(egui::Key::Z),
                    redo: ctrl
                        && (input.key_pressed(egui::Key::Y)
                            || (input.modifiers.shift && input.key_pressed(egui::Key::Z))),
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
            } else if keys.escape && self.command.is_active() {
                self.cancel_command();
                self.input_consumed_escape = true;
            } else if keys.enter && self.command.is_active() {
                self.finish_command();
            } else if keys.line && !self.command.is_active() {
                self.start_line_command();
            } else if keys.distance && !self.command.is_active() {
                self.start_distance_command();
            }
        }
        if self.load_rx.is_some() || self.capture.is_some() {
            ctx.request_repaint();
        }

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.heading("MyCad");
                ui.separator();
                ui.menu_button("File", |ui| {
                    if ui.button("Open…").clicked() {
                        ui.close();
                        self.open_dialog();
                    }
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
                    if ui
                        .add_enabled(!self.command.is_active(), egui::Button::new("Line    L"))
                        .clicked()
                    {
                        ui.close();
                        self.start_line_command();
                    }
                    if ui
                        .add_enabled(
                            !self.command.is_active(),
                            egui::Button::new("Distance    D"),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.start_distance_command();
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
                    if ui.button("Reset layout").clicked() {
                        ui.close();
                        self.dock_state = workspace::default_dock_state();
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
                ui.label(self.command.prompt()).on_hover_text(&self.status);
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

        self.show_discard_dialog(ctx);

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

fn handle_viewport_input(app: &mut MyCadApp, ui: &Ui, response: &egui::Response, rect: Rect) {
    let origin = Point2::new(rect.min.x as f64, rect.min.y as f64);
    let size = Point2::new(rect.width() as f64, rect.height() as f64);
    let bindings = app.settings.bindings.clone();
    let modifiers = ui.input(|i| i.modifiers);
    let typing = ui.ctx().wants_keyboard_input();

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
            let point = app.drafting.resolve_point(
                raw,
                app.command.base_point(),
                modifiers.shift,
                &app.camera,
                size.y,
                &app.snaps,
            );
            app.cursor_world = Some(point);
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

    if app.command.requests_point() {
        if response.clicked_by(PointerButton::Secondary) {
            app.finish_command();
        } else if response.clicked_by(PointerButton::Primary) {
            if let Some(point) = app.drafting.current_point {
                match app.command.accept_point(point) {
                    Some(CommandOutput::LineSegment(segment)) => {
                        app.commit_line_segment(segment);
                    }
                    Some(CommandOutput::Distance(report)) => {
                        app.report_distance(report);
                    }
                    None => {}
                }
                app.drafting.command_base_point = app.command.base_point();
            }
        }
    } else if !box_active {
        for button in [
            PointerButton::Primary,
            PointerButton::Middle,
            PointerButton::Secondary,
        ] {
            if response.double_clicked_by(button)
                && bindings.double_clicked(InputAction::ZoomExtents, button, modifiers)
            {
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
            if bindings.clicked(InputAction::SelectToggle, button, modifiers) {
                if let Some(Some(id)) = world_hit {
                    app.selection.toggle(id);
                }
            } else if bindings.clicked(InputAction::SelectReplace, button, modifiers) {
                match world_hit {
                    Some(Some(id)) => app.selection.replace(id),
                    _ => app.selection.clear(),
                }
            }
        }
    }

    if !box_active {
        for button in [
            PointerButton::Primary,
            PointerButton::Middle,
            PointerButton::Secondary,
        ] {
            if response.dragged_by(button) && bindings.dragged(InputAction::Pan, button, modifiers)
            {
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
            let toggle =
                bindings.selects_with_pointer(InputAction::SelectToggle, button, modifiers);
            let replace =
                bindings.selects_with_pointer(InputAction::SelectReplace, button, modifiers);
            if !toggle && !replace {
                continue;
            }
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
                toggle,
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
            app.selection.commit_box(&drag.candidates, drag.toggle);
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
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
