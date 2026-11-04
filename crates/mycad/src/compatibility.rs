//! Plain-language compatibility information built alongside background previews.
//!
//! Counts describe what EntoCAD has loaded, not an independent AutoCAD audit.

use std::collections::BTreeSet;
use std::path::Path;

use cad_core::{is_user_editable_block_name, Document};
use cad_io::{
    apply_companion_for_drawing, read_drawing_companion, CadFileFormat, CompanionReport, SaveReport,
};
use eframe::egui::{self, Color32, RichText, Ui};

const WARNING_COLOR: Color32 = Color32::from_rgb(235, 183, 105);
/// Keep imported diagnostic data and the corresponding UI bounded.
const MAX_DETAIL_ROWS: usize = 512;
const MAX_DETAIL_CHARACTERS: usize = 2_048;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum HelperStatus {
    #[default]
    Unsaved,
    Embedded,
    Missing,
    NotRequired,
    Current {
        restored: usize,
    },
    Stale {
        reason: String,
    },
    Partial {
        restored: usize,
        skipped: usize,
        reason: String,
    },
    Unavailable {
        reason: String,
    },
}

impl HelperStatus {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Unsaved => "Not saved yet",
            Self::Embedded => "Included in native file",
            Self::Missing => "Missing",
            Self::NotRequired => "Not required",
            Self::Current { .. } => "Verified when opened or saved",
            Self::Stale { .. } => "Stale or unverified",
            Self::Partial { .. } => "Partially restored",
            Self::Unavailable { .. } => "Unavailable",
        }
    }

    fn needs_attention(&self) -> bool {
        matches!(
            self,
            Self::Stale { .. } | Self::Partial { .. } | Self::Unavailable { .. }
        )
    }

    fn explanation(&self) -> &str {
        match self {
            Self::Unsaved => "Save the drawing as DWG/DXF. Its geometry is stored in the drawing and EntoCAD behavior in a companion helper.",
            Self::Embedded => "EntoCAD behavior is stored inside the native drawing. No separate helper is needed.",
            Self::Missing => "No EntoCAD helper was found. The drawing opens using its own geometry and any supported AutoCAD controls.",
            Self::NotRequired => "This saved drawing has no EntoCAD dynamic behavior that requires a helper.",
            Self::Current { .. } => "The helper matched the drawing bytes at the last open or save. Keep both files together to retain EntoCAD behavior.",
            Self::Stale { reason } | Self::Partial { reason, .. } | Self::Unavailable { reason } => reason,
        }
    }

    pub(crate) fn from_report(report: &CompanionReport) -> Self {
        if let Some(reason) = &report.warning {
            if reason.starts_with("Drawing changed since its companion was saved")
                || reason.starts_with("Legacy or unbound companion was not applied")
            {
                return Self::Stale {
                    reason: bounded_text(reason),
                };
            }
            if report.restored == 0 && report.skipped == 0 {
                return Self::Unavailable {
                    reason: bounded_text(reason),
                };
            }
            return Self::Partial {
                restored: report.restored,
                skipped: report.skipped,
                reason: bounded_text(reason),
            };
        }
        if report.skipped > 0 {
            return Self::Partial {
                restored: report.restored,
                skipped: report.skipped,
                reason: "Some blocks no longer match their helper data. Their drawing geometry was kept without restoring that behavior.".into(),
            };
        }
        Self::Current {
            restored: report.restored,
        }
    }

    pub(crate) fn after_save(path: &Path, has_dynamic_content: bool, report: &SaveReport) -> Self {
        if CadFileFormat::from_path(path).is_some_and(CadFileFormat::is_native) {
            return Self::Embedded;
        }
        if let Some(warning) = report
            .warnings
            .iter()
            .find(|line| line.contains("companion file"))
        {
            return Self::Unavailable {
                reason: bounded_text(warning),
            };
        }
        if has_dynamic_content {
            Self::Current { restored: 0 }
        } else {
            Self::NotRequired
        }
    }
}

/// Run on the opening worker: all helper file I/O and verification stay there.
pub(crate) fn apply_open_companion(
    path: &Path,
    document: &mut Document,
) -> (Option<String>, HelperStatus) {
    if CadFileFormat::from_path(path).is_some_and(CadFileFormat::is_native) {
        return (None, HelperStatus::Embedded);
    }
    match read_drawing_companion(path) {
        Ok(Some(companion)) => match apply_companion_for_drawing(path, document, companion) {
            Ok(report) => {
                if let Some(warning) = &report.warning {
                    document
                        .diagnostics
                        .note_lossy(format!("Companion file was not fully applied: {warning}"));
                }
                (report.status_line(), HelperStatus::from_report(&report))
            }
            Err(err) => helper_failure(document, err.to_string()),
        },
        Ok(None) => (None, HelperStatus::Missing),
        Err(err) => helper_failure(document, err.to_string()),
    }
}

fn helper_failure(document: &mut Document, error: String) -> (Option<String>, HelperStatus) {
    let warning = format!("Companion file was not applied: {error}");
    document.diagnostics.note_lossy(warning.clone());
    (
        Some(warning),
        HelperStatus::Unavailable {
            reason: bounded_text(&error),
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BlockStatus {
    Editable,
    ImportedEditable,
    ImportedStatic,
}

impl BlockStatus {
    fn label(&self) -> &'static str {
        match self {
            Self::Editable => "EntoCAD controls",
            Self::ImportedEditable => "Supported AutoCAD controls",
            Self::ImportedStatic => "Static geometry",
        }
    }
}

#[derive(Debug, Clone)]
struct BlockSummary {
    name: String,
    status: BlockStatus,
    detail: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CompatibilitySummary {
    pub(crate) generation: u64,
    editable_definitions: usize,
    ordinary_definitions: usize,
    imported_editable_references: usize,
    imported_static_items: usize,
    unsupported_entities: u64,
    unsupported_classes: usize,
    has_save_loss: bool,
    blocks: Vec<BlockSummary>,
    blocks_truncated: bool,
    unsupported: Vec<(String, u64)>,
    warnings: Vec<String>,
    details_truncated: bool,
}

impl CompatibilitySummary {
    /// Called by load/preview workers, never by the panel's per-frame paint.
    pub(crate) fn from_document(document: &Document) -> Self {
        let diagnostics = &document.diagnostics;
        let mut summary = Self {
            generation: document.content_generation(),
            imported_editable_references: diagnostics.dynamic_editable,
            imported_static_items: diagnostics.dynamic_kept_static,
            unsupported_entities: diagnostics.unsupported_total(),
            unsupported_classes: diagnostics.unhandled_classes.len(),
            has_save_loss: diagnostics.has_save_loss(),
            ..Default::default()
        };
        // Import notes identify definitions whose controls were translated. A
        // missing provenance note never becomes a claim of AutoCAD support.
        let editable_imports: BTreeSet<&str> = diagnostics
            .dynamic_lines
            .iter()
            .filter_map(|line| line.strip_prefix("dynamic "))
            .filter_map(|line| line.split_once(": ").map(|(name, _)| name))
            .collect();
        for block in document.blocks.values() {
            if !is_user_editable_block_name(&block.name) {
                continue;
            }
            if block.is_dynamic() {
                summary.editable_definitions += 1;
                summary.push_block(BlockSummary {
                    name: block.name.clone(),
                    status: if editable_imports.contains(block.name.as_str()) {
                        BlockStatus::ImportedEditable
                    } else {
                        BlockStatus::Editable
                    },
                    detail: None,
                });
            } else {
                summary.ordinary_definitions += 1;
            }
        }
        // Kept-static diagnostics can name anonymous definitions too. Only
        // offer a reveal link if that exact definition is present.
        for line in &diagnostics.dynamic_lines {
            let Some((identity, reason)) = line
                .strip_prefix("dynamic ")
                .and_then(|line| line.split_once(" kept static ("))
            else {
                continue;
            };
            let name = identity
                .split_once(" -> ")
                .map_or(identity, |(name, _)| name);
            if let Some(block) = document.block_by_name(name) {
                if !block.is_dynamic() {
                    summary.push_block(BlockSummary {
                        name: block.name.clone(),
                        status: BlockStatus::ImportedStatic,
                        detail: Some(bounded_text(reason.trim_end_matches(')'))),
                    });
                }
            }
        }
        for (name, count) in diagnostics
            .unsupported_counts
            .iter()
            .chain(&diagnostics.unhandled_classes)
        {
            if summary.unsupported.len() < MAX_DETAIL_ROWS {
                summary.unsupported.push((bounded_text(name), *count));
            } else {
                summary.details_truncated = true;
            }
        }
        for warning in diagnostics.lossy_notes.iter().chain(&diagnostics.warnings) {
            let warning = bounded_text(warning);
            if summary.warnings.contains(&warning) {
                continue;
            }
            if summary.warnings.len() < MAX_DETAIL_ROWS {
                summary.warnings.push(warning);
            } else {
                summary.details_truncated = true;
            }
        }
        summary
    }

    fn push_block(&mut self, block: BlockSummary) {
        if self.blocks.len() < MAX_DETAIL_ROWS {
            if !self
                .blocks
                .iter()
                .any(|existing| existing.name.eq_ignore_ascii_case(&block.name))
            {
                self.blocks.push(block);
            }
        } else {
            self.blocks_truncated = true;
        }
    }
}

#[derive(Debug)]
pub(crate) struct CompatibilityPanel {
    pub(crate) summary: Option<CompatibilitySummary>,
    pub(crate) helper: HelperStatus,
    export_format: CadFileFormat,
    search: String,
}

impl Default for CompatibilityPanel {
    fn default() -> Self {
        Self {
            summary: None,
            helper: HelperStatus::default(),
            export_format: CadFileFormat::Dwg,
            search: String::new(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CompatibilityAction {
    RevealBlock(String),
    ShowDiagnostics,
    Refresh,
}

pub(crate) fn show(
    ui: &mut Ui,
    panel: &mut CompatibilityPanel,
    current_generation: Option<u64>,
    dirty: bool,
) -> Option<CompatibilityAction> {
    ui.heading("Drawing compatibility");
    ui.weak("EntoCAD behavior and AutoCAD exchange");
    ui.separator();
    if current_generation.is_none() {
        ui.label("Open or create a drawing to inspect its compatibility.");
        return None;
    }
    let mut action = None;
    egui::ScrollArea::vertical().id_salt("compatibility-summary").show(ui, |ui| {
        ui.label(RichText::new("EntoCAD helper").strong());
        let label = RichText::new(panel.helper.label());
        ui.label(if panel.helper.needs_attention() { label.color(WARNING_COLOR) } else { label });
        ui.label(panel.helper.explanation());
        match &panel.helper {
            HelperStatus::Current { restored } if *restored > 0 => { ui.weak(format!("{restored} blocks restored when opened.")); }
            HelperStatus::Partial { restored, skipped, .. } => { ui.weak(format!("{restored} restored; {skipped} left unchanged.")); }
            _ => {}
        }
        if dirty {
            ui.label(RichText::new("Unsaved edits are not yet included in the saved drawing or helper.").color(WARNING_COLOR));
        }
        ui.add_space(10.0);
        let Some(summary) = &panel.summary else {
            ui.label("A compatibility summary has not been collected for this drawing yet.");
            if ui.button("Refresh summary").clicked() { action = Some(CompatibilityAction::Refresh); }
            ui.add_space(10.0);
            ui.label(RichText::new("Before saving or exporting").strong());
            show_export_summary(ui, panel.export_format, None);
            return;
        };
        let stale = Some(summary.generation) != current_generation;
        if stale {
            ui.weak("Last evaluated summary. The drawing has changed since these counts were collected.");
            if ui.button("Refresh summary").clicked() { action = Some(CompatibilityAction::Refresh); }
        }
        ui.label(RichText::new("Loaded behavior").strong());
        ui.label(format!("{} named block definitions with EntoCAD controls", summary.editable_definitions));
        ui.label(format!("{} named ordinary block definitions", summary.ordinary_definitions));
        if summary.imported_editable_references > 0 || summary.imported_static_items > 0 {
            ui.weak(format!("At AutoCAD import: {} references became editable; {} dynamic items were kept static.", summary.imported_editable_references, summary.imported_static_items));
            ui.weak("Import counts describe the original import, not additional blocks to add to the totals above.");
        }
        if summary.has_save_loss {
            ui.label(RichText::new("Some original content cannot be preserved on exchange save.").color(WARNING_COLOR));
        } else {
            ui.label("No known save-loss warning was recorded for the loaded content.");
        }
        ui.weak("This is not an AutoCAD validation or a guarantee of complete compatibility.");
        if summary.unsupported_entities > 0 || summary.unsupported_classes > 0 {
            ui.label(format!("{} unsupported entities; {} unreadable object classes", summary.unsupported_entities, summary.unsupported_classes));
        }
        ui.add_space(10.0);
        ui.collapsing("Affected and editable blocks", |ui| {
            ui.add(egui::TextEdit::singleline(&mut panel.search).hint_text("Find a block"));
            let query = panel.search.trim().to_lowercase();
            let matches: Vec<_> = summary.blocks.iter()
                .filter(|block| block.name.to_lowercase().contains(&query)).collect();
            let row_height = ui.spacing().interact_size.y + ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y;
            egui::ScrollArea::vertical().id_salt("compatibility-blocks").max_height(220.0)
                .show_rows(ui, row_height, matches.len(), |ui, range| {
                    for index in range {
                        let Some(block) = matches.get(index) else { continue; };
                        if ui.add_enabled(!stale && is_user_editable_block_name(&block.name), egui::Button::new(&block.name).truncate())
                            .on_hover_text(format!("{}\nReveal this named definition in the Blocks panel", block.name)).clicked() {
                            action = Some(CompatibilityAction::RevealBlock(block.name.clone()));
                        }
                        let description = block.detail.as_deref().unwrap_or_else(|| block.status.label());
                        ui.add(egui::Label::new(RichText::new(block.status.label()).weak()).truncate()).on_hover_text(description);
                    }
                });
            if matches.is_empty() { ui.weak("No matching affected or dynamic blocks."); }
            if summary.blocks_truncated { ui.weak("Showing the first 512 block entries. Use the Blocks panel for the full drawing."); }
        });
        ui.collapsing("Unsupported content and warnings", |ui| {
            if summary.unsupported.is_empty() && summary.warnings.is_empty() { ui.weak("No warnings recorded."); }
            let rows = summary.unsupported.len() + summary.warnings.len();
            egui::ScrollArea::vertical().id_salt("compatibility-warnings").max_height(220.0)
                .show_rows(ui, ui.text_style_height(&egui::TextStyle::Body), rows, |ui, range| {
                    for index in range {
                        if let Some((name, count)) = summary.unsupported.get(index) {
                            let detail = format!("{count} × {name}");
                            ui.add(egui::Label::new(&detail).truncate()).on_hover_text(detail);
                        } else if let Some(warning) = index.checked_sub(summary.unsupported.len()).and_then(|index| summary.warnings.get(index)) {
                            ui.add(egui::Label::new(warning).truncate()).on_hover_text(warning);
                        }
                    }
                });
            if rows > 0 { ui.weak("Hover over a row to read its details."); }
            if summary.details_truncated { ui.weak("Some details are omitted here; open Diagnostics for the full report."); }
        });
        if ui.button("Open technical diagnostics").clicked() { action = Some(CompatibilityAction::ShowDiagnostics); }
        ui.add_space(10.0);
        ui.separator();
        ui.label(RichText::new("Before saving or exporting").strong());
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut panel.export_format, CadFileFormat::Dwg, "DWG");
            ui.selectable_value(&mut panel.export_format, CadFileFormat::Dxf, "DXF");
        });
        show_export_summary(ui, panel.export_format, Some(summary));
    });
    action
}

pub(crate) fn show_export_summary(
    ui: &mut Ui,
    format: CadFileFormat,
    summary: Option<&CompatibilitySummary>,
) {
    for line in export_summary(format, summary.is_some_and(|summary| summary.has_save_loss)) {
        ui.label(line);
    }
}

fn export_summary(format: CadFileFormat, has_save_loss: bool) -> Vec<&'static str> {
    let mut lines = if format.is_native() {
        vec![
            "Native EntoCAD files keep the loaded geometry, parameters, actions and instance settings together.",
            "AutoCAD does not open the native EntoCAD format. Use DWG/DXF for exchange.",
        ]
    } else {
        vec![
            "DWG/DXF contains the last evaluated block appearance as ordinary geometry.",
            "A separate hidden helper retains EntoCAD parameters and actions. AutoCAD does not read that helper.",
            "AutoCAD's original dynamic controls are not preserved by this writer.",
        ]
    };
    if has_save_loss {
        lines.push("Unsupported original objects cannot be recovered by choosing another save format. Review the warnings and retain the original file.");
    }
    lines
}

fn bounded_text(text: &str) -> String {
    let mut result: String = text.chars().take(MAX_DETAIL_CHARACTERS).collect();
    if text.chars().nth(MAX_DETAIL_CHARACTERS).is_some() {
        result.push('…');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_core::{
        BlockDefinition, DynamicDefinition, NumericParameter, ParameterDef, ParameterId, Point3,
    };

    #[test]
    fn helper_reports_distinguish_verified_stale_partial_and_failed_saves() {
        let mut report = CompanionReport {
            restored: 2,
            skipped: 0,
            warning: None,
        };
        assert_eq!(
            HelperStatus::from_report(&report),
            HelperStatus::Current { restored: 2 }
        );
        report.skipped = 1;
        assert!(matches!(
            HelperStatus::from_report(&report),
            HelperStatus::Partial {
                restored: 2,
                skipped: 1,
                ..
            }
        ));
        report.warning = Some("Drawing changed since its companion was saved; the drawing was opened without helper features".into());
        assert!(matches!(
            HelperStatus::from_report(&report),
            HelperStatus::Stale { .. }
        ));
        report.warning = Some("Legacy or unbound companion was not applied; the drawing was opened without helper features".into());
        assert!(matches!(
            HelperStatus::from_report(&report),
            HelperStatus::Stale { .. }
        ));
        let mut saved = SaveReport::default();
        assert_eq!(
            HelperStatus::after_save(Path::new("a.dwg"), true, &saved),
            HelperStatus::Current { restored: 0 }
        );
        assert_eq!(
            HelperStatus::after_save(Path::new("a.dxf"), false, &saved),
            HelperStatus::NotRequired
        );
        assert_eq!(
            HelperStatus::after_save(Path::new("a.mycad"), true, &saved),
            HelperStatus::Embedded
        );
        saved
            .warnings
            .push("companion file was not written: permission denied".into());
        assert!(matches!(
            HelperStatus::after_save(Path::new("a.dwg"), true, &saved),
            HelperStatus::Unavailable { .. }
        ));
    }

    #[test]
    fn opening_reports_missing_verified_changed_and_unreadable_helpers() {
        let folder = std::env::temp_dir().join(format!(
            "mycad-compatibility-helper-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&folder).unwrap();
        let drawing = folder.join("drawing.dxf");
        std::fs::write(&drawing, b"original saved drawing bytes").unwrap();
        let mut document = Document::default();
        assert_eq!(
            apply_open_companion(&drawing, &mut document),
            (None, HelperStatus::Missing)
        );
        let helper = cad_io::companion_path(&drawing);
        cad_io::write_companion(&helper, &drawing, &document, &document, &[], &[]).unwrap();
        assert_eq!(
            apply_open_companion(&drawing, &mut document),
            (None, HelperStatus::Current { restored: 0 })
        );
        std::fs::write(&drawing, b"externally edited drawing bytes").unwrap();
        let (note, status) = apply_open_companion(&drawing, &mut document);
        assert!(note.is_some());
        assert!(matches!(status, HelperStatus::Stale { .. }));
        assert!(document.diagnostics.has_save_loss());
        std::fs::write(&helper, b"unreadable helper").unwrap();
        assert!(matches!(
            apply_open_companion(&drawing, &mut document).1,
            HelperStatus::Unavailable { .. }
        ));
        assert_eq!(
            apply_open_companion(&folder.join("native.mycad"), &mut document),
            (None, HelperStatus::Embedded)
        );
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn current_summary_keeps_import_counts_separate_and_reveals_actual_definitions() {
        let mut document = Document::default();
        let mut editable = BlockDefinition::plain("Door", Point3::default(), vec![]);
        editable.dynamic = Some(DynamicDefinition {
            parameters: vec![ParameterDef::number(
                ParameterId(1),
                "Width",
                NumericParameter::length(100.0),
            )],
            ..Default::default()
        });
        let mut native = editable.clone();
        native.name = "Native frame".into();
        document.blocks.insert(editable.name.clone(), editable);
        document.blocks.insert(native.name.clone(), native);
        document.blocks.insert(
            "Static frame".into(),
            BlockDefinition::plain("Static frame", Point3::default(), vec![]),
        );
        document.diagnostics.dynamic_editable = 7;
        document.diagnostics.dynamic_kept_static = 2;
        document.diagnostics.dynamic_lines = vec![
            "dynamic Door: supported properties".into(),
            "dynamic Static frame kept static (unsupported array)".into(),
            "dynamic Missing kept static (no definition)".into(),
        ];
        let summary = CompatibilitySummary::from_document(&document);
        assert_eq!(summary.editable_definitions, 2);
        assert_eq!(summary.ordinary_definitions, 1);
        assert_eq!(summary.imported_editable_references, 7);
        assert_eq!(summary.imported_static_items, 2);
        assert!(summary.has_save_loss);
        assert_eq!(summary.blocks.len(), 3);
        assert!(summary
            .blocks
            .iter()
            .any(|block| block.name == "Door" && block.status == BlockStatus::ImportedEditable));
        assert!(summary
            .blocks
            .iter()
            .any(|block| block.name == "Native frame" && block.status == BlockStatus::Editable));
        assert!(summary.blocks.iter().any(
            |block| block.name == "Static frame" && block.status == BlockStatus::ImportedStatic
        ));
        assert!(!summary.blocks.iter().any(|block| block.name == "Missing"));
    }

    #[test]
    fn retention_copy_never_promises_autocad_dynamic_control_preservation() {
        let exchange = export_summary(CadFileFormat::Dwg, true).join(" ");
        assert!(exchange.contains("ordinary geometry"));
        assert!(exchange.contains("original dynamic controls are not preserved"));
        assert!(exchange.contains("retain the original file"));
        let native = export_summary(CadFileFormat::Mycad, false).join(" ");
        assert!(native.contains("parameters, actions and instance settings together"));
        assert!(native.contains("AutoCAD does not open"));
    }

    #[test]
    fn diagnostics_are_bounded_without_cutting_utf8() {
        let text = "ø".repeat(MAX_DETAIL_CHARACTERS + 2);
        let clipped = bounded_text(&text);
        assert_eq!(clipped.chars().count(), MAX_DETAIL_CHARACTERS + 1);
        assert!(clipped.ends_with('…'));
        let mut document = Document::default();
        for i in 0..MAX_DETAIL_ROWS + 10 {
            document.diagnostics.warnings.push(format!("warning {i}"));
        }
        let summary = CompatibilitySummary::from_document(&document);
        assert_eq!(summary.warnings.len(), MAX_DETAIL_ROWS);
        assert!(summary.details_truncated);
    }
}
