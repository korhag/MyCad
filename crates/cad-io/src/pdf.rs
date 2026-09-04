//! Vector PDF of model-space geometry. Does not use cad-render tessellation
//! or capture the viewport.

use std::path::Path;

use cad_core::{
    CadColor, Document, Entity, Extents2, Geometry, HatchEdge, HatchPath, Point2, Point3,
    PolyVertex, Rgb, Transform2,
};

use crate::error::ExportError;
use crate::options::{PdfExportOptions, PdfPlotStyle, SaveReport};

// ------------------------------------------------------------
// Function: export_pdf
// Purpose: Fit plottable model-space geometry to the chosen paper as vectors.
// ------------------------------------------------------------
pub fn export_pdf(
    document: &Document,
    path: &Path,
    options: &PdfExportOptions,
) -> Result<SaveReport, ExportError> {
    let (page_width_pt, page_height_pt) = options.page_size_pt();
    let margin_pt = options.margin_pt();
    if !page_width_pt.is_finite()
        || !page_height_pt.is_finite()
        || page_width_pt <= 0.0
        || page_height_pt <= 0.0
        || !margin_pt.is_finite()
        || margin_pt < 0.0
    {
        return Err(ExportError::Invalid(
            "PDF page size must be greater than zero",
        ));
    }
    if page_width_pt - 2.0 * margin_pt < 1.0 || page_height_pt - 2.0 * margin_pt < 1.0 {
        return Err(ExportError::Invalid(
            "PDF margins leave no drawable area on the page",
        ));
    }
    let extents = document
        .compute_extents()
        .unwrap_or_else(|| Extents2::from_corners(Point2::new(0.0, 0.0), Point2::new(1.0, 1.0)));
    let mut report = SaveReport::default();
    let mut paths = String::new();
    let mapper = PageMap::fit(extents, *options);
    let mut stack = Vec::new();
    for entity in &document.model_space {
        report.entities_written += emit_entity(
            document,
            entity,
            Transform2::identity(),
            CadColor::Aci(7),
            mapper,
            *options,
            &mut stack,
            &mut paths,
        );
    }
    let content = format!("q\n{:.4} w\n{paths}Q\n", options.stroke_pt.max(0.05));
    let pdf = assemble_pdf(page_width_pt, page_height_pt, &content);
    crate::atomic::write_atomic(path, &pdf).map_err(|source| ExportError::io(path, source))?;
    Ok(report)
}

#[derive(Clone, Copy)]
struct PageMap {
    origin: Point2,
    scale: f64,
    margin: f64,
}

impl PageMap {
    fn fit(extents: Extents2, options: PdfExportOptions) -> Self {
        let (page_width_pt, page_height_pt) = options.page_size_pt();
        let margin = options.margin_pt();
        let width = extents.width().max(1e-9);
        let height = extents.height().max(1e-9);
        let inner_w = (page_width_pt - 2.0 * margin).max(1.0);
        let inner_h = (page_height_pt - 2.0 * margin).max(1.0);
        let scale = (inner_w / width).min(inner_h / height);
        Self {
            origin: extents.min,
            scale,
            margin,
        }
    }

    fn map(&self, point: Point2) -> (f64, f64) {
        (
            self.margin + (point.x - self.origin.x) * self.scale,
            self.margin + (point.y - self.origin.y) * self.scale,
        )
    }
}

fn plot_stroke_rgb(style: PdfPlotStyle, rgb: Rgb) -> (f64, f64, f64) {
    match style {
        PdfPlotStyle::Monochrome => (0.0, 0.0, 0.0),
        PdfPlotStyle::Color if rgb.r > 250 && rgb.g > 250 && rgb.b > 250 => (0.0, 0.0, 0.0),
        PdfPlotStyle::Color => (
            f64::from(rgb.r) / 255.0,
            f64::from(rgb.g) / 255.0,
            f64::from(rgb.b) / 255.0,
        ),
    }
}

fn set_stroke_color(out: &mut String, style: PdfPlotStyle, rgb: Rgb) {
    let (r, g, b) = plot_stroke_rgb(style, rgb);
    out.push_str(&format!("{r:.4} {g:.4} {b:.4} RG\n"));
}

fn entity_rgb(document: &Document, entity: &Entity, block_color: CadColor) -> Rgb {
    let layer_color = document
        .layer(&entity.layer)
        .map(|layer| layer.color)
        .unwrap_or(CadColor::Aci(7));
    entity.color.resolve(layer_color, block_color)
}

fn emit_entity(
    document: &Document,
    entity: &Entity,
    transform: Transform2,
    block_color: CadColor,
    mapper: PageMap,
    options: PdfExportOptions,
    stack: &mut Vec<String>,
    out: &mut String,
) -> usize {
    if !entity.visible || !document.layer_is_plottable(&entity.layer) {
        return 0;
    }
    match &entity.geometry {
        Geometry::Line { start, end } => {
            set_stroke_color(
                out,
                options.style,
                entity_rgb(document, entity, block_color),
            );
            stroke_polyline(
                &mapper,
                out,
                &[transform.apply3(*start), transform.apply3(*end)],
                false,
            );
            1
        }
        Geometry::Point { position } => {
            set_stroke_color(
                out,
                options.style,
                entity_rgb(document, entity, block_color),
            );
            let p = transform.apply3(*position);
            let (x, y) = mapper.map(p.xy());
            out.push_str(&format!("{x:.4} {y:.4} m {x:.4} {y:.4} l S\n"));
            1
        }
        Geometry::Circle { center, radius, .. } => {
            set_stroke_color(
                out,
                options.style,
                entity_rgb(document, entity, block_color),
            );
            stroke_circle(
                &mapper,
                out,
                transform.apply3(*center).xy(),
                radius.abs() * transform.scale_x(),
            );
            1
        }
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            ..
        } => {
            set_stroke_color(
                out,
                options.style,
                entity_rgb(document, entity, block_color),
            );
            stroke_arc(
                &mapper,
                out,
                transform.apply3(*center).xy(),
                radius.abs() * transform.scale_x(),
                *start_angle + transform.rotation_component(),
                *end_angle + transform.rotation_component(),
            );
            1
        }
        Geometry::LwPolyline {
            vertices, closed, ..
        }
        | Geometry::Polyline {
            vertices, closed, ..
        } => {
            set_stroke_color(
                out,
                options.style,
                entity_rgb(document, entity, block_color),
            );
            let points: Vec<Point3> = vertices
                .iter()
                .map(|vertex| transform.apply3(vertex.point))
                .collect();
            stroke_bulged(&mapper, out, vertices, &points, *closed, transform);
            1
        }
        Geometry::Ellipse {
            center,
            major_axis,
            axis_ratio,
            ..
        } => {
            set_stroke_color(
                out,
                options.style,
                entity_rgb(document, entity, block_color),
            );
            let center = transform.apply3(*center).xy();
            let axis = transform.apply_vector(major_axis.xy());
            stroke_ellipse(&mapper, out, center, axis, *axis_ratio);
            1
        }
        Geometry::Spline { control_points, .. } => {
            set_stroke_color(
                out,
                options.style,
                entity_rgb(document, entity, block_color),
            );
            let points: Vec<Point3> = control_points
                .iter()
                .map(|p| transform.apply3(*p))
                .collect();
            stroke_polyline(&mapper, out, &points, false);
            1
        }
        Geometry::Solid { corners, .. } => {
            set_stroke_color(
                out,
                options.style,
                entity_rgb(document, entity, block_color),
            );
            let points: Vec<Point3> = corners.iter().map(|p| transform.apply3(*p)).collect();
            stroke_polyline(&mapper, out, &points, true);
            1
        }
        Geometry::Leader { vertices } | Geometry::MLine { vertices, .. } => {
            set_stroke_color(
                out,
                options.style,
                entity_rgb(document, entity, block_color),
            );
            let points: Vec<Point3> = vertices.iter().map(|p| transform.apply3(*p)).collect();
            stroke_polyline(&mapper, out, &points, false);
            1
        }
        Geometry::Text(data) => {
            set_stroke_color(
                out,
                options.style,
                entity_rgb(document, entity, block_color),
            );
            let p = transform.apply3(data.insertion);
            let (x, y) = mapper.map(p.xy());
            out.push_str(&format!("{x:.4} {y:.4} m {x:.4} {y:.4} l S\n"));
            1
        }
        Geometry::MText(data) => {
            set_stroke_color(
                out,
                options.style,
                entity_rgb(document, entity, block_color),
            );
            let p = transform.apply3(data.insertion);
            let (x, y) = mapper.map(p.xy());
            out.push_str(&format!("{x:.4} {y:.4} m {x:.4} {y:.4} l S\n"));
            1
        }
        Geometry::Insert {
            block_name,
            insertion,
            scale,
            rotation,
            extrusion,
            column_count,
            row_count,
            column_spacing,
            row_spacing,
            ..
        } => {
            if stack
                .iter()
                .any(|name| name.eq_ignore_ascii_case(block_name))
            {
                return 0;
            }
            let Some(block) = document.blocks.get(block_name) else {
                return 0;
            };
            stack.push(block_name.clone());
            let inherit = match entity.color {
                CadColor::ByLayer | CadColor::ByBlock => block_color,
                other => other,
            };
            let mut written = 0;
            let cols = (*column_count).max(1);
            let rows = (*row_count).max(1);
            for col in 0..cols {
                for row in 0..rows {
                    let extra = Transform2::translate(
                        col as f64 * *column_spacing,
                        row as f64 * *row_spacing,
                    );
                    let nested = transform.then(
                        Transform2::block_insert(
                            *insertion,
                            *scale,
                            *rotation,
                            *extrusion,
                            block.base_pt,
                        )
                        .then(extra),
                    );
                    for child in &block.entities {
                        written += emit_entity(
                            document, child, nested, inherit, mapper, options, stack, out,
                        );
                    }
                }
            }
            stack.pop();
            written
        }
        Geometry::Hatch(hatch) => {
            set_stroke_color(
                out,
                options.style,
                entity_rgb(document, entity, block_color),
            );
            for path in &hatch.paths {
                stroke_hatch_path(&mapper, out, path, transform);
            }
            1
        }
        Geometry::Dimension { block_name } => {
            let Some(block) = document.blocks.get(block_name) else {
                return 0;
            };
            if stack
                .iter()
                .any(|name| name.eq_ignore_ascii_case(block_name))
            {
                return 0;
            }
            stack.push(block_name.clone());
            let mut written = 0;
            for child in &block.entities {
                written += emit_entity(
                    document,
                    child,
                    transform,
                    block_color,
                    mapper,
                    options,
                    stack,
                    out,
                );
            }
            stack.pop();
            written
        }
    }
}

fn stroke_hatch_path(mapper: &PageMap, out: &mut String, path: &HatchPath, transform: Transform2) {
    match path {
        HatchPath::Polyline { vertices, closed } => {
            let points: Vec<Point3> = vertices
                .iter()
                .map(|vertex| transform.apply3(vertex.point))
                .collect();
            stroke_bulged(mapper, out, vertices, &points, *closed, transform);
        }
        HatchPath::Edges(edges) => {
            for edge in edges {
                match edge {
                    HatchEdge::Line { start, end } => stroke_polyline(
                        mapper,
                        out,
                        &[transform.apply3(*start), transform.apply3(*end)],
                        false,
                    ),
                    HatchEdge::Arc {
                        center,
                        radius,
                        start_angle,
                        end_angle,
                        ..
                    } => stroke_arc(
                        mapper,
                        out,
                        transform.apply3(*center).xy(),
                        radius.abs() * transform.scale_x(),
                        *start_angle + transform.rotation_component(),
                        *end_angle + transform.rotation_component(),
                    ),
                    HatchEdge::Ellipse {
                        center,
                        major_endpoint,
                        axis_ratio,
                        ..
                    } => {
                        let center = transform.apply3(*center).xy();
                        let axis = transform.apply_vector(major_endpoint.xy());
                        stroke_ellipse(mapper, out, center, axis, *axis_ratio);
                    }
                    HatchEdge::Spline { control_points } => {
                        let points: Vec<Point3> = control_points
                            .iter()
                            .map(|p| transform.apply3(*p))
                            .collect();
                        stroke_polyline(mapper, out, &points, false);
                    }
                }
            }
        }
    }
}

fn stroke_polyline(mapper: &PageMap, out: &mut String, points: &[Point3], closed: bool) {
    let Some(first) = points.first() else {
        return;
    };
    let (x, y) = mapper.map(first.xy());
    out.push_str(&format!("{x:.4} {y:.4} m\n"));
    for point in points.iter().skip(1) {
        let (x, y) = mapper.map(point.xy());
        out.push_str(&format!("{x:.4} {y:.4} l\n"));
    }
    if closed {
        out.push_str("h\n");
    }
    out.push_str("S\n");
}

fn stroke_bulged(
    mapper: &PageMap,
    out: &mut String,
    vertices: &[PolyVertex],
    points: &[Point3],
    closed: bool,
    _transform: Transform2,
) {
    if vertices.iter().all(|v| v.bulge.abs() <= 1e-12) {
        stroke_polyline(mapper, out, points, closed);
        return;
    }
    stroke_polyline(mapper, out, points, closed);
}

fn stroke_circle(mapper: &PageMap, out: &mut String, center: Point2, radius: f64) {
    stroke_arc(mapper, out, center, radius, 0.0, std::f64::consts::TAU);
}

fn stroke_arc(
    mapper: &PageMap,
    out: &mut String,
    center: Point2,
    radius: f64,
    start: f64,
    end: f64,
) {
    let mut sweep = end - start;
    if sweep <= 1e-12 {
        sweep += std::f64::consts::TAU;
    }
    let steps = ((sweep.abs() / (std::f64::consts::FRAC_PI_8)).ceil() as usize).clamp(4, 64);
    for i in 0..=steps {
        let t = start + sweep * (i as f64 / steps as f64);
        let p = Point2::new(center.x + radius * t.cos(), center.y + radius * t.sin());
        let (x, y) = mapper.map(p);
        if i == 0 {
            out.push_str(&format!("{x:.4} {y:.4} m\n"));
        } else {
            out.push_str(&format!("{x:.4} {y:.4} l\n"));
        }
    }
    out.push_str("S\n");
}

fn stroke_ellipse(mapper: &PageMap, out: &mut String, center: Point2, major: Point2, ratio: f64) {
    let steps = 48;
    for i in 0..=steps {
        let t = std::f64::consts::TAU * (i as f64 / steps as f64);
        let p = Point2::new(
            center.x + major.x * t.cos() - major.y * ratio * t.sin(),
            center.y + major.y * t.cos() + major.x * ratio * t.sin(),
        );
        let (x, y) = mapper.map(p);
        if i == 0 {
            out.push_str(&format!("{x:.4} {y:.4} m\n"));
        } else {
            out.push_str(&format!("{x:.4} {y:.4} l\n"));
        }
    }
    out.push_str("h S\n");
}

fn assemble_pdf(width: f64, height: f64, content: &str) -> Vec<u8> {
    let stream = content.as_bytes();
    let objects = [
        "1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n".to_string(),
        "2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n".to_string(),
        format!(
            "3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 {width:.2} {height:.2}] /Contents 4 0 R /Resources << >> >> endobj\n"
        ),
        format!(
            "4 0 obj << /Length {} >> stream\n{}endstream\nendobj\n",
            stream.len(),
            content
        ),
    ];
    let mut pdf = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for object in &objects {
        offsets.push(pdf.len());
        pdf.extend_from_slice(object.as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
    );
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer << /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::{PdfOrientation, PdfPaperSize};
    use cad_core::{CadColor, Entity, Geometry, Layer, Point3};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn write_pdf(document: &Document, options: &PdfExportOptions) -> (SaveReport, String) {
        let path = std::env::temp_dir().join(format!(
            "mycad-cad-io-{}.pdf",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let report = export_pdf(document, &path, options).expect("pdf");
        let bytes = fs::read(&path).expect("read");
        let _ = fs::remove_file(&path);
        (report, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn line_document() -> Document {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(10.0, 0.0),
        }));
        document
    }

    #[test]
    fn pdf_contains_header_and_line_operators() {
        let (report, text) = write_pdf(&line_document(), &PdfExportOptions::default());
        assert_eq!(report.entities_written, 1);
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains(" m\n") || text.contains(" m "));
        assert!(text.contains(" l\n") || text.contains(" l "));
        assert!(!text.contains("/XObject"));
        assert!(!text.contains("/Image"));
    }

    #[test]
    fn hidden_layer_is_not_plotted() {
        let mut document = line_document();
        document.layers.insert(
            "OFF".into(),
            Layer {
                name: "OFF".into(),
                visible: false,
                frozen: false,
                color: CadColor::Aci(1),
                linetype: "CONTINUOUS".into(),
            },
        );
        let mut hidden = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 5.0),
            end: Point3::from_xy(10.0, 5.0),
        });
        hidden.layer = "OFF".into();
        document.add_entity(hidden);
        let (report, text) = write_pdf(&document, &PdfExportOptions::default());
        assert_eq!(report.entities_written, 1);
        assert_eq!(count_op(&text, "m"), 1);
        assert_eq!(count_op(&text, "l"), 1);
    }

    #[test]
    fn color_style_keeps_aci_red_and_monochrome_does_not() {
        let mut document = line_document();
        document.model_space[0].color = CadColor::Aci(1);
        let mut color = PdfExportOptions::default();
        color.style = PdfPlotStyle::Color;
        let (_, color_text) = write_pdf(&document, &color);
        assert!(color_text.contains("1.0000 0.0000 0.0000 RG"));
        let mut mono = color;
        mono.style = PdfPlotStyle::Monochrome;
        let (_, mono_text) = write_pdf(&document, &mono);
        assert!(mono_text.contains("0.0000 0.0000 0.0000 RG"));
        assert!(!mono_text.contains("1.0000 0.0000 0.0000 RG"));
    }

    #[test]
    fn white_aci_plots_black_on_paper() {
        let mut document = line_document();
        document.model_space[0].color = CadColor::Aci(7);
        let (_, text) = write_pdf(&document, &PdfExportOptions::default());
        assert!(text.contains("0.0000 0.0000 0.0000 RG"));
        assert!(!text.contains("1.0000 1.0000 1.0000 RG"));
    }

    fn path_points(pdf: &str) -> Vec<(f64, f64)> {
        let tokens: Vec<&str> = pdf.split_whitespace().collect();
        let mut points = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            if *token != "m" && *token != "l" {
                continue;
            }
            if index < 2 {
                continue;
            }
            let Ok(x) = tokens[index - 2].parse::<f64>() else {
                continue;
            };
            let Ok(y) = tokens[index - 1].parse::<f64>() else {
                continue;
            };
            points.push((x, y));
        }
        points
    }

    fn count_op(pdf: &str, op: &str) -> usize {
        pdf.split_whitespace().filter(|token| *token == op).count()
    }

    #[test]
    fn landscape_swaps_mediabox() {
        let mut options = PdfExportOptions::default();
        options.orientation = PdfOrientation::Landscape;
        options.paper = PdfPaperSize::A4;
        let (_, text) = write_pdf(&line_document(), &options);
        let (width, height) = options.page_size_pt();
        assert!(text.contains(&format!("MediaBox [0 0 {width:.2} {height:.2}]")));
        assert!(width > height);
    }

    #[test]
    fn pdf_is_valid_with_requested_page_size_and_vectors() {
        let mut options = PdfExportOptions::default();
        options.paper = PdfPaperSize::A4;
        options.orientation = PdfOrientation::Portrait;
        options.margin_mm = 10.0;
        let (report, text) = write_pdf(&line_document(), &options);
        let (width, height) = options.page_size_pt();
        assert_eq!(report.entities_written, 1);
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains("%%EOF"));
        assert!(text.contains("xref"));
        assert!(text.contains(&format!("MediaBox [0 0 {width:.2} {height:.2}]")));
        let points = path_points(&text);
        assert!(
            !points.is_empty() && count_op(&text, "m") > 0 && count_op(&text, "l") > 0,
            "expected vector path operators, got {points:?}"
        );
        assert!(!text.contains("/XObject"));
        assert!(!text.contains("/Image"));
        let margin = options.margin_pt();
        let slack = 1e-3;
        for (x, y) in points {
            assert!(
                x >= margin - slack
                    && x <= width - margin + slack
                    && y >= margin - slack
                    && y <= height - margin + slack,
                "path point ({x}, {y}) is outside printable region [{margin}, {}, {margin}, {}]",
                width - margin,
                height - margin
            );
        }
    }

    #[test]
    fn frozen_visible_layer_is_not_plotted() {
        let mut document = line_document();
        document.layers.insert(
            "FROZEN".into(),
            Layer {
                name: "FROZEN".into(),
                visible: true,
                frozen: true,
                color: CadColor::Aci(1),
                linetype: "CONTINUOUS".into(),
            },
        );
        let mut frozen = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 50.0),
            end: Point3::from_xy(10.0, 50.0),
        });
        frozen.layer = "FROZEN".into();
        document.add_entity(frozen);
        let (report, text) = write_pdf(&document, &PdfExportOptions::default());
        assert_eq!(report.entities_written, 1);
        assert_eq!(count_op(&text, "m"), 1);
        assert_eq!(count_op(&text, "l"), 1);
    }
}
