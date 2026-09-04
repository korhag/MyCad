//! Vector PDF of model-space geometry. Does not use cad-render tessellation
//! or capture the viewport.

use std::path::Path;

use cad_core::{
    arc_points, bspline_points, circle_points, default_extrusion, ellipse_points, polyline_points,
    stroke_text, strip_mtext, CadColor, Document, Entity, Extents2, Geometry, HatchEdge,
    HatchPath, Point2, Point3, Rgb, Transform2, CIRCLE_SEGMENTS,
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

fn set_fill_color(out: &mut String, style: PdfPlotStyle, rgb: Rgb) {
    let (r, g, b) = plot_stroke_rgb(style, rgb);
    out.push_str(&format!("{r:.4} {g:.4} {b:.4} rg\n"));
}

#[derive(Clone, Copy)]
enum Paint {
    Stroke,
    FillStroke,
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
            color_stroke(out, document, entity, block_color, options);
            stroke_polyline(
                &mapper,
                out,
                &[transform.apply3(*start), transform.apply3(*end)],
                false,
            );
            1
        }
        Geometry::Point { position } => {
            color_stroke(out, document, entity, block_color, options);
            let p = transform.apply(position.xy());
            let s = 0.5_f64.max(transform.scale_x().abs().max(transform.scale_y().abs()) * 0.4);
            paint_path(
                &mapper,
                out,
                &[Point2::new(p.x - s, p.y), Point2::new(p.x + s, p.y)],
                false,
                Paint::Stroke,
            );
            paint_path(
                &mapper,
                out,
                &[Point2::new(p.x, p.y - s), Point2::new(p.x, p.y + s)],
                false,
                Paint::Stroke,
            );
            1
        }
        Geometry::Circle {
            center,
            radius,
            extrusion,
        } => {
            color_stroke(out, document, entity, block_color, options);
            let pts: Vec<Point2> = circle_points(*center, *radius, *extrusion, CIRCLE_SEGMENTS)
                .into_iter()
                .map(|p| transform.apply(p))
                .collect();
            paint_path(&mapper, out, &pts, true, Paint::Stroke);
            1
        }
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            extrusion,
        } => {
            color_stroke(out, document, entity, block_color, options);
            let pts: Vec<Point2> = arc_points(
                *center,
                *radius,
                *start_angle,
                *end_angle,
                true,
                *extrusion,
                CIRCLE_SEGMENTS,
            )
            .into_iter()
            .map(|p| transform.apply(p))
            .collect();
            paint_path(&mapper, out, &pts, false, Paint::Stroke);
            1
        }
        Geometry::LwPolyline {
            vertices,
            closed,
            extrusion,
            ..
        } => {
            color_stroke(out, document, entity, block_color, options);
            let pts: Vec<Point2> = polyline_points(vertices, *closed, *extrusion)
                .into_iter()
                .map(|p| transform.apply(p))
                .collect();
            paint_path(&mapper, out, &pts, *closed, Paint::Stroke);
            1
        }
        Geometry::Polyline {
            vertices, closed, ..
        } => {
            color_stroke(out, document, entity, block_color, options);
            let pts: Vec<Point2> = polyline_points(vertices, *closed, default_extrusion())
                .into_iter()
                .map(|p| transform.apply(p))
                .collect();
            paint_path(&mapper, out, &pts, *closed, Paint::Stroke);
            1
        }
        Geometry::Ellipse {
            center,
            major_axis,
            axis_ratio,
            start_param,
            end_param,
            extrusion,
        } => {
            color_stroke(out, document, entity, block_color, options);
            let pts: Vec<Point2> = ellipse_points(
                *center,
                *major_axis,
                *axis_ratio,
                *start_param,
                *end_param,
                *extrusion,
                CIRCLE_SEGMENTS,
            )
            .into_iter()
            .map(|p| transform.apply(p))
            .collect();
            let closed = (*end_param - *start_param).abs() >= std::f64::consts::TAU - 1e-6;
            paint_path(&mapper, out, &pts, closed, Paint::Stroke);
            1
        }
        Geometry::Spline {
            degree,
            control_points,
            fit_points,
            knots,
            weights,
            closed,
        } => {
            color_stroke(out, document, entity, block_color, options);
            let sampled = if control_points.len() >= 2 {
                bspline_points(*degree, control_points, knots, weights, 24)
            } else {
                fit_points.iter().map(|p| p.xy()).collect()
            };
            let pts: Vec<Point2> = sampled.into_iter().map(|p| transform.apply(p)).collect();
            paint_path(&mapper, out, &pts, *closed, Paint::Stroke);
            1
        }
        Geometry::Solid { corners, .. } => {
            color_fill(out, document, entity, block_color, options);
            let pts: Vec<Point2> = corners.iter().map(|c| transform.apply(c.xy())).collect();
            paint_path(&mapper, out, &pts, true, Paint::FillStroke);
            1
        }
        Geometry::Leader { vertices } | Geometry::MLine { vertices, .. } => {
            color_stroke(out, document, entity, block_color, options);
            let pts: Vec<Point2> = vertices.iter().map(|p| transform.apply(p.xy())).collect();
            paint_path(&mapper, out, &pts, false, Paint::Stroke);
            1
        }
        Geometry::Text(data) => {
            color_stroke(out, document, entity, block_color, options);
            emit_text(
                &mapper,
                out,
                transform,
                data.insertion,
                data.height,
                data.rotation,
                &data.value,
            );
            1
        }
        Geometry::MText(data) => {
            color_stroke(out, document, entity, block_color, options);
            let cleaned = strip_mtext(&data.value);
            let mut y_off = 0.0;
            for line in cleaned.lines() {
                let insertion =
                    Point3::new(data.insertion.x, data.insertion.y - y_off, data.insertion.z);
                emit_text(
                    &mapper,
                    out,
                    transform,
                    insertion,
                    data.height,
                    data.rotation,
                    line,
                );
                y_off += data.height * 1.6;
            }
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
            let rgb = entity_rgb(document, entity, block_color);
            set_stroke_color(out, options.style, rgb);
            if hatch.solid_fill {
                set_fill_color(out, options.style, rgb);
            }
            for path in &hatch.paths {
                let pts = hatch_path_points(path)
                    .into_iter()
                    .map(|p| transform.apply(p))
                    .collect::<Vec<_>>();
                let paint = if hatch.solid_fill {
                    Paint::FillStroke
                } else {
                    Paint::Stroke
                };
                paint_path(&mapper, out, &pts, true, paint);
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

fn color_stroke(
    out: &mut String,
    document: &Document,
    entity: &Entity,
    block_color: CadColor,
    options: PdfExportOptions,
) {
    set_stroke_color(out, options.style, entity_rgb(document, entity, block_color));
}

fn color_fill(
    out: &mut String,
    document: &Document,
    entity: &Entity,
    block_color: CadColor,
    options: PdfExportOptions,
) {
    let rgb = entity_rgb(document, entity, block_color);
    set_stroke_color(out, options.style, rgb);
    set_fill_color(out, options.style, rgb);
}

fn emit_text(
    mapper: &PageMap,
    out: &mut String,
    transform: Transform2,
    insertion: Point3,
    height: f64,
    rotation: f64,
    value: &str,
) {
    for [a, b] in stroke_text(insertion.xy(), height.max(1e-6), rotation, value) {
        paint_path(
            mapper,
            out,
            &[transform.apply(a), transform.apply(b)],
            false,
            Paint::Stroke,
        );
    }
}

fn hatch_path_points(path: &HatchPath) -> Vec<Point2> {
    match path {
        HatchPath::Polyline { vertices, closed } => {
            polyline_points(vertices, *closed, default_extrusion())
        }
        HatchPath::Edges(edges) => {
            let mut pts = Vec::new();
            for edge in edges {
                match edge {
                    HatchEdge::Line { start, end } => {
                        if pts
                            .last()
                            .map(|p: &Point2| p.distance(start.xy()) > 1e-9)
                            .unwrap_or(true)
                        {
                            pts.push(start.xy());
                        }
                        pts.push(end.xy());
                    }
                    HatchEdge::Arc {
                        center,
                        radius,
                        start_angle,
                        end_angle,
                        is_ccw,
                    } => {
                        let mut arc = arc_points(
                            *center,
                            *radius,
                            *start_angle,
                            *end_angle,
                            *is_ccw,
                            default_extrusion(),
                            CIRCLE_SEGMENTS,
                        );
                        if !pts.is_empty() && !arc.is_empty() {
                            arc.remove(0);
                        }
                        pts.extend(arc);
                    }
                    HatchEdge::Ellipse {
                        center,
                        major_endpoint,
                        axis_ratio,
                        start_angle,
                        end_angle,
                        is_ccw,
                    } => {
                        let (start_param, end_param) = if *is_ccw {
                            (*start_angle, *end_angle)
                        } else {
                            (*end_angle, *start_angle)
                        };
                        let mut ellipse = ellipse_points(
                            *center,
                            *major_endpoint,
                            *axis_ratio,
                            start_param,
                            end_param,
                            default_extrusion(),
                            CIRCLE_SEGMENTS,
                        );
                        if !pts.is_empty() && !ellipse.is_empty() {
                            ellipse.remove(0);
                        }
                        pts.extend(ellipse);
                    }
                    HatchEdge::Spline { control_points } => {
                        let mut spline = bspline_points(3, control_points, &[], &[], 24);
                        if !pts.is_empty() && !spline.is_empty() {
                            spline.remove(0);
                        }
                        pts.extend(spline);
                    }
                }
            }
            pts
        }
    }
}

fn paint_path(mapper: &PageMap, out: &mut String, points: &[Point2], closed: bool, paint: Paint) {
    let Some(first) = points.first() else {
        return;
    };
    let (x, y) = mapper.map(*first);
    out.push_str(&format!("{x:.4} {y:.4} m\n"));
    for point in points.iter().skip(1) {
        let (x, y) = mapper.map(*point);
        out.push_str(&format!("{x:.4} {y:.4} l\n"));
    }
    if closed {
        out.push_str("h\n");
    }
    match paint {
        Paint::Stroke => out.push_str("S\n"),
        Paint::FillStroke => out.push_str("B\n"),
    }
}

fn stroke_polyline(mapper: &PageMap, out: &mut String, points: &[Point3], closed: bool) {
    let pts: Vec<Point2> = points.iter().map(|p| p.xy()).collect();
    paint_path(mapper, out, &pts, closed, Paint::Stroke);
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
