//! Walk a cad-core document into GPU-ready line and triangle batches.
//! Geometry is sampled in f64, then stored relative to a document origin as f32.

use cad_core::{
    CadColor, Document, Entity, Geometry, HatchEdge, HatchPath, LineType, Point2, Point3, Rgb,
    Transform2,
};

use crate::curves::{
    arc_points, bspline_points, circle_points, ellipse_points, polyline_points, CIRCLE_SEGMENTS,
};
use crate::dash::{generate_path_dashes, line_chain, polyline_path_segs, scaled_pattern, PathSeg};
use crate::pick::EntityPick;
use crate::stroke_font::{strip_mtext, stroke_text};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

// ------------------------------------------------------------
// Type: DisplayList
// Purpose: Cached tessellation for the wgpu renderer. Document
//          coordinates remain f64 in cad-core; this is a display cache.
// ------------------------------------------------------------
#[derive(Clone, Default)]
pub struct DisplayList {
    pub origin: Point2,
    pub line_vertices: Vec<GpuVertex>,
    pub triangle_vertices: Vec<GpuVertex>,
    pub picks: Vec<EntityPick>,
}

impl DisplayList {
    pub fn is_empty(&self) -> bool {
        self.line_vertices.is_empty() && self.triangle_vertices.is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.line_vertices.len() / 2
    }

    pub fn pick_for(&self, entity_index: usize) -> Option<&EntityPick> {
        self.picks
            .iter()
            .find(|pick| pick.entity_index == entity_index)
    }
}

struct TessSink<'a> {
    list: &'a mut DisplayList,
    pick: Option<&'a mut EntityPick>,
}

impl TessSink<'_> {
    fn polyline(
        &mut self,
        pts: &[Point2],
        closed: bool,
        rgb: Rgb,
        linetype: &LineType,
        scale: f64,
    ) {
        self.path(
            pts,
            closed,
            &line_chain(pts, closed),
            true,
            rgb,
            linetype,
            scale,
        );
    }

    fn path(
        &mut self,
        pick_pts: &[Point2],
        closed: bool,
        segs: &[PathSeg],
        plinegen: bool,
        rgb: Rgb,
        linetype: &LineType,
        scale: f64,
    ) {
        if let Some(pick) = self.pick.as_mut() {
            pick.add_stroke(pick_pts, closed);
        }
        if linetype.is_continuous() {
            emit_solid_polyline(self.list, pick_pts, closed, rgb);
            return;
        }
        let pattern = scaled_pattern(&linetype.dashes, scale);
        for (a, b) in generate_path_dashes(segs, &pattern, plinegen, CIRCLE_SEGMENTS) {
            push_line(self.list, a, b, rgb);
        }
    }

    fn fan(&mut self, pts: &[Point2], rgb: Rgb) {
        if let Some(pick) = self.pick.as_mut() {
            pick.add_fill(pts);
        }
        emit_fan(self.list, pts, rgb);
    }

    fn text(
        &mut self,
        transform: Transform2,
        rgb: Rgb,
        insertion: Point3,
        height: f64,
        rotation: f64,
        value: &str,
    ) {
        emit_text(
            self.list,
            self.pick.as_deref_mut(),
            transform,
            rgb,
            insertion,
            height,
            rotation,
            value,
        );
    }
}

pub fn tessellate_document(document: &Document) -> DisplayList {
    let origin = document
        .diagnostics
        .extents
        .or_else(|| document.compute_extents())
        .map(|e| e.center())
        .unwrap_or(Point2::new(0.0, 0.0));
    let mut list = DisplayList {
        origin,
        line_vertices: Vec::with_capacity(64 * 1024),
        triangle_vertices: Vec::new(),
        picks: Vec::with_capacity(document.model_space.len()),
    };
    let mut stack = Vec::new();
    for (entity_index, entity) in document.model_space.iter().enumerate() {
        let mut pick = EntityPick::new(entity_index);
        {
            let mut sink = TessSink {
                list: &mut list,
                pick: Some(&mut pick),
            };
            tessellate_entity(
                document,
                entity,
                Transform2::identity(),
                CadColor::Aci(7),
                "CONTINUOUS",
                &mut stack,
                &mut sink,
            );
        }
        if !pick.is_empty() {
            list.picks.push(pick);
        }
    }
    list
}

fn tessellate_entity(
    document: &Document,
    entity: &Entity,
    transform: Transform2,
    block_color: CadColor,
    block_linetype: &str,
    stack: &mut Vec<String>,
    sink: &mut TessSink<'_>,
) {
    if !entity.visible || !document.layer_is_visible(&entity.layer) {
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
                        tessellate_entity(
                            document,
                            child,
                            nested,
                            inherit,
                            inherit_lt.as_str(),
                            stack,
                            sink,
                        );
                    }
                }
            }
            for attrib in attribs {
                sink.text(
                    transform,
                    rgb,
                    attrib.insertion,
                    attrib.height,
                    attrib.rotation,
                    &attrib.value,
                );
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
                    tessellate_entity(
                        document,
                        child,
                        transform,
                        block_color,
                        block_linetype,
                        stack,
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
            sink.polyline(
                &[Point2::new(p.x - s, p.y), Point2::new(p.x + s, p.y)],
                false,
                rgb,
                &LineType::continuous("CONTINUOUS"),
                1.0,
            );
            sink.polyline(
                &[Point2::new(p.x, p.y - s), Point2::new(p.x, p.y + s)],
                false,
                rgb,
                &LineType::continuous("CONTINUOUS"),
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
            sink.polyline(&pts, false, rgb, &linetype, scale);
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
            sink.polyline(&pts, *closed, rgb, &linetype, scale);
        }
        Geometry::Text(text) => {
            if text.is_attrib_def && !stack.is_empty() {
                return;
            }
            sink.text(
                transform,
                rgb,
                text.insertion,
                text.height,
                text.rotation,
                &text.value,
            );
        }
        Geometry::MText(text) => {
            let cleaned = strip_mtext(&text.value);
            let mut y_off = 0.0;
            for line in cleaned.lines() {
                let insertion =
                    Point3::new(text.insertion.x, text.insertion.y - y_off, text.insertion.z);
                sink.text(transform, rgb, insertion, text.height, text.rotation, line);
                y_off += text.height * 1.6;
            }
        }
        Geometry::Hatch(hatch) => {
            for path in &hatch.paths {
                let pts = hatch_path_points(path);
                let world: Vec<Point2> = pts.into_iter().map(|p| transform.apply(p)).collect();
                sink.polyline(&world, true, rgb, &LineType::continuous("CONTINUOUS"), 1.0);
                if hatch.solid_fill && world.len() >= 3 {
                    sink.fan(&world, rgb);
                }
            }
        }
        Geometry::Solid { corners, .. } => {
            let pts: Vec<Point2> = corners.iter().map(|c| transform.apply(c.xy())).collect();
            sink.polyline(&pts, true, rgb, &LineType::continuous("CONTINUOUS"), 1.0);
            sink.fan(&pts, rgb);
        }
        Geometry::Leader { vertices } | Geometry::MLine { vertices, .. } => {
            let pts: Vec<Point2> = vertices.iter().map(|p| transform.apply(p.xy())).collect();
            sink.polyline(&pts, false, rgb, &linetype, scale);
        }
    }
}

fn circle_path_segs(pts: &[Point2]) -> Vec<PathSeg> {
    let unique: Vec<Point2> = if pts.len() >= 2 && pts.first() == pts.last() {
        pts[..pts.len() - 1].to_vec()
    } else {
        pts.to_vec()
    };
    PathSeg::full_circle_from_points(&unique)
        .map(|seg| vec![seg])
        .unwrap_or_else(|| line_chain(pts, true))
}

fn arc_path_segs(pts: &[Point2]) -> Vec<PathSeg> {
    if pts.len() >= 3 {
        if let Some(seg) =
            PathSeg::from_three_arc_points(pts[0], pts[pts.len() / 2], *pts.last().unwrap())
        {
            return vec![seg];
        }
    }
    line_chain(pts, false)
}

fn hatch_path_points(path: &HatchPath) -> Vec<Point2> {
    match path {
        HatchPath::Polyline { vertices, closed } => {
            polyline_points(vertices, *closed, Point3::new(0.0, 0.0, 1.0))
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
                            Point3::new(0.0, 0.0, 1.0),
                            32,
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
                        ..
                    } => {
                        let major = *major_endpoint - *center;
                        let mut e = ellipse_points(
                            *center,
                            major,
                            *axis_ratio,
                            *start_angle,
                            *end_angle,
                            Point3::new(0.0, 0.0, 1.0),
                            32,
                        );
                        if !pts.is_empty() && !e.is_empty() {
                            e.remove(0);
                        }
                        pts.extend(e);
                    }
                    HatchEdge::Spline { control_points } => {
                        pts.extend(bspline_points(3, control_points, &[], &[], 24));
                    }
                }
            }
            pts
        }
    }
}

#[allow(dead_code)]
fn emit_hatch_pattern(
    list: &mut DisplayList,
    transform: Transform2,
    rgb: Rgb,
    def: &cad_core::HatchPatternLine,
    paths: &[HatchPath],
) {
    let mut hull = Vec::new();
    for path in paths {
        hull.extend(
            hatch_path_points(path)
                .into_iter()
                .map(|p| transform.apply(p)),
        );
    }
    if hull.len() < 2 {
        return;
    }
    let mut min = hull[0];
    let mut max = hull[0];
    for p in &hull {
        min.x = min.x.min(p.x);
        min.y = min.y.min(p.y);
        max.x = max.x.max(p.x);
        max.y = max.y.max(p.y);
    }
    let dir = Point2::new(def.angle.cos(), def.angle.sin());
    let offset = transform.apply(def.offset.xy()) - transform.apply(Point2::new(0.0, 0.0));
    let step = offset.distance(Point2::new(0.0, 0.0)).max(1e-3);
    let span = (max.x - min.x).max(max.y - min.y) * 2.0;
    let n = ((span / step).ceil() as i32).clamp(1, 256);
    let base = transform.apply(def.base.xy());
    let perp = Point2::new(-dir.y, dir.x);
    for i in -n..=n {
        let origin = Point2::new(
            base.x + perp.x * step * i as f64,
            base.y + perp.y * step * i as f64,
        );
        let a = Point2::new(origin.x - dir.x * span, origin.y - dir.y * span);
        let b = Point2::new(origin.x + dir.x * span, origin.y + dir.y * span);
        if segment_hits_hull(a, b, &hull) {
            let hatch_lt = LineType {
                name: "HATCH".into(),
                dashes: def.dashes.clone(),
            };
            if hatch_lt.is_continuous() {
                emit_solid_polyline(list, &[a, b], false, rgb);
            } else {
                let pattern = scaled_pattern(&hatch_lt.dashes, 1.0);
                for (p0, p1) in
                    generate_path_dashes(&[PathSeg::Line { a, b }], &pattern, true, CIRCLE_SEGMENTS)
                {
                    push_line(list, p0, p1, rgb);
                }
            }
        }
    }
}

fn segment_hits_hull(a: Point2, b: Point2, hull: &[Point2]) -> bool {
    let mid = a.lerp(b, 0.5);
    point_in_polygon(mid, hull)
}

fn point_in_polygon(p: Point2, poly: &[Point2]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let pi = poly[i];
        let pj = poly[j];
        if ((pi.y > p.y) != (pj.y > p.y))
            && (p.x < (pj.x - pi.x) * (p.y - pi.y) / (pj.y - pi.y + 1e-30) + pi.x)
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[allow(clippy::too_many_arguments)]
fn emit_text(
    list: &mut DisplayList,
    mut pick: Option<&mut EntityPick>,
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
        if let Some(pick) = pick.as_mut() {
            pick.add_stroke(&[a, b], false);
        }
        push_line(list, a, b, rgb);
    }
}

fn emit_solid_polyline(list: &mut DisplayList, pts: &[Point2], closed: bool, rgb: Rgb) {
    if pts.len() < 2 {
        return;
    }
    let n = if closed { pts.len() } else { pts.len() - 1 };
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % pts.len()];
        push_line(list, a, b, rgb);
    }
}

fn emit_fan(list: &mut DisplayList, pts: &[Point2], rgb: Rgb) {
    if pts.len() < 3 {
        return;
    }
    let color = rgb.to_array();
    let origin = list.origin;
    let p0 = to_gpu(pts[0], origin, color);
    for i in 1..pts.len() - 1 {
        list.triangle_vertices.push(p0);
        list.triangle_vertices.push(to_gpu(pts[i], origin, color));
        list.triangle_vertices
            .push(to_gpu(pts[i + 1], origin, color));
    }
}

fn push_line(list: &mut DisplayList, a: Point2, b: Point2, rgb: Rgb) {
    if !a.is_finite() || !b.is_finite() {
        return;
    }
    let color = rgb.to_array();
    list.line_vertices.push(to_gpu(a, list.origin, color));
    list.line_vertices.push(to_gpu(b, list.origin, color));
}

fn to_gpu(p: Point2, origin: Point2, color: [f32; 4]) -> GpuVertex {
    GpuVertex {
        position: [(p.x - origin.x) as f32, (p.y - origin.y) as f32],
        color,
    }
}
