//! Evaluate dynamic block definitions from source geometry and instance values.
//!
//! Every evaluation starts from the definition's source entities. The previous
//! evaluated result is never used as the next input. Output is disposable
//! derived state and must not replace the source definition.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::document::{BlockDefinition, Document};
use crate::dynamic::{
    capability_for, numeric_current, resolve_values, translate_point, validate_definition,
    BehaviorKind, CompositionRule, DynamicBehavior, DynamicDefinition, DynamicError,
    GeometryTarget, InstanceConfiguration, NormalizedValue, ParameterValue, EVALUATOR_VERSION,
};
use crate::entity::{Entity, EntityId, Geometry};
use crate::entity_transform::transform_entity_matrix;
use crate::geom::Point2;
use crate::ids::{ActionId, ParameterId};
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
    let key = eval_key(document, definition, &values)?;
    if let Some(hit) = cache.get(&key) {
        return Ok(hit);
    }
    let entities = apply_behaviors(&definition.entities, dynamic, &values)?;
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
            entity.geometry.set_insert_configuration(Some(config.clone()));
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
                },
            ],
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
            }],
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
            found.iter().any(|snap| (snap.point.x).abs() < 1e-6 && (snap.point.y - 1200.0).abs() < 1e-6),
            "expected local +X stretch at world (0,1200), got {found:?}"
        );
    }

    #[test]
    fn nested_reference_keeps_occurrence_geometry() {
        let (mut document, _, param) = span_frame();
        let child = insert_with_span(&mut document, 0.0, 1200.0, param);
        document.remove_model_entity(child.id);
        let mut nested = BlockDefinition::plain(
            "Assembly",
            Point3::from_xy(0.0, 0.0),
            vec![child],
        );
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
        assert!(found.iter().any(|snap| (snap.point.x - 1200.0).abs() < 1e-6));
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
        let err = validate_definition(broken.dynamic.as_ref().unwrap(), &broken.entities)
            .unwrap_err();
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
    fn polyline_vertex_stretch_is_rejected() {
        let mut line = Entity::new(Geometry::LwPolyline {
            vertices: vec![
                crate::entity::PolyVertex {
                    point: Point3::from_xy(0.0, 0.0),
                    bulge: 0.0,
                },
                crate::entity::PolyVertex {
                    point: Point3::from_xy(10.0, 0.0),
                    bulge: 0.0,
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
        assert!(err.contains("durable vertex identity"));
    }
}
