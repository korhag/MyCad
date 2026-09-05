//! Vector PDF of model-space geometry from the shared cad-core plot stream.

use std::path::Path;

use cad_core::{plot_geometry, Document, Extents2, PlotFill, PlotGeometry, Point2, Rgb};

use crate::error::ExportError;
use crate::options::{PdfExportOptions, PdfPlotArea, PdfPlotStyle, SaveReport};

// ------------------------------------------------------------
// Function: export_pdf
// Purpose: Fit plottable vector primitives to the chosen paper.
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

    let plot = plot_geometry(document);
    let world_extents = match options.plot_area {
        PdfPlotArea::Extents => plot.extents(),
        PdfPlotArea::Window(window) => Some(window),
    }
    .unwrap_or_else(|| Extents2::from_corners(Point2::new(0.0, 0.0), Point2::new(1.0, 1.0)));
    let world_extents = world_extents.expanded_to_square_if_degenerate();
    let mapper = PageMap::fit(world_extents, *options);

    let mut report = SaveReport {
        warnings: plot.warnings.clone(),
        entities_written: plot.entities_written,
    };
    let mut paths = String::new();
    if let PdfPlotArea::Window(window) = options.plot_area {
        emit_window_clip(&mut paths, &mapper, window);
    }
    emit_plot(&plot, &mapper, *options, &mut paths, &mut report);

    let content = format!("q\n{:.4} w\n{paths}Q\n", options.stroke_pt.max(0.05));
    let pdf = assemble_pdf(page_width_pt, page_height_pt, &content);
    crate::atomic::write_atomic(path, &pdf).map_err(|source| ExportError::io(path, source))?;
    Ok(report)
}

#[derive(Clone, Copy)]
struct PageMap {
    origin: Point2,
    scale: f64,
    offset: Point2,
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
        let mapped_w = width * scale;
        let mapped_h = height * scale;
        let (ox, oy) = if options.center_plot {
            (
                margin + (inner_w - mapped_w) * 0.5,
                margin + (inner_h - mapped_h) * 0.5,
            )
        } else {
            (margin, margin)
        };
        Self {
            origin: extents.min,
            scale,
            offset: Point2::new(ox, oy),
        }
    }

    fn map(&self, point: Point2) -> Option<(f64, f64)> {
        let x = self.offset.x + (point.x - self.origin.x) * self.scale;
        let y = self.offset.y + (point.y - self.origin.y) * self.scale;
        (x.is_finite() && y.is_finite()).then_some((x, y))
    }

    fn mapped_rect(&self, extents: Extents2) -> Option<(f64, f64, f64, f64)> {
        let (x0, y0) = self.map(extents.min)?;
        let (x1, y1) = self.map(extents.max)?;
        let x = x0.min(x1);
        let y = y0.min(y1);
        let width = (x1 - x0).abs();
        let height = (y1 - y0).abs();
        (width.is_finite() && height.is_finite()).then_some((x, y, width, height))
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

fn emit_window_clip(out: &mut String, mapper: &PageMap, window: Extents2) {
    let Some((x, y, width, height)) = mapper.mapped_rect(window) else {
        return;
    };
    out.push_str(&format!("{x:.4} {y:.4} {width:.4} {height:.4} re W n\n"));
}

fn emit_plot(
    plot: &PlotGeometry,
    mapper: &PageMap,
    options: PdfExportOptions,
    out: &mut String,
    report: &mut SaveReport,
) {
    let mut last_stroke: Option<Rgb> = None;
    for stroke in &plot.strokes {
        if last_stroke != Some(stroke.rgb) {
            set_stroke_color(out, options.style, stroke.rgb);
            last_stroke = Some(stroke.rgb);
        }
        if !emit_path(mapper, out, &stroke.points, stroke.closed) {
            report
                .warnings
                .push("Skipped non-finite plot coordinates".into());
        }
    }
    for fill in &plot.fills {
        set_stroke_color(out, options.style, fill.rgb);
        set_fill_color(out, options.style, fill.rgb);
        if !emit_fill(mapper, out, fill) {
            report
                .warnings
                .push("Skipped non-finite plot coordinates".into());
        }
    }
}

fn emit_path(mapper: &PageMap, out: &mut String, points: &[Point2], closed: bool) -> bool {
    let mut mapped = Vec::with_capacity(points.len());
    for point in points {
        let Some(page) = mapper.map(*point) else {
            return false;
        };
        mapped.push(page);
    }
    let Some(first) = mapped.first() else {
        return true;
    };
    out.push_str(&format!("{:.4} {:.4} m\n", first.0, first.1));
    for (x, y) in mapped.iter().skip(1) {
        out.push_str(&format!("{x:.4} {y:.4} l\n"));
    }
    if closed {
        out.push_str("h\n");
    }
    out.push_str("S\n");
    true
}

fn emit_fill(mapper: &PageMap, out: &mut String, fill: &PlotFill) -> bool {
    let mut any = false;
    for contour in &fill.contours {
        let mut mapped = Vec::with_capacity(contour.len());
        for point in contour {
            let Some(page) = mapper.map(*point) else {
                return false;
            };
            mapped.push(page);
        }
        let Some(first) = mapped.first() else {
            continue;
        };
        out.push_str(&format!("{:.4} {:.4} m\n", first.0, first.1));
        for (x, y) in mapped.iter().skip(1) {
            out.push_str(&format!("{x:.4} {y:.4} l\n"));
        }
        out.push_str("h\n");
        any = true;
    }
    if !any {
        return true;
    }
    if fill.even_odd {
        out.push_str("f*\n");
    } else {
        out.push_str("f\n");
    }
    true
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
    use crate::options::{PdfOrientation, PdfPaperSize, PDF_STROKE_HEAVY_PT};
    use cad_core::{
        default_extrusion, plot_geometry, BlockDefinition, CadColor, Entity, Geometry, HatchData,
        HatchEdge, HatchPath, Layer, LineType, MTextData, Point3, PolyVertex, TextData,
    };
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

    fn layer0(document: &mut Document) {
        document.layers.insert(
            "0".into(),
            Layer {
                name: "0".into(),
                visible: true,
                frozen: false,
                color: CadColor::Aci(7),
                linetype: "CONTINUOUS".into(),
            },
        );
    }

    fn line_document() -> Document {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(10.0, 0.0),
        }));
        document
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

    fn stream_has_invalid_number(pdf: &str) -> bool {
        pdf.split_whitespace().any(|token| {
            let lower = token.to_ascii_lowercase();
            lower.contains("nan") || lower.contains("inf")
        })
    }

    fn insert_entity(
        name: &str,
        insertion: Point3,
        scale: Point3,
        rotation: f64,
        attribs: Vec<TextData>,
    ) -> Entity {
        Entity::new(Geometry::Insert {
            block_name: name.into(),
            insertion,
            scale,
            rotation,
            extrusion: default_extrusion(),
            attribs,
            column_count: 1,
            row_count: 1,
            column_spacing: 0.0,
            row_spacing: 0.0,
            configuration: None,
        })
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

    #[test]
    fn text_only_drawing_far_from_origin_fits() {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(Entity::new(Geometry::Text(TextData {
            insertion: Point3::from_xy(50_000.0, 80_000.0),
            height: 250.0,
            rotation: 0.0,
            value: "TITLE".into(),
            extrusion: default_extrusion(),
            is_attrib_def: false,
        })));
        let options = PdfExportOptions::default();
        let (report, text) = write_pdf(&document, &options);
        assert_eq!(report.entities_written, 1);
        assert!(count_op(&text, "m") > 0);
        let (width, height) = options.page_size_pt();
        let margin = options.margin_pt();
        for (x, y) in path_points(&text) {
            assert!(x >= margin - 1e-3 && x <= width - margin + 1e-3);
            assert!(y >= margin - 1e-3 && y <= height - margin + 1e-3);
        }
        assert!(!stream_has_invalid_number(&text));
    }

    #[test]
    fn mtext_exports_every_line() {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(Entity::new(Geometry::MText(MTextData {
            insertion: Point3::from_xy(0.0, 0.0),
            height: 2.0,
            rotation: 0.0,
            width: 40.0,
            value: "ONE\\PTWO".into(),
            extrusion: default_extrusion(),
        })));
        let plot = plot_geometry(&document);
        let (_, text) = write_pdf(&document, &PdfExportOptions::default());
        assert_eq!(count_op(&text, "m"), plot.strokes.len());
        assert!(plot.strokes.len() > 4);
    }

    #[test]
    fn insert_exports_attrib_values_not_attdef() {
        let mut document = Document::default();
        layer0(&mut document);
        document.blocks.insert(
            "EQ".into(),
            BlockDefinition {
                name: "EQ".into(),
                base_pt: Point3::from_xy(0.0, 0.0),
                entities: vec![
                    Entity::new(Geometry::Line {
                        start: Point3::from_xy(0.0, 0.0),
                        end: Point3::from_xy(4.0, 0.0),
                    }),
                    Entity::new(Geometry::Text(TextData {
                        insertion: Point3::from_xy(0.0, 1.0),
                        height: 1.0,
                        rotation: 0.0,
                        value: "PLACEHOLDER".into(),
                        extrusion: default_extrusion(),
                        is_attrib_def: true,
                    })),
                ],
                ..Default::default()
            },
        );
        document.add_entity(insert_entity(
            "EQ",
            Point3::from_xy(10.0, 0.0),
            Point3::new(1.0, 1.0, 1.0),
            0.0,
            vec![TextData {
                insertion: Point3::from_xy(10.0, 1.0),
                height: 1.0,
                rotation: 0.0,
                value: "P-101".into(),
                extrusion: default_extrusion(),
                is_attrib_def: false,
            }],
        ));
        let with_attrib = plot_geometry(&document);
        let mut definition_only = Document::default();
        layer0(&mut definition_only);
        definition_only
            .blocks
            .insert("EQ".into(), document.blocks.get("EQ").unwrap().clone());
        definition_only.add_entity(insert_entity(
            "EQ",
            Point3::from_xy(10.0, 0.0),
            Point3::new(1.0, 1.0, 1.0),
            0.0,
            Vec::new(),
        ));
        let without_attrib = plot_geometry(&definition_only);
        assert!(with_attrib.strokes.len() > without_attrib.strokes.len());
        let (_, text) = write_pdf(&document, &PdfExportOptions::default());
        assert_eq!(count_op(&text, "m"), with_attrib.strokes.len());
    }

    #[test]
    fn rotated_scaled_mirrored_insert_matches_plot_stream() {
        let mut document = Document::default();
        layer0(&mut document);
        document.blocks.insert(
            "LEAF".into(),
            BlockDefinition {
                name: "LEAF".into(),
                base_pt: Point3::from_xy(0.0, 0.0),
                entities: vec![Entity::new(Geometry::Line {
                    start: Point3::from_xy(0.0, 0.0),
                    end: Point3::from_xy(10.0, 0.0),
                })],
                ..Default::default()
            },
        );
        document.add_entity(insert_entity(
            "LEAF",
            Point3::from_xy(5.0, 2.0),
            Point3::new(-2.0, 2.0, 1.0),
            std::f64::consts::FRAC_PI_4,
            Vec::new(),
        ));
        let plot = plot_geometry(&document);
        let (_, text) = write_pdf(&document, &PdfExportOptions::default());
        assert_eq!(count_op(&text, "m"), plot.strokes.len());
        assert_eq!(plot.strokes.len(), 1);
        let start = plot.strokes[0].points[0];
        let end = *plot.strokes[0].points.last().unwrap();
        assert!(start.distance(end) > 1.0);
    }

    #[test]
    fn bulged_polyline_matches_viewport_side() {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(Entity::new(Geometry::LwPolyline {
            vertices: vec![
                PolyVertex {
                    point: Point3::from_xy(0.0, 0.0),
                    bulge: 1.0,
                    vertex_id: Default::default(),
                },
                PolyVertex {
                    point: Point3::from_xy(2.0, 0.0),
                    bulge: 0.0,
                    vertex_id: Default::default(),
                },
            ],
            closed: false,
            extrusion: default_extrusion(),
            linetype_generation_continuous: false,
        }));
        let plot = plot_geometry(&document);
        let mid = plot.strokes[0].points[plot.strokes[0].points.len() / 2];
        assert!(mid.y < 0.0, "positive bulge stays below the chord");
        let (_, text) = write_pdf(&document, &PdfExportOptions::default());
        assert_eq!(count_op(&text, "m"), 1);
    }

    #[test]
    fn negative_z_polyline_orientation_matches_plot_stream() {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(Entity::new(Geometry::LwPolyline {
            vertices: vec![
                PolyVertex {
                    point: Point3::from_xy(0.0, 0.0),
                    bulge: 1.0,
                    vertex_id: Default::default(),
                },
                PolyVertex {
                    point: Point3::from_xy(2.0, 0.0),
                    bulge: 0.0,
                    vertex_id: Default::default(),
                },
            ],
            closed: false,
            extrusion: Point3::new(0.0, 0.0, -1.0),
            linetype_generation_continuous: false,
        }));
        let plot = plot_geometry(&document);
        let pts = &plot.strokes[0].points;
        assert!((pts[0].x).abs() < 1e-9);
        assert!(pts.last().unwrap().x < 0.0);
        assert!(pts[pts.len() / 2].y < 0.0);
        let (_, text) = write_pdf(&document, &PdfExportOptions::default());
        assert!(count_op(&text, "m") > 0);
    }

    fn hatch_arc_document(ccw: bool) -> Document {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(Entity::new(Geometry::Hatch(HatchData {
            extrusion: default_extrusion(),
            elevation: 0.0,
            solid_fill: false,
            paths: vec![HatchPath::Edges(vec![HatchEdge::Arc {
                center: Point3::from_xy(0.0, 0.0),
                radius: 1.0,
                start_angle: 0.0,
                end_angle: std::f64::consts::FRAC_PI_2,
                is_ccw: ccw,
            }])],
            pattern_lines: Vec::new(),
        })));
        document
    }

    #[test]
    fn hatch_circular_arc_cw_and_ccw() {
        let ccw = plot_geometry(&hatch_arc_document(true));
        let cw = plot_geometry(&hatch_arc_document(false));
        let ccw_mid = ccw.strokes[0].points[ccw.strokes[0].points.len() / 2];
        let cw_mid = cw.strokes[0].points[cw.strokes[0].points.len() / 2];
        assert!(ccw_mid.x > 0.0 && ccw_mid.y > 0.0);
        assert!(cw_mid.x < 0.0);
        let (_, ccw_pdf) = write_pdf(&hatch_arc_document(true), &PdfExportOptions::default());
        let (_, cw_pdf) = write_pdf(&hatch_arc_document(false), &PdfExportOptions::default());
        assert!(count_op(&ccw_pdf, "m") > 0);
        assert!(count_op(&cw_pdf, "m") > 0);
        assert_ne!(path_points(&ccw_pdf), path_points(&cw_pdf));
    }

    fn hatch_ellipse_document(ccw: bool) -> Document {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(Entity::new(Geometry::Hatch(HatchData {
            extrusion: default_extrusion(),
            elevation: 0.0,
            solid_fill: false,
            paths: vec![HatchPath::Edges(vec![HatchEdge::Ellipse {
                center: Point3::from_xy(0.0, 0.0),
                major_endpoint: Point3::from_xy(2.0, 0.0),
                axis_ratio: 0.5,
                start_angle: 0.0,
                end_angle: std::f64::consts::FRAC_PI_2,
                is_ccw: ccw,
            }])],
            pattern_lines: Vec::new(),
        })));
        document
    }

    #[test]
    fn hatch_elliptic_arc_cw_and_ccw() {
        let ccw = plot_geometry(&hatch_ellipse_document(true));
        let cw = plot_geometry(&hatch_ellipse_document(false));
        let ccw_mid = ccw.strokes[0].points[ccw.strokes[0].points.len() / 2];
        let cw_mid = cw.strokes[0].points[cw.strokes[0].points.len() / 2];
        assert!(ccw_mid.x > 0.0 && ccw_mid.y > 0.0);
        assert!(cw_mid.x < 0.0);
        let (_, text) = write_pdf(&hatch_ellipse_document(true), &PdfExportOptions::default());
        assert!(count_op(&text, "m") > 0);
    }

    #[test]
    fn solid_hatch_with_hole_uses_even_odd_fill() {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(Entity::new(Geometry::Hatch(HatchData {
            extrusion: default_extrusion(),
            elevation: 0.0,
            solid_fill: true,
            paths: vec![
                HatchPath::Polyline {
                    vertices: vec![
                        PolyVertex {
                            point: Point3::from_xy(0.0, 0.0),
                            bulge: 0.0,
                            vertex_id: Default::default(),
                        },
                        PolyVertex {
                            point: Point3::from_xy(10.0, 0.0),
                            bulge: 0.0,
                            vertex_id: Default::default(),
                        },
                        PolyVertex {
                            point: Point3::from_xy(10.0, 10.0),
                            bulge: 0.0,
                            vertex_id: Default::default(),
                        },
                        PolyVertex {
                            point: Point3::from_xy(0.0, 10.0),
                            bulge: 0.0,
                            vertex_id: Default::default(),
                        },
                    ],
                    closed: true,
                },
                HatchPath::Polyline {
                    vertices: vec![
                        PolyVertex {
                            point: Point3::from_xy(3.0, 3.0),
                            bulge: 0.0,
                            vertex_id: Default::default(),
                        },
                        PolyVertex {
                            point: Point3::from_xy(7.0, 3.0),
                            bulge: 0.0,
                            vertex_id: Default::default(),
                        },
                        PolyVertex {
                            point: Point3::from_xy(7.0, 7.0),
                            bulge: 0.0,
                            vertex_id: Default::default(),
                        },
                        PolyVertex {
                            point: Point3::from_xy(3.0, 7.0),
                            bulge: 0.0,
                            vertex_id: Default::default(),
                        },
                    ],
                    closed: true,
                },
            ],
            pattern_lines: Vec::new(),
        })));
        let (_, text) = write_pdf(&document, &PdfExportOptions::default());
        assert!(text.contains(" f*\n") || text.split_whitespace().any(|t| t == "f*"));
        assert_eq!(count_op(&text, "f*"), 1);
    }

    #[test]
    fn dashed_and_center_linetypes_are_not_continuous() {
        let mut document = Document::default();
        layer0(&mut document);
        document
            .linetypes
            .insert("DASHED".into(), LineType::builtin("DASHED"));
        document
            .linetypes
            .insert("CENTER".into(), LineType::builtin("CENTER"));
        let mut dashed = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(100.0, 0.0),
        });
        dashed.linetype = "DASHED".into();
        document.add_entity(dashed);
        let mut center = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 10.0),
            end: Point3::from_xy(100.0, 10.0),
        });
        center.linetype = "CENTER".into();
        document.add_entity(center);
        let (report, text) = write_pdf(&document, &PdfExportOptions::default());
        assert_eq!(report.entities_written, 2);
        assert!(
            count_op(&text, "m") > 4,
            "dashed/center must emit multiple dash segments, got {}",
            count_op(&text, "m")
        );
    }

    #[test]
    fn window_clips_and_is_direction_independent() {
        let document = line_document();
        let a = Extents2::from_corners(Point2::new(2.0, -1.0), Point2::new(5.0, 1.0));
        let b = Extents2::from_corners(Point2::new(5.0, 1.0), Point2::new(2.0, -1.0));
        let mut options_a = PdfExportOptions::default();
        options_a.plot_area = PdfPlotArea::Window(a);
        let mut options_b = options_a;
        options_b.plot_area = PdfPlotArea::Window(b);
        let (_, text_a) = write_pdf(&document, &options_a);
        let (_, text_b) = write_pdf(&document, &options_b);
        assert!(text_a.contains(" re "));
        assert!(text_a.split_whitespace().any(|t| t == "W"));
        assert!(text_a.split_whitespace().any(|t| t == "n"));
        assert_eq!(path_points(&text_a), path_points(&text_b));
        let tokens: Vec<&str> = text_a.split_whitespace().collect();
        let re_at = tokens.iter().position(|t| *t == "re").expect("re");
        let width: f64 = tokens[re_at - 2].parse().unwrap();
        let height: f64 = tokens[re_at - 1].parse().unwrap();
        assert!(width > 1.0 && height > 1.0);
    }

    #[test]
    fn window_through_circle_emits_clip() {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(Entity::new(Geometry::Circle {
            center: Point3::from_xy(0.0, 0.0),
            radius: 10.0,
            extrusion: default_extrusion(),
        }));
        let mut options = PdfExportOptions::default();
        options.plot_area = PdfPlotArea::Window(Extents2::from_corners(
            Point2::new(-2.0, -2.0),
            Point2::new(2.0, 2.0),
        ));
        let (_, text) = write_pdf(&document, &options);
        assert!(text.split_whitespace().any(|t| t == "W"));
        assert!(count_op(&text, "m") > 0);
    }

    #[test]
    fn large_coordinates_are_stable() {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(1.0e7, 2.0e7),
            end: Point3::from_xy(1.0e7 + 50.0, 2.0e7 + 10.0),
        }));
        let (report, text) = write_pdf(&document, &PdfExportOptions::default());
        assert_eq!(report.entities_written, 1);
        assert!(!stream_has_invalid_number(&text));
        assert!(count_op(&text, "m") > 0);
    }

    #[test]
    fn non_finite_coordinates_are_skipped_with_warning() {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(1.0, 0.0),
        }));
        document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(f64::NAN, 0.0),
            end: Point3::from_xy(1.0, 0.0),
        }));
        let (report, text) = write_pdf(&document, &PdfExportOptions::default());
        assert!(!report.warnings.is_empty());
        assert!(!stream_has_invalid_number(&text));
        assert_eq!(count_op(&text, "m"), 1);
    }

    #[test]
    fn a3_landscape_is_centered() {
        let mut options = PdfExportOptions::default();
        options.paper = PdfPaperSize::A3;
        options.orientation = PdfOrientation::Landscape;
        options.center_plot = true;
        let (width, height) = options.page_size_pt();
        assert!(width > height);
        let (_, text) = write_pdf(&line_document(), &options);
        let points = path_points(&text);
        let xs: Vec<f64> = points.iter().map(|p| p.0).collect();
        let min_x = xs.iter().copied().fold(f64::INFINITY, f64::min);
        let max_x = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let mid = (min_x + max_x) * 0.5;
        assert!(
            (mid - width * 0.5).abs() < 2.0,
            "drawing should be centered, mid={mid} page={width}"
        );
        assert!(text.contains(&format!("MediaBox [0 0 {width:.2} {height:.2}]")));
    }

    #[test]
    fn a4_portrait_is_centered() {
        let options = PdfExportOptions::default();
        let (width, height) = options.page_size_pt();
        assert!(height > width);
        let (_, text) = write_pdf(&line_document(), &options);
        let points = path_points(&text);
        let ys: Vec<f64> = points.iter().map(|p| p.1).collect();
        let min_y = ys.iter().copied().fold(f64::INFINITY, f64::min);
        let max_y = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let mid = (min_y + max_y) * 0.5;
        assert!((mid - height * 0.5).abs() < 2.0);
    }

    #[test]
    fn stroke_thickness_is_written() {
        let mut options = PdfExportOptions::default();
        options.stroke_pt = PDF_STROKE_HEAVY_PT;
        let (_, text) = write_pdf(&line_document(), &options);
        assert!(text.contains("1.0000 w"));
    }

    #[test]
    fn plot_stream_contains_no_invalid_numbers() {
        let (_, text) = write_pdf(&line_document(), &PdfExportOptions::default());
        assert!(!stream_has_invalid_number(&text));
    }
}
