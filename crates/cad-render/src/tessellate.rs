//! Walk a cad-core document into GPU-ready line and triangle batches.
//! Geometry is sampled in f64, then stored relative to a document origin as f32.

use std::collections::HashMap;

use cad_core::dash::{generate_path_dashes, scaled_pattern, PathSeg};
use cad_core::{
    hatch_path_points, vectorize_entity, CadColor, Document, Entity, EntityId, Extents2, HatchPath,
    LineType, Point2, Rgb, Transform2, VectorSink, VectorVisibility, CIRCLE_SEGMENTS,
};

use crate::pick::{box_select_into, EntityPick, SelectBoxMode, SpatialIndex};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

// ------------------------------------------------------------
// Type: EntityDrawRange
// Purpose: GPU vertex span belonging to one top-level entity.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, Default)]
pub struct EntityDrawRange {
    pub line_start: u32,
    pub line_end: u32,
    pub fill_start: u32,
    pub fill_end: u32,
}

// ------------------------------------------------------------
// Type: AppendedGeometry
// Purpose: Vertex counts before an incremental entity was appended,
//          so the GPU can upload only the new tail.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppendedGeometry {
    pub line_start: u32,
    pub fill_start: u32,
}

// ------------------------------------------------------------
// Type: OverlayBatches
// Purpose: Merged GPU draw ranges for selection or live preview.
// ------------------------------------------------------------
#[derive(Debug, Clone, Default)]
pub struct OverlayBatches {
    pub lines: Vec<std::ops::Range<u32>>,
    pub fills: Vec<std::ops::Range<u32>>,
}

impl OverlayBatches {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.fills.is_empty()
    }

    pub fn range_count(&self) -> usize {
        self.lines.len() + self.fills.len()
    }
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
    pub draw_ranges: Vec<EntityDrawRange>,
    pick_of: HashMap<EntityId, u32>,
    spatial: SpatialIndex,
}

impl DisplayList {
    pub fn is_empty(&self) -> bool {
        self.line_vertices.is_empty() && self.triangle_vertices.is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.line_vertices.len() / 2
    }

    pub fn pick_for(&self, entity_id: EntityId) -> Option<&EntityPick> {
        let slot = *self.pick_of.get(&entity_id)?;
        self.picks.get(slot as usize)
    }

    pub fn draw_range_for(&self, entity_id: EntityId) -> Option<EntityDrawRange> {
        let slot = *self.pick_of.get(&entity_id)?;
        self.draw_ranges.get(slot as usize).copied()
    }

    pub fn spatial(&self) -> &SpatialIndex {
        &self.spatial
    }

    pub fn box_select_into(&self, region: Extents2, mode: SelectBoxMode, out: &mut Vec<EntityId>) {
        box_select_into(&self.picks, Some(&self.spatial), region, mode, out);
    }

    pub fn overlay_batches(&self, ids: &[EntityId]) -> OverlayBatches {
        overlay_batches(self, ids)
    }

    pub fn append_entity(
        &mut self,
        document: &Document,
        entity: &Entity,
    ) -> Option<AppendedGeometry> {
        let entity_index = document
            .model_space
            .iter()
            .position(|existing| existing.id == entity.id)
            .unwrap_or(document.model_space.len().saturating_sub(1));
        let mut stack = Vec::new();
        let appended = emit_top_level_entity(document, entity, entity_index, self, &mut stack)?;
        let slot = (self.picks.len() - 1) as u32;
        let bounds = self.picks[slot as usize].bounds;
        if self.spatial.is_empty() {
            self.spatial = SpatialIndex::build(
                self.picks
                    .iter()
                    .enumerate()
                    .map(|(index, pick)| (index as u32, pick.bounds)),
            );
        } else {
            self.spatial.insert(slot, bounds);
        }
        Some(appended)
    }

    pub fn replace_entity(&mut self, document: &Document, entity: &Entity) -> bool {
        if !self.pick_of.contains_key(&entity.id) {
            return self.append_entity(document, entity).is_some();
        }
        let mut scratch = DisplayList {
            origin: self.origin,
            ..DisplayList::default()
        };
        let mut stack = Vec::new();
        if emit_top_level_entity(document, entity, 0, &mut scratch, &mut stack).is_none() {
            return self.remove_entity(entity.id);
        }
        let Some(&slot) = self.pick_of.get(&entity.id) else {
            return false;
        };
        let old_range = self.draw_ranges[slot as usize];
        let old_bounds = self.picks[slot as usize].bounds;
        let new_pick = scratch.picks.pop().expect("tessellated pick");
        let new_range = scratch.draw_ranges.pop().expect("tessellated range");
        let new_line_count = new_range.line_end - new_range.line_start;
        let new_fill_count = new_range.fill_end - new_range.fill_start;
        let old_line_count = old_range.line_end - old_range.line_start;
        let old_fill_count = old_range.fill_end - old_range.fill_start;
        self.spatial.remove(slot, old_bounds);
        if new_line_count == old_line_count && new_fill_count == old_fill_count {
            let line_start = old_range.line_start as usize;
            let fill_start = old_range.fill_start as usize;
            self.line_vertices[line_start..line_start + new_line_count as usize]
                .copy_from_slice(&scratch.line_vertices);
            self.triangle_vertices[fill_start..fill_start + new_fill_count as usize]
                .copy_from_slice(&scratch.triangle_vertices);
            self.picks[slot as usize] = new_pick;
            self.spatial.insert(slot, self.picks[slot as usize].bounds);
            return true;
        }
        collapse_vertices(
            &mut self.line_vertices,
            old_range.line_start,
            old_range.line_end,
        );
        collapse_vertices(
            &mut self.triangle_vertices,
            old_range.fill_start,
            old_range.fill_end,
        );
        let line_start = self.line_vertices.len() as u32;
        let fill_start = self.triangle_vertices.len() as u32;
        self.line_vertices.extend_from_slice(&scratch.line_vertices);
        self.triangle_vertices
            .extend_from_slice(&scratch.triangle_vertices);
        self.draw_ranges[slot as usize] = EntityDrawRange {
            line_start,
            line_end: self.line_vertices.len() as u32,
            fill_start,
            fill_end: self.triangle_vertices.len() as u32,
        };
        self.picks[slot as usize] = new_pick;
        self.spatial.insert(slot, self.picks[slot as usize].bounds);
        true
    }

    pub fn remove_entity(&mut self, entity_id: EntityId) -> bool {
        let Some(&slot) = self.pick_of.get(&entity_id) else {
            return false;
        };
        let range = self.draw_ranges[slot as usize];
        let old_bounds = self.picks[slot as usize].bounds;
        collapse_vertices(&mut self.line_vertices, range.line_start, range.line_end);
        collapse_vertices(
            &mut self.triangle_vertices,
            range.fill_start,
            range.fill_end,
        );
        self.spatial.remove(slot, old_bounds);
        self.picks[slot as usize] = EntityPick::new(entity_id);
        self.draw_ranges[slot as usize] = EntityDrawRange::default();
        self.pick_of.remove(&entity_id);
        true
    }
}

fn collapse_vertices(verts: &mut [GpuVertex], start: u32, end: u32) {
    let start = start as usize;
    let end = (end as usize).min(verts.len());
    if start >= end {
        return;
    }
    let collapsed = GpuVertex {
        position: verts[start].position,
        color: [0.0; 4],
    };
    for vertex in &mut verts[start..end] {
        *vertex = collapsed;
    }
}

pub fn overlay_batches(display: &DisplayList, ids: &[EntityId]) -> OverlayBatches {
    let _span = cad_core::perf::span("overlay_batches");
    let mut lines = Vec::with_capacity(ids.len());
    let mut fills = Vec::with_capacity(ids.len());
    for &entity_id in ids {
        let Some(range) = display.draw_range_for(entity_id) else {
            continue;
        };
        if range.line_end > range.line_start {
            lines.push(range.line_start..range.line_end);
        }
        if range.fill_end > range.fill_start {
            fills.push(range.fill_start..range.fill_end);
        }
    }
    merge_vertex_ranges(&mut lines);
    merge_vertex_ranges(&mut fills);
    OverlayBatches { lines, fills }
}

pub fn merge_vertex_ranges(ranges: &mut Vec<std::ops::Range<u32>>) {
    if ranges.len() <= 1 {
        return;
    }
    ranges.sort_unstable_by_key(|range| range.start);
    let mut write = 0usize;
    for read in 1..ranges.len() {
        if ranges[read].start <= ranges[write].end {
            ranges[write].end = ranges[write].end.max(ranges[read].end);
        } else {
            write += 1;
            ranges[write] = ranges[read].clone();
        }
    }
    ranges.truncate(write + 1);
}

struct TessSink<'a> {
    list: &'a mut DisplayList,
    pick: Option<&'a mut EntityPick>,
    dim: bool,
}

impl VectorSink for TessSink<'_> {
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
        let rgb = if self.dim {
            rgb.dim_for_block_context()
        } else {
            rgb
        };
        if linetype.is_continuous() {
            emit_solid_polyline(self.list, pick_pts, closed, rgb);
            return;
        }
        let pattern = scaled_pattern(&linetype.dashes, scale);
        for (a, b) in generate_path_dashes(segs, &pattern, plinegen, CIRCLE_SEGMENTS) {
            push_line(self.list, a, b, rgb);
        }
    }

    fn fill(&mut self, pts: &[Point2], rgb: Rgb) {
        if let Some(pick) = self.pick.as_mut() {
            pick.add_fill(pts);
        }
        let rgb = if self.dim {
            rgb.dim_for_block_context()
        } else {
            rgb
        };
        emit_fan(self.list, pts, rgb);
    }
}

pub fn tessellate_document(document: &Document) -> DisplayList {
    let _span = cad_core::perf::span("tessellate_document");
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
        draw_ranges: Vec::with_capacity(document.model_space.len()),
        pick_of: HashMap::with_capacity(document.model_space.len()),
        spatial: SpatialIndex::empty(),
    };
    let mut stack = Vec::new();
    for (entity_index, entity) in document.model_space.iter().enumerate() {
        emit_top_level_entity(document, entity, entity_index, &mut list, &mut stack);
    }
    list.spatial = SpatialIndex::build(
        list.picks
            .iter()
            .enumerate()
            .map(|(slot, pick)| (slot as u32, pick.bounds)),
    );
    list
}

// ------------------------------------------------------------
// Type: BlockEditView
// Purpose: Nested INSERT path currently being edited in place.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct BlockEditViewFrame {
    pub instance_id: EntityId,
    pub block_name: String,
}

#[derive(Debug, Clone, Default)]
pub struct BlockEditView {
    pub frames: Vec<BlockEditViewFrame>,
}

pub fn tessellate_document_for_block_edit(
    document: &Document,
    view: &BlockEditView,
) -> DisplayList {
    let _span = cad_core::perf::span("tessellate_document_for_block_edit");
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
        picks: Vec::with_capacity(document.model_space.len() + 16),
        draw_ranges: Vec::with_capacity(document.model_space.len() + 16),
        pick_of: HashMap::with_capacity(document.model_space.len() + 16),
        spatial: SpatialIndex::empty(),
    };
    let mut stack = Vec::new();
    emit_block_edit_space(
        document,
        &document.model_space,
        Transform2::identity(),
        0,
        view,
        &mut list,
        &mut stack,
    );
    list.spatial = SpatialIndex::build(
        list.picks
            .iter()
            .enumerate()
            .map(|(slot, pick)| (slot as u32, pick.bounds)),
    );
    list
}

fn emit_block_edit_space(
    document: &Document,
    entities: &[cad_core::Entity],
    transform: Transform2,
    depth: usize,
    view: &BlockEditView,
    list: &mut DisplayList,
    stack: &mut Vec<String>,
) {
    let next = view.frames.get(depth);
    for entity in entities {
        if next.is_some_and(|frame| entity.id == frame.instance_id) {
            if let cad_core::Geometry::Insert {
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
            } = &entity.geometry
            {
                if stack.iter().any(|n| n.eq_ignore_ascii_case(block_name)) {
                    continue;
                }
                let Some(block) = document.block_by_name(block_name) else {
                    continue;
                };
                stack.push(block_name.clone());
                let local = Transform2::block_insert(
                    *insertion,
                    *scale,
                    *rotation,
                    *extrusion,
                    block.base_pt,
                );
                let nested = transform.then(local);
                let cols = (*column_count).max(1);
                let rows = (*row_count).max(1);
                for col in 0..cols {
                    for row in 0..rows {
                        let extra = Transform2::translate(
                            col as f64 * *column_spacing,
                            row as f64 * *row_spacing,
                        );
                        let instance = nested.then(extra);
                        if depth + 1 == view.frames.len() {
                            for child in &block.entities {
                                emit_pickable_entity(
                                    document, child, instance, child.id, false, list, stack,
                                );
                            }
                        } else {
                            emit_block_edit_space(
                                document,
                                &block.entities,
                                instance,
                                depth + 1,
                                view,
                                list,
                                stack,
                            );
                        }
                    }
                }
                stack.pop();
                continue;
            }
        }
        let dim = true;
        emit_pickable_entity(document, entity, transform, entity.id, dim, list, stack);
    }
}

fn emit_pickable_entity(
    document: &Document,
    entity: &cad_core::Entity,
    transform: Transform2,
    pick_id: EntityId,
    dim: bool,
    list: &mut DisplayList,
    stack: &mut Vec<String>,
) {
    let pick_id = if pick_id.is_assigned() {
        pick_id
    } else {
        EntityId(list.picks.len() as u64 + 1)
    };
    let line_start = list.line_vertices.len() as u32;
    let fill_start = list.triangle_vertices.len() as u32;
    let mut pick = EntityPick::new(pick_id);
    {
        let mut sink = TessSink {
            list,
            pick: Some(&mut pick),
            dim,
        };
        vectorize_entity(
            document,
            entity,
            transform,
            CadColor::Aci(7),
            "CONTINUOUS",
            stack,
            VectorVisibility::Viewport,
            &mut sink,
        );
    }
    if pick.is_empty() {
        return;
    }
    pick.finalize();
    list.pick_of.insert(pick_id, list.picks.len() as u32);
    list.picks.push(pick);
    list.draw_ranges.push(EntityDrawRange {
        line_start,
        line_end: list.line_vertices.len() as u32,
        fill_start,
        fill_end: list.triangle_vertices.len() as u32,
    });
}

fn emit_top_level_entity(
    document: &Document,
    entity: &Entity,
    entity_index: usize,
    list: &mut DisplayList,
    stack: &mut Vec<String>,
) -> Option<AppendedGeometry> {
    let entity_id = if entity.id.is_assigned() {
        entity.id
    } else {
        EntityId(entity_index as u64)
    };
    let line_start = list.line_vertices.len() as u32;
    let fill_start = list.triangle_vertices.len() as u32;
    let mut pick = EntityPick::new(entity_id);
    {
        let mut sink = TessSink {
            list,
            pick: Some(&mut pick),
            dim: false,
        };
        vectorize_entity(
            document,
            entity,
            Transform2::identity(),
            CadColor::Aci(7),
            "CONTINUOUS",
            stack,
            VectorVisibility::Viewport,
            &mut sink,
        );
    }
    if pick.is_empty() {
        return None;
    }
    pick.finalize();
    list.pick_of.insert(entity_id, list.picks.len() as u32);
    list.picks.push(pick);
    list.draw_ranges.push(EntityDrawRange {
        line_start,
        line_end: list.line_vertices.len() as u32,
        fill_start,
        fill_end: list.triangle_vertices.len() as u32,
    });
    Some(AppendedGeometry {
        line_start,
        fill_start,
    })
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
            hatch_path_points(path, cad_core::default_extrusion(), 0.0)
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
