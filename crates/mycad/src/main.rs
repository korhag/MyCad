//! MyCad — Linux-first 2D CAD application (Milestone 1: DWG viewer).

mod app;
mod audit;
mod commands;
mod context_menu;
mod diagnostics;
mod drafting;
mod dynamic_input;
mod history;
mod home;
mod measurement;
mod input;
mod preview;
mod properties;
mod ribbon;
mod selection;
mod settings;
mod settings_ui;
mod theme;
mod workspace;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let import_only = args.iter().any(|a| a == "--import-only");
    let path = args
        .into_iter()
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from);

    if import_only {
        return run_import_only(path);
    }

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([800.0, 560.0])
            .with_title("MyCad"),
        ..Default::default()
    };

    let result = eframe::run_native(
        "MyCad",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::MyCadApp::new(cc, path)))),
    );

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("MyCad failed to start: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_import_only(path: Option<PathBuf>) -> ExitCode {
    let Some(path) = path else {
        eprintln!("Usage: mycad --import-only <file.dwg>");
        return ExitCode::FAILURE;
    };
    match dwg_import::import_dwg(&path) {
        Ok(doc) => {
            let d = &doc.diagnostics;
            println!("file: {}", doc.file_name());
            println!("version: {}", d.dwg_version);
            println!("layers: {}", d.layer_count);
            println!("blocks: {}", d.block_count);
            println!("objects: {}", d.object_count);
            println!("entities: {}", d.entity_total());
            println!("unsupported: {}", d.unsupported_total());
            println!("import_s: {:.3}", d.import_time.as_secs_f64());
            if let Some(e) = d.extents {
                println!(
                    "extents: {:.6},{:.6} {:.6},{:.6}",
                    e.min.x, e.min.y, e.max.x, e.max.y
                );
            }
            for (name, count) in &d.entity_counts {
                println!("entity {name} {count}");
            }
            for (name, count) in &d.unsupported_counts {
                println!("unsupported {name} {count}");
            }
            for warning in &d.warnings {
                println!("warning: {warning}");
            }
            let prepare = std::time::Instant::now();
            let display = cad_render::tessellate_document(&doc);
            println!("render_prepare_s: {:.3}", prepare.elapsed().as_secs_f64());
            println!("line_segments: {}", display.line_count());
            println!("triangle_vertices: {}", display.triangle_vertices.len());
            println!("triangles: {}", display.triangle_vertices.len() / 3);
            audit::print_display_list_audit(&display);
            audit::print_geometry_audit(&doc);
            audit::print_linetype_audit(&doc);
            if let Some(extents) = doc.diagnostics.extents {
                let preview_path = std::path::Path::new("test-data").join("MyCad-preview.ppm");
                if let Err(err) =
                    preview::write_preview_ppm(&preview_path, &display, extents, 1600, 1000)
                {
                    eprintln!("preview write failed: {err}");
                } else {
                    println!("preview: {}", preview_path.display());
                }
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("import failed: {err}");
            ExitCode::FAILURE
        }
    }
}
