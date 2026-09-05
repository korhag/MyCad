//! Device-independent vectorization shared by the viewport and PDF.
//!
//! Entity visibility, INSERT attribs, ATTDEF skipping, curve sampling,
//! hatch OCS, and linetype resolution live here so screen and plot cannot
//! diverge.

use crate::color::{CadColor, Rgb};
use crate::curves::{
    arc_points, bspline_points, circle_points, ellipse_points, polyline_points, CIRCLE_SEGMENTS,
};
use crate::dash::{
    arc_path_segs, circle_path_segs, generate_path_dashes, line_chain, polyline_path_segs,
    scaled_pattern, PathSeg,
};
use crate::document::Document;
use crate::entity::{Entity, Geometry, TextData};
use crate::extents::Extents2;
use crate::geom::{Point2, Point3};
use crate::hatch::hatch_path_points;
use crate::linetype::LineType;
use crate::stroke_font::{strip_mtext, stroke_text};
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
}

fn continuous() -> LineType {
    LineType::continuous("CONTINUOUS")
}

fn emit_text(
    sink: &mut impl VectorSink,
    transform: Transform2,
    rgb: Rgb,
    insertion: Point3,
    height: f64,
    rotation: f64,
    value: &str,
) {
    let origin = transform.apply(insertion.xy());
    let h = height * transform.scale_y().abs().max(transform.scale_x().abs());
    let rot = rotation + transform.rotation_component();
    for [a, b] in stroke_text(origin, h.max(1e-6), rot, value) {
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
    let mut y_off = 0.0;
    for line in cleaned.lines() {
        let insertion = Point3::new(text.insertion.x, text.insertion.y - y_off, text.insertion.z);
        emit_text(
            sink,
            transform,
            rgb,
            insertion,
            text.height,
            text.rotation,
            line,
        );
        y_off += text.height * 1.6;
    }
}

fn emit_attrib(sink: &mut impl VectorSink, transform: Transform2, rgb: Rgb, attrib: &TextData) {
    emit_text(
        sink,
        transform,
        rgb,
        attrib.insertion,
        attrib.height,
        attrib.rotation,
        &attrib.value,
    );
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
    let linetype_name = document.resolved_linetype_name(entity, block_linetype);
    let linetype = document
        .linetype(&linetype_name)
        .cloned()
        .unwrap_or_else(|| LineType::continuous(&linetype_name));
    let scale = document.effective_linetype_scale(entity);

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
            if stack.iter().any(|n| n.eq_ignore_ascii_case(block_name)) {
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
        Geometry::Dimension { block_name } => {
            if stack.iter().any(|n| n.eq_ignore_ascii_case(block_name)) {
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
            let pts: Vec<Point2> = circle_points(*center, *radius, *extrusion, CIRCLE_SEGMENTS)
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
                CIRCLE_SEGMENTS,
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
                CIRCLE_SEGMENTS,
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
            let pts: Vec<Point2> = polyline_points(vertices, *closed, *extrusion)
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
            let pts: Vec<Point2> = polyline_points(vertices, *closed, extrusion)
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
                bspline_points(*degree, control_points, knots, weights, 24)
            } else {
                fit_points.iter().map(|p| p.xy()).collect()
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
            emit_text(
                sink,
                transform,
                rgb,
                text.insertion,
                text.height,
                text.rotation,
                &text.value,
            );
        }
        Geometry::MText(text) => {
            emit_mtext(sink, transform, rgb, text);
        }
        Geometry::Hatch(hatch) => {
            let mut contours = Vec::new();
            for path in &hatch.paths {
                let world: Vec<Point2> = hatch_path_points(path, hatch.extrusion, hatch.elevation)
                    .into_iter()
                    .map(|p| transform.apply(p))
                    .collect();
                sink.path(
                    &world,
                    true,
                    &line_chain(&world, true),
                    true,
                    rgb,
                    &continuous(),
                    1.0,
                );
                contours.push(world);
            }
            if hatch.solid_fill {
                sink.fill_even_odd(&contours, rgb);
            }
        }
        Geometry::Solid { corners, .. } => {
            let pts: Vec<Point2> = corners.iter().map(|c| transform.apply(c.xy())).collect();
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
}

fn points_usable(pts: &[Point2]) -> bool {
    !pts.is_empty() && pts.iter().all(|p| p.is_finite())
}

impl VectorSink for CollectingSink {
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
            });
            return;
        }
        let pattern = scaled_pattern(&linetype.dashes, scale);
        for (a, b) in generate_path_dashes(segs, &pattern, plinegen, CIRCLE_SEGMENTS) {
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
    use crate::entity::{default_extrusion, HatchData, HatchPath, MTextData, PolyVertex, TextData};

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
        })));
        let plot = plot_geometry(&document);
        let one = stroke_text(Point2::new(0.0, 0.0), 2.0, 0.0, "ONE");
        let two = stroke_text(Point2::new(0.0, -3.2), 2.0, 0.0, "TWO");
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
                        },
                        PolyVertex {
                            point: Point3::from_xy(10.0, 0.0),
                            bulge: 0.0,
                        },
                        PolyVertex {
                            point: Point3::from_xy(10.0, 10.0),
                            bulge: 0.0,
                        },
                        PolyVertex {
                            point: Point3::from_xy(0.0, 10.0),
                            bulge: 0.0,
                        },
                    ],
                    closed: true,
                },
                HatchPath::Polyline {
                    vertices: vec![
                        PolyVertex {
                            point: Point3::from_xy(3.0, 3.0),
                            bulge: 0.0,
                        },
                        PolyVertex {
                            point: Point3::from_xy(7.0, 3.0),
                            bulge: 0.0,
                        },
                        PolyVertex {
                            point: Point3::from_xy(7.0, 7.0),
                            bulge: 0.0,
                        },
                        PolyVertex {
                            point: Point3::from_xy(3.0, 7.0),
                            bulge: 0.0,
                        },
                    ],
                    closed: true,
                },
            ],
            pattern_lines: Vec::new(),
        })));
        let plot = plot_geometry(&document);
        assert_eq!(plot.fills.len(), 1);
        assert!(plot.fills[0].even_odd);
        assert_eq!(plot.fills[0].contours.len(), 2);
    }
}
