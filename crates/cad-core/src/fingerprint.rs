//! Block geometry fingerprint shared by the DWG companion and dynamic import.
//!
//! The companion uses it to notice when a block was edited outside EntoCAD.
//! Dynamic import uses the same measurement to decide whether an evaluated
//! AutoCAD block still matches the anonymous copy in the file.

use crate::document::BlockDefinition;
use crate::entity::{Entity, Geometry, HatchEdge, HatchPath};

const FINGERPRINT_SCALE: f64 = 10_000.0;

// ------------------------------------------------------------
// Type: BlockFingerprint
// Purpose: Entity count plus a quantized 2D bounding box.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub struct BlockFingerprint {
    pub entities: u64,
    pub min: Option<[f64; 2]>,
    pub max: Option<[f64; 2]>,
}

pub fn fingerprint_block(block: &BlockDefinition) -> BlockFingerprint {
    fingerprint_entities(&block.entities, false)
}

pub fn fingerprint_visible_entities(entities: &[Entity]) -> BlockFingerprint {
    fingerprint_entities(entities, true)
}

fn fingerprint_entities(entities: &[Entity], visible_only: bool) -> BlockFingerprint {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut any = false;
    let mut count = 0u64;
    for entity in entities {
        if visible_only && !entity.visible {
            continue;
        }
        count += 1;
        for (x, y) in entity_points(&entity.geometry) {
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            any = true;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    BlockFingerprint {
        entities: count,
        min: any.then(|| [quantize(min_x), quantize(min_y)]),
        max: any.then(|| [quantize(max_x), quantize(max_y)]),
    }
}

/// Sorted quantized points of visible geometry.
/// `None` when the visible geometry has more than `cap` points.
pub fn visible_geometry_key(entities: &[Entity], cap: usize) -> Option<Vec<(i64, i64)>> {
    let mut points = Vec::new();
    for entity in entities {
        if !entity.visible {
            continue;
        }
        for (x, y) in entity_points(&entity.geometry) {
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            let scaled_x = x * FINGERPRINT_SCALE;
            let scaled_y = y * FINGERPRINT_SCALE;
            if !scaled_x.is_finite() || !scaled_y.is_finite() {
                continue;
            }
            if points.len() >= cap {
                return None;
            }
            points.push((scaled_x.round() as i64, scaled_y.round() as i64));
        }
    }
    points.sort_unstable();
    Some(points)
}

fn quantize(value: f64) -> f64 {
    (value * FINGERPRINT_SCALE).round() / FINGERPRINT_SCALE
}

fn entity_points(geometry: &Geometry) -> Vec<(f64, f64)> {
    let mut points = Vec::new();
    macro_rules! add {
        ($point:expr) => {
            points.push(($point.x, $point.y))
        };
    }
    macro_rules! add_xy {
        ($x:expr, $y:expr $(,)?) => {
            points.push(($x, $y))
        };
    }
    match geometry {
        Geometry::Line { start, end } => {
            add!(*start);
            add!(*end);
        }
        Geometry::Point { position } => add!(*position),
        Geometry::Circle { center, radius, .. } => {
            add!(*center);
            add_xy!(center.x + radius, center.y);
            add_xy!(center.x - radius, center.y);
            add_xy!(center.x, center.y + radius);
            add_xy!(center.x, center.y - radius);
        }
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            ..
        } => {
            add!(*center);
            add_xy!(
                center.x + radius * start_angle.cos(),
                center.y + radius * start_angle.sin(),
            );
            add_xy!(
                center.x + radius * end_angle.cos(),
                center.y + radius * end_angle.sin(),
            );
        }
        Geometry::Ellipse {
            center, major_axis, ..
        } => {
            add!(*center);
            add_xy!(center.x + major_axis.x, center.y + major_axis.y);
        }
        Geometry::LwPolyline { vertices, .. } | Geometry::Polyline { vertices, .. } => {
            for vertex in vertices {
                add!(vertex.point);
            }
        }
        Geometry::Spline {
            control_points,
            fit_points,
            ..
        } => {
            for point in control_points.iter().chain(fit_points.iter()) {
                add!(*point);
            }
        }
        Geometry::Insert {
            insertion,
            scale,
            rotation,
            attribs,
            ..
        } => {
            add!(*insertion);
            add_xy!(
                insertion.x + rotation.cos() * scale.x,
                insertion.y + rotation.sin() * scale.y,
            );
            for attrib in attribs {
                add!(attrib.insertion);
            }
        }
        Geometry::Text(data) => {
            add!(data.insertion);
            add_xy!(
                data.insertion.x + data.height,
                data.insertion.y + data.height
            );
        }
        Geometry::MText(data) => {
            add!(data.insertion);
            add_xy!(
                data.insertion.x + data.width,
                data.insertion.y + data.height
            );
        }
        Geometry::Hatch(hatch) => {
            for path in &hatch.paths {
                match path {
                    HatchPath::Polyline { vertices, .. } => {
                        for vertex in vertices {
                            add!(vertex.point);
                        }
                    }
                    HatchPath::Edges(edges) => {
                        for edge in edges {
                            match edge {
                                HatchEdge::Line { start, end } => {
                                    add!(*start);
                                    add!(*end);
                                }
                                HatchEdge::Arc { center, radius, .. } => {
                                    add!(*center);
                                    add_xy!(center.x + radius, center.y);
                                }
                                HatchEdge::Ellipse {
                                    center,
                                    major_endpoint,
                                    ..
                                } => {
                                    add!(*center);
                                    add!(*major_endpoint);
                                }
                                HatchEdge::Spline {
                                    control_points,
                                    fit_points,
                                    ..
                                } => {
                                    for point in control_points.iter().chain(fit_points.iter()) {
                                        add!(*point);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Geometry::Dimension(_) => {}
        Geometry::Image(frame) | Geometry::Wipeout(frame) => {
            for corner in frame.corners() {
                add!(corner);
            }
        }
        Geometry::Solid { corners, .. } => {
            for corner in corners {
                add!(*corner);
            }
        }
        Geometry::Leader { vertices } | Geometry::MLine { vertices, .. } => {
            for vertex in vertices {
                add!(*vertex);
            }
        }
        Geometry::MultiLeader { lines, content, .. } => {
            for line in lines {
                for point in line.polyline() {
                    add!(point);
                }
            }
            if let Some(location) = content.location() {
                add!(location);
            }
        }
        Geometry::Region { wires } => {
            for wire in wires {
                for point in wire {
                    add!(*point);
                }
            }
        }
        Geometry::Viewport(viewport) => {
            for corner in viewport.corners() {
                add!(corner);
            }
        }
    }
    points
}
