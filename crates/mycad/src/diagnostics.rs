//! Diagnostics side panel extracted from the application chrome.

use std::path::Path;

use eframe::egui::{self, Ui};

use crate::app::MyCadApp;

pub fn show(ui: &mut Ui, app: &MyCadApp) {
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
