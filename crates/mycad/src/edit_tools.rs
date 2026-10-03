//! Trim, extend, offset, and stretch against the open document.
//!
//! Geometry math stays in `cad_core::curve_edit`. This module only
//! chooses a bounded set of nearby edges and maps the result back
//! into the active block's local space.

use cad_core::{
    stretch_geometry, transform_geometry, trim_geometry, Document, EditError, Entity, EntityId,
    EntitySpace, Extents2, Geometry, MeasureIndex, Point2, StretchOutcome, Transform2, TrimResult,
    MAX_EXTEND_EDGES, MAX_STRETCH_PREVIEW, MAX_TRIM_EDGES,
};

const EXTEND_REACH: f64 = 8.0;

pub fn trim_at(
    document: &Document,
    measures: &MeasureIndex,
    active: &EntitySpace,
    to_world: Transform2,
    pick: Point2,
    aperture: f64,
) -> Result<(Entity, TrimResult), EditError> {
    let entity = picked_entity(document, measures, active, pick, aperture)?;
    let region =
        owner_bounds(measures, entity.id).unwrap_or_else(|| box_around(pick, aperture.max(1.0)));
    let edges = edge_geometries(
        document,
        measures,
        region,
        entity.id,
        active,
        to_world,
        MAX_TRIM_EDGES,
    );
    let world =
        transform_geometry(&entity.geometry, to_world).map_err(|_| EditError::Unsupported)?;
    let trimmed = trim_geometry(&world, pick, &edges)?;
    Ok((entity, trimmed))
}

pub fn extend_at(
    document: &Document,
    measures: &MeasureIndex,
    active: &EntitySpace,
    to_world: Transform2,
    pick: Point2,
    aperture: f64,
) -> Result<(Entity, Geometry), EditError> {
    let entity = picked_entity(document, measures, active, pick, aperture)?;
    let region = owner_bounds(measures, entity.id)
        .map(|bounds| grow_bounds(bounds, EXTEND_REACH))
        .unwrap_or_else(|| box_around(pick, aperture.max(1.0) * EXTEND_REACH));
    let edges = edge_geometries(
        document,
        measures,
        region,
        entity.id,
        active,
        to_world,
        MAX_EXTEND_EDGES,
    );
    let world =
        transform_geometry(&entity.geometry, to_world).map_err(|_| EditError::Unsupported)?;
    let extended = cad_core::extend_geometry(&world, pick, &edges)?;
    Ok((entity, extended))
}

pub fn offset_copy(
    document: &Document,
    active: &EntitySpace,
    to_world: Transform2,
    id: EntityId,
    distance: f64,
    side: Point2,
) -> Result<(Entity, Geometry), EditError> {
    let Some((space, _)) = document.find_entity_location(id) else {
        return Err(EditError::Unsupported);
    };
    if space != *active {
        return Err(EditError::Unsupported);
    }
    let Some(entity) = document.entity_by_id(id).cloned() else {
        return Err(EditError::Unsupported);
    };
    let world =
        transform_geometry(&entity.geometry, to_world).map_err(|_| EditError::Unsupported)?;
    let offset = cad_core::offset_geometry(&world, distance, side)?;
    Ok((entity, offset))
}

pub struct StretchHit {
    pub before: Entity,
    pub after_world: Geometry,
}

pub fn stretch_hits(
    document: &Document,
    measures: &MeasureIndex,
    active: &EntitySpace,
    to_world: Transform2,
    window: Extents2,
    dx: f64,
    dy: f64,
) -> Result<Vec<StretchHit>, EditError> {
    let mut primitives = Vec::new();
    measures.query(window, &mut primitives);
    let mut seen = Vec::new();
    let mut hits = Vec::new();
    for primitive in primitives {
        let id = primitive.owner;
        if seen.contains(&id) {
            continue;
        }
        seen.push(id);
        if hits.len() >= MAX_STRETCH_PREVIEW {
            break;
        }
        let Some((space, _)) = document.find_entity_location(id) else {
            continue;
        };
        if space != *active {
            continue;
        }
        let Some(entity) = document.entity_by_id(id).cloned() else {
            continue;
        };
        let Ok(world) = transform_geometry(&entity.geometry, to_world) else {
            continue;
        };
        match stretch_geometry(&world, window, dx, dy) {
            Ok(StretchOutcome::Unchanged) => {}
            Ok(StretchOutcome::Replaced(after_world)) => hits.push(StretchHit {
                before: entity,
                after_world,
            }),
            Err(EditError::NoOp) => return Err(EditError::NoOp),
            Err(EditError::Unsupported) => {}
            Err(err) => return Err(err),
        }
    }
    Ok(hits)
}

pub fn preview_segments(geometry: &Geometry, cap: usize, out: &mut Vec<[Point2; 2]>) {
    if out.len() >= cap {
        return;
    }
    match geometry {
        Geometry::Line { start, end } => out.push([start.xy(), end.xy()]),
        Geometry::LwPolyline {
            vertices, closed, ..
        }
        | Geometry::Polyline {
            vertices, closed, ..
        } => {
            if vertices.len() < 2 {
                return;
            }
            let count = if *closed {
                vertices.len()
            } else {
                vertices.len() - 1
            };
            for index in 0..count {
                if out.len() >= cap {
                    break;
                }
                let next = (index + 1) % vertices.len();
                out.push([vertices[index].point.xy(), vertices[next].point.xy()]);
            }
        }
        _ => {}
    }
}

fn picked_entity(
    document: &Document,
    measures: &MeasureIndex,
    active: &EntitySpace,
    pick: Point2,
    aperture: f64,
) -> Result<Entity, EditError> {
    let Some(hit) = measures.pick(pick, aperture, None) else {
        return Err(EditError::NoIntersection);
    };
    let id = hit.owner;
    let Some((space, _)) = document.find_entity_location(id) else {
        return Err(EditError::Unsupported);
    };
    if space != *active {
        return Err(EditError::Unsupported);
    }
    document
        .entity_by_id(id)
        .cloned()
        .ok_or(EditError::Unsupported)
}

fn edge_geometries(
    document: &Document,
    measures: &MeasureIndex,
    region: Extents2,
    skip: EntityId,
    active: &EntitySpace,
    to_world: Transform2,
    cap: usize,
) -> Vec<Geometry> {
    let mut primitives = Vec::new();
    measures.query(region, &mut primitives);
    let mut seen = Vec::new();
    let mut edges = Vec::new();
    for primitive in primitives {
        let id = primitive.owner;
        if id == skip || seen.contains(&id) {
            continue;
        }
        seen.push(id);
        if edges.len() >= cap {
            break;
        }
        let Some((space, _)) = document.find_entity_location(id) else {
            continue;
        };
        if space != *active {
            continue;
        }
        let Some(entity) = document.entity_by_id(id) else {
            continue;
        };
        if let Ok(geometry) = transform_geometry(&entity.geometry, to_world) {
            edges.push(geometry);
        }
    }
    edges
}

fn owner_bounds(measures: &MeasureIndex, id: EntityId) -> Option<Extents2> {
    let mut bounds: Option<Extents2> = None;
    for primitive in measures.primitives_for_owner(id) {
        match &mut bounds {
            Some(current) => current.union(primitive.bounds),
            None => bounds = Some(primitive.bounds),
        }
    }
    bounds
}

fn grow_bounds(bounds: Extents2, scale: f64) -> Extents2 {
    let center_x = (bounds.min.x + bounds.max.x) * 0.5;
    let center_y = (bounds.min.y + bounds.max.y) * 0.5;
    let half_x = ((bounds.max.x - bounds.min.x) * 0.5).max(1.0) * scale;
    let half_y = ((bounds.max.y - bounds.min.y) * 0.5).max(1.0) * scale;
    Extents2::from_corners(
        Point2::new(center_x - half_x, center_y - half_y),
        Point2::new(center_x + half_x, center_y + half_y),
    )
}

fn box_around(point: Point2, radius: f64) -> Extents2 {
    Extents2::from_corners(
        Point2::new(point.x - radius, point.y - radius),
        Point2::new(point.x + radius, point.y + radius),
    )
}
