//! MyCad — Linux-first 2D CAD application (Milestone 1: DWG viewer).

mod app;
mod audit;
mod block_edit;
mod block_prefetch;
mod blocks;
mod command_line;
mod commands;
mod config_form;
mod context_menu;
mod diagnostics;
mod drafting;
mod dynamic_block;
mod dynamic_input;
mod edit_tools;
mod history;
mod home;
mod input;
mod measurement;
mod perf_baseline;
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

// Printed when the window fails to open so a graphics problem is actionable
// on any machine, including a VM or a Remote Desktop session.
const GRAPHICS_STARTUP_HINT: &str = "\
If the window did not open, update the GPU driver or set WGPU_BACKEND to dx12, vulkan, or gl.";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let import_only = args.iter().any(|a| a == "--import-only");
    let perf_baseline = args.iter().any(|a| a == "--perf-baseline");
    let path = args
        .into_iter()
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from);

    if import_only {
        return run_import_only(path);
    }
    if perf_baseline {
        return perf_baseline::run_from_path(
            path.unwrap_or_else(perf_baseline::default_sample_path),
        );
    }

    install_panic_hook();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([800.0, 560.0])
            .with_title("MyCad"),
        multisampling: cad_render::VIEWPORT_MSAA_SAMPLES as u16,
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
            eprintln!("{GRAPHICS_STARTUP_HINT}");
            ExitCode::FAILURE
        }
    }
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = panic_message(info);
        let location = info
            .location()
            .map(|place| format!("{}:{}:{}", place.file(), place.line(), place.column()))
            .unwrap_or_else(|| "unknown location".into());
        let backtrace = std::backtrace::Backtrace::force_capture();
        let body = format!(
            "MyCad {}\n{message}\n{location}\n\n{backtrace}\n",
            env!("CARGO_PKG_VERSION")
        );
        let path = crash_log_path();
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")));
        let _ = std::fs::write(&path, body);
        let _ = rfd::MessageDialog::new()
            .set_title("MyCad stopped unexpectedly")
            .set_level(rfd::MessageLevel::Error)
            .set_description(format!(
                "MyCad hit an unexpected error.\n\nA report was saved to:\n{}",
                path.display()
            ))
            .show();
        previous(info);
    }));
}

fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    if let Some(message) = info.payload().downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = info.payload().downcast_ref::<String>() {
        message.clone()
    } else {
        "unexpected error".into()
    }
}

fn crash_log_path() -> PathBuf {
    eframe::storage_dir("MyCad")
        .unwrap_or_else(std::env::temp_dir)
        .join("crash.log")
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
