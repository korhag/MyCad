//! Block definitions, naming, references, and membership operations.

use std::collections::BTreeMap;

use crate::document::{BlockDefinition, Document, EntitySpace};
use crate::entity::{default_extrusion, Entity, EntityId, Geometry};
use crate::entity_transform::{transform_entity_matrix, TransformError};
use crate::geom::{Point2, Point3};
use crate::transform::Transform2;

pub const NON_UNIFORM_MEMBERSHIP_MESSAGE: &str = "Cannot move this object into the block because this block instance uses a non-uniform transform that this geometry type cannot yet preserve exactly.";

// ------------------------------------------------------------
// Enum: BlockError
// Purpose: Recoverable block-workflow failures shown in the status bar.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockError {
    EmptySelection,
    ReservedName(String),
    DuplicateName(String),
    StarPrefix,
    InvalidName,
    MissingBlock,
    MissingEntity,
    Cycle,
    NonUniform,
    Transform(String),
}

impl std::fmt::Display for BlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySelection => f.write_str("Select objects first"),
            Self::ReservedName(name) => write!(f, "Block name '{name}' is reserved"),
            Self::DuplicateName(name) => write!(f, "A block named '{name}' already exists"),
            Self::StarPrefix => f.write_str("User block names cannot begin with '*'"),
            Self::InvalidName => f.write_str("Invalid block name"),
            Self::MissingBlock => f.write_str("Block definition was not found"),
            Self::MissingEntity => f.write_str("Selected object is no longer in the drawing"),
            Self::Cycle => f.write_str("Cannot create a circular block reference"),
            Self::NonUniform => f.write_str(NON_UNIFORM_MEMBERSHIP_MESSAGE),
            Self::Transform(message) => f.write_str(message),
        }
    }
}

impl From<TransformError> for BlockError {
    fn from(err: TransformError) -> Self {
        match err {
            TransformError::Unsupported(_) => Self::NonUniform,
            other => Self::Transform(other.to_string()),
        }
    }
}

// ------------------------------------------------------------
// Type: CreateBlockResult
// Purpose: Document mutations for one Create Block command, for undo.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct CreateBlockResult {
    pub name: String,
    pub definition: BlockDefinition,
    pub removed: Vec<(EntitySpace, usize, Entity)>,
    pub insert: Option<(EntitySpace, usize, Entity)>,
}

#[derive(Debug, Clone)]
pub struct TransferResult {
    pub source: EntitySpace,
    pub source_index: usize,
    pub dest: EntitySpace,
    pub dest_index: usize,
    pub before: Entity,
    pub after: Entity,
}

#[derive(Debug, Clone)]
pub struct MakeUniqueResult {
    pub new_name: String,
    pub definition: BlockDefinition,
    pub insert_before: Entity,
    pub insert_after: Entity,
    pub insert_space: EntitySpace,
    pub insert_index: usize,
    pub entity_map: std::collections::BTreeMap<EntityId, EntityId>,
}

pub fn is_system_block_name(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return true;
    }
    let upper = trimmed.to_ascii_uppercase();
    trimmed.starts_with('*')
        || upper == "MODEL_SPACE"
        || upper == "PAPER_SPACE"
        || upper == "*MODEL_SPACE"
        || upper == "*PAPER_SPACE"
}

pub fn is_user_editable_block_name(name: &str) -> bool {
    !is_system_block_name(name)
}

pub fn validate_user_block_name(document: &Document, name: &str) -> Result<String, BlockError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(BlockError::InvalidName);
    }
    if trimmed.starts_with('*') {
        return Err(BlockError::StarPrefix);
    }
    if is_system_block_name(trimmed) {
        return Err(BlockError::ReservedName(trimmed.to_string()));
    }
    if document.block_key(trimmed).is_some() {
        return Err(BlockError::DuplicateName(trimmed.to_string()));
    }
    Ok(trimmed.to_string())
}

/// `Ok(None)` is a no-op (same name). `Ok(Some(name))` is a valid new name.
pub fn validate_block_rename(
    document: &Document,
    from: &str,
    to: &str,
) -> Result<Option<String>, BlockError> {
    if !is_user_editable_block_name(from) {
        return Err(BlockError::ReservedName(from.to_string()));
    }
    if document.block_key(from).is_none() {
        return Err(BlockError::MissingBlock);
    }
    let trimmed = to.trim();
    if trimmed.is_empty() {
        return Err(BlockError::InvalidName);
    }
    if trimmed.starts_with('*') {
        return Err(BlockError::StarPrefix);
    }
    if is_system_block_name(trimmed) {
        return Err(BlockError::ReservedName(trimmed.to_string()));
    }
    let Some(old_key) = document.block_key(from) else {
        return Err(BlockError::MissingBlock);
    };
    if trimmed == old_key {
        return Ok(None);
    }
    if trimmed.eq_ignore_ascii_case(&old_key) {
        return Ok(Some(trimmed.to_string()));
    }
    if document.block_key(trimmed).is_some() {
        return Err(BlockError::DuplicateName(trimmed.to_string()));
    }
    Ok(Some(trimmed.to_string()))
}

pub fn next_user_block_name(document: &Document) -> String {
    for index in 1..10_000 {
        let candidate = format!("Block_{index:03}");
        if document.block_key(&candidate).is_none() {
            return candidate;
        }
    }
    format!("Block_{}", document.blocks.len() + 1)
}

pub fn resolve_block_name(document: &Document, requested: &str) -> Result<String, BlockError> {
    if requested.trim().is_empty() {
        Ok(next_user_block_name(document))
    } else {
        validate_user_block_name(document, requested)
    }
}

pub fn count_block_references(document: &Document, name: &str) -> usize {
    count_inserts(&document.model_space, name)
        + document
            .blocks
            .values()
            .map(|block| count_inserts(&block.entities, name))
            .sum::<usize>()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockTreeChild {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockTreeIndex {
    children: BTreeMap<String, Vec<BlockTreeChild>>,
}

impl BlockTreeIndex {
    pub fn build(document: &Document) -> Self {
        let _span = crate::perf::span("BlockTreeIndex::build");
        let mut children = BTreeMap::new();
        children.insert(
            String::new(),
            tally_block_children(&document.model_space, document),
        );
        for (name, definition) in &document.blocks {
            children.insert(
                name.to_ascii_lowercase(),
                tally_block_children(&definition.entities, document),
            );
        }
        Self { children }
    }

    pub fn model_children(&self) -> &[BlockTreeChild] {
        self.children_of("")
    }

    pub fn children_of(&self, parent: &str) -> &[BlockTreeChild] {
        let key = if parent.is_empty() {
            String::new()
        } else {
            parent.to_ascii_lowercase()
        };
        self.children.get(&key).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn rename(&mut self, from: &str, to: &str) {
        let from_key = from.to_ascii_lowercase();
        let to_key = to.to_ascii_lowercase();
        for kids in self.children.values_mut() {
            for child in kids.iter_mut() {
                if child.name.eq_ignore_ascii_case(from) {
                    child.name = to.to_string();
                }
            }
            kids.sort_by(|a, b| {
                a.name
                    .to_ascii_lowercase()
                    .cmp(&b.name.to_ascii_lowercase())
            });
        }
        if from_key != to_key {
            if let Some(kids) = self.children.remove(&from_key) {
                self.children.insert(to_key, kids);
            }
        }
    }
}

pub fn insert_instance_ids(document: &Document, name: &str) -> Vec<EntityId> {
    let mut ids = Vec::new();
    collect_insert_ids(&document.model_space, name, &mut ids);
    for block in document.blocks.values() {
        collect_insert_ids(&block.entities, name, &mut ids);
    }
    ids
}

pub fn insert_instance_ids_in_space(
    document: &Document,
    name: &str,
    space: &EntitySpace,
) -> Vec<EntityId> {
    let Some(entities) = document.entities(space) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    collect_insert_ids(entities, name, &mut ids);
    ids
}

fn collect_insert_ids(entities: &[Entity], name: &str, ids: &mut Vec<EntityId>) {
    for entity in entities {
        if let Geometry::Insert { block_name, .. } = &entity.geometry {
            if block_name.eq_ignore_ascii_case(name) {
                ids.push(entity.id);
            }
        }
    }
}

fn tally_block_children(entities: &[Entity], document: &Document) -> Vec<BlockTreeChild> {
    let mut map: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for entity in entities {
        let Some(raw) = referenced_block_name(entity) else {
            continue;
        };
        let display = document
            .block_by_name(raw)
            .map(|block| block.name.clone())
            .unwrap_or_else(|| raw.to_string());
        let key = display.to_ascii_lowercase();
        let entry = map.entry(key).or_insert((display, 0));
        entry.1 += 1;
    }
    let mut children: Vec<BlockTreeChild> = map
        .into_values()
        .map(|(name, count)| BlockTreeChild { name, count })
        .collect();
    children.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    children
}

fn referenced_block_name(entity: &Entity) -> Option<&str> {
    match &entity.geometry {
        Geometry::Insert { block_name, .. } | Geometry::Dimension { block_name } => {
            Some(block_name)
        }
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct BlockListEntry {
    pub name: String,
    pub references: usize,
    pub entities: usize,
    pub nested: Vec<String>,
}

pub fn user_block_list(document: &Document) -> Vec<BlockListEntry> {
    let mut entries: Vec<BlockListEntry> = document
        .blocks
        .values()
        .filter(|block| is_user_editable_block_name(&block.name))
        .map(|block| {
            let nested = block
                .entities
                .iter()
                .filter_map(|entity| match &entity.geometry {
                    Geometry::Insert { block_name, .. }
                        if is_user_editable_block_name(block_name) =>
                    {
                        Some(block_name.clone())
                    }
                    _ => None,
                })
                .collect();
            BlockListEntry {
                name: block.name.clone(),
                references: count_block_references(document, &block.name),
                entities: block.entities.len(),
                nested,
            }
        })
        .collect();
    entries.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    entries
}

pub fn duplicate_block_definition(
    document: &mut Document,
    name: &str,
) -> Result<BlockDefinition, BlockError> {
    if !is_user_editable_block_name(name) {
        return Err(BlockError::ReservedName(name.to_string()));
    }
    let source = document
        .block_by_name(name)
        .cloned()
        .ok_or(BlockError::MissingBlock)?;
    let new_name = unique_clone_name(document, &source.name);
    let (definition, _) = clone_definition_with_new_ids(document, &source, new_name);
    document.replace_block_definition(definition.clone());
    Ok(definition)
}

pub fn rename_block(document: &mut Document, from: &str, to: &str) -> Result<(), BlockError> {
    document.rename_block(from, to)
}

impl Document {
    pub fn rename_block(&mut self, old_name: &str, new_name: &str) -> Result<(), BlockError> {
        let Some(new_name) = validate_block_rename(self, old_name, new_name)? else {
            return Ok(());
        };
        let old_key = self.block_key(old_name).ok_or(BlockError::MissingBlock)?;
        let mut definition = self
            .blocks
            .remove(&old_key)
            .ok_or(BlockError::MissingBlock)?;
        rewrite_insert_names(&mut self.model_space, &old_key, &new_name);
        for block in self.blocks.values_mut() {
            rewrite_insert_names(&mut block.entities, &old_key, &new_name);
        }
        rewrite_insert_names(&mut definition.entities, &old_key, &new_name);
        definition.name = new_name.clone();
        self.blocks.insert(new_name.clone(), definition);
        self.retarget_block_space(&old_key, &new_name);
        self.bump_generation();
        Ok(())
    }
}

fn rewrite_insert_names(entities: &mut [Entity], from: &str, to: &str) {
    for entity in entities {
        match &mut entity.geometry {
            Geometry::Insert { block_name, .. } | Geometry::Dimension { block_name } => {
                if block_name.eq_ignore_ascii_case(from) {
                    *block_name = to.to_string();
                }
            }
            _ => {}
        }
    }
}

pub fn purge_unused_user_blocks(document: &mut Document) -> Vec<BlockDefinition> {
    let live = live_block_names(document);
    let names: Vec<String> = document.blocks.keys().cloned().collect();
    let mut removed = Vec::new();
    for name in names {
        if is_system_block_name(&name) {
            continue;
        }
        if live
            .iter()
            .any(|live_name| live_name.eq_ignore_ascii_case(&name))
        {
            continue;
        }
        if let Some(definition) = document.remove_block_definition(&name) {
            removed.push(definition);
        }
    }
    removed
}

pub fn insert_transform(document: &Document, entity: &Entity) -> Option<Transform2> {
    match &entity.geometry {
        Geometry::Insert {
            block_name,
            insertion,
            scale,
            rotation,
            extrusion,
            ..
        } => {
            let block = document.block_by_name(block_name)?;
            Some(Transform2::block_insert(
                *insertion,
                *scale,
                *rotation,
                *extrusion,
                block.base_pt,
            ))
        }
        _ => None,
    }
}

pub fn block_depends_on(document: &Document, block: &str, target: &str) -> bool {
    let mut stack = Vec::new();
    block_depends_on_inner(document, block, target, &mut stack)
}

pub fn would_create_block_cycle(
    document: &Document,
    destination: &str,
    inserted_block: &str,
) -> bool {
    destination.eq_ignore_ascii_case(inserted_block)
        || block_depends_on(document, inserted_block, destination)
}

pub fn entity_would_create_cycle(document: &Document, destination: &str, entity: &Entity) -> bool {
    match &entity.geometry {
        Geometry::Insert { block_name, .. } => {
            would_create_block_cycle(document, destination, block_name)
        }
        _ => false,
    }
}

pub fn identity_insert(block_name: String, insertion: Point3) -> Geometry {
    Geometry::Insert {
        block_name,
        insertion,
        scale: Point3::new(1.0, 1.0, 1.0),
        rotation: 0.0,
        extrusion: default_extrusion(),
        attribs: Vec::new(),
        column_count: 1,
        row_count: 1,
        column_spacing: 0.0,
        row_spacing: 0.0,
        configuration: None,
    }
}

pub fn create_block_from_entities(
    document: &mut Document,
    space: &EntitySpace,
    ids: &[EntityId],
    requested_name: &str,
    base: Point2,
    replace_with_insert: bool,
) -> Result<CreateBlockResult, BlockError> {
    if ids.is_empty() {
        return Err(BlockError::EmptySelection);
    }
    let name = resolve_block_name(document, requested_name)?;
    let mut members = Vec::new();
    for id in ids {
        let entity = document
            .entity_by_id_in(space, *id)
            .cloned()
            .ok_or(BlockError::MissingEntity)?;
        if let EntitySpace::Block(dest) = space {
            if entity_would_create_cycle(document, dest, &entity) {
                return Err(BlockError::Cycle);
            }
        }
        members.push(entity);
    }
    let translation = Transform2::translate(-base.x, -base.y);
    let mut local_members = Vec::new();
    for entity in &members {
        local_members.push(transform_entity_matrix(entity, translation)?);
    }
    if let EntitySpace::Block(parent) = space {
        for entity in &local_members {
            if entity_would_create_cycle(document, parent, entity) {
                return Err(BlockError::Cycle);
            }
        }
    }

    let mut removals = Vec::new();
    for id in ids {
        let index = document
            .entity_index_in(space, *id)
            .ok_or(BlockError::MissingEntity)?;
        let entity = document
            .entity_by_id_in(space, *id)
            .cloned()
            .ok_or(BlockError::MissingEntity)?;
        removals.push((space.clone(), index, entity));
    }
    removals.sort_by_key(|(_, index, _)| std::cmp::Reverse(*index));
    for (_, _, entity) in &removals {
        document
            .remove_entity_from(space, entity.id)
            .ok_or(BlockError::MissingEntity)?;
    }

    let definition = BlockDefinition::plain(name.clone(), Point3::from_xy(0.0, 0.0), local_members);
    document.replace_block_definition(definition.clone());

    let insert = if replace_with_insert {
        let entity = document.new_entity(identity_insert(
            name.clone(),
            Point3::from_xy(base.x, base.y),
        ));
        let inserted = document
            .add_entity_to(space, entity)
            .ok_or(BlockError::MissingBlock)?;
        let index = document.entity_index_in(space, inserted.id).unwrap_or(0);
        Some((space.clone(), index, inserted))
    } else {
        None
    };

    Ok(CreateBlockResult {
        name,
        definition,
        removed: removals,
        insert,
    })
}

pub fn transfer_entity(
    document: &mut Document,
    id: EntityId,
    dest: &EntitySpace,
    dest_from_source: Transform2,
) -> Result<TransferResult, BlockError> {
    let (source, _) = document
        .find_entity_location(id)
        .ok_or(BlockError::MissingEntity)?;
    if source == *dest {
        return Err(BlockError::Transform(
            "Object is already in this block".into(),
        ));
    }
    let before = document
        .entity_by_id_in(&source, id)
        .cloned()
        .ok_or(BlockError::MissingEntity)?;
    if let EntitySpace::Block(dest_name) = dest {
        if entity_would_create_cycle(document, dest_name, &before) {
            return Err(BlockError::Cycle);
        }
        if !dest_from_source.is_uniform_scale() && !geometry_survives_non_uniform(&before.geometry)
        {
            return Err(BlockError::NonUniform);
        }
    }
    let after = transform_entity_matrix(&before, dest_from_source)?;
    let (source_index, _) = document
        .remove_entity_from(&source, id)
        .ok_or(BlockError::MissingEntity)?;
    let inserted = document
        .add_entity_to(dest, after)
        .ok_or(BlockError::MissingBlock)?;
    let dest_index = document.entity_index_in(dest, inserted.id).unwrap_or(0);
    Ok(TransferResult {
        source,
        source_index,
        dest: dest.clone(),
        dest_index,
        before,
        after: inserted,
    })
}

pub fn make_unique_block(
    document: &mut Document,
    insert_id: EntityId,
) -> Result<MakeUniqueResult, BlockError> {
    let (space, index) = document
        .find_entity_location(insert_id)
        .ok_or(BlockError::MissingEntity)?;
    let insert_before = document
        .entity_by_id_in(&space, insert_id)
        .cloned()
        .ok_or(BlockError::MissingEntity)?;
    let Geometry::Insert { block_name, .. } = &insert_before.geometry else {
        return Err(BlockError::Transform("Select a block reference".into()));
    };
    if !is_user_editable_block_name(block_name) {
        return Err(BlockError::ReservedName(block_name.clone()));
    }
    let source = document
        .block_by_name(block_name)
        .cloned()
        .ok_or(BlockError::MissingBlock)?;
    let new_name = unique_clone_name(document, &source.name);
    let (definition, entity_map) =
        clone_definition_with_new_ids(document, &source, new_name.clone());
    document.replace_block_definition(definition.clone());
    let mut insert_after = insert_before.clone();
    if let Geometry::Insert {
        block_name,
        configuration,
        ..
    } = &mut insert_after.geometry
    {
        *block_name = new_name.clone();
        if let (Some(config), Some(dynamic)) = (configuration.as_mut(), definition.dynamic.as_ref())
        {
            let parameters: std::collections::BTreeMap<_, _> = source
                .dynamic
                .as_ref()
                .map(|original| {
                    original
                        .parameters
                        .iter()
                        .zip(dynamic.parameters.iter())
                        .map(|(from, to)| (from.id, to.id))
                        .collect()
                })
                .unwrap_or_default();
            config.remap_identities(&parameters, &BTreeMap::new());
        }
    }
    let _ = document.replace_entity_in(&space, insert_id, insert_after.clone());
    Ok(MakeUniqueResult {
        new_name,
        definition,
        insert_before,
        insert_after,
        insert_space: space,
        insert_index: index,
        entity_map,
    })
}

fn clone_definition_with_new_ids(
    document: &mut Document,
    source: &BlockDefinition,
    new_name: String,
) -> (
    BlockDefinition,
    std::collections::BTreeMap<EntityId, EntityId>,
) {
    let mut definition = source.clone();
    definition.id = document.allocate_definition_id();
    definition.name = new_name;
    definition.content_revision = 0;
    let mut entity_map = std::collections::BTreeMap::new();
    for entity in &mut definition.entities {
        let old = entity.id;
        entity.id = document.allocate_id();
        if old.is_assigned() {
            entity_map.insert(old, entity.id);
        }
    }
    let vertex_map = document.remap_entity_vertex_ids(&mut definition.entities);
    let mut parameter_map = std::collections::BTreeMap::new();
    let mut option_map = std::collections::BTreeMap::new();
    if let Some(dynamic) = definition.dynamic.as_mut() {
        let mut actions = std::collections::BTreeMap::new();
        let mut anchors = std::collections::BTreeMap::new();
        let mut presets = std::collections::BTreeMap::new();
        for parameter in &dynamic.parameters {
            parameter_map.insert(parameter.id, document.allocate_parameter_id());
            if let crate::dynamic::ParameterKind::Choice(choice) = &parameter.kind {
                for option in &choice.options {
                    option_map.insert(option.id, document.allocate_option_id());
                }
            }
        }
        for id in dynamic.collect_action_ids() {
            actions.insert(id, document.allocate_action_id());
        }
        for anchor in &dynamic.anchors {
            anchors.insert(anchor.id, document.allocate_anchor_id());
        }
        for preset in &dynamic.presets {
            presets.insert(preset.id, document.allocate_preset_id());
        }
        let _ = dynamic.remap_ids_with(
            &parameter_map,
            &option_map,
            &actions,
            &anchors,
            &presets,
            &entity_map,
            &vertex_map,
        );
    }
    if !parameter_map.is_empty() || !option_map.is_empty() {
        for entity in &mut definition.entities {
            if let Some(config) = entity.geometry.insert_configuration_mut() {
                if let Some(values) = config.as_mut() {
                    values.remap_identities(&parameter_map, &option_map);
                }
            }
        }
    }
    (definition, entity_map)
}

fn unique_clone_name(document: &Document, source: &str) -> String {
    for index in 1..10_000 {
        let candidate = format!("{source}_{index:03}");
        if document.block_key(&candidate).is_none() && is_user_editable_block_name(&candidate) {
            return candidate;
        }
    }
    format!("{source}_copy")
}

fn count_inserts(entities: &[Entity], name: &str) -> usize {
    entities
        .iter()
        .filter(|entity| match &entity.geometry {
            Geometry::Insert { block_name, .. } | Geometry::Dimension { block_name } => {
                block_name.eq_ignore_ascii_case(name)
            }
            _ => false,
        })
        .count()
}

fn live_block_names(document: &Document) -> Vec<String> {
    let mut live = Vec::new();
    push_referenced_names(&document.model_space, &mut live);
    let mut index = 0;
    while index < live.len() {
        let name = live[index].clone();
        index += 1;
        if let Some(definition) = document.block_by_name(&name) {
            push_referenced_names(&definition.entities, &mut live);
        }
    }
    live
}

fn push_referenced_names(entities: &[Entity], live: &mut Vec<String>) {
    for entity in entities {
        let name = match &entity.geometry {
            Geometry::Insert { block_name, .. } | Geometry::Dimension { block_name } => block_name,
            _ => continue,
        };
        if !live
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(name))
        {
            live.push(name.clone());
        }
    }
}

fn block_depends_on_inner(
    document: &Document,
    block: &str,
    target: &str,
    stack: &mut Vec<String>,
) -> bool {
    if stack.iter().any(|name| name.eq_ignore_ascii_case(block)) {
        return false;
    }
    let Some(definition) = document.block_by_name(block) else {
        return false;
    };
    stack.push(block.to_string());
    let found = definition
        .entities
        .iter()
        .any(|entity| match &entity.geometry {
            Geometry::Insert { block_name, .. } => {
                block_name.eq_ignore_ascii_case(target)
                    || block_depends_on_inner(document, block_name, target, stack)
            }
            _ => false,
        });
    stack.pop();
    found
}

fn geometry_survives_non_uniform(geometry: &Geometry) -> bool {
    matches!(
        geometry,
        Geometry::Line { .. }
            | Geometry::Point { .. }
            | Geometry::LwPolyline { .. }
            | Geometry::Polyline { .. }
            | Geometry::Spline { .. }
            | Geometry::Leader { .. }
            | Geometry::MLine { .. }
            | Geometry::Solid { .. }
            | Geometry::Insert { .. }
            | Geometry::Text(_)
            | Geometry::MText(_)
    )
}

pub fn membership_matrix(
    source_world_from_local: Transform2,
    dest_world_from_local: Transform2,
) -> Result<Transform2, BlockError> {
    let dest_local_from_world = dest_world_from_local
        .inverse()
        .ok_or_else(|| BlockError::Transform("Block transform is not invertible".into()))?;
    Ok(dest_local_from_world.compose(source_world_from_local))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::entity::Entity;
    use crate::extents::Extents2;

    fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> Entity {
        Entity::new(Geometry::Line {
            start: Point3::from_xy(x0, y0),
            end: Point3::from_xy(x1, y1),
        })
    }

    fn circle(x: f64, y: f64, radius: f64) -> Entity {
        Entity::new(Geometry::Circle {
            center: Point3::from_xy(x, y),
            radius,
            extrusion: default_extrusion(),
        })
    }

    fn insert_at(name: &str, x: f64, y: f64) -> Entity {
        Entity::new(identity_insert(name.into(), Point3::from_xy(x, y)))
    }

    fn extents_close(a: Extents2, b: Extents2) {
        assert!((a.min.x - b.min.x).abs() < 1e-9);
        assert!((a.min.y - b.min.y).abs() < 1e-9);
        assert!((a.max.x - b.max.x).abs() < 1e-9);
        assert!((a.max.y - b.max.y).abs() < 1e-9);
    }

    #[test]
    fn create_block_moves_geometry_and_preserves_extents() {
        let mut document = Document::default();
        let a = document.add_entity(line(10.0, 0.0, 20.0, 0.0));
        let b = document.add_entity(circle(15.0, 5.0, 2.0));
        let before = document.compute_extents().unwrap();
        let result = create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[a.id, b.id],
            "TestBlock",
            Point2::new(15.0, 2.5),
            true,
        )
        .expect("create");
        assert_eq!(document.model_space.len(), 1);
        match &document.model_space[0].geometry {
            Geometry::Insert {
                block_name,
                insertion,
                ..
            } => {
                assert_eq!(block_name, "TestBlock");
                assert!((insertion.x - 15.0).abs() < 1e-12);
                assert!((insertion.y - 2.5).abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
        let def = document.block_by_name("TestBlock").expect("def");
        assert_eq!(def.entities.len(), 2);
        assert_eq!(def.entities[0].id, a.id);
        assert_eq!(def.entities[1].id, b.id);
        assert_eq!(def.entities[0].layer, "0");
        let after = document.compute_extents().unwrap();
        extents_close(before, after);
        assert_eq!(result.name, "TestBlock");
        assert!(result.insert.is_some());
    }

    #[test]
    fn insert_instance_ids_in_space_excludes_nested_references() {
        let mut document = Document::default();
        document.replace_block_definition(BlockDefinition {
            name: "Leaf".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: Vec::new(),
            ..Default::default()
        });
        document.replace_block_definition(BlockDefinition {
            name: "Holder".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![insert_at("Leaf", 1.0, 0.0)],
            ..Default::default()
        });
        let model = document.add_entity(insert_at("Leaf", 5.0, 0.0));
        document.add_entity(insert_at("Holder", 0.0, 0.0));
        let model_ids = insert_instance_ids_in_space(&document, "Leaf", &EntitySpace::ModelSpace);
        assert_eq!(model_ids, vec![model.id]);
        let nested =
            insert_instance_ids_in_space(&document, "Leaf", &EntitySpace::Block("Holder".into()));
        assert_eq!(nested.len(), 1);
        assert_ne!(nested[0], model.id);
        assert_eq!(insert_instance_ids(&document, "Leaf").len(), 2);
    }

    #[test]
    fn far_from_origin_base_point_keeps_world_geometry() {
        let mut document = Document::default();
        let a = document.add_entity(line(1000.0, 2000.0, 1010.0, 2000.0));
        let b = document.add_entity(line(1000.0, 2000.0, 1000.0, 2005.0));
        let before = document.compute_extents().unwrap();
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[a.id, b.id],
            "Far",
            Point2::new(1000.0, 2000.0),
            true,
        )
        .unwrap();
        extents_close(before, document.compute_extents().unwrap());
    }

    #[test]
    fn generated_names_are_unique_and_reject_collisions() {
        let mut document = Document::default();
        let a = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[a.id],
            "",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        assert!(document.block_key("Block_001").is_some());
        let err = validate_user_block_name(&document, "block_001").unwrap_err();
        assert!(matches!(err, BlockError::DuplicateName(_)));
        assert!(matches!(
            validate_user_block_name(&document, "*D12"),
            Err(BlockError::StarPrefix)
        ));
        assert_eq!(next_user_block_name(&document), "Block_002");
    }

    #[test]
    fn cycle_detection_rejects_direct_and_indirect_loops() {
        let mut document = Document::default();
        document.replace_block_definition(BlockDefinition {
            name: "A".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![insert_at("B", 0.0, 0.0)],
            ..Default::default()
        });
        document.replace_block_definition(BlockDefinition {
            name: "B".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![insert_at("C", 0.0, 0.0)],
            ..Default::default()
        });
        document.replace_block_definition(BlockDefinition {
            name: "C".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![line(0.0, 0.0, 1.0, 0.0)],
            ..Default::default()
        });
        assert!(would_create_block_cycle(&document, "A", "A"));
        assert!(would_create_block_cycle(&document, "C", "A"));
        assert!(would_create_block_cycle(&document, "B", "A"));
        assert!(!would_create_block_cycle(&document, "A", "C"));
        assert!(block_depends_on(&document, "A", "C"));
        assert!(!block_depends_on(&document, "C", "A"));
    }

    #[test]
    fn add_and_remove_preserve_world_position() {
        let mut document = Document::default();
        let member = document.add_entity(line(4.0, 1.0, 6.0, 1.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[member.id],
            "A",
            Point2::new(5.0, 1.0),
            true,
        )
        .unwrap();
        let extra = document.add_entity(line(20.0, 0.0, 22.0, 0.0));
        let extra_id = extra.id;
        let before = document
            .entity_world_extents(&extra, Transform2::identity())
            .unwrap();
        let dest = EntitySpace::Block("A".into());
        let matrix =
            membership_matrix(Transform2::identity(), Transform2::translate(5.0, 1.0)).unwrap();
        transfer_entity(&mut document, extra_id, &dest, matrix).unwrap();
        assert!(document
            .entity_by_id_in(&EntitySpace::ModelSpace, extra_id)
            .is_none());
        let moved = document.entity_by_id_in(&dest, extra_id).unwrap();
        let world = Transform2::translate(5.0, 1.0);
        let after = document.entity_world_extents(moved, world).unwrap();
        extents_close(before, after);
        assert_eq!(count_block_references(&document, "A"), 1);

        transfer_entity(
            &mut document,
            extra_id,
            &EntitySpace::ModelSpace,
            membership_matrix(world, Transform2::identity()).unwrap(),
        )
        .unwrap();
        let restored = document
            .entity_by_id_in(&EntitySpace::ModelSpace, extra_id)
            .unwrap();
        extents_close(
            before,
            document
                .entity_world_extents(restored, Transform2::identity())
                .unwrap(),
        );
    }

    #[test]
    fn nested_remove_moves_into_parent_block() {
        let mut document = Document::default();
        let circle_ent = document.add_entity(circle(0.0, 0.0, 1.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[circle_ent.id],
            "B",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let line_ent = document.add_entity(line(8.0, 0.0, 10.0, 0.0));
        let b_insert_id = document.model_space[0].id;
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[line_ent.id, b_insert_id],
            "A",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let b_def = document.block_by_name("B").unwrap();
        let member_id = b_def.entities[0].id;
        transfer_entity(
            &mut document,
            member_id,
            &EntitySpace::Block("A".into()),
            Transform2::identity(),
        )
        .unwrap();
        assert!(document
            .entity_by_id_in(&EntitySpace::Block("B".into()), member_id)
            .is_none());
        assert!(document
            .entity_by_id_in(&EntitySpace::Block("A".into()), member_id)
            .is_some());
    }

    #[test]
    fn non_uniform_insert_rejects_circle_membership() {
        let mut document = Document::default();
        document.replace_block_definition(BlockDefinition {
            name: "A".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: Vec::new(),
            ..Default::default()
        });
        let circle_ent = document.add_entity(circle(1.0, 0.0, 1.0));
        let xf = Transform2::insert(Point3::from_xy(0.0, 0.0), Point3::new(2.0, 1.0, 1.0), 0.4);
        let err = transfer_entity(
            &mut document,
            circle_ent.id,
            &EntitySpace::Block("A".into()),
            xf.inverse().unwrap(),
        )
        .unwrap_err();
        assert_eq!(err, BlockError::NonUniform);
        assert!(err.to_string().contains("non-uniform"));
        assert!(document
            .entity_by_id_in(&EntitySpace::ModelSpace, circle_ent.id)
            .is_some());
    }

    #[test]
    fn make_unique_clones_definition_and_repoints_one_insert() {
        let mut document = Document::default();
        let member = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[member.id],
            "Motor",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let first = document.model_space[0].clone();
        let mut second = first.clone();
        second.id = crate::entity::EntityId::UNASSIGNED;
        let second = document.add_entity(second);
        let result = make_unique_block(&mut document, second.id).unwrap();
        assert_eq!(result.new_name, "Motor_001");
        assert!(document.block_by_name("Motor").is_some());
        assert!(document.block_by_name("Motor_001").is_some());
        match &document.model_space[0].geometry {
            Geometry::Insert { block_name, .. } => assert_eq!(block_name, "Motor"),
            other => panic!("{other:?}"),
        }
        match &document.model_space[1].geometry {
            Geometry::Insert { block_name, .. } => assert_eq!(block_name, "Motor_001"),
            other => panic!("{other:?}"),
        }
        let orig_id = document.block_by_name("Motor").unwrap().entities[0].id;
        let clone_id = document.block_by_name("Motor_001").unwrap().entities[0].id;
        assert_ne!(orig_id, clone_id);
    }

    #[test]
    fn nested_create_places_insert_in_parent_block() {
        let mut document = Document::default();
        let a = document.add_entity(line(0.0, 0.0, 2.0, 0.0));
        let b = document.add_entity(line(0.0, 1.0, 2.0, 1.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[a.id, b.id],
            "Machine",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let space = EntitySpace::Block("Machine".into());
        let ids: Vec<_> = document
            .entities(&space)
            .unwrap()
            .iter()
            .map(|e| e.id)
            .collect();
        create_block_from_entities(
            &mut document,
            &space,
            &ids[..1],
            "Nested",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let machine = document.block_by_name("Machine").unwrap();
        assert_eq!(machine.entities.len(), 2);
        assert!(matches!(
            machine.entities.last().unwrap().geometry,
            Geometry::Insert { .. }
        ));
        assert_eq!(document.block_by_name("Nested").unwrap().entities.len(), 1);
    }

    #[test]
    fn editing_definition_updates_every_reference() {
        let mut document = Document::default();
        let member = document.add_entity(line(0.0, 0.0, 10.0, 0.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[member.id],
            "Shared",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let mut second = document.model_space[0].clone();
        second.id = crate::entity::EntityId::UNASSIGNED;
        if let Geometry::Insert { insertion, .. } = &mut second.geometry {
            *insertion = Point3::from_xy(0.0, 20.0);
        }
        document.add_entity(second);
        assert_eq!(count_block_references(&document, "Shared"), 2);
        let space = EntitySpace::Block("Shared".into());
        let id = document.entities(&space).unwrap()[0].id;
        let mut moved = document.entity_by_id_in(&space, id).unwrap().clone();
        if let Geometry::Line { end, .. } = &mut moved.geometry {
            *end = Point3::from_xy(10.0, 4.0);
        }
        document.replace_entity_in(&space, id, moved);
        let first = document
            .entity_world_extents(&document.model_space[0], Transform2::identity())
            .unwrap();
        let second_ext = document
            .entity_world_extents(&document.model_space[1], Transform2::identity())
            .unwrap();
        assert!((first.max.y - 4.0).abs() < 1e-9);
        assert!((second_ext.max.y - 24.0).abs() < 1e-9);
    }

    #[test]
    fn duplicate_rename_and_purge_unused_blocks() {
        let mut document = Document::default();
        let member = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[member.id],
            "Motor",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let unused = document.add_entity(line(8.0, 0.0, 9.0, 0.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[unused.id],
            "Spare",
            Point2::new(8.0, 0.0),
            false,
        )
        .unwrap();
        assert!(document
            .entity_by_id_in(&EntitySpace::ModelSpace, unused.id)
            .is_none());
        let clone = duplicate_block_definition(&mut document, "Motor").unwrap();
        assert_eq!(clone.name, "Motor_001");
        assert!(document.block_by_name("Motor").is_some());
        match &document.model_space[0].geometry {
            Geometry::Insert { block_name, .. } => assert_eq!(block_name, "Motor"),
            other => panic!("{other:?}"),
        }
        rename_block(&mut document, "Motor", "Drive").unwrap();
        match &document.model_space[0].geometry {
            Geometry::Insert { block_name, .. } => assert_eq!(block_name, "Drive"),
            other => panic!("{other:?}"),
        }
        assert!(document.block_by_name("Motor").is_none());
        let purged = purge_unused_user_blocks(&mut document);
        let names: Vec<_> = purged.iter().map(|block| block.name.as_str()).collect();
        assert!(names.contains(&"Spare"));
        assert!(names.contains(&"Motor_001"));
        assert!(document.block_by_name("Drive").is_some());
        assert!(document.block_by_name("Spare").is_none());
    }

    #[test]
    fn tree_index_groups_repeated_inserts_and_stops_cycles() {
        let mut document = Document::default();
        document.replace_block_definition(BlockDefinition {
            name: "Motor".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![line(0.0, 0.0, 1.0, 0.0)],
            ..Default::default()
        });
        document.replace_block_definition(BlockDefinition {
            name: "Machine".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![
                insert_at("Motor", 0.0, 0.0),
                insert_at("Motor", 10.0, 0.0),
                insert_at("Machine", 20.0, 0.0),
            ],
            ..Default::default()
        });
        document.add_entity(insert_at("Machine", 0.0, 0.0));
        document.add_entity(insert_at("Machine", 40.0, 0.0));
        let index = BlockTreeIndex::build(&document);
        let roots = index.model_children();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].name, "Machine");
        assert_eq!(roots[0].count, 2);
        let nested = index.children_of("Machine");
        let motor = nested.iter().find(|child| child.name == "Motor").unwrap();
        assert_eq!(motor.count, 2);
        assert!(nested.iter().any(|child| child.name == "Machine"));
        let seen = vec!["Machine".to_string()];
        let cycle = nested.iter().any(|child| {
            child.name == "Machine"
                && seen
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(&child.name))
        });
        assert!(cycle);
    }

    #[test]
    fn rename_is_case_aware_and_preserves_ids() {
        let mut document = Document::default();
        let member = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[member.id],
            "Motor",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let insert_id = document.model_space[0].id;
        let member_id = document.block_by_name("Motor").unwrap().entities[0].id;
        document.rename_block("Motor", "MOTOR").unwrap();
        assert!(document.block_by_name("MOTOR").is_some());
        match &document.model_space[0].geometry {
            Geometry::Insert { block_name, .. } => assert_eq!(block_name, "MOTOR"),
            other => panic!("{other:?}"),
        }
        assert_eq!(document.model_space[0].id, insert_id);
        assert_eq!(
            document.block_by_name("MOTOR").unwrap().entities[0].id,
            member_id
        );
        assert!(matches!(
            validate_block_rename(&document, "MOTOR", ""),
            Err(BlockError::InvalidName)
        ));
        assert!(matches!(
            validate_block_rename(&document, "MOTOR", "*D1"),
            Err(BlockError::StarPrefix)
        ));
        document.replace_block_definition(BlockDefinition {
            name: "Other".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: Vec::new(),
            ..Default::default()
        });
        assert!(matches!(
            validate_block_rename(&document, "MOTOR", "other"),
            Err(BlockError::DuplicateName(_))
        ));
        assert_eq!(
            validate_block_rename(&document, "MOTOR", "MOTOR").unwrap(),
            None
        );
    }

    #[test]
    fn rename_rewrites_nested_inserts_in_other_definitions() {
        let mut document = Document::default();
        document.replace_block_definition(BlockDefinition {
            name: "Motor".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![line(0.0, 0.0, 1.0, 0.0)],
            ..Default::default()
        });
        document.replace_block_definition(BlockDefinition {
            name: "Machine".into(),
            base_pt: Point3::from_xy(0.0, 0.0),
            entities: vec![insert_at("Motor", 0.0, 0.0), insert_at("Motor", 10.0, 0.0)],
            ..Default::default()
        });
        document.add_entity(insert_at("Machine", 0.0, 0.0));
        let machine_insert_id = document.model_space[0].id;
        let nested_id = document.block_by_name("Machine").unwrap().entities[0].id;
        document.rename_block("Motor", "Drive").unwrap();
        match &document.block_by_name("Machine").unwrap().entities[0].geometry {
            Geometry::Insert { block_name, .. } => assert_eq!(block_name, "Drive"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            document.block_by_name("Machine").unwrap().entities[0].id,
            nested_id
        );
        assert_eq!(document.model_space[0].id, machine_insert_id);
        let index = BlockTreeIndex::build(&document);
        assert_eq!(index.children_of("Machine")[0].name, "Drive");
        assert_eq!(index.children_of("Machine")[0].count, 2);
    }
}
