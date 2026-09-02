//! Application chrome: menus, viewport interaction, diagnostics, loading.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use cad_core::{Document, Point2};
use cad_render::{tessellate_document, CadFrame, CadGpu, DisplayList};
use cad_viewport::Camera2;
use dwg_import::ImportError;
use eframe::egui::{self, PointerButton, Rect, Ui};

use crate::theme;

enum LoadMsg {
    Success {
        document: Document,
        display: DisplayList,
    },
    Failure {
        path: PathBuf,
        message: String,
    },
}

// ------------------------------------------------------------
// Type: MyCadApp
// Purpose: Native GUI shell around the CAD document and wgpu viewport.
// ------------------------------------------------------------
pub struct MyCadApp {
    camera: Camera2,
    document: Option<Arc<Document>>,
    display: Arc<DisplayList>,
    display_generation: u64,
    load_rx: Option<Receiver<LoadMsg>>,
    loading_path: Option<PathBuf>,
    status: String,
    error: Option<String>,
    show_diagnostics: bool,
    pending_open: Option<PathBuf>,
    last_pointer: Option<egui::Pos2>,
}

impl MyCadApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_path: Option<PathBuf>) -> Self {
        theme::apply(&cc.egui_ctx);
        if let Some(render_state) = &cc.wgpu_render_state {
            let mut renderer = render_state.renderer.write();
            renderer
                .callback_resources
                .insert(CadGpu::new(&render_state.device, render_state.target_format));
        }
        let mut app = Self {
            camera: Camera2::default(),
            document: None,
            display: Arc::new(DisplayList::default()),
            display_generation: 0,
            load_rx: None,
            loading_path: None,
            status: "Ready".to_string(),
            error: None,
            show_diagnostics: true,
            pending_open: initial_path,
            last_pointer: None,
        };
        if let Some(path) = app.pending_open.take() {
            app.start_load(path);
        }
        app
    }

    fn start_load(&mut self, path: PathBuf) {
        if self.load_rx.is_some() {
            self.pending_open = Some(path);
            return;
        }
        self.error = None;
        self.loading_path = Some(path.clone());
        self.status = format!("Loading {}…", file_name(&path));
        let (tx, rx) = mpsc::channel();
        self.load_rx = Some(rx);
        thread::Builder::new()
            .name("mycad-dwg-import".into())
            .spawn(move || {
                match dwg_import::import_dwg(&path) {
                    Ok(mut document) => {
                        let prepare = Instant::now();
                        let display = tessellate_document(&document);
                        document.diagnostics.render_prepare_time = prepare.elapsed();
                        let _ = tx.send(LoadMsg::Success {
                            document,
                            display,
                        });
                    }
                    Err(err) => {
                        let _ = tx.send(LoadMsg::Failure {
                            path,
                            message: format_import_error(&err),
                        });
                    }
                }
            })
            .expect("import thread");
    }

    fn poll_load(&mut self, viewport: Rect) {
        let Some(rx) = self.load_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(LoadMsg::Success { document, display }) => {
                if let Some(extents) = document.diagnostics.extents.or_else(|| document.compute_extents())
                {
                    self.camera
                        .zoom_extents(extents, viewport.width() as f64, viewport.height() as f64);
                }
                self.status = format!(
                    "Loaded {}  •  {} entities  •  {} unsupported",
                    document.file_name(),
                    document.diagnostics.entity_total(),
                    document.diagnostics.unsupported_total()
                );
                self.document = Some(Arc::new(document));
                self.display = Arc::new(display);
                self.display_generation = self.display_generation.wrapping_add(1);
                self.loading_path = None;
                self.load_rx = None;
                self.error = None;
            }
            Ok(LoadMsg::Failure { path, message }) => {
                self.status = format!("Failed to open {}", file_name(&path));
                self.error = Some(message);
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
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("DWG drawings", &["dwg", "DWG"])
            .add_filter("All files", &["*"])
            .pick_file()
        {
            self.start_load(path);
        }
    }
}

impl eframe::App for MyCadApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.load_rx.is_some() {
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
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui.button("Zoom Extents").clicked() {
                        ui.close();
                        if let Some(doc) = &self.document {
                            if let Some(e) = doc.diagnostics.extents {
                                let size = ctx.available_rect();
                                self.camera.zoom_extents(
                                    e,
                                    size.width() as f64,
                                    size.height() as f64,
                                );
                            }
                        }
                    }
                    ui.checkbox(&mut self.show_diagnostics, "Diagnostics");
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                let name = self
                    .document
                    .as_ref()
                    .map(|d| d.file_name())
                    .or_else(|| self.loading_path.as_ref().map(|p| file_name(p)))
                    .unwrap_or_else(|| "(no file)".into());
                ui.monospace(name);
                ui.separator();
                ui.label(&self.status);
                if let Some(err) = &self.error {
                    ui.separator();
                    ui.colored_label(egui::Color32::from_rgb(220, 120, 90), err);
                }
            });
        });

        if self.show_diagnostics {
            egui::SidePanel::right("diagnostics")
                .default_width(320.0)
                .show(ctx, |ui| {
                    diagnostics_panel(ui, self);
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let rect = ui.available_rect_before_wrap();
            self.poll_load(rect);
            let response = ui.interact(rect, ui.id().with("cad-viewport"), egui::Sense::click_and_drag());
            handle_navigation(self, ui, &response, rect);

            let painter = ui.painter_at(rect);
            painter.rect_filled(
                rect,
                0.0,
                egui::Color32::from_rgb(10, 14, 12),
            );
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
            painter.add(egui_wgpu::Callback::new_paint_callback(
                rect,
                CadFrame {
                    camera: self.camera,
                    origin: self.display.origin,
                    generation: self.display_generation,
                    display: Arc::clone(&self.display),
                    aspect,
                },
            ));
        });
    }
}

fn handle_navigation(app: &mut MyCadApp, ui: &Ui, response: &egui::Response, rect: Rect) {
    let origin = Point2::new(rect.min.x as f64, rect.min.y as f64);
    let size = Point2::new(rect.width() as f64, rect.height() as f64);

    if response.dragged_by(PointerButton::Middle) {
        if let (Some(prev), Some(now)) = (app.last_pointer, ui.input(|i| i.pointer.latest_pos())) {
            let delta = app.camera.pan_screen(
                Point2::new(prev.x as f64, prev.y as f64),
                Point2::new(now.x as f64, now.y as f64),
                origin,
                size,
            );
            app.camera.pan_world(delta);
        }
    }
    app.last_pointer = ui.input(|i| i.pointer.latest_pos());

    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.0 {
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                let world = app.camera.screen_to_world(
                    Point2::new(pos.x as f64, pos.y as f64),
                    origin,
                    size,
                );
                let factor = 1.001_f64.powf(scroll as f64);
                app.camera.zoom_at(world, factor);
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::E) && i.modifiers.command) {
            if let Some(doc) = &app.document {
                if let Some(e) = doc.diagnostics.extents {
                    app.camera.zoom_extents(e, size.x, size.y);
                }
            }
        }
    }

    if response.double_clicked() {
        if let Some(doc) = &app.document {
            if let Some(e) = doc.diagnostics.extents {
                app.camera.zoom_extents(e, size.x, size.y);
            }
        }
    }
}

fn diagnostics_panel(ui: &mut Ui, app: &MyCadApp) {
    ui.heading("Diagnostics");
    ui.separator();
    let Some(doc) = &app.document else {
        if let Some(path) = &app.loading_path {
            ui.label(format!("Loading {}", file_name(path)));
        } else {
            ui.label("No drawing loaded.");
        }
        if let Some(err) = &app.error {
            ui.add_space(8.0);
            ui.colored_label(egui::Color32::from_rgb(220, 120, 90), err);
        }
        return;
    };
    let d = &doc.diagnostics;
    ui.monospace(format!("DWG version: {}", d.dwg_version));
    ui.monospace(format!("Layers: {}", d.layer_count));
    ui.monospace(format!("Blocks: {}", d.block_count));
    ui.monospace(format!("Objects: {}", d.object_count));
    ui.monospace(format!("Imported entities: {}", d.entity_total()));
    ui.monospace(format!("Unsupported entities: {}", d.unsupported_total()));
    ui.monospace(format!("Import: {:.3}s", d.import_time.as_secs_f64()));
    ui.monospace(format!(
        "Render prepare: {:.3}s",
        d.render_prepare_time.as_secs_f64()
    ));
    if let Some(e) = d.extents {
        ui.monospace(format!(
            "Extents: ({:.3}, {:.3}) – ({:.3}, {:.3})",
            e.min.x, e.min.y, e.max.x, e.max.y
        ));
        ui.monospace(format!("Size: {:.3} × {:.3}", e.width(), e.height()));
    }
    ui.add_space(8.0);
    ui.label("Entity counts");
    egui::ScrollArea::vertical()
        .id_salt("entity-counts")
        .max_height(220.0)
        .show(ui, |ui| {
            for (name, count) in &d.entity_counts {
                ui.monospace(format!("{name:>24}  {count}"));
            }
        });
    ui.add_space(8.0);
    ui.label("Unsupported (reported, not discarded silently)");
    if d.unsupported_counts.is_empty() {
        ui.weak("None");
    } else {
        for (name, count) in &d.unsupported_counts {
            ui.monospace(format!("{name:>24}  {count}"));
        }
    }
    if !d.warnings.is_empty() {
        ui.add_space(8.0);
        ui.label("Warnings");
        for w in &d.warnings {
            ui.weak(w);
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
