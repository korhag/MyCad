//! AutoCAD dynamic blocks, read from a DWG and mapped onto EntoCAD's model.
//!
//! The definition block keeps every visibility state. Each modified INSERT
//! points at an anonymous `*U` block, which is the geometry AutoCAD already
//! evaluated. This module rebuilds the parameters, checks that evaluating
//! them reproduces that `*U` block, and only then retargets the INSERT.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::{c_void, CString};
use std::os::raw::c_char;

use cad_core::{
    evaluate_definition, fingerprint_visible_entities, normalize_direction, visible_geometry_key,
    ActionId, BehaviorKind, BooleanParameter, ChoiceOption, ChoiceParameter, CompositionRule,
    Document, DynamicBehavior, DynamicDefinition, Entity, EntityId, EntityTransform,
    EvaluationCache, EvaluationRequest, FollowRole, Geometry, GeometryTarget,
    InstanceConfiguration, NumericDomain, NumericParameter, NumericQuantity, ParameterCondition,
    ParameterDef, ParameterId, ParameterKind, ParameterUnit, ParameterValue, Point2,
    ReflectionBehavior, RotationBehavior, RotationSource, StepOrigin, StepPolicy,
    TextReflectPolicy, VisibilityGroup,
};

use crate::convert::{block_record_name, resolve_block_name};
use crate::dynapi::{
    embedded_object, get_array_field, get_common_field, get_field, get_utf8_field, object_dxfname,
    object_fixedtype, Point3D,
};

const MAX_DYNAMIC_NODES: usize = 256;
const MAX_DYNAMIC_STATES: usize = 64;
const MAX_DYNAMIC_HANDLES: usize = 65_536;
const MAX_STRETCH_INDEXES: usize = 256;
const MAX_DICT_ITEMS: usize = 128;
const MAX_DICT_DEPTH: u32 = 4;
const MAX_NAME_CHARS: usize = 256;
const MAX_DYNAMIC_EVALUATIONS: usize = 256;
const MAX_CONFIG_TRIALS: usize = 32;
const MAX_GEOMETRY_POINTS: usize = 4_096;
const MAX_REPORT_LINES: usize = 200;
const VALUE_SET_MIN: u32 = 1;
const VALUE_SET_MAX: u32 = 2;
const VALUE_SET_INCREMENT: u32 = 4;
const VALUE_SET_LIST: u32 = 8;

// ------------------------------------------------------------
// Intermediate graph
// ------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct AcadGraph {
    pub block_name: String,
    pub parameters: Vec<AcadParam>,
    pub actions: Vec<AcadAction>,
    pub unsupported: Vec<String>,
    pub unresolved: usize,
}

#[derive(Debug, Clone)]
pub(crate) enum AcadParam {
    Visibility {
        name: String,
        show: bool,
        states: Vec<AcadVisibilityState>,
    },
    Linear {
        name: String,
        show: bool,
        base: Point2,
        end: Point2,
        default_distance: f64,
        current_distance: f64,
        value_set: AcadValueSet,
    },
    Flip {
        name: String,
        show: bool,
        axis_a: Point2,
        axis_b: Point2,
        base_label: String,
        flipped_label: String,
    },
    Rotation {
        name: String,
        show: bool,
        angle_deg: f64,
        current_deg: f64,
        value_set: AcadValueSet,
    },
    /// AutoCAD point parameter. EntoCAD stores it as two distances, Position X
    /// and Position Y, so the existing move and stretch math can follow both axes.
    Point {
        name: String,
        show: bool,
        base: Point2,
        current: Point2,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct AcadVisibilityState {
    pub name: String,
    pub entities: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AcadValueSet {
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub increment: Option<f64>,
    pub list: Vec<f64>,
}

#[derive(Debug, Clone)]
pub(crate) enum AcadAction {
    Stretch {
        parameter: String,
        offset_x: f64,
        offset_y: f64,
        connection: String,
        targets: Vec<AcadStretchTarget>,
    },
    Move {
        parameter: String,
        offset_x: f64,
        offset_y: f64,
        connection: String,
        entities: Vec<usize>,
    },
    Flip {
        parameter: String,
        entities: Vec<usize>,
    },
    Rotate {
        parameter: String,
        pivot: Point2,
        angle_offset: f64,
        entities: Vec<usize>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct AcadStretchTarget {
    pub entity: usize,
    pub indexes: Vec<u32>,
}

impl AcadParam {
    fn name(&self) -> &str {
        match self {
            Self::Visibility { name, .. }
            | Self::Linear { name, .. }
            | Self::Flip { name, .. }
            | Self::Rotation { name, .. }
            | Self::Point { name, .. } => name,
        }
    }

    fn show(&self) -> bool {
        match self {
            Self::Visibility { show, .. }
            | Self::Linear { show, .. }
            | Self::Flip { show, .. }
            | Self::Rotation { show, .. }
            | Self::Point { show, .. } => *show,
        }
    }
}

impl AcadAction {
    fn parameter(&self) -> &str {
        match self {
            Self::Stretch { parameter, .. }
            | Self::Move { parameter, .. }
            | Self::Flip { parameter, .. }
            | Self::Rotate { parameter, .. } => parameter,
        }
    }
}

// ------------------------------------------------------------
// Map a graph onto a dynamic definition
// ------------------------------------------------------------

pub(crate) fn materialize_definition(
    document: &mut Document,
    entities: &mut [Entity],
    graph: &AcadGraph,
) -> Result<DynamicDefinition, String> {
    if let Some(kind) = graph.unsupported.first() {
        return Err(format!("{kind} is not editable yet"));
    }
    if graph.unresolved > 0 {
        return Err("a target was not in the block".into());
    }
    assign_entity_ids(document, entities);
    let mut dynamic = DynamicDefinition::default();
    let mut names = BTreeMap::new();
    let mut point_axes: BTreeMap<String, PointAxes> = BTreeMap::new();
    for (index, parameter) in graph.parameters.iter().enumerate() {
        if !parameter.show()
            && !graph
                .actions
                .iter()
                .any(|action| action.parameter().eq_ignore_ascii_case(parameter.name()))
        {
            continue;
        }
        if let AcadParam::Point { base, .. } = parameter {
            let raw = parameter_key(parameter.name());
            let x_label = unique_name(&names, &format!("{raw} X"));
            let x_id = document.allocate_parameter_id();
            names.insert(x_label.to_ascii_lowercase(), (x_id, x_label.clone()));
            let y_label = unique_name(&names, &format!("{raw} Y"));
            let y_id = document.allocate_parameter_id();
            names.insert(y_label.to_ascii_lowercase(), (y_id, y_label.clone()));
            names.insert(raw.to_ascii_lowercase(), (x_id, x_label.clone()));
            point_axes.insert(
                raw.to_ascii_lowercase(),
                PointAxes {
                    x: x_id,
                    y: y_id,
                    base: *base,
                },
            );
            dynamic
                .parameters
                .push(axis_parameter(x_id, x_label, base.x, index as i32));
            dynamic
                .parameters
                .push(axis_parameter(y_id, y_label, base.y, index as i32));
            continue;
        }
        let name = unique_name(&names, parameter.name());
        let id = document.allocate_parameter_id();
        names.insert(name.to_ascii_lowercase(), (id, name.clone()));
        dynamic.parameters.push(parameter_def(
            document,
            entities,
            parameter,
            id,
            name,
            index as i32,
        )?);
    }
    if dynamic.parameters.is_empty() {
        return Err("no editable properties".into());
    }
    for parameter in &graph.parameters {
        if !parameter.show() {
            continue;
        }
        let linked = graph
            .actions
            .iter()
            .any(|action| action.parameter().eq_ignore_ascii_case(parameter.name()));
        let needs_action = matches!(
            parameter,
            AcadParam::Linear { .. }
                | AcadParam::Flip { .. }
                | AcadParam::Rotation { .. }
                | AcadParam::Point { .. }
        );
        if needs_action && !linked {
            return Err(format!("{} has no action", parameter.name()));
        }
    }

    push_visibility_groups(document, entities, &mut dynamic, graph)?;
    reveal_visibility(entities, &dynamic);
    let mut rotations = Vec::new();
    for action in &graph.actions {
        let Some((parameter, _)) = lookup_name(&names, action.parameter()) else {
            return Err(format!(
                "{} is not connected to a property",
                action_label(action)
            ));
        };
        if let Some(axes) = point_axes
            .get(&action.parameter().trim().to_ascii_lowercase())
            .copied()
        {
            match action {
                AcadAction::Stretch {
                    offset_x,
                    offset_y,
                    connection,
                    targets,
                    ..
                } => {
                    push_point_stretch(
                        document,
                        entities,
                        &mut dynamic,
                        &axes,
                        *offset_x,
                        *offset_y,
                        connection,
                        targets,
                    )?;
                }
                AcadAction::Move {
                    offset_x,
                    offset_y,
                    connection,
                    entities: members,
                    ..
                } => {
                    push_point_move(
                        document,
                        entities,
                        &mut dynamic,
                        &axes,
                        *offset_x,
                        *offset_y,
                        connection,
                        members,
                    )?;
                }
                AcadAction::Flip { .. } | AcadAction::Rotate { .. } => {
                    return Err("a point parameter cannot drive this action".into());
                }
            }
            continue;
        }
        match action {
            AcadAction::Stretch {
                offset_x,
                offset_y,
                connection,
                targets,
                ..
            } => {
                // The Y component stays with the action until stretch uses a 2D offset.
                let _ = offset_y;
                push_stretch(
                    document,
                    entities,
                    &mut dynamic,
                    parameter,
                    *offset_x,
                    connection,
                    targets,
                    &graph,
                )?;
            }
            AcadAction::Move {
                offset_x,
                offset_y,
                connection,
                entities: members,
                ..
            } => {
                // The Y component stays with the action until move uses a 2D offset.
                let _ = offset_y;
                push_move(
                    document,
                    entities,
                    &mut dynamic,
                    parameter,
                    *offset_x,
                    connection,
                    members,
                    &graph,
                )?;
            }
            AcadAction::Flip {
                entities: members, ..
            } => {
                push_flip(document, entities, &mut dynamic, parameter, members, &graph)?;
            }
            AcadAction::Rotate {
                pivot,
                entities: members,
                angle_offset,
                ..
            } => {
                if angle_offset.abs() > 1e-6 {
                    return Err("rotation offset is not editable yet".into());
                }
                push_rotate(
                    document,
                    entities,
                    &mut dynamic,
                    parameter,
                    *pivot,
                    members,
                    &mut rotations,
                )?;
            }
        }
    }
    unrotate_members(entities, &dynamic, &rotations)?;
    cad_core::validate_definition(&dynamic, entities).map_err(|err| err.to_string())?;
    Ok(dynamic)
}

fn parameter_def(
    document: &mut Document,
    entities: &[Entity],
    parameter: &AcadParam,
    id: ParameterId,
    name: String,
    order: i32,
) -> Result<ParameterDef, String> {
    let kind = match parameter {
        AcadParam::Visibility { states, .. } => {
            if states.is_empty() || states.len() > MAX_DYNAMIC_STATES {
                return Err(format!("{name} has no visibility states"));
            }
            let mut options = Vec::new();
            let mut used_labels = BTreeSet::new();
            for state in states {
                let base = if state.name.trim().is_empty() {
                    format!("State {}", options.len() + 1)
                } else {
                    state.name.clone()
                };
                let mut label = base.clone();
                let mut suffix = 2u32;
                while !used_labels.insert(label.to_ascii_lowercase()) {
                    label = format!("{base} {suffix}");
                    suffix += 1;
                }
                options.push(ChoiceOption {
                    id: document.allocate_option_id(),
                    label,
                });
            }
            let default = default_visibility_option(entities, states, &options);
            ParameterKind::Choice(ChoiceParameter { options, default })
        }
        AcadParam::Linear {
            default_distance,
            value_set,
            ..
        } => {
            let mut numeric = NumericParameter::length(*default_distance);
            numeric.reference = *default_distance;
            numeric.display_order = order;
            apply_value_set(&mut numeric, value_set, *default_distance)?;
            ParameterKind::Number(numeric)
        }
        AcadParam::Flip {
            base_label,
            flipped_label,
            ..
        } => ParameterKind::Boolean(BooleanParameter {
            default: false,
            true_label: label_or(flipped_label, "Flipped"),
            false_label: label_or(base_label, "Not flipped"),
        }),
        AcadParam::Rotation {
            angle_deg,
            value_set,
            ..
        } => {
            let mut numeric = NumericParameter::length(*angle_deg);
            numeric.quantity = NumericQuantity::Angle;
            numeric.unit = ParameterUnit::Degrees;
            numeric.reference = *angle_deg;
            numeric.display_order = order;
            apply_value_set(&mut numeric, value_set, *angle_deg)?;
            ParameterKind::Number(numeric)
        }
        AcadParam::Point { .. } => {
            return Err("a point parameter is stored as Position X and Position Y".into());
        }
    };
    Ok(ParameterDef {
        id,
        name,
        description: None,
        display_order: order,
        kind,
    })
}

fn default_visibility_option(
    entities: &[Entity],
    states: &[AcadVisibilityState],
    options: &[ChoiceOption],
) -> cad_core::OptionId {
    let mut best = 0usize;
    let mut best_score = i32::MIN;
    for (index, state) in states.iter().enumerate() {
        let mut score = 0i32;
        for (entity_index, entity) in entities.iter().enumerate() {
            let listed = state.entities.contains(&entity_index);
            if listed == entity.visible {
                score += 1;
            } else if listed || entity.visible {
                score -= 1;
            }
        }
        if score > best_score {
            best_score = score;
            best = index;
        }
    }
    options
        .get(best)
        .map(|option| option.id)
        .unwrap_or(options[0].id)
}

fn apply_value_set(
    numeric: &mut NumericParameter,
    value_set: &AcadValueSet,
    default: f64,
) -> Result<(), String> {
    if let Some(min) = value_set.minimum {
        if min.is_finite() {
            numeric.min = Some(min.min(default));
        }
    }
    if let Some(max) = value_set.maximum {
        if max.is_finite() {
            numeric.max = Some(max.max(default));
        }
    }
    if let Some(step) = value_set.increment {
        if step.is_finite() && step > 0.0 {
            numeric.step = Some(step);
            numeric.step_policy = StepPolicy::IncrementOnly;
            numeric.step_origin = StepOrigin::Minimum;
        }
    }
    if !value_set.list.is_empty() {
        let mut list = value_set.list.clone();
        if !list
            .iter()
            .any(|item| cad_core::numbers_equal(*item, default))
        {
            list.push(default);
        }
        numeric.domain = NumericDomain::AllowedValues(list);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct PointAxes {
    x: ParameterId,
    y: ParameterId,
    base: Point2,
}

struct AxisDrive {
    parameter: ParameterId,
    direction: Point2,
    reference: f64,
    multiplier: f64,
}

fn parameter_key(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "Position".to_string()
    } else {
        trimmed.to_string()
    }
}

fn axis_parameter(id: ParameterId, name: String, reference: f64, order: i32) -> ParameterDef {
    let mut numeric = NumericParameter::length(reference);
    numeric.display_order = order;
    ParameterDef {
        id,
        name,
        description: None,
        display_order: order,
        kind: ParameterKind::Number(numeric),
    }
}

fn point_drives(axes: &PointAxes, offset_x: f64, offset_y: f64, connection: &str) -> Vec<AxisDrive> {
    let sign = connection_sign(connection);
    [
        AxisDrive {
            parameter: axes.x,
            direction: Point2::new(1.0, 0.0),
            reference: axes.base.x,
            multiplier: offset_x * sign,
        },
        AxisDrive {
            parameter: axes.y,
            direction: Point2::new(0.0, 1.0),
            reference: axes.base.y,
            multiplier: offset_y * sign,
        },
    ]
    .into_iter()
    .filter(|drive| drive.multiplier.abs() > 1e-12)
    .collect()
}

fn push_point_stretch(
    document: &mut Document,
    entities: &[Entity],
    dynamic: &mut DynamicDefinition,
    axes: &PointAxes,
    offset_x: f64,
    offset_y: f64,
    connection: &str,
    targets: &[AcadStretchTarget],
) -> Result<(), String> {
    if targets.is_empty() {
        return Ok(());
    }
    let drives = point_drives(axes, offset_x, offset_y, connection);
    if drives.is_empty() {
        return Err("stretch has no effect".into());
    }
    let mut points = Vec::new();
    let mut whole = Vec::new();
    for target in targets {
        let entity = entities
            .get(target.entity)
            .ok_or("stretch target is missing")?;
        match classify_stretch(entity, &target.indexes)? {
            StretchUse::Whole => whole.push(GeometryTarget::Entity(entity.id)),
            StretchUse::Points(found) => points.extend(found),
        }
    }
    if points.is_empty() && whole.is_empty() {
        return Err("stretch has no effect".into());
    }
    for drive in &drives {
        if !points.is_empty() {
            merge_behavior(
                dynamic,
                behavior(
                    document.allocate_action_id(),
                    BehaviorKind::Stretch,
                    drive.parameter,
                    cad_core::dedupe_targets(points.clone()),
                    drive.direction,
                    drive.reference,
                    drive.multiplier,
                    FollowRole::Second,
                )?,
            );
        }
        if !whole.is_empty() {
            merge_behavior(
                dynamic,
                behavior(
                    document.allocate_action_id(),
                    BehaviorKind::Move,
                    drive.parameter,
                    cad_core::dedupe_targets(whole.clone()),
                    drive.direction,
                    drive.reference,
                    drive.multiplier,
                    FollowRole::Custom,
                )?,
            );
        }
    }
    Ok(())
}

fn push_point_move(
    document: &mut Document,
    entities: &[Entity],
    dynamic: &mut DynamicDefinition,
    axes: &PointAxes,
    offset_x: f64,
    offset_y: f64,
    connection: &str,
    members: &[usize],
) -> Result<(), String> {
    if members.is_empty() {
        return Ok(());
    }
    let drives = point_drives(axes, offset_x, offset_y, connection);
    if drives.is_empty() {
        return Err("move has no effect".into());
    }
    let mut targets = Vec::new();
    for index in members {
        let entity = entities.get(*index).ok_or("move target is missing")?;
        let target = GeometryTarget::Entity(entity.id);
        cad_core::capability_for(BehaviorKind::Move, &entity.geometry, target)
            .map_err(|reason| reason.to_string())?;
        targets.push(target);
    }
    let targets = cad_core::dedupe_targets(targets);
    for drive in &drives {
        merge_behavior(
            dynamic,
            behavior(
                document.allocate_action_id(),
                BehaviorKind::Move,
                drive.parameter,
                targets.clone(),
                drive.direction,
                drive.reference,
                drive.multiplier,
                FollowRole::Custom,
            )?,
        );
    }
    Ok(())
}

fn push_stretch(
    document: &mut Document,
    entities: &[Entity],
    dynamic: &mut DynamicDefinition,
    parameter: ParameterId,
    offset_x: f64,
    connection: &str,
    targets: &[AcadStretchTarget],
    graph: &AcadGraph,
) -> Result<(), String> {
    if targets.is_empty() {
        return Ok(());
    }
    let (direction, reference) = linear_direction(dynamic, parameter, graph)?;
    let multiplier = offset_x * connection_sign(connection);
    if multiplier.abs() <= 1e-12 {
        return Err("stretch has no effect".into());
    }
    let mut points = Vec::new();
    let mut whole = Vec::new();
    for target in targets {
        let entity = entities
            .get(target.entity)
            .ok_or("stretch target is missing")?;
        match classify_stretch(entity, &target.indexes)? {
            StretchUse::Whole => whole.push(GeometryTarget::Entity(entity.id)),
            StretchUse::Points(found) => points.extend(found),
        }
    }
    if !points.is_empty() {
        merge_behavior(
            dynamic,
            behavior(
                document.allocate_action_id(),
                BehaviorKind::Stretch,
                parameter,
                cad_core::dedupe_targets(points),
                direction,
                reference,
                multiplier,
                FollowRole::Second,
            )?,
        );
    }
    if !whole.is_empty() {
        merge_behavior(
            dynamic,
            behavior(
                document.allocate_action_id(),
                BehaviorKind::Move,
                parameter,
                cad_core::dedupe_targets(whole),
                direction,
                reference,
                multiplier,
                FollowRole::Custom,
            )?,
        );
    }
    Ok(())
}

fn push_move(
    document: &mut Document,
    entities: &[Entity],
    dynamic: &mut DynamicDefinition,
    parameter: ParameterId,
    offset_x: f64,
    connection: &str,
    members: &[usize],
    graph: &AcadGraph,
) -> Result<(), String> {
    if members.is_empty() {
        return Ok(());
    }
    let (direction, reference) = linear_direction(dynamic, parameter, graph)?;
    let multiplier = offset_x * connection_sign(connection);
    if multiplier.abs() <= 1e-12 {
        return Err("move has no effect".into());
    }
    let mut targets = Vec::new();
    for index in members {
        let entity = entities.get(*index).ok_or("move target is missing")?;
        let target = GeometryTarget::Entity(entity.id);
        cad_core::capability_for(BehaviorKind::Move, &entity.geometry, target)
            .map_err(|reason| reason.to_string())?;
        targets.push(target);
    }
    merge_behavior(
        dynamic,
        behavior(
            document.allocate_action_id(),
            BehaviorKind::Move,
            parameter,
            cad_core::dedupe_targets(targets),
            direction,
            reference,
            multiplier,
            FollowRole::Custom,
        )?,
    );
    Ok(())
}

fn merge_behavior(dynamic: &mut DynamicDefinition, behavior: DynamicBehavior) {
    let Some(existing) = dynamic.behaviors.iter_mut().find(|item| {
        item.parameter == behavior.parameter
            && item.kind == behavior.kind
            && item.multiplier == behavior.multiplier
            && item.reference_value == behavior.reference_value
            && item.local_direction.x == behavior.local_direction.x
            && item.local_direction.y == behavior.local_direction.y
    }) else {
        dynamic.behaviors.push(behavior);
        return;
    };
    existing.targets.extend(behavior.targets);
    existing.targets = cad_core::dedupe_targets(std::mem::take(&mut existing.targets));
}

fn push_flip(
    document: &mut Document,
    entities: &[Entity],
    dynamic: &mut DynamicDefinition,
    parameter: ParameterId,
    members: &[usize],
    graph: &AcadGraph,
) -> Result<(), String> {
    let Some(AcadParam::Flip { axis_a, axis_b, .. }) = graph.parameters.iter().find(|item| {
        item.name()
            .eq_ignore_ascii_case(param_name(dynamic, parameter))
    }) else {
        return Err("flip action has no flip property".into());
    };
    if axis_a.distance(*axis_b) <= 1e-9 {
        return Err("flip axis is degenerate".into());
    }
    if members.is_empty() {
        return Err("flip has no geometry".into());
    }
    let mut ids = Vec::new();
    for index in members {
        let entity = entities.get(*index).ok_or("flip target is missing")?;
        ids.push(entity.id);
    }
    let id = document.allocate_action_id();
    dynamic.reflections.push(ReflectionBehavior {
        id,
        name: None,
        members: ids,
        axis_a: *axis_a,
        axis_b: *axis_b,
        condition: ParameterCondition::Boolean {
            parameter,
            state: true,
        },
        text_policy: TextReflectPolicy::KeepReadable,
    });
    dynamic.transform_order.push(id);
    Ok(())
}

fn push_rotate(
    document: &mut Document,
    entities: &[Entity],
    dynamic: &mut DynamicDefinition,
    parameter: ParameterId,
    pivot: Point2,
    members: &[usize],
    rotations: &mut Vec<(ActionId, f64)>,
) -> Result<(), String> {
    if members.is_empty() {
        return Err("rotation has no geometry".into());
    }
    let angle = dynamic
        .parameter(parameter)
        .and_then(|item| match &item.kind {
            ParameterKind::Number(numeric) => Some(numeric.default),
            _ => None,
        })
        .ok_or("rotation action has no angle")?;
    let id = document.allocate_action_id();
    dynamic.rotations.push(RotationBehavior {
        id,
        name: None,
        members: rotation_members(entities, members)?,
        pivot,
        source: RotationSource::AngleParameter(parameter),
    });
    dynamic.transform_order.push(id);
    rotations.push((id, angle));
    Ok(())
}

fn push_visibility_groups(
    document: &mut Document,
    entities: &[Entity],
    dynamic: &mut DynamicDefinition,
    graph: &AcadGraph,
) -> Result<(), String> {
    for parameter in &graph.parameters {
        let AcadParam::Visibility { name, states, .. } = parameter else {
            continue;
        };
        let Some((parameter_id, option_ids)) = dynamic.parameters.iter().find_map(|item| {
            if !item.name.eq_ignore_ascii_case(name) {
                return None;
            }
            let ParameterKind::Choice(choice) = &item.kind else {
                return None;
            };
            Some((
                item.id,
                choice
                    .options
                    .iter()
                    .map(|option| option.id)
                    .collect::<Vec<_>>(),
            ))
        }) else {
            continue;
        };
        let mut buckets: BTreeMap<Vec<usize>, Vec<EntityId>> = BTreeMap::new();
        for (index, entity) in entities.iter().enumerate() {
            let membership: Vec<usize> = states
                .iter()
                .enumerate()
                .filter(|(_, state)| state.entities.contains(&index))
                .map(|(state_index, _)| state_index)
                .collect();
            if membership.is_empty() {
                continue;
            }
            buckets.entry(membership).or_default().push(entity.id);
        }
        if buckets.is_empty() && states.iter().any(|state| !state.entities.is_empty()) {
            return Err("visibility entities did not resolve".into());
        }
        for (membership, members) in buckets {
            let options: Vec<_> = membership
                .iter()
                .filter_map(|index| option_ids.get(*index).copied())
                .collect();
            if options.is_empty() {
                continue;
            }
            dynamic.visibility.push(VisibilityGroup {
                id: document.allocate_action_id(),
                name: name.clone(),
                members,
                conditions: vec![ParameterCondition::Choice {
                    parameter: parameter_id,
                    options,
                }],
            });
        }
    }
    Ok(())
}

enum StretchUse {
    Whole,
    Points(Vec<GeometryTarget>),
}

fn classify_stretch(entity: &Entity, indexes: &[u32]) -> Result<StretchUse, String> {
    if indexes.is_empty() {
        let target = GeometryTarget::Entity(entity.id);
        cad_core::capability_for(BehaviorKind::Move, &entity.geometry, target)
            .map_err(|reason| reason.to_string())?;
        return Ok(StretchUse::Whole);
    }
    match &entity.geometry {
        Geometry::Line { .. } => {
            let mut points = Vec::new();
            for index in indexes {
                let target = match index {
                    0 => GeometryTarget::LineStart(entity.id),
                    1 => GeometryTarget::LineEnd(entity.id),
                    _ => return Err("line stretch index is out of range".into()),
                };
                cad_core::capability_for(BehaviorKind::Stretch, &entity.geometry, target)
                    .map_err(|reason| reason.to_string())?;
                points.push(target);
            }
            Ok(StretchUse::Points(points))
        }
        Geometry::LwPolyline { vertices, .. } | Geometry::Polyline { vertices, .. } => {
            let all = indexes.len() >= vertices.len()
                && (0..vertices.len()).all(|index| indexes.contains(&(index as u32)));
            if all || entity.geometry.polyline_has_curves() && all {
                let target = GeometryTarget::Entity(entity.id);
                cad_core::capability_for(BehaviorKind::Move, &entity.geometry, target)
                    .map_err(|reason| reason.to_string())?;
                return Ok(StretchUse::Whole);
            }
            if entity.geometry.polyline_has_curves() {
                return Err("curved polyline stretch is not editable yet".into());
            }
            let mut points = Vec::new();
            for index in indexes {
                let Some(vertex) = vertices.get(*index as usize) else {
                    return Err("polyline stretch index is out of range".into());
                };
                let target = GeometryTarget::Vertex {
                    entity: entity.id,
                    vertex: vertex.vertex_id,
                };
                cad_core::capability_for(BehaviorKind::Stretch, &entity.geometry, target)
                    .map_err(|reason| reason.to_string())?;
                points.push(target);
            }
            Ok(StretchUse::Points(points))
        }
        _ => {
            let target = GeometryTarget::Entity(entity.id);
            cad_core::capability_for(BehaviorKind::Move, &entity.geometry, target)
                .map_err(|reason| reason.to_string())?;
            Ok(StretchUse::Whole)
        }
    }
}

fn behavior(
    id: ActionId,
    kind: BehaviorKind,
    parameter: ParameterId,
    targets: Vec<GeometryTarget>,
    direction: Point2,
    reference: f64,
    multiplier: f64,
    follow: FollowRole,
) -> Result<DynamicBehavior, String> {
    if targets.is_empty() {
        return Err("action has no geometry".into());
    }
    Ok(DynamicBehavior {
        id,
        kind,
        parameter,
        targets,
        local_direction: direction,
        reference_value: reference,
        multiplier,
        composition: CompositionRule::Additive,
        follow,
        name: None,
    })
}

fn linear_direction(
    dynamic: &DynamicDefinition,
    parameter: ParameterId,
    graph: &AcadGraph,
) -> Result<(Point2, f64), String> {
    let name = param_name(dynamic, parameter);
    let Some(AcadParam::Linear {
        base,
        end,
        default_distance,
        ..
    }) = graph
        .parameters
        .iter()
        .find(|item| item.name().eq_ignore_ascii_case(name))
    else {
        return Err("stretch is not driven by a distance".into());
    };
    let direction = normalize_direction(*end - *base).ok_or("distance direction is zero")?;
    Ok((direction, *default_distance))
}

fn unrotate_members(
    entities: &mut [Entity],
    dynamic: &DynamicDefinition,
    rotations: &[(ActionId, f64)],
) -> Result<(), String> {
    for (id, degrees) in rotations.iter().rev() {
        if degrees.abs() <= 1e-9 {
            continue;
        }
        let Some(behavior) = dynamic.rotations.iter().find(|item| item.id == *id) else {
            continue;
        };
        let radians = -degrees.to_radians();
        for member in &behavior.members {
            let Some(entity) = entities.iter_mut().find(|entity| entity.id == *member) else {
                return Err("rotation member is missing".into());
            };
            let rotated = cad_core::transform_entity(
                entity,
                EntityTransform::Rotate {
                    base: behavior.pivot,
                    radians,
                },
            )
            .map_err(|err| err.to_string())?;
            *entity = rotated;
        }
    }
    Ok(())
}

fn connection_sign(name: &str) -> f64 {
    let lower = name.to_ascii_lowercase();
    if lower.contains("end") || lower.contains("displacement") || lower.is_empty() {
        1.0
    } else if lower.contains("base") || lower.contains("start") || lower.contains("origin") {
        -1.0
    } else {
        1.0
    }
}

fn param_name(dynamic: &DynamicDefinition, parameter: ParameterId) -> &str {
    dynamic
        .parameter(parameter)
        .map(|item| item.name.as_str())
        .unwrap_or("")
}

fn lookup_name(
    names: &BTreeMap<String, (ParameterId, String)>,
    name: &str,
) -> Option<(ParameterId, String)> {
    names
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(&name.trim().to_ascii_lowercase()))
        .map(|(_, value)| value.clone())
}

fn unique_name(used: &BTreeMap<String, (ParameterId, String)>, raw: &str) -> String {
    let base = {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            "Property".to_string()
        } else {
            trimmed.to_string()
        }
    };
    if !used.contains_key(&base.to_ascii_lowercase()) {
        return base;
    }
    for index in 2..64 {
        let candidate = format!("{base} {index}");
        if !used.contains_key(&candidate.to_ascii_lowercase()) {
            return candidate;
        }
    }
    format!("{base} {}", used.len() + 1)
}

fn label_or(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn action_label(action: &AcadAction) -> &'static str {
    match action {
        AcadAction::Stretch { .. } => "stretch",
        AcadAction::Move { .. } => "move",
        AcadAction::Flip { .. } => "flip",
        AcadAction::Rotate { .. } => "rotation",
    }
}

fn assign_entity_ids(document: &mut Document, entities: &mut [Entity]) {
    for entity in entities {
        if !entity.id.is_assigned() {
            entity.id = document.allocate_id();
        }
        if let Some(vertices) = entity.geometry.polyline_vertices_mut() {
            for vertex in vertices {
                if !vertex.vertex_id.is_assigned() {
                    vertex.vertex_id = document.allocate_vertex_id();
                }
            }
        }
    }
}

fn reveal_visibility(entities: &mut [Entity], dynamic: &DynamicDefinition) {
    let mut managed = BTreeSet::new();
    for group in &dynamic.visibility {
        managed.extend(group.members.iter().copied());
    }
    for entity in entities {
        if managed.contains(&entity.id) {
            entity.visible = true;
        }
    }
}

fn geometries_match(evaluated: &[Entity], reference: &[Entity]) -> bool {
    if fingerprint_visible_entities(evaluated) != fingerprint_visible_entities(reference) {
        return false;
    }
    match (
        visible_geometry_key(evaluated, MAX_GEOMETRY_POINTS),
        visible_geometry_key(reference, MAX_GEOMETRY_POINTS),
    ) {
        (Some(left), Some(right)) => left == right,
        (None, None) => true,
        _ => false,
    }
}

fn rotation_members(entities: &[Entity], indexes: &[usize]) -> Result<Vec<EntityId>, String> {
    let mut ids = Vec::new();
    for index in indexes {
        let entity = entities.get(*index).ok_or("rotation target is missing")?;
        ids.push(entity.id);
    }
    Ok(ids)
}

// ------------------------------------------------------------
// Import
// ------------------------------------------------------------

struct ReadDynamic {
    graphs: Vec<AcadGraph>,
    links: BTreeMap<String, String>,
    objects: usize,
    kinds: BTreeMap<String, usize>,
}

pub(crate) unsafe fn import_acad_dynamic(
    dwg: *mut libredwg_sys::Dwg_Data,
    document: &mut Document,
    handles: &BTreeMap<String, Vec<u64>>,
) {
    let read = unsafe { read_dynamic(dwg, handles) };
    apply_dynamic(document, read);
    document.diagnostics.block_count = document.blocks.len();
}

fn apply_dynamic(document: &mut Document, read: ReadDynamic) {
    let mut lines = Vec::new();
    if read.graphs.is_empty() {
        if read.objects == 0 {
            push_line(&mut lines, "dynamic: no AutoCAD dynamic blocks");
        } else {
            push_line(
                &mut lines,
                &format!(
                    "dynamic: {} AutoCAD dynamic objects were not linked to a block",
                    read.objects
                ),
            );
            for (name, count) in &read.kinds {
                push_line(&mut lines, &format!("dynamic-object {name} {count}"));
            }
        }
        document.diagnostics.dynamic_lines = lines;
        return;
    }
    let mut evaluations = 0usize;
    let mut attached = BTreeSet::new();
    for graph in read
        .graphs
        .iter()
        .filter(|graph| !is_anonymous(&graph.block_name))
    {
        match attach_definition(document, graph, &mut evaluations) {
            Ok(editable) => {
                attached.insert(graph.block_name.to_ascii_lowercase());
                document.diagnostics.dynamic_definitions += 1;
                document.diagnostics.dynamic_editable += editable;
                push_line(
                    &mut lines,
                    &format!("dynamic {}: {}", graph.block_name, describe_graph(graph)),
                );
            }
            Err(reason) => {
                document.diagnostics.dynamic_kept_static += 1;
                push_line(
                    &mut lines,
                    &format!("dynamic {} kept static ({reason})", graph.block_name),
                );
            }
        }
    }
    for graph in read
        .graphs
        .iter()
        .filter(|graph| is_anonymous(&graph.block_name))
    {
        let source = read
            .links
            .get(&graph.block_name.to_ascii_lowercase())
            .cloned()
            .or_else(|| unique_source(&read.graphs, graph));
        let Some(source) = source else {
            document.diagnostics.dynamic_kept_static += 1;
            push_line(
                &mut lines,
                &format!("dynamic {} kept static (no source block)", graph.block_name),
            );
            continue;
        };
        if !attached.contains(&source.to_ascii_lowercase()) {
            document.diagnostics.dynamic_kept_static += 1;
            push_line(
                &mut lines,
                &format!(
                    "dynamic {} kept static ({} was not imported)",
                    graph.block_name, source
                ),
            );
            continue;
        }
        match verify_anonymous(document, &source, graph, &mut evaluations) {
            Ok(configuration) => {
                let count = retarget_block(document, &graph.block_name, &source, &configuration);
                document.diagnostics.dynamic_editable += count;
                push_line(
                    &mut lines,
                    &format!(
                        "dynamic {} -> {source} verified ({count})",
                        graph.block_name
                    ),
                );
                drop_unreferenced(document, &graph.block_name);
            }
            Err(reason) => {
                document.diagnostics.dynamic_kept_static += 1;
                push_line(
                    &mut lines,
                    &format!(
                        "dynamic {} -> {source} kept static ({reason})",
                        graph.block_name
                    ),
                );
            }
        }
    }
    push_line(
        &mut lines,
        &format!(
            "dynamic definitions: {} editable: {} kept static: {}",
            document.diagnostics.dynamic_definitions,
            document.diagnostics.dynamic_editable,
            document.diagnostics.dynamic_kept_static
        ),
    );
    document.diagnostics.dynamic_lines = lines;
}

fn attach_definition(
    document: &mut Document,
    graph: &AcadGraph,
    evaluations: &mut usize,
) -> Result<usize, String> {
    if *evaluations >= MAX_DYNAMIC_EVALUATIONS {
        return Err("evaluation cap reached".into());
    }
    let key = document
        .block_key(&graph.block_name)
        .ok_or_else(|| format!("{} is missing", graph.block_name))?;
    let snapshot = document
        .block_by_name(&key)
        .map(|block| block.entities.clone())
        .ok_or_else(|| format!("{} is missing", graph.block_name))?;
    if snapshot.len() > MAX_DYNAMIC_HANDLES {
        return Err("block is too large".into());
    }
    let mut entities = snapshot.clone();
    let dynamic = materialize_definition(document, &mut entities, graph)?;
    if let Some(block) = document.block_by_name_mut(&key) {
        block.entities = entities;
        block.dynamic = Some(dynamic);
    }
    *evaluations += 1;
    let matched = identity_matches(document, &key, &snapshot)?;
    if !matched {
        if let Some(block) = document.block_by_name_mut(&key) {
            block.entities = snapshot;
            block.dynamic = None;
        }
        return Err("evaluated default does not match the block".into());
    }
    Ok(configure_direct_inserts(document, &key))
}

fn identity_matches(document: &Document, key: &str, snapshot: &[Entity]) -> Result<bool, String> {
    let Some(block) = document.block_by_name(key) else {
        return Err("block disappeared".into());
    };
    let request = EvaluationRequest {
        generation: document.content_generation(),
    };
    let mut cache = EvaluationCache::default();
    let evaluated = evaluate_definition(document, block, None, &mut cache, request)
        .map_err(|err| err.to_string())?;
    Ok(geometries_match(&evaluated.entities, snapshot))
}

fn verify_anonymous(
    document: &Document,
    source: &str,
    anonymous: &AcadGraph,
    evaluations: &mut usize,
) -> Result<InstanceConfiguration, String> {
    let source_key = document
        .block_key(source)
        .ok_or_else(|| format!("{source} is missing"))?;
    let reference = document
        .block_by_name(&anonymous.block_name)
        .map(|block| block.entities.clone())
        .ok_or_else(|| format!("{} is missing", anonymous.block_name))?;
    let hint = instance_hint(document, &source_key, anonymous)?;
    let trials = trial_configs(document, &source_key, &hint);
    if trials.is_empty() {
        return Err("no configuration to try".into());
    }
    let request = EvaluationRequest {
        generation: document.content_generation(),
    };
    let mut cache = EvaluationCache::default();
    for configuration in trials {
        if *evaluations >= MAX_DYNAMIC_EVALUATIONS {
            return Err("evaluation cap reached".into());
        }
        *evaluations += 1;
        let Some(block) = document.block_by_name(&source_key) else {
            return Err(format!("{source} is missing"));
        };
        let Ok(evaluated) =
            evaluate_definition(document, block, Some(&configuration), &mut cache, request)
        else {
            continue;
        };
        if geometries_match(&evaluated.entities, &reference) {
            return Ok(configuration);
        }
    }
    Err("geometry did not match".into())
}

struct InstanceHint {
    numbers: BTreeMap<ParameterId, f64>,
    choices: Vec<ParameterId>,
    flags: Vec<ParameterId>,
}

fn instance_hint(
    document: &Document,
    source_key: &str,
    anonymous: &AcadGraph,
) -> Result<InstanceHint, String> {
    let dynamic = document
        .block_by_name(source_key)
        .and_then(|block| block.dynamic.as_ref())
        .ok_or_else(|| format!("{source_key} is not dynamic"))?;
    let mut numbers = BTreeMap::new();
    let mut choices = Vec::new();
    let mut flags = Vec::new();
    for parameter in &dynamic.parameters {
        match &parameter.kind {
            ParameterKind::Number(numeric) => {
                let current = anonymous
                    .parameters
                    .iter()
                    .find_map(|item| hinted_number(item, &parameter.name))
                    .unwrap_or(numeric.default);
                numbers.insert(parameter.id, current);
            }
            ParameterKind::Choice(_) => choices.push(parameter.id),
            ParameterKind::Boolean(_) => flags.push(parameter.id),
            ParameterKind::Text(_) => {}
        }
    }
    Ok(InstanceHint {
        numbers,
        choices,
        flags,
    })
}

fn hinted_number(parameter: &AcadParam, name: &str) -> Option<f64> {
    match parameter {
        AcadParam::Linear {
            name: parameter_name,
            current_distance,
            ..
        } if parameter_name.eq_ignore_ascii_case(name) => Some(*current_distance),
        AcadParam::Rotation {
            name: parameter_name,
            current_deg,
            ..
        } if parameter_name.eq_ignore_ascii_case(name) => Some(*current_deg),
        AcadParam::Point {
            name: parameter_name,
            current,
            ..
        } => {
            if name.eq_ignore_ascii_case(&format!("{parameter_name} X")) {
                Some(current.x)
            } else if name.eq_ignore_ascii_case(&format!("{parameter_name} Y")) {
                Some(current.y)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn trial_configs(
    document: &Document,
    source_key: &str,
    hint: &InstanceHint,
) -> Vec<InstanceConfiguration> {
    let Some(dynamic) = document
        .block_by_name(source_key)
        .and_then(|block| block.dynamic.as_ref())
    else {
        return Vec::new();
    };
    let mut configs = vec![InstanceConfiguration::default()];
    for (id, value) in &hint.numbers {
        for config in &mut configs {
            config.set(*id, ParameterValue::Number(*value));
        }
    }
    for id in &hint.choices {
        let Some(parameter) = dynamic.parameter(*id) else {
            continue;
        };
        let ParameterKind::Choice(choice) = &parameter.kind else {
            continue;
        };
        let mut expanded = Vec::new();
        for config in &configs {
            for option in &choice.options {
                if expanded.len() >= MAX_CONFIG_TRIALS {
                    break;
                }
                let mut next = config.clone();
                next.set(*id, ParameterValue::Choice(option.id));
                expanded.push(next);
            }
        }
        configs = expanded;
    }
    for id in &hint.flags {
        let mut expanded = Vec::new();
        for config in &configs {
            if expanded.len() >= MAX_CONFIG_TRIALS {
                break;
            }
            let mut off = config.clone();
            off.set(*id, ParameterValue::Boolean(false));
            expanded.push(off);
            if expanded.len() >= MAX_CONFIG_TRIALS {
                break;
            }
            let mut on = config.clone();
            on.set(*id, ParameterValue::Boolean(true));
            expanded.push(on);
        }
        configs = expanded;
    }
    configs.truncate(MAX_CONFIG_TRIALS);
    configs
}

fn configure_direct_inserts(document: &mut Document, source_key: &str) -> usize {
    let Some(dynamic) = document
        .block_by_name(source_key)
        .and_then(|block| block.dynamic.clone())
    else {
        return 0;
    };
    let defaults = default_configuration(&dynamic);
    let mut count = 0;
    for entity in all_entities_mut(document) {
        let Some(name) = entity.geometry.insert_block_name() else {
            continue;
        };
        if !name.eq_ignore_ascii_case(source_key) || is_anonymous(name) {
            continue;
        }
        if entity.geometry.insert_configuration().is_none() {
            entity
                .geometry
                .set_insert_configuration(Some(defaults.clone()));
            count += 1;
        }
    }
    count
}

fn default_configuration(dynamic: &DynamicDefinition) -> InstanceConfiguration {
    let mut configuration = InstanceConfiguration::default();
    for parameter in &dynamic.parameters {
        let value = match &parameter.kind {
            ParameterKind::Number(numeric) => ParameterValue::Number(numeric.default),
            ParameterKind::Choice(choice) => ParameterValue::Choice(choice.default),
            ParameterKind::Boolean(flag) => ParameterValue::Boolean(flag.default),
            ParameterKind::Text(text) => ParameterValue::Text(text.default.clone()),
        };
        configuration.set(parameter.id, value);
    }
    configuration
}

fn retarget_block(
    document: &mut Document,
    from: &str,
    to: &str,
    configuration: &InstanceConfiguration,
) -> usize {
    let Some(target) = document.block_key(to) else {
        return 0;
    };
    let mut count = 0;
    for entity in all_entities_mut(document) {
        let Some(name) = entity.geometry.insert_block_name() else {
            continue;
        };
        if !name.eq_ignore_ascii_case(from) {
            continue;
        }
        if let Some(block_name) = entity.geometry.insert_block_name_mut() {
            *block_name = target.clone();
        }
        entity
            .geometry
            .set_insert_configuration(Some(configuration.clone()));
        count += 1;
    }
    count
}

fn drop_unreferenced(document: &mut Document, name: &str) {
    if !is_anonymous(name) || block_is_referenced(document, name) {
        return;
    }
    if let Some(key) = document.block_key(name) {
        document.blocks.remove(&key);
    }
}

fn block_is_referenced(document: &Document, name: &str) -> bool {
    let mut spaces = vec![document.model_space.as_slice()];
    for block in document.blocks.values() {
        if block.name.eq_ignore_ascii_case(name) {
            continue;
        }
        spaces.push(block.entities.as_slice());
    }
    spaces.into_iter().any(|entities| {
        entities.iter().any(|entity| {
            entity
                .geometry
                .insert_block_name()
                .is_some_and(|block| block.eq_ignore_ascii_case(name))
        })
    })
}

fn all_entities_mut(document: &mut Document) -> Vec<&mut Entity> {
    let mut entities = Vec::new();
    for entity in &mut document.model_space {
        entities.push(entity);
    }
    for block in document.blocks.values_mut() {
        for entity in &mut block.entities {
            entities.push(entity);
        }
    }
    entities
}

fn unique_source(graphs: &[AcadGraph], anonymous: &AcadGraph) -> Option<String> {
    let wanted: BTreeSet<String> = anonymous
        .parameters
        .iter()
        .map(|parameter| parameter.name().to_ascii_lowercase())
        .collect();
    if wanted.is_empty() {
        return None;
    }
    let mut found = None;
    for graph in graphs {
        if is_anonymous(&graph.block_name) || !graph.unsupported.is_empty() {
            continue;
        }
        let names: BTreeSet<String> = graph
            .parameters
            .iter()
            .map(|parameter| parameter.name().to_ascii_lowercase())
            .collect();
        if names != wanted {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(graph.block_name.clone());
    }
    found
}

fn describe_graph(graph: &AcadGraph) -> String {
    let mut parts = Vec::new();
    for parameter in &graph.parameters {
        let kind = match parameter {
            AcadParam::Visibility { .. } => "visibility",
            AcadParam::Linear { .. } => "linear",
            AcadParam::Flip { .. } => "flip",
            AcadParam::Rotation { .. } => "rotation",
            AcadParam::Point { .. } => "point",
        };
        parts.push(format!("{kind} {}", parameter.name()));
    }
    if parts.is_empty() {
        "no properties".into()
    } else {
        parts.join(", ")
    }
}

fn is_layout_block(name: &str) -> bool {
    let upper = name.trim().to_ascii_uppercase();
    upper == "*MODEL_SPACE" || upper.starts_with("*PAPER_SPACE")
}

fn is_anonymous(name: &str) -> bool {
    name.trim().to_ascii_uppercase().starts_with("*U")
}

fn push_line(lines: &mut Vec<String>, line: &str) {
    if lines.len() < MAX_REPORT_LINES {
        lines.push(line.to_string());
    }
}

// ------------------------------------------------------------
// LibreDWG reader
// ------------------------------------------------------------

unsafe fn read_dynamic(
    dwg: *mut libredwg_sys::Dwg_Data,
    handles: &BTreeMap<String, Vec<u64>>,
) -> ReadDynamic {
    let mut graphs: Vec<AcadGraph> = Vec::new();
    let mut links = BTreeMap::new();
    let mut objects = 0usize;
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut grouped: BTreeMap<String, Vec<RawNode>> = BTreeMap::new();
    let count = unsafe { libredwg_sys::dwg_get_num_objects(dwg) };
    for index in 0..count {
        let object = unsafe { libredwg_sys::dwg_get_object(dwg, index) };
        if object.is_null() {
            continue;
        }
        let dxf = object_dxfname(object);
        if is_dynamic_object(&dxf) {
            objects += 1;
            *kinds.entry(dxf.clone()).or_default() += 1;
            if matches!(
                classify_node(&dxf),
                NodeClass::Parameter | NodeClass::Action | NodeClass::Unsupported
            ) {
                if let Some(name) = owning_block_name(dwg, object) {
                    if !is_layout_block(&name) {
                        let payload = object_payload(object);
                        let (id, parent) = eval_ids(payload, &dxf);
                        grouped.entry(name).or_default().push(RawNode {
                            id,
                            parent,
                            dxf: dxf.clone(),
                            payload,
                        });
                    }
                }
            }
        }
        if dxf.eq_ignore_ascii_case("BLOCKREPRESENTATION") {
            let payload = object_payload(object);
            let source = get_field::<*mut libredwg_sys::Dwg_Object_Ref>(
                payload,
                "BLOCKREPRESENTATION",
                "block",
            )
            .and_then(resolve_block_name)
            .filter(|name| !name.is_empty());
            if let (Some(source), Some(host)) = (source, owning_block_name(dwg, object)) {
                if is_anonymous(&host) {
                    links.insert(host.to_ascii_lowercase(), source);
                }
            }
        }
        if dxf.eq_ignore_ascii_case("EVALUATION_GRAPH") {
            if let Some(name) = owning_block_name(dwg, object) {
                if !is_layout_block(&name)
                    && !graphs
                        .iter()
                        .any(|graph: &AcadGraph| graph.block_name.eq_ignore_ascii_case(&name))
                {
                    let entity_handles = lookup_handles(handles, &name);
                    graphs.push(read_graph(dwg, &name, object, entity_handles));
                }
            }
        }
        let fixed = object_fixedtype(object);
        if fixed == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_BLOCK_HEADER {
            let header = unsafe { libredwg_sys::uncad_object_object_ptr(object) };
            if header.is_null() {
                continue;
            }
            let Some(name) = block_record_name(header) else {
                continue;
            };
            if let Some(source) = representation_source(dwg, header) {
                links.insert(name.to_ascii_lowercase(), source);
            }
            if let Some(dict) = extension_dictionary(dwg, header) {
                if let Some(graph) = find_named(dwg, dict, "EVALUATION_GRAPH") {
                    let entity_handles = lookup_handles(handles, &name);
                    graphs.push(read_graph(dwg, &name, graph, entity_handles));
                }
            }
        } else if fixed == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_INSERT
            || fixed == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MINSERT
        {
            let entity = unsafe { libredwg_sys::uncad_object_entity_ptr(object) };
            if entity.is_null() {
                continue;
            }
            let dxf = if fixed == libredwg_sys::DWG_OBJECT_TYPE_DWG_TYPE_MINSERT {
                "MINSERT"
            } else {
                "INSERT"
            };
            let block_name =
                get_field::<*mut libredwg_sys::Dwg_Object_Ref>(entity, dxf, "block_header")
                    .and_then(resolve_block_name)
                    .unwrap_or_default();
            if is_anonymous(&block_name) {
                if let Some(source) = representation_source(dwg, entity) {
                    links.insert(block_name.to_ascii_lowercase(), source);
                }
            }
        }
    }
    for (name, nodes) in &grouped {
        if graphs
            .iter()
            .any(|graph: &AcadGraph| graph.block_name.eq_ignore_ascii_case(name))
        {
            continue;
        }
        let entity_handles = lookup_handles(handles, name);
        graphs.push(unsafe { assemble_graph(dwg, name, nodes, entity_handles) });
    }
    ReadDynamic {
        graphs,
        links,
        objects,
        kinds,
    }
}

fn owning_block_name(
    dwg: *mut libredwg_sys::Dwg_Data,
    start: *mut libredwg_sys::Dwg_Object,
) -> Option<String> {
    let mut current = start;
    for _ in 0..8 {
        if current.is_null() {
            return None;
        }
        let payload = object_payload(current);
        if payload.is_null() {
            return None;
        }
        let dxf = object_dxfname(current);
        if dxf.eq_ignore_ascii_case("INSERT") || dxf.eq_ignore_ascii_case("MINSERT") {
            let name =
                get_field::<*mut libredwg_sys::Dwg_Object_Ref>(payload, &dxf, "block_header")
                    .and_then(resolve_block_name)
                    .unwrap_or_default();
            if !name.is_empty() {
                return Some(name);
            }
        }
        if dxf.eq_ignore_ascii_case("BLOCK_HEADER") {
            return block_record_name(payload).filter(|name| !name.is_empty());
        }
        let owner = get_common_field::<*mut libredwg_sys::Dwg_Object_Ref>(payload, "ownerhandle")?;
        let next = ref_object(dwg, owner);
        if next.is_null() || next == current {
            return None;
        }
        current = next;
    }
    None
}

fn object_payload(object: *mut libredwg_sys::Dwg_Object) -> *mut c_void {
    unsafe {
        let entity = libredwg_sys::uncad_object_entity_ptr(object);
        if !entity.is_null() {
            return entity;
        }
        libredwg_sys::uncad_object_object_ptr(object)
    }
}

fn is_dynamic_object(dxf: &str) -> bool {
    dxf.eq_ignore_ascii_case("EVALUATION_GRAPH")
        || dxf.eq_ignore_ascii_case("BLOCKREPRESENTATION")
        || (dxf.starts_with("BLOCK") && (dxf.ends_with("PARAMETER") || dxf.ends_with("ACTION")))
}

fn lookup_handles<'a>(handles: &'a BTreeMap<String, Vec<u64>>, name: &str) -> Option<&'a [u64]> {
    if let Some(found) = handles.get(name) {
        return Some(found.as_slice());
    }
    handles
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, values)| values.as_slice())
}

unsafe fn read_graph(
    dwg: *mut libredwg_sys::Dwg_Data,
    name: &str,
    graph: *mut libredwg_sys::Dwg_Object,
    entity_handles: Option<&[u64]>,
) -> AcadGraph {
    let nodes = unsafe { graph_nodes(dwg, graph) };
    unsafe { assemble_graph(dwg, name, &nodes, entity_handles) }
}

unsafe fn assemble_graph(
    dwg: *mut libredwg_sys::Dwg_Data,
    name: &str,
    nodes: &[RawNode],
    entity_handles: Option<&[u64]>,
) -> AcadGraph {
    let mut result = AcadGraph {
        block_name: name.to_string(),
        parameters: Vec::new(),
        actions: Vec::new(),
        unsupported: Vec::new(),
        unresolved: 0,
    };
    let handle_index = handle_index(entity_handles);
    let mut raw_params = Vec::new();
    let mut raw_actions = Vec::new();
    let mut param_names: Vec<(i64, String)> = Vec::new();
    for node in nodes {
        match classify_node(&node.dxf) {
            NodeClass::Unsupported => {
                if !result.unsupported.iter().any(|item| item == &node.dxf) {
                    result.unsupported.push(node.dxf.clone());
                }
            }
            NodeClass::Parameter => raw_params.push(node),
            NodeClass::Action => raw_actions.push(node),
            NodeClass::Ignore => {}
        }
    }
    for node in &raw_params {
        if node.parent >= 0 && raw_params.iter().any(|other| other.id == node.parent) {
            if !result
                .unsupported
                .iter()
                .any(|item| item == "chained parameter")
            {
                result.unsupported.push("chained parameter".into());
            }
        }
        if let Some(parameter) = unsafe { read_parameter(node, &handle_index) } {
            param_names.push((node.id, parameter.name().to_string()));
            result.parameters.push(parameter);
        }
    }
    let mut links: Vec<(&RawNode, String)> = Vec::new();
    for node in &raw_actions {
        if raw_actions
            .iter()
            .any(|other| other.id == node.parent && node.parent >= 0)
        {
            if !result
                .unsupported
                .iter()
                .any(|item| item == "chained action")
            {
                result.unsupported.push("chained action".into());
            }
            continue;
        }
        if let Some(parameter_name) = parameter_for_action(node, &param_names, &result.parameters)
            .or_else(|| parameter_from_action_nodes(node, &param_names))
            .or_else(|| parameter_from_props(node, &raw_params, &param_names))
            .or_else(|| parameter_from_connection_codes(node, &param_names))
            .or_else(|| parameter_from_deps(node, &raw_params, &param_names))
        {
            links.push((node, parameter_name));
        } else if !result
            .unsupported
            .iter()
            .any(|item| item == "unconnected action")
        {
            result.unsupported.push("unconnected action".into());
        }
    }
    for (node, parameter_name) in links {
        if let Some(action) = unsafe {
            read_action(
                dwg,
                node,
                &parameter_name,
                &handle_index,
                &mut result.unresolved,
            )
        } {
            result.actions.push(action);
        }
    }
    result
}

#[derive(Clone, Copy)]
enum NodeClass {
    Parameter,
    Action,
    Unsupported,
    Ignore,
}

fn classify_node(dxf: &str) -> NodeClass {
    match dxf {
        "BLOCKVISIBILITYPARAMETER"
        | "BLOCKLINEARPARAMETER"
        | "BLOCKROTATIONPARAMETER"
        | "BLOCKFLIPPARAMETER"
        | "BLOCKPOINTPARAMETER" => NodeClass::Parameter,
        "BLOCKSTRETCHACTION" | "BLOCKMOVEACTION" | "BLOCKFLIPACTION" | "BLOCKROTATEACTION" => {
            NodeClass::Action
        }
        "BLOCKARRAYACTION"
        | "BLOCKSCALEACTION"
        | "BLOCKLOOKUPACTION"
        | "BLOCKLOOKUPPARAMETER"
        | "BLOCKPOLARSTRETCHACTION"
        | "BLOCKPOLARPARAMETER"
        | "BLOCKXYPARAMETER"
        | "BLOCKALIGNMENTPARAMETER" => NodeClass::Unsupported,
        _ if dxf.ends_with("GRIP")
            || dxf.contains("CONSTRAINT")
            || dxf == "BLOCKBASEPOINTPARAMETER"
            || dxf == "BLOCKGRIPLOCATIONCOMPONENT" =>
        {
            NodeClass::Ignore
        }
        _ if dxf.starts_with("BLOCK")
            && (dxf.ends_with("ACTION") || dxf.ends_with("PARAMETER")) =>
        {
            NodeClass::Unsupported
        }
        _ => NodeClass::Ignore,
    }
}

struct RawNode {
    id: i64,
    parent: i64,
    dxf: String,
    payload: *mut c_void,
}

fn parameter_for_action(
    action: &RawNode,
    params: &[(i64, String)],
    built: &[AcadParam],
) -> Option<String> {
    if action.parent >= 0 {
        if let Some((_, name)) = params.iter().find(|(id, _)| *id == action.parent) {
            return Some(name.clone());
        }
    }
    let want = match action.dxf.as_str() {
        "BLOCKSTRETCHACTION" | "BLOCKMOVEACTION" => "linear",
        "BLOCKFLIPACTION" => "flip",
        "BLOCKROTATEACTION" => "rotation",
        _ => return None,
    };
    let matches: Vec<&AcadParam> = built
        .iter()
        .filter(|parameter| match (want, parameter) {
            ("linear", AcadParam::Linear { .. } | AcadParam::Point { .. })
            | ("flip", AcadParam::Flip { .. })
            | ("rotation", AcadParam::Rotation { .. }) => true,
            _ => false,
        })
        .collect();
    if matches.len() == 1 {
        Some(matches[0].name().to_string())
    } else {
        None
    }
}

fn parameter_from_action_nodes(action: &RawNode, names: &[(i64, String)]) -> Option<String> {
    let listed = get_array_field::<u32, u32>(action.payload, &action.dxf, "num_actions", "actions");
    one_named_id(listed.into_iter().map(i64::from), names)
}

fn parameter_from_props(
    action: &RawNode,
    params: &[&RawNode],
    names: &[(i64, String)],
) -> Option<String> {
    if action.id <= 0 {
        return None;
    }
    let mut found = None;
    for param in params {
        if !prop_lists_node(param, action.id) {
            continue;
        }
        let Some((_, name)) = names.iter().find(|(id, _)| *id == param.id) else {
            continue;
        };
        if found
            .as_ref()
            .is_some_and(|current: &String| current != name)
        {
            return None;
        }
        found = Some(name.clone());
    }
    found
}

fn parameter_from_connection_codes(action: &RawNode, names: &[(i64, String)]) -> Option<String> {
    one_named_id(
        connection_codes(action.payload, &action.dxf).into_iter(),
        names,
    )
}

fn one_named_id(ids: impl IntoIterator<Item = i64>, names: &[(i64, String)]) -> Option<String> {
    let mut found = None;
    for id in ids {
        if id <= 0 {
            continue;
        }
        let Some((_, name)) = names.iter().find(|(node, _)| *node == id) else {
            continue;
        };
        if found
            .as_ref()
            .is_some_and(|current: &String| current != name)
        {
            return None;
        }
        found = Some(name.clone());
    }
    found
}

fn prop_lists_node(param: &RawNode, node_id: i64) -> bool {
    if param.payload.is_null() {
        return false;
    }
    for prop in ["prop1", "prop2", "prop3", "prop4"] {
        let Some(field) = entity_field(&param.dxf, prop) else {
            continue;
        };
        let base = unsafe { param.payload.byte_add(field.offset as usize) };
        let Some(count) = read_count(base, "BLOCKPARAMETER_PropInfo", "num_connections") else {
            continue;
        };
        let Some(pointer) = read_ptr::<u8>(base, "BLOCKPARAMETER_PropInfo", "connections") else {
            continue;
        };
        let Some(stride) = subclass_stride("BLOCKPARAMETER_connection") else {
            continue;
        };
        if stride == 0 {
            continue;
        }
        let count = count.min(16);
        for index in 0..count {
            let record = unsafe { pointer.byte_add(index as usize * stride) };
            let code =
                read_pod::<u32>(record.cast(), "BLOCKPARAMETER_connection", "code").unwrap_or(0);
            if i64::from(code) == node_id {
                return true;
            }
        }
    }
    false
}

fn connection_codes(payload: *mut c_void, dxf: &str) -> Vec<i64> {
    let Some(field) = entity_field(dxf, "conn_pts") else {
        return Vec::new();
    };
    let Some(stride) = subclass_stride("BLOCKACTION_connectionpts") else {
        return Vec::new();
    };
    if stride == 0 || field.size as usize == 0 || payload.is_null() {
        return Vec::new();
    }
    let count = (field.size as usize / stride).min(6);
    let base = unsafe { payload.byte_add(field.offset as usize) };
    let mut codes = Vec::new();
    for index in 0..count {
        let point = unsafe { base.byte_add(index * stride) };
        let code = read_pod::<u32>(point, "BLOCKACTION_connectionpts", "code").unwrap_or(0);
        if code != 0 {
            codes.push(i64::from(code));
        }
    }
    codes
}

fn parameter_from_deps(
    action: &RawNode,
    params: &[&RawNode],
    names: &[(i64, String)],
) -> Option<String> {
    let deps = get_array_field::<u32, *mut libredwg_sys::Dwg_Object_Ref>(
        action.payload,
        &action.dxf,
        "num_deps",
        "deps",
    );
    let mut found = None;
    for reference in deps {
        let handle = handle_value(reference);
        if handle == 0 {
            continue;
        }
        for param in params {
            if generic_handle(param.payload) != handle {
                continue;
            }
            let Some((_, name)) = names.iter().find(|(id, _)| *id == param.id) else {
                continue;
            };
            if found
                .as_ref()
                .is_some_and(|current: &String| current != name)
            {
                return None;
            }
            found = Some(name.clone());
        }
    }
    found
}

fn generic_handle(payload: *mut c_void) -> u64 {
    if payload.is_null() {
        return 0;
    }
    unsafe { libredwg_sys::dwg_obj_generic_handlevalue(payload) }
}

unsafe fn read_parameter(node: &RawNode, handles: &HashMap<u64, usize>) -> Option<AcadParam> {
    let payload = node.payload;
    if payload.is_null() {
        return None;
    }
    let dxf = node.dxf.as_str();
    let show = get_field::<u8>(payload, dxf, "show_properties").unwrap_or(1) != 0;
    let element_name = get_utf8_field(payload, dxf, "name").unwrap_or_default();
    Some(match dxf {
        "BLOCKVISIBILITYPARAMETER" => AcadParam::Visibility {
            name: named(
                &get_utf8_field(payload, dxf, "blockvisi_name").unwrap_or_default(),
                &element_name,
                "Visibility",
            ),
            show,
            states: read_visibility_states(payload, handles),
        },
        "BLOCKLINEARPARAMETER" => {
            let base = point_field(payload, dxf, "def_basept")?;
            let end = point_field(payload, dxf, "def_endpt")?;
            let default_distance = point_distance(base, end);
            let current_distance = current_distance(payload, dxf, base, end, default_distance);
            AcadParam::Linear {
                name: named(
                    &get_utf8_field(payload, dxf, "distance_name").unwrap_or_default(),
                    &element_name,
                    "Distance",
                ),
                show,
                base,
                end,
                default_distance,
                current_distance,
                value_set: read_value_set(payload, dxf, "value_set", false),
            }
        }
        "BLOCKFLIPPARAMETER" => AcadParam::Flip {
            name: named(&element_name, "", "Flip"),
            show,
            axis_a: point_field(payload, dxf, "def_basept")?,
            axis_b: point_field(payload, dxf, "def_endpt")?,
            base_label: get_utf8_field(payload, dxf, "base_state_label").unwrap_or_default(),
            flipped_label: get_utf8_field(payload, dxf, "flipped_state_label").unwrap_or_default(),
        },
        "BLOCKROTATIONPARAMETER" => {
            let angle = angle_to_degrees(get_field::<f64>(payload, dxf, "angle").unwrap_or(0.0));
            let current = angle_to_degrees(current_angle(payload, dxf, angle));
            AcadParam::Rotation {
                name: named(
                    &get_utf8_field(payload, dxf, "angle_name").unwrap_or_default(),
                    &element_name,
                    "Angle",
                ),
                show,
                angle_deg: angle,
                current_deg: current,
                value_set: read_value_set(payload, dxf, "angle_value_set", true),
            }
        }
        "BLOCKPOINTPARAMETER" => {
            let base = point_field(payload, dxf, "def_pt").unwrap_or(Point2::new(0.0, 0.0));
            AcadParam::Point {
                name: named(
                    &get_utf8_field(payload, dxf, "position_name").unwrap_or_default(),
                    &element_name,
                    "Position",
                ),
                show,
                current: current_point(payload, dxf, base),
                base,
            }
        }
        _ => return None,
    })
}

unsafe fn read_action(
    dwg: *mut libredwg_sys::Dwg_Data,
    node: &RawNode,
    parameter: &str,
    handles: &HashMap<u64, usize>,
    unresolved: &mut usize,
) -> Option<AcadAction> {
    let payload = node.payload;
    if payload.is_null() {
        return None;
    }
    let dxf = node.dxf.as_str();
    let connection = first_connection(payload, dxf);
    let offset_x = get_field::<f64>(payload, dxf, "action_offset_x").unwrap_or(1.0);
    let mut stretch_targets = if dxf == "BLOCKSTRETCHACTION" {
        read_stretch_targets(payload, handles, unresolved)
    } else {
        Vec::new()
    };
    if dxf == "BLOCKSTRETCHACTION" && stretch_targets.is_empty() {
        for entity in resolve_deps(dwg, payload, dxf, handles, unresolved) {
            stretch_targets.push(AcadStretchTarget {
                entity,
                indexes: Vec::new(),
            });
        }
    }
    Some(match dxf {
        "BLOCKSTRETCHACTION" => AcadAction::Stretch {
            parameter: parameter.to_string(),
            offset_x,
            offset_y: get_field::<f64>(payload, dxf, "action_offset_y").unwrap_or(1.0),
            connection,
            targets: stretch_targets,
        },
        "BLOCKMOVEACTION" => AcadAction::Move {
            parameter: parameter.to_string(),
            offset_x,
            offset_y: get_field::<f64>(payload, dxf, "action_offset_y").unwrap_or(1.0),
            connection,
            entities: resolve_deps(dwg, payload, dxf, handles, unresolved),
        },
        "BLOCKFLIPACTION" => AcadAction::Flip {
            parameter: parameter.to_string(),
            entities: resolve_deps(dwg, payload, dxf, handles, unresolved),
        },
        "BLOCKROTATEACTION" => AcadAction::Rotate {
            parameter: parameter.to_string(),
            pivot: point_field(payload, dxf, "base_pt").unwrap_or(Point2::new(0.0, 0.0)),
            angle_offset: get_field::<f64>(payload, dxf, "angle_offset").unwrap_or(0.0),
            entities: resolve_deps(dwg, payload, dxf, handles, unresolved),
        },
        _ => return None,
    })
}

fn named(primary: &str, secondary: &str, fallback: &str) -> String {
    for candidate in [primary, secondary] {
        let trimmed = candidate.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    fallback.to_string()
}

unsafe fn current_point(payload: *mut c_void, dxf: &str, base: Point2) -> Point2 {
    for field in ["pt", "upd_pt", "updated_pt"] {
        let Some(point) = point_field(payload, dxf, field) else {
            continue;
        };
        if point.x.is_finite() && point.y.is_finite() {
            return point;
        }
    }
    base
}

fn point_distance(base: Point2, end: Point2) -> f64 {
    let delta = end - base;
    (delta.x * delta.x + delta.y * delta.y).sqrt()
}

fn current_distance(
    payload: *mut c_void,
    dxf: &str,
    base: Point2,
    end: Point2,
    default_distance: f64,
) -> f64 {
    let upd = match (
        point_field(payload, dxf, "upd_basept"),
        point_field(payload, dxf, "upd_endpt"),
    ) {
        (Some(start), Some(finish)) => point_distance(start, finish),
        _ => default_distance,
    };
    let field = get_field::<f64>(payload, dxf, "distance").unwrap_or(default_distance);
    if upd.is_finite() && (upd - point_distance(base, end)).abs() > 1e-4 {
        upd
    } else if field.is_finite() {
        field
    } else {
        default_distance
    }
}

fn current_angle(payload: *mut c_void, dxf: &str, fallback_degrees: f64) -> f64 {
    get_field::<f64>(payload, dxf, "angle").unwrap_or(fallback_degrees.to_radians())
}

fn angle_to_degrees(raw: f64) -> f64 {
    if !raw.is_finite() {
        return 0.0;
    }
    if raw.abs() > std::f64::consts::TAU + 1e-3 {
        raw
    } else {
        raw.to_degrees()
    }
}

fn point_field(payload: *mut c_void, dxf: &str, field: &str) -> Option<Point2> {
    let point = get_field::<Point3D>(payload, dxf, field)?;
    if !point.x.is_finite() || !point.y.is_finite() {
        return None;
    }
    Some(Point2::new(point.x, point.y))
}

fn read_value_set(payload: *mut c_void, dxf: &str, field: &str, angles: bool) -> AcadValueSet {
    let Some(value_set) = embedded_object(payload, dxf, field) else {
        return AcadValueSet::default();
    };
    let flags = read_pod::<u32>(value_set, "BLOCKPARAMVALUESET", "flags").unwrap_or(0);
    let convert = |value: f64| {
        if angles {
            angle_to_degrees(value)
        } else {
            value
        }
    };
    let mut set = AcadValueSet::default();
    if flags & VALUE_SET_MIN != 0 {
        set.minimum = read_pod::<f64>(value_set, "BLOCKPARAMVALUESET", "minimum").map(convert);
    }
    if flags & VALUE_SET_MAX != 0 {
        set.maximum = read_pod::<f64>(value_set, "BLOCKPARAMVALUESET", "maximum").map(convert);
    }
    if flags & VALUE_SET_INCREMENT != 0 {
        set.increment =
            read_pod::<f64>(value_set, "BLOCKPARAMVALUESET", "increment").map(|value| {
                if angles {
                    value.to_degrees().abs()
                } else {
                    value
                }
            });
    }
    if flags & VALUE_SET_LIST != 0 {
        set.list = read_f64_list(
            value_set,
            "BLOCKPARAMVALUESET",
            "num_valuelist",
            "valuelist",
        )
        .into_iter()
        .map(convert)
        .filter(|value| value.is_finite())
        .take(MAX_DYNAMIC_STATES)
        .collect();
    }
    set
}

fn read_visibility_states(
    payload: *mut c_void,
    handles: &HashMap<u64, usize>,
) -> Vec<AcadVisibilityState> {
    let count = get_field::<u32>(payload, "BLOCKVISIBILITYPARAMETER", "num_states").unwrap_or(0);
    let Some(states) = get_field::<*mut u8>(payload, "BLOCKVISIBILITYPARAMETER", "states") else {
        return Vec::new();
    };
    if states.is_null() || count == 0 {
        return Vec::new();
    }
    let Some(stride) = subclass_stride("BLOCKVISIBILITYPARAMETER_state") else {
        return Vec::new();
    };
    let count = count.min(MAX_DYNAMIC_STATES as u32);
    let mut result = Vec::new();
    for index in 0..count {
        let state = unsafe { states.byte_add(index as usize * stride) };
        let name = read_string(state.cast(), "BLOCKVISIBILITYPARAMETER_state", "name");
        let entities = read_handle_values(
            state.cast(),
            "BLOCKVISIBILITYPARAMETER_state",
            "num_blocks",
            "blocks",
        )
        .into_iter()
        .filter_map(|handle| handles.get(&handle).copied())
        .take(MAX_DYNAMIC_HANDLES)
        .collect();
        result.push(AcadVisibilityState { name, entities });
    }
    result
}

fn read_stretch_targets(
    payload: *mut c_void,
    handles: &HashMap<u64, usize>,
    unresolved: &mut usize,
) -> Vec<AcadStretchTarget> {
    let count = get_field::<u32>(payload, "BLOCKSTRETCHACTION", "num_hdls").unwrap_or(0);
    let Some(records) = get_field::<*mut u8>(payload, "BLOCKSTRETCHACTION", "hdls") else {
        return Vec::new();
    };
    if records.is_null() || count == 0 {
        return Vec::new();
    }
    let Some(stride) = subclass_stride("BLOCKSTRETCHACTION_handles") else {
        return Vec::new();
    };
    let count = count.min(MAX_DYNAMIC_HANDLES as u32);
    let mut targets = Vec::new();
    for index in 0..count {
        let record = unsafe { records.byte_add(index as usize * stride) };
        let Some(reference) = read_ptr::<libredwg_sys::Dwg_Object_Ref>(
            record.cast(),
            "BLOCKSTRETCHACTION_handles",
            "hdl",
        ) else {
            *unresolved += 1;
            continue;
        };
        let handle = handle_value(reference);
        let Some(entity) = handles.get(&handle).copied() else {
            *unresolved += 1;
            continue;
        };
        let indexes = read_u32_list(
            record.cast(),
            "BLOCKSTRETCHACTION_handles",
            "num_indexes",
            "indexes",
        );
        targets.push(AcadStretchTarget {
            entity,
            indexes: indexes.into_iter().take(MAX_STRETCH_INDEXES).collect(),
        });
    }
    targets
}

fn resolve_deps(
    dwg: *mut libredwg_sys::Dwg_Data,
    payload: *mut c_void,
    dxf: &str,
    handles: &HashMap<u64, usize>,
    unresolved: &mut usize,
) -> Vec<usize> {
    let references =
        get_array_field::<u32, *mut libredwg_sys::Dwg_Object_Ref>(payload, dxf, "num_deps", "deps");
    let mut entities = Vec::new();
    for reference in references.into_iter().take(MAX_DYNAMIC_HANDLES) {
        let handle = handle_value(reference);
        if handle == 0 {
            continue;
        }
        if let Some(index) = handles.get(&handle) {
            entities.push(*index);
            continue;
        }
        let object = ref_object(dwg, reference);
        if object.is_null() {
            *unresolved += 1;
            continue;
        }
        let mut error = 0;
        let entity = unsafe { libredwg_sys::dwg_object_to_entity(object, &mut error) };
        if !entity.is_null() {
            *unresolved += 1;
        }
    }
    entities
}

fn first_connection(payload: *mut c_void, dxf: &str) -> String {
    let Some(field) = entity_field(dxf, "conn_pts") else {
        return String::new();
    };
    let Some(stride) = subclass_stride("BLOCKACTION_connectionpts") else {
        return String::new();
    };
    if stride == 0 || field.size as usize == 0 {
        return String::new();
    }
    let count = (field.size as usize / stride).min(4);
    let base = unsafe { payload.byte_add(field.offset as usize) };
    for index in 0..count {
        let point = unsafe { base.byte_add(index * stride) };
        let name = read_string(point, "BLOCKACTION_connectionpts", "name");
        if !name.is_empty() {
            return name;
        }
    }
    String::new()
}

fn handle_index(handles: Option<&[u64]>) -> HashMap<u64, usize> {
    let mut index = HashMap::new();
    if let Some(handles) = handles {
        for (entity, handle) in handles.iter().enumerate() {
            if *handle != 0 {
                index.insert(*handle, entity);
            }
        }
    }
    index
}

unsafe fn graph_nodes(
    dwg: *mut libredwg_sys::Dwg_Data,
    graph: *mut libredwg_sys::Dwg_Object,
) -> Vec<RawNode> {
    let payload = unsafe { libredwg_sys::uncad_object_object_ptr(graph) };
    if payload.is_null() {
        return Vec::new();
    }
    let count = get_field::<u32>(payload, "EVALUATION_GRAPH", "num_nodes").unwrap_or(0) as usize;
    let count = count.min(MAX_DYNAMIC_NODES);
    let Some(nodes) = get_field::<*mut u8>(payload, "EVALUATION_GRAPH", "nodes") else {
        return Vec::new();
    };
    if nodes.is_null() {
        return Vec::new();
    }
    let Some(stride) = subclass_stride("EVAL_Node") else {
        return Vec::new();
    };
    let Some(eval_offset) = subclass_offset("EVAL_Node", "evalexpr") else {
        return Vec::new();
    };
    let mut raw = Vec::new();
    for index in 0..count {
        let node = unsafe { nodes.byte_add(index * stride) };
        let evalexpr = unsafe {
            node.byte_add(eval_offset)
                .cast::<*mut libredwg_sys::Dwg_Object_Ref>()
                .read()
        };
        let object = ref_object(dwg, evalexpr);
        if object.is_null() {
            continue;
        }
        let object_payload = unsafe { libredwg_sys::uncad_object_object_ptr(object) };
        let dxf = object_dxfname(object);
        let (id, parent) = eval_ids(object_payload, &dxf);
        let id = if id >= 0 {
            id
        } else {
            read_pod::<u32>(node.cast(), "EVAL_Node", "id").unwrap_or(index as u32) as i64
        };
        raw.push(RawNode {
            id,
            parent,
            dxf,
            payload: object_payload,
        });
    }
    raw
}

fn eval_ids(payload: *mut c_void, dxf: &str) -> (i64, i64) {
    let Some(eval) = embedded_object(payload, dxf, "evalexpr") else {
        return (-1, -1);
    };
    let parent = read_pod::<i32>(eval, "EvalExpr", "parentid").unwrap_or(-1) as i64;
    let node = read_pod::<u32>(eval, "EvalExpr", "nodeid").unwrap_or(0) as i64;
    (node, parent)
}

fn representation_source(dwg: *mut libredwg_sys::Dwg_Data, owner: *mut c_void) -> Option<String> {
    let dict = extension_dictionary(dwg, owner)?;
    let representation = find_named(dwg, dict, "BLOCKREPRESENTATION")?;
    let payload = unsafe { libredwg_sys::uncad_object_object_ptr(representation) };
    if payload.is_null() {
        return None;
    }
    get_field::<*mut libredwg_sys::Dwg_Object_Ref>(payload, "BLOCKREPRESENTATION", "block")
        .and_then(resolve_block_name)
        .filter(|name| !name.is_empty())
}

pub(crate) fn extension_dictionary(
    dwg: *mut libredwg_sys::Dwg_Data,
    owner: *mut c_void,
) -> Option<*mut libredwg_sys::Dwg_Object> {
    let reference = get_common_field::<*mut libredwg_sys::Dwg_Object_Ref>(owner, "xdicobjhandle")?;
    let object = ref_object(dwg, reference);
    if object.is_null() {
        return None;
    }
    let dxf = object_dxfname(object);
    if dxf.eq_ignore_ascii_case("DICTIONARY") || dxf.eq_ignore_ascii_case("DICTIONARYWDFLT") {
        Some(object)
    } else {
        None
    }
}

pub(crate) fn find_named(
    dwg: *mut libredwg_sys::Dwg_Data,
    object: *mut libredwg_sys::Dwg_Object,
    want: &str,
) -> Option<*mut libredwg_sys::Dwg_Object> {
    let mut seen = HashSet::new();
    find_named_in(dwg, object, want, 0, &mut seen)
}

fn find_named_in(
    dwg: *mut libredwg_sys::Dwg_Data,
    object: *mut libredwg_sys::Dwg_Object,
    want: &str,
    depth: u32,
    seen: &mut HashSet<usize>,
) -> Option<*mut libredwg_sys::Dwg_Object> {
    if object.is_null() || depth > MAX_DICT_DEPTH || !seen.insert(object as usize) {
        return None;
    }
    if object_dxfname(object).eq_ignore_ascii_case(want) {
        return Some(object);
    }
    let dxf = object_dxfname(object);
    if !dxf.eq_ignore_ascii_case("DICTIONARY") && !dxf.eq_ignore_ascii_case("DICTIONARYWDFLT") {
        return None;
    }
    let payload = unsafe { libredwg_sys::uncad_object_object_ptr(object) };
    for (_key, child) in dict_entries(dwg, payload).into_iter().take(MAX_DICT_ITEMS) {
        if let Some(found) = find_named_in(dwg, child, want, depth + 1, seen) {
            return Some(found);
        }
    }
    None
}

fn dict_entries(
    dwg: *mut libredwg_sys::Dwg_Data,
    dict: *mut c_void,
) -> Vec<(String, *mut libredwg_sys::Dwg_Object)> {
    if dict.is_null() {
        return Vec::new();
    }
    for dxf in ["DICTIONARY", "DICTIONARYWDFLT"] {
        let texts = get_array_field::<u32, *mut c_char>(dict, dxf, "numitems", "texts");
        let handles = get_array_field::<u32, *mut libredwg_sys::Dwg_Object_Ref>(
            dict,
            dxf,
            "numitems",
            "itemhandles",
        );
        if texts.is_empty() && handles.is_empty() {
            continue;
        }
        let count = texts.len().min(handles.len()).min(MAX_DICT_ITEMS);
        let mut entries = Vec::with_capacity(count);
        for index in 0..count {
            let name = bounded_text(texts[index], MAX_NAME_CHARS).unwrap_or_default();
            let object = ref_object(dwg, handles[index]);
            if !object.is_null() {
                entries.push((name, object));
            }
        }
        return entries;
    }
    Vec::new()
}

fn ref_object(
    dwg: *mut libredwg_sys::Dwg_Data,
    reference: *mut libredwg_sys::Dwg_Object_Ref,
) -> *mut libredwg_sys::Dwg_Object {
    if reference.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        let existing = (*reference).obj;
        if !existing.is_null() {
            return existing.cast();
        }
        libredwg_sys::dwg_ref_object(dwg, reference)
    }
}

fn handle_value(reference: *mut libredwg_sys::Dwg_Object_Ref) -> u64 {
    if reference.is_null() {
        return 0;
    }
    unsafe {
        let absolute = (*reference).absolute_ref;
        if absolute != 0 {
            absolute
        } else {
            (*reference).handleref.value
        }
    }
}

fn entity_field(dxf: &str, field: &str) -> Option<libredwg_sys::Dwg_DYNAPI_field> {
    let dxf = CString::new(dxf).ok()?;
    let field = CString::new(field).ok()?;
    let desc = unsafe { libredwg_sys::dwg_dynapi_entity_field(dxf.as_ptr(), field.as_ptr()) };
    if desc.is_null() {
        None
    } else {
        Some(unsafe { *desc })
    }
}

fn subclass_field(subclass: &str, field: &str) -> Option<libredwg_sys::Dwg_DYNAPI_field> {
    let subclass = CString::new(subclass).ok()?;
    let field = CString::new(field).ok()?;
    let desc =
        unsafe { libredwg_sys::dwg_dynapi_subclass_field(subclass.as_ptr(), field.as_ptr()) };
    if desc.is_null() {
        None
    } else {
        Some(unsafe { *desc })
    }
}

fn subclass_stride(name: &str) -> Option<usize> {
    let name = CString::new(name).ok()?;
    let size = unsafe { libredwg_sys::dwg_dynapi_fields_size(name.as_ptr()) };
    if size <= 0 {
        None
    } else {
        Some(size as usize)
    }
}

fn subclass_offset(subclass: &str, field: &str) -> Option<usize> {
    Some(subclass_field(subclass, field)?.offset as usize)
}

fn read_pod<T: Copy>(base: *mut c_void, subclass: &str, field: &str) -> Option<T> {
    let desc = subclass_field(subclass, field)?;
    if desc.size as usize != std::mem::size_of::<T>() || desc.is_malloc() != 0 {
        return None;
    }
    if base.is_null() {
        return None;
    }
    let ptr = unsafe { base.byte_add(desc.offset as usize) };
    Some(unsafe { ptr.cast::<T>().read() })
}

fn read_ptr<T>(base: *mut c_void, subclass: &str, field: &str) -> Option<*mut T> {
    let desc = subclass_field(subclass, field)?;
    if desc.size as usize != std::mem::size_of::<*mut T>() || base.is_null() {
        return None;
    }
    let slot = unsafe { base.byte_add(desc.offset as usize).cast::<*mut T>().read() };
    if slot.is_null() {
        None
    } else {
        Some(slot)
    }
}

fn read_count(base: *mut c_void, subclass: &str, field: &str) -> Option<u32> {
    let desc = subclass_field(subclass, field)?;
    match desc.size {
        2 => read_pod::<u16>(base, subclass, field).map(u32::from),
        4 => read_pod::<u32>(base, subclass, field),
        _ => None,
    }
}

fn read_string(base: *mut c_void, subclass: &str, field: &str) -> String {
    read_ptr::<c_char>(base, subclass, field)
        .and_then(|text| bounded_text(text, MAX_NAME_CHARS))
        .unwrap_or_default()
}

fn read_handle_values(
    base: *mut c_void,
    subclass: &str,
    count_field: &str,
    array_field: &str,
) -> Vec<u64> {
    let Some(count) = read_count(base, subclass, count_field) else {
        return Vec::new();
    };
    let count = count.min(MAX_DYNAMIC_HANDLES as u32) as usize;
    let Some(pointer) = read_ptr::<*mut libredwg_sys::Dwg_Object_Ref>(base, subclass, array_field)
    else {
        return Vec::new();
    };
    let slice = unsafe { std::slice::from_raw_parts(pointer, count) };
    slice
        .iter()
        .copied()
        .map(handle_value)
        .filter(|handle| *handle != 0)
        .collect()
}

fn read_u32_list(
    base: *mut c_void,
    subclass: &str,
    count_field: &str,
    array_field: &str,
) -> Vec<u32> {
    let Some(count) = read_count(base, subclass, count_field) else {
        return Vec::new();
    };
    let count = count.min(MAX_STRETCH_INDEXES as u32) as usize;
    let Some(pointer) = read_ptr::<u32>(base, subclass, array_field) else {
        return Vec::new();
    };
    unsafe { std::slice::from_raw_parts(pointer, count) }.to_vec()
}

fn read_f64_list(
    base: *mut c_void,
    subclass: &str,
    count_field: &str,
    array_field: &str,
) -> Vec<f64> {
    let Some(count) = read_count(base, subclass, count_field) else {
        return Vec::new();
    };
    let count = count.min(MAX_DYNAMIC_STATES as u32) as usize;
    let Some(pointer) = read_ptr::<f64>(base, subclass, array_field) else {
        return Vec::new();
    };
    unsafe { std::slice::from_raw_parts(pointer, count) }.to_vec()
}

fn bounded_text(text: *const c_char, max: usize) -> Option<String> {
    if text.is_null() || max == 0 {
        return None;
    }
    let mut length = 0usize;
    while length < max {
        if unsafe { *text.add(length) } == 0 {
            break;
        }
        length += 1;
    }
    if length == max {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(text.cast::<u8>(), length) };
    let owned = String::from_utf8_lossy(bytes).trim().to_string();
    if owned.is_empty() {
        None
    } else {
        Some(owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_core::{Geometry, Point3};

    fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> Entity {
        Entity::new(Geometry::Line {
            start: Point3::from_xy(x0, y0),
            end: Point3::from_xy(x1, y1),
        })
    }

    fn empty_graph(name: &str) -> AcadGraph {
        AcadGraph {
            block_name: name.into(),
            parameters: Vec::new(),
            actions: Vec::new(),
            unsupported: Vec::new(),
            unresolved: 0,
        }
    }

    fn install(
        document: &mut Document,
        name: &str,
        entities: Vec<Entity>,
        dynamic: DynamicDefinition,
    ) {
        let mut block = cad_core::BlockDefinition::plain(name, Point3::default(), entities);
        block.dynamic = Some(dynamic);
        document.replace_block_definition(block);
    }

    fn evaluate(
        document: &Document,
        name: &str,
        configuration: Option<&InstanceConfiguration>,
    ) -> Vec<Entity> {
        let block = document.block_by_name(name).expect("block");
        let request = EvaluationRequest {
            generation: document.content_generation(),
        };
        evaluate_definition(
            document,
            block,
            configuration,
            &mut EvaluationCache::default(),
            request,
        )
        .expect("evaluate")
        .entities
        .clone()
    }

    #[test]
    fn stretch_moves_the_far_end_and_leaves_the_default_alone() {
        let mut document = Document::default();
        let mut entities = vec![line(0.0, 0.0, 10.0, 0.0)];
        let mut graph = empty_graph("Frame");
        graph.parameters.push(AcadParam::Linear {
            name: "Width".into(),
            show: true,
            base: Point2::new(0.0, 0.0),
            end: Point2::new(10.0, 0.0),
            default_distance: 10.0,
            current_distance: 10.0,
            value_set: AcadValueSet {
                list: vec![10.0, 20.0],
                ..AcadValueSet::default()
            },
        });
        graph.actions.push(AcadAction::Stretch {
            parameter: "Width".into(),
            offset_x: 1.0,
            offset_y: 0.0,
            connection: String::new(),
            targets: vec![AcadStretchTarget {
                entity: 0,
                indexes: vec![1],
            }],
        });
        let dynamic = materialize_definition(&mut document, &mut entities, &graph).expect("map");
        let ParameterKind::Number(numeric) = &dynamic.parameters[0].kind else {
            panic!("number");
        };
        match &numeric.domain {
            NumericDomain::AllowedValues(list) => {
                assert!(list.iter().any(|value| (*value - 10.0).abs() < 1e-6));
            }
            other => panic!("{other:?}"),
        }
        install(&mut document, "Frame", entities, dynamic);
        let parameter = document
            .block_by_name("Frame")
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
            .parameters[0]
            .id;
        let drawn = evaluate(&document, "Frame", None);
        match &drawn[0].geometry {
            Geometry::Line { end, .. } => assert!((end.x - 10.0).abs() < 1e-6),
            other => panic!("{other:?}"),
        }
        let mut configuration = InstanceConfiguration::default();
        configuration.set(parameter, ParameterValue::Number(20.0));
        let stretched = evaluate(&document, "Frame", Some(&configuration));
        match &stretched[0].geometry {
            Geometry::Line { start, end } => {
                assert!((start.x).abs() < 1e-6);
                assert!((end.x - 20.0).abs() < 1e-6);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn point_parameter_moves_in_x_and_y() {
        let mut document = Document::default();
        let mut entities = vec![line(0.0, 0.0, 10.0, 0.0), line(0.0, 0.0, 0.0, 4.0)];
        let mut graph = empty_graph("Bend");
        graph.parameters.push(AcadParam::Point {
            name: "Position".into(),
            show: true,
            base: Point2::new(0.0, 0.0),
            current: Point2::new(0.0, 0.0),
        });
        graph.actions.push(AcadAction::Move {
            parameter: "Position".into(),
            offset_x: 1.0,
            offset_y: 1.0,
            connection: String::new(),
            entities: vec![0],
        });
        graph.actions.push(AcadAction::Stretch {
            parameter: "Position".into(),
            offset_x: 1.0,
            offset_y: 1.0,
            connection: String::new(),
            targets: vec![AcadStretchTarget {
                entity: 1,
                indexes: vec![1],
            }],
        });
        let dynamic = materialize_definition(&mut document, &mut entities, &graph).expect("map");
        assert_eq!(dynamic.parameters[0].name, "Position X");
        assert_eq!(dynamic.parameters[1].name, "Position Y");
        let x_id = dynamic.parameters[0].id;
        let y_id = dynamic.parameters[1].id;
        install(&mut document, "Bend", entities, dynamic);
        let drawn = evaluate(&document, "Bend", None);
        match &drawn[0].geometry {
            Geometry::Line { start, end } => {
                assert!(start.x.abs() < 1e-6 && start.y.abs() < 1e-6);
                assert!((end.x - 10.0).abs() < 1e-6 && end.y.abs() < 1e-6);
            }
            other => panic!("{other:?}"),
        }
        let mut configuration = InstanceConfiguration::default();
        configuration.set(x_id, ParameterValue::Number(5.0));
        configuration.set(y_id, ParameterValue::Number(-3.0));
        let moved = evaluate(&document, "Bend", Some(&configuration));
        match &moved[0].geometry {
            Geometry::Line { start, end } => {
                assert!((start.x - 5.0).abs() < 1e-6);
                assert!((start.y + 3.0).abs() < 1e-6);
                assert!((end.x - 15.0).abs() < 1e-6);
                assert!((end.y + 3.0).abs() < 1e-6);
            }
            other => panic!("{other:?}"),
        }
        match &moved[1].geometry {
            Geometry::Line { start, end } => {
                assert!(start.x.abs() < 1e-6 && start.y.abs() < 1e-6);
                assert!((end.x - 5.0).abs() < 1e-6);
                assert!((end.y - 1.0).abs() < 1e-6);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn visibility_default_matches_the_drawn_state() {
        let mut document = Document::default();
        let mut hidden = line(0.0, 1.0, 2.0, 1.0);
        hidden.visible = false;
        let mut entities = vec![line(0.0, 0.0, 2.0, 0.0), hidden];
        let mut graph = empty_graph("Door");
        graph.parameters.push(AcadParam::Visibility {
            name: "View".into(),
            show: true,
            states: vec![
                AcadVisibilityState {
                    name: "Open".into(),
                    entities: vec![0],
                },
                AcadVisibilityState {
                    name: "Closed".into(),
                    entities: vec![1],
                },
            ],
        });
        let dynamic = materialize_definition(&mut document, &mut entities, &graph).expect("map");
        let closed_option = {
            let ParameterKind::Choice(choice) = &dynamic.parameters[0].kind else {
                panic!("choice");
            };
            assert_eq!(choice.options[0].label, "Open");
            assert_eq!(choice.default, choice.options[0].id);
            choice.options[1].id
        };
        install(&mut document, "Door", entities, dynamic);
        let drawn = evaluate(&document, "Door", None);
        assert!(drawn[0].visible);
        assert!(!drawn[1].visible);
        let mut configuration = InstanceConfiguration::default();
        configuration.set(
            document
                .block_by_name("Door")
                .unwrap()
                .dynamic
                .as_ref()
                .unwrap()
                .parameters[0]
                .id,
            ParameterValue::Choice(closed_option),
        );
        let closed = evaluate(&document, "Door", Some(&configuration));
        assert!(!closed[0].visible);
        assert!(closed[1].visible);
    }

    #[test]
    fn flip_mirrors_across_the_axis_when_checked() {
        let mut document = Document::default();
        let mut entities = vec![line(1.0, 0.0, 2.0, 0.0)];
        let mut graph = empty_graph("Panel");
        graph.parameters.push(AcadParam::Flip {
            name: "Hand".into(),
            show: true,
            axis_a: Point2::new(0.0, 0.0),
            axis_b: Point2::new(0.0, 1.0),
            base_label: "Left".into(),
            flipped_label: "Right".into(),
        });
        graph.actions.push(AcadAction::Flip {
            parameter: "Hand".into(),
            entities: vec![0],
        });
        let dynamic = materialize_definition(&mut document, &mut entities, &graph).expect("map");
        install(&mut document, "Panel", entities, dynamic);
        let parameter = document
            .block_by_name("Panel")
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
            .parameters[0]
            .id;
        let mut configuration = InstanceConfiguration::default();
        configuration.set(parameter, ParameterValue::Boolean(true));
        let flipped = evaluate(&document, "Panel", Some(&configuration));
        match &flipped[0].geometry {
            Geometry::Line { start, end } => {
                assert!((start.x + 1.0).abs() < 1e-6, "{start:?}");
                assert!((end.x + 2.0).abs() < 1e-6, "{end:?}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rotation_turns_about_the_pivot() {
        let mut document = Document::default();
        let mut entities = vec![line(1.0, 0.0, 2.0, 0.0)];
        let mut graph = empty_graph("Swing");
        graph.parameters.push(AcadParam::Rotation {
            name: "Angle".into(),
            show: true,
            angle_deg: 0.0,
            current_deg: 0.0,
            value_set: AcadValueSet::default(),
        });
        graph.actions.push(AcadAction::Rotate {
            parameter: "Angle".into(),
            pivot: Point2::new(0.0, 0.0),
            angle_offset: 0.0,
            entities: vec![0],
        });
        let dynamic = materialize_definition(&mut document, &mut entities, &graph).expect("map");
        install(&mut document, "Swing", entities, dynamic);
        let parameter = document
            .block_by_name("Swing")
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
            .parameters[0]
            .id;
        let mut configuration = InstanceConfiguration::default();
        configuration.set(parameter, ParameterValue::Number(90.0));
        let turned = evaluate(&document, "Swing", Some(&configuration));
        match &turned[0].geometry {
            Geometry::Line { start, end } => {
                assert!(
                    (start.x).abs() < 1e-5 && (start.y - 1.0).abs() < 1e-5,
                    "{start:?}"
                );
                assert!(
                    (end.x).abs() < 1e-5 && (end.y - 2.0).abs() < 1e-5,
                    "{end:?}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn array_action_keeps_the_block_static() {
        let mut document = Document::default();
        let mut entities = vec![line(0.0, 0.0, 1.0, 0.0)];
        let mut graph = empty_graph("Arrayed");
        graph.unsupported.push("BLOCKARRAYACTION".into());
        let error = materialize_definition(&mut document, &mut entities, &graph).unwrap_err();
        assert!(error.contains("BLOCKARRAYACTION"), "{error}");
    }

    #[test]
    fn mapped_block_round_trips_through_the_companion() {
        use cad_core::{export_materialized, identity_insert};
        use cad_io::{apply_companion, companion_path, read_companion, write_companion};

        let mut document = Document::default();
        let mut entities = vec![line(0.0, 0.0, 10.0, 0.0)];
        let mut graph = empty_graph("Frame");
        graph.parameters.push(AcadParam::Linear {
            name: "Width".into(),
            show: true,
            base: Point2::new(0.0, 0.0),
            end: Point2::new(10.0, 0.0),
            default_distance: 10.0,
            current_distance: 10.0,
            value_set: AcadValueSet::default(),
        });
        graph.actions.push(AcadAction::Stretch {
            parameter: "Width".into(),
            offset_x: 1.0,
            offset_y: 0.0,
            connection: String::new(),
            targets: vec![AcadStretchTarget {
                entity: 0,
                indexes: vec![1],
            }],
        });
        let dynamic = materialize_definition(&mut document, &mut entities, &graph).expect("map");
        let parameter = dynamic.parameters[0].id;
        install(&mut document, "Frame", entities, dynamic);
        let mut insert = Entity::new(identity_insert("Frame".into(), Point3::from_xy(1.0, 2.0)));
        let mut configuration = InstanceConfiguration::default();
        configuration.set(parameter, ParameterValue::Number(14.0));
        insert
            .geometry
            .set_insert_configuration(Some(configuration));
        document.add_entity(insert);
        let request = EvaluationRequest {
            generation: document.content_generation(),
        };
        let exported = export_materialized(&document, &mut EvaluationCache::default(), request)
            .expect("export");
        let dest = std::env::temp_dir().join(format!(
            "mycad-acad-dynamic-{}.dwg",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let sidecar = companion_path(&dest);
        match crate::write_dwg(&exported.document, &dest) {
            Ok(_) => {
                write_companion(
                    &sidecar,
                    &document,
                    &exported.document,
                    &exported.links,
                    &exported.generated_blocks,
                )
                .expect("companion");
                let mut loaded = crate::import_dwg(&dest).expect("import");
                let companion = read_companion(&sidecar).expect("read companion");
                let report = apply_companion(&mut loaded, companion);
                let _ = std::fs::remove_file(&dest);
                let _ = std::fs::remove_file(&sidecar);
                assert_eq!(report.skipped, 0, "{report:?}");
                let definition = loaded.block_by_name("Frame").expect("definition");
                assert!(definition.is_dynamic(), "{report:?}");
                assert_eq!(
                    definition.dynamic.as_ref().unwrap().parameters[0].name,
                    "Width"
                );
                let restored = definition.dynamic.as_ref().unwrap().parameters[0].id;
                let width = loaded.model_space[0]
                    .geometry
                    .insert_configuration()
                    .and_then(|config| config.get(restored))
                    .and_then(|value| value.as_number())
                    .expect("width");
                assert!((width - 14.0).abs() < 1e-6);
            }
            Err(crate::DwgWriteError::Convert(crate::ExportError::Critical { .. })) => {
                let _ = std::fs::remove_file(&dest);
                let _ = std::fs::remove_file(&sidecar);
            }
            Err(other) => {
                let _ = std::fs::remove_file(&dest);
                let _ = std::fs::remove_file(&sidecar);
                panic!("unexpected write_dwg error: {other}");
            }
        }
    }
}
