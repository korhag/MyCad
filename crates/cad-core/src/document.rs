//! Native CAD document: layers, blocks, model space, diagnostics.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::color::CadColor;
use crate::entity::{Entity, EntityId, Geometry};
use crate::extents::Extents2;
use crate::geom::{Point2, Point3};
use crate::linetype::{is_byblock_name, is_bylayer_name, normalize_linetype_name, LineType};
use crate::transform::Transform2;

// ------------------------------------------------------------
// Type: Layer
// Purpose: Named drawing layer with visibility and ByLayer style.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Layer {
    pub name: String,
    pub visible: bool,
    pub frozen: bool,
    pub color: CadColor,
    pub linetype: String,
}

impl Layer {
    pub fn is_plottable(&self) -> bool {
        self.visible && !self.frozen
    }
}

#[derive(Debug, Clone)]
pub struct BlockDefinition {
    pub name: String,
    pub base_pt: Point3,
    pub entities: Vec<Entity>,
}

// ------------------------------------------------------------
// Type: ImportDiagnostics
// Purpose: Milestone-required import/render accounting.
// ------------------------------------------------------------
#[derive(Debug, Clone, Default)]
pub struct ImportDiagnostics {
    pub dwg_version: String,
    pub entity_counts: BTreeMap<String, u64>,
    pub unsupported_counts: BTreeMap<String, u64>,
    pub layer_count: usize,
    pub block_count: usize,
    pub extents: Option<Extents2>,
    pub import_time: Duration,
    pub render_prepare_time: Duration,
    pub warnings: Vec<String>,
    pub object_count: u64,
}

impl ImportDiagnostics {
    pub fn bump_entity(&mut self, type_name: &str) {
        *self.entity_counts.entry(type_name.to_string()).or_insert(0) += 1;
    }

    pub fn bump_unsupported(&mut self, type_name: &str) {
        *self
            .unsupported_counts
            .entry(type_name.to_string())
            .or_insert(0) += 1;
    }

    pub fn unsupported_total(&self) -> u64 {
        self.unsupported_counts.values().copied().sum()
    }

    pub fn entity_total(&self) -> u64 {
        self.entity_counts.values().copied().sum()
    }
}

// ------------------------------------------------------------
// Enum: DrawingUnits
// Purpose: AutoCAD $INSUNITS codes used when reporting measurements.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawingUnits {
    Unspecified,
    Inches,
    Feet,
    Miles,
    Millimeters,
    Centimeters,
    Meters,
    Kilometers,
    Microinches,
    Mils,
    Yards,
    Angstroms,
    Nanometers,
    Microns,
    Decimeters,
    Decameters,
    Hectometers,
    Gigameters,
    AstronomicalUnits,
    LightYears,
    Parsecs,
    Other(u16),
}

impl Default for DrawingUnits {
    fn default() -> Self {
        Self::Unspecified
    }
}

impl DrawingUnits {
    pub fn from_insunits(code: u16) -> Self {
        match code {
            0 => Self::Unspecified,
            1 => Self::Inches,
            2 => Self::Feet,
            3 => Self::Miles,
            4 => Self::Millimeters,
            5 => Self::Centimeters,
            6 => Self::Meters,
            7 => Self::Kilometers,
            8 => Self::Microinches,
            9 => Self::Mils,
            10 => Self::Yards,
            11 => Self::Angstroms,
            12 => Self::Nanometers,
            13 => Self::Microns,
            14 => Self::Decimeters,
            15 => Self::Decameters,
            16 => Self::Hectometers,
            17 => Self::Gigameters,
            18 => Self::AstronomicalUnits,
            19 => Self::LightYears,
            20 => Self::Parsecs,
            other => Self::Other(other),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unspecified | Self::Other(_) => "drawing units",
            Self::Inches => "in",
            Self::Feet => "ft",
            Self::Miles => "mi",
            Self::Millimeters => "mm",
            Self::Centimeters => "cm",
            Self::Meters => "m",
            Self::Kilometers => "km",
            Self::Microinches => "µin",
            Self::Mils => "mil",
            Self::Yards => "yd",
            Self::Angstroms => "Å",
            Self::Nanometers => "nm",
            Self::Microns => "µm",
            Self::Decimeters => "dm",
            Self::Decameters => "dam",
            Self::Hectometers => "hm",
            Self::Gigameters => "Gm",
            Self::AstronomicalUnits => "au",
            Self::LightYears => "ly",
            Self::Parsecs => "pc",
        }
    }
}

// ------------------------------------------------------------
// Type: Document
// Purpose: Application CAD model. Future DXF import and editing
//          talk to this type, never to LibreDWG structures.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Document {
    pub source_path: Option<PathBuf>,
    pub layers: BTreeMap<String, Layer>,
    pub linetypes: BTreeMap<String, LineType>,
    pub blocks: BTreeMap<String, BlockDefinition>,
    pub model_space: Vec<Entity>,
    pub diagnostics: ImportDiagnostics,
    pub ltscale: f64,
    pub current_layer: String,
    pub units: DrawingUnits,
    next_entity_id: u64,
}

impl Default for Document {
    fn default() -> Self {
        let mut document = Self {
            source_path: None,
            layers: BTreeMap::new(),
            linetypes: BTreeMap::new(),
            blocks: BTreeMap::new(),
            model_space: Vec::new(),
            diagnostics: ImportDiagnostics::default(),
            ltscale: 1.0,
            current_layer: "0".into(),
            units: DrawingUnits::Unspecified,
            next_entity_id: 1,
        };
        document.ensure_layer_zero();
        document
    }
}

impl Document {
    pub fn file_name(&self) -> String {
        self.source_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "(untitled)".to_string())
    }

    pub fn ensure_layer_zero(&mut self) {
        self.layers.entry("0".into()).or_insert_with(|| Layer {
            name: "0".into(),
            visible: true,
            frozen: false,
            color: CadColor::Aci(7),
            linetype: "CONTINUOUS".into(),
        });
    }

    pub fn allocate_id(&mut self) -> EntityId {
        let id = EntityId(self.next_entity_id);
        self.next_entity_id = self.next_entity_id.saturating_add(1).max(1);
        id
    }

    pub fn assign_missing_ids(&mut self) {
        let mut next = self.next_entity_id.max(1);
        for entity in self.model_space.iter_mut().chain(
            self.blocks
                .values_mut()
                .flat_map(|block| block.entities.iter_mut()),
        ) {
            if entity.id.is_assigned() {
                next = next.max(entity.id.raw() + 1);
            } else {
                entity.id = EntityId(next);
                next += 1;
            }
        }
        self.next_entity_id = next;
    }

    pub fn new_entity(&self, geometry: Geometry) -> Entity {
        let mut entity = Entity::new(geometry);
        entity.layer = self.current_layer.clone();
        entity.color = CadColor::ByLayer;
        entity.linetype = "BYLAYER".into();
        entity
    }

    pub fn insert_model_entity(&mut self, index: usize, mut entity: Entity) -> Entity {
        if entity.id.is_assigned() {
            self.next_entity_id = self.next_entity_id.max(entity.id.raw() + 1);
        } else {
            entity.id = self.allocate_id();
        }
        let index = index.min(self.model_space.len());
        self.model_space.insert(index, entity.clone());
        entity
    }

    pub fn add_entity(&mut self, entity: Entity) -> Entity {
        let index = self.model_space.len();
        self.insert_model_entity(index, entity)
    }

    pub fn remove_model_entity(&mut self, id: EntityId) -> Option<(usize, Entity)> {
        let index = self.model_space.iter().position(|entity| entity.id == id)?;
        let entity = self.model_space.remove(index);
        Some((index, entity))
    }

    pub fn replace_model_entity(&mut self, id: EntityId, mut entity: Entity) -> Option<Entity> {
        let index = self
            .model_space
            .iter()
            .position(|existing| existing.id == id)?;
        if !entity.id.is_assigned() {
            entity.id = id;
        }
        Some(std::mem::replace(&mut self.model_space[index], entity))
    }

    pub fn entity_by_id(&self, id: EntityId) -> Option<&Entity> {
        self.model_space.iter().find(|entity| entity.id == id)
    }

    pub fn entity_index(&self, id: EntityId) -> Option<usize> {
        self.model_space.iter().position(|entity| entity.id == id)
    }

    pub fn layer_can_be_current(&self, name: &str) -> bool {
        self.layers.get(name).is_some_and(|layer| !layer.frozen)
    }

    pub fn set_current_layer(&mut self, name: &str) -> bool {
        if self.layer_can_be_current(name) {
            self.current_layer = name.to_string();
            true
        } else {
            false
        }
    }

    pub fn apply_current_layer(&mut self, requested: Option<&str>) {
        self.ensure_layer_zero();
        if let Some(name) = requested.filter(|name| !name.is_empty()) {
            if self.set_current_layer(name) {
                return;
            }
        }
        if self.set_current_layer("0") {
            return;
        }
        if let Some(name) = self
            .layers
            .values()
            .find(|layer| !layer.frozen)
            .map(|layer| layer.name.clone())
        {
            self.current_layer = name;
            return;
        }
        if let Some(layer) = self.layers.get_mut("0") {
            layer.frozen = false;
        }
        self.current_layer = "0".into();
    }

    pub fn layer(&self, name: &str) -> Option<&Layer> {
        self.layers.get(name).or_else(|| self.layers.get("0"))
    }

    pub fn layer_is_visible(&self, name: &str) -> bool {
        self.layer(name).map(|l| l.is_plottable()).unwrap_or(true)
    }

    pub fn linetype(&self, name: &str) -> Option<&LineType> {
        let key = normalize_linetype_name(name);
        self.linetypes.get(&key).or_else(|| {
            self.linetypes
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(&key))
                .map(|(_, lt)| lt)
        })
    }

    pub fn resolved_linetype_name(&self, entity: &Entity, block_linetype: &str) -> String {
        let name = normalize_linetype_name(&entity.linetype);
        if is_bylayer_name(&name) {
            let layer_lt = self
                .layer(&entity.layer)
                .map(|l| normalize_linetype_name(&l.linetype))
                .unwrap_or_else(|| "CONTINUOUS".into());
            if is_bylayer_name(&layer_lt) {
                "CONTINUOUS".into()
            } else if is_byblock_name(&layer_lt) {
                resolved_byblock(block_linetype)
            } else {
                layer_lt
            }
        } else if is_byblock_name(&name) {
            resolved_byblock(block_linetype)
        } else {
            name
        }
    }

    pub fn effective_linetype_scale(&self, entity: &Entity) -> f64 {
        (self.ltscale * entity.linetype_scale).max(1e-6)
    }

    pub fn compute_extents(&self) -> Option<Extents2> {
        let mut extents = Extents2::empty();
        let mut any = false;
        let mut stack = Vec::new();
        for entity in &self.model_space {
            collect_entity_points(self, entity, Transform2::identity(), &mut stack, &mut |p| {
                if p.is_finite() {
                    extents.include(p);
                    any = true;
                }
            });
        }
        any.then_some(extents)
    }
}

fn resolved_byblock(block_linetype: &str) -> String {
    let inherited = normalize_linetype_name(block_linetype);
    if inherited.is_empty() || is_byblock_name(&inherited) || is_bylayer_name(&inherited) {
        "CONTINUOUS".into()
    } else {
        inherited
    }
}

fn collect_entity_points(
    document: &Document,
    entity: &Entity,
    transform: Transform2,
    block_stack: &mut Vec<String>,
    visit: &mut impl FnMut(Point2),
) {
    if !entity.visible || !document.layer_is_visible(&entity.layer) {
        return;
    }
    match &entity.geometry {
        crate::entity::Geometry::Insert {
            block_name,
            insertion,
            scale,
            rotation,
            extrusion,
            column_count,
            row_count,
            column_spacing,
            row_spacing,
            attribs,
        } => {
            if block_stack
                .iter()
                .any(|n| n.eq_ignore_ascii_case(block_name))
            {
                return;
            }
            let Some(block) = document.blocks.get(block_name) else {
                return;
            };
            block_stack.push(block_name.clone());
            let cols = (*column_count).max(1);
            let rows = (*row_count).max(1);
            for col in 0..cols {
                for row in 0..rows {
                    let extra = Transform2::translate(
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
                    .then(extra);
                    let nested = transform.then(local);
                    for child in &block.entities {
                        collect_entity_points(document, child, nested, block_stack, visit);
                    }
                }
            }
            for attrib in attribs {
                visit(transform.apply(attrib.insertion.xy()));
            }
            block_stack.pop();
        }
        crate::entity::Geometry::Dimension { block_name } => {
            if let Some(block) = document.blocks.get(block_name) {
                if block_stack
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(block_name))
                {
                    return;
                }
                block_stack.push(block_name.clone());
                for child in &block.entities {
                    collect_entity_points(document, child, transform, block_stack, visit);
                }
                block_stack.pop();
            }
        }
        crate::entity::Geometry::Line { start, end } => {
            visit(transform.apply(start.xy()));
            visit(transform.apply(end.xy()));
        }
        crate::entity::Geometry::Point { position } => visit(transform.apply(position.xy())),
        crate::entity::Geometry::Circle { center, radius, .. } => {
            let c = transform.apply(center.xy());
            let r = *radius * transform.scale_x().abs().max(transform.scale_y().abs());
            visit(Point2::new(c.x - r, c.y - r));
            visit(Point2::new(c.x + r, c.y + r));
        }
        crate::entity::Geometry::Arc { center, radius, .. } => {
            let c = transform.apply(center.xy());
            let r = *radius * transform.scale_x().abs().max(transform.scale_y().abs());
            visit(Point2::new(c.x - r, c.y - r));
            visit(Point2::new(c.x + r, c.y + r));
        }
        crate::entity::Geometry::Ellipse {
            center, major_axis, ..
        } => {
            visit(transform.apply(center.xy()));
            visit(transform.apply((*center + *major_axis).xy()));
            visit(transform.apply((*center - *major_axis).xy()));
        }
        crate::entity::Geometry::LwPolyline { vertices, .. }
        | crate::entity::Geometry::Polyline { vertices, .. } => {
            for v in vertices {
                visit(transform.apply(v.point.xy()));
            }
        }
        crate::entity::Geometry::Spline {
            control_points,
            fit_points,
            ..
        } => {
            for p in control_points.iter().chain(fit_points.iter()) {
                visit(transform.apply(p.xy()));
            }
        }
        crate::entity::Geometry::Text(_) | crate::entity::Geometry::MText(_) => {}
        crate::entity::Geometry::Solid { corners, .. } => {
            for c in corners {
                visit(transform.apply(c.xy()));
            }
        }
        crate::entity::Geometry::Leader { vertices }
        | crate::entity::Geometry::MLine { vertices, .. } => {
            for p in vertices {
                visit(transform.apply(p.xy()));
            }
        }
        crate::entity::Geometry::Hatch(hatch) => {
            for path in &hatch.paths {
                match path {
                    crate::entity::HatchPath::Polyline { vertices, .. } => {
                        for v in vertices {
                            visit(transform.apply(v.point.xy()));
                        }
                    }
                    crate::entity::HatchPath::Edges(edges) => {
                        for edge in edges {
                            match edge {
                                crate::entity::HatchEdge::Line { start, end } => {
                                    visit(transform.apply(start.xy()));
                                    visit(transform.apply(end.xy()));
                                }
                                crate::entity::HatchEdge::Arc { center, radius, .. } => {
                                    let c = transform.apply(center.xy());
                                    let r = *radius;
                                    visit(Point2::new(c.x - r, c.y - r));
                                    visit(Point2::new(c.x + r, c.y + r));
                                }
                                crate::entity::HatchEdge::Ellipse {
                                    center,
                                    major_endpoint,
                                    ..
                                } => {
                                    visit(transform.apply(center.xy()));
                                    visit(transform.apply(major_endpoint.xy()));
                                }
                                crate::entity::HatchEdge::Spline { control_points } => {
                                    for p in control_points {
                                        visit(transform.apply(p.xy()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{Geometry, PolyVertex};
    use crate::geom::Point3;

    fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> Entity {
        Entity::new(Geometry::Line {
            start: Point3::from_xy(x0, y0),
            end: Point3::from_xy(x1, y1),
        })
    }

    #[test]
    fn extents_include_nested_insert() {
        let mut document = Document::default();
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
        document.blocks.insert(
            "SYM".into(),
            BlockDefinition {
                name: "SYM".into(),
                base_pt: Point3::from_xy(0.0, 0.0),
                entities: vec![line(0.0, 0.0, 10.0, 0.0)],
            },
        );
        document.model_space.push(Entity::new(Geometry::Insert {
            block_name: "SYM".into(),
            insertion: Point3::from_xy(100.0, 50.0),
            scale: Point3::new(2.0, 2.0, 1.0),
            rotation: 0.0,
            extrusion: Point3::new(0.0, 0.0, 1.0),
            attribs: Vec::new(),
            column_count: 1,
            row_count: 1,
            column_spacing: 0.0,
            row_spacing: 0.0,
        }));
        let e = document.compute_extents().unwrap();
        assert!((e.min.x - 100.0).abs() < 1e-9);
        assert!((e.max.x - 120.0).abs() < 1e-9);
        assert!((e.min.y - 50.0).abs() < 1e-9);
    }

    #[test]
    fn extents_subtract_nested_block_base_point() {
        let mut document = Document::default();
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
        document.blocks.insert(
            "SYM".into(),
            BlockDefinition {
                name: "SYM".into(),
                base_pt: Point3::from_xy(50.0, 20.0),
                entities: vec![line(50.0, 20.0, 60.0, 20.0)],
            },
        );
        document.model_space.push(Entity::new(Geometry::Insert {
            block_name: "SYM".into(),
            insertion: Point3::from_xy(100.0, 50.0),
            scale: Point3::new(1.0, 1.0, 1.0),
            rotation: 0.0,
            extrusion: Point3::new(0.0, 0.0, 1.0),
            attribs: Vec::new(),
            column_count: 1,
            row_count: 1,
            column_spacing: 0.0,
            row_spacing: 0.0,
        }));
        let e = document.compute_extents().unwrap();
        assert!((e.min.x - 100.0).abs() < 1e-9);
        assert!((e.max.x - 110.0).abs() < 1e-9);
        assert!((e.min.y - 50.0).abs() < 1e-9);
        assert!((e.max.y - 50.0).abs() < 1e-9);
    }

    #[test]
    fn hidden_layer_is_excluded_from_extents() {
        let mut document = Document::default();
        document.layers.insert(
            "OFF".into(),
            Layer {
                name: "OFF".into(),
                visible: false,
                frozen: false,
                color: CadColor::Aci(7),
                linetype: "CONTINUOUS".into(),
            },
        );
        let mut entity = line(0.0, 0.0, 50.0, 50.0);
        entity.layer = "OFF".into();
        document.model_space.push(entity);
        assert!(document.compute_extents().is_none());
    }

    #[test]
    fn polyline_vertices_contribute_to_extents() {
        let mut document = Document::default();
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
        document.model_space.push(Entity::new(Geometry::LwPolyline {
            vertices: vec![
                PolyVertex {
                    point: Point3::from_xy(0.0, 0.0),
                    bulge: 0.0,
                },
                PolyVertex {
                    point: Point3::from_xy(8.0, 2.0),
                    bulge: 0.0,
                },
            ],
            closed: false,
            extrusion: Point3::new(0.0, 0.0, 1.0),
            linetype_generation_continuous: false,
        }));
        let e = document.compute_extents().unwrap();
        assert_eq!(e.min, Point2::new(0.0, 0.0));
        assert_eq!(e.max, Point2::new(8.0, 2.0));
    }

    #[test]
    fn far_text_does_not_expand_extents() {
        let mut document = Document::default();
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
        document.model_space.push(line(0.0, 0.0, 10.0, 0.0));
        document
            .model_space
            .push(Entity::new(Geometry::Text(crate::entity::TextData {
                insertion: Point3::from_xy(8.0e7, 1.0e6),
                height: 2.5,
                rotation: 0.0,
                value: "stray".into(),
                extrusion: Point3::new(0.0, 0.0, 1.0),
                is_attrib_def: false,
            })));
        let e = document.compute_extents().unwrap();
        assert!((e.min.x - 0.0).abs() < 1e-9);
        assert!((e.max.x - 10.0).abs() < 1e-9);
    }

    #[test]
    fn added_entities_receive_stable_unique_ids() {
        let mut document = Document::default();
        let a = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        let b = document.add_entity(line(1.0, 0.0, 2.0, 0.0));
        assert!(a.id.is_assigned());
        assert!(b.id.is_assigned());
        assert_ne!(a.id, b.id);
        document.remove_model_entity(a.id);
        assert_eq!(
            document.entity_by_id(b.id).map(|entity| entity.id),
            Some(b.id)
        );
        let restored = document.insert_model_entity(0, a.clone());
        assert_eq!(restored.id, a.id);
        assert_eq!(document.model_space[0].id, a.id);
        assert_eq!(document.model_space[1].id, b.id);
    }

    #[test]
    fn frozen_layer_cannot_become_current() {
        let mut document = Document::default();
        document.layers.insert(
            "FROZEN".into(),
            Layer {
                name: "FROZEN".into(),
                visible: true,
                frozen: true,
                color: CadColor::Aci(1),
                linetype: "CONTINUOUS".into(),
            },
        );
        assert!(!document.set_current_layer("FROZEN"));
        assert_eq!(document.current_layer, "0");
        document.apply_current_layer(Some("FROZEN"));
        assert_eq!(document.current_layer, "0");
        assert!(document.set_current_layer("0"));
    }

    #[test]
    fn new_entity_inherits_current_layer() {
        let mut document = Document::default();
        document.layers.insert(
            "WALL".into(),
            Layer {
                name: "WALL".into(),
                visible: true,
                frozen: false,
                color: CadColor::Aci(1),
                linetype: "CONTINUOUS".into(),
            },
        );
        assert!(document.set_current_layer("WALL"));
        let entity = document.new_entity(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(1.0, 0.0),
        });
        assert_eq!(entity.layer, "WALL");
        assert_eq!(entity.color, CadColor::ByLayer);
        assert_eq!(entity.linetype, "BYLAYER");
    }

    #[test]
    fn insunits_maps_to_drawing_units() {
        assert_eq!(DrawingUnits::from_insunits(0).label(), "drawing units");
        assert_eq!(DrawingUnits::from_insunits(4).label(), "mm");
        assert_eq!(DrawingUnits::from_insunits(1).label(), "in");
        assert_eq!(DrawingUnits::from_insunits(99).label(), "drawing units");
    }
}
