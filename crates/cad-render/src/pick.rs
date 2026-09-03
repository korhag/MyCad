//! Screen-space picking against tessellated world-space primitives.

use cad_core::{EntityId, Extents2, Point2};
use cad_viewport::Camera2;

pub const DEFAULT_PICK_TOLERANCE_PX: f64 = 6.0;
pub(crate) const COMPLEX_PRIMITIVE_COUNT: usize = 48;

// ------------------------------------------------------------
// Type: PickKind
// Purpose: Distinguishes stroked outlines from filled interiors.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickKind {
    Stroke { closed: bool },
    Fill,
}

// ------------------------------------------------------------
// Type: PickPrimitive
// Purpose: One world-space polyline used for hit tests and highlights.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct PickPrimitive {
    pub kind: PickKind,
    pub points: Vec<Point2>,
    pub bounds: Extents2,
}

// ------------------------------------------------------------
// Type: EntityPick
// Purpose: Pick geometry for one top-level model-space entity.
//          Nested block contents stay attached to the parent Insert.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct EntityPick {
    pub entity_id: EntityId,
    pub bounds: Extents2,
    pub primitives: Vec<PickPrimitive>,
    primitive_index: Option<SpatialIndex>,
}

impl EntityPick {
    pub fn new(entity_id: EntityId) -> Self {
        Self {
            entity_id,
            bounds: Extents2::empty(),
            primitives: Vec::new(),
            primitive_index: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.primitives.is_empty() || !self.bounds.is_valid()
    }

    pub fn add_stroke(&mut self, pts: &[Point2], closed: bool) {
        self.push_primitive(PickKind::Stroke { closed }, pts);
    }

    pub fn add_fill(&mut self, pts: &[Point2]) {
        self.push_primitive(PickKind::Fill, pts);
    }

    fn push_primitive(&mut self, kind: PickKind, pts: &[Point2]) {
        let mut points = Vec::with_capacity(pts.len());
        for p in pts {
            if p.is_finite() {
                self.bounds.include(*p);
                points.push(*p);
            }
        }
        if points.len() >= 2 || matches!(kind, PickKind::Fill) && points.len() >= 3 {
            let mut bounds = Extents2::empty();
            for p in &points {
                bounds.include(*p);
            }
            self.primitives.push(PickPrimitive {
                kind,
                points,
                bounds,
            });
        }
    }

    pub fn finalize(&mut self) {
        if self.primitives.len() >= COMPLEX_PRIMITIVE_COUNT {
            self.primitive_index = Some(SpatialIndex::build(
                self.primitives
                    .iter()
                    .enumerate()
                    .map(|(i, primitive)| (i as u32, primitive.bounds)),
            ));
        } else {
            self.primitive_index = None;
        }
    }

    pub fn has_primitive_index(&self) -> bool {
        self.primitive_index.is_some()
    }

    pub fn primitive_index_refs(&self) -> usize {
        self.primitive_index
            .as_ref()
            .map(SpatialIndex::ref_count)
            .unwrap_or(0)
    }

    pub fn stroke_edges(&self) -> impl Iterator<Item = [Point2; 2]> + '_ {
        self.primitives.iter().flat_map(PickPrimitive::stroke_edges)
    }
}

impl PickPrimitive {
    pub fn stroke_edges(&self) -> impl Iterator<Item = [Point2; 2]> + '_ {
        let closed = match self.kind {
            PickKind::Stroke { closed } => closed,
            PickKind::Fill => true,
        };
        stroke_edges(&self.points, closed)
    }
}

// ------------------------------------------------------------
// Function: stroke_edges
// Purpose: Independent line-list pairs for one polyline. Open
//          strokes emit adjacent pairs only; closed strokes add
//          exactly one closing pair. Never joins separate paths.
// ------------------------------------------------------------
pub fn stroke_edges(points: &[Point2], closed: bool) -> impl Iterator<Item = [Point2; 2]> + '_ {
    let count = if points.len() < 2 {
        0
    } else if closed {
        points.len()
    } else {
        points.len() - 1
    };
    (0..count).map(move |i| {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        [a, b]
    })
}

// ------------------------------------------------------------
// Type: SelectBoxMode
// Purpose: AutoCAD window (fully inside) vs crossing (any touch).
//          Left-to-right screen drag is window; right-to-left is crossing.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectBoxMode {
    Window,
    Crossing,
}

impl SelectBoxMode {
    pub fn from_screen_drag(start: Point2, current: Point2) -> Self {
        if current.x >= start.x {
            Self::Window
        } else {
            Self::Crossing
        }
    }
}

// ------------------------------------------------------------
// Type: SpatialIndex
// Purpose: Uniform AABB grid over entity or primitive bounds.
// ------------------------------------------------------------
#[derive(Debug, Clone, Default)]
pub struct SpatialIndex {
    cells: Vec<Vec<u32>>,
    origin_x: f64,
    origin_y: f64,
    inv_cell: f64,
    cols: usize,
    rows: usize,
}

impl SpatialIndex {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub fn build(bounds: impl IntoIterator<Item = (u32, Extents2)>) -> Self {
        let items: Vec<(u32, Extents2)> = bounds
            .into_iter()
            .filter(|(_, extents)| extents.is_valid())
            .collect();
        if items.is_empty() {
            return Self::default();
        }
        let mut world = Extents2::empty();
        for (_, extents) in &items {
            world.union(*extents);
        }
        if !world.is_valid() {
            return Self::default();
        }
        let n = items.len().max(1);
        let target = ((n as f64).sqrt().ceil() as usize).clamp(8, 96);
        let span = world.width().max(world.height()).max(1e-9);
        let cell = (span / target as f64).max(1e-9);
        let cols = ((world.width() / cell).ceil() as usize).clamp(1, 96);
        let rows = ((world.height() / cell).ceil() as usize).clamp(1, 96);
        let mut index = Self {
            cells: vec![Vec::new(); cols * rows],
            origin_x: world.min.x,
            origin_y: world.min.y,
            inv_cell: 1.0 / cell,
            cols,
            rows,
        };
        for (id, extents) in items {
            index.insert(id, extents);
        }
        index
    }

    pub fn gather(&self, region: Extents2, out: &mut Vec<u32>) {
        out.clear();
        if self.cells.is_empty() || !region.is_valid() {
            return;
        }
        let (x0, x1, y0, y1) = self.cell_range(region);
        for y in y0..=y1 {
            for x in x0..=x1 {
                out.extend_from_slice(&self.cells[y * self.cols + x]);
            }
        }
        out.sort_unstable();
        out.dedup();
    }

    pub fn ref_count(&self) -> usize {
        self.cells.iter().map(Vec::len).sum()
    }

    pub fn insert(&mut self, id: u32, extents: Extents2) {
        if !extents.is_valid() {
            return;
        }
        if self.cells.is_empty() {
            *self = Self::build([(id, extents)]);
            return;
        }
        let (x0, x1, y0, y1) = self.cell_range(extents);
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.cells[y * self.cols + x].push(id);
            }
        }
    }

    fn cell_range(&self, region: Extents2) -> (usize, usize, usize, usize) {
        let x0 = self.clamp_col((region.min.x - self.origin_x) * self.inv_cell);
        let x1 = self.clamp_col((region.max.x - self.origin_x) * self.inv_cell);
        let y0 = self.clamp_row((region.min.y - self.origin_y) * self.inv_cell);
        let y1 = self.clamp_row((region.max.y - self.origin_y) * self.inv_cell);
        (x0.min(x1), x0.max(x1), y0.min(y1), y0.max(y1))
    }

    fn clamp_col(&self, value: f64) -> usize {
        if !value.is_finite() {
            return 0;
        }
        value
            .floor()
            .clamp(0.0, (self.cols.saturating_sub(1)) as f64) as usize
    }

    fn clamp_row(&self, value: f64) -> usize {
        if !value.is_finite() {
            return 0;
        }
        value
            .floor()
            .clamp(0.0, (self.rows.saturating_sub(1)) as f64) as usize
    }
}

// ------------------------------------------------------------
// Function: box_select
// Purpose: Return every top-level entity that matches a window or
//          crossing box in world coordinates.
// ------------------------------------------------------------
pub fn box_select(picks: &[EntityPick], region: Extents2, mode: SelectBoxMode) -> Vec<EntityId> {
    let mut out = Vec::new();
    box_select_into(picks, None, region, mode, &mut out);
    out
}

pub fn box_select_into(
    picks: &[EntityPick],
    spatial: Option<&SpatialIndex>,
    region: Extents2,
    mode: SelectBoxMode,
    out: &mut Vec<EntityId>,
) {
    out.clear();
    if !region.is_valid() || region.width() < 1e-15 || region.height() < 1e-15 {
        return;
    }
    let mut scratch = Vec::new();
    let mut slots = Vec::new();
    if let Some(index) = spatial.filter(|index| !index.is_empty()) {
        index.gather(region, &mut slots);
        for slot in slots {
            let Some(pick) = picks.get(slot as usize) else {
                continue;
            };
            if pick_matches(pick, region, mode, &mut scratch) {
                out.push(pick.entity_id);
            }
        }
    } else {
        for pick in picks {
            if pick_matches(pick, region, mode, &mut scratch) {
                out.push(pick.entity_id);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
}

fn pick_matches(
    pick: &EntityPick,
    region: Extents2,
    mode: SelectBoxMode,
    scratch: &mut Vec<u32>,
) -> bool {
    if pick.is_empty() {
        return false;
    }
    match mode {
        SelectBoxMode::Window => region.contains_extents(pick.bounds),
        SelectBoxMode::Crossing => {
            region.intersects(pick.bounds)
                && (region.contains_extents(pick.bounds)
                    || pick_crosses_region(pick, region, scratch))
        }
    }
}

fn pick_crosses_region(pick: &EntityPick, region: Extents2, scratch: &mut Vec<u32>) -> bool {
    if let Some(index) = pick.primitive_index.as_ref() {
        index.gather(region, scratch);
        return scratch.iter().copied().any(|slot| {
            pick.primitives
                .get(slot as usize)
                .is_some_and(|primitive| primitive_crosses_region(primitive, region))
        });
    }
    pick.primitives
        .iter()
        .any(|primitive| primitive_crosses_region(primitive, region))
}

fn primitive_crosses_region(primitive: &PickPrimitive, region: Extents2) -> bool {
    if !region.intersects(primitive.bounds) {
        return false;
    }
    if primitive.points.iter().any(|point| region.contains(*point)) {
        return true;
    }
    for [a, b] in primitive.stroke_edges() {
        if segment_intersects_extents(a, b, region) {
            return true;
        }
    }
    if matches!(primitive.kind, PickKind::Fill) && primitive.points.len() >= 3 {
        let corners = [
            region.min,
            Point2::new(region.max.x, region.min.y),
            region.max,
            Point2::new(region.min.x, region.max.y),
        ];
        return corners
            .iter()
            .any(|corner| point_in_polygon(*corner, &primitive.points));
    }
    false
}

fn segment_intersects_extents(a: Point2, b: Point2, region: Extents2) -> bool {
    let min_x = a.x.min(b.x);
    let max_x = a.x.max(b.x);
    let min_y = a.y.min(b.y);
    let max_y = a.y.max(b.y);
    if max_x < region.min.x || min_x > region.max.x || max_y < region.min.y || min_y > region.max.y
    {
        return false;
    }
    if region.contains(a) || region.contains(b) {
        return true;
    }
    let corners = [
        region.min,
        Point2::new(region.max.x, region.min.y),
        region.max,
        Point2::new(region.min.x, region.max.y),
    ];
    for i in 0..4 {
        if segments_intersect(a, b, corners[i], corners[(i + 1) % 4]) {
            return true;
        }
    }
    false
}

fn segments_intersect(a: Point2, b: Point2, c: Point2, d: Point2) -> bool {
    let d1 = orient(c, d, a);
    let d2 = orient(c, d, b);
    let d3 = orient(a, b, c);
    let d4 = orient(a, b, d);
    if ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0)) {
        return true;
    }
    d1.abs() <= 1e-12 && on_segment(c, d, a)
        || d2.abs() <= 1e-12 && on_segment(c, d, b)
        || d3.abs() <= 1e-12 && on_segment(a, b, c)
        || d4.abs() <= 1e-12 && on_segment(a, b, d)
}

fn orient(a: Point2, b: Point2, c: Point2) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

fn on_segment(a: Point2, b: Point2, p: Point2) -> bool {
    p.x >= a.x.min(b.x) - 1e-12
        && p.x <= a.x.max(b.x) + 1e-12
        && p.y >= a.y.min(b.y) - 1e-12
        && p.y <= a.y.max(b.y) + 1e-12
}

// ------------------------------------------------------------
// Function: hit_test
// Purpose: Return the top-level entity under the pointer. Strokes
//          within the pixel aperture beat filled interiors so a
//          line on a hatch remains selectable. Equal distances use
//          later draw order.
// ------------------------------------------------------------
pub fn hit_test(
    picks: &[EntityPick],
    camera: &Camera2,
    screen: Point2,
    viewport_origin: Point2,
    viewport_size: Point2,
    tolerance_px: f64,
) -> Option<EntityId> {
    let tolerance_px = tolerance_px.max(0.5);
    let mut best_stroke: Option<Hit> = None;
    let mut best_fill: Option<Hit> = None;

    for (draw_order, pick) in picks.iter().enumerate() {
        if pick.is_empty() {
            continue;
        }
        if !screen_bounds_hit(
            pick,
            camera,
            screen,
            viewport_origin,
            viewport_size,
            tolerance_px,
        ) {
            continue;
        }
        for primitive in &pick.primitives {
            match primitive.kind {
                PickKind::Stroke { .. } => {
                    if let Some(dist) = stroke_screen_distance(
                        primitive,
                        camera,
                        screen,
                        viewport_origin,
                        viewport_size,
                    ) {
                        if dist <= tolerance_px {
                            consider_hit(&mut best_stroke, pick.entity_id, draw_order, dist);
                        }
                    }
                }
                PickKind::Fill => {
                    if fill_contains_screen(
                        primitive,
                        camera,
                        screen,
                        viewport_origin,
                        viewport_size,
                    ) {
                        consider_hit(&mut best_fill, pick.entity_id, draw_order, 0.0);
                    }
                }
            }
        }
    }

    best_stroke.or(best_fill).map(|hit| hit.entity_id)
}

struct Hit {
    entity_id: EntityId,
    draw_order: usize,
    distance: f64,
}

fn consider_hit(slot: &mut Option<Hit>, entity_id: EntityId, draw_order: usize, distance: f64) {
    match slot {
        Some(existing) if existing.distance + 1e-6 < distance => {}
        Some(existing) if (existing.distance - distance).abs() <= 1e-6 => {
            if draw_order > existing.draw_order {
                *existing = Hit {
                    entity_id,
                    draw_order,
                    distance,
                };
            }
        }
        _ => {
            *slot = Some(Hit {
                entity_id,
                draw_order,
                distance,
            });
        }
    }
}

fn screen_bounds_hit(
    pick: &EntityPick,
    camera: &Camera2,
    screen: Point2,
    viewport_origin: Point2,
    viewport_size: Point2,
    tolerance_px: f64,
) -> bool {
    let corners = [
        Point2::new(pick.bounds.min.x, pick.bounds.min.y),
        Point2::new(pick.bounds.max.x, pick.bounds.min.y),
        Point2::new(pick.bounds.max.x, pick.bounds.max.y),
        Point2::new(pick.bounds.min.x, pick.bounds.max.y),
    ];
    let mut bounds = Extents2::empty();
    for corner in corners {
        bounds.include(camera.world_to_screen(corner, viewport_origin, viewport_size));
    }
    bounds.inflated(tolerance_px).contains(screen)
}

fn stroke_screen_distance(
    primitive: &PickPrimitive,
    camera: &Camera2,
    screen: Point2,
    viewport_origin: Point2,
    viewport_size: Point2,
) -> Option<f64> {
    let mut best = f64::INFINITY;
    for [a, b] in primitive.stroke_edges() {
        let sa = camera.world_to_screen(a, viewport_origin, viewport_size);
        let sb = camera.world_to_screen(b, viewport_origin, viewport_size);
        best = best.min(point_to_segment_distance(screen, sa, sb));
    }
    best.is_finite().then_some(best)
}

fn fill_contains_screen(
    primitive: &PickPrimitive,
    camera: &Camera2,
    screen: Point2,
    viewport_origin: Point2,
    viewport_size: Point2,
) -> bool {
    if primitive.points.len() < 3 {
        return false;
    }
    let poly: Vec<Point2> = primitive
        .points
        .iter()
        .map(|p| camera.world_to_screen(*p, viewport_origin, viewport_size))
        .collect();
    point_in_polygon(screen, &poly)
}

fn point_to_segment_distance(p: Point2, a: Point2, b: Point2) -> f64 {
    let abx = b.x - a.x;
    let aby = b.y - a.y;
    let len_sq = abx * abx + aby * aby;
    let t = if len_sq < 1e-24 {
        0.0
    } else {
        (((p.x - a.x) * abx + (p.y - a.y) * aby) / len_sq).clamp(0.0, 1.0)
    };
    let cx = a.x + abx * t;
    let cy = a.y + aby * t;
    let dx = p.x - cx;
    let dy = p.y - cy;
    (dx * dx + dy * dy).sqrt()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point2 {
        Point2::new(x, y)
    }

    #[test]
    fn open_stroke_emits_adjacent_pairs_only() {
        let edges: Vec<_> =
            stroke_edges(&[p(0.0, 0.0), p(10.0, 0.0), p(100.0, 100.0)], false).collect();
        assert_eq!(
            edges,
            vec![[p(0.0, 0.0), p(10.0, 0.0)], [p(10.0, 0.0), p(100.0, 100.0)],]
        );
    }

    #[test]
    fn closed_stroke_adds_exactly_one_closing_pair() {
        let edges: Vec<_> = stroke_edges(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0)], true).collect();
        assert_eq!(edges.len(), 3);
        assert_eq!(edges[2], [p(1.0, 1.0), p(0.0, 0.0)]);
    }

    #[test]
    fn separate_primitives_do_not_grow_a_connector_edge() {
        let mut pick = EntityPick::new(EntityId(0));
        pick.add_stroke(&[p(0.0, 0.0), p(10.0, 0.0)], false);
        pick.add_stroke(&[p(100.0, 100.0), p(110.0, 100.0)], false);
        pick.add_stroke(&[p(-200.0, 50.0), p(-190.0, 50.0)], false);
        let edges: Vec<_> = pick.stroke_edges().collect();
        assert_eq!(edges.len(), 3);
        assert!(!contains_segment(&edges, [p(10.0, 0.0), p(100.0, 100.0)]));
        assert!(!contains_segment(
            &edges,
            [p(110.0, 100.0), p(-200.0, 50.0)]
        ));
    }

    #[test]
    fn repeated_points_stay_inside_one_primitive() {
        let edges: Vec<_> = stroke_edges(&[p(0.0, 0.0), p(0.0, 0.0), p(5.0, 0.0)], false).collect();
        assert_eq!(
            edges,
            vec![[p(0.0, 0.0), p(0.0, 0.0)], [p(0.0, 0.0), p(5.0, 0.0)]]
        );
    }

    fn contains_segment(edges: &[[Point2; 2]], needle: [Point2; 2]) -> bool {
        edges
            .iter()
            .any(|edge| (*edge == needle) || (*edge == [needle[1], needle[0]]))
    }

    #[test]
    fn left_to_right_drag_is_window_mode() {
        assert_eq!(
            SelectBoxMode::from_screen_drag(p(10.0, 40.0), p(80.0, 10.0)),
            SelectBoxMode::Window
        );
        assert_eq!(
            SelectBoxMode::from_screen_drag(p(80.0, 10.0), p(10.0, 40.0)),
            SelectBoxMode::Crossing
        );
    }

    #[test]
    fn insert_adds_bounds_to_an_existing_grid() {
        let mut index =
            SpatialIndex::build([(0, Extents2::from_corners(p(0.0, 0.0), p(10.0, 10.0)))]);
        index.insert(1, Extents2::from_corners(p(8.0, 8.0), p(9.0, 9.0)));
        let mut slots = Vec::new();
        index.gather(Extents2::from_corners(p(7.5, 7.5), p(9.5, 9.5)), &mut slots);
        assert!(slots.contains(&0));
        assert!(slots.contains(&1));
    }

    #[test]
    fn insert_builds_a_grid_when_the_index_is_empty() {
        let mut index = SpatialIndex::empty();
        index.insert(3, Extents2::from_corners(p(1.0, 1.0), p(2.0, 2.0)));
        let mut slots = Vec::new();
        index.gather(Extents2::from_corners(p(0.5, 0.5), p(2.5, 2.5)), &mut slots);
        assert_eq!(slots, vec![3]);
    }
}
