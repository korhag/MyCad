//! Semantic measurable primitives, built once per document like SnapIndex.

use crate::entity::PolyVertex;
use crate::measure::{
    angle_on_arc, bulge_circle, point_bulge_distance, point_in_closed_polyline, point_on_circle,
    point_segment_distance, MeasureError,
};
use crate::{
    ocs_to_wcs, Document, Entity, EntityId, Extents2, Geometry, Point2, Point3, Transform2,
    GEOM_TOLERANCE,
};

pub const MEASURE_APERTURE_PX: f64 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasureRole {
    Straight,
    Curve,
    Closed,
}

#[derive(Debug, Clone)]
pub enum MeasureGeom {
    Straight {
        start: Point2,
        end: Point2,
    },
    Circle {
        local_center: Point2,
        local_radius: f64,
        to_world: Transform2,
        uniform: bool,
    },
    Arc {
        local_center: Point2,
        local_radius: f64,
        start_angle: f64,
        end_angle: f64,
        to_world: Transform2,
        uniform: bool,
    },
    Bulge {
        start: Point2,
        end: Point2,
        bulge: f64,
    },
    ClosedLoop {
        vertices: Vec<PolyVertex>,
        uniform: bool,
    },
}

#[derive(Debug, Clone)]
pub struct MeasurePrimitive {
    pub owner: EntityId,
    pub nested: Vec<EntityId>,
    pub geom: MeasureGeom,
    pub bounds: Extents2,
}

impl MeasurePrimitive {
    pub fn role(&self) -> MeasureRole {
        match self.geom {
            MeasureGeom::Straight { .. } => MeasureRole::Straight,
            MeasureGeom::Circle { .. } | MeasureGeom::Arc { .. } => MeasureRole::Curve,
            MeasureGeom::Bulge { .. } => MeasureRole::Curve,
            MeasureGeom::ClosedLoop { .. } => MeasureRole::Closed,
        }
    }

    pub fn distance_to(&self, point: Point2) -> f64 {
        match &self.geom {
            MeasureGeom::Straight { start, end } => point_segment_distance(point, *start, *end),
            MeasureGeom::Circle {
                local_center,
                local_radius,
                to_world,
                ..
            } => world_curve_distance(point, *to_world, |local| {
                Some(point_on_circle(*local_center, *local_radius, local))
            }),
            MeasureGeom::Arc {
                local_center,
                local_radius,
                start_angle,
                end_angle,
                to_world,
                ..
            } => world_curve_distance(point, *to_world, |local| {
                if angle_on_arc(local, *local_center, *start_angle, *end_angle) {
                    Some(point_on_circle(*local_center, *local_radius, local))
                } else {
                    None
                }
            }),
            MeasureGeom::Bulge { start, end, bulge } => {
                point_bulge_distance(point, *start, *end, *bulge)
            }
            MeasureGeom::ClosedLoop { vertices, .. } => {
                let mut best = f64::INFINITY;
                let n = vertices.len();
                for i in 0..n {
                    let a = vertices[i];
                    let b = vertices[(i + 1) % n];
                    let d = if a.bulge.abs() > GEOM_TOLERANCE {
                        point_bulge_distance(point, a.point.xy(), b.point.xy(), a.bulge)
                    } else {
                        point_segment_distance(point, a.point.xy(), b.point.xy())
                    };
                    best = best.min(d);
                }
                if point_in_closed_polyline(point, vertices) {
                    best = best.min(0.0);
                }
                best
            }
        }
    }
}

// ------------------------------------------------------------
// Type: MeasureIndex
// Purpose: Spatial index of exact measurable primitives.
// ------------------------------------------------------------
#[derive(Debug, Clone, Default)]
pub struct MeasureIndex {
    primitives: Vec<MeasurePrimitive>,
    cells: Vec<Vec<u32>>,
    origin: Point2,
    inv_cell: f64,
    cols: usize,
    rows: usize,
}

impl MeasureIndex {
    pub fn build(document: &Document) -> Self {
        let mut primitives = Vec::new();
        let mut stack = Vec::new();
        let mut path = Vec::new();
        for entity in &document.model_space {
            collect(
                document,
                entity,
                entity.id,
                Transform2::identity(),
                &mut stack,
                &mut path,
                &mut primitives,
            );
        }
        Self::from_primitives(primitives)
    }

    pub fn append_entity(&mut self, document: &Document, entity: &Entity) {
        let mut added = Vec::new();
        let mut stack = Vec::new();
        let mut path = Vec::new();
        collect(
            document,
            entity,
            entity.id,
            Transform2::identity(),
            &mut stack,
            &mut path,
            &mut added,
        );
        if added.is_empty() {
            return;
        }
        if self.cells.is_empty() {
            *self = Self::from_primitives(added);
            return;
        }
        for primitive in added {
            let slot = self.primitives.len() as u32;
            self.insert_slot(slot, primitive.bounds);
            self.primitives.push(primitive);
        }
    }

    pub fn from_primitives(primitives: Vec<MeasurePrimitive>) -> Self {
        let Some(world) = primitives.iter().fold(None, |acc, primitive| {
            let mut e = acc.unwrap_or_else(Extents2::empty);
            e.union(primitive.bounds);
            e.is_valid().then_some(e)
        }) else {
            return Self::default();
        };
        let target = ((primitives.len().max(1) as f64).sqrt().ceil() as usize).clamp(8, 96);
        let span = world.width().max(world.height()).max(1e-9);
        let cell_size = (span / target as f64).max(1e-9);
        let cols = ((world.width() / cell_size).ceil() as usize).clamp(1, 96);
        let rows = ((world.height() / cell_size).ceil() as usize).clamp(1, 96);
        let mut index = Self {
            primitives,
            cells: vec![Vec::new(); cols * rows],
            origin: world.min,
            inv_cell: 1.0 / cell_size,
            cols,
            rows,
        };
        for slot in 0..index.primitives.len() {
            let bounds = index.primitives[slot].bounds;
            index.insert_slot(slot as u32, bounds);
        }
        index
    }

    pub fn is_empty(&self) -> bool {
        self.primitives.is_empty()
    }

    pub fn pick(
        &self,
        point: Point2,
        aperture: f64,
        role: Option<MeasureRole>,
    ) -> Option<&MeasurePrimitive> {
        let region = Extents2::from_corners(
            Point2::new(point.x - aperture, point.y - aperture),
            Point2::new(point.x + aperture, point.y + aperture),
        );
        let mut best: Option<(f64, usize)> = None;
        self.for_region(region, |slot| {
            let primitive = &self.primitives[slot];
            if let Some(want) = role {
                if !matches_role(&primitive.geom, want) {
                    return;
                }
            }
            let distance = primitive.distance_to(point);
            if distance <= aperture {
                match best {
                    Some((best_d, _)) if distance >= best_d => {}
                    _ => best = Some((distance, slot)),
                }
            }
        });
        best.map(|(_, slot)| &self.primitives[slot])
    }

    pub fn primitives_for_owner(&self, owner: EntityId) -> impl Iterator<Item = &MeasurePrimitive> {
        self.primitives.iter().filter(move |p| p.owner == owner)
    }

    fn for_region(&self, region: Extents2, mut visit: impl FnMut(usize)) {
        if self.cells.is_empty() || !region.is_valid() {
            return;
        }
        let (x0, y0) = self.cell(region.min);
        let (x1, y1) = self.cell(region.max);
        let mut seen = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                for slot in &self.cells[y * self.cols + x] {
                    let slot = *slot as usize;
                    if !seen.contains(&slot) {
                        seen.push(slot);
                        visit(slot);
                    }
                }
            }
        }
    }

    fn insert_slot(&mut self, slot: u32, bounds: Extents2) {
        if self.cells.is_empty() || !bounds.is_valid() {
            return;
        }
        let (x0, y0) = self.cell(bounds.min);
        let (x1, y1) = self.cell(bounds.max);
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.cells[y * self.cols + x].push(slot);
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

fn matches_role(geom: &MeasureGeom, role: MeasureRole) -> bool {
    match role {
        MeasureRole::Straight => matches!(geom, MeasureGeom::Straight { .. }),
        MeasureRole::Curve => {
            matches!(geom, MeasureGeom::Circle { .. } | MeasureGeom::Arc { .. })
        }
        MeasureRole::Closed => {
            matches!(
                geom,
                MeasureGeom::ClosedLoop { .. } | MeasureGeom::Circle { .. }
            )
        }
    }
}

fn world_curve_distance(
    point: Point2,
    to_world: Transform2,
    closest_local: impl Fn(Point2) -> Option<Point2>,
) -> f64 {
    let Some(inverse) = to_world.try_inverse() else {
        return f64::INFINITY;
    };
    let local = inverse.apply(point);
    let Some(closest) = closest_local(local) else {
        return f64::INFINITY;
    };
    point.distance(to_world.apply(closest))
}

fn collect(
    document: &Document,
    entity: &Entity,
    owner: EntityId,
    transform: Transform2,
    block_stack: &mut Vec<String>,
    path: &mut Vec<EntityId>,
    out: &mut Vec<MeasurePrimitive>,
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
                        path.push(child.id);
                        collect(document, child, owner, nested, block_stack, path, out);
                        path.pop();
                    }
                }
            }
            block_stack.pop();
        }
        Geometry::Line { start, end } => {
            let start = transform.apply(start.xy());
            let end = transform.apply(end.xy());
            push_straight(out, owner, path, start, end);
        }
        Geometry::Circle {
            center,
            radius,
            extrusion,
        } => push_circle(out, owner, path, transform, *center, *radius, *extrusion),
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            extrusion,
        } => push_arc(
            out,
            owner,
            path,
            transform,
            *center,
            *radius,
            *start_angle,
            *end_angle,
            *extrusion,
        ),
        Geometry::LwPolyline {
            vertices,
            closed,
            extrusion,
            ..
        } => push_polyline(out, owner, path, transform, vertices, *closed, *extrusion),
        Geometry::Polyline {
            vertices, closed, ..
        } => push_polyline(
            out,
            owner,
            path,
            transform,
            vertices,
            *closed,
            Point3::new(0.0, 0.0, 1.0),
        ),
        _ => {}
    }
}

fn push_straight(
    out: &mut Vec<MeasurePrimitive>,
    owner: EntityId,
    path: &[EntityId],
    start: Point2,
    end: Point2,
) {
    if start.distance(end) <= GEOM_TOLERANCE {
        return;
    }
    let bounds = Extents2::from_corners(start, end);
    out.push(MeasurePrimitive {
        owner,
        nested: path.to_vec(),
        geom: MeasureGeom::Straight { start, end },
        bounds,
    });
}

fn push_circle(
    out: &mut Vec<MeasurePrimitive>,
    owner: EntityId,
    path: &[EntityId],
    transform: Transform2,
    center: Point3,
    radius: f64,
    extrusion: Point3,
) {
    let local_center = ocs_to_wcs(center, extrusion).xy();
    let world_center = transform.apply(local_center);
    let uniform = transform.is_uniform_scale();
    let scale = transform.scale_x().max(transform.scale_y());
    let world_radius = radius * scale;
    if world_radius <= GEOM_TOLERANCE {
        return;
    }
    let bounds = Extents2::from_corners(
        Point2::new(world_center.x - world_radius, world_center.y - world_radius),
        Point2::new(world_center.x + world_radius, world_center.y + world_radius),
    );
    out.push(MeasurePrimitive {
        owner,
        nested: path.to_vec(),
        geom: MeasureGeom::Circle {
            local_center,
            local_radius: radius,
            to_world: transform,
            uniform,
        },
        bounds,
    });
}

fn push_arc(
    out: &mut Vec<MeasurePrimitive>,
    owner: EntityId,
    path: &[EntityId],
    transform: Transform2,
    center: Point3,
    radius: f64,
    start_angle: f64,
    end_angle: f64,
    extrusion: Point3,
) {
    let local_center = ocs_to_wcs(center, extrusion).xy();
    let world_center = transform.apply(local_center);
    let uniform = transform.is_uniform_scale();
    let scale = transform.scale_x().max(transform.scale_y());
    let world_radius = radius * scale;
    if world_radius <= GEOM_TOLERANCE {
        return;
    }
    let bounds = Extents2::from_corners(
        Point2::new(world_center.x - world_radius, world_center.y - world_radius),
        Point2::new(world_center.x + world_radius, world_center.y + world_radius),
    );
    out.push(MeasurePrimitive {
        owner,
        nested: path.to_vec(),
        geom: MeasureGeom::Arc {
            local_center,
            local_radius: radius,
            start_angle,
            end_angle,
            to_world: transform,
            uniform,
        },
        bounds,
    });
}

fn push_polyline(
    out: &mut Vec<MeasurePrimitive>,
    owner: EntityId,
    path: &[EntityId],
    transform: Transform2,
    vertices: &[PolyVertex],
    closed: bool,
    extrusion: Point3,
) {
    if vertices.len() < 2 {
        return;
    }
    let world: Vec<PolyVertex> = vertices
        .iter()
        .map(|vertex| PolyVertex {
            point: {
                let p = transform.apply(ocs_to_wcs(vertex.point, extrusion).xy());
                crate::geom::Point3::from_xy(p.x, p.y)
            },
            bulge: if transform.is_uniform_scale() {
                vertex.bulge * transform.uniform_scale().unwrap_or(1.0).signum()
            } else {
                vertex.bulge
            },
        })
        .collect();
    let n = world.len();
    let segments = if closed { n } else { n - 1 };
    for i in 0..segments {
        let a = world[i];
        let b = world[(i + 1) % n];
        if a.bulge.abs() > GEOM_TOLERANCE && transform.is_uniform_scale() {
            let start = a.point.xy();
            let end = b.point.xy();
            let bounds = bulge_circle(start, end, a.bulge)
                .map(|arc| {
                    Extents2::from_corners(
                        Point2::new(arc.center.x - arc.radius, arc.center.y - arc.radius),
                        Point2::new(arc.center.x + arc.radius, arc.center.y + arc.radius),
                    )
                })
                .unwrap_or_else(|| Extents2::from_corners(start, end));
            out.push(MeasurePrimitive {
                owner,
                nested: path.to_vec(),
                geom: MeasureGeom::Bulge {
                    start,
                    end,
                    bulge: a.bulge,
                },
                bounds,
            });
        } else {
            push_straight(out, owner, path, a.point.xy(), b.point.xy());
        }
    }
    if closed && n >= 2 {
        let mut bounds = Extents2::empty();
        for vertex in &world {
            bounds.include(vertex.point.xy());
        }
        if bounds.is_valid() {
            out.push(MeasurePrimitive {
                owner,
                nested: path.to_vec(),
                geom: MeasureGeom::ClosedLoop {
                    vertices: world,
                    uniform: transform.is_uniform_scale(),
                },
                bounds,
            });
        }
    }
}

pub fn radius_from_primitive(
    primitive: &MeasurePrimitive,
    toward: Point2,
) -> Result<crate::measure::RadiusMeasurement, MeasureError> {
    match &primitive.geom {
        MeasureGeom::Circle {
            local_center,
            local_radius,
            to_world,
            uniform,
        } => {
            if !*uniform {
                return Err(MeasureError::NonUniformScale);
            }
            let scale = to_world.scale_x();
            let center = to_world.apply(*local_center);
            crate::measure::RadiusMeasurement::circle(center, *local_radius * scale, toward)
                .ok_or(MeasureError::InvalidGeometry)
        }
        MeasureGeom::Arc {
            local_center,
            local_radius,
            start_angle,
            end_angle,
            to_world,
            uniform,
        } => {
            if !*uniform {
                return Err(MeasureError::NonUniformScale);
            }
            let scale = to_world.scale_x();
            let center = to_world.apply(*local_center);
            crate::measure::RadiusMeasurement::arc(
                center,
                *local_radius * scale,
                *start_angle,
                *end_angle,
                toward,
            )
            .ok_or(MeasureError::InvalidGeometry)
        }
        _ => Err(MeasureError::Unsupported),
    }
}

pub fn area_from_primitive(
    primitive: &MeasurePrimitive,
) -> Result<crate::measure::AreaMeasurement, MeasureError> {
    match &primitive.geom {
        MeasureGeom::Circle {
            local_center,
            local_radius,
            to_world,
            uniform,
        } => {
            if !*uniform {
                return Err(MeasureError::NonUniformScale);
            }
            let center = to_world.apply(*local_center);
            crate::measure::AreaMeasurement::from_circle(center, *local_radius * to_world.scale_x())
                .ok_or(MeasureError::InvalidGeometry)
        }
        MeasureGeom::ClosedLoop { vertices, uniform } => {
            if !*uniform
                && vertices
                    .iter()
                    .any(|vertex| vertex.bulge.abs() > GEOM_TOLERANCE)
            {
                return Err(MeasureError::NonUniformScale);
            }
            crate::measure::AreaMeasurement::from_polyline(vertices, true)
        }
        _ => Err(MeasureError::Unsupported),
    }
}

pub fn straight_of(primitive: &MeasurePrimitive) -> Option<(Point2, Point2)> {
    match primitive.geom {
        MeasureGeom::Straight { start, end } => Some((start, end)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{default_extrusion, Entity, Geometry};

    fn line_doc() -> Document {
        let mut document = Document::default();
        document.model_space.push(Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(10.0, 0.0),
        }));
        document.assign_missing_ids();
        document
    }

    #[test]
    fn picks_a_line_within_aperture_not_the_whole_document() {
        let document = line_doc();
        let index = MeasureIndex::build(&document);
        let hit = index
            .pick(Point2::new(5.0, 0.02), 0.1, Some(MeasureRole::Straight))
            .expect("hit");
        assert_eq!(hit.role(), MeasureRole::Straight);
        assert!(matches!(hit.geom, MeasureGeom::Straight { .. }));
        assert!(index
            .pick(Point2::new(5.0, 5.0), 0.1, Some(MeasureRole::Straight))
            .is_none());
    }

    #[test]
    fn nested_uniform_scale_multiplies_radius() {
        let mut document = Document::default();
        document.blocks.insert(
            "C".into(),
            crate::document::BlockDefinition {
                name: "C".into(),
                base_pt: Point3::from_xy(0.0, 0.0),
                entities: vec![Entity::new(Geometry::Circle {
                    center: Point3::from_xy(0.0, 0.0),
                    radius: 2.0,
                    extrusion: default_extrusion(),
                })],
            },
        );
        document.model_space.push(Entity::new(Geometry::Insert {
            block_name: "C".into(),
            insertion: Point3::from_xy(10.0, 0.0),
            scale: Point3::new(3.0, 3.0, 1.0),
            rotation: 0.0,
            extrusion: default_extrusion(),
            attribs: Vec::new(),
            column_count: 1,
            row_count: 1,
            column_spacing: 0.0,
            row_spacing: 0.0,
        }));
        document.assign_missing_ids();
        let index = MeasureIndex::build(&document);
        let hit = index
            .pick(Point2::new(16.0, 0.0), 0.5, Some(MeasureRole::Curve))
            .expect("circle");
        let radius = radius_from_primitive(hit, Point2::new(16.0, 0.0)).expect("radius");
        assert!((radius.radius - 6.0).abs() < 1e-9);
    }

    #[test]
    fn mirroring_keeps_a_single_radius() {
        let mut document = Document::default();
        document.blocks.insert(
            "M".into(),
            crate::document::BlockDefinition {
                name: "M".into(),
                base_pt: Point3::from_xy(0.0, 0.0),
                entities: vec![Entity::new(Geometry::Circle {
                    center: Point3::from_xy(0.0, 0.0),
                    radius: 5.0,
                    extrusion: default_extrusion(),
                })],
            },
        );
        document.model_space.push(Entity::new(Geometry::Insert {
            block_name: "M".into(),
            insertion: Point3::from_xy(0.0, 0.0),
            scale: Point3::new(-1.0, 1.0, 1.0),
            rotation: 0.0,
            extrusion: default_extrusion(),
            attribs: Vec::new(),
            column_count: 1,
            row_count: 1,
            column_spacing: 0.0,
            row_spacing: 0.0,
        }));
        document.assign_missing_ids();
        let index = MeasureIndex::build(&document);
        let hit = index
            .pick(Point2::new(5.0, 0.0), 0.5, Some(MeasureRole::Curve))
            .expect("mirrored circle");
        let radius = radius_from_primitive(hit, Point2::new(5.0, 0.0)).expect("radius");
        assert!((radius.radius - 5.0).abs() < 1e-9);
    }

    #[test]
    fn non_uniform_scale_rejects_radius() {
        let mut document = Document::default();
        document.blocks.insert(
            "C".into(),
            crate::document::BlockDefinition {
                name: "C".into(),
                base_pt: Point3::from_xy(0.0, 0.0),
                entities: vec![Entity::new(Geometry::Circle {
                    center: Point3::from_xy(0.0, 0.0),
                    radius: 2.0,
                    extrusion: default_extrusion(),
                })],
            },
        );
        document.model_space.push(Entity::new(Geometry::Insert {
            block_name: "C".into(),
            insertion: Point3::from_xy(0.0, 0.0),
            scale: Point3::new(2.0, 1.0, 1.0),
            rotation: 0.0,
            extrusion: default_extrusion(),
            attribs: Vec::new(),
            column_count: 1,
            row_count: 1,
            column_spacing: 0.0,
            row_spacing: 0.0,
        }));
        document.assign_missing_ids();
        let index = MeasureIndex::build(&document);
        let hit = index
            .pick(Point2::new(4.0, 0.0), 0.6, Some(MeasureRole::Curve))
            .expect("ellipse-like");
        assert_eq!(
            radius_from_primitive(hit, Point2::new(4.0, 0.0)),
            Err(MeasureError::NonUniformScale)
        );
    }
}
