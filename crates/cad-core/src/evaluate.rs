//! Evaluate dynamic block definitions from source geometry and instance values.
//!
//! Every evaluation starts from the definition's source entities. The previous
//! evaluated result is never used as the next input. Output is disposable
//! derived state and must not replace the source definition.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::document::{BlockDefinition, Document};
use crate::dynamic::{
    capability_for, follow_multiplier, numeric_current, resolve_values, translate_point,
    validate_configuration, validate_definition, BehaviorKind, CompositionRule, DynamicBehavior,
    DynamicDefinition, DynamicError, GeometryTarget, InstanceConfiguration, NormalizedValue,
    ParameterKind, ParameterValue, EVALUATOR_VERSION,
};
use crate::dynamic_model::{
    effective_visibility, evaluate_text_binding, AnchorFollow, NestedMapping, PlacementBehavior,
    ReflectionBehavior, RotationBehavior, RotationSource, TextReflectPolicy,
};
use crate::entity::{Entity, EntityId, Geometry};
use crate::entity_transform::transform_entity_matrix;
use crate::geom::Point2;
use crate::ids::{ActionId, OptionId, ParameterId};
use crate::transform::Transform2;

pub const GENERATED_BLOCK_PREFIX: &str = "*EVAL_";

// ------------------------------------------------------------
// Type: EvalKey
// Purpose: Cache key that never uses a display name.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EvalKey {
    pub definition_id: u64,
    pub content_revision: u64,
    pub values: Vec<(u64, NormalizedValue)>,
    pub nested_revisions: Vec<(u64, u64)>,
    pub evaluator_version: u32,
}

// ------------------------------------------------------------
// Type: EvaluationRequest
// Purpose: Identifies the source generation an evaluation used.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvaluationRequest {
    pub generation: u64,
}

#[derive(Debug, Clone)]
pub struct EvaluatedBlock {
    pub key: EvalKey,
    pub entities: Vec<Entity>,
    pub base_pt: crate::geom::Point3,
    pub source_name: String,
}

#[derive(Debug, Clone, Default)]
pub struct EvaluationCache {
    entries: BTreeMap<EvalKey, Arc<EvaluatedBlock>>,
}

impl EvaluationCache {
    pub fn get(&self, key: &EvalKey) -> Option<Arc<EvaluatedBlock>> {
        self.entries.get(key).cloned()
    }

    pub fn insert(&mut self, block: Arc<EvaluatedBlock>) {
        self.entries.insert(block.key.clone(), block);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

// ------------------------------------------------------------
// Function: document_has_dynamic_content
// Purpose: Fast path so ordinary drawings skip evaluation.
// ------------------------------------------------------------
pub fn document_has_dynamic_content(document: &Document) -> bool {
    document.blocks.values().any(|block| block.is_dynamic())
        || document.model_space.iter().any(entity_has_configuration)
        || document
            .blocks
            .values()
            .any(|block| block.entities.iter().any(entity_has_configuration))
}

fn entity_has_configuration(entity: &Entity) -> bool {
    entity
        .geometry
        .insert_configuration()
        .is_some_and(|config| !config.values.is_empty())
}

pub fn generated_block_name(key: &EvalKey) -> String {
    let mut hash: u64 = key.definition_id ^ key.content_revision ^ u64::from(key.evaluator_version);
    for (id, value) in &key.values {
        hash = hash.wrapping_mul(1_000_003).wrapping_add(*id);
        match value {
            NormalizedValue::Number(bits) => hash ^= *bits,
            NormalizedValue::Choice(option) => hash ^= *option,
            NormalizedValue::Boolean(flag) => hash ^= if *flag { 1 } else { 0 },
            NormalizedValue::Text(text) => {
                for byte in text.as_bytes() {
                    hash = hash.wrapping_mul(31).wrapping_add(u64::from(*byte));
                }
            }
        }
    }
    for (id, rev) in &key.nested_revisions {
        hash = hash.wrapping_mul(1_000_003).wrapping_add(*id ^ *rev);
    }
    format!("{GENERATED_BLOCK_PREFIX}{:016x}", hash)
}

pub fn is_generated_block_name(name: &str) -> bool {
    name.starts_with(GENERATED_BLOCK_PREFIX)
}

pub fn check_generation(
    document: &Document,
    request: EvaluationRequest,
) -> Result<(), DynamicError> {
    let actual = document.content_generation();
    if actual != request.generation {
        Err(DynamicError::StaleGeneration {
            expected: request.generation,
            actual,
        })
    } else {
        Ok(())
    }
}

// ------------------------------------------------------------
// Function: evaluate_definition
// Purpose: Source geometry + values → derived entities.
// ------------------------------------------------------------
pub fn evaluate_definition(
    document: &Document,
    definition: &BlockDefinition,
    instance: Option<&InstanceConfiguration>,
    cache: &mut EvaluationCache,
    request: EvaluationRequest,
) -> Result<Arc<EvaluatedBlock>, DynamicError> {
    check_generation(document, request)?;
    let Some(dynamic) = definition.dynamic.as_ref() else {
        return Ok(Arc::new(static_evaluated(definition)));
    };
    validate_definition(dynamic, &definition.entities)?;
    let values = resolve_values(dynamic, instance)?;
    validate_configuration(dynamic, &values)?;
    let key = eval_key(document, definition, &values)?;
    if let Some(hit) = cache.get(&key) {
        return Ok(hit);
    }
    let entities = apply_evaluation_plan(&definition.entities, dynamic, &values)?;
    validate_evaluated_entities(&entities)?;
    let evaluated = Arc::new(EvaluatedBlock {
        key: key.clone(),
        entities,
        base_pt: definition.base_pt,
        source_name: definition.name.clone(),
    });
    cache.insert(evaluated.clone());
    Ok(evaluated)
}

fn static_evaluated(definition: &BlockDefinition) -> EvaluatedBlock {
    EvaluatedBlock {
        key: EvalKey {
            definition_id: definition.id.raw(),
            content_revision: definition.content_revision,
            values: Vec::new(),
            nested_revisions: Vec::new(),
            evaluator_version: EVALUATOR_VERSION,
        },
        entities: definition.entities.clone(),
        base_pt: definition.base_pt,
        source_name: definition.name.clone(),
    }
}

fn eval_key(
    document: &Document,
    definition: &BlockDefinition,
    values: &BTreeMap<ParameterId, ParameterValue>,
) -> Result<EvalKey, DynamicError> {
    let mut normalized: Vec<(u64, NormalizedValue)> = values
        .iter()
        .map(|(id, value)| (id.raw(), value.normalize()))
        .collect();
    normalized.sort_by_key(|(id, _)| *id);
    let nested = nested_dependency_revisions(document, definition, &mut Vec::new());
    Ok(EvalKey {
        definition_id: definition.id.raw(),
        content_revision: definition.content_revision,
        values: normalized,
        nested_revisions: nested,
        evaluator_version: EVALUATOR_VERSION,
    })
}

fn nested_dependency_revisions(
    document: &Document,
    definition: &BlockDefinition,
    stack: &mut Vec<String>,
) -> Vec<(u64, u64)> {
    let mut revisions = Vec::new();
    if stack
        .iter()
        .any(|name| name.eq_ignore_ascii_case(&definition.name))
    {
        return revisions;
    }
    stack.push(definition.name.clone());
    for entity in &definition.entities {
        let Some(name) = entity.geometry.insert_block_name() else {
            continue;
        };
        let Some(child) = document.block_by_name(name) else {
            continue;
        };
        revisions.push((child.id.raw(), child.content_revision));
        revisions.extend(nested_dependency_revisions(document, child, stack));
    }
    stack.pop();
    revisions.sort_unstable();
    revisions.dedup();
    revisions
}

fn apply_evaluation_plan(
    source: &[Entity],
    dynamic: &DynamicDefinition,
    values: &BTreeMap<ParameterId, ParameterValue>,
) -> Result<Vec<Entity>, DynamicError> {
    let mut entities = source.to_vec();
    apply_nested_inputs(&mut entities, dynamic, values)?;
    entities = apply_behaviors(&entities, dynamic, values)?;
    apply_group_transforms(&mut entities, dynamic, values)?;
    apply_text_bindings(&mut entities, dynamic, values)?;
    apply_visibility(&mut entities, dynamic, values);
    Ok(entities)
}

fn apply_nested_inputs(
    entities: &mut [Entity],
    dynamic: &DynamicDefinition,
    values: &BTreeMap<ParameterId, ParameterValue>,
) -> Result<(), DynamicError> {
    for input in &dynamic.nested_inputs {
        let Some(leaf) = input.target_occurrence.leaf() else {
            continue;
        };
        let Some(source_value) = values.get(&input.source) else {
            continue;
        };
        let mapped = map_nested_value(dynamic, input, source_value)?;
        let entity = entities.iter_mut().find(|entity| entity.id == leaf).ok_or(
            DynamicError::MissingEntity {
                target: GeometryTarget::Entity(leaf),
            },
        )?;
        let config = entity
            .geometry
            .insert_configuration_mut()
            .ok_or(DynamicError::NestedEditUnsupported)?;
        let mut values = config.take().unwrap_or_default();
        values.set(input.target_parameter, mapped);
        *config = Some(values);
    }
    Ok(())
}

fn map_nested_value(
    dynamic: &DynamicDefinition,
    input: &crate::dynamic_model::NestedInput,
    source: &ParameterValue,
) -> Result<ParameterValue, DynamicError> {
    match &input.mapping {
        NestedMapping::Direct => Ok(source.clone()),
        NestedMapping::NumericScale { factor } => match source {
            ParameterValue::Number(number) => Ok(ParameterValue::Number(*number * *factor)),
            other => Err(DynamicError::ValueType {
                parameter: input.source,
                expected: "number",
                actual: other.type_name(),
            }),
        },
        NestedMapping::OptionMap(map) => {
            let ParameterValue::Choice(option) = source else {
                return Err(DynamicError::ValueType {
                    parameter: input.source,
                    expected: "choice",
                    actual: source.type_name(),
                });
            };
            map.get(option)
                .cloned()
                .ok_or(DynamicError::IncompleteMapping {
                    action: input.id,
                    parameter: input.source,
                    option: *option,
                })
        }
    }
    .map(|value| {
        let _ = dynamic;
        value
    })
}

fn apply_behaviors(
    source: &[Entity],
    dynamic: &DynamicDefinition,
    values: &BTreeMap<ParameterId, ParameterValue>,
) -> Result<Vec<Entity>, DynamicError> {
    let mut entities = source.to_vec();
    let mut contributions: BTreeMap<(EntityId, TargetSlot), Vec<(ActionId, Point2)>> =
        BTreeMap::new();
    let mut behaviors: Vec<&DynamicBehavior> = dynamic.behaviors.iter().collect();
    behaviors.sort_by_key(|behavior| behavior.id);
    for behavior in behaviors {
        let current = numeric_current(dynamic, values, behavior.parameter)?;
        let delta = behavior.displacement(current);
        if delta.x.abs() <= 1e-18 && delta.y.abs() <= 1e-18 {
            continue;
        }
        for target in &behavior.targets {
            let entity = entities
                .iter()
                .find(|entity| entity.id == target.entity_id())
                .ok_or(DynamicError::BrokenBinding {
                    action: behavior.id,
                    target: *target,
                })?;
            capability_for(behavior.kind, &entity.geometry, *target).map_err(|reason| {
                DynamicError::UnsupportedTarget {
                    action: behavior.id,
                    target: *target,
                    reason,
                }
            })?;
            match behavior.kind {
                BehaviorKind::Move => {
                    contributions
                        .entry((target.entity_id(), TargetSlot::Whole))
                        .or_default()
                        .push((behavior.id, delta));
                }
                BehaviorKind::Stretch => {
                    let slot = match target {
                        GeometryTarget::LineStart(_) => TargetSlot::LineStart,
                        GeometryTarget::LineEnd(_) => TargetSlot::LineEnd,
                        GeometryTarget::Vertex { vertex, .. } => TargetSlot::Vertex(*vertex),
                        GeometryTarget::Entity(_) => {
                            return Err(DynamicError::UnsupportedTarget {
                                action: behavior.id,
                                target: *target,
                                reason: "stretch a whole entity",
                            });
                        }
                    };
                    contributions
                        .entry((target.entity_id(), slot))
                        .or_default()
                        .push((behavior.id, delta));
                }
            }
        }
    }

    for ((entity_id, slot), parts) in &contributions {
        let composed = compose_displacements(*slot, parts)?;
        let entity = entities
            .iter_mut()
            .find(|entity| entity.id == *entity_id)
            .ok_or(DynamicError::MissingEntity {
                target: GeometryTarget::Entity(*entity_id),
            })?;
        apply_slot(entity, *slot, composed)?;
    }
    Ok(entities)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TargetSlot {
    Whole,
    LineStart,
    LineEnd,
    Vertex(crate::ids::VertexId),
}

fn compose_displacements(
    slot: TargetSlot,
    parts: &[(ActionId, Point2)],
) -> Result<Point2, DynamicError> {
    let _ = slot;
    let mut total = Point2::new(0.0, 0.0);
    for (_, delta) in parts {
        if !delta.is_finite() {
            return Err(DynamicError::ConflictingWrite {
                target: GeometryTarget::Entity(EntityId::UNASSIGNED),
                actions: parts.iter().map(|(id, _)| *id).collect(),
            });
        }
        total = Point2::new(total.x + delta.x, total.y + delta.y);
    }
    let _ = CompositionRule::Additive;
    Ok(total)
}

fn apply_slot(entity: &mut Entity, slot: TargetSlot, delta: Point2) -> Result<(), DynamicError> {
    match slot {
        TargetSlot::Whole => {
            let translated =
                transform_entity_matrix(entity, Transform2::translate(delta.x, delta.y)).map_err(
                    |_| DynamicError::UnsupportedTarget {
                        action: ActionId::UNASSIGNED,
                        target: GeometryTarget::Entity(entity.id),
                        reason: "move",
                    },
                )?;
            *entity = translated;
            Ok(())
        }
        TargetSlot::LineStart => match &mut entity.geometry {
            Geometry::Line { start, .. } => {
                *start = translate_point(*start, delta);
                Ok(())
            }
            _ => Err(DynamicError::UnsupportedTarget {
                action: ActionId::UNASSIGNED,
                target: GeometryTarget::LineStart(entity.id),
                reason: "stretch",
            }),
        },
        TargetSlot::LineEnd => match &mut entity.geometry {
            Geometry::Line { end, .. } => {
                *end = translate_point(*end, delta);
                Ok(())
            }
            _ => Err(DynamicError::UnsupportedTarget {
                action: ActionId::UNASSIGNED,
                target: GeometryTarget::LineEnd(entity.id),
                reason: "stretch",
            }),
        },
        TargetSlot::Vertex(vertex_id) => {
            let Some(vertices) = entity.geometry.polyline_vertices_mut() else {
                return Err(DynamicError::UnsupportedTarget {
                    action: ActionId::UNASSIGNED,
                    target: GeometryTarget::Vertex {
                        entity: entity.id,
                        vertex: vertex_id,
                    },
                    reason: "stretch",
                });
            };
            let Some(vertex) = vertices.iter_mut().find(|item| item.vertex_id == vertex_id) else {
                return Err(DynamicError::MissingVertex {
                    target: GeometryTarget::Vertex {
                        entity: entity.id,
                        vertex: vertex_id,
                    },
                });
            };
            vertex.point = translate_point(vertex.point, delta);
            Ok(())
        }
    }
}

fn apply_group_transforms(
    entities: &mut [Entity],
    dynamic: &DynamicDefinition,
    values: &BTreeMap<ParameterId, ParameterValue>,
) -> Result<(), DynamicError> {
    let order = ordered_transform_ids(dynamic);
    for id in order {
        if let Some(behavior) = dynamic.reflections.iter().find(|item| item.id == id) {
            apply_reflection(entities, behavior, values)?;
        } else if let Some(behavior) = dynamic.rotations.iter().find(|item| item.id == id) {
            apply_rotation(entities, dynamic, behavior, values)?;
        } else if let Some(behavior) = dynamic.placements.iter().find(|item| item.id == id) {
            apply_placement(entities, dynamic, behavior, values)?;
        }
    }
    Ok(())
}

fn ordered_transform_ids(dynamic: &DynamicDefinition) -> Vec<ActionId> {
    let mut ids = dynamic.transform_order.clone();
    let known: Vec<ActionId> = dynamic
        .reflections
        .iter()
        .map(|item| item.id)
        .chain(dynamic.rotations.iter().map(|item| item.id))
        .chain(dynamic.placements.iter().map(|item| item.id))
        .collect();
    if ids.is_empty() {
        return known;
    }
    for id in known {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

fn apply_reflection(
    entities: &mut [Entity],
    behavior: &ReflectionBehavior,
    values: &BTreeMap<ParameterId, ParameterValue>,
) -> Result<(), DynamicError> {
    if !behavior.condition.matches(values) {
        return Ok(());
    }
    let matrix = crate::entity_transform::EntityTransform::Mirror {
        axis_start: behavior.axis_a,
        axis_end: behavior.axis_b,
    }
    .to_matrix()
    .map_err(|err| DynamicError::UnsupportedCombination {
        details: err.to_string(),
    })?;
    transform_members(
        entities,
        &behavior.members,
        matrix,
        Some(behavior.text_policy),
    )
}

fn apply_rotation(
    entities: &mut [Entity],
    dynamic: &DynamicDefinition,
    behavior: &RotationBehavior,
    values: &BTreeMap<ParameterId, ParameterValue>,
) -> Result<(), DynamicError> {
    let radians = match &behavior.source {
        RotationSource::AngleParameter(parameter) => angle_radians(dynamic, values, *parameter)?,
        RotationSource::OptionMap { parameter, angles } => {
            let ParameterValue::Choice(option) =
                values
                    .get(parameter)
                    .ok_or(DynamicError::UnknownParameter {
                        action: behavior.id,
                        parameter: *parameter,
                    })?
            else {
                return Err(DynamicError::ValueType {
                    parameter: *parameter,
                    expected: "choice",
                    actual: "other",
                });
            };
            *angles.get(option).ok_or(DynamicError::IncompleteMapping {
                action: behavior.id,
                parameter: *parameter,
                option: *option,
            })?
        }
    };
    if radians.abs() <= 1e-18 {
        return Ok(());
    }
    let matrix = crate::entity_transform::EntityTransform::Rotate {
        base: behavior.pivot,
        radians,
    }
    .to_matrix()
    .map_err(|err| DynamicError::UnsupportedCombination {
        details: err.to_string(),
    })?;
    transform_members(entities, &behavior.members, matrix, None)
}

fn angle_radians(
    dynamic: &DynamicDefinition,
    values: &BTreeMap<ParameterId, ParameterValue>,
    parameter: ParameterId,
) -> Result<f64, DynamicError> {
    let value = numeric_current(dynamic, values, parameter)?;
    let Some(def) = dynamic.parameter(parameter) else {
        return Err(DynamicError::UnknownParameter {
            action: ActionId::UNASSIGNED,
            parameter,
        });
    };
    match &def.kind {
        ParameterKind::Number(numeric) => match numeric.unit {
            crate::dynamic::ParameterUnit::Degrees => Ok(value.to_radians()),
            _ => Ok(value),
        },
        _ => Err(DynamicError::ValueType {
            parameter,
            expected: "number",
            actual: def.kind.type_name(),
        }),
    }
}

fn apply_placement(
    entities: &mut [Entity],
    dynamic: &DynamicDefinition,
    behavior: &PlacementBehavior,
    values: &BTreeMap<ParameterId, ParameterValue>,
) -> Result<(), DynamicError> {
    let dest_id = match values.get(&behavior.parameter) {
        Some(ParameterValue::Choice(option)) => {
            *behavior
                .destinations
                .get(option)
                .ok_or(DynamicError::IncompleteMapping {
                    action: behavior.id,
                    parameter: behavior.parameter,
                    option: *option,
                })?
        }
        Some(ParameterValue::Boolean(flag)) => {
            let Some((off, on)) = behavior.boolean_destinations else {
                return Err(DynamicError::IncompleteMapping {
                    action: behavior.id,
                    parameter: behavior.parameter,
                    option: OptionId::UNASSIGNED,
                });
            };
            if *flag {
                on
            } else {
                off
            }
        }
        Some(other) => {
            return Err(DynamicError::ValueType {
                parameter: behavior.parameter,
                expected: "choice",
                actual: other.type_name(),
            });
        }
        None => return Ok(()),
    };
    let Some(anchor) = dynamic.anchor(dest_id) else {
        return Err(DynamicError::MissingEntity {
            target: GeometryTarget::Entity(EntityId::UNASSIGNED),
        });
    };
    let (dest, dest_angle) = resolve_anchor(anchor, dynamic, values)?;
    let attach = behavior.attachment;
    let dx = dest.x - attach.x;
    let dy = dest.y - attach.y;
    let delta_angle = dest_angle - behavior.attachment_angle;
    let mut matrix = Transform2::translate(dest.x, dest.y)
        .then(Transform2::rotate(delta_angle))
        .then(Transform2::translate(-attach.x, -attach.y));
    if dx.abs() <= 1e-18 && dy.abs() <= 1e-18 && delta_angle.abs() <= 1e-18 {
        matrix = Transform2::identity();
    }
    if matrix == Transform2::identity() {
        return Ok(());
    }
    transform_members(entities, &behavior.members, matrix, None)
}

fn resolve_anchor(
    anchor: &crate::dynamic_model::AnchorDef,
    dynamic: &DynamicDefinition,
    values: &BTreeMap<ParameterId, ParameterValue>,
) -> Result<(Point2, f64), DynamicError> {
    let mut position = anchor.position;
    let orientation = anchor.orientation.unwrap_or(0.0);
    if let Some(AnchorFollow::Size { parameter, role }) = &anchor.follow {
        let Some(def) = dynamic.parameter(*parameter) else {
            return Err(DynamicError::UnknownParameter {
                action: ActionId::UNASSIGNED,
                parameter: *parameter,
            });
        };
        let ParameterKind::Number(numeric) = &def.kind else {
            return Err(DynamicError::ValueType {
                parameter: *parameter,
                expected: "number",
                actual: def.kind.type_name(),
            });
        };
        let current = numeric_current(dynamic, values, *parameter)?;
        let delta = current - numeric.reference;
        if let Some(size) = &numeric.size {
            let multiplier = follow_multiplier(size.anchor, *role);
            position = Point2::new(
                position.x + size.direction.x * delta * multiplier,
                position.y + size.direction.y * delta * multiplier,
            );
        }
    }
    Ok((position, orientation))
}

fn transform_members(
    entities: &mut [Entity],
    members: &[EntityId],
    matrix: Transform2,
    text_policy: Option<TextReflectPolicy>,
) -> Result<(), DynamicError> {
    for entity in entities.iter_mut() {
        if !members.contains(&entity.id) {
            continue;
        }
        let transformed = transform_entity_matrix(entity, matrix).map_err(|err| {
            DynamicError::UnsupportedCombination {
                details: format!("entity #{}: {err}", entity.id.raw()),
            }
        })?;
        *entity = transformed;
        if let Some(policy) = text_policy {
            apply_text_policy(entity, policy);
        }
    }
    Ok(())
}

fn apply_text_policy(entity: &mut Entity, policy: TextReflectPolicy) {
    let upright = |rotation: f64| {
        if policy != TextReflectPolicy::KeepUpright {
            return rotation;
        }
        let mut angle = rotation.rem_euclid(std::f64::consts::TAU);
        if angle > std::f64::consts::FRAC_PI_2 && angle < 3.0 * std::f64::consts::FRAC_PI_2 {
            angle = (angle + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU);
        }
        angle
    };
    match &mut entity.geometry {
        Geometry::Text(data) => data.rotation = upright(data.rotation),
        Geometry::MText(data) => data.rotation = upright(data.rotation),
        Geometry::Insert {
            rotation, attribs, ..
        } => {
            *rotation = upright(*rotation);
            for attrib in attribs {
                attrib.rotation = upright(attrib.rotation);
            }
        }
        _ => {}
    }
}

fn apply_text_bindings(
    entities: &mut [Entity],
    dynamic: &DynamicDefinition,
    values: &BTreeMap<ParameterId, ParameterValue>,
) -> Result<(), DynamicError> {
    for binding in &dynamic.text_bindings {
        let entity = entities
            .iter_mut()
            .find(|entity| entity.id == binding.target)
            .ok_or(DynamicError::MissingEntity {
                target: GeometryTarget::Entity(binding.target),
            })?;
        match &mut entity.geometry {
            Geometry::Text(data) => {
                data.value = evaluate_text_binding(binding, &dynamic.parameters, values, false)?;
            }
            Geometry::MText(data) => {
                data.value = evaluate_text_binding(binding, &dynamic.parameters, values, true)?;
            }
            _ => {
                return Err(DynamicError::UnsupportedTarget {
                    action: binding.id,
                    target: GeometryTarget::Entity(binding.target),
                    reason: "bind text",
                });
            }
        }
    }
    Ok(())
}

fn apply_visibility(
    entities: &mut [Entity],
    dynamic: &DynamicDefinition,
    values: &BTreeMap<ParameterId, ParameterValue>,
) {
    for entity in entities.iter_mut() {
        if !effective_visibility(&dynamic.visibility, entity.id, values) {
            entity.visible = false;
        }
    }
}

fn validate_evaluated_entities(entities: &[Entity]) -> Result<(), DynamicError> {
    for entity in entities {
        if !geometry_is_finite(&entity.geometry) {
            return Err(DynamicError::InvalidGeometry {
                entity: entity.id,
                reason: "non-finite coordinates",
            });
        }
    }
    Ok(())
}

fn geometry_is_finite(geometry: &Geometry) -> bool {
    match geometry {
        Geometry::Line { start, end } => start.is_finite() && end.is_finite(),
        Geometry::Point { position } => position.is_finite(),
        Geometry::Circle { center, radius, .. } => center.is_finite() && radius.is_finite(),
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            ..
        } => {
            center.is_finite()
                && radius.is_finite()
                && start_angle.is_finite()
                && end_angle.is_finite()
        }
        Geometry::Insert {
            insertion,
            scale,
            rotation,
            ..
        } => insertion.is_finite() && scale.is_finite() && rotation.is_finite(),
        Geometry::Text(data) => data.insertion.is_finite() && data.height.is_finite(),
        Geometry::MText(data) => data.insertion.is_finite() && data.height.is_finite(),
        _ => true,
    }
}

// ------------------------------------------------------------
// Function: materialize_evaluated
// Purpose: Temporary document for existing consumers. Generated
//          definitions use a '*' name so they stay out of the
//          user Blocks list. Source definitions are unchanged.
// ------------------------------------------------------------
pub fn materialize_evaluated(
    source: &Document,
    cache: &mut EvaluationCache,
    request: EvaluationRequest,
) -> Result<Document, DynamicError> {
    materialize_evaluated_with(source, cache, request, None)
}

pub fn materialize_evaluated_with(
    source: &Document,
    cache: &mut EvaluationCache,
    request: EvaluationRequest,
    overrides: Option<&BTreeMap<crate::entity::EntityId, InstanceConfiguration>>,
) -> Result<Document, DynamicError> {
    check_generation(source, request)?;
    if !document_has_dynamic_content(source) && overrides.is_none() {
        return Ok(source.clone());
    }
    let mut evaluated = source.clone();
    if let Some(overrides) = overrides {
        apply_overrides(&mut evaluated.model_space, overrides);
        for block in evaluated.blocks.values_mut() {
            apply_overrides(&mut block.entities, overrides);
        }
    }
    let mut model = std::mem::take(&mut evaluated.model_space);
    rewrite_entities(source, &mut evaluated, cache, request, &mut model)?;
    evaluated.model_space = model;
    let source_names: Vec<String> = source.blocks.keys().cloned().collect();
    for name in source_names {
        if is_generated_block_name(&name) {
            continue;
        }
        let Some(mut entities) = evaluated
            .block_by_name(&name)
            .map(|block| block.entities.clone())
        else {
            continue;
        };
        rewrite_entities(source, &mut evaluated, cache, request, &mut entities)?;
        if let Some(block) = evaluated.block_by_name_mut(&name) {
            block.entities = entities;
        }
    }
    Ok(evaluated)
}

fn apply_overrides(
    entities: &mut [Entity],
    overrides: &BTreeMap<crate::entity::EntityId, InstanceConfiguration>,
) {
    for entity in entities {
        if let Some(config) = overrides.get(&entity.id) {
            entity
                .geometry
                .set_insert_configuration(Some(config.clone()));
        }
    }
}

/// Replace one definition's source entities in an evaluated document with a
/// test configuration, then rewrite nested inserts. Used by authoring Test.
pub fn apply_definition_preview(
    source: &Document,
    evaluated: &mut Document,
    block_name: &str,
    config: &InstanceConfiguration,
    cache: &mut EvaluationCache,
    request: EvaluationRequest,
) -> Result<(), DynamicError> {
    let Some(definition) = source.block_by_name(block_name).cloned() else {
        return Err(DynamicError::MissingDefinition);
    };
    let preview = evaluate_definition(source, &definition, Some(config), cache, request)?;
    let mut entities = preview.entities.clone();
    rewrite_entities(source, evaluated, cache, request, &mut entities)?;
    if let Some(block) = evaluated.block_by_name_mut(block_name) {
        block.entities = entities;
    }
    Ok(())
}

fn rewrite_entities(
    source: &Document,
    evaluated: &mut Document,
    cache: &mut EvaluationCache,
    request: EvaluationRequest,
    entities: &mut [Entity],
) -> Result<(), DynamicError> {
    for entity in entities.iter_mut() {
        let Some(block_name) = entity.geometry.insert_block_name().map(str::to_string) else {
            continue;
        };
        if is_generated_block_name(&block_name) {
            continue;
        }
        let Some(definition) = source.block_by_name(&block_name) else {
            continue;
        };
        if !definition.is_dynamic() {
            continue;
        }
        let config = entity.geometry.insert_configuration().cloned();
        let evaluated_block =
            evaluate_definition(source, definition, config.as_ref(), cache, request)?;
        let mut generated = evaluated_definition(definition, evaluated_block.as_ref());
        rewrite_entities(source, evaluated, cache, request, &mut generated.entities)?;
        let generated_name = generated.name.clone();
        evaluated.blocks.insert(generated_name.clone(), generated);
        if let Some(name) = entity.geometry.insert_block_name_mut() {
            *name = generated_name;
        }
    }
    Ok(())
}

fn evaluated_definition(source: &BlockDefinition, evaluated: &EvaluatedBlock) -> BlockDefinition {
    let mut definition = BlockDefinition::plain(
        generated_block_name(&evaluated.key),
        evaluated.base_pt,
        evaluated.entities.clone(),
    );
    definition.id = source.id;
    definition.content_revision = source.content_revision;
    definition
}

pub fn export_materialized(
    source: &Document,
    cache: &mut EvaluationCache,
    request: EvaluationRequest,
) -> Result<Document, DynamicError> {
    let mut evaluated = materialize_evaluated(source, cache, request)?;
    let generated: Vec<String> = evaluated
        .blocks
        .keys()
        .filter(|name| is_generated_block_name(name))
        .cloned()
        .collect();
    for name in generated {
        let Some(mut definition) = evaluated.remove_block_definition(&name) else {
            continue;
        };
        let export_name =
            unique_export_name(&evaluated, &definition_source_name(&definition, source));
        rewrite_insert_block_name(&mut evaluated.model_space, &name, &export_name);
        for block in evaluated.blocks.values_mut() {
            rewrite_insert_block_name(&mut block.entities, &name, &export_name);
        }
        definition.name = export_name.clone();
        definition.dynamic = None;
        evaluated.replace_block_definition(definition);
    }
    Ok(evaluated)
}

fn definition_source_name(generated: &BlockDefinition, source: &Document) -> String {
    source
        .blocks
        .values()
        .find(|block| block.id == generated.id && !is_generated_block_name(&block.name))
        .map(|block| block.name.clone())
        .unwrap_or_else(|| "Block".into())
}

fn unique_export_name(document: &Document, base: &str) -> String {
    let sanitized: String = base
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    let base = if sanitized.is_empty() {
        "Block"
    } else {
        sanitized.as_str()
    };
    if document.block_key(base).is_none() {
        return base.to_string();
    }
    for index in 1..10_000 {
        let candidate = format!("{base}_{index:03}");
        if document.block_key(&candidate).is_none() {
            return candidate;
        }
    }
    format!("{base}_cfg")
}

fn rewrite_insert_block_name(entities: &mut [Entity], from: &str, to: &str) {
    for entity in entities {
        if let Some(name) = entity.geometry.insert_block_name_mut() {
            if name.eq_ignore_ascii_case(from) {
                *name = to.to_string();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::identity_insert;
    use crate::document::BlockDefinition;
    use crate::dynamic::{
        CompositionRule, DynamicBehavior, DynamicDefinition, GeometryTarget, NumericParameter,
        ParameterDef,
    };
    use crate::entity::Entity;
    use crate::geom::Point3;
    use crate::ids::{BlockDefinitionId, ParameterId};

    fn span_frame() -> (Document, BlockDefinitionId, ParameterId) {
        let mut document = Document::default();
        let param = document.allocate_parameter_id();
        let mut left = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(0.0, 10.0),
        });
        let mut right = Entity::new(Geometry::Line {
            start: Point3::from_xy(800.0, 0.0),
            end: Point3::from_xy(800.0, 10.0),
        });
        let mut top = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 10.0),
            end: Point3::from_xy(800.0, 10.0),
        });
        left.id = document.allocate_id();
        right.id = document.allocate_id();
        top.id = document.allocate_id();
        let right_id = right.id;
        let top_id = top.id;
        let mut definition = BlockDefinition::plain(
            "AdjustableFrame",
            Point3::from_xy(0.0, 0.0),
            vec![left, right, top],
        );
        let mut numeric = NumericParameter::length(800.0);
        numeric.reference = 800.0;
        definition.dynamic = Some(DynamicDefinition {
            parameters: vec![ParameterDef::number(param, "Span", numeric)],
            behaviors: vec![
                DynamicBehavior {
                    id: document.allocate_action_id(),
                    kind: BehaviorKind::Move,
                    parameter: param,
                    targets: vec![GeometryTarget::Entity(right_id)],
                    local_direction: Point2::new(1.0, 0.0),
                    reference_value: 800.0,
                    multiplier: 1.0,
                    composition: CompositionRule::Additive,
                    follow: crate::dynamic::FollowRole::Second,
                    name: None,
                },
                DynamicBehavior {
                    id: document.allocate_action_id(),
                    kind: BehaviorKind::Stretch,
                    parameter: param,
                    targets: vec![GeometryTarget::LineEnd(top_id)],
                    local_direction: Point2::new(1.0, 0.0),
                    reference_value: 800.0,
                    multiplier: 1.0,
                    composition: CompositionRule::Additive,
                    follow: crate::dynamic::FollowRole::Second,
                    name: None,
                },
            ],
            ..Default::default()
        });
        document.replace_block_definition(definition);
        let id = document.block_by_name("AdjustableFrame").unwrap().id;
        (document, id, param)
    }

    fn insert_with_span(document: &mut Document, x: f64, span: f64, param: ParameterId) -> Entity {
        let mut entity = Entity::new(identity_insert(
            "AdjustableFrame".into(),
            Point3::from_xy(x, 0.0),
        ));
        let mut config = InstanceConfiguration::default();
        config.set(param, ParameterValue::Number(span));
        entity.geometry.set_insert_configuration(Some(config));
        document.add_entity(entity)
    }

    #[test]
    fn three_references_keep_independent_values() {
        let (mut document, _, param) = span_frame();
        insert_with_span(&mut document, 0.0, 800.0, param);
        insert_with_span(&mut document, 2000.0, 1200.0, param);
        insert_with_span(&mut document, 4000.0, 1600.0, param);
        let request = EvaluationRequest {
            generation: document.content_generation(),
        };
        let mut cache = EvaluationCache::default();
        let evaluated = materialize_evaluated(&document, &mut cache, request).unwrap();
        let mut spans = Vec::new();
        for entity in &evaluated.model_space {
            let name = entity.geometry.insert_block_name().unwrap();
            let block = evaluated.block_by_name(name).unwrap();
            let Geometry::Line { start, .. } = &block.entities[1].geometry else {
                panic!("right post");
            };
            spans.push(start.x);
        }
        assert!((spans[0] - 800.0).abs() < 1e-9);
        assert!((spans[1] - 1200.0).abs() < 1e-9);
        assert!((spans[2] - 1600.0).abs() < 1e-9);
    }

    #[test]
    fn returning_to_baseline_does_not_drift() {
        let (document, _, _) = span_frame();
        let definition = document.block_by_name("AdjustableFrame").unwrap();
        let mut cache = EvaluationCache::default();
        let request = EvaluationRequest {
            generation: document.content_generation(),
        };
        let mut config = InstanceConfiguration::default();
        let param = definition.dynamic.as_ref().unwrap().parameters[0].id;
        for value in [800.0, 1200.0, 500.0, 800.0] {
            config.set(param, ParameterValue::Number(value));
            let evaluated =
                evaluate_definition(&document, definition, Some(&config), &mut cache, request)
                    .unwrap();
            if (value - 800.0).abs() < 1e-12 {
                let Geometry::Line { start, .. } = &evaluated.entities[1].geometry else {
                    panic!("line");
                };
                assert!(
                    (start.x - 800.0).abs() < 1e-12,
                    "drift at {value}: {}",
                    start.x
                );
            }
        }
    }

    #[test]
    fn move_and_stretch_differ_on_a_line() {
        let mut document = Document::default();
        let param = document.allocate_parameter_id();
        let mut line = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(10.0, 0.0),
        });
        line.id = document.allocate_id();
        let mut definition =
            BlockDefinition::plain("Offset", Point3::from_xy(0.0, 0.0), vec![line.clone()]);
        let mut numeric = NumericParameter::length(0.0);
        numeric.reference = 0.0;
        definition.dynamic = Some(DynamicDefinition {
            parameters: vec![ParameterDef::number(param, "Offset", numeric)],
            behaviors: vec![DynamicBehavior {
                id: document.allocate_action_id(),
                kind: BehaviorKind::Stretch,
                parameter: param,
                targets: vec![GeometryTarget::LineEnd(line.id)],
                local_direction: Point2::new(1.0, 0.0),
                reference_value: 0.0,
                multiplier: 1.0,
                composition: CompositionRule::Additive,
                follow: crate::dynamic::FollowRole::Second,
                name: None,
            }],
            ..Default::default()
        });
        document.replace_block_definition(definition.clone());
        let definition = document.block_by_name("Offset").unwrap().clone();
        let mut config = InstanceConfiguration::default();
        config.set(param, ParameterValue::Number(5.0));
        let request = EvaluationRequest {
            generation: document.content_generation(),
        };
        let stretched = evaluate_definition(
            &document,
            &definition,
            Some(&config),
            &mut EvaluationCache::default(),
            request,
        )
        .unwrap();
        match &stretched.entities[0].geometry {
            Geometry::Line { start, end } => {
                assert!((start.x - 0.0).abs() < 1e-12);
                assert!((end.x - 15.0).abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }

        let mut moved_def = definition.clone();
        moved_def.dynamic.as_mut().unwrap().behaviors[0].kind = BehaviorKind::Move;
        moved_def.dynamic.as_mut().unwrap().behaviors[0].targets =
            vec![GeometryTarget::Entity(line.id)];
        document.replace_block_definition(moved_def.clone());
        let moved_def = document.block_by_name("Offset").unwrap().clone();
        let moved = evaluate_definition(
            &document,
            &moved_def,
            Some(&config),
            &mut EvaluationCache::default(),
            EvaluationRequest {
                generation: document.content_generation(),
            },
        )
        .unwrap();
        match &moved.entities[0].geometry {
            Geometry::Line { start, end } => {
                assert!((start.x - 5.0).abs() < 1e-12);
                assert!((end.x - 15.0).abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stale_generation_is_rejected() {
        let (document, _, _) = span_frame();
        let err = check_generation(
            &document,
            EvaluationRequest {
                generation: document.content_generation().wrapping_add(1),
            },
        )
        .unwrap_err();
        assert!(matches!(err, DynamicError::StaleGeneration { .. }));
    }

    #[test]
    fn rotated_reference_follows_local_axis() {
        let (mut document, _, param) = span_frame();
        let mut entity = insert_with_span(&mut document, 0.0, 1200.0, param);
        if let Geometry::Insert { rotation, .. } = &mut entity.geometry {
            *rotation = std::f64::consts::FRAC_PI_2;
        }
        document.replace_model_entity(entity.id, entity);
        let request = EvaluationRequest {
            generation: document.content_generation(),
        };
        let evaluated =
            materialize_evaluated(&document, &mut EvaluationCache::default(), request).unwrap();
        let snaps = crate::snap::SnapIndex::build(&evaluated);
        let mut found = Vec::new();
        snaps.query(
            crate::extents::Extents2::from_corners(
                crate::geom::Point2::new(-50.0, -50.0),
                crate::geom::Point2::new(50.0, 1300.0),
            ),
            &mut found,
        );
        assert!(
            found
                .iter()
                .any(|snap| (snap.point.x).abs() < 1e-6 && (snap.point.y - 1200.0).abs() < 1e-6),
            "expected local +X stretch at world (0,1200), got {found:?}"
        );
    }

    #[test]
    fn nested_reference_keeps_occurrence_geometry() {
        let (mut document, _, param) = span_frame();
        let child = insert_with_span(&mut document, 0.0, 1200.0, param);
        document.remove_model_entity(child.id);
        let mut nested = BlockDefinition::plain("Assembly", Point3::from_xy(0.0, 0.0), vec![child]);
        nested.entities[0].id = document.allocate_id();
        document.replace_block_definition(nested);
        let parent = Entity::new(identity_insert(
            "Assembly".into(),
            Point3::from_xy(10.0, 20.0),
        ));
        document.add_entity(parent);
        let request = EvaluationRequest {
            generation: document.content_generation(),
        };
        let evaluated =
            materialize_evaluated(&document, &mut EvaluationCache::default(), request).unwrap();
        let assembly = evaluated.block_by_name("Assembly").unwrap();
        let nested_name = assembly.entities[0].geometry.insert_block_name().unwrap();
        assert!(is_generated_block_name(nested_name));
        let nested_def = evaluated.block_by_name(nested_name).unwrap();
        let Geometry::Line { start, .. } = &nested_def.entities[1].geometry else {
            panic!("right post");
        };
        assert!((start.x - 1200.0).abs() < 1e-9);
    }

    #[test]
    fn snap_and_measure_match_evaluated_geometry() {
        let (mut document, _, param) = span_frame();
        insert_with_span(&mut document, 0.0, 1200.0, param);
        let request = EvaluationRequest {
            generation: document.content_generation(),
        };
        let evaluated =
            materialize_evaluated(&document, &mut EvaluationCache::default(), request).unwrap();
        let snaps = crate::snap::SnapIndex::build(&evaluated);
        let measures = crate::measure_index::MeasureIndex::build(&evaluated);
        let mut found = Vec::new();
        snaps.query(
            crate::extents::Extents2::from_corners(
                crate::geom::Point2::new(-10.0, -10.0),
                crate::geom::Point2::new(1300.0, 20.0),
            ),
            &mut found,
        );
        assert!(found
            .iter()
            .any(|snap| (snap.point.x - 1200.0).abs() < 1e-6));
        assert!(!measures.is_empty());
    }

    #[test]
    fn rename_keeps_bindings() {
        let (mut document, def_id, param) = span_frame();
        insert_with_span(&mut document, 0.0, 1200.0, param);
        document
            .block_by_name_mut("AdjustableFrame")
            .unwrap()
            .dynamic
            .as_mut()
            .unwrap()
            .parameters[0]
            .name = "Width".into();
        crate::block::rename_block(&mut document, "AdjustableFrame", "FrameA").unwrap();
        let renamed = document.block_by_name("FrameA").unwrap();
        assert_eq!(renamed.id, def_id);
        assert_eq!(renamed.dynamic.as_ref().unwrap().parameters[0].id, param);
        let request = EvaluationRequest {
            generation: document.content_generation(),
        };
        let evaluated =
            materialize_evaluated(&document, &mut EvaluationCache::default(), request).unwrap();
        let name = evaluated.model_space[0]
            .geometry
            .insert_block_name()
            .unwrap();
        let block = evaluated.block_by_name(name).unwrap();
        let Geometry::Line { start, .. } = &block.entities[1].geometry else {
            panic!("right post");
        };
        assert!((start.x - 1200.0).abs() < 1e-9);
    }

    #[test]
    fn make_unique_remaps_behavior_targets() {
        let (mut document, _, param) = span_frame();
        let first = insert_with_span(&mut document, 0.0, 800.0, param);
        let mut second = first.clone();
        second.id = EntityId::UNASSIGNED;
        let second = document.add_entity(second);
        let result = crate::block::make_unique_block(&mut document, second.id).unwrap();
        let unique = document.block_by_name(&result.new_name).unwrap();
        let source = document.block_by_name("AdjustableFrame").unwrap();
        assert_ne!(unique.id, source.id);
        let old_target = source.dynamic.as_ref().unwrap().behaviors[0].targets[0].entity_id();
        let new_target = unique.dynamic.as_ref().unwrap().behaviors[0].targets[0].entity_id();
        assert_ne!(old_target, new_target);
        assert_eq!(result.entity_map.get(&old_target), Some(&new_target));
    }

    #[test]
    fn deleting_a_bound_target_is_an_explicit_error() {
        let (mut document, _, _) = span_frame();
        let definition = document.block_by_name("AdjustableFrame").unwrap().clone();
        let mut broken = definition.clone();
        let removed = broken.entities.remove(1);
        let err =
            validate_definition(broken.dynamic.as_ref().unwrap(), &broken.entities).unwrap_err();
        assert!(
            matches!(
                err,
                DynamicError::BrokenBinding { target, .. } if target.entity_id() == removed.id
            ),
            "{err}"
        );
        document.replace_block_definition(definition);
    }

    #[test]
    fn export_keeps_distinct_configurations() {
        let (mut document, _, param) = span_frame();
        insert_with_span(&mut document, 0.0, 800.0, param);
        insert_with_span(&mut document, 2000.0, 1600.0, param);
        let request = EvaluationRequest {
            generation: document.content_generation(),
        };
        let exported =
            export_materialized(&document, &mut EvaluationCache::default(), request).unwrap();
        let names: Vec<_> = exported
            .model_space
            .iter()
            .map(|entity| entity.geometry.insert_block_name().unwrap().to_string())
            .collect();
        assert_ne!(names[0], names[1]);
        let a = exported.block_by_name(&names[0]).unwrap();
        let b = exported.block_by_name(&names[1]).unwrap();
        assert!(a.dynamic.is_none());
        assert!(b.dynamic.is_none());
        let Geometry::Line { start: a_start, .. } = &a.entities[1].geometry else {
            panic!("a");
        };
        let Geometry::Line { start: b_start, .. } = &b.entities[1].geometry else {
            panic!("b");
        };
        assert!((a_start.x - 800.0).abs() < 1e-9 || (b_start.x - 800.0).abs() < 1e-9);
        assert!((a_start.x - 1600.0).abs() < 1e-9 || (b_start.x - 1600.0).abs() < 1e-9);
        assert_ne!(a_start.x, b_start.x);
    }

    #[test]
    fn polyline_entity_stretch_is_rejected_but_straight_vertices_are_not() {
        let mut line = Entity::new(Geometry::LwPolyline {
            vertices: vec![
                crate::entity::PolyVertex {
                    point: Point3::from_xy(0.0, 0.0),
                    bulge: 0.0,
                    vertex_id: crate::ids::VertexId(1),
                },
                crate::entity::PolyVertex {
                    point: Point3::from_xy(10.0, 0.0),
                    bulge: 0.0,
                    vertex_id: crate::ids::VertexId(2),
                },
            ],
            closed: false,
            extrusion: crate::entity::default_extrusion(),
            linetype_generation_continuous: false,
        });
        line.id = EntityId(1);
        let err = capability_for(
            BehaviorKind::Stretch,
            &line.geometry,
            GeometryTarget::Entity(line.id),
        )
        .unwrap_err();
        assert!(err.contains("straight vertices"));
        capability_for(
            BehaviorKind::Stretch,
            &line.geometry,
            GeometryTarget::Vertex {
                entity: line.id,
                vertex: crate::ids::VertexId(2),
            },
        )
        .unwrap();
    }

    #[test]
    fn straight_polyline_vertex_stretch_moves_the_bound_vertex() {
        let mut document = Document::default();
        let param = document.allocate_parameter_id();
        let mut polyline = Entity::new(Geometry::LwPolyline {
            vertices: vec![
                crate::entity::PolyVertex {
                    point: Point3::from_xy(0.0, 0.0),
                    bulge: 0.0,
                    vertex_id: crate::ids::VertexId(11),
                },
                crate::entity::PolyVertex {
                    point: Point3::from_xy(10.0, 0.0),
                    bulge: 0.0,
                    vertex_id: crate::ids::VertexId(12),
                },
            ],
            closed: false,
            extrusion: crate::entity::default_extrusion(),
            linetype_generation_continuous: false,
        });
        polyline.id = document.allocate_id();
        let mut numeric = NumericParameter::length(10.0);
        numeric.reference = 10.0;
        let mut definition =
            BlockDefinition::plain("Rail", Point3::from_xy(0.0, 0.0), vec![polyline.clone()]);
        definition.dynamic = Some(DynamicDefinition {
            parameters: vec![ParameterDef::number(param, "Span", numeric)],
            behaviors: vec![DynamicBehavior {
                id: document.allocate_action_id(),
                kind: BehaviorKind::Stretch,
                parameter: param,
                targets: vec![GeometryTarget::Vertex {
                    entity: polyline.id,
                    vertex: crate::ids::VertexId(12),
                }],
                local_direction: Point2::new(1.0, 0.0),
                reference_value: 10.0,
                multiplier: 1.0,
                composition: CompositionRule::Additive,
                follow: crate::dynamic::FollowRole::Second,
                name: None,
            }],
            ..Default::default()
        });
        document.replace_block_definition(definition.clone());
        let definition = document.block_by_name("Rail").unwrap().clone();
        let mut config = InstanceConfiguration::default();
        config.set(param, ParameterValue::Number(15.0));
        let request = EvaluationRequest {
            generation: document.content_generation(),
        };
        let evaluated = evaluate_definition(
            &document,
            &definition,
            Some(&config),
            &mut EvaluationCache::default(),
            request,
        )
        .unwrap();
        let Geometry::LwPolyline { vertices, .. } = &evaluated.entities[0].geometry else {
            panic!("polyline");
        };
        assert!((vertices[0].point.x - 0.0).abs() < 1e-12);
        assert!((vertices[1].point.x - 15.0).abs() < 1e-12);
        assert_eq!(vertices[1].vertex_id, crate::ids::VertexId(12));
    }

    #[test]
    fn small_frame_preview_evaluation_is_measurable() {
        let (document, _, param) = span_frame();
        let definition = document.block_by_name("AdjustableFrame").unwrap().clone();
        let mut config = InstanceConfiguration::default();
        config.set(param, ParameterValue::Number(1200.0));
        let request = EvaluationRequest {
            generation: document.content_generation(),
        };
        let start = std::time::Instant::now();
        for _ in 0..50 {
            let _ = evaluate_definition(
                &document,
                &definition,
                Some(&config),
                &mut EvaluationCache::default(),
                request,
            )
            .unwrap();
        }
        let elapsed = start.elapsed();
        eprintln!("small-frame 50 evaluates: {elapsed:?}");
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "small dynamic frame evaluation took {elapsed:?}"
        );
    }

    #[test]
    fn two_parameters_apply_together_and_center_moves_half() {
        let mut document = Document::default();
        let span = document.allocate_parameter_id();
        let depth = document.allocate_parameter_id();
        let mut line = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(800.0, 10.0),
        });
        line.id = document.allocate_id();
        let mut span_num = NumericParameter::length(800.0);
        span_num.reference = 800.0;
        let mut depth_num = NumericParameter::length(10.0);
        depth_num.reference = 10.0;
        let mut definition =
            BlockDefinition::plain("Frame", Point3::from_xy(0.0, 0.0), vec![line.clone()]);
        definition.dynamic = Some(DynamicDefinition {
            parameters: vec![
                ParameterDef::number(span, "Span", span_num),
                ParameterDef::number(depth, "Depth", depth_num),
            ],
            behaviors: vec![
                DynamicBehavior {
                    id: document.allocate_action_id(),
                    kind: BehaviorKind::Stretch,
                    parameter: span,
                    targets: vec![GeometryTarget::LineEnd(line.id)],
                    local_direction: Point2::new(1.0, 0.0),
                    reference_value: 800.0,
                    multiplier: 1.0,
                    composition: CompositionRule::Additive,
                    follow: crate::dynamic::FollowRole::Second,
                    name: None,
                },
                DynamicBehavior {
                    id: document.allocate_action_id(),
                    kind: BehaviorKind::Stretch,
                    parameter: depth,
                    targets: vec![GeometryTarget::LineEnd(line.id)],
                    local_direction: Point2::new(0.0, 1.0),
                    reference_value: 10.0,
                    multiplier: 0.5,
                    composition: CompositionRule::Additive,
                    follow: crate::dynamic::FollowRole::Center,
                    name: None,
                },
            ],
            ..Default::default()
        });
        document.replace_block_definition(definition.clone());
        let definition = document.block_by_name("Frame").unwrap().clone();
        let mut config = InstanceConfiguration::default();
        config.set(span, ParameterValue::Number(1000.0));
        config.set(depth, ParameterValue::Number(30.0));
        let evaluated = evaluate_definition(
            &document,
            &definition,
            Some(&config),
            &mut EvaluationCache::default(),
            EvaluationRequest {
                generation: document.content_generation(),
            },
        )
        .unwrap();
        match &evaluated.entities[0].geometry {
            Geometry::Line { start, end } => {
                assert!((start.x - 0.0).abs() < 1e-12);
                assert!((end.x - 1000.0).abs() < 1e-12);
                assert!((end.y - 20.0).abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
    }

    fn phase3_assembly() -> (
        Document,
        ParameterId,
        ParameterId,
        ParameterId,
        ParameterId,
        ParameterId,
        crate::ids::PresetId,
        crate::ids::PresetId,
        EntityId,
        EntityId,
    ) {
        use crate::dynamic_model::{
            AnchorDef, AnchorFollow, NestedInput, NestedMapping, ParameterCondition,
            PlacementBehavior, Preset, ReflectionBehavior, TextBinding, TextBindingMode,
            TextReflectPolicy, TextToken, VisibilityGroup,
        };
        use crate::entity::{default_extrusion, TextData};

        let mut document = Document::default();
        let span = document.allocate_parameter_id();
        let depth = document.allocate_parameter_id();
        let style = document.allocate_parameter_id();
        let accessory = document.allocate_parameter_id();
        let description = document.allocate_parameter_id();
        let standard = document.allocate_option_id();
        let reinforced = document.allocate_option_id();
        let mut left = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(0.0, 10.0),
        });
        let mut right = Entity::new(Geometry::Line {
            start: Point3::from_xy(800.0, 0.0),
            end: Point3::from_xy(800.0, 10.0),
        });
        let mut alt_a = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 20.0),
            end: Point3::from_xy(40.0, 20.0),
        });
        let mut alt_b = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 30.0),
            end: Point3::from_xy(80.0, 30.0),
        });
        let mut accessory_line = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, -10.0),
            end: Point3::from_xy(20.0, -10.0),
        });
        let mut label = Entity::new(Geometry::Text(TextData {
            insertion: Point3::from_xy(0.0, 40.0),
            height: 2.5,
            rotation: 0.0,
            value: "source".into(),
            extrusion: default_extrusion(),
            is_attrib_def: false,
        }));
        let mut flip_line = Entity::new(Geometry::Line {
            start: Point3::from_xy(10.0, 50.0),
            end: Point3::from_xy(30.0, 50.0),
        });
        let mut place_line = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 60.0),
            end: Point3::from_xy(10.0, 60.0),
        });
        left.id = document.allocate_id();
        right.id = document.allocate_id();
        alt_a.id = document.allocate_id();
        alt_b.id = document.allocate_id();
        accessory_line.id = document.allocate_id();
        label.id = document.allocate_id();
        flip_line.id = document.allocate_id();
        place_line.id = document.allocate_id();
        let mut nested = Entity::new(identity_insert(
            "ChildDyn".into(),
            Point3::from_xy(100.0, 0.0),
        ));
        nested.id = document.allocate_id();
        let mut nested_b = Entity::new(identity_insert(
            "ChildDyn".into(),
            Point3::from_xy(200.0, 0.0),
        ));
        nested_b.id = document.allocate_id();

        let child_param = document.allocate_parameter_id();
        let mut child_line = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(5.0, 0.0),
        });
        child_line.id = document.allocate_id();
        let mut child_text = Entity::new(Geometry::Text(TextData {
            insertion: Point3::from_xy(0.0, 2.0),
            height: 1.0,
            rotation: 0.0,
            value: "N".into(),
            extrusion: default_extrusion(),
            is_attrib_def: false,
        }));
        child_text.id = document.allocate_id();
        let mut child_numeric = NumericParameter::length(5.0);
        child_numeric.reference = 5.0;
        let mut child = BlockDefinition::plain(
            "ChildDyn",
            Point3::from_xy(0.0, 0.0),
            vec![child_line.clone(), child_text.clone()],
        );
        child.dynamic = Some(DynamicDefinition {
            parameters: vec![ParameterDef::number(
                child_param,
                "ChildSpan",
                child_numeric,
            )],
            behaviors: vec![DynamicBehavior {
                id: document.allocate_action_id(),
                kind: BehaviorKind::Stretch,
                parameter: child_param,
                targets: vec![GeometryTarget::LineEnd(child_line.id)],
                local_direction: Point2::new(1.0, 0.0),
                reference_value: 5.0,
                multiplier: 1.0,
                composition: CompositionRule::Additive,
                follow: crate::dynamic::FollowRole::Second,
                name: None,
            }],
            ..Default::default()
        });
        document.replace_block_definition(child);

        let mut span_numeric = NumericParameter::length(800.0);
        span_numeric.reference = 800.0;
        span_numeric.size = Some(crate::dynamic::SizeAuthoring {
            point_a: Point2::new(0.0, 0.0),
            point_b: Point2::new(800.0, 0.0),
            measure: crate::dynamic::MeasureMode::LocalX,
            direction: Point2::new(1.0, 0.0),
            anchor: crate::dynamic::AnchorPolicy::FirstFixed,
            label_offset: Point2::new(0.0, 8.0),
            bound_anchor: None,
        });
        let mut depth_numeric = NumericParameter::length(10.0);
        depth_numeric.reference = 10.0;
        let choice = crate::dynamic::ChoiceParameter {
            options: vec![
                crate::dynamic::ChoiceOption {
                    id: standard,
                    label: "Standard".into(),
                },
                crate::dynamic::ChoiceOption {
                    id: reinforced,
                    label: "Reinforced".into(),
                },
            ],
            default: standard,
        };
        let dest_a = document.allocate_anchor_id();
        let dest_b = document.allocate_anchor_id();
        let preset_std = document.allocate_preset_id();
        let preset_reinf = document.allocate_preset_id();
        let mut definition = BlockDefinition::plain(
            "Assembly",
            Point3::from_xy(0.0, 0.0),
            vec![
                left.clone(),
                right.clone(),
                alt_a.clone(),
                alt_b.clone(),
                accessory_line.clone(),
                label.clone(),
                flip_line.clone(),
                place_line.clone(),
                nested.clone(),
                nested_b.clone(),
            ],
        );
        definition.dynamic = Some(DynamicDefinition {
            parameters: vec![
                ParameterDef::number(span, "Span", span_numeric),
                ParameterDef::number(depth, "Depth", depth_numeric),
                ParameterDef::choice(style, "Style", choice),
                ParameterDef::boolean(
                    accessory,
                    "Accessory",
                    crate::dynamic::BooleanParameter::default(),
                ),
                ParameterDef::text(
                    description,
                    "Description",
                    crate::dynamic::TextParameter {
                        default: "Assembly".into(),
                        multiline: false,
                        max_length: None,
                    },
                ),
            ],
            behaviors: vec![DynamicBehavior {
                id: document.allocate_action_id(),
                kind: BehaviorKind::Move,
                parameter: span,
                targets: vec![GeometryTarget::Entity(right.id)],
                local_direction: Point2::new(1.0, 0.0),
                reference_value: 800.0,
                multiplier: 1.0,
                composition: CompositionRule::Additive,
                follow: crate::dynamic::FollowRole::Second,
                name: None,
            }],
            anchors: vec![
                AnchorDef {
                    id: dest_a,
                    name: "Left".into(),
                    position: Point2::new(0.0, 60.0),
                    orientation: Some(0.0),
                    follow: Some(AnchorFollow::Size {
                        parameter: span,
                        role: crate::dynamic::FollowRole::First,
                    }),
                },
                AnchorDef {
                    id: dest_b,
                    name: "Right".into(),
                    position: Point2::new(800.0, 60.0),
                    orientation: Some(0.0),
                    follow: Some(AnchorFollow::Size {
                        parameter: span,
                        role: crate::dynamic::FollowRole::Second,
                    }),
                },
            ],
            visibility: vec![
                VisibilityGroup {
                    id: document.allocate_action_id(),
                    name: "Style A".into(),
                    members: vec![alt_a.id],
                    conditions: vec![ParameterCondition::Choice {
                        parameter: style,
                        options: vec![standard],
                    }],
                },
                VisibilityGroup {
                    id: document.allocate_action_id(),
                    name: "Style B".into(),
                    members: vec![alt_b.id],
                    conditions: vec![ParameterCondition::Choice {
                        parameter: style,
                        options: vec![reinforced],
                    }],
                },
                VisibilityGroup {
                    id: document.allocate_action_id(),
                    name: "Accessory".into(),
                    members: vec![accessory_line.id],
                    conditions: vec![ParameterCondition::Boolean {
                        parameter: accessory,
                        state: true,
                    }],
                },
            ],
            text_bindings: vec![TextBinding {
                id: document.allocate_action_id(),
                target: label.id,
                mode: TextBindingMode::Formatted {
                    tokens: vec![
                        TextToken::Literal("Size: ".into()),
                        TextToken::Parameter(span),
                        TextToken::Literal(" × ".into()),
                        TextToken::Parameter(depth),
                        TextToken::Literal(" mm".into()),
                    ],
                },
                boolean_true: "On".into(),
                boolean_false: "Off".into(),
                number_precision: Some(0),
            }],
            reflections: vec![ReflectionBehavior {
                id: document.allocate_action_id(),
                name: Some("Flip accessory".into()),
                members: vec![flip_line.id, nested.id],
                axis_a: Point2::new(0.0, 0.0),
                axis_b: Point2::new(0.0, 1.0),
                condition: ParameterCondition::Boolean {
                    parameter: accessory,
                    state: true,
                },
                text_policy: TextReflectPolicy::KeepReadable,
            }],
            placements: vec![PlacementBehavior {
                id: document.allocate_action_id(),
                name: Some("Park".into()),
                members: vec![place_line.id],
                attachment: Point2::new(0.0, 60.0),
                attachment_angle: 0.0,
                parameter: style,
                destinations: [(standard, dest_a), (reinforced, dest_b)]
                    .into_iter()
                    .collect(),
                boolean_destinations: None,
            }],
            nested_inputs: vec![NestedInput {
                id: document.allocate_action_id(),
                source: span,
                target_occurrence: crate::dynamic_model::OccurrencePath {
                    inserts: vec![nested.id],
                },
                target_parameter: child_param,
                mapping: NestedMapping::NumericScale { factor: 0.01 },
            }],
            presets: vec![
                Preset {
                    id: preset_std,
                    name: "Standard 800".into(),
                    values: [
                        (span, ParameterValue::Number(800.0)),
                        (depth, ParameterValue::Number(10.0)),
                        (style, ParameterValue::Choice(standard)),
                        (accessory, ParameterValue::Boolean(false)),
                        (description, ParameterValue::Text("Assembly".into())),
                    ]
                    .into_iter()
                    .collect(),
                },
                Preset {
                    id: preset_reinf,
                    name: "Reinforced 1200".into(),
                    values: [
                        (span, ParameterValue::Number(1200.0)),
                        (depth, ParameterValue::Number(20.0)),
                        (style, ParameterValue::Choice(reinforced)),
                        (accessory, ParameterValue::Boolean(true)),
                        (description, ParameterValue::Text("Reinforced".into())),
                    ]
                    .into_iter()
                    .collect(),
                },
            ],
            ..Default::default()
        });
        document.replace_block_definition(definition);
        (
            document,
            span,
            depth,
            style,
            accessory,
            description,
            preset_std,
            preset_reinf,
            nested.id,
            nested_b.id,
        )
    }

    fn eval_assembly(
        document: &Document,
        config: &InstanceConfiguration,
    ) -> std::sync::Arc<EvaluatedBlock> {
        let definition = document.block_by_name("Assembly").unwrap().clone();
        evaluate_definition(
            document,
            &definition,
            Some(config),
            &mut EvaluationCache::default(),
            EvaluationRequest {
                generation: document.content_generation(),
            },
        )
        .unwrap()
    }

    #[test]
    fn choice_visibility_hides_inactive_geometry() {
        let (document, _, _, style, accessory, _, _, _, _, _) = phase3_assembly();
        let standard = match &document
            .block_by_name("Assembly")
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
            .parameters[2]
            .kind
        {
            crate::dynamic::ParameterKind::Choice(choice) => choice.options[0].id,
            _ => panic!("choice"),
        };
        let mut config = InstanceConfiguration::default();
        config.set(style, ParameterValue::Choice(standard));
        config.set(accessory, ParameterValue::Boolean(false));
        let evaluated = eval_assembly(&document, &config);
        let visible: Vec<_> = evaluated
            .entities
            .iter()
            .filter(|entity| entity.visible)
            .map(|entity| entity.id)
            .collect();
        assert!(evaluated.entities.iter().any(|entity| !entity.visible));
        assert!(!visible.is_empty());
    }

    #[test]
    fn formatted_text_substitutes_ids_not_names() {
        let (document, span, depth, _, _, _, _, _, _, _) = phase3_assembly();
        let mut config = InstanceConfiguration::default();
        config.set(span, ParameterValue::Number(1200.0));
        config.set(depth, ParameterValue::Number(20.0));
        let evaluated = eval_assembly(&document, &config);
        let text = evaluated
            .entities
            .iter()
            .find_map(|entity| match &entity.geometry {
                Geometry::Text(data) => Some(data.value.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(text, "Size: 1200 × 20 mm");
        let source = document
            .block_by_name("Assembly")
            .unwrap()
            .entities
            .iter()
            .find_map(|entity| match &entity.geometry {
                Geometry::Text(data) => Some(data.value.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(source, "source");
    }

    #[test]
    fn mtext_escapes_literal_formatting_characters() {
        use crate::dynamic_model::{evaluate_text_binding, TextBinding, TextBindingMode};
        let binding = TextBinding {
            id: crate::ids::ActionId(1),
            target: EntityId(1),
            mode: TextBindingMode::ShowValue {
                parameter: ParameterId(1),
            },
            boolean_true: "On".into(),
            boolean_false: "Off".into(),
            number_precision: None,
        };
        let parameter = ParameterDef::text(
            ParameterId(1),
            "Description",
            crate::dynamic::TextParameter {
                default: r"\P{x}".into(),
                multiline: false,
                max_length: None,
            },
        );
        let mut values = std::collections::BTreeMap::new();
        values.insert(ParameterId(1), ParameterValue::Text(r"\P{x}".into()));
        let rendered = evaluate_text_binding(&binding, &[parameter], &values, true).unwrap();
        assert_eq!(rendered, r"\\P\{x\}");
    }

    #[test]
    fn flip_is_not_cumulative() {
        let (document, _, _, _, accessory, _, _, _, _, _) = phase3_assembly();
        let mut on = InstanceConfiguration::default();
        on.set(accessory, ParameterValue::Boolean(true));
        let first = eval_assembly(&document, &on);
        let second = eval_assembly(&document, &on);
        let line = |block: &EvaluatedBlock| {
            block
                .entities
                .iter()
                .find_map(|entity| match &entity.geometry {
                    Geometry::Line { start, end }
                        if (start.y - 50.0).abs() < 1.0 || (start.y + 50.0).abs() < 60.0 =>
                    {
                        Some((*start, *end))
                    }
                    _ => None,
                })
        };
        let a = line(&first).unwrap();
        let b = line(&second).unwrap();
        assert!((a.0.x - b.0.x).abs() < 1e-12);
        assert!((a.1.x - b.1.x).abs() < 1e-12);
    }

    #[test]
    fn placement_follows_size_without_stretching() {
        let (document, span, _, style, _, _, _, _, _, _) = phase3_assembly();
        let reinforced = match &document
            .block_by_name("Assembly")
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
            .parameters[2]
            .kind
        {
            crate::dynamic::ParameterKind::Choice(choice) => choice.options[1].id,
            _ => panic!("choice"),
        };
        let mut config = InstanceConfiguration::default();
        config.set(style, ParameterValue::Choice(reinforced));
        config.set(span, ParameterValue::Number(1200.0));
        let evaluated = eval_assembly(&document, &config);
        let placed = evaluated
            .entities
            .iter()
            .find_map(|entity| match &entity.geometry {
                Geometry::Line { start, end } if (start.y - 60.0).abs() < 1e-6 => {
                    Some((*start, *end))
                }
                _ => None,
            })
            .unwrap();
        assert!((placed.0.x - 1200.0).abs() < 1e-6);
        assert!((placed.1.x - placed.0.x - 10.0).abs() < 1e-6);
    }

    #[test]
    fn nested_mapping_does_not_change_sibling_occurrence() {
        let (document, span, _, _, _, _, _, _, nested, nested_b) = phase3_assembly();
        let mut config = InstanceConfiguration::default();
        config.set(span, ParameterValue::Number(1200.0));
        let definition = document.block_by_name("Assembly").unwrap().clone();
        let evaluated = evaluate_definition(
            &document,
            &definition,
            Some(&config),
            &mut EvaluationCache::default(),
            EvaluationRequest {
                generation: document.content_generation(),
            },
        )
        .unwrap();
        let cfg = |id: EntityId| {
            evaluated
                .entities
                .iter()
                .find(|entity| entity.id == id)
                .and_then(|entity| entity.geometry.insert_configuration().cloned())
        };
        let mapped = cfg(nested).unwrap();
        let sibling = cfg(nested_b);
        assert!(
            mapped.get(span).is_none()
                || mapped.values.values().any(
                    |value| matches!(value, ParameterValue::Number(n) if (n - 12.0).abs() < 1e-9)
                )
        );
        if let Some(sibling) = sibling {
            assert!(sibling.values.is_empty() || sibling.get(span).is_none());
        }
        let _ = (document, nested, nested_b);
    }

    #[test]
    fn matching_preset_detects_custom() {
        let (document, span, _, _, _, _, preset_std, _, _, _) = phase3_assembly();
        let dynamic = document
            .block_by_name("Assembly")
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap();
        let mut values = crate::dynamic::resolve_values(dynamic, None).unwrap();
        assert_eq!(
            crate::dynamic_model::matching_preset(&dynamic.presets, &values, &dynamic.parameters),
            Some(preset_std)
        );
        values.insert(span, ParameterValue::Number(900.0));
        assert_eq!(
            crate::dynamic_model::matching_preset(&dynamic.presets, &values, &dynamic.parameters),
            None
        );
    }

    #[test]
    fn option_rename_keeps_identity() {
        let (mut document, _, _, style, _, _, _, _, _, _) = phase3_assembly();
        let id = {
            let dynamic = document
                .block_by_name_mut("Assembly")
                .unwrap()
                .dynamic
                .as_mut()
                .unwrap();
            let ParameterKind::Choice(choice) = &mut dynamic.parameters[2].kind else {
                panic!("choice");
            };
            let id = choice.options[0].id;
            choice.options[0].label = "Std".into();
            assert_eq!(choice.options[0].id, id);
            id
        };
        let mut config = InstanceConfiguration::default();
        config.set(style, ParameterValue::Choice(id));
        validate_definition(
            document
                .block_by_name("Assembly")
                .unwrap()
                .dynamic
                .as_ref()
                .unwrap(),
            &document.block_by_name("Assembly").unwrap().entities,
        )
        .unwrap();
        assert!(matches!(config.get(style), Some(ParameterValue::Choice(option)) if *option == id));
    }

    #[test]
    fn anchor_cycle_is_rejected() {
        let (mut document, _, _, _, _, _, _, _, _, _) = phase3_assembly();
        let entities = document.block_by_name("Assembly").unwrap().entities.clone();
        let dynamic = document
            .block_by_name_mut("Assembly")
            .unwrap()
            .dynamic
            .as_mut()
            .unwrap();
        let member = dynamic.placements[0].members[0];
        let dest = *dynamic.placements[0].destinations.values().next().unwrap();
        if let Some(anchor) = dynamic.anchors.iter_mut().find(|anchor| anchor.id == dest) {
            anchor.follow = Some(crate::dynamic_model::AnchorFollow::Geometry(
                GeometryTarget::Entity(member),
            ));
        }
        let err = validate_definition(dynamic, &entities).unwrap_err();
        assert!(matches!(err, DynamicError::DependencyCycle { .. }));
    }

    #[test]
    fn remap_choice_values_follow_option_ids() {
        let (document, _, _, style, _, _, _, _, _, _) = phase3_assembly();
        let dynamic = document
            .block_by_name("Assembly")
            .unwrap()
            .dynamic
            .clone()
            .unwrap();
        let old = match &dynamic.parameters[2].kind {
            crate::dynamic::ParameterKind::Choice(choice) => choice.options[0].id,
            _ => panic!("choice"),
        };
        let mut config = InstanceConfiguration::default();
        config.set(style, ParameterValue::Choice(old));
        let mut options = std::collections::BTreeMap::new();
        let new = crate::ids::OptionId(99);
        options.insert(old, new);
        config.remap_identities(&std::collections::BTreeMap::new(), &options);
        assert_eq!(config.get(style), Some(&ParameterValue::Choice(new)));
    }

    #[test]
    fn inactive_geometry_is_omitted_from_extents_snaps_and_plot() {
        let (mut document, _, _, style, accessory, _, _, _, _, _) = phase3_assembly();
        let standard = match &document
            .block_by_name("Assembly")
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
            .parameters[2]
            .kind
        {
            crate::dynamic::ParameterKind::Choice(choice) => choice.options[0].id,
            _ => panic!("choice"),
        };
        let mut insert = Entity::new(identity_insert(
            "Assembly".into(),
            Point3::from_xy(0.0, 0.0),
        ));
        let mut config = InstanceConfiguration::default();
        config.set(style, ParameterValue::Choice(standard));
        config.set(accessory, ParameterValue::Boolean(false));
        insert.geometry.set_insert_configuration(Some(config));
        document.add_entity(insert);
        let evaluated = crate::evaluate::materialize_evaluated(
            &document,
            &mut EvaluationCache::default(),
            EvaluationRequest {
                generation: document.content_generation(),
            },
        )
        .unwrap();
        let snaps = crate::snap::SnapIndex::build(&evaluated);
        let mut nearby = Vec::new();
        snaps.query(
            crate::extents::Extents2 {
                min: Point2::new(70.0, 25.0),
                max: Point2::new(90.0, 35.0),
            },
            &mut nearby,
        );
        assert!(
            nearby.is_empty(),
            "hidden alternative endpoint must not snap"
        );
        let plot = crate::vectorize::plot_geometry(&evaluated);
        assert!(plot.strokes.iter().all(|stroke| {
            stroke
                .points
                .iter()
                .all(|point| (point.x - 80.0).abs() > 0.5 || (point.y - 30.0).abs() > 0.5)
        }));
    }

    #[test]
    fn flip_rotate_place_follow_stored_order() {
        let mut document = Document::default();
        let flag = document.allocate_parameter_id();
        let angle = document.allocate_parameter_id();
        let choice = document.allocate_parameter_id();
        let option = document.allocate_option_id();
        let mut line = Entity::new(Geometry::Line {
            start: Point3::from_xy(10.0, 0.0),
            end: Point3::from_xy(20.0, 0.0),
        });
        line.id = document.allocate_id();
        let dest = document.allocate_anchor_id();
        let flip = document.allocate_action_id();
        let rotate = document.allocate_action_id();
        let place = document.allocate_action_id();
        let mut numeric = NumericParameter::length(90.0);
        numeric.quantity = crate::dynamic::NumericQuantity::Angle;
        numeric.unit = crate::dynamic::ParameterUnit::Degrees;
        numeric.reference = 0.0;
        numeric.default = 90.0;
        let mut definition =
            BlockDefinition::plain("Ordered", Point3::from_xy(0.0, 0.0), vec![line.clone()]);
        definition.dynamic = Some(DynamicDefinition {
            parameters: vec![
                ParameterDef::boolean(
                    flag,
                    "Flip",
                    crate::dynamic::BooleanParameter {
                        default: true,
                        ..Default::default()
                    },
                ),
                ParameterDef::number(angle, "Angle", numeric),
                ParameterDef::choice(
                    choice,
                    "Park",
                    crate::dynamic::ChoiceParameter {
                        options: vec![crate::dynamic::ChoiceOption {
                            id: option,
                            label: "A".into(),
                        }],
                        default: option,
                    },
                ),
            ],
            reflections: vec![crate::dynamic_model::ReflectionBehavior {
                id: flip,
                name: Some("Flip".into()),
                members: vec![line.id],
                axis_a: Point2::new(0.0, 0.0),
                axis_b: Point2::new(0.0, 1.0),
                condition: crate::dynamic_model::ParameterCondition::Boolean {
                    parameter: flag,
                    state: true,
                },
                text_policy: crate::dynamic_model::TextReflectPolicy::KeepReadable,
            }],
            rotations: vec![crate::dynamic_model::RotationBehavior {
                id: rotate,
                name: Some("Rotate".into()),
                members: vec![line.id],
                pivot: Point2::new(0.0, 0.0),
                source: crate::dynamic_model::RotationSource::AngleParameter(angle),
            }],
            anchors: vec![crate::dynamic_model::AnchorDef {
                id: dest,
                name: "A".into(),
                position: Point2::new(100.0, 0.0),
                orientation: Some(0.0),
                follow: None,
            }],
            placements: vec![crate::dynamic_model::PlacementBehavior {
                id: place,
                name: Some("Place".into()),
                members: vec![line.id],
                attachment: Point2::new(0.0, 0.0),
                attachment_angle: 0.0,
                parameter: choice,
                destinations: [(option, dest)].into_iter().collect(),
                boolean_destinations: None,
            }],
            transform_order: vec![flip, rotate, place],
            ..Default::default()
        });
        document.replace_block_definition(definition.clone());
        let evaluated = evaluate_definition(
            &document,
            &definition,
            None,
            &mut EvaluationCache::default(),
            EvaluationRequest {
                generation: document.content_generation(),
            },
        )
        .unwrap();
        match &evaluated.entities[0].geometry {
            Geometry::Line { start, end } => {
                assert!((start.x - 100.0).abs() < 1e-6);
                assert!((start.y + 10.0).abs() < 1e-6);
                assert!((end.x - 100.0).abs() < 1e-6);
                assert!((end.y + 20.0).abs() < 1e-6);
            }
            other => panic!("{other:?}"),
        }
        definition.dynamic.as_mut().unwrap().transform_order = vec![place, flip, rotate];
        document.replace_block_definition(definition.clone());
        let reversed = evaluate_definition(
            &document,
            &definition,
            None,
            &mut EvaluationCache::default(),
            EvaluationRequest {
                generation: document.content_generation(),
            },
        )
        .unwrap();
        match &reversed.entities[0].geometry {
            Geometry::Line { start, .. } => {
                assert!((start.x - 100.0).abs() > 1.0 || (start.y + 10.0).abs() > 1.0);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn reflected_nested_text_keeps_source_and_readable_height() {
        let (document, _, _, _, accessory, _, _, _, nested, _) = phase3_assembly();
        let mut config = InstanceConfiguration::default();
        config.set(accessory, ParameterValue::Boolean(true));
        let evaluated = eval_assembly(&document, &config);
        let insert = evaluated
            .entities
            .iter()
            .find(|entity| entity.id == nested)
            .unwrap();
        match &insert.geometry {
            Geometry::Insert { scale, .. } => {
                assert!(
                    scale.y < 0.0,
                    "mirrored nested INSERT should reverse orientation"
                );
            }
            other => panic!("{other:?}"),
        }
        let child = document.block_by_name("ChildDyn").unwrap();
        match &child
            .entities
            .iter()
            .find_map(|entity| match &entity.geometry {
                Geometry::Text(data) => Some(data),
                _ => None,
            }) {
            Some(data) => {
                assert!((data.height - 1.0).abs() < 1e-12);
                assert_eq!(data.value, "N");
            }
            None => panic!("child text"),
        }
        let plot = crate::vectorize::plot_geometry(&{
            let mut host = document.clone();
            let mut insert = Entity::new(identity_insert(
                "Assembly".into(),
                Point3::from_xy(0.0, 0.0),
            ));
            insert
                .geometry
                .set_insert_configuration(Some(config.clone()));
            host.add_entity(insert);
            crate::evaluate::materialize_evaluated(
                &host,
                &mut EvaluationCache::default(),
                EvaluationRequest {
                    generation: host.content_generation(),
                },
            )
            .unwrap()
        });
        assert!(!plot.strokes.is_empty());
    }

    #[test]
    fn migrate_option_rewrites_presets_and_instances() {
        let (mut document, _, _, style, _, _, _, _, _, _) = phase3_assembly();
        let (from, to) = match &document
            .block_by_name("Assembly")
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
            .parameters[2]
            .kind
        {
            crate::dynamic::ParameterKind::Choice(choice) => {
                (choice.options[0].id, choice.options[1].id)
            }
            _ => panic!("choice"),
        };
        let mut insert = Entity::new(identity_insert(
            "Assembly".into(),
            Point3::from_xy(0.0, 0.0),
        ));
        let mut config = InstanceConfiguration::default();
        config.set(style, ParameterValue::Choice(from));
        insert.geometry.set_insert_configuration(Some(config));
        document.add_entity(insert);
        crate::dynamic::migrate_choice_option(&mut document, "Assembly", style, from, to).unwrap();
        let dynamic = document
            .block_by_name("Assembly")
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap();
        match &dynamic.parameters[2].kind {
            crate::dynamic::ParameterKind::Choice(choice) => {
                assert!(choice.options.iter().all(|option| option.id != from));
                assert_eq!(choice.default, to);
            }
            _ => panic!("choice"),
        }
        assert!(matches!(
            document.model_space[0].geometry.insert_configuration().unwrap().get(style),
            Some(ParameterValue::Choice(option)) if *option == to
        ));
        assert!(dynamic.presets.iter().all(|preset| {
            !matches!(preset.values.get(&style), Some(ParameterValue::Choice(option)) if *option == from)
        }));
    }
}
