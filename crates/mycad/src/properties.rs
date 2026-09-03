//! Compact read-only inspector for the current selection.

use std::collections::BTreeMap;

use cad_core::{CadColor, Document, Entity, Geometry, Point3, PolyVertex};
use eframe::egui::{self, RichText, Ui};

use crate::app::MyCadApp;

pub fn show(ui: &mut Ui, app: &MyCadApp) {
    ui.heading("Properties");
    ui.separator();
    let Some(document) = app.document.as_deref() else {
        ui.weak("Open a drawing, then click an entity.");
        return;
    };
    let selection = app.selection.indices();
    if selection.is_empty() {
        ui.weak("Click a line, circle, polyline, or block.");
        ui.add_space(8.0);
        ui.weak("Esc clears the selection.");
        return;
    }
    if selection.len() == 1 {
        if let Some(entity) = document.model_space.get(selection[0]) {
            show_single(ui, document, entity, selection[0]);
            return;
        }
    }
    show_multiple(ui, document, selection);
}

fn show_single(ui: &mut Ui, document: &Document, entity: &Entity, index: usize) {
    ui.label(RichText::new(entity.geometry.type_name()).strong());
    ui.add_space(4.0);
    ui.label(RichText::new("Common").small().weak());
    egui::Grid::new("props-common")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            kv(ui, "Index", format!("#{index}"));
            kv(ui, "Layer", entity.layer.clone());
            kv(ui, "Color", color_label(document, entity));
            kv(ui, "Linetype", entity.linetype.clone());
            kv(ui, "Linetype scale", format_num(entity.linetype_scale));
            kv(ui, "Drawing LTSCALE", format_num(document.ltscale));
            kv(
                ui,
                "Effective LT scale",
                format_num((document.ltscale * entity.linetype_scale).max(1e-6)),
            );
            kv(ui, "Visible", yes_no(entity.visible));
        });
    ui.add_space(10.0);
    ui.label(RichText::new("Geometry").small().weak());
    egui::Grid::new("props-geom")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            geometry_rows(ui, &entity.geometry);
        });
}

fn show_multiple(ui: &mut Ui, document: &Document, indices: &[usize]) {
    let entities: Vec<&Entity> = indices
        .iter()
        .filter_map(|i| document.model_space.get(*i))
        .collect();
    ui.label(RichText::new(format!("{} selected", entities.len())).strong());
    ui.add_space(6.0);

    let mut types: BTreeMap<&str, usize> = BTreeMap::new();
    let mut layers: BTreeMap<&str, usize> = BTreeMap::new();
    for entity in &entities {
        *types.entry(entity.geometry.type_name()).or_insert(0) += 1;
        *layers.entry(entity.layer.as_str()).or_insert(0) += 1;
    }
    ui.label(RichText::new("Types").small().weak());
    for (name, count) in types {
        ui.monospace(format!("{name}  {count}"));
    }
    ui.add_space(6.0);
    ui.label(RichText::new("Layers").small().weak());
    for (name, count) in layers {
        ui.monospace(format!("{name}  {count}"));
    }
    ui.add_space(10.0);
    ui.label(RichText::new("Shared").small().weak());
    egui::Grid::new("props-shared")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            kv(
                ui,
                "Layer",
                shared_text(entities.iter().map(|e| e.layer.as_str())),
            );
            kv(
                ui,
                "Color",
                shared_text(entities.iter().map(|e| color_label(document, e))),
            );
            kv(
                ui,
                "Linetype",
                shared_text(entities.iter().map(|e| e.linetype.as_str())),
            );
        });
}

fn geometry_rows(ui: &mut Ui, geometry: &Geometry) {
    match geometry {
        Geometry::Line { start, end } => {
            kv(ui, "Start", point_label(*start));
            kv(ui, "End", point_label(*end));
            kv(ui, "Length", format_num(start.xy().distance(end.xy())));
        }
        Geometry::Circle { center, radius, .. } => {
            kv(ui, "Center", point_label(*center));
            kv(ui, "Radius", format_num(*radius));
            kv(ui, "Diameter", format_num(*radius * 2.0));
        }
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            ..
        } => {
            kv(ui, "Center", point_label(*center));
            kv(ui, "Radius", format_num(*radius));
            kv(ui, "Start", format_deg(*start_angle));
            kv(ui, "End", format_deg(*end_angle));
        }
        Geometry::LwPolyline {
            vertices,
            closed,
            linetype_generation_continuous,
            ..
        }
        | Geometry::Polyline {
            vertices,
            closed,
            linetype_generation_continuous,
        } => {
            kv(ui, "Vertices", vertices.len().to_string());
            kv(ui, "Closed", yes_no(*closed));
            kv(ui, "Length", format_num(polyline_length(vertices, *closed)));
            kv(
                ui,
                "Linetype generation",
                if *linetype_generation_continuous {
                    "Continuous"
                } else {
                    "Per segment"
                },
            );
        }
        Geometry::Insert {
            block_name,
            insertion,
            scale,
            rotation,
            attribs,
            column_count,
            row_count,
            ..
        } => {
            kv(ui, "Block", block_name.clone());
            kv(ui, "Insertion", point_label(*insertion));
            kv(ui, "Scale", format!("{:.4}, {:.4}", scale.x, scale.y));
            kv(ui, "Rotation", format_deg(*rotation));
            if *column_count > 1 || *row_count > 1 {
                kv(ui, "Array", format!("{column_count} × {row_count}"));
            }
            if !attribs.is_empty() {
                kv(ui, "Attributes", attribs.len().to_string());
            }
        }
        Geometry::Point { position } => kv(ui, "Position", point_label(*position)),
        Geometry::Ellipse {
            center,
            major_axis,
            axis_ratio,
            ..
        } => {
            kv(ui, "Center", point_label(*center));
            kv(ui, "Major", format_num(major_axis.length()));
            kv(ui, "Ratio", format_num(*axis_ratio));
        }
        Geometry::Spline {
            degree,
            control_points,
            closed,
            ..
        } => {
            kv(ui, "Degree", degree.to_string());
            kv(ui, "Controls", control_points.len().to_string());
            kv(ui, "Closed", yes_no(*closed));
        }
        Geometry::Text(text) => {
            kv(ui, "Insertion", point_label(text.insertion));
            kv(ui, "Height", format_num(text.height));
            kv(ui, "Rotation", format_deg(text.rotation));
            kv(ui, "Value", truncate(&text.value, 48));
        }
        Geometry::MText(text) => {
            kv(ui, "Insertion", point_label(text.insertion));
            kv(ui, "Height", format_num(text.height));
            kv(ui, "Width", format_num(text.width));
            kv(ui, "Value", truncate(&text.value, 48));
        }
        Geometry::Hatch(hatch) => {
            kv(
                ui,
                "Fill",
                if hatch.solid_fill { "Solid" } else { "Pattern" },
            );
            kv(ui, "Paths", hatch.paths.len().to_string());
        }
        Geometry::Solid { corners, .. } => {
            kv(ui, "Corner 1", point_label(corners[0]));
            kv(ui, "Corner 2", point_label(corners[1]));
        }
        Geometry::Leader { vertices } => kv(ui, "Vertices", vertices.len().to_string()),
        Geometry::MLine { vertices, closed } => {
            kv(ui, "Vertices", vertices.len().to_string());
            kv(ui, "Closed", yes_no(*closed));
        }
        Geometry::Dimension { block_name } => kv(ui, "Block", block_name.clone()),
    }
}

fn kv(ui: &mut Ui, label: &str, value: impl Into<String>) {
    ui.weak(label);
    ui.monospace(value.into());
    ui.end_row();
}

fn yes_no(value: bool) -> String {
    if value { "Yes" } else { "No" }.into()
}

fn point_label(p: Point3) -> String {
    if p.z.abs() > 1e-9 {
        format!("{:.4}, {:.4}, {:.4}", p.x, p.y, p.z)
    } else {
        format!("{:.4}, {:.4}", p.x, p.y)
    }
}

fn format_num(value: f64) -> String {
    if !value.is_finite() {
        "—".into()
    } else if value.abs() >= 1000.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.4}")
    }
}

fn format_deg(radians: f64) -> String {
    format!("{:.2}°", radians.to_degrees())
}

fn truncate(value: &str, max: usize) -> String {
    let cleaned = value.replace(['\r', '\n'], " ");
    if cleaned.chars().count() <= max {
        cleaned
    } else {
        let cut: String = cleaned.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

fn color_label(document: &Document, entity: &Entity) -> String {
    match entity.color {
        CadColor::ByLayer => {
            let layer = document
                .layer(&entity.layer)
                .map(|l| l.color.display_name())
                .unwrap_or_else(|| "ACI 7".into());
            format!("ByLayer ({layer})")
        }
        other => other.display_name(),
    }
}

fn shared_text<I, S>(values: I) -> String
where
    I: Iterator<Item = S>,
    S: Into<String>,
{
    let mut iter = values.map(Into::into);
    let Some(first) = iter.next() else {
        return "—".into();
    };
    if iter.all(|value| value == first) {
        first
    } else {
        "Mixed".into()
    }
}

fn polyline_length(vertices: &[PolyVertex], closed: bool) -> f64 {
    if vertices.len() < 2 {
        return 0.0;
    }
    let n = if closed {
        vertices.len()
    } else {
        vertices.len() - 1
    };
    let mut total = 0.0;
    for i in 0..n {
        let a = vertices[i];
        let b = vertices[(i + 1) % vertices.len()];
        total += segment_length(a, b);
    }
    total
}

fn segment_length(a: PolyVertex, b: PolyVertex) -> f64 {
    let chord = a.point.xy().distance(b.point.xy());
    if a.bulge.abs() < 1e-12 {
        return chord;
    }
    let theta = 4.0 * a.bulge.atan();
    if theta.abs() < 1e-12 {
        chord
    } else {
        let radius = chord / (2.0 * (theta * 0.5).sin().abs().max(1e-12));
        (radius * theta).abs()
    }
}
