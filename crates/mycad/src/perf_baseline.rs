//! CPU-side performance baseline for large drawings.
//!
//! Times the operations that freeze the UI today so later phases have
//! numbers to beat. Interactive Move/Copy/Rotate/Erase/Undo/Redo all
//! currently end in a full `refresh_derived`; that combined cost is the
//! proxy for those commands.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use cad_core::perf::threshold_label;
use cad_core::{BlockTreeIndex, Document, EntityId, Geometry, MeasureIndex, Point2, SnapIndex};
use cad_io::{export_pdf, write_dxf, DxfExportOptions, PdfExportOptions};
use cad_render::{
    tessellate_document, tessellate_document_for_block_edit, BlockEditView, BlockEditViewFrame,
    DisplayList,
};
use cad_viewport::Camera2;
use dwg_import::write_dwg;

use crate::selection::{box_pick_entities_into, pick_entity};

const VIEW_WIDTH: f64 = 1440.0;
const VIEW_HEIGHT: f64 = 900.0;
pub const DEFAULT_SAMPLE: &str = "samples/KD-1413-260825 Assir Poultry Internal Logistics.dwg";

// ------------------------------------------------------------
// Type: Sample
// Purpose: One named Instant measurement from the baseline suite.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Sample {
    pub name: &'static str,
    pub elapsed: Duration,
    pub detail: Option<String>,
}

impl Sample {
    fn new(name: &'static str, elapsed: Duration) -> Self {
        Self {
            name,
            elapsed,
            detail: None,
        }
    }

    fn with_detail(name: &'static str, elapsed: Duration, detail: String) -> Self {
        Self {
            name,
            elapsed,
            detail: Some(detail),
        }
    }

    pub fn millis(&self) -> f64 {
        self.elapsed.as_secs_f64() * 1000.0
    }
}

// ------------------------------------------------------------
// Type: BaselineReport
// Purpose: Collected CPU timings plus drawing identity.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct BaselineReport {
    pub file: String,
    pub entities: u64,
    pub samples: Vec<Sample>,
}

impl BaselineReport {
    pub fn print(&self) {
        println!("file: {}", self.file);
        println!("entities: {}", self.entities);
        println!();
        println!(
            "{:<48} {:>12} {:>10}  {}",
            "operation", "ms", "budget", "note"
        );
        println!("{}", "-".repeat(90));
        for sample in &self.samples {
            let budget = threshold_label(sample.elapsed).unwrap_or("ok");
            let note = sample.detail.as_deref().unwrap_or("");
            println!(
                "{:<48} {:>12.2} {:>10}  {}",
                sample.name,
                sample.millis(),
                budget,
                note
            );
        }
        println!();
        println!("pointer/input target: <16ms preferred, <33ms acceptable");
        println!("routine edits must not freeze the UI for hundreds of milliseconds");
    }
}

// ------------------------------------------------------------
// Function: run_cpu_baseline
// Purpose: Time derived caches, picking, overlays, lookups, and optional IO.
// ------------------------------------------------------------
pub fn run_cpu_baseline(document: &Document, io: bool) -> BaselineReport {
    let mut samples = Vec::new();
    let extents = timed("compute_extents", &mut samples, || {
        document.compute_extents()
    });
    let display = timed("tessellate_document (open)", &mut samples, || {
        tessellate_document(document)
    });
    timed("SnapIndex::build (open)", &mut samples, || {
        SnapIndex::build(document)
    });
    timed("MeasureIndex::build (open)", &mut samples, || {
        MeasureIndex::build(document)
    });
    timed("BlockTreeIndex::build (open)", &mut samples, || {
        BlockTreeIndex::build(document)
    });

    let clone_elapsed = {
        let start = Instant::now();
        let _clone = document.clone();
        start.elapsed()
    };
    samples.push(Sample::with_detail(
        "Document::clone",
        clone_elapsed,
        format!(
            "{}",
            if clone_elapsed.as_secs_f64() * 1000.0 > 16.0 {
                "exceeds frame budget — Phase 2 must not clone on the UI thread"
            } else {
                "cheap enough to snapshot on the UI thread"
            }
        ),
    ));

    time_interaction(&display, extents, document, &mut samples);
    time_refresh_derived_proxy(document, &mut samples);
    time_block_edit(document, &mut samples);

    if io {
        time_save_export(document, &mut samples);
    }

    BaselineReport {
        file: document.file_name(),
        entities: document
            .diagnostics
            .entity_total()
            .max(document.model_space.len() as u64),
        samples,
    }
}

fn time_interaction(
    display: &DisplayList,
    extents: Option<cad_core::Extents2>,
    document: &Document,
    samples: &mut Vec<Sample>,
) {
    let mut camera = Camera2::default();
    if let Some(extents) = extents.or(document.diagnostics.extents) {
        camera.zoom_extents(extents, VIEW_WIDTH, VIEW_HEIGHT);
    }
    let origin = Point2::new(0.0, 0.0);
    let size = Point2::new(VIEW_WIDTH, VIEW_HEIGHT);

    let pick_id = display
        .picks
        .iter()
        .find(|pick| !pick.is_empty())
        .map(|pick| pick.entity_id);
    if let Some(id) = pick_id {
        let pick = display
            .picks
            .iter()
            .find(|pick| pick.entity_id == id)
            .expect("pick id from display");
        let world = pick.bounds.center();
        let screen = camera.world_to_screen(world, origin, size);
        timed("click_picking", samples, || {
            pick_entity(display, &camera, screen, origin, size);
        });

        let overlay_ids = display
            .picks
            .iter()
            .filter(|pick| !pick.is_empty())
            .map(|pick| pick.entity_id)
            .take(64)
            .collect::<Vec<_>>();
        timed("overlay_batches (1)", samples, || {
            display.overlay_batches(&[id]);
        });
        timed("overlay_batches (upto 64)", samples, || {
            display.overlay_batches(&overlay_ids);
        });
    } else {
        samples.push(Sample::with_detail(
            "click_picking",
            Duration::ZERO,
            "no pickable entities".into(),
        ));
    }

    let (start, current) = if let Some(extents) = extents.or(document.diagnostics.extents) {
        let a = camera.world_to_screen(extents.min, origin, size);
        let b = camera.world_to_screen(extents.center(), origin, size);
        (a, b)
    } else {
        (Point2::new(80.0, 80.0), Point2::new(420.0, 320.0))
    };
    timed("box_selection (marquee)", samples, || {
        let mut candidates = Vec::new();
        box_pick_entities_into(
            display,
            &camera,
            start,
            current,
            origin,
            size,
            &mut candidates,
        );
        candidates.len()
    });

    let ids: Vec<EntityId> = document
        .model_space
        .iter()
        .map(|entity| entity.id)
        .collect();
    if ids.is_empty() {
        return;
    }
    timed("entity_by_id (1)", samples, || {
        document.entity_by_id(ids[0]);
    });
    timed("entity_by_id (50)", samples, || {
        for id in ids.iter().cycle().take(50) {
            document.entity_by_id(*id);
        }
    });
    timed("entity_by_id (1000)", samples, || {
        for id in ids.iter().cycle().take(1000) {
            document.entity_by_id(*id);
        }
    });
}

fn time_refresh_derived_proxy(document: &Document, samples: &mut Vec<Sample>) {
    timed("refresh_derived (Move/Undo/Erase proxy)", samples, || {
        let _extents = document.compute_extents();
        let _display = tessellate_document(document);
        let _snaps = SnapIndex::build(document);
        let _measures = MeasureIndex::build(document);
        let _tree = BlockTreeIndex::build(document);
    });
}

fn time_block_edit(document: &Document, samples: &mut Vec<Sample>) {
    let Some(entity) = document
        .model_space
        .iter()
        .find(|entity| matches!(entity.geometry, Geometry::Insert { .. }))
    else {
        samples.push(Sample::with_detail(
            "tessellate_document_for_block_edit",
            Duration::ZERO,
            "no INSERT in model space".into(),
        ));
        return;
    };
    let Geometry::Insert { block_name, .. } = &entity.geometry else {
        return;
    };
    let view = BlockEditView {
        frames: vec![BlockEditViewFrame {
            instance_id: entity.id,
            block_name: block_name.clone(),
        }],
    };
    timed("tessellate_document_for_block_edit", samples, || {
        tessellate_document_for_block_edit(document, &view);
    });
}

fn time_save_export(document: &Document, samples: &mut Vec<Sample>) {
    let dir = std::env::temp_dir();
    let stem = format!("mycad-perf-{}", std::process::id());
    let dxf = dir.join(format!("{stem}.dxf"));
    let dwg = dir.join(format!("{stem}.dwg"));
    let pdf = dir.join(format!("{stem}.pdf"));

    time_io("write_dxf", samples, &dxf, || {
        write_dxf(document, &dxf, &DxfExportOptions::default())
            .map(|_| ())
            .map_err(|err| err.to_string())
    });
    time_io("write_dwg", samples, &dwg, || {
        write_dwg(document, &dwg)
            .map(|_| ())
            .map_err(|err| err.to_string())
    });
    time_io("export_pdf", samples, &pdf, || {
        export_pdf(document, &pdf, &PdfExportOptions::default())
            .map(|_| ())
            .map_err(|err| err.to_string())
    });
}

fn time_io(
    name: &'static str,
    samples: &mut Vec<Sample>,
    path: &Path,
    write: impl FnOnce() -> Result<(), String>,
) {
    let start = Instant::now();
    let result = write();
    let elapsed = start.elapsed();
    let _ = std::fs::remove_file(path);
    match result {
        Ok(()) => samples.push(Sample::new(name, elapsed)),
        Err(err) => samples.push(Sample::with_detail(name, elapsed, format!("failed: {err}"))),
    }
}

fn timed<T>(name: &'static str, samples: &mut Vec<Sample>, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let value = f();
    samples.push(Sample::new(name, start.elapsed()));
    value
}

// ------------------------------------------------------------
// Function: run_from_path
// Purpose: Import a DWG then run the CPU baseline, including save/export.
// ------------------------------------------------------------
pub fn run_from_path(path: PathBuf) -> ExitCode {
    if !path.exists() {
        eprintln!("perf-baseline: file not found: {}", path.display());
        return ExitCode::FAILURE;
    }
    println!("importing {}…", path.display());
    let imported = Instant::now();
    let document = match dwg_import::import_dwg(&path) {
        Ok(document) => document,
        Err(err) => {
            eprintln!("perf-baseline: import failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    let import_elapsed = imported.elapsed();
    let mut report = run_cpu_baseline(&document, true);
    report.file = path.display().to_string();
    report.samples.insert(
        0,
        Sample::with_detail(
            "import_dwg (open)",
            import_elapsed,
            format!(
                "worker thread today; import_s={:.3}",
                import_elapsed.as_secs_f64()
            ),
        ),
    );
    report.print();
    ExitCode::SUCCESS
}

pub fn default_sample_path() -> PathBuf {
    PathBuf::from(DEFAULT_SAMPLE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_core::primitives_document;

    #[test]
    fn cpu_baseline_covers_core_operations_on_primitives() {
        let document = primitives_document();
        let report = run_cpu_baseline(&document, false);
        let names: Vec<_> = report.samples.iter().map(|sample| sample.name).collect();
        for required in [
            "tessellate_document (open)",
            "SnapIndex::build (open)",
            "MeasureIndex::build (open)",
            "BlockTreeIndex::build (open)",
            "Document::clone",
            "click_picking",
            "box_selection (marquee)",
            "overlay_batches (1)",
            "entity_by_id (1)",
            "refresh_derived (Move/Undo/Erase proxy)",
            "tessellate_document_for_block_edit",
        ] {
            assert!(names.contains(&required), "missing {required} in {names:?}");
        }
        assert!(report
            .samples
            .iter()
            .all(|sample| sample.elapsed < Duration::from_secs(5)));
    }
}
