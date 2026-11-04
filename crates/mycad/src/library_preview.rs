//! Bounded, worker-built linework shared by library thumbnails and placement.

use cad_core::{Document, Entity, Extents2, Geometry, Point2, Point3};
use eframe::egui::{self, Color32};

/// Bounds the per-frame placement painter independently of source complexity.
pub(crate) const MAX_LIBRARY_PREVIEW_SEGMENTS: usize = 4_096;
const MAX_THUMBNAIL_SEGMENTS: usize = 256;

#[derive(Debug, Clone, Default)]
pub(crate) struct LibraryPreview {
    pub lines: Vec<[Point2; 2]>,
    pub bounds: Option<Extents2>,
    pub simplified: bool,
}

pub(crate) fn preview_for_asset(asset: &cad_io::BlockAsset) -> Result<LibraryPreview, String> {
    let mut document = Document::default();
    let name = cad_io::import_block_asset(
        &mut document,
        cad_io::BlockAsset {
            blocks: asset.blocks.clone(),
            layers: asset.layers.clone(),
            linetypes: asset.linetypes.clone(),
            text_styles: asset.text_styles.clone(),
        },
    )
    .map_err(|error| error.to_string())?;
    document.add_entity(Entity::new(Geometry::Insert {
        block_name: name,
        insertion: Point3::default(),
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
    }));
    let mut document = cad_core::materialize_evaluated(
        &document,
        &mut cad_core::EvaluationCache::default(),
        cad_core::EvaluationRequest {
            generation: document.content_generation(),
        },
    )
    .map_err(|error| error.to_string())?;
    let bounds = document.compute_extents();
    document.diagnostics.extents = bounds;
    let display = cad_render::tessellate_document(&document);
    let line_count = display.line_vertices.len() / 2;
    let triangle_count = display.triangle_vertices.len() / 3;
    let segment_count = line_count.saturating_add(triangle_count.saturating_mul(3));
    let stride = segment_count.div_ceil(MAX_LIBRARY_PREVIEW_SEGMENTS).max(1);
    let mut lines = Vec::with_capacity(segment_count.min(MAX_LIBRARY_PREVIEW_SEGMENTS));
    let point = |vertex: &cad_render::GpuVertex| {
        Point2::new(
            display.origin.x + f64::from(vertex.position[0]),
            display.origin.y + f64::from(vertex.position[1]),
        )
    };
    for index in (0..segment_count)
        .step_by(stride)
        .take(MAX_LIBRARY_PREVIEW_SEGMENTS)
    {
        let segment = if index < line_count {
            [
                point(&display.line_vertices[index * 2]),
                point(&display.line_vertices[index * 2 + 1]),
            ]
        } else {
            let edge = index - line_count;
            let triangle = edge / 3;
            let corner = edge % 3;
            [
                point(&display.triangle_vertices[triangle * 3 + corner]),
                point(&display.triangle_vertices[triangle * 3 + (corner + 1) % 3]),
            ]
        };
        if segment.iter().all(|point| point.is_finite()) {
            lines.push(segment);
        }
    }
    Ok(LibraryPreview {
        lines,
        bounds,
        simplified: stride > 1,
    })
}

pub(crate) fn paint_thumbnail(ui: &mut egui::Ui, preview: Option<&LibraryPreview>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(80.0, 58.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_rgb(33, 40, 48));
    let Some(preview) = preview else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Preview",
            egui::FontId::proportional(11.0),
            Color32::GRAY,
        );
        return;
    };
    let Some(bounds) = preview.bounds.filter(|bounds| bounds.is_valid()) else {
        return;
    };
    let center = bounds.center();
    let scale = ((rect.width() - 10.0) as f64 / bounds.width().max(1e-6))
        .min((rect.height() - 10.0) as f64 / bounds.height().max(1e-6));
    if !center.is_finite() || !scale.is_finite() || scale <= 0.0 {
        return;
    }
    let screen = |point: Point2| {
        egui::pos2(
            rect.center().x + ((point.x - center.x) * scale) as f32,
            rect.center().y - ((point.y - center.y) * scale) as f32,
        )
    };
    let stride = preview.lines.len().div_ceil(MAX_THUMBNAIL_SEGMENTS).max(1);
    for line in preview
        .lines
        .iter()
        .step_by(stride)
        .take(MAX_THUMBNAIL_SEGMENTS)
    {
        let points = [screen(line[0]), screen(line[1])];
        if points.iter().all(|point| point.is_finite()) {
            painter.line_segment(
                points,
                egui::Stroke::new(1.0_f32, Color32::from_rgb(190, 224, 216)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_preview_respects_the_block_base_and_bounds_complex_geometry() {
        let asset = cad_io::BlockAsset {
            blocks: vec![cad_core::BlockDefinition::plain(
                "Lines",
                Point3::new(10.0, 20.0, 0.0),
                (0..10_000)
                    .map(|i| {
                        Entity::new(Geometry::Line {
                            start: Point3::new(10.0, 20.0 + i as f64, 0.0),
                            end: Point3::new(20.0, 20.0 + i as f64, 0.0),
                        })
                    })
                    .collect(),
            )],
            layers: Vec::new(),
            linetypes: Vec::new(),
            text_styles: Vec::new(),
        };
        let preview = preview_for_asset(&asset).unwrap();
        assert!(preview.simplified);
        assert!(preview.lines.len() <= MAX_LIBRARY_PREVIEW_SEGMENTS);
        let bounds = preview.bounds.unwrap();
        assert_eq!(bounds.min, Point2::new(0.0, 0.0));
        assert_eq!(bounds.max, Point2::new(10.0, 9_999.0));
    }
}
