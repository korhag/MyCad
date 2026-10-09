//! Device-independent vectorization shared by the viewport and PDF.
//!
//! Entity visibility, INSERT attribs, ATTDEF skipping, curve sampling,
//! hatch OCS, and linetype resolution live here so screen and plot cannot
//! diverge.

use crate::color::{CadColor, Rgb};
use crate::curves::{
    arc_points, bspline_points, catmull_rom_fit_points, circle_points, ellipse_points,
    polyline_points_with_tolerance, segments_per_turn, spline_sample_count,
};
use crate::dash::{
    arc_path_segs, circle_path_segs, generate_path_dashes_with_tolerance, line_chain,
    polyline_path_segs, scaled_pattern, PathSeg,
};
use crate::document::Document;
use crate::entity::{Entity, Geometry, TextData, TextHAlign, TextVAlign};
use crate::extents::Extents2;
use crate::geom::{ocs_to_wcs, Point2, Point3};
use crate::hatch::{
    hatch_fill_contours, hatch_path_points_with_tolerance, hatch_pattern_segments, GradientRamp,
};
use crate::linetype::LineType;
use crate::stroke_font::{measure_styled_width, strip_mtext, stroke_text_styled};
use crate::transform::Transform2;

// ------------------------------------------------------------
// Enum: VectorVisibility
// Purpose: Viewport uses layer visibility; PDF uses plottable layers.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorVisibility {
    Viewport,
    Plot,
}

impl VectorVisibility {
    fn layer_allowed(self, document: &Document, layer: &str) -> bool {
        match self {
            Self::Viewport => document.layer_is_visible(layer),
            Self::Plot => document.layer_is_plottable(layer),
        }
    }
}

// ------------------------------------------------------------
// Trait: VectorSink
// Purpose: Consume world-space strokes and fills. GPU tessellation
//          and PDF collection are adapters over this walk.
// ------------------------------------------------------------
pub trait VectorSink {
    fn path(
        &mut self,
        pts: &[Point2],
        closed: bool,
        segs: &[PathSeg],
        plinegen: bool,
        rgb: Rgb,
        linetype: &LineType,
        scale: f64,
    );

    fn fill(&mut self, pts: &[Point2], rgb: Rgb);

    fn fill_even_odd(&mut self, contours: &[Vec<Point2>], rgb: Rgb) {
        for contour in contours {
            self.fill(contour, rgb);
        }
    }

    /// A gradient fill. Plot and PDF use the color at the middle of the ramp.
    fn fill_gradient(&mut self, contours: &[Vec<Point2>], ramp: &GradientRamp) {
        self.fill_even_odd(contours, ramp.midpoint_color());
    }

    /// Hatch boundaries are pickable but not drawn. AutoCAD does not stroke them.
    fn pick_outline(&mut self, _pts: &[Point2], _closed: bool) {}

    fn stroke_mm(&mut self, _mm: f64) {}
}

fn continuous() -> LineType {
    LineType::continuous("CONTINUOUS")
}

fn resolved_stroke_mm(document: &Document, entity: &Entity) -> f64 {
    let raw = match entity.lineweight {
        crate::LINEWEIGHT_BYLAYER => document
            .layer(&entity.layer)
            .map(|layer| layer.lineweight)
            .unwrap_or(crate::LINEWEIGHT_DEFAULT),
        crate::LINEWEIGHT_BYBLOCK | crate::LINEWEIGHT_DEFAULT => crate::LINEWEIGHT_DEFAULT,
        other => other,
    };
    if raw < 0 {
        0.0
    } else {
        f64::from(raw) / 100.0
    }
}

fn local_chord_tolerance(document: &Document, transform: Transform2) -> f64 {
    let scale = transform
        .scale_x()
        .abs()
        .max(transform.scale_y().abs())
        .max(1e-12);
    document.display_chord_tolerance() / scale
}

fn emit_text_data(sink: &mut impl VectorSink, transform: Transform2, rgb: Rgb, text: &TextData) {
    let placed = place_text(text, transform);
    emit_styled_line(
        sink,
        rgb,
        placed.origin,
        placed.height,
        placed.rotation,
        placed.width_factor,
        placed.oblique,
        &placed.value,
    );
}

struct PlacedText {
    origin: Point2,
    height: f64,
    rotation: f64,
    width_factor: f64,
    oblique: f64,
    value: String,
}

fn place_text(text: &TextData, transform: Transform2) -> PlacedText {
    let scale = transform
        .scale_x()
        .abs()
        .max(transform.scale_y().abs())
        .max(1e-12);
    let height = (text.height * scale).max(1e-6);
    let insertion = transform.apply(text.insertion.xy());
    let alignment = transform.apply(text.alignment.xy());
    let (rotation, width_factor, origin_anchor) =
        if matches!(text.halign, TextHAlign::Aligned | TextHAlign::Fit) {
            let delta = Point2::new(alignment.x - insertion.x, alignment.y - insertion.y);
            let distance = delta.distance(Point2::new(0.0, 0.0)).max(1e-9);
            let rotation = delta.y.atan2(delta.x);
            let natural = measure_styled_width(&text.value, height, 1.0).max(1e-9);
            if matches!(text.halign, TextHAlign::Aligned) {
                let fitted_height = height * (distance / natural);
                return PlacedText {
                    origin: insertion,
                    height: fitted_height.max(1e-6),
                    rotation,
                    width_factor: 1.0,
                    oblique: text.oblique,
                    value: text.value.clone(),
                };
            }
            (rotation, distance / natural, insertion)
        } else {
            (
                text.rotation + transform.rotation_component(),
                text.width_factor.max(1e-6),
                if text.halign.uses_alignment_point() || text.valign.uses_alignment_point() {
                    alignment
                } else {
                    insertion
                },
            )
        };
    let width = measure_styled_width(&text.value, height, width_factor);
    let (dx, dy) = text_anchor_offset(text.halign, text.valign, width, height);
    let (sin, cos) = rotation.sin_cos();
    PlacedText {
        origin: Point2::new(
            origin_anchor.x + dx * cos - dy * sin,
            origin_anchor.y + dx * sin + dy * cos,
        ),
        height,
        rotation,
        width_factor,
        oblique: text.oblique,
        value: text.value.clone(),
    }
}

fn text_anchor_offset(
    halign: TextHAlign,
    valign: TextVAlign,
    width: f64,
    height: f64,
) -> (f64, f64) {
    if matches!(halign, TextHAlign::Middle) {
        return (-width * 0.5, -height * 0.5);
    }
    let dx = match halign {
        TextHAlign::Center => -width * 0.5,
        TextHAlign::Right => -width,
        _ => 0.0,
    };
    let dy = match valign {
        TextVAlign::Middle => -height * 0.5,
        TextVAlign::Top => -height,
        TextVAlign::Baseline | TextVAlign::Bottom => 0.0,
    };
    (dx, dy)
}

fn emit_styled_line(
    sink: &mut impl VectorSink,
    rgb: Rgb,
    origin: Point2,
    height: f64,
    rotation: f64,
    width_factor: f64,
    oblique: f64,
    value: &str,
) {
    for [a, b] in stroke_text_styled(origin, height, rotation, width_factor, oblique, value) {
        sink.path(
            &[a, b],
            false,
            &[PathSeg::Line { a, b }],
            true,
            rgb,
            &continuous(),
            1.0,
        );
    }
}

fn emit_mtext(
    sink: &mut impl VectorSink,
    transform: Transform2,
    rgb: Rgb,
    text: &crate::entity::MTextData,
) {
    let cleaned = strip_mtext(&text.value);
    let scale = transform
        .scale_x()
        .abs()
        .max(transform.scale_y().abs())
        .max(1e-12);
    let height = (text.height * scale).max(1e-6);
    let rotation = text.rotation + transform.rotation_component();
    let width_limit = text.width.abs() * scale;
    let lines = wrap_mtext(&cleaned, height, width_limit);
    let step = height * text.line_spacing.max(0.25) * (5.0 / 3.0);
    let anchor = transform.apply(text.insertion.xy());
    let (sin, cos) = rotation.sin_cos();
    let baseline = mtext_first_baseline(text.attachment, lines.len(), height, step);
    for (index, line) in lines.iter().enumerate() {
        let line_width = measure_styled_width(line, height, 1.0);
        let dx = mtext_line_dx(text.attachment, line_width);
        let dy = baseline - index as f64 * step;
        let origin = Point2::new(
            anchor.x + dx * cos - dy * sin,
            anchor.y + dx * sin + dy * cos,
        );
        emit_styled_line(sink, rgb, origin, height, rotation, 1.0, 0.0, line);
    }
}

fn wrap_mtext(text: &str, height: f64, width_limit: f64) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if width_limit <= 1e-9 {
            lines.push(paragraph.to_string());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let trial = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if current.is_empty() || measure_styled_width(&trial, height, 1.0) <= width_limit {
                current = trial;
            } else {
                lines.push(std::mem::take(&mut current));
                current = word.to_string();
            }
        }
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn mtext_line_dx(attachment: i16, line_width: f64) -> f64 {
    match (attachment.clamp(1, 9) - 1) % 3 {
        1 => -line_width * 0.5,
        2 => -line_width,
        _ => 0.0,
    }
}

fn mtext_first_baseline(attachment: i16, line_count: usize, cap: f64, step: f64) -> f64 {
    let lines = line_count.max(1) as f64;
    let block = cap + (lines - 1.0) * step;
    match (attachment.clamp(1, 9) - 1) / 3 {
        0 => -cap,
        1 => -cap + block * 0.5,
        _ => (lines - 1.0) * step,
    }
}

fn emit_attrib(sink: &mut impl VectorSink, transform: Transform2, rgb: Rgb, attrib: &TextData) {
    if attrib
        .attribute
        .as_ref()
        .is_some_and(|info| info.invisible())
    {
        return;
    }
    emit_text_data(sink, transform, rgb, attrib);
}

// ------------------------------------------------------------
// Function: vectorize_entity
// Purpose: Walk one entity, including nested INSERT/DIMENSION,
//          into world-space strokes and fills.
// ------------------------------------------------------------
pub fn vectorize_entity(
    document: &Document,
    entity: &Entity,
    transform: Transform2,
    block_color: CadColor,
    block_linetype: &str,
    stack: &mut Vec<String>,
    visibility: VectorVisibility,
    sink: &mut impl VectorSink,
) {
    if !entity.visible || !visibility.layer_allowed(document, &entity.layer) {
        return;
    }
    let layer_color = document
        .layer(&entity.layer)
        .map(|l| l.color)
        .unwrap_or(CadColor::Aci(7));
    let rgb = entity.color.resolve(layer_color, block_color);
    sink.stroke_mm(resolved_stroke_mm(document, entity));
    let linetype_name = document.resolved_linetype_name(entity, block_linetype);
    let linetype = document
        .linetype(&linetype_name)
        .cloned()
        .unwrap_or_else(|| LineType::continuous(&linetype_name));
    let scale = document.effective_linetype_scale(entity);
    let chord_tol = local_chord_tolerance(document, transform);

    match &entity.geometry {
        Geometry::Insert {
            block_name,
            insertion,
            scale: ins_scale,
            rotation,
            extrusion,
            attribs,
            column_count,
            row_count,
            column_spacing,
            row_spacing,
            configuration: _,
        } => {
            if crate::nesting_too_deep(stack)
                || stack.iter().any(|n| n.eq_ignore_ascii_case(block_name))
            {
                return;
            }
            let Some(block) = document.blocks.get(block_name) else {
                return;
            };
            stack.push(block_name.clone());
            let inherit = match entity.color {
                CadColor::ByLayer | CadColor::ByBlock => block_color,
                other => other,
            };
            let inherit_lt = document.resolved_linetype_name(entity, block_linetype);
            let (cols, rows) = crate::clamped_array_counts(*column_count, *row_count);
            for col in 0..cols {
                for row in 0..rows {
                    let extra = Transform2::translate(
                        col as f64 * *column_spacing,
                        row as f64 * *row_spacing,
                    );
                    let nested = transform.then(
                        Transform2::block_insert(
                            *insertion,
                            *ins_scale,
                            *rotation,
                            *extrusion,
                            block.base_pt,
                        )
                        .then(extra),
                    );
                    for child in &block.entities {
                        vectorize_entity(
                            document,
                            child,
                            nested,
                            inherit,
                            inherit_lt.as_str(),
                            stack,
                            visibility,
                            sink,
                        );
                    }
                }
            }
            for attrib in attribs {
                emit_attrib(sink, transform, rgb, attrib);
            }
            stack.pop();
        }
        Geometry::Dimension(data) => {
            let block_name = &data.block_name;
            if crate::nesting_too_deep(stack)
                || stack.iter().any(|n| n.eq_ignore_ascii_case(block_name))
            {
                return;
            }
            if let Some(block) = document.blocks.get(block_name) {
                stack.push(block_name.clone());
                for child in &block.entities {
                    vectorize_entity(
                        document,
                        child,
                        transform,
                        block_color,
                        block_linetype,
                        stack,
                        visibility,
                        sink,
                    );
                }
                stack.pop();
            }
        }
        Geometry::Line { start, end } => {
            let a = transform.apply(start.xy());
            let b = transform.apply(end.xy());
            sink.path(
                &[a, b],
                false,
                &[PathSeg::Line { a, b }],
                true,
                rgb,
                &linetype,
                scale,
            );
        }
        Geometry::Point { position } => {
            let p = transform.apply(position.xy());
            let s = transform.scale_x().abs().max(0.1) * 0.5;
            sink.path(
                &[Point2::new(p.x - s, p.y), Point2::new(p.x + s, p.y)],
                false,
                &[PathSeg::Line {
                    a: Point2::new(p.x - s, p.y),
                    b: Point2::new(p.x + s, p.y),
                }],
                true,
                rgb,
                &continuous(),
                1.0,
            );
            sink.path(
                &[Point2::new(p.x, p.y - s), Point2::new(p.x, p.y + s)],
                false,
                &[PathSeg::Line {
                    a: Point2::new(p.x, p.y - s),
                    b: Point2::new(p.x, p.y + s),
                }],
                true,
                rgb,
                &continuous(),
                1.0,
            );
        }
        Geometry::Circle {
            center,
            radius,
            extrusion,
        } => {
            let pts: Vec<Point2> = circle_points(
                *center,
                *radius,
                *extrusion,
                segments_per_turn(*radius, chord_tol),
            )
            .into_iter()
            .map(|p| transform.apply(p))
            .collect();
            sink.path(
                &pts,
                true,
                &circle_path_segs(&pts),
                true,
                rgb,
                &linetype,
                scale,
            );
        }
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            extrusion,
        } => {
            let pts: Vec<Point2> = arc_points(
                *center,
                *radius,
                *start_angle,
                *end_angle,
                true,
                *extrusion,
                segments_per_turn(*radius, chord_tol),
            )
            .into_iter()
            .map(|p| transform.apply(p))
            .collect();
            sink.path(
                &pts,
                false,
                &arc_path_segs(&pts),
                true,
                rgb,
                &linetype,
                scale,
            );
        }
        Geometry::Ellipse {
            center,
            major_axis,
            axis_ratio,
            start_param,
            end_param,
            extrusion,
        } => {
            let pts: Vec<Point2> = ellipse_points(
                *center,
                *major_axis,
                *axis_ratio,
                *start_param,
                *end_param,
                *extrusion,
                segments_per_turn(major_axis.length(), chord_tol),
            )
            .into_iter()
            .map(|p| transform.apply(p))
            .collect();
            sink.path(
                &pts,
                false,
                &line_chain(&pts, false),
                true,
                rgb,
                &linetype,
                scale,
            );
        }
        Geometry::LwPolyline {
            vertices,
            closed,
            extrusion,
            linetype_generation_continuous,
        } => {
            let pts: Vec<Point2> =
                polyline_points_with_tolerance(vertices, *closed, *extrusion, Some(chord_tol))
                    .into_iter()
                    .map(|p| transform.apply(p))
                    .collect();
            let segs = polyline_path_segs(vertices, *closed, *extrusion, transform);
            sink.path(
                &pts,
                *closed,
                &segs,
                *linetype_generation_continuous,
                rgb,
                &linetype,
                scale,
            );
        }
        Geometry::Polyline {
            vertices,
            closed,
            linetype_generation_continuous,
        } => {
            let extrusion = Point3::new(0.0, 0.0, 1.0);
            let pts: Vec<Point2> =
                polyline_points_with_tolerance(vertices, *closed, extrusion, Some(chord_tol))
                    .into_iter()
                    .map(|p| transform.apply(p))
                    .collect();
            let segs = polyline_path_segs(vertices, *closed, extrusion, transform);
            sink.path(
                &pts,
                *closed,
                &segs,
                *linetype_generation_continuous,
                rgb,
                &linetype,
                scale,
            );
        }
        Geometry::Spline {
            degree,
            control_points,
            fit_points,
            knots,
            weights,
            closed,
        } => {
            let sampled = if control_points.len() >= 2 {
                bspline_points(
                    *degree,
                    control_points,
                    knots,
                    weights,
                    spline_sample_count(control_points.len(), *degree),
                )
            } else {
                catmull_rom_fit_points(fit_points, *closed)
            };
            let pts: Vec<Point2> = sampled.into_iter().map(|p| transform.apply(p)).collect();
            sink.path(
                &pts,
                *closed,
                &line_chain(&pts, *closed),
                true,
                rgb,
                &linetype,
                scale,
            );
        }
        Geometry::Text(text) => {
            if text.is_attrib_def && !stack.is_empty() {
                return;
            }
            if text.attribute.as_ref().is_some_and(|info| info.invisible()) {
                return;
            }
            emit_text_data(sink, transform, rgb, text);
        }
        Geometry::MText(text) => {
            emit_mtext(sink, transform, rgb, text);
        }
        Geometry::Hatch(hatch) => {
            let mut contours = Vec::new();
            for path in &hatch.paths {
                let world: Vec<Point2> = hatch_path_points_with_tolerance(
                    path,
                    hatch.extrusion,
                    hatch.elevation,
                    Some(chord_tol),
                )
                .into_iter()
                .map(|p| transform.apply(p))
                .collect();
                sink.pick_outline(&world, true);
                contours.push(world);
            }
            let filled = hatch_fill_contours(hatch.style, &contours);
            if let Some(gradient) = &hatch.gradient {
                sink.fill_gradient(&filled, &GradientRamp::from_gradient(gradient));
            } else if hatch.solid_fill {
                sink.fill_even_odd(&filled, rgb);
            } else {
                for (a, b) in hatch_pattern_segments(hatch, &filled, transform) {
                    sink.path(
                        &[a, b],
                        false,
                        &line_chain(&[a, b], false),
                        true,
                        rgb,
                        &continuous(),
                        1.0,
                    );
                }
            }
        }
        Geometry::Solid { corners, extrusion } => {
            let pts: Vec<Point2> = corners
                .iter()
                .map(|c| transform.apply(ocs_to_wcs(*c, *extrusion).xy()))
                .collect();
            sink.path(
                &pts,
                true,
                &line_chain(&pts, true),
                true,
                rgb,
                &continuous(),
                1.0,
            );
            sink.fill(&pts, rgb);
        }
        Geometry::Leader { vertices } | Geometry::MLine { vertices, .. } => {
            let pts: Vec<Point2> = vertices.iter().map(|p| transform.apply(p.xy())).collect();
            sink.path(
                &pts,
                false,
                &line_chain(&pts, false),
                true,
                rgb,
                &linetype,
                scale,
            );
        }
        Geometry::Image(frame) | Geometry::Wipeout(frame) => {
            let pts: Vec<Point2> = frame
                .corners()
                .iter()
                .map(|corner| transform.apply(corner.xy()))
                .collect();
            sink.path(
                &pts,
                true,
                &line_chain(&pts, true),
                true,
                rgb,
                &linetype,
                scale,
            );
        }
        Geometry::Viewport(viewport) => {
            let pts: Vec<Point2> = viewport
                .corners()
                .iter()
                .map(|corner| transform.apply(corner.xy()))
                .collect();
            sink.path(
                &pts,
                true,
                &line_chain(&pts, true),
                true,
                rgb,
                &linetype,
                scale,
            );
        }
    }
}

// ------------------------------------------------------------
// Type: PlotStroke / PlotFill / PlotGeometry
// Purpose: The same primitive stream used for PDF output and
//          plot-extents, so bounds and ink cannot disagree.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct PlotStroke {
    pub points: Vec<Point2>,
    pub closed: bool,
    pub rgb: Rgb,
    /// Millimetres. Zero keeps the PDF page default.
    pub width_mm: f64,
}

#[derive(Debug, Clone)]
pub struct PlotFill {
    pub contours: Vec<Vec<Point2>>,
    pub even_odd: bool,
    pub rgb: Rgb,
}

#[derive(Debug, Clone, Default)]
pub struct PlotGeometry {
    pub strokes: Vec<PlotStroke>,
    pub fills: Vec<PlotFill>,
    pub warnings: Vec<String>,
    pub entities_written: usize,
}

impl PlotGeometry {
    pub fn extents(&self) -> Option<Extents2> {
        let mut extents = Extents2::empty();
        let mut any = false;
        for stroke in &self.strokes {
            for point in &stroke.points {
                if point.is_finite() {
                    extents.include(*point);
                    any = true;
                }
            }
        }
        for fill in &self.fills {
            for contour in &fill.contours {
                for point in contour {
                    if point.is_finite() {
                        extents.include(*point);
                        any = true;
                    }
                }
            }
        }
        any.then_some(extents)
    }
}

struct CollectingSink {
    geometry: PlotGeometry,
    chord_tolerance: f64,
    stroke_mm: f64,
}

fn points_usable(pts: &[Point2]) -> bool {
    !pts.is_empty() && pts.iter().all(|p| p.is_finite())
}

impl VectorSink for CollectingSink {
    fn stroke_mm(&mut self, mm: f64) {
        self.stroke_mm = if mm.is_finite() && mm > 0.0 { mm } else { 0.0 };
    }

    fn path(
        &mut self,
        pts: &[Point2],
        closed: bool,
        segs: &[PathSeg],
        plinegen: bool,
        rgb: Rgb,
        linetype: &LineType,
        scale: f64,
    ) {
        if !points_usable(pts) {
            if pts.iter().any(|p| !p.is_finite()) {
                self.geometry
                    .warnings
                    .push("Skipped non-finite plot coordinates".into());
            }
            return;
        }
        if linetype.is_continuous() {
            self.geometry.strokes.push(PlotStroke {
                points: pts.to_vec(),
                closed,
                rgb,
                width_mm: self.stroke_mm,
            });
            return;
        }
        let pattern = scaled_pattern(&linetype.dashes, scale);
        for (a, b) in generate_path_dashes_with_tolerance(
            segs,
            &pattern,
            plinegen,
            0,
            Some(self.chord_tolerance),
        ) {
            if !a.is_finite() || !b.is_finite() {
                self.geometry
                    .warnings
                    .push("Skipped non-finite plot coordinates".into());
                continue;
            }
            self.geometry.strokes.push(PlotStroke {
                points: vec![a, b],
                closed: false,
                rgb,
                width_mm: self.stroke_mm,
            });
        }
    }

    fn fill(&mut self, pts: &[Point2], rgb: Rgb) {
        if !points_usable(pts) {
            if pts.iter().any(|p| !p.is_finite()) {
                self.geometry
                    .warnings
                    .push("Skipped non-finite plot coordinates".into());
            }
            return;
        }
        self.geometry.fills.push(PlotFill {
            contours: vec![pts.to_vec()],
            even_odd: false,
            rgb,
        });
    }

    fn fill_even_odd(&mut self, contours: &[Vec<Point2>], rgb: Rgb) {
        let mut kept = Vec::new();
        for contour in contours {
            if !points_usable(contour) {
                if contour.iter().any(|p| !p.is_finite()) {
                    self.geometry
                        .warnings
                        .push("Skipped non-finite plot coordinates".into());
                }
                continue;
            }
            kept.push(contour.clone());
        }
        if kept.is_empty() {
            return;
        }
        self.geometry.fills.push(PlotFill {
            contours: kept,
            even_odd: true,
            rgb,
        });
    }
}

// ------------------------------------------------------------
// Function: plot_geometry
// Purpose: Vectorize plottable model space for PDF extents and ink.
// ------------------------------------------------------------
pub fn plot_geometry(document: &Document) -> PlotGeometry {
    let mut sink = CollectingSink {
        geometry: PlotGeometry::default(),
        chord_tolerance: document.display_chord_tolerance(),
        stroke_mm: 0.0,
    };
    let mut stack = Vec::new();
    for entity in &document.model_space {
        let before = sink.geometry.strokes.len() + sink.geometry.fills.len();
        vectorize_entity(
            document,
            entity,
            Transform2::identity(),
            CadColor::Aci(7),
            "CONTINUOUS",
            &mut stack,
            VectorVisibility::Plot,
            &mut sink,
        );
        let after = sink.geometry.strokes.len() + sink.geometry.fills.len();
        if after > before {
            sink.geometry.entities_written += 1;
        }
    }
    sink.geometry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{BlockDefinition, Layer};
    use crate::entity::{
        default_extrusion, HatchData, HatchPath, HatchPatternLine, MTextData, PolyVertex, TextData,
        MAX_HATCH_PATTERN_SEGMENTS,
    };
    use crate::stroke_font::stroke_text;

    fn layer0(document: &mut Document) {
        document.layers.insert(
            "0".into(),
            Layer {
                name: "0".into(),
                visible: true,
                frozen: false,
                color: CadColor::Aci(7),
                linetype: "CONTINUOUS".into(),
                ..Layer::default()
            },
        );
    }

    fn stroke_has_text(geometry: &PlotGeometry, needle: &str) -> bool {
        let expected = stroke_text(Point2::new(0.0, 0.0), 2.5, 0.0, needle);
        !expected.is_empty() && geometry.strokes.len() >= expected.len()
    }

    #[test]
    fn text_only_drawing_has_plot_extents() {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(Entity::new(Geometry::Text(TextData {
            insertion: Point3::from_xy(1200.0, 3400.0),
            height: 2.5,
            rotation: 0.0,
            value: "TAG".into(),
            extrusion: default_extrusion(),
            is_attrib_def: false,
            ..Default::default()
        })));
        assert!(document.compute_extents().is_none());
        let plot = plot_geometry(&document);
        let extents = plot.extents().expect("text extents");
        assert!(extents.width() > 0.0 && extents.height() > 0.0);
        assert!(extents.min.x > 1000.0);
        assert!(stroke_has_text(&plot, "TAG"));
    }

    #[test]
    fn insert_attribs_are_vectorized_and_attdef_is_skipped() {
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
                        ..Default::default()
                    })),
                ],
                ..Default::default()
            },
        );
        document.add_entity(Entity::new(Geometry::Insert {
            block_name: "EQ".into(),
            insertion: Point3::from_xy(10.0, 0.0),
            scale: Point3::new(1.0, 1.0, 1.0),
            rotation: 0.0,
            extrusion: default_extrusion(),
            attribs: vec![TextData {
                insertion: Point3::from_xy(10.0, 1.0),
                height: 1.0,
                rotation: 0.0,
                value: "P-101".into(),
                extrusion: default_extrusion(),
                is_attrib_def: false,
                ..Default::default()
            }],
            column_count: 1,
            row_count: 1,
            column_spacing: 0.0,
            row_spacing: 0.0,
            configuration: None,
        }));
        let plot = plot_geometry(&document);
        let placeholder = stroke_text(Point2::new(0.0, 1.0), 1.0, 0.0, "PLACEHOLDER");
        let attrib = stroke_text(Point2::new(10.0, 1.0), 1.0, 0.0, "P-101");
        assert!(!placeholder.is_empty());
        assert!(!attrib.is_empty());
        let has_placeholder = plot
            .strokes
            .iter()
            .any(|stroke| stroke.points.first().copied() == placeholder.first().map(|seg| seg[0]));
        let has_attrib = plot.strokes.iter().any(|stroke| {
            stroke
                .points
                .first()
                .copied()
                .is_some_and(|p| p.distance(attrib[0][0]) < 1e-6)
        });
        assert!(!has_placeholder, "ATTDEF must not plot inside the insert");
        assert!(has_attrib, "ATTRIB value must plot");
    }

    #[test]
    fn mtext_emits_every_line() {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(Entity::new(Geometry::MText(MTextData {
            insertion: Point3::from_xy(0.0, 0.0),
            height: 2.0,
            rotation: 0.0,
            width: 40.0,
            value: "ONE\\PTWO".into(),
            extrusion: default_extrusion(),
            ..Default::default()
        })));
        let plot = plot_geometry(&document);
        let step = 2.0 * (5.0 / 3.0);
        let one = stroke_text(Point2::new(0.0, -2.0), 2.0, 0.0, "ONE");
        let two = stroke_text(Point2::new(0.0, -2.0 - step), 2.0, 0.0, "TWO");
        assert!(plot
            .strokes
            .iter()
            .any(|s| s.points[0].distance(one[0][0]) < 1e-6));
        assert!(plot
            .strokes
            .iter()
            .any(|s| s.points[0].distance(two[0][0]) < 1e-6));
    }

    #[test]
    fn dashed_linetype_is_not_a_single_continuous_stroke() {
        let mut document = Document::default();
        layer0(&mut document);
        document
            .linetypes
            .insert("DASHED".into(), LineType::builtin("DASHED"));
        let mut line = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(100.0, 0.0),
        });
        line.linetype = "DASHED".into();
        document.add_entity(line);
        let plot = plot_geometry(&document);
        assert!(
            plot.strokes.len() > 1,
            "dashed line must emit multiple dash segments, got {}",
            plot.strokes.len()
        );
    }

    #[test]
    fn solid_hatch_with_hole_is_even_odd() {
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
            ..HatchData::default()
        })));
        let plot = plot_geometry(&document);
        assert_eq!(plot.fills.len(), 1);
        assert!(plot.fills[0].even_odd);
        assert_eq!(plot.fills[0].contours.len(), 2);
        assert!(
            plot.strokes.is_empty(),
            "hatch boundaries are pick-only, not stroked"
        );
    }

    #[test]
    fn center_right_and_middle_text_use_the_alignment_point() {
        let mut document = Document::default();
        layer0(&mut document);
        let width = measure_styled_width("M", 10.0, 1.0);
        document.add_entity(Entity::new(Geometry::Text(TextData {
            insertion: Point3::from_xy(0.0, 0.0),
            alignment: Point3::from_xy(100.0, 20.0),
            height: 10.0,
            value: "M".into(),
            halign: TextHAlign::Center,
            ..TextData::default()
        })));
        document.add_entity(Entity::new(Geometry::Text(TextData {
            alignment: Point3::from_xy(40.0, 5.0),
            height: 10.0,
            value: "M".into(),
            halign: TextHAlign::Right,
            valign: TextVAlign::Top,
            ..TextData::default()
        })));
        document.add_entity(Entity::new(Geometry::Text(TextData {
            alignment: Point3::from_xy(0.0, 0.0),
            height: 10.0,
            value: "M".into(),
            halign: TextHAlign::Middle,
            ..TextData::default()
        })));
        let plot = plot_geometry(&document);
        let center = stroke_text(Point2::new(100.0 - width * 0.5, 20.0), 10.0, 0.0, "M");
        let right = stroke_text(Point2::new(40.0 - width, 5.0 - 10.0), 10.0, 0.0, "M");
        let middle = stroke_text(Point2::new(-width * 0.5, -5.0), 10.0, 0.0, "M");
        assert!(plot_has_stroke(&plot, center[0][0]));
        assert!(plot_has_stroke(&plot, right[0][0]));
        assert!(plot_has_stroke(&plot, middle[0][0]));
    }

    #[test]
    fn mtext_top_right_attachment_shifts_the_block() {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(Entity::new(Geometry::MText(MTextData {
            insertion: Point3::from_xy(50.0, 80.0),
            height: 4.0,
            value: "HI".into(),
            attachment: 3,
            ..MTextData::default()
        })));
        let width = measure_styled_width("HI", 4.0, 1.0);
        let expected = stroke_text(Point2::new(50.0 - width, 80.0 - 4.0), 4.0, 0.0, "HI");
        let plot = plot_geometry(&document);
        assert!(plot_has_stroke(&plot, expected[0][0]));
    }

    fn square_hatch_path(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> HatchPath {
        let corners = [
            (min_x, min_y),
            (max_x, min_y),
            (max_x, max_y),
            (min_x, max_y),
        ];
        HatchPath::Polyline {
            vertices: corners
                .into_iter()
                .map(|(x, y)| PolyVertex {
                    point: Point3::from_xy(x, y),
                    bulge: 0.0,
                    vertex_id: Default::default(),
                })
                .collect(),
            closed: true,
        }
    }

    fn horizontal_pattern(base: Point2, spacing: f64, dashes: Vec<f64>) -> HatchPatternLine {
        HatchPatternLine {
            angle: 0.0,
            base: Point3::from_xy(base.x, base.y),
            offset: Point3::from_xy(0.0, spacing),
            dashes,
        }
    }

    fn pattern_hatch(paths: Vec<HatchPath>, lines: Vec<HatchPatternLine>) -> Entity {
        Entity::new(Geometry::Hatch(HatchData {
            solid_fill: false,
            pattern_name: "ANSI31".into(),
            pattern_scale: 1.0,
            pattern_angle: 0.0,
            pattern_type: 1,
            paths,
            pattern_lines: lines,
            ..HatchData::default()
        }))
    }

    fn pattern_segments(plot: &PlotGeometry) -> Vec<(Point2, Point2)> {
        plot.strokes
            .iter()
            .filter_map(|stroke| {
                if stroke.points.len() == 2 {
                    Some((stroke.points[0], stroke.points[1]))
                } else {
                    None
                }
            })
            .collect()
    }

    fn near_segment(got: (Point2, Point2), a: Point2, b: Point2) -> bool {
        const EPS: f64 = 1e-4;
        (got.0.distance(a) < EPS && got.1.distance(b) < EPS)
            || (got.0.distance(b) < EPS && got.1.distance(a) < EPS)
    }

    #[test]
    fn pattern_hatch_segments_stay_inside_the_boundary() {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(pattern_hatch(
            vec![square_hatch_path(0.0, 0.0, 10.0, 10.0)],
            vec![horizontal_pattern(Point2::new(0.0, 0.0), 2.0, Vec::new())],
        ));
        let segments = pattern_segments(&plot_geometry(&document));
        assert!(!segments.is_empty());
        for (a, b) in &segments {
            for point in [*a, *b] {
                assert!(
                    (0.0..=10.0).contains(&point.x) && (0.0..=10.0).contains(&point.y),
                    "pattern segment {:?} extends outside the hatch",
                    (a, b)
                );
            }
        }
        assert!(
            segments.iter().any(|segment| near_segment(
                *segment,
                Point2::new(0.0, 4.0),
                Point2::new(10.0, 4.0)
            )),
            "expected a clipped line across y=4, got {segments:?}"
        );
    }

    #[test]
    fn pattern_hatch_skips_the_hole() {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(pattern_hatch(
            vec![
                square_hatch_path(0.0, 0.0, 10.0, 10.0),
                square_hatch_path(3.0, 3.0, 7.0, 7.0),
            ],
            vec![horizontal_pattern(Point2::new(0.0, 0.0), 2.0, Vec::new())],
        ));
        let segments = pattern_segments(&plot_geometry(&document));
        assert!(segments.iter().any(|segment| {
            near_segment(*segment, Point2::new(0.0, 4.0), Point2::new(3.0, 4.0))
        }));
        assert!(segments.iter().any(|segment| {
            near_segment(*segment, Point2::new(7.0, 4.0), Point2::new(10.0, 4.0))
        }));
        for (a, b) in &segments {
            let mid = Point2::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
            let inside_hole = mid.x > 3.0 + 1e-6
                && mid.x < 7.0 - 1e-6
                && mid.y > 3.0 + 1e-6
                && mid.y < 7.0 - 1e-6;
            assert!(!inside_hole, "segment midpoint {mid:?} falls in the hole");
        }
    }

    #[test]
    fn pattern_hatch_base_far_from_boundary_still_fills() {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(pattern_hatch(
            vec![square_hatch_path(0.0, 0.0, 10.0, 10.0)],
            vec![horizontal_pattern(
                Point2::new(-1000.0, -1000.0),
                2.0,
                Vec::new(),
            )],
        ));
        let segments = pattern_segments(&plot_geometry(&document));
        assert!(
            segments.iter().any(|segment| near_segment(
                *segment,
                Point2::new(0.0, 4.0),
                Point2::new(10.0, 4.0)
            )),
            "a base far outside the boundary must still seed lines across it, got {segments:?}"
        );
        for (a, b) in &segments {
            for point in [*a, *b] {
                assert!((0.0..=10.0).contains(&point.x) && (0.0..=10.0).contains(&point.y));
            }
        }
    }

    #[test]
    fn pattern_hatch_dashes_are_clipped_to_the_dash_length() {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(pattern_hatch(
            vec![square_hatch_path(0.0, 0.0, 10.0, 10.0)],
            vec![horizontal_pattern(
                Point2::new(0.0, 5.0),
                20.0,
                vec![1.0, -1.0],
            )],
        ));
        let segments = pattern_segments(&plot_geometry(&document));
        assert!(
            segments.len() > 1,
            "a dashed line must break into more than one segment, got {segments:?}"
        );
        for (a, b) in &segments {
            let length = a.distance(*b);
            assert!(
                length <= 1.0 + 1e-4,
                "dash length {length} exceeds the pattern dash"
            );
            assert!((0.0..=10.0).contains(&a.x) && (0.0..=10.0).contains(&b.x));
            assert!((a.y - 5.0).abs() < 1e-4 && (b.y - 5.0).abs() < 1e-4);
        }
    }

    #[test]
    fn pattern_hatch_dash_phase_follows_the_row_offset() {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(pattern_hatch(
            vec![square_hatch_path(0.0, 0.0, 10.0, 10.0)],
            vec![HatchPatternLine {
                angle: 0.0,
                base: Point3::from_xy(0.0, 0.0),
                offset: Point3::from_xy(0.5, 2.0),
                dashes: vec![1.0, -1.0],
            }],
        ));
        let segments = pattern_segments(&plot_geometry(&document));
        let row: Vec<_> = segments
            .iter()
            .copied()
            .filter(|(a, b)| (a.y - 4.0).abs() < 1e-4 && (b.y - 4.0).abs() < 1e-4)
            .collect();
        assert!(
            row.iter().any(|segment| near_segment(
                *segment,
                Point2::new(1.0, 4.0),
                Point2::new(2.0, 4.0)
            )),
            "row offset must shift the dash phase, got {row:?}"
        );
        assert!(
            row.iter().all(|(a, b)| {
                let mid_x = (a.x + b.x) * 0.5;
                !(mid_x > 0.0 && mid_x < 1.0)
            }),
            "x 0..1 on y=4 is a gap for this row offset, got {row:?}"
        );
    }

    #[test]
    fn pattern_hatch_follows_insert_transform() {
        let mut document = Document::default();
        layer0(&mut document);
        document.blocks.insert(
            "TREE".into(),
            BlockDefinition {
                name: "TREE".into(),
                base_pt: Point3::from_xy(0.0, 0.0),
                entities: vec![pattern_hatch(
                    vec![square_hatch_path(0.0, 0.0, 10.0, 10.0)],
                    vec![horizontal_pattern(Point2::new(0.0, 0.0), 2.0, Vec::new())],
                )],
                ..Default::default()
            },
        );
        document.add_entity(Entity::new(Geometry::Insert {
            block_name: "TREE".into(),
            insertion: Point3::from_xy(100.0, 200.0),
            scale: Point3::new(2.0, 2.0, 1.0),
            rotation: std::f64::consts::FRAC_PI_2,
            extrusion: default_extrusion(),
            attribs: Vec::new(),
            column_count: 1,
            row_count: 1,
            column_spacing: 0.0,
            row_spacing: 0.0,
            configuration: None,
        }));
        let transform = Transform2::block_insert(
            Point3::from_xy(100.0, 200.0),
            Point3::new(2.0, 2.0, 1.0),
            std::f64::consts::FRAC_PI_2,
            default_extrusion(),
            Point3::from_xy(0.0, 0.0),
        );
        let corners = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let world: Vec<Point2> = corners
            .into_iter()
            .map(|(x, y)| transform.apply(Point2::new(x, y)))
            .collect();
        let min_x = world.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
        let max_x = world.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
        let min_y = world.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let max_y = world.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
        let segments = pattern_segments(&plot_geometry(&document));
        assert!(!segments.is_empty());
        for (a, b) in &segments {
            for point in [*a, *b] {
                assert!(
                    point.x >= min_x - 1e-4
                        && point.x <= max_x + 1e-4
                        && point.y >= min_y - 1e-4
                        && point.y <= max_y + 1e-4,
                    "segment {:?} is outside the inserted boundary",
                    (a, b)
                );
            }
        }
    }

    #[test]
    fn dense_pattern_stays_within_the_segment_cap_and_covers_the_boundary() {
        let mut document = Document::default();
        layer0(&mut document);
        document.add_entity(pattern_hatch(
            vec![square_hatch_path(0.0, 0.0, 10.0, 10.0)],
            vec![horizontal_pattern(Point2::new(0.0, 0.0), 0.001, Vec::new())],
        ));
        let segments = pattern_segments(&plot_geometry(&document));
        assert!(segments.len() <= MAX_HATCH_PATTERN_SEGMENTS);
        assert!(segments.len() > 1);
        let min_y = segments
            .iter()
            .map(|(a, b)| (a.y + b.y) * 0.5)
            .fold(f64::INFINITY, f64::min);
        let max_y = segments
            .iter()
            .map(|(a, b)| (a.y + b.y) * 0.5)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            min_y < 1.0 && max_y > 9.0,
            "thinned pattern must still cover the boundary, y span {min_y}..{max_y}"
        );
    }

    fn plot_has_stroke(plot: &PlotGeometry, point: Point2) -> bool {
        plot.strokes.iter().any(|stroke| {
            stroke
                .points
                .first()
                .is_some_and(|start| start.distance(point) < 1e-6)
        })
    }
}
