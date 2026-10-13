//! Walk a cad-core document into GPU-ready line and triangle batches.
//! Geometry is sampled in f64, then stored relative to a document origin as f32.

use std::collections::HashMap;

use cad_core::dash::{generate_path_dashes_with_tolerance, scaled_pattern, PathSeg};
use cad_core::{
    vectorize_entity, CadColor, Document, DrawBand, Entity, EntityId, Extents2, GradientRamp,
    LineType, PaintList, Point2, Rgb, Transform2, VectorSink, VectorVisibility,
};

use crate::pick::{box_select_into, EntityPick, SelectBoxMode, SpatialIndex};
use crate::triangulate::{triangulate_even_odd, triangulate_polygon};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
    /// Smaller values are closer. Zero is in front of imported geometry.
    pub depth: f32,
}

/// One corner of a screen-space lineweight quad.
///
/// `position` is this end of the segment and `other` is the far end, both
/// relative to the display origin. The shader turns `half_px` into a pixel
/// offset, so zooming does not rebuild the buffer.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WeightVertex {
    pub position: [f32; 2],
    pub other: [f32; 2],
    pub color: [f32; 4],
    pub depth: f32,
    pub side: f32,
    pub half_px: f32,
    _pad: f32,
}

const _: () = assert!(std::mem::size_of::<WeightVertex>() == 48);

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
    pub weight_start: u32,
    pub weight_end: u32,
}

/// A half-open span of GPU vertices rewritten by one edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VertexSpan {
    pub start: u32,
    pub end: u32,
}

impl VertexSpan {
    pub const EMPTY: Self = Self { start: 0, end: 0 };

    pub fn new(start: u32, end: u32) -> Self {
        if end <= start {
            Self::EMPTY
        } else {
            Self { start, end }
        }
    }

    pub fn is_empty(self) -> bool {
        self.end <= self.start
    }
}

/// Vertex spans a remove or replace must upload. In-place edits fill
/// `lines`/`fills`. A size change also appends a tail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TouchedVertices {
    pub lines: VertexSpan,
    pub fills: VertexSpan,
    pub weights: VertexSpan,
    pub appended_lines: VertexSpan,
    pub appended_fills: VertexSpan,
    pub appended_weights: VertexSpan,
}

impl TouchedVertices {
    fn rewritten(lines: VertexSpan, fills: VertexSpan, weights: VertexSpan) -> Self {
        Self {
            lines,
            fills,
            weights,
            appended_lines: VertexSpan::EMPTY,
            appended_fills: VertexSpan::EMPTY,
            appended_weights: VertexSpan::EMPTY,
        }
    }

    pub fn push_ranges(
        self,
        lines: &mut Vec<std::ops::Range<u32>>,
        fills: &mut Vec<std::ops::Range<u32>>,
        weights: &mut Vec<std::ops::Range<u32>>,
    ) {
        push_span(lines, self.lines);
        push_span(lines, self.appended_lines);
        push_span(fills, self.fills);
        push_span(fills, self.appended_fills);
        push_span(weights, self.weights);
        push_span(weights, self.appended_weights);
    }
}

fn push_span(out: &mut Vec<std::ops::Range<u32>>, span: VertexSpan) {
    if !span.is_empty() {
        out.push(span.start..span.end);
    }
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
    pub weight_start: u32,
}

// ------------------------------------------------------------
// Type: OverlayBatches
// Purpose: Merged GPU draw ranges for selection or live preview.
// ------------------------------------------------------------
#[derive(Debug, Clone, Default)]
pub struct OverlayBatches {
    pub lines: Vec<std::ops::Range<u32>>,
    pub fills: Vec<std::ops::Range<u32>>,
    pub weights: Vec<std::ops::Range<u32>>,
}

impl OverlayBatches {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.fills.is_empty() && self.weights.is_empty()
    }

    pub fn range_count(&self) -> usize {
        self.lines.len() + self.fills.len() + self.weights.len()
    }
}

// ------------------------------------------------------------
// Type: DisplayList
// Purpose: Cached tessellation for the wgpu renderer. Document
//          coordinates remain f64 in cad-core; this is a display cache.
// ------------------------------------------------------------
#[derive(Clone)]
pub struct DisplayList {
    pub origin: Point2,
    pub line_vertices: Vec<GpuVertex>,
    pub triangle_vertices: Vec<GpuVertex>,
    pub weight_vertices: Vec<WeightVertex>,
    pub picks: Vec<EntityPick>,
    pub draw_ranges: Vec<EntityDrawRange>,
    /// Wipeout fill. Edits of one entity reuse this color.
    pub mask: Rgb,
    pick_of: HashMap<EntityId, u32>,
    spatial: SpatialIndex,
}

impl Default for DisplayList {
    fn default() -> Self {
        Self {
            origin: Point2::default(),
            line_vertices: Vec::new(),
            triangle_vertices: Vec::new(),
            weight_vertices: Vec::new(),
            picks: Vec::new(),
            draw_ranges: Vec::new(),
            mask: viewport_mask_rgb(),
            pick_of: HashMap::new(),
            spatial: SpatialIndex::default(),
        }
    }
}

impl DisplayList {
    pub fn is_empty(&self) -> bool {
        self.line_vertices.is_empty()
            && self.triangle_vertices.is_empty()
            && self.weight_vertices.is_empty()
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

    // --------------------------------------------------------
    // Method: finish_build
    // Purpose: Index the picks and keep edit headroom, so the first
    //          appended entity does not copy the whole list.
    // --------------------------------------------------------
    fn finish_build(&mut self) {
        self.spatial = SpatialIndex::build(
            self.picks
                .iter()
                .enumerate()
                .map(|(slot, pick)| (slot as u32, pick.bounds)),
        );
        cad_core::perf::reserve_vec_headroom(&mut self.line_vertices);
        cad_core::perf::reserve_vec_headroom(&mut self.triangle_vertices);
        cad_core::perf::reserve_vec_headroom(&mut self.weight_vertices);
        cad_core::perf::reserve_vec_headroom(&mut self.picks);
        cad_core::perf::reserve_vec_headroom(&mut self.draw_ranges);
        cad_core::perf::reserve_map_headroom(&mut self.pick_of);
    }

    pub fn append_entity(
        &mut self,
        document: &Document,
        entity: &Entity,
    ) -> Option<AppendedGeometry> {
        self.append_entity_with(document, entity, Transform2::identity(), false)
    }

    // --------------------------------------------------------
    // Method: append_entity_with
    // Purpose: Draw one entity under a world transform. Block-edit
    //          members pass the open instance transform and stay bright.
    // --------------------------------------------------------
    pub fn append_entity_with(
        &mut self,
        document: &Document,
        entity: &Entity,
        transform: Transform2,
        dim: bool,
    ) -> Option<AppendedGeometry> {
        let line_start = self.line_vertices.len() as u32;
        let fill_start = self.triangle_vertices.len() as u32;
        let weight_start = self.weight_vertices.len() as u32;
        let before = self.picks.len();
        let mut stack = Vec::new();
        let (rank, span) = paint_for(document, entity);
        emit_pickable_entity(
            document, entity, transform, entity.id, dim, rank, span, self, &mut stack,
        );
        if self.picks.len() == before {
            return None;
        }
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
        Some(AppendedGeometry {
            line_start,
            fill_start,
            weight_start,
        })
    }

    pub fn replace_entity(
        &mut self,
        document: &Document,
        entity: &Entity,
    ) -> Option<TouchedVertices> {
        self.replace_entity_with(document, entity, Transform2::identity())
    }

    pub fn replace_entity_with(
        &mut self,
        document: &Document,
        entity: &Entity,
        transform: Transform2,
    ) -> Option<TouchedVertices> {
        if !self.pick_of.contains_key(&entity.id) {
            return self
                .append_entity_with(document, entity, transform, false)
                .map(|range| self.touched_tail(range));
        }
        let mut scratch = DisplayList {
            origin: self.origin,
            mask: self.mask,
            ..DisplayList::default()
        };
        let before = scratch.picks.len();
        let mut stack = Vec::new();
        let (rank, span) = paint_for(document, entity);
        emit_pickable_entity(
            document,
            entity,
            transform,
            entity.id,
            false,
            rank,
            span,
            &mut scratch,
            &mut stack,
        );
        if scratch.picks.len() == before {
            return self.remove_entity(entity.id);
        }
        let &slot = self.pick_of.get(&entity.id)?;
        let old_range = self.draw_ranges[slot as usize];
        let old_bounds = self.picks[slot as usize].bounds;
        let new_pick = scratch.picks.pop()?;
        let new_range = scratch.draw_ranges.pop()?;
        let new_line_count = new_range.line_end - new_range.line_start;
        let new_fill_count = new_range.fill_end - new_range.fill_start;
        let new_weight_count = new_range.weight_end - new_range.weight_start;
        let old_line_count = old_range.line_end - old_range.line_start;
        let old_fill_count = old_range.fill_end - old_range.fill_start;
        let old_weight_count = old_range.weight_end - old_range.weight_start;
        self.spatial.remove(slot, old_bounds);
        if new_line_count == old_line_count
            && new_fill_count == old_fill_count
            && new_weight_count == old_weight_count
        {
            let line_start = old_range.line_start as usize;
            let fill_start = old_range.fill_start as usize;
            let weight_start = old_range.weight_start as usize;
            self.line_vertices[line_start..line_start + new_line_count as usize]
                .copy_from_slice(&scratch.line_vertices);
            self.triangle_vertices[fill_start..fill_start + new_fill_count as usize]
                .copy_from_slice(&scratch.triangle_vertices);
            self.weight_vertices[weight_start..weight_start + new_weight_count as usize]
                .copy_from_slice(&scratch.weight_vertices);
            self.picks[slot as usize] = new_pick;
            self.spatial.insert(slot, self.picks[slot as usize].bounds);
            return Some(TouchedVertices::rewritten(
                VertexSpan::new(old_range.line_start, old_range.line_end),
                VertexSpan::new(old_range.fill_start, old_range.fill_end),
                VertexSpan::new(old_range.weight_start, old_range.weight_end),
            ));
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
        collapse_weights(
            &mut self.weight_vertices,
            old_range.weight_start,
            old_range.weight_end,
        );
        let line_start = self.line_vertices.len() as u32;
        let fill_start = self.triangle_vertices.len() as u32;
        let weight_start = self.weight_vertices.len() as u32;
        self.line_vertices.extend_from_slice(&scratch.line_vertices);
        self.triangle_vertices
            .extend_from_slice(&scratch.triangle_vertices);
        self.weight_vertices
            .extend_from_slice(&scratch.weight_vertices);
        self.draw_ranges[slot as usize] = EntityDrawRange {
            line_start,
            line_end: self.line_vertices.len() as u32,
            fill_start,
            fill_end: self.triangle_vertices.len() as u32,
            weight_start,
            weight_end: self.weight_vertices.len() as u32,
        };
        self.picks[slot as usize] = new_pick;
        self.spatial.insert(slot, self.picks[slot as usize].bounds);
        Some(TouchedVertices {
            lines: VertexSpan::new(old_range.line_start, old_range.line_end),
            fills: VertexSpan::new(old_range.fill_start, old_range.fill_end),
            weights: VertexSpan::new(old_range.weight_start, old_range.weight_end),
            appended_lines: VertexSpan::new(line_start, self.line_vertices.len() as u32),
            appended_fills: VertexSpan::new(fill_start, self.triangle_vertices.len() as u32),
            appended_weights: VertexSpan::new(weight_start, self.weight_vertices.len() as u32),
        })
    }

    pub fn remove_entity(&mut self, entity_id: EntityId) -> Option<TouchedVertices> {
        let &slot = self.pick_of.get(&entity_id)?;
        let range = self.draw_ranges[slot as usize];
        let old_bounds = self.picks[slot as usize].bounds;
        collapse_vertices(&mut self.line_vertices, range.line_start, range.line_end);
        collapse_vertices(
            &mut self.triangle_vertices,
            range.fill_start,
            range.fill_end,
        );
        collapse_weights(
            &mut self.weight_vertices,
            range.weight_start,
            range.weight_end,
        );
        self.spatial.remove(slot, old_bounds);
        self.picks[slot as usize] = EntityPick::new(entity_id);
        self.draw_ranges[slot as usize] = EntityDrawRange::default();
        self.pick_of.remove(&entity_id);
        Some(TouchedVertices::rewritten(
            VertexSpan::new(range.line_start, range.line_end),
            VertexSpan::new(range.fill_start, range.fill_end),
            VertexSpan::new(range.weight_start, range.weight_end),
        ))
    }

    fn touched_tail(&self, range: AppendedGeometry) -> TouchedVertices {
        TouchedVertices {
            lines: VertexSpan::EMPTY,
            fills: VertexSpan::EMPTY,
            weights: VertexSpan::EMPTY,
            appended_lines: VertexSpan::new(range.line_start, self.line_vertices.len() as u32),
            appended_fills: VertexSpan::new(range.fill_start, self.triangle_vertices.len() as u32),
            appended_weights: VertexSpan::new(
                range.weight_start,
                self.weight_vertices.len() as u32,
            ),
        }
    }

    // --------------------------------------------------------
    // Method: derive_block_edit
    // Purpose: Build a one-level in-place edit from the model-space
    //          picture. Context vertices are dimmed in place and only
    //          the open block is redrawn, so the first open does not
    //          walk the whole drawing again.
    //          Returns None for nested edits and for an INSERT that is
    //          not a direct model-space entity.
    // --------------------------------------------------------
    pub fn derive_block_edit(
        &self,
        document: &Document,
        view: &BlockEditView,
    ) -> Option<DisplayList> {
        if view.frames.len() != 1 {
            return None;
        }
        let frame = &view.frames[0];
        let entity = document
            .model_space
            .iter()
            .find(|candidate| candidate.id == frame.instance_id)?;
        if !matches!(entity.geometry, cad_core::Geometry::Insert { .. }) {
            return None;
        }
        let mut list = self.clone();
        dim_vertex_colors(&mut list.line_vertices);
        dim_vertex_colors(&mut list.triangle_vertices);
        dim_weight_colors(&mut list.weight_vertices);
        list.remove_entity(entity.id);
        let appended_at = list.picks.len();
        let mut stack = Vec::new();
        emit_edited_insert(
            document,
            entity,
            Transform2::identity(),
            0,
            view,
            &mut list,
            &mut stack,
        );
        list.index_appended_picks(appended_at);
        Some(list)
    }

    fn index_appended_picks(&mut self, start: usize) {
        if start >= self.picks.len() {
            return;
        }
        if self.spatial.is_empty() {
            self.spatial = SpatialIndex::build(
                self.picks
                    .iter()
                    .enumerate()
                    .map(|(slot, pick)| (slot as u32, pick.bounds)),
            );
            return;
        }
        for slot in start..self.picks.len() {
            self.spatial.insert(slot as u32, self.picks[slot].bounds);
        }
    }
}

fn collapse_weights(verts: &mut [WeightVertex], start: u32, end: u32) {
    let start = start as usize;
    let end = (end as usize).min(verts.len());
    if start >= end {
        return;
    }
    for vertex in &mut verts[start..end] {
        vertex.color = [0.0; 4];
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
        depth: verts[start].depth,
    };
    for vertex in &mut verts[start..end] {
        *vertex = collapsed;
    }
}

pub fn overlay_batches(display: &DisplayList, ids: &[EntityId]) -> OverlayBatches {
    let _span = cad_core::perf::span("overlay_batches");
    let mut lines = Vec::with_capacity(ids.len());
    let mut fills = Vec::with_capacity(ids.len());
    let mut weights = Vec::with_capacity(ids.len());
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
        if range.weight_end > range.weight_start {
            weights.push(range.weight_start..range.weight_end);
        }
    }
    merge_vertex_ranges(&mut lines);
    merge_vertex_ranges(&mut fills);
    merge_vertex_ranges(&mut weights);
    OverlayBatches {
        lines,
        fills,
        weights,
    }
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
    show_weights: bool,
    stroke_mm: f64,
    chord_tolerance: f64,
    band: DrawBand,
}

impl TessSink<'_> {
    /// On a light sheet, near-white ink becomes black so color 7 stays visible.
    /// Stored dark colors stay as they are. Block-edit dimming follows that.
    fn display_rgb(&self, rgb: Rgb) -> Rgb {
        let rgb = ink_on_background(rgb, self.list.mask);
        if self.dim {
            rgb.dim_for_block_context()
        } else {
            rgb
        }
    }

    /// Pixel half-width when lineweight display is on and the stroke is
    /// wider than one pixel. `None` keeps the hairline line list.
    fn weight_half_px(&self) -> Option<f32> {
        if !self.show_weights {
            return None;
        }
        let hundredths = (self.stroke_mm * 100.0).round() as i16;
        let pixels = cad_core::lineweight_pixels(hundredths);
        if pixels <= 1 {
            None
        } else {
            Some(pixels as f32 * 0.5)
        }
    }
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
        let rgb = self.display_rgb(rgb);
        let half_px = self.weight_half_px();
        if linetype.is_continuous() {
            emit_solid_polyline(self.list, pick_pts, closed, rgb, self.band.depth, half_px);
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
            push_stroke(self.list, a, b, rgb, self.band.depth, half_px);
        }
    }

    fn stroke_mm(&mut self, mm: f64) {
        self.stroke_mm = if mm.is_finite() && mm > 0.0 { mm } else { 0.0 };
    }

    fn fill(&mut self, pts: &[Point2], rgb: Rgb) {
        if let Some(pick) = self.pick.as_mut() {
            pick.add_fill(pts);
        }
        let rgb = self.display_rgb(rgb);
        emit_triangles(self.list, &triangulate_polygon(pts), rgb, self.band.depth);
    }

    fn fill_mask(&mut self, pts: &[Point2]) {
        if let Some(pick) = self.pick.as_mut() {
            pick.add_fill(pts);
        }
        let rgb = self.list.mask;
        emit_triangles(self.list, &triangulate_polygon(pts), rgb, self.band.depth);
    }

    fn begin_entity_draw(&mut self, rank: u32, span: u32) {
        self.band.begin(rank, span);
    }

    fn end_entity_draw(&mut self) {
        self.band.end();
    }

    fn fill_even_odd(&mut self, contours: &[Vec<Point2>], rgb: Rgb) {
        let rgb = self.display_rgb(rgb);
        for contour in contours {
            if let Some(pick) = self.pick.as_mut() {
                pick.add_fill(contour);
            }
        }
        emit_triangles(
            self.list,
            &triangulate_even_odd(contours),
            rgb,
            self.band.depth,
        );
    }

    fn fill_gradient(&mut self, contours: &[Vec<Point2>], ramp: &GradientRamp) {
        for contour in contours {
            if let Some(pick) = self.pick.as_mut() {
                pick.add_fill(contour);
            }
        }
        let (min, max) = contour_bounds(contours);
        let origin = self.list.origin;
        for tri in triangulate_even_odd(contours) {
            for point in tri {
                let rgb = self.display_rgb(ramp.color_at(ramp.parameter(point, min, max)));
                self.list.triangle_vertices.push(to_gpu(
                    point,
                    origin,
                    rgb.to_array(),
                    self.band.depth,
                ));
            }
        }
    }

    fn pick_outline(&mut self, pts: &[Point2], closed: bool) {
        if let Some(pick) = self.pick.as_mut() {
            pick.add_stroke(pts, closed);
        }
    }
}

fn contour_bounds(contours: &[Vec<Point2>]) -> (Point2, Point2) {
    let mut min = Point2::new(f64::INFINITY, f64::INFINITY);
    let mut max = Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
    for contour in contours {
        for point in contour {
            if !point.is_finite() {
                continue;
            }
            min.x = min.x.min(point.x);
            min.y = min.y.min(point.y);
            max.x = max.x.max(point.x);
            max.y = max.y.max(point.y);
        }
    }
    if !min.x.is_finite() {
        (Point2::new(0.0, 0.0), Point2::new(0.0, 0.0))
    } else {
        (min, max)
    }
}

pub fn tessellate_document(document: &Document) -> DisplayList {
    tessellate_document_on(document, viewport_mask_rgb())
}

pub fn tessellate_document_on(document: &Document, mask: Rgb) -> DisplayList {
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
        weight_vertices: Vec::new(),
        picks: Vec::with_capacity(document.model_space.len()),
        draw_ranges: Vec::with_capacity(document.model_space.len()),
        mask,
        pick_of: HashMap::with_capacity(document.model_space.len()),
        spatial: SpatialIndex::empty(),
    };
    let paint = PaintList::from_entities(&document.model_space);
    let mut stack = Vec::new();
    for (entity_index, entity) in document.model_space.iter().enumerate() {
        emit_top_level_entity(
            document,
            entity,
            entity_index,
            paint.rank(&document.model_space, entity_index),
            paint.span,
            &mut list,
            &mut stack,
        );
    }
    list.finish_build();
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
    tessellate_document_for_block_edit_on(document, view, viewport_mask_rgb())
}

pub fn tessellate_document_for_block_edit_on(
    document: &Document,
    view: &BlockEditView,
    mask: Rgb,
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
        weight_vertices: Vec::new(),
        picks: Vec::with_capacity(document.model_space.len() + 16),
        draw_ranges: Vec::with_capacity(document.model_space.len() + 16),
        mask,
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
    list.finish_build();
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
    let paint = PaintList::from_entities(entities);
    let next = view.frames.get(depth);
    for (index, entity) in entities.iter().enumerate() {
        if next.is_some_and(|frame| entity.id == frame.instance_id)
            && emit_edited_insert(document, entity, transform, depth, view, list, stack)
        {
            continue;
        }
        let dim = true;
        emit_pickable_entity(
            document,
            entity,
            transform,
            entity.id,
            dim,
            paint.rank(entities, index),
            paint.span,
            list,
            stack,
        );
    }
}

// --------------------------------------------------------
// Function: emit_edited_insert
// Purpose: Draw one INSERT that is open for edit. Array cells share
//          this path with the full rebuild and with derive_block_edit.
//          Returns false when the entity is not an INSERT so the caller
//          can draw it as ordinary dimmed context.
// --------------------------------------------------------
fn emit_edited_insert(
    document: &Document,
    entity: &cad_core::Entity,
    transform: Transform2,
    depth: usize,
    view: &BlockEditView,
    list: &mut DisplayList,
    stack: &mut Vec<String>,
) -> bool {
    let cad_core::Geometry::Insert {
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
    else {
        return false;
    };
    if cad_core::nesting_too_deep(stack)
        || stack
            .iter()
            .any(|name| name.eq_ignore_ascii_case(block_name))
    {
        return true;
    }
    let Some(block) = document.block_by_name(block_name) else {
        return true;
    };
    stack.push(block_name.clone());
    let local = Transform2::block_insert(*insertion, *scale, *rotation, *extrusion, block.base_pt);
    let nested = transform.then(local);
    let (cols, rows) = cad_core::clamped_array_counts(*column_count, *row_count);
    let at_leaf = depth + 1 == view.frames.len();
    for col in 0..cols {
        for row in 0..rows {
            let extra =
                Transform2::translate(col as f64 * *column_spacing, row as f64 * *row_spacing);
            let instance = nested.then(extra);
            if at_leaf {
                let paint = PaintList::from_entities(&block.entities);
                for (index, child) in block.entities.iter().enumerate() {
                    emit_pickable_entity(
                        document,
                        child,
                        instance,
                        child.id,
                        false,
                        paint.rank(&block.entities, index),
                        paint.span,
                        list,
                        stack,
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
    true
}

fn dim_weight_colors(vertices: &mut [WeightVertex]) {
    for vertex in vertices {
        let alpha = vertex.color[3];
        if alpha <= 0.0 {
            continue;
        }
        let rgb = Rgb {
            r: unorm8(vertex.color[0]),
            g: unorm8(vertex.color[1]),
            b: unorm8(vertex.color[2]),
        };
        let dimmed = rgb.dim_for_block_context().to_array();
        vertex.color = [dimmed[0], dimmed[1], dimmed[2], alpha];
    }
}

fn dim_vertex_colors(vertices: &mut [GpuVertex]) {
    for vertex in vertices {
        let alpha = vertex.color[3];
        if alpha <= 0.0 {
            continue;
        }
        let rgb = Rgb {
            r: unorm8(vertex.color[0]),
            g: unorm8(vertex.color[1]),
            b: unorm8(vertex.color[2]),
        };
        let dimmed = rgb.dim_for_block_context().to_array();
        vertex.color = [dimmed[0], dimmed[1], dimmed[2], alpha];
    }
}

fn unorm8(channel: f32) -> u8 {
    (channel * 255.0).round().clamp(0.0, 255.0) as u8
}

fn paint_for(document: &Document, entity: &Entity) -> (u32, u32) {
    let paint = PaintList::from_entities(&document.model_space);
    if let Some(index) = document
        .model_space
        .iter()
        .position(|item| item.id == entity.id && item.id.is_assigned())
    {
        return (paint.rank(&document.model_space, index), paint.span);
    }
    (
        entity.draw_order,
        paint.span.max(entity.draw_order.saturating_add(1)),
    )
}

fn emit_pickable_entity(
    document: &Document,
    entity: &cad_core::Entity,
    transform: Transform2,
    pick_id: EntityId,
    dim: bool,
    rank: u32,
    span: u32,
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
    let weight_start = list.weight_vertices.len() as u32;
    let mut pick = EntityPick::new(pick_id);
    {
        let mut sink = TessSink {
            list,
            pick: Some(&mut pick),
            dim,
            show_weights: document.lwdisplay,
            stroke_mm: 0.0,
            chord_tolerance: document.display_chord_tolerance(),
            band: DrawBand::new(),
        };
        sink.band.begin(rank, span);
        vectorize_entity(
            document,
            entity,
            transform,
            CadColor::Aci(7),
            "CONTINUOUS",
            "",
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
        weight_start,
        weight_end: list.weight_vertices.len() as u32,
    });
}

fn emit_top_level_entity(
    document: &Document,
    entity: &Entity,
    entity_index: usize,
    rank: u32,
    span: u32,
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
    let weight_start = list.weight_vertices.len() as u32;
    let mut pick = EntityPick::new(entity_id);
    {
        let mut sink = TessSink {
            list,
            pick: Some(&mut pick),
            dim: false,
            show_weights: document.lwdisplay,
            stroke_mm: 0.0,
            chord_tolerance: document.display_chord_tolerance(),
            band: DrawBand::new(),
        };
        sink.band.begin(rank, span);
        vectorize_entity(
            document,
            entity,
            Transform2::identity(),
            CadColor::Aci(7),
            "CONTINUOUS",
            "",
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
        weight_start,
        weight_end: list.weight_vertices.len() as u32,
    });
    Some(AppendedGeometry {
        line_start,
        fill_start,
        weight_start,
    })
}

fn emit_solid_polyline(
    list: &mut DisplayList,
    pts: &[Point2],
    closed: bool,
    rgb: Rgb,
    depth: f32,
    half_px: Option<f32>,
) {
    if pts.len() < 2 {
        return;
    }
    let n = if closed { pts.len() } else { pts.len() - 1 };
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % pts.len()];
        push_stroke(list, a, b, rgb, depth, half_px);
    }
}

fn emit_triangles(list: &mut DisplayList, tris: &[[Point2; 3]], rgb: Rgb, depth: f32) {
    let color = rgb.to_array();
    let origin = list.origin;
    for tri in tris {
        for point in tri {
            list.triangle_vertices
                .push(to_gpu(*point, origin, color, depth));
        }
    }
}

fn push_stroke(
    list: &mut DisplayList,
    a: Point2,
    b: Point2,
    rgb: Rgb,
    depth: f32,
    half_px: Option<f32>,
) {
    if let Some(half_px) = half_px {
        push_weight_segment(list, a, b, rgb, depth, half_px);
    } else {
        push_line(list, a, b, rgb, depth);
    }
}

fn push_line(list: &mut DisplayList, a: Point2, b: Point2, rgb: Rgb, depth: f32) {
    if !a.is_finite() || !b.is_finite() {
        return;
    }
    let color = rgb.to_array();
    list.line_vertices
        .push(to_gpu(a, list.origin, color, depth));
    list.line_vertices
        .push(to_gpu(b, list.origin, color, depth));
}

fn push_weight_segment(
    list: &mut DisplayList,
    a: Point2,
    b: Point2,
    rgb: Rgb,
    depth: f32,
    half_px: f32,
) {
    if !a.is_finite() || !b.is_finite() || !half_px.is_finite() || half_px <= 0.0 {
        return;
    }
    let color = rgb.to_array();
    let origin = list.origin;
    // Perpendicular flips when the shader walks the segment backwards,
    // so the far end uses the opposite side sign.
    let corners = [(a, b, -1.0), (a, b, 1.0), (b, a, 1.0), (a, b, 1.0), (b, a, -1.0), (b, a, 1.0)];
    for (position, other, side) in corners {
        list.weight_vertices.push(WeightVertex {
            position: to_local(position, origin),
            other: to_local(other, origin),
            color,
            depth,
            side,
            half_px,
            _pad: 0.0,
        });
    }
}

fn to_local(point: Point2, origin: Point2) -> [f32; 2] {
    [(point.x - origin.x) as f32, (point.y - origin.y) as f32]
}

fn to_gpu(p: Point2, origin: Point2, color: [f32; 4], depth: f32) -> GpuVertex {
    GpuVertex {
        position: [(p.x - origin.x) as f32, (p.y - origin.y) as f32],
        color,
        depth,
    }
}

/// Near-white ink on a light sheet becomes black. Dark colors stay stored.
fn ink_on_background(rgb: Rgb, mask: Rgb) -> Rgb {
    if mask.luma() >= 0.5 && rgb.r > 250 && rgb.g > 250 && rgb.b > 250 {
        Rgb { r: 0, g: 0, b: 0 }
    } else {
        rgb
    }
}

fn viewport_mask_rgb() -> Rgb {
    let [r, g, b, _] = crate::VIEWPORT_CLEAR;
    Rgb {
        r: (r * 255.0).round() as u8,
        g: (g * 255.0).round() as u8,
        b: (b * 255.0).round() as u8,
    }
}
