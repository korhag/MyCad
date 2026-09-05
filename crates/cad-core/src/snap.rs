//! Semantic object-snap features extracted from native CAD geometry.

use std::collections::HashMap;

use crate::{
    ocs_to_wcs, Document, Entity, EntityId, Extents2, Geometry, Point2, Point3, Transform2,
};

// ------------------------------------------------------------
// Type: SnapKind
// Purpose: Semantic feature type used by object-snap resolution
//          and its viewport marker.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapKind {
    Endpoint,
    Midpoint,
    Center,
}

// ------------------------------------------------------------
// Type: SnapFeature
// Purpose: One exact world-space snap location. These locations
//          come from entity geometry, never tessellated vertices.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapFeature {
    pub point: Point2,
    pub kind: SnapKind,
}

// ------------------------------------------------------------
// Type: SnapIndex
// Purpose: Uniform world-space grid built once per document so
//          pointer movement only examines nearby snap features.
// ------------------------------------------------------------
#[derive(Debug, Clone, Default)]
pub struct SnapIndex {
    features: Vec<SnapFeature>,
    alive: Vec<bool>,
    live_count: usize,
    owner_slots: HashMap<EntityId, Vec<u32>>,
    cells: Vec<Vec<u32>>,
    origin: Point2,
    inv_cell: f64,
    cols: usize,
    rows: usize,
}

impl SnapIndex {
    pub fn build(document: &Document) -> Self {
        let _span = crate::perf::span("SnapIndex::build");
        let mut features = Vec::new();
        let mut stack = Vec::new();
        for entity in &document.model_space {
            collect_entity_features(
                document,
                entity,
                entity.id,
                Transform2::identity(),
                &mut stack,
                &mut features,
            );
        }
        Self::from_owned(features)
    }

    pub fn append_entity(&mut self, document: &Document, entity: &Entity) {
        let mut added = Vec::new();
        let mut stack = Vec::new();
        collect_entity_features(
            document,
            entity,
            entity.id,
            Transform2::identity(),
            &mut stack,
            &mut added,
        );
        if added.is_empty() {
            return;
        }
        if self.cells.is_empty() {
            *self = Self::from_owned(added);
            return;
        }
        for (feature, owner) in added {
            let slot = self.features.len() as u32;
            let (x, y) = self.cell(feature.point);
            self.features.push(feature);
            self.alive.push(true);
            self.live_count += 1;
            self.owner_slots.entry(owner).or_default().push(slot);
            self.cells[y * self.cols + x].push(slot);
        }
    }

    pub fn remove_entity(&mut self, id: EntityId) {
        let Some(slots) = self.owner_slots.remove(&id) else {
            return;
        };
        for slot in slots {
            if let Some(alive) = self.alive.get_mut(slot as usize) {
                if *alive {
                    *alive = false;
                    self.live_count = self.live_count.saturating_sub(1);
                }
            }
        }
    }

    pub fn replace_entity(&mut self, document: &Document, entity: &Entity) {
        self.remove_entity(entity.id);
        self.append_entity(document, entity);
    }

    pub fn from_features(features: Vec<SnapFeature>) -> Self {
        Self::from_owned(
            features
                .into_iter()
                .map(|feature| (feature, EntityId::UNASSIGNED))
                .collect(),
        )
    }

    fn from_owned(items: Vec<(SnapFeature, EntityId)>) -> Self {
        if items.is_empty() {
            return Self::default();
        }
        let mut features = Vec::with_capacity(items.len());
        let mut owner_slots: HashMap<EntityId, Vec<u32>> = HashMap::new();
        for (index, (feature, owner)) in items.into_iter().enumerate() {
            owner_slots.entry(owner).or_default().push(index as u32);
            features.push(feature);
        }
        let Some(world) = Extents2::from_points(features.iter().map(|feature| feature.point))
        else {
            return Self::default();
        };
        let target = ((features.len().max(1) as f64).sqrt().ceil() as usize).clamp(8, 96);
        let span = world.width().max(world.height()).max(1e-9);
        let cell_size = (span / target as f64).max(1e-9);
        let cols = ((world.width() / cell_size).ceil() as usize).clamp(1, 96);
        let rows = ((world.height() / cell_size).ceil() as usize).clamp(1, 96);
        let live_count = features.len();
        let mut index = Self {
            alive: vec![true; live_count],
            live_count,
            features,
            owner_slots,
            cells: vec![Vec::new(); cols * rows],
            origin: world.min,
            inv_cell: 1.0 / cell_size,
            cols,
            rows,
        };
        for slot in 0..index.features.len() {
            let point = index.features[slot].point;
            let (x, y) = index.cell(point);
            index.cells[y * index.cols + x].push(slot as u32);
        }
        index
    }

    pub fn is_empty(&self) -> bool {
        self.live_count == 0
    }

    pub fn len(&self) -> usize {
        self.live_count
    }

    pub fn query(&self, region: Extents2, out: &mut Vec<SnapFeature>) {
        out.clear();
        if self.cells.is_empty() || !region.is_valid() {
            return;
        }
        let (x0, y0) = self.cell(region.min);
        let (x1, y1) = self.cell(region.max);
        for y in y0.min(y1)..=y0.max(y1) {
            for x in x0.min(x1)..=x0.max(x1) {
                out.extend(
                    self.cells[y * self.cols + x]
                        .iter()
                        .copied()
                        .filter(|&slot| self.alive.get(slot as usize).copied().unwrap_or(false))
                        .filter_map(|slot| self.features.get(slot as usize))
                        .copied()
                        .filter(|feature| region.contains(feature.point)),
                );
            }
        }
    }

    fn cell(&self, point: Point2) -> (usize, usize) {
        let x = ((point.x - self.origin.x) * self.inv_cell)
            .floor()
            .clamp(0.0, self.cols.saturating_sub(1) as f64) as usize;
        let y = ((point.y - self.origin.y) * self.inv_cell)
            .floor()
            .clamp(0.0, self.rows.saturating_sub(1) as f64) as usize;
        (x, y)
    }
}

fn collect_entity_features(
    document: &Document,
    entity: &Entity,
    owner: EntityId,
    transform: Transform2,
    block_stack: &mut Vec<String>,
    out: &mut Vec<(SnapFeature, EntityId)>,
) {
    if !entity.visible || !document.layer_is_visible(&entity.layer) {
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
            if block_stack
                .iter()
                .any(|name| name.eq_ignore_ascii_case(block_name))
            {
                return;
            }
            let Some(block) = document.blocks.get(block_name) else {
                return;
            };
            block_stack.push(block_name.clone());
            for col in 0..(*column_count).max(1) {
                for row in 0..(*row_count).max(1) {
                    let array_offset = Transform2::translate(
                        col as f64 * *column_spacing,
                        row as f64 * *row_spacing,
                    );
                    let local = Transform2::block_insert(
                        *insertion,
                        *scale,
                        *rotation,
                        *extrusion,
                        block.base_pt,
                    )
                    .then(array_offset);
                    let nested = transform.then(local);
                    for child in &block.entities {
                        collect_entity_features(document, child, owner, nested, block_stack, out);
                    }
                }
            }
            block_stack.pop();
        }
        Geometry::Dimension { block_name } => {
            if block_stack
                .iter()
                .any(|name| name.eq_ignore_ascii_case(block_name))
            {
                return;
            }
            if let Some(block) = document.blocks.get(block_name) {
                block_stack.push(block_name.clone());
                for child in &block.entities {
                    collect_entity_features(document, child, owner, transform, block_stack, out);
                }
                block_stack.pop();
            }
        }
        Geometry::Line { start, end } => {
            let start = transform.apply(start.xy());
            let end = transform.apply(end.xy());
            push(out, owner, start, SnapKind::Endpoint);
            push(out, owner, end, SnapKind::Endpoint);
            push(out, owner, start.lerp(end, 0.5), SnapKind::Midpoint);
        }
        Geometry::Circle {
            center, extrusion, ..
        } => {
            push(
                out,
                owner,
                transform.apply(ocs_to_wcs(*center, *extrusion).xy()),
                SnapKind::Center,
            );
        }
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            extrusion,
        } => {
            let mut sweep = *end_angle - *start_angle;
            if sweep.abs() < 1e-15 {
                sweep = std::f64::consts::TAU;
            }
            while sweep <= 0.0 {
                sweep += std::f64::consts::TAU;
            }
            while sweep > std::f64::consts::TAU + 1e-12 {
                sweep -= std::f64::consts::TAU;
            }
            let point_at = |angle: f64| {
                transform.apply(
                    ocs_to_wcs(
                        Point3::new(
                            center.x + radius * angle.cos(),
                            center.y + radius * angle.sin(),
                            center.z,
                        ),
                        *extrusion,
                    )
                    .xy(),
                )
            };
            push(
                out,
                owner,
                transform.apply(ocs_to_wcs(*center, *extrusion).xy()),
                SnapKind::Center,
            );
            push(out, owner, point_at(*start_angle), SnapKind::Endpoint);
            push(
                out,
                owner,
                point_at(*start_angle + sweep),
                SnapKind::Endpoint,
            );
            push(
                out,
                owner,
                point_at(*start_angle + sweep * 0.5),
                SnapKind::Midpoint,
            );
        }
        Geometry::LwPolyline {
            vertices,
            closed,
            extrusion,
            ..
        } => collect_polyline_features(vertices, *closed, *extrusion, transform, owner, out),
        Geometry::Polyline {
            vertices, closed, ..
        } => collect_polyline_features(
            vertices,
            *closed,
            Point3::new(0.0, 0.0, 1.0),
            transform,
            owner,
            out,
        ),
        _ => {}
    }
}

fn collect_polyline_features(
    vertices: &[crate::PolyVertex],
    closed: bool,
    extrusion: Point3,
    transform: Transform2,
    owner: EntityId,
    out: &mut Vec<(SnapFeature, EntityId)>,
) {
    for vertex in vertices {
        push(
            out,
            owner,
            transform.apply(ocs_to_wcs(vertex.point, extrusion).xy()),
            SnapKind::Endpoint,
        );
    }
    let segment_count = if closed {
        vertices.len()
    } else {
        vertices.len().saturating_sub(1)
    };
    for index in 0..segment_count {
        let a = vertices[index];
        let b = vertices[(index + 1) % vertices.len()];
        let midpoint = bulge_midpoint(a.point.xy(), b.point.xy(), a.bulge)
            .unwrap_or_else(|| a.point.xy().lerp(b.point.xy(), 0.5));
        let midpoint = ocs_to_wcs(Point3::new(midpoint.x, midpoint.y, a.point.z), extrusion).xy();
        push(out, owner, transform.apply(midpoint), SnapKind::Midpoint);
    }
}

fn bulge_midpoint(start: Point2, end: Point2, bulge: f64) -> Option<Point2> {
    if bulge.abs() < 1e-12 {
        return None;
    }
    let chord = start.distance(end);
    if chord < 1e-15 {
        return None;
    }
    let offset = chord * (1.0 - bulge * bulge) / (4.0 * bulge);
    let ux = (end.x - start.x) / chord;
    let uy = (end.y - start.y) / chord;
    let center = Point2::new(
        (start.x + end.x) * 0.5 - uy * offset,
        (start.y + end.y) * 0.5 + ux * offset,
    );
    let radius = chord * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
    let start_angle = (start.y - center.y).atan2(start.x - center.x);
    let angle = start_angle + 2.0 * bulge.atan();
    Some(Point2::new(
        center.x + radius * angle.cos(),
        center.y + radius * angle.sin(),
    ))
}

fn push(out: &mut Vec<(SnapFeature, EntityId)>, owner: EntityId, point: Point2, kind: SnapKind) {
    if point.is_finite() {
        out.push((SnapFeature { point, kind }, owner));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockDefinition, Geometry, PolyVertex};

    #[test]
    fn line_features_are_semantic() {
        let mut document = Document::default();
        document.model_space.push(Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(10.0, 0.0),
        }));
        let index = SnapIndex::build(&document);
        assert_eq!(index.len(), 3);
        let mut found = Vec::new();
        index.query(
            Extents2::from_corners(Point2::new(4.9, -0.1), Point2::new(5.1, 0.1)),
            &mut found,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, SnapKind::Midpoint);
    }

    #[test]
    fn append_entity_snaps_the_new_line_immediately() {
        let mut document = Document::default();
        document.model_space.push(Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(10.0, 0.0),
        }));
        let mut index = SnapIndex::build(&document);
        let added = Entity::new(Geometry::Line {
            start: Point3::from_xy(10.0, 0.0),
            end: Point3::from_xy(20.0, 0.0),
        });
        document.model_space.push(added.clone());
        index.append_entity(&document, &added);
        assert_eq!(index.len(), 6);
        let mut found = Vec::new();
        index.query(
            Extents2::from_corners(Point2::new(19.9, -0.1), Point2::new(20.1, 0.1)),
            &mut found,
        );
        assert!(found
            .iter()
            .any(|feature| feature.kind == SnapKind::Endpoint
                && (feature.point.x - 20.0).abs() < 1e-9));
        found.clear();
        index.query(
            Extents2::from_corners(Point2::new(14.9, -0.1), Point2::new(15.1, 0.1)),
            &mut found,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, SnapKind::Midpoint);
    }

    #[test]
    fn append_entity_builds_an_empty_index() {
        let mut document = Document::default();
        let added = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(4.0, 0.0),
        });
        document.model_space.push(added.clone());
        let mut index = SnapIndex::default();
        index.append_entity(&document, &added);
        assert_eq!(index.len(), 3);
        let mut found = Vec::new();
        index.query(
            Extents2::from_corners(Point2::new(-0.1, -0.1), Point2::new(0.1, 0.1)),
            &mut found,
        );
        assert!(found
            .iter()
            .any(|feature| feature.kind == SnapKind::Endpoint));
    }

    #[test]
    fn nested_insert_applies_block_transform() {
        let mut document = Document::default();
        document.blocks.insert(
            "B".into(),
            BlockDefinition {
                name: "B".into(),
                base_pt: Point3::from_xy(1.0, 0.0),
                entities: vec![Entity::new(Geometry::Line {
                    start: Point3::from_xy(1.0, 0.0),
                    end: Point3::from_xy(3.0, 0.0),
                })],
                ..Default::default()
            },
        );
        document.model_space.push(Entity::new(Geometry::Insert {
            block_name: "B".into(),
            insertion: Point3::from_xy(10.0, 5.0),
            scale: Point3::new(2.0, 2.0, 1.0),
            rotation: 0.0,
            extrusion: Point3::new(0.0, 0.0, 1.0),
            attribs: Vec::new(),
            column_count: 1,
            row_count: 1,
            column_spacing: 0.0,
            row_spacing: 0.0,
            configuration: None,
        }));
        let index = SnapIndex::build(&document);
        let mut found = Vec::new();
        index.query(
            Extents2::from_corners(Point2::new(9.9, 4.9), Point2::new(10.1, 5.1)),
            &mut found,
        );
        assert!(found
            .iter()
            .any(|feature| feature.kind == SnapKind::Endpoint));
    }

    #[test]
    fn bulged_polyline_midpoint_is_on_arc() {
        let vertices = vec![
            PolyVertex {
                point: Point3::from_xy(0.0, 0.0),
                bulge: 1.0,
            },
            PolyVertex {
                point: Point3::from_xy(10.0, 0.0),
                bulge: 0.0,
            },
        ];
        let mut features = Vec::new();
        collect_polyline_features(
            &vertices,
            false,
            Point3::new(0.0, 0.0, 1.0),
            Transform2::identity(),
            EntityId::UNASSIGNED,
            &mut features,
        );
        let midpoint = features
            .iter()
            .find(|(feature, _)| feature.kind == SnapKind::Midpoint)
            .map(|(feature, _)| feature)
            .unwrap();
        assert!((midpoint.point.x - 5.0).abs() < 1e-9);
        assert!((midpoint.point.y + 5.0).abs() < 1e-9);
    }

    #[test]
    fn replace_and_remove_entity_update_nearby_snaps() {
        let mut document = Document::default();
        let first = document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(10.0, 0.0),
        }));
        let second = document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 4.0),
            end: Point3::from_xy(10.0, 4.0),
        }));
        let mut index = SnapIndex::build(&document);
        assert_eq!(index.len(), 6);

        let mut moved = first.clone();
        if let Geometry::Line { start, end } = &mut moved.geometry {
            *start = Point3::from_xy(0.0, 8.0);
            *end = Point3::from_xy(10.0, 8.0);
        }
        document.replace_entity_in(&crate::EntitySpace::ModelSpace, first.id, moved.clone());
        index.replace_entity(&document, &moved);

        let mut found = Vec::new();
        index.query(
            Extents2::from_corners(Point2::new(4.9, -0.1), Point2::new(5.1, 0.1)),
            &mut found,
        );
        assert!(found.is_empty());
        index.query(
            Extents2::from_corners(Point2::new(4.9, 7.9), Point2::new(5.1, 8.1)),
            &mut found,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, SnapKind::Midpoint);

        index.remove_entity(second.id);
        assert_eq!(index.len(), 3);
        index.query(
            Extents2::from_corners(Point2::new(4.9, 3.9), Point2::new(5.1, 4.1)),
            &mut found,
        );
        assert!(found.is_empty());
    }
}
