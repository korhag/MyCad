//! Native CAD document: layers, blocks, model space, diagnostics.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Duration;

use crate::color::CadColor;
use crate::dynamic::DynamicDefinition;
use crate::entity::{Entity, EntityId, Geometry};
use crate::extents::Extents2;
use crate::geom::{Point2, Point3};
use crate::ids::{ActionId, BlockDefinitionId, OptionId, ParameterId, VertexId};
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

#[derive(Debug, Clone, PartialEq)]
pub struct BlockDefinition {
    pub id: BlockDefinitionId,
    pub name: String,
    pub base_pt: Point3,
    pub entities: Vec<Entity>,
    pub dynamic: Option<DynamicDefinition>,
    pub content_revision: u64,
}

impl Default for BlockDefinition {
    fn default() -> Self {
        Self {
            id: BlockDefinitionId::UNASSIGNED,
            name: String::new(),
            base_pt: Point3::default(),
            entities: Vec::new(),
            dynamic: None,
            content_revision: 0,
        }
    }
}

impl BlockDefinition {
    pub fn plain(name: impl Into<String>, base_pt: Point3, entities: Vec<Entity>) -> Self {
        Self {
            name: name.into(),
            base_pt,
            entities,
            ..Self::default()
        }
    }

    pub fn is_dynamic(&self) -> bool {
        self.dynamic
            .as_ref()
            .is_some_and(|dynamic| !dynamic.is_empty())
    }
}

// ------------------------------------------------------------
// Enum: EntitySpace
// Purpose: Editable container for entities: model space or a named block.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EntitySpace {
    ModelSpace,
    Block(String),
}

impl EntitySpace {
    pub fn is_model(&self) -> bool {
        matches!(self, Self::ModelSpace)
    }

    pub fn block_name(&self) -> Option<&str> {
        match self {
            Self::ModelSpace => None,
            Self::Block(name) => Some(name.as_str()),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::ModelSpace => "Model".into(),
            Self::Block(name) => name.clone(),
        }
    }

    pub fn same_block(&self, name: &str) -> bool {
        match self {
            Self::ModelSpace => false,
            Self::Block(existing) => existing.eq_ignore_ascii_case(name),
        }
    }

    pub fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::ModelSpace, Self::ModelSpace) => true,
            (Self::Block(left), Self::Block(right)) => left.eq_ignore_ascii_case(right),
            _ => false,
        }
    }
}

// ------------------------------------------------------------
// Type: EntityLocation
// Purpose: O(1) lookup of an entity's container and vector index.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityLocation {
    pub space: EntitySpace,
    pub index: usize,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

    pub fn to_insunits(self) -> u16 {
        match self {
            Self::Unspecified => 0,
            Self::Inches => 1,
            Self::Feet => 2,
            Self::Miles => 3,
            Self::Millimeters => 4,
            Self::Centimeters => 5,
            Self::Meters => 6,
            Self::Kilometers => 7,
            Self::Microinches => 8,
            Self::Mils => 9,
            Self::Yards => 10,
            Self::Angstroms => 11,
            Self::Nanometers => 12,
            Self::Microns => 13,
            Self::Decimeters => 14,
            Self::Decameters => 15,
            Self::Hectometers => 16,
            Self::Gigameters => 17,
            Self::AstronomicalUnits => 18,
            Self::LightYears => 19,
            Self::Parsecs => 20,
            Self::Other(code) => code,
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

    pub fn area_label(self) -> String {
        match self {
            Self::Unspecified | Self::Other(_) => "drawing units²".into(),
            _ => format!("{}²", self.label()),
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
    next_definition_id: u64,
    next_parameter_id: u64,
    next_option_id: u64,
    next_action_id: u64,
    next_vertex_id: u64,
    content_generation: u64,
    saved_revision: u64,
    entity_locations: HashMap<EntityId, EntityLocation>,
    extents_stale: bool,
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
            next_definition_id: 1,
            next_parameter_id: 1,
            next_option_id: 1,
            next_action_id: 1,
            next_vertex_id: 1,
            content_generation: 1,
            saved_revision: 0,
            entity_locations: HashMap::new(),
            extents_stale: false,
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

    pub fn allocate_definition_id(&mut self) -> BlockDefinitionId {
        let id = BlockDefinitionId(self.next_definition_id);
        self.next_definition_id = self.next_definition_id.saturating_add(1).max(1);
        id
    }

    pub fn allocate_parameter_id(&mut self) -> ParameterId {
        let id = ParameterId(self.next_parameter_id);
        self.next_parameter_id = self.next_parameter_id.saturating_add(1).max(1);
        id
    }

    pub fn allocate_option_id(&mut self) -> OptionId {
        let id = OptionId(self.next_option_id);
        self.next_option_id = self.next_option_id.saturating_add(1).max(1);
        id
    }

    pub fn allocate_action_id(&mut self) -> ActionId {
        let id = ActionId(self.next_action_id);
        self.next_action_id = self.next_action_id.saturating_add(1).max(1);
        id
    }

    pub fn allocate_vertex_id(&mut self) -> VertexId {
        let id = VertexId(self.next_vertex_id);
        self.next_vertex_id = self.next_vertex_id.saturating_add(1).max(1);
        id
    }

    pub fn next_vertex_id(&self) -> u64 {
        self.next_vertex_id
    }

    pub fn set_next_vertex_id(&mut self, next: u64) {
        self.next_vertex_id = next.max(1);
    }

    pub fn content_generation(&self) -> u64 {
        self.content_generation
    }

    pub fn saved_revision(&self) -> u64 {
        self.saved_revision
    }

    pub fn bump_generation(&mut self) {
        self.content_generation = self.content_generation.saturating_add(1).max(1);
    }

    pub fn mark_saved_revision(&mut self) {
        self.saved_revision = self.saved_revision.saturating_add(1);
    }

    pub fn set_identity_counters(
        &mut self,
        next_entity_id: u64,
        next_definition_id: u64,
        next_parameter_id: u64,
        next_option_id: u64,
        next_action_id: u64,
        content_generation: u64,
        saved_revision: u64,
    ) {
        self.next_entity_id = next_entity_id.max(1);
        self.next_definition_id = next_definition_id.max(1);
        self.next_parameter_id = next_parameter_id.max(1);
        self.next_option_id = next_option_id.max(1);
        self.next_action_id = next_action_id.max(1);
        self.content_generation = content_generation.max(1);
        self.saved_revision = saved_revision;
    }

    pub fn identity_counters(&self) -> (u64, u64, u64, u64, u64, u64, u64) {
        (
            self.next_entity_id,
            self.next_definition_id,
            self.next_parameter_id,
            self.next_option_id,
            self.next_action_id,
            self.content_generation,
            self.saved_revision,
        )
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
        self.assign_missing_definition_ids();
        self.assign_missing_vertex_ids();
        self.rebuild_entity_index();
    }

    fn assign_missing_definition_ids(&mut self) {
        let mut next_def = self.next_definition_id.max(1);
        let mut next_param = self.next_parameter_id.max(1);
        let mut next_option = self.next_option_id.max(1);
        let mut next_action = self.next_action_id.max(1);
        for block in self.blocks.values() {
            if block.id.is_assigned() {
                next_def = next_def.max(block.id.raw() + 1);
            }
            if let Some(dynamic) = &block.dynamic {
                for parameter in &dynamic.parameters {
                    if parameter.id.is_assigned() {
                        next_param = next_param.max(parameter.id.raw() + 1);
                    }
                    if let crate::dynamic::ParameterKind::Choice(choice) = &parameter.kind {
                        for option in &choice.options {
                            if option.id.is_assigned() {
                                next_option = next_option.max(option.id.raw() + 1);
                            }
                        }
                    }
                }
                for behavior in &dynamic.behaviors {
                    if behavior.id.is_assigned() {
                        next_action = next_action.max(behavior.id.raw() + 1);
                    }
                }
            }
        }
        for block in self.blocks.values_mut() {
            if !block.id.is_assigned() {
                block.id = BlockDefinitionId(next_def);
                next_def += 1;
            }
        }
        self.next_definition_id = next_def;
        self.next_parameter_id = next_param;
        self.next_option_id = next_option;
        self.next_action_id = next_action;
    }

    pub fn assign_missing_vertex_ids(&mut self) {
        let mut next = self.next_vertex_id.max(1);
        for entity in self.model_space.iter_mut().chain(
            self.blocks
                .values_mut()
                .flat_map(|block| block.entities.iter_mut()),
        ) {
            bump_vertex_counter(entity, &mut next);
        }
        for entity in self.model_space.iter_mut().chain(
            self.blocks
                .values_mut()
                .flat_map(|block| block.entities.iter_mut()),
        ) {
            assign_entity_vertex_ids(entity, &mut next);
        }
        self.next_vertex_id = next;
    }

    pub fn ensure_entity_vertex_ids(&mut self, entity: &mut Entity) {
        assign_entity_vertex_ids(entity, &mut self.next_vertex_id);
    }

    pub fn remap_entity_vertex_ids(
        &mut self,
        entities: &mut [Entity],
    ) -> std::collections::BTreeMap<VertexId, VertexId> {
        let mut map = std::collections::BTreeMap::new();
        for entity in entities.iter_mut() {
            if let Some(vertices) = entity.geometry.polyline_vertices_mut() {
                for vertex in vertices {
                    if vertex.vertex_id.is_assigned() {
                        let new_id = self.allocate_vertex_id();
                        map.insert(vertex.vertex_id, new_id);
                        vertex.vertex_id = new_id;
                    } else {
                        vertex.vertex_id = self.allocate_vertex_id();
                    }
                }
            }
        }
        map
    }

    pub fn new_entity(&self, geometry: Geometry) -> Entity {
        let mut entity = Entity::new(geometry);
        entity.layer = self.current_layer.clone();
        entity.color = CadColor::ByLayer;
        entity.linetype = "BYLAYER".into();
        entity
    }

    pub fn insert_model_entity(&mut self, index: usize, entity: Entity) -> Entity {
        self.insert_entity(&EntitySpace::ModelSpace, index, entity)
            .expect("model space is always present")
    }

    pub fn add_entity(&mut self, entity: Entity) -> Entity {
        self.add_entity_to(&EntitySpace::ModelSpace, entity)
            .expect("model space is always present")
    }

    pub fn remove_model_entity(&mut self, id: EntityId) -> Option<(usize, Entity)> {
        self.remove_entity_from(&EntitySpace::ModelSpace, id)
    }

    pub fn replace_model_entity(&mut self, id: EntityId, entity: Entity) -> Option<Entity> {
        self.replace_entity_in(&EntitySpace::ModelSpace, id, entity)
    }

    pub fn entities(&self, space: &EntitySpace) -> Option<&[Entity]> {
        match space {
            EntitySpace::ModelSpace => Some(&self.model_space),
            EntitySpace::Block(name) => self
                .block_by_name(name)
                .map(|block| block.entities.as_slice()),
        }
    }

    pub fn entities_mut(&mut self, space: &EntitySpace) -> Option<&mut Vec<Entity>> {
        match space {
            EntitySpace::ModelSpace => Some(&mut self.model_space),
            EntitySpace::Block(name) => {
                let key = self.block_key(name)?;
                self.blocks.get_mut(&key).map(|block| &mut block.entities)
            }
        }
    }

    pub fn insert_entity(
        &mut self,
        space: &EntitySpace,
        index: usize,
        mut entity: Entity,
    ) -> Option<Entity> {
        if entity.id.is_assigned() {
            self.next_entity_id = self.next_entity_id.max(entity.id.raw() + 1);
        } else {
            entity.id = self.allocate_id();
        }
        assign_entity_vertex_ids(&mut entity, &mut self.next_vertex_id);
        let space = self.canonical_space(space)?;
        let index = {
            let entities = self.entities_mut(&space)?;
            let index = index.min(entities.len());
            entities.insert(index, entity.clone());
            index
        };
        self.on_entity_inserted(&space, index, entity.id);
        self.bump_generation();
        Some(entity)
    }

    pub fn add_entity_to(&mut self, space: &EntitySpace, entity: Entity) -> Option<Entity> {
        let index = self.entities(space)?.len();
        self.insert_entity(space, index, entity)
    }

    pub fn remove_entity_from(
        &mut self,
        space: &EntitySpace,
        id: EntityId,
    ) -> Option<(usize, Entity)> {
        let index = self.entity_index_in(space, id)?;
        let space = self.canonical_space(space)?;
        let entity = self.entities_mut(&space)?.remove(index);
        self.on_entity_removed(&space, index, entity.id);
        self.bump_generation();
        Some((index, entity))
    }

    pub fn replace_entity_in(
        &mut self,
        space: &EntitySpace,
        id: EntityId,
        mut entity: Entity,
    ) -> Option<Entity> {
        let index = self.entity_index_in(space, id)?;
        let space = self.canonical_space(space)?;
        if !entity.id.is_assigned() {
            entity.id = id;
        } else {
            self.next_entity_id = self.next_entity_id.max(entity.id.raw() + 1);
        }
        let new_id = entity.id;
        let previous = std::mem::replace(&mut self.entities_mut(&space)?[index], entity);
        if previous.id != new_id {
            self.entity_locations.remove(&previous.id);
            self.entity_locations
                .insert(new_id, EntityLocation { space, index });
        }
        self.bump_generation();
        Some(previous)
    }

    pub fn entity_by_id_in(&self, space: &EntitySpace, id: EntityId) -> Option<&Entity> {
        let location = self.entity_locations.get(&id)?;
        if !location.space.matches(space) {
            return None;
        }
        self.entities(&location.space)?
            .get(location.index)
            .filter(|entity| entity.id == id)
    }

    pub fn entity_index_in(&self, space: &EntitySpace, id: EntityId) -> Option<usize> {
        let location = self.entity_locations.get(&id)?;
        location.space.matches(space).then_some(location.index)
    }

    pub fn find_entity_location(&self, id: EntityId) -> Option<(EntitySpace, usize)> {
        self.entity_locations
            .get(&id)
            .map(|location| (location.space.clone(), location.index))
    }

    pub fn entity_location(&self, id: EntityId) -> Option<&EntityLocation> {
        self.entity_locations.get(&id)
    }

    pub fn entity_by_id(&self, id: EntityId) -> Option<&Entity> {
        let location = self.entity_locations.get(&id)?;
        self.entities(&location.space)?
            .get(location.index)
            .filter(|entity| entity.id == id)
    }

    pub fn entity_index(&self, id: EntityId) -> Option<usize> {
        self.entity_index_in(&EntitySpace::ModelSpace, id)
    }

    pub fn rebuild_entity_index(&mut self) {
        self.entity_locations = self.scan_entity_locations();
    }

    pub fn entity_index_is_consistent(&self) -> bool {
        self.entity_locations == self.scan_entity_locations()
    }

    fn scan_entity_locations(&self) -> HashMap<EntityId, EntityLocation> {
        let extra: usize = self.blocks.values().map(|block| block.entities.len()).sum();
        let mut locations = HashMap::with_capacity(self.model_space.len() + extra);
        for (index, entity) in self.model_space.iter().enumerate() {
            if entity.id.is_assigned() {
                locations.insert(
                    entity.id,
                    EntityLocation {
                        space: EntitySpace::ModelSpace,
                        index,
                    },
                );
            }
        }
        for (name, block) in &self.blocks {
            for (index, entity) in block.entities.iter().enumerate() {
                if entity.id.is_assigned() {
                    locations.insert(
                        entity.id,
                        EntityLocation {
                            space: EntitySpace::Block(name.clone()),
                            index,
                        },
                    );
                }
            }
        }
        locations
    }

    fn canonical_space(&self, space: &EntitySpace) -> Option<EntitySpace> {
        match space {
            EntitySpace::ModelSpace => Some(EntitySpace::ModelSpace),
            EntitySpace::Block(name) => self.block_key(name).map(EntitySpace::Block),
        }
    }

    fn on_entity_inserted(&mut self, space: &EntitySpace, index: usize, id: EntityId) {
        self.shift_indices(space, index, 1);
        self.entity_locations.insert(
            id,
            EntityLocation {
                space: space.clone(),
                index,
            },
        );
    }

    fn on_entity_removed(&mut self, space: &EntitySpace, index: usize, id: EntityId) {
        self.entity_locations.remove(&id);
        self.shift_indices(space, index + 1, -1);
    }

    fn shift_indices(&mut self, space: &EntitySpace, from: usize, delta: isize) {
        for location in self.entity_locations.values_mut() {
            if location.space.matches(space) && location.index >= from {
                location.index = location.index.saturating_add_signed(delta);
            }
        }
    }

    fn forget_space(&mut self, space: &EntitySpace) {
        self.entity_locations
            .retain(|_, location| !location.space.matches(space));
    }

    pub(crate) fn retarget_block_space(&mut self, from: &str, to: &str) {
        for location in self.entity_locations.values_mut() {
            if location.space.same_block(from) {
                location.space = EntitySpace::Block(to.to_string());
            }
        }
    }

    pub fn block_key(&self, name: &str) -> Option<String> {
        if self.blocks.contains_key(name) {
            return Some(name.to_string());
        }
        self.blocks
            .keys()
            .find(|key| key.eq_ignore_ascii_case(name))
            .cloned()
    }

    pub fn block_by_name(&self, name: &str) -> Option<&BlockDefinition> {
        let key = self.block_key(name)?;
        self.blocks.get(&key)
    }

    pub fn block_by_name_mut(&mut self, name: &str) -> Option<&mut BlockDefinition> {
        let key = self.block_key(name)?;
        self.blocks.get_mut(&key)
    }

    pub fn replace_block_definition(
        &mut self,
        mut definition: BlockDefinition,
    ) -> Option<BlockDefinition> {
        if !definition.id.is_assigned() {
            definition.id = self.allocate_definition_id();
        } else {
            self.next_definition_id = self.next_definition_id.max(definition.id.raw() + 1);
        }
        for entity in &mut definition.entities {
            if entity.id.is_assigned() {
                self.next_entity_id = self.next_entity_id.max(entity.id.raw() + 1);
            } else {
                entity.id = self.allocate_id();
            }
            assign_entity_vertex_ids(entity, &mut self.next_vertex_id);
        }
        let previous = if let Some(key) = self.block_key(&definition.name) {
            self.forget_space(&EntitySpace::Block(key.clone()));
            self.blocks.remove(&key)
        } else {
            None
        };
        if let Some(previous) = &previous {
            if previous.id == definition.id {
                definition.content_revision = previous.content_revision.saturating_add(1);
            }
        }
        let new_name = definition.name.clone();
        self.blocks.insert(new_name.clone(), definition);
        if let Some(block) = self.blocks.get(&new_name) {
            let entries: Vec<(EntityId, usize)> = block
                .entities
                .iter()
                .enumerate()
                .filter_map(|(index, entity)| entity.id.is_assigned().then_some((entity.id, index)))
                .collect();
            let space = EntitySpace::Block(new_name);
            for (id, index) in entries {
                self.entity_locations.insert(
                    id,
                    EntityLocation {
                        space: space.clone(),
                        index,
                    },
                );
            }
        }
        self.bump_generation();
        previous
    }

    pub fn remove_block_definition(&mut self, name: &str) -> Option<BlockDefinition> {
        let key = self.block_key(name)?;
        self.forget_space(&EntitySpace::Block(key.clone()));
        let removed = self.blocks.remove(&key);
        if removed.is_some() {
            self.bump_generation();
        }
        removed
    }

    pub fn entity_world_extents(&self, entity: &Entity, transform: Transform2) -> Option<Extents2> {
        let mut extents = Extents2::empty();
        let mut any = false;
        let mut stack = Vec::new();
        collect_entity_points(self, entity, transform, &mut stack, &mut |p| {
            if p.is_finite() {
                extents.include(p);
                any = true;
            }
        });
        any.then_some(extents)
    }

    pub fn entities_extents(&self, space: &EntitySpace, ids: &[EntityId]) -> Option<Extents2> {
        let mut extents = Extents2::empty();
        let mut any = false;
        for id in ids {
            let Some(entity) = self.entity_by_id_in(space, *id) else {
                continue;
            };
            if let Some(extra) = self.entity_world_extents(entity, Transform2::identity()) {
                extents.union(extra);
                any = true;
            }
        }
        any.then_some(extents)
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

    pub fn layer_is_plottable(&self, name: &str) -> bool {
        self.layer(name).map(|l| l.is_plottable()).unwrap_or(true)
    }

    pub fn layer_is_visible(&self, name: &str) -> bool {
        self.layer_is_plottable(name)
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

    pub fn expand_extents_for(&mut self, entity: &Entity) {
        let extra = {
            let mut extents = Extents2::empty();
            let mut any = false;
            let mut stack = Vec::new();
            collect_entity_points(self, entity, Transform2::identity(), &mut stack, &mut |p| {
                if p.is_finite() {
                    extents.include(p);
                    any = true;
                }
            });
            any.then_some(extents)
        };
        let Some(extra) = extra else {
            return;
        };
        match self.diagnostics.extents.as_mut() {
            Some(existing) => existing.union(extra),
            None => self.diagnostics.extents = Some(extra),
        }
    }

    pub fn mark_extents_stale(&mut self) {
        self.extents_stale = true;
    }

    pub fn ensure_cached_extents(&mut self) {
        if !self.extents_stale {
            return;
        }
        self.diagnostics.extents = self.compute_extents();
        self.extents_stale = false;
    }

    pub fn note_entity_removed_from_extents(&mut self, entity: &Entity) {
        if self.entity_touches_cached_extents(entity) {
            self.mark_extents_stale();
        }
    }

    pub fn note_entity_replaced_in_extents(&mut self, before: &Entity, after: &Entity) {
        if self.entity_touches_cached_extents(before) {
            self.mark_extents_stale();
        }
        self.expand_extents_for(after);
    }

    pub fn recompute_cached_extents(&mut self) {
        self.diagnostics.extents = self.compute_extents();
        self.extents_stale = false;
    }

    fn entity_touches_cached_extents(&self, entity: &Entity) -> bool {
        let Some(world) = self.diagnostics.extents else {
            return false;
        };
        let Some(bounds) = self.entity_world_extents(entity, Transform2::identity()) else {
            return false;
        };
        const EPS: f64 = 1e-9;
        bounds.min.x <= world.min.x + EPS
            || bounds.min.y <= world.min.y + EPS
            || bounds.max.x >= world.max.x - EPS
            || bounds.max.y >= world.max.y - EPS
    }

    pub fn compute_extents(&self) -> Option<Extents2> {
        let _span = crate::perf::span("compute_extents");
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
            configuration: _,
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

fn bump_vertex_counter(entity: &Entity, next: &mut u64) {
    if let Some(vertices) = entity.geometry.polyline_vertices() {
        for vertex in vertices {
            if vertex.vertex_id.is_assigned() {
                *next = (*next).max(vertex.vertex_id.raw() + 1);
            }
        }
    }
}

fn assign_entity_vertex_ids(entity: &mut Entity, next: &mut u64) {
    if let Some(vertices) = entity.geometry.polyline_vertices_mut() {
        for vertex in vertices {
            if !vertex.vertex_id.is_assigned() {
                vertex.vertex_id = VertexId(*next);
                *next = next.saturating_add(1).max(1);
            } else {
                *next = (*next).max(vertex.vertex_id.raw() + 1);
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
                ..Default::default()
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
            configuration: None,
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
                ..Default::default()
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
            configuration: None,
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
                vertex_id: Default::default(),
        },
                PolyVertex {
                    point: Point3::from_xy(8.0, 2.0),
                    bulge: 0.0,
                vertex_id: Default::default(),
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
        assert_eq!(DrawingUnits::Millimeters.to_insunits(), 4);
        assert_eq!(DrawingUnits::from_insunits(4).to_insunits(), 4);
    }

    #[test]
    fn expand_extents_for_appends_without_rebuilding() {
        let mut document = Document::default();
        let first = document.new_entity(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(2.0, 0.0),
        });
        document.expand_extents_for(&first);
        let second = document.new_entity(Geometry::Circle {
            center: Point3::from_xy(10.0, 0.0),
            radius: 1.0,
            extrusion: crate::entity::default_extrusion(),
        });
        document.expand_extents_for(&second);
        let extents = document.diagnostics.extents.expect("extents");
        assert!((extents.min.x - 0.0).abs() < 1e-12);
        assert!((extents.max.x - 11.0).abs() < 1e-12);
    }

    #[test]
    fn extents_mark_stale_when_a_boundary_entity_moves_inward() {
        let mut document = Document::default();
        let left = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        let right = document.add_entity(line(10.0, 0.0, 11.0, 0.0));
        document.expand_extents_for(&left);
        document.expand_extents_for(&right);
        let mut moved = right.clone();
        if let Geometry::Line { start, end } = &mut moved.geometry {
            *start = Point3::from_xy(4.0, 0.0);
            *end = Point3::from_xy(5.0, 0.0);
        }
        document.replace_entity_in(&EntitySpace::ModelSpace, right.id, moved.clone());
        document.note_entity_replaced_in_extents(&right, &moved);
        document.ensure_cached_extents();
        let extents = document.diagnostics.extents.expect("extents");
        assert!((extents.max.x - 5.0).abs() < 1e-12);
    }

    #[test]
    fn entity_index_tracks_insert_remove_replace_and_block_rename() {
        let mut document = Document::default();
        let a = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        let b = document.add_entity(line(1.0, 0.0, 2.0, 0.0));
        let c = document.insert_model_entity(0, line(2.0, 0.0, 3.0, 0.0));
        assert_eq!(document.entity_index(c.id), Some(0));
        assert_eq!(document.entity_index(a.id), Some(1));
        assert_eq!(document.entity_index(b.id), Some(2));
        assert!(document.entity_index_is_consistent());

        document.remove_model_entity(a.id);
        assert!(document.entity_by_id(a.id).is_none());
        assert_eq!(document.entity_index(b.id), Some(1));
        assert!(document.entity_index_is_consistent());

        let mut moved = b.clone();
        if let Geometry::Line { end, .. } = &mut moved.geometry {
            end.x = 9.0;
        }
        document.replace_model_entity(b.id, moved);
        assert_eq!(
            document
                .entity_by_id(b.id)
                .and_then(|entity| match &entity.geometry {
                    Geometry::Line { end, .. } => Some(end.x),
                    _ => None,
                }),
            Some(9.0)
        );

        document.replace_block_definition(BlockDefinition {
            name: "SYM".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: Vec::new(),
            ..Default::default()
        });
        let member = document
            .add_entity_to(&EntitySpace::Block("SYM".into()), line(0.0, 0.0, 4.0, 0.0))
            .unwrap();
        assert_eq!(
            document.find_entity_location(member.id),
            Some((EntitySpace::Block("SYM".into()), 0))
        );
        document.rename_block("SYM", "MARK").unwrap();
        assert_eq!(
            document.find_entity_location(member.id),
            Some((EntitySpace::Block("MARK".into()), 0))
        );
        assert!(document.entity_by_id(member.id).is_some());
        assert!(document.entity_index_is_consistent());

        document.remove_block_definition("MARK");
        assert!(document.entity_by_id(member.id).is_none());
        assert!(document.entity_index_is_consistent());
    }

    #[test]
    fn entity_index_survives_randomized_edits() {
        let mut document = Document::default();
        document.replace_block_definition(BlockDefinition {
            name: "CELL".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: Vec::new(),
            ..Default::default()
        });
        let space = EntitySpace::Block("CELL".into());
        let mut seed = 0x9e3779b97f4a7c15u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for step in 0..400 {
            let roll = next() % 8;
            match roll {
                0 => {
                    let x = (next() % 50) as f64;
                    document.add_entity(line(x, 0.0, x + 1.0, 0.0));
                }
                1 if !document.model_space.is_empty() => {
                    let index = (next() as usize) % document.model_space.len();
                    let id = document.model_space[index].id;
                    document.remove_model_entity(id);
                }
                2 if !document.model_space.is_empty() => {
                    let index = (next() as usize) % document.model_space.len();
                    let id = document.model_space[index].id;
                    let x = (next() % 50) as f64;
                    document.replace_model_entity(id, line(x, 1.0, x + 1.0, 1.0));
                }
                3 => {
                    let at = (next() as usize) % (document.model_space.len() + 1);
                    document
                        .insert_model_entity(at, line(step as f64, 2.0, step as f64 + 1.0, 2.0));
                }
                4 => {
                    let x = (next() % 50) as f64;
                    let _ = document.add_entity_to(&space, line(x, 3.0, x + 1.0, 3.0));
                }
                5 => {
                    let id = document.block_by_name("CELL").and_then(|block| {
                        if block.entities.is_empty() {
                            None
                        } else {
                            Some(block.entities[(next() as usize) % block.entities.len()].id)
                        }
                    });
                    if let Some(id) = id {
                        document.remove_entity_from(&space, id);
                    }
                }
                6 if !document.model_space.is_empty() => {
                    let index = (next() as usize) % document.model_space.len();
                    let id = document.model_space[index].id;
                    let _ = crate::transfer_entity(
                        &mut document,
                        id,
                        &space,
                        crate::Transform2::identity(),
                    );
                }
                _ => {
                    let id = document.block_by_name("CELL").and_then(|block| {
                        if block.entities.is_empty() {
                            None
                        } else {
                            Some(block.entities[(next() as usize) % block.entities.len()].id)
                        }
                    });
                    if let Some(id) = id {
                        let _ = crate::transfer_entity(
                            &mut document,
                            id,
                            &EntitySpace::ModelSpace,
                            crate::Transform2::identity(),
                        );
                    }
                }
            }
            assert!(
                document.entity_index_is_consistent(),
                "entity index drifted at step {step}"
            );
        }
        let cloned = document.clone();
        assert!(cloned.entity_index_is_consistent());
        for entity in &document.model_space {
            assert_eq!(
                cloned.entity_by_id(entity.id).map(|found| found.id),
                Some(entity.id)
            );
        }
    }
}
