//! Temporary geometry diagnostics for transform bugs.
//! Prints long edges and suspicious INSERT / block-base values.
//! Not used to filter or hide entities.

use std::collections::BTreeMap;

use cad_core::{Document, Entity, Geometry, Point2, Point3, Transform2};
use cad_render::{
    curves::{bspline_points, circle_points, ellipse_points, polyline_points, CIRCLE_SEGMENTS},
    DisplayList,
};

const LONG_EDGE: f64 = 50_000.0;
const EXTREME_COORD: f64 = 1_000_000.0;
const TOP_N: usize = 20;

#[derive(Clone)]
struct ExtremePoint {
    kind: String,
    stack: String,
    local: Point2,
    world: Point2,
    insert_note: String,
}

#[derive(Clone)]
struct LongEdge {
    length: f64,
    kind: String,
    stack: String,
    local_a: Point2,
    local_b: Point2,
    world_a: Point2,
    world_b: Point2,
    local_len: f64,
    insert_note: String,
}

struct InsertNote {
    name: String,
    insertion: Point3,
    scale: Point3,
    rotation: f64,
    extrusion: Point3,
    base: Point3,
}

struct AuditCtx<'a> {
    document: &'a Document,
    stack: Vec<String>,
    insert_stack: Vec<InsertNote>,
    long_edges: Vec<LongEdge>,
    extreme_points: Vec<ExtremePoint>,
    kind_counts: BTreeMap<String, u64>,
    block_counts: BTreeMap<String, u64>,
    extreme_kind: BTreeMap<String, u64>,
    buckets: [u64; 4],
    scale_samples: Vec<String>,
    mid_kind: BTreeMap<String, u64>,
    mid_block: BTreeMap<String, u64>,
    mid_edges: Vec<LongEdge>,
    huge_text: Vec<String>,
    x_kind: BTreeMap<String, u64>,
    x_block: BTreeMap<String, u64>,
    x_edges: Vec<LongEdge>,
}

impl<'a> AuditCtx<'a> {
    fn insert_note_text(&self) -> String {
        self.insert_stack
            .last()
            .map(|ins| {
                format!(
                    "insert={} ins=({:.6},{:.6},{:.6}) scale=({:.6},{:.6},{:.6}) rot={:.6} ext=({:.6},{:.6},{:.6}) base=({:.6},{:.6},{:.6})",
                    ins.name,
                    ins.insertion.x, ins.insertion.y, ins.insertion.z,
                    ins.scale.x, ins.scale.y, ins.scale.z,
                    ins.rotation,
                    ins.extrusion.x, ins.extrusion.y, ins.extrusion.z,
                    ins.base.x, ins.base.y, ins.base.z
                )
            })
            .unwrap_or_default()
    }

    fn stack_text(&self) -> String {
        if self.stack.is_empty() {
            "*MODEL_SPACE".into()
        } else {
            self.stack.join("/")
        }
    }

    fn consider_point(&mut self, kind: &str, local: Point2, transform: Transform2) {
        let world = transform.apply(local);
        if !world.is_finite() {
            return;
        }
        if world.x.abs() < EXTREME_COORD && world.y.abs() < EXTREME_COORD {
            return;
        }
        *self.extreme_kind.entry(kind.to_string()).or_insert(0) += 1;
        self.extreme_points.push(ExtremePoint {
            kind: kind.to_string(),
            stack: self.stack_text(),
            local,
            world,
            insert_note: self.insert_note_text(),
        });
        if self.extreme_points.len() > TOP_N * 8 {
            self.extreme_points.sort_by(|a, b| {
                let da = a.world.x.abs().max(a.world.y.abs());
                let db = b.world.x.abs().max(b.world.y.abs());
                db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
            });
            self.extreme_points.truncate(TOP_N * 4);
        }
    }

    fn consider_edge(
        &mut self,
        kind: &str,
        local_a: Point2,
        local_b: Point2,
        transform: Transform2,
    ) {
        self.consider_point(kind, local_a, transform);
        self.consider_point(kind, local_b, transform);
        let world_a = transform.apply(local_a);
        let world_b = transform.apply(local_b);
        if !world_a.is_finite() || !world_b.is_finite() {
            return;
        }
        let length = world_a.distance(world_b);
        let dx = (world_b.x - world_a.x).abs();
        let dy = (world_b.y - world_a.y).abs();
        if (8000.0..9200.0).contains(&length) && dx > 2000.0 && dy > 2000.0 {
            *self.x_kind.entry(kind.to_string()).or_insert(0) += 1;
            let block_key = self
                .stack
                .last()
                .cloned()
                .unwrap_or_else(|| "*MODEL_SPACE".into());
            *self.x_block.entry(block_key).or_insert(0) += 1;
            if self.x_edges.len() < 12 {
                self.x_edges.push(LongEdge {
                    length,
                    kind: kind.to_string(),
                    stack: self.stack_text(),
                    local_a,
                    local_b,
                    world_a,
                    world_b,
                    local_len: local_a.distance(local_b),
                    insert_note: self.insert_note_text(),
                });
            }
        }
        if length > 10_000.0 {
            self.buckets[0] += 1;
            let block_key = self
                .stack
                .last()
                .cloned()
                .unwrap_or_else(|| "*MODEL_SPACE".into());
            *self.mid_kind.entry(kind.to_string()).or_insert(0) += 1;
            *self.mid_block.entry(block_key).or_insert(0) += 1;
            if length < LONG_EDGE {
                self.mid_edges.push(LongEdge {
                    length,
                    kind: kind.to_string(),
                    stack: self.stack_text(),
                    local_a,
                    local_b,
                    world_a,
                    world_b,
                    local_len: local_a.distance(local_b),
                    insert_note: self.insert_note_text(),
                });
                if self.mid_edges.len() > TOP_N * 8 {
                    self.mid_edges.sort_by(|a, b| {
                        b.length
                            .partial_cmp(&a.length)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                    self.mid_edges.truncate(TOP_N * 4);
                }
            }
        }
        if length > 50_000.0 {
            self.buckets[1] += 1;
        }
        if length > 100_000.0 {
            self.buckets[2] += 1;
        }
        if length > 1_000_000.0 {
            self.buckets[3] += 1;
        }
        if length < LONG_EDGE {
            return;
        }
        *self.kind_counts.entry(kind.to_string()).or_insert(0) += 1;
        let block_key = self
            .stack
            .last()
            .cloned()
            .unwrap_or_else(|| "*MODEL_SPACE".into());
        *self.block_counts.entry(block_key).or_insert(0) += 1;
        self.long_edges.push(LongEdge {
            length,
            kind: kind.to_string(),
            stack: self.stack_text(),
            local_a,
            local_b,
            world_a,
            world_b,
            local_len: local_a.distance(local_b),
            insert_note: self.insert_note_text(),
        });
        if self.long_edges.len() > TOP_N * 8 {
            self.long_edges.sort_by(|a, b| {
                b.length
                    .partial_cmp(&a.length)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            self.long_edges.truncate(TOP_N * 4);
        }
    }

    fn consider_polyline(&mut self, kind: &str, pts: &[Point2], transform: Transform2) {
        if pts.is_empty() {
            return;
        }
        if pts.len() == 1 {
            self.consider_point(kind, pts[0], transform);
            return;
        }
        for pair in pts.windows(2) {
            self.consider_edge(kind, pair[0], pair[1], transform);
        }
    }

    fn walk(&mut self, entity: &Entity, transform: Transform2) {
        if !entity.visible || !self.document.layer_is_visible(&entity.layer) {
            return;
        }
        match &entity.geometry {
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
                if self
                    .stack
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(block_name))
                {
                    return;
                }
                let Some(block) = self.document.blocks.get(block_name) else {
                    return;
                };
                let sx = scale.x.abs();
                let sy = scale.y.abs();
                if !(0.01..=100.0).contains(&sx) || !(0.01..=100.0).contains(&sy) {
                    self.scale_samples.push(format!(
                        "{block_name} scale=({:.9},{:.9},{:.9}) ins=({:.6},{:.6})",
                        scale.x, scale.y, scale.z, insertion.x, insertion.y
                    ));
                }
                self.stack.push(block_name.clone());
                self.insert_stack.push(InsertNote {
                    name: block_name.clone(),
                    insertion: *insertion,
                    scale: *scale,
                    rotation: *rotation,
                    extrusion: *extrusion,
                    base: block.base_pt,
                });
                let cols = (*column_count).max(1);
                let rows = (*row_count).max(1);
                let children = block.entities.clone();
                let base = block.base_pt;
                for col in 0..cols {
                    for row in 0..rows {
                        let extra = Transform2::translate(
                            col as f64 * *column_spacing,
                            row as f64 * *row_spacing,
                        );
                        let nested = transform.then(
                            Transform2::block_insert(
                                *insertion, *scale, *rotation, *extrusion, base,
                            )
                            .then(extra),
                        );
                        for child in &children {
                            self.walk(child, nested);
                        }
                    }
                }
                self.insert_stack.pop();
                self.stack.pop();
            }
            Geometry::Dimension { block_name } => {
                if self
                    .stack
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(block_name))
                {
                    return;
                }
                if let Some(block) = self.document.blocks.get(block_name) {
                    let children = block.entities.clone();
                    self.stack.push(block_name.clone());
                    for child in &children {
                        self.walk(child, transform);
                    }
                    self.stack.pop();
                }
            }
            Geometry::Line { start, end } => {
                self.consider_edge("LINE", start.xy(), end.xy(), transform);
            }
            Geometry::Point { position } => self.consider_point("POINT", position.xy(), transform),
            Geometry::Circle {
                center,
                radius,
                extrusion,
            } => self.consider_polyline(
                "CIRCLE",
                &circle_points(*center, *radius, *extrusion, CIRCLE_SEGMENTS),
                transform,
            ),
            Geometry::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                extrusion,
            } => self.consider_polyline(
                "ARC",
                &cad_render::curves::arc_points(
                    *center,
                    *radius,
                    *start_angle,
                    *end_angle,
                    true,
                    *extrusion,
                    CIRCLE_SEGMENTS,
                ),
                transform,
            ),
            Geometry::Ellipse {
                center,
                major_axis,
                axis_ratio,
                start_param,
                end_param,
                extrusion,
            } => self.consider_polyline(
                "ELLIPSE",
                &ellipse_points(
                    *center,
                    *major_axis,
                    *axis_ratio,
                    *start_param,
                    *end_param,
                    *extrusion,
                    CIRCLE_SEGMENTS,
                ),
                transform,
            ),
            Geometry::LwPolyline {
                vertices,
                closed,
                extrusion,
                ..
            } => self.consider_polyline(
                "LWPOLYLINE",
                &polyline_points(vertices, *closed, *extrusion),
                transform,
            ),
            Geometry::Polyline {
                vertices, closed, ..
            } => self.consider_polyline(
                "POLYLINE",
                &polyline_points(vertices, *closed, Point3::new(0.0, 0.0, 1.0)),
                transform,
            ),
            Geometry::Spline {
                degree,
                control_points,
                fit_points,
                knots,
                weights,
                ..
            } => {
                for p in control_points {
                    self.consider_point("SPLINE_CTRL", p.xy(), transform);
                }
                let sampled = if control_points.len() >= 2 {
                    bspline_points(*degree, control_points, knots, weights, 24)
                } else {
                    fit_points.iter().map(|p| p.xy()).collect()
                };
                self.consider_polyline("SPLINE", &sampled, transform);
            }
            Geometry::Text(text) => {
                let h = text.height * transform.scale_y().abs().max(transform.scale_x().abs());
                if h > 500.0 {
                    self.huge_text.push(format!(
                        "h={h:.3} val={:?} stack={} ins=({:.3},{:.3})",
                        text.value.chars().take(40).collect::<String>(),
                        self.stack_text(),
                        text.insertion.x,
                        text.insertion.y
                    ));
                }
                self.consider_point("TEXT", text.insertion.xy(), transform);
            }
            Geometry::MText(text) => {
                let h = text.height * transform.scale_y().abs().max(transform.scale_x().abs());
                if h > 500.0 {
                    self.huge_text
                        .push(format!("mtext h={h:.3} stack={}", self.stack_text()));
                }
                self.consider_point("MTEXT", text.insertion.xy(), transform);
            }
            Geometry::Solid { corners, .. } => {
                for c in corners {
                    self.consider_point("SOLID", c.xy(), transform);
                }
            }
            Geometry::Hatch(hatch) => {
                for path in &hatch.paths {
                    match path {
                        cad_core::HatchPath::Polyline { vertices, closed } => self
                            .consider_polyline(
                                "HATCH_POLY",
                                &polyline_points(vertices, *closed, hatch.extrusion),
                                transform,
                            ),
                        cad_core::HatchPath::Edges(edges) => {
                            for edge in edges {
                                match edge {
                                    cad_core::HatchEdge::Line { start, end } => {
                                        self.consider_edge(
                                            "HATCH_LINE",
                                            start.xy(),
                                            end.xy(),
                                            transform,
                                        );
                                    }
                                    cad_core::HatchEdge::Ellipse {
                                        center,
                                        major_endpoint,
                                        ..
                                    } => {
                                        self.consider_point(
                                            "HATCH_ELLIPSE",
                                            center.xy(),
                                            transform,
                                        );
                                        self.consider_point(
                                            "HATCH_ELLIPSE",
                                            major_endpoint.xy(),
                                            transform,
                                        );
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
            Geometry::Leader { vertices } | Geometry::MLine { vertices, .. } => self
                .consider_polyline(
                    "LEADER_OR_MLINE",
                    &vertices.iter().map(|p| p.xy()).collect::<Vec<_>>(),
                    transform,
                ),
        }
    }
}

pub fn print_geometry_audit(document: &Document) {
    let mut ctx = AuditCtx {
        document,
        stack: Vec::new(),
        insert_stack: Vec::new(),
        long_edges: Vec::new(),
        extreme_points: Vec::new(),
        kind_counts: BTreeMap::new(),
        block_counts: BTreeMap::new(),
        extreme_kind: BTreeMap::new(),
        buckets: [0; 4],
        scale_samples: Vec::new(),
        mid_kind: BTreeMap::new(),
        mid_block: BTreeMap::new(),
        mid_edges: Vec::new(),
        huge_text: Vec::new(),
        x_kind: BTreeMap::new(),
        x_block: BTreeMap::new(),
        x_edges: Vec::new(),
    };
    for entity in &document.model_space {
        ctx.walk(entity, Transform2::identity());
    }

    ctx.long_edges.sort_by(|a, b| {
        b.length
            .partial_cmp(&a.length)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ctx.long_edges.truncate(TOP_N);

    println!("audit_long_edge_gt_10k {}", ctx.buckets[0]);
    println!("audit_long_edge_gt_50k {}", ctx.buckets[1]);
    println!("audit_long_edge_gt_100k {}", ctx.buckets[2]);
    println!("audit_long_edge_gt_1e6 {}", ctx.buckets[3]);

    let mut nonzero_base = 0u64;
    let mut max_base = 0.0_f64;
    for block in document.blocks.values() {
        let mag = block.base_pt.xy().distance(Point2::new(0.0, 0.0));
        if mag > 1e-6 {
            nonzero_base += 1;
            max_base = max_base.max(mag);
        }
    }
    println!("audit_blocks_nonzero_base {nonzero_base}");
    println!("audit_max_base_distance {max_base:.6}");
    println!(
        "audit_inserts_tiny_or_huge_scale {}",
        ctx.scale_samples.len()
    );
    for sample in ctx.scale_samples.iter().take(15) {
        println!("  {sample}");
    }

    println!("audit_10k_by_kind");
    for (kind, count) in &ctx.mid_kind {
        println!("  {kind} {count}");
    }
    println!("audit_10k_by_block");
    for (name, count) in ctx.mid_block.iter().take(40) {
        println!("  {name} {count}");
    }
    ctx.mid_edges.sort_by(|a, b| {
        b.length
            .partial_cmp(&a.length)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    println!("audit_top_10k_to_50k_edges");
    for (i, edge) in ctx.mid_edges.iter().take(15).enumerate() {
        println!(
            "  #{i} len={:.3} local_len={:.3} kind={} stack={}",
            edge.length, edge.local_len, edge.kind, edge.stack
        );
        println!(
            "     local=({:.6},{:.6})-({:.6},{:.6}) world=({:.6},{:.6})-({:.6},{:.6})",
            edge.local_a.x,
            edge.local_a.y,
            edge.local_b.x,
            edge.local_b.y,
            edge.world_a.x,
            edge.world_a.y,
            edge.world_b.x,
            edge.world_b.y
        );
        if !edge.insert_note.is_empty() {
            println!("     {}", edge.insert_note);
        }
    }

    println!("audit_long_by_kind");
    for (kind, count) in &ctx.kind_counts {
        println!("  {kind} {count}");
    }
    println!("audit_long_by_block");
    for (name, count) in ctx.block_counts.iter().take(30) {
        println!("  {name} {count}");
    }

    println!("audit_extreme_by_kind");
    for (kind, count) in &ctx.extreme_kind {
        println!("  {kind} {count}");
    }
    ctx.extreme_points.sort_by(|a, b| {
        let da = a.world.x.abs().max(a.world.y.abs());
        let db = b.world.x.abs().max(b.world.y.abs());
        db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
    });
    println!("audit_top_extreme_points");
    for (i, p) in ctx.extreme_points.iter().take(TOP_N).enumerate() {
        println!(
            "  #{i} kind={} stack={} local=({:.6},{:.6}) world=({:.6},{:.6})",
            p.kind, p.stack, p.local.x, p.local.y, p.world.x, p.world.y
        );
        if !p.insert_note.is_empty() {
            println!("     {}", p.insert_note);
        }
    }

    println!("audit_nonzero_block_bases");
    let mut bases: Vec<_> = document
        .blocks
        .values()
        .filter(|b| b.base_pt.xy().distance(Point2::new(0.0, 0.0)) > 1e-6)
        .collect();
    bases.sort_by(|a, b| {
        b.base_pt
            .xy()
            .distance(Point2::new(0.0, 0.0))
            .partial_cmp(&a.base_pt.xy().distance(Point2::new(0.0, 0.0)))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for block in bases.iter().take(15) {
        println!(
            "  {} base=({:.6},{:.6},{:.6})",
            block.name, block.base_pt.x, block.base_pt.y, block.base_pt.z
        );
    }

    println!("audit_top_long_edges");
    for (i, edge) in ctx.long_edges.iter().enumerate() {
        println!(
            "  #{i} len={:.3} local_len={:.3} kind={} stack={}",
            edge.length, edge.local_len, edge.kind, edge.stack
        );
        println!(
            "     local=({:.6},{:.6})-({:.6},{:.6}) world=({:.6},{:.6})-({:.6},{:.6})",
            edge.local_a.x,
            edge.local_a.y,
            edge.local_b.x,
            edge.local_b.y,
            edge.world_a.x,
            edge.world_a.y,
            edge.world_b.x,
            edge.world_b.y
        );
        if !edge.insert_note.is_empty() {
            println!("     {}", edge.insert_note);
        }
    }

    println!("audit_x_fan_kind");
    for (kind, count) in &ctx.x_kind {
        println!("  {kind} {count}");
    }
    println!("audit_x_fan_block");
    for (name, count) in &ctx.x_block {
        println!("  {name} {count}");
    }
    println!("audit_x_fan_samples");
    for (i, edge) in ctx.x_edges.iter().enumerate() {
        println!(
            "  #{i} len={:.3} local_len={:.3} kind={} stack={}",
            edge.length, edge.local_len, edge.kind, edge.stack
        );
        println!(
            "     local=({:.6},{:.6})-({:.6},{:.6}) world=({:.6},{:.6})-({:.6},{:.6})",
            edge.local_a.x,
            edge.local_a.y,
            edge.local_b.x,
            edge.local_b.y,
            edge.world_a.x,
            edge.world_a.y,
            edge.world_b.x,
            edge.world_b.y
        );
        if !edge.insert_note.is_empty() {
            println!("     {}", edge.insert_note);
        }
    }

    ctx.huge_text.sort_by(|a, b| b.cmp(a));
    println!("audit_huge_text {}", ctx.huge_text.len());
    for sample in ctx.huge_text.iter().take(20) {
        println!("  {sample}");
    }

    dump_block_splines(document, "Kefe");
    dump_block_splines(document, "A$C497de78b");
}

fn dump_block_splines(document: &Document, name: &str) {
    let Some(block) = document.blocks.get(name).or_else(|| {
        document
            .blocks
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, b)| b)
    }) else {
        println!("audit_block {name} missing");
        return;
    };
    println!(
        "audit_block {name} base=({:.4},{:.4},{:.4}) entities={}",
        block.base_pt.x,
        block.base_pt.y,
        block.base_pt.z,
        block.entities.len()
    );
    let mut n_spline = 0u64;
    for entity in &block.entities {
        match &entity.geometry {
            Geometry::Spline {
                degree,
                control_points,
                fit_points,
                knots,
                weights,
                ..
            } => {
                n_spline += 1;
                if n_spline > 3 {
                    continue;
                }
                let mut min = Point2::new(f64::INFINITY, f64::INFINITY);
                let mut max = Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
                for p in control_points {
                    min.x = min.x.min(p.x);
                    min.y = min.y.min(p.y);
                    max.x = max.x.max(p.x);
                    max.y = max.y.max(p.y);
                }
                let sampled = bspline_points(*degree, control_points, knots, weights, 24);
                let mut smin = Point2::new(f64::INFINITY, f64::INFINITY);
                let mut smax = Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
                for p in &sampled {
                    smin.x = smin.x.min(p.x);
                    smin.y = smin.y.min(p.y);
                    smax.x = smax.x.max(p.x);
                    smax.y = smax.y.max(p.y);
                }
                println!(
                    "  spline#{n_spline} deg={degree} ctrl={} fit={} knots={} w={} ctrl_bbox=({:.4},{:.4})-({:.4},{:.4}) sample_bbox=({:.4},{:.4})-({:.4},{:.4})",
                    control_points.len(),
                    fit_points.len(),
                    knots.len(),
                    weights.len(),
                    min.x, min.y, max.x, max.y,
                    smin.x, smin.y, smax.x, smax.y
                );
                for (i, p) in control_points.iter().take(6).enumerate() {
                    println!("    ctrl[{i}]=({:.6},{:.6},{:.6})", p.x, p.y, p.z);
                }
            }
            Geometry::Insert {
                block_name,
                insertion,
                scale,
                rotation,
                ..
            } => {
                println!(
                    "  insert {block_name} ins=({:.4},{:.4}) scale=({:.4},{:.4},{:.4}) rot={:.4}",
                    insertion.x, insertion.y, scale.x, scale.y, scale.z, rotation
                );
            }
            _ => {}
        }
    }
    println!("  spline_count {n_spline}");
}

pub fn print_display_list_audit(display: &DisplayList) {
    let origin = display.origin;
    let to_world = |p: [f32; 2]| Point2::new(origin.x + p[0] as f64, origin.y + p[1] as f64);

    let mut diag_white = 0u64;
    let mut diag_other = 0u64;
    let mut axis = 0u64;
    let mut top_diag: Vec<(f64, Point2, Point2, [f32; 4])> = Vec::new();
    let verts = &display.line_vertices;
    let mut i = 0;
    while i + 1 < verts.len() {
        let a = verts[i];
        let b = verts[i + 1];
        i += 2;
        let wa = to_world(a.position);
        let wb = to_world(b.position);
        if !wa.is_finite() || !wb.is_finite() {
            continue;
        }
        let len = wa.distance(wb);
        if len < 2_000.0 {
            continue;
        }
        let dx = (wb.x - wa.x).abs();
        let dy = (wb.y - wa.y).abs();
        let aligned = dx < len * 0.08 || dy < len * 0.08;
        if aligned {
            axis += 1;
            continue;
        }
        let white = a.color[0] > 0.8 && a.color[1] > 0.8 && a.color[2] > 0.8;
        if white {
            diag_white += 1;
        } else {
            diag_other += 1;
        }
        if len > 3_000.0 {
            top_diag.push((len, wa, wb, a.color));
        }
    }
    top_diag.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    println!("audit_gpu_long_axis_aligned {axis}");
    println!("audit_gpu_long_diagonal_white {diag_white}");
    println!("audit_gpu_long_diagonal_other {diag_other}");
    println!("audit_gpu_top_diagonal");
    for (i, (len, a, b, c)) in top_diag.iter().take(15).enumerate() {
        println!(
            "  #{i} len={len:.1} ({:.1},{:.1})-({:.1},{:.1}) rgba={:.2},{:.2},{:.2}",
            a.x, a.y, b.x, b.y, c[0], c[1], c[2]
        );
    }

    let mut big_tri = 0u64;
    let mut top_tri: Vec<(f64, Point2, Point2, Point2, [f32; 4])> = Vec::new();
    let tris = &display.triangle_vertices;
    let mut t = 0;
    while t + 2 < tris.len() {
        let a = to_world(tris[t].position);
        let b = to_world(tris[t + 1].position);
        let c = to_world(tris[t + 2].position);
        t += 3;
        let area = ((a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y)).abs()) * 0.5;
        if area > 5_000_000.0 {
            big_tri += 1;
            top_tri.push((area, a, b, c, tris[t.saturating_sub(3)].color));
        }
    }
    top_tri.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(std::cmp::Ordering::Equal));
    println!("audit_gpu_big_triangles {big_tri}");
    for (i, (area, a, b, c, col)) in top_tri.iter().take(10).enumerate() {
        println!(
            "  #{i} area={area:.0} a=({:.1},{:.1}) b=({:.1},{:.1}) c=({:.1},{:.1}) rgb={:.2},{:.2},{:.2}",
            a.x, a.y, b.x, b.y, c.x, c.y, col[0], col[1], col[2]
        );
    }
}

pub fn print_linetype_audit(document: &Document) {
    println!("ltscale: {}", document.ltscale);
    println!("linetype_defs: {}", document.linetypes.len());
    for (name, lt) in &document.linetypes {
        let dashes = lt
            .dashes
            .iter()
            .map(|d| format!("{d}"))
            .collect::<Vec<_>>()
            .join(",");
        println!("  ltype {name} pattern=[{dashes}]");
    }

    let mut usage: BTreeMap<String, u64> = BTreeMap::new();
    let mut samples: Vec<String> = Vec::new();
    let mut stack = Vec::new();
    for entity in &document.model_space {
        collect_linetype_usage(
            document,
            entity,
            "CONTINUOUS",
            &mut stack,
            &mut usage,
            &mut samples,
        );
    }
    println!("linetype_usage");
    for (name, count) in &usage {
        println!("  used {name} {count}");
    }
    println!("linetype_samples {}", samples.len());
    for sample in samples.iter().take(8) {
        println!("  {sample}");
    }
}

fn collect_linetype_usage(
    document: &Document,
    entity: &Entity,
    block_linetype: &str,
    stack: &mut Vec<String>,
    usage: &mut BTreeMap<String, u64>,
    samples: &mut Vec<String>,
) {
    let resolved = document.resolved_linetype_name(entity, block_linetype);
    *usage.entry(resolved.clone()).or_insert(0) += 1;
    if samples.len() < 8 {
        match &entity.geometry {
            Geometry::Line { start, end } => {
                let len = start.xy().distance(end.xy());
                if len > 500.0
                    && document
                        .linetype(&resolved)
                        .map(|lt| !lt.is_continuous())
                        .unwrap_or(false)
                {
                    samples.push(format!(
                        "LINE raw={} resolved={} layer={} ltscale={:.4} effective={:.4} len={:.1}",
                        entity.linetype,
                        resolved,
                        entity.layer,
                        entity.linetype_scale,
                        document.effective_linetype_scale(entity),
                        len
                    ));
                }
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
                if document
                    .linetype(&resolved)
                    .map(|lt| !lt.is_continuous())
                    .unwrap_or(false)
                    && vertices.len() >= 2
                {
                    samples.push(format!(
                        "POLYLINE raw={} resolved={} layer={} ltscale={:.4} effective={:.4} plinegen={} verts={} closed={}",
                        entity.linetype,
                        resolved,
                        entity.layer,
                        entity.linetype_scale,
                        document.effective_linetype_scale(entity),
                        linetype_generation_continuous,
                        vertices.len(),
                        closed
                    ));
                }
            }
            _ => {}
        }
    }
    if let Geometry::Insert { block_name, .. } = &entity.geometry {
        if stack.iter().any(|n| n.eq_ignore_ascii_case(block_name)) {
            return;
        }
        let Some(block) = document.blocks.get(block_name) else {
            return;
        };
        let inherit = document.resolved_linetype_name(entity, block_linetype);
        stack.push(block_name.clone());
        for child in &block.entities {
            collect_linetype_usage(document, child, &inherit, stack, usage, samples);
        }
        stack.pop();
    }
}
