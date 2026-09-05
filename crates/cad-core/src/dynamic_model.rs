//! Phase 3 dynamic-block types: conditions, text, visibility, transforms,
//! nested inputs, compatibility rules, and presets.
//!
//! Parameter names and product-specific equipment types do not belong here.

use std::collections::BTreeMap;

use crate::dynamic::{
    numbers_equal, DynamicError, FollowRole, GeometryTarget, InstanceConfiguration, ParameterDef,
    ParameterKind, ParameterValue,
};
use crate::entity::EntityId;
use crate::geom::Point2;
use crate::ids::{ActionId, AnchorId, OptionId, ParameterId, PresetId};

// ------------------------------------------------------------
// Type: OccurrencePath
// Purpose: Distinguishes repeated nested INSERTs. A child entity
//          ID alone cannot identify which occurrence was selected.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct OccurrencePath {
    pub inserts: Vec<EntityId>,
}

impl OccurrencePath {
    pub fn leaf(&self) -> Option<EntityId> {
        self.inserts.last().copied()
    }

    pub fn remap(&self, entities: &BTreeMap<EntityId, EntityId>) -> Option<Self> {
        let mut inserts = Vec::with_capacity(self.inserts.len());
        for id in &self.inserts {
            inserts.push(*entities.get(id)?);
        }
        Some(Self { inserts })
    }
}

// ------------------------------------------------------------
// Type: ParameterCondition
// Purpose: One Choice-or-boolean gate. Options within the condition
//          combine with OR; independent conditions combine with AND.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParameterCondition {
    Choice {
        parameter: ParameterId,
        options: Vec<OptionId>,
    },
    Boolean {
        parameter: ParameterId,
        state: bool,
    },
}

impl ParameterCondition {
    pub fn parameter(&self) -> ParameterId {
        match self {
            Self::Choice { parameter, .. } | Self::Boolean { parameter, .. } => *parameter,
        }
    }

    pub fn matches(&self, values: &BTreeMap<ParameterId, ParameterValue>) -> bool {
        match self {
            Self::Choice { parameter, options } => match values.get(parameter) {
                Some(ParameterValue::Choice(current)) => options.contains(current),
                _ => false,
            },
            Self::Boolean { parameter, state } => match values.get(parameter) {
                Some(ParameterValue::Boolean(current)) => *current == *state,
                _ => false,
            },
        }
    }

    pub fn remap(
        &mut self,
        parameters: &BTreeMap<ParameterId, ParameterId>,
        options: &BTreeMap<OptionId, OptionId>,
    ) {
        match self {
            Self::Choice {
                parameter,
                options: ids,
            } => {
                if let Some(mapped) = parameters.get(parameter) {
                    *parameter = *mapped;
                }
                for id in ids.iter_mut() {
                    if let Some(mapped) = options.get(id) {
                        *id = *mapped;
                    }
                }
            }
            Self::Boolean { parameter, .. } => {
                if let Some(mapped) = parameters.get(parameter) {
                    *parameter = *mapped;
                }
            }
        }
    }
}

pub fn conditions_match(
    conditions: &[ParameterCondition],
    values: &BTreeMap<ParameterId, ParameterValue>,
) -> bool {
    conditions.iter().all(|condition| condition.matches(values))
}

// ------------------------------------------------------------
// Type: GeometryGroup
// Purpose: Named, durable set of member entities used by flip,
//          rotate, place, and visibility.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeometryGroup {
    pub id: ActionId,
    pub name: String,
    pub members: Vec<EntityId>,
}

impl GeometryGroup {
    pub fn remap(
        &mut self,
        actions: &BTreeMap<ActionId, ActionId>,
        entities: &BTreeMap<EntityId, EntityId>,
    ) -> Result<(), DynamicError> {
        if let Some(mapped) = actions.get(&self.id) {
            self.id = *mapped;
        }
        let mut members = Vec::with_capacity(self.members.len());
        for id in &self.members {
            let Some(mapped) = entities.get(id) else {
                return Err(DynamicError::MissingEntity {
                    target: GeometryTarget::Entity(*id),
                });
            };
            members.push(*mapped);
        }
        self.members = members;
        Ok(())
    }
}

// ------------------------------------------------------------
// Enum: AnchorFollow / Type: AnchorDef
// Purpose: Named local destination that can track a size parameter.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub enum AnchorFollow {
    Size {
        parameter: ParameterId,
        role: FollowRole,
    },
    Geometry(GeometryTarget),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnchorDef {
    pub id: AnchorId,
    pub name: String,
    pub position: Point2,
    pub orientation: Option<f64>,
    pub follow: Option<AnchorFollow>,
}

impl AnchorDef {
    pub fn remap(
        &mut self,
        parameters: &BTreeMap<ParameterId, ParameterId>,
        anchors: &BTreeMap<AnchorId, AnchorId>,
        entities: &BTreeMap<EntityId, EntityId>,
        vertices: &BTreeMap<crate::ids::VertexId, crate::ids::VertexId>,
    ) -> Result<(), DynamicError> {
        if let Some(mapped) = anchors.get(&self.id) {
            self.id = *mapped;
        }
        if let Some(follow) = &mut self.follow {
            match follow {
                AnchorFollow::Size { parameter, .. } => {
                    if let Some(mapped) = parameters.get(parameter) {
                        *parameter = *mapped;
                    }
                }
                AnchorFollow::Geometry(target) => {
                    *target = target.remap(entities, vertices).ok_or(
                        DynamicError::MissingEntity {
                            target: *target,
                        },
                    )?;
                }
            }
        }
        Ok(())
    }
}

// ------------------------------------------------------------
// Type: VisibilityGroup
// Purpose: Show members only when every condition is true.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub struct VisibilityGroup {
    pub id: ActionId,
    pub name: String,
    pub members: Vec<EntityId>,
    pub conditions: Vec<ParameterCondition>,
}

impl VisibilityGroup {
    pub fn is_active(&self, values: &BTreeMap<ParameterId, ParameterValue>) -> bool {
        conditions_match(&self.conditions, values)
    }

    pub fn remap(
        &mut self,
        parameters: &BTreeMap<ParameterId, ParameterId>,
        options: &BTreeMap<OptionId, OptionId>,
        actions: &BTreeMap<ActionId, ActionId>,
        entities: &BTreeMap<EntityId, EntityId>,
    ) -> Result<(), DynamicError> {
        if let Some(mapped) = actions.get(&self.id) {
            self.id = *mapped;
        }
        for condition in &mut self.conditions {
            condition.remap(parameters, options);
        }
        let mut members = Vec::with_capacity(self.members.len());
        for id in &self.members {
            let Some(mapped) = entities.get(id) else {
                return Err(DynamicError::MissingEntity {
                    target: GeometryTarget::Entity(*id),
                });
            };
            members.push(*mapped);
        }
        self.members = members;
        Ok(())
    }
}

pub fn effective_visibility(
    groups: &[VisibilityGroup],
    entity: EntityId,
    values: &BTreeMap<ParameterId, ParameterValue>,
) -> bool {
    let mut saw_group = false;
    for group in groups {
        if !group.members.contains(&entity) {
            continue;
        }
        saw_group = true;
        if !group.is_active(values) {
            return false;
        }
    }
    let _ = saw_group;
    true
}

pub fn visibility_conditions_for(
    groups: &[VisibilityGroup],
    entity: EntityId,
) -> Vec<&ParameterCondition> {
    groups
        .iter()
        .filter(|group| group.members.contains(&entity))
        .flat_map(|group| group.conditions.iter())
        .collect()
}

// ------------------------------------------------------------
// Enum: TextToken / TextBindingMode / TextReflectPolicy / TextBinding
// Purpose: Substitute evaluated strings without a scripting language.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextToken {
    Literal(String),
    Parameter(ParameterId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextBindingMode {
    ShowValue {
        parameter: ParameterId,
    },
    OptionMap {
        parameter: ParameterId,
        texts: BTreeMap<OptionId, String>,
    },
    Formatted {
        tokens: Vec<TextToken>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextReflectPolicy {
    KeepReadable,
    KeepUpright,
    Mirror,
}

impl Default for TextReflectPolicy {
    fn default() -> Self {
        Self::KeepReadable
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextBinding {
    pub id: ActionId,
    pub target: EntityId,
    pub mode: TextBindingMode,
    pub boolean_true: String,
    pub boolean_false: String,
    pub number_precision: Option<u8>,
}

impl Default for TextBinding {
    fn default() -> Self {
        Self {
            id: ActionId::UNASSIGNED,
            target: EntityId::UNASSIGNED,
            mode: TextBindingMode::ShowValue {
                parameter: ParameterId::UNASSIGNED,
            },
            boolean_true: "On".into(),
            boolean_false: "Off".into(),
            number_precision: None,
        }
    }
}

impl TextBinding {
    pub fn remap(
        &mut self,
        parameters: &BTreeMap<ParameterId, ParameterId>,
        options: &BTreeMap<OptionId, OptionId>,
        actions: &BTreeMap<ActionId, ActionId>,
        entities: &BTreeMap<EntityId, EntityId>,
    ) -> Result<(), DynamicError> {
        if let Some(mapped) = actions.get(&self.id) {
            self.id = *mapped;
        }
        self.target = *entities.get(&self.target).ok_or(DynamicError::MissingEntity {
            target: GeometryTarget::Entity(self.target),
        })?;
        match &mut self.mode {
            TextBindingMode::ShowValue { parameter } => {
                if let Some(mapped) = parameters.get(parameter) {
                    *parameter = *mapped;
                }
            }
            TextBindingMode::OptionMap { parameter, texts } => {
                if let Some(mapped) = parameters.get(parameter) {
                    *parameter = *mapped;
                }
                let mut remapped = BTreeMap::new();
                for (id, text) in std::mem::take(texts) {
                    remapped.insert(options.get(&id).copied().unwrap_or(id), text);
                }
                *texts = remapped;
            }
            TextBindingMode::Formatted { tokens } => {
                for token in tokens {
                    if let TextToken::Parameter(parameter) = token {
                        if let Some(mapped) = parameters.get(parameter) {
                            *parameter = *mapped;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

pub fn escape_mtext_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn format_parameter_display(
    parameter: &ParameterDef,
    value: &ParameterValue,
    binding: &TextBinding,
) -> Result<String, DynamicError> {
    match (&parameter.kind, value) {
        (ParameterKind::Number(numeric), ParameterValue::Number(number)) => {
            let precision = binding.number_precision.unwrap_or(numeric.display_precision);
            Ok(crate::dynamic::format_display_number(*number, precision))
        }
        (ParameterKind::Choice(choice), ParameterValue::Choice(option)) => choice
            .options
            .iter()
            .find(|item| item.id == *option)
            .map(|item| item.label.clone())
            .ok_or(DynamicError::UnknownChoice {
                parameter: parameter.id,
                option: *option,
            }),
        (ParameterKind::Boolean(_), ParameterValue::Boolean(flag)) => Ok(if *flag {
            binding.boolean_true.clone()
        } else {
            binding.boolean_false.clone()
        }),
        (ParameterKind::Text(_), ParameterValue::Text(text)) => Ok(text.clone()),
        (expected, actual) => Err(DynamicError::ValueType {
            parameter: parameter.id,
            expected: expected.type_name(),
            actual: actual.type_name(),
        }),
    }
}

pub fn evaluate_text_binding(
    binding: &TextBinding,
    parameters: &[ParameterDef],
    values: &BTreeMap<ParameterId, ParameterValue>,
    mtext: bool,
) -> Result<String, DynamicError> {
    let lookup = |id: ParameterId| {
        parameters
            .iter()
            .find(|parameter| parameter.id == id)
            .ok_or(DynamicError::UnknownParameter {
                action: binding.id,
                parameter: id,
            })
    };
    let raw = match &binding.mode {
        TextBindingMode::ShowValue { parameter } => {
            let def = lookup(*parameter)?;
            let value = values.get(parameter).ok_or(DynamicError::UnknownParameter {
                action: binding.id,
                parameter: *parameter,
            })?;
            format_parameter_display(def, value, binding)?
        }
        TextBindingMode::OptionMap { parameter, texts } => {
            let value = values.get(parameter).ok_or(DynamicError::UnknownParameter {
                action: binding.id,
                parameter: *parameter,
            })?;
            let ParameterValue::Choice(option) = value else {
                return Err(DynamicError::ValueType {
                    parameter: *parameter,
                    expected: "choice",
                    actual: value.type_name(),
                });
            };
            texts
                .get(option)
                .cloned()
                .ok_or(DynamicError::IncompleteMapping {
                    action: binding.id,
                    parameter: *parameter,
                    option: *option,
                })?
        }
        TextBindingMode::Formatted { tokens } => {
            let mut out = String::new();
            for token in tokens {
                match token {
                    TextToken::Literal(text) => out.push_str(text),
                    TextToken::Parameter(parameter) => {
                        let def = lookup(*parameter)?;
                        let value = values.get(parameter).ok_or(DynamicError::UnknownParameter {
                            action: binding.id,
                            parameter: *parameter,
                        })?;
                        out.push_str(&format_parameter_display(def, value, binding)?);
                    }
                }
            }
            out
        }
    };
    if mtext {
        Ok(escape_mtext_literal(&raw))
    } else {
        Ok(raw)
    }
}

// ------------------------------------------------------------
// Types: Reflection / Rotation / Placement
// Purpose: Ordered rigid group transforms after size deformation.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub struct ReflectionBehavior {
    pub id: ActionId,
    pub name: Option<String>,
    pub members: Vec<EntityId>,
    pub axis_a: Point2,
    pub axis_b: Point2,
    pub condition: ParameterCondition,
    pub text_policy: TextReflectPolicy,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RotationSource {
    AngleParameter(ParameterId),
    OptionMap {
        parameter: ParameterId,
        angles: BTreeMap<OptionId, f64>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RotationBehavior {
    pub id: ActionId,
    pub name: Option<String>,
    pub members: Vec<EntityId>,
    pub pivot: Point2,
    pub source: RotationSource,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacementBehavior {
    pub id: ActionId,
    pub name: Option<String>,
    pub members: Vec<EntityId>,
    pub attachment: Point2,
    pub attachment_angle: f64,
    pub parameter: ParameterId,
    pub destinations: BTreeMap<OptionId, AnchorId>,
    pub boolean_destinations: Option<(AnchorId, AnchorId)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformKind {
    Reflection,
    Rotation,
    Placement,
}

impl TransformKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Reflection => "flip",
            Self::Rotation => "rotate",
            Self::Placement => "place",
        }
    }
}

// ------------------------------------------------------------
// Type: NestedInput
// Purpose: Parent-to-child parameter mapping on one occurrence.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub enum NestedMapping {
    Direct,
    OptionMap(BTreeMap<OptionId, ParameterValue>),
    NumericScale {
        factor: f64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct NestedInput {
    pub id: ActionId,
    pub source: ParameterId,
    pub target_occurrence: OccurrencePath,
    pub target_parameter: ParameterId,
    pub mapping: NestedMapping,
}

// ------------------------------------------------------------
// Enum: CompatibilityRule
// Purpose: Declarative restrictions between parameters.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub enum CompatibilityRule {
    ChoiceAllowsChoice {
        id: ActionId,
        when: ParameterId,
        when_option: OptionId,
        target: ParameterId,
        allowed: Vec<OptionId>,
    },
    ChoiceRestrictsNumeric {
        id: ActionId,
        when: ParameterId,
        when_option: OptionId,
        target: ParameterId,
        min: Option<f64>,
        max: Option<f64>,
        allowed: Option<Vec<f64>>,
    },
    BooleanPermits {
        id: ActionId,
        when: ParameterId,
        when_state: bool,
        target: ParameterId,
        allowed_options: Option<Vec<OptionId>>,
        required_boolean: Option<bool>,
    },
}

impl CompatibilityRule {
    pub fn id(&self) -> ActionId {
        match self {
            Self::ChoiceAllowsChoice { id, .. }
            | Self::ChoiceRestrictsNumeric { id, .. }
            | Self::BooleanPermits { id, .. } => *id,
        }
    }

    pub fn remap(
        &mut self,
        parameters: &BTreeMap<ParameterId, ParameterId>,
        options: &BTreeMap<OptionId, OptionId>,
        actions: &BTreeMap<ActionId, ActionId>,
    ) {
        let map_param = |id: &mut ParameterId| {
            if let Some(mapped) = parameters.get(id) {
                *id = *mapped;
            }
        };
        let map_opt = |id: &mut OptionId| {
            if let Some(mapped) = options.get(id) {
                *id = *mapped;
            }
        };
        match self {
            Self::ChoiceAllowsChoice {
                id,
                when,
                when_option,
                target,
                allowed,
            } => {
                if let Some(mapped) = actions.get(id) {
                    *id = *mapped;
                }
                map_param(when);
                map_opt(when_option);
                map_param(target);
                for option in allowed {
                    map_opt(option);
                }
            }
            Self::ChoiceRestrictsNumeric {
                id,
                when,
                when_option,
                target,
                ..
            } => {
                if let Some(mapped) = actions.get(id) {
                    *id = *mapped;
                }
                map_param(when);
                map_opt(when_option);
                map_param(target);
            }
            Self::BooleanPermits {
                id,
                when,
                target,
                allowed_options,
                ..
            } => {
                if let Some(mapped) = actions.get(id) {
                    *id = *mapped;
                }
                map_param(when);
                map_param(target);
                if let Some(allowed) = allowed_options {
                    for option in allowed {
                        map_opt(option);
                    }
                }
            }
        }
    }
}

pub fn rule_reason(rule: &CompatibilityRule, parameters: &[ParameterDef]) -> String {
    let name = |id: ParameterId| {
        parameters
            .iter()
            .find(|parameter| parameter.id == id)
            .map(|parameter| parameter.name.clone())
            .unwrap_or_else(|| id.to_string())
    };
    match rule {
        CompatibilityRule::ChoiceAllowsChoice {
            when, target, ..
        } => format!("{} restricts {}", name(*when), name(*target)),
        CompatibilityRule::ChoiceRestrictsNumeric {
            when, target, ..
        } => format!("{} restricts {}", name(*when), name(*target)),
        CompatibilityRule::BooleanPermits {
            when, target, ..
        } => format!("{} restricts {}", name(*when), name(*target)),
    }
}

pub fn active_compatibility_rules<'a>(
    rules: &'a [CompatibilityRule],
    values: &BTreeMap<ParameterId, ParameterValue>,
) -> Vec<&'a CompatibilityRule> {
    rules
        .iter()
        .filter(|rule| match rule {
            CompatibilityRule::ChoiceAllowsChoice {
                when, when_option, ..
            }
            | CompatibilityRule::ChoiceRestrictsNumeric {
                when, when_option, ..
            } => matches!(
                values.get(when),
                Some(ParameterValue::Choice(option)) if option == when_option
            ),
            CompatibilityRule::BooleanPermits {
                when, when_state, ..
            } => matches!(
                values.get(when),
                Some(ParameterValue::Boolean(flag)) if flag == when_state
            ),
        })
        .collect()
}

pub fn value_allowed_by_rules(
    parameter: &ParameterDef,
    value: &ParameterValue,
    rules: &[&CompatibilityRule],
) -> Result<(), String> {
    for rule in rules {
        match rule {
            CompatibilityRule::ChoiceAllowsChoice {
                target, allowed, ..
            } if *target == parameter.id => {
                let ParameterValue::Choice(option) = value else {
                    continue;
                };
                if !allowed.contains(option) {
                    return Err("option is not allowed by a compatibility rule".into());
                }
            }
            CompatibilityRule::ChoiceRestrictsNumeric {
                target,
                min,
                max,
                allowed,
                ..
            } if *target == parameter.id => {
                let ParameterValue::Number(number) = value else {
                    continue;
                };
                if let Some(min) = min {
                    if *number < *min {
                        return Err("value is below the restricted minimum".into());
                    }
                }
                if let Some(max) = max {
                    if *number > *max {
                        return Err("value is above the restricted maximum".into());
                    }
                }
                if let Some(allowed) = allowed {
                    if !allowed.iter().any(|item| numbers_equal(*item, *number)) {
                        return Err("value is not in the restricted list".into());
                    }
                }
            }
            CompatibilityRule::BooleanPermits {
                target,
                allowed_options,
                required_boolean,
                ..
            } if *target == parameter.id => {
                if let (Some(allowed), ParameterValue::Choice(option)) = (allowed_options, value) {
                    if !allowed.contains(option) {
                        return Err("option is not allowed by a compatibility rule".into());
                    }
                }
                if let (Some(required), ParameterValue::Boolean(flag)) = (required_boolean, value) {
                    if flag != required {
                        return Err("state is required by a compatibility rule".into());
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

// ------------------------------------------------------------
// Type: Preset
// Purpose: Complete resolved snapshot, independent of later defaults.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub id: PresetId,
    pub name: String,
    pub values: BTreeMap<ParameterId, ParameterValue>,
}

impl Preset {
    pub fn remap(
        &mut self,
        parameters: &BTreeMap<ParameterId, ParameterId>,
        options: &BTreeMap<OptionId, OptionId>,
        presets: &BTreeMap<PresetId, PresetId>,
    ) {
        if let Some(mapped) = presets.get(&self.id) {
            self.id = *mapped;
        }
        let mut values = BTreeMap::new();
        for (id, value) in std::mem::take(&mut self.values) {
            let mapped_id = parameters.get(&id).copied().unwrap_or(id);
            values.insert(mapped_id, remap_choice_value(value, options));
        }
        self.values = values;
    }
}

pub fn remap_choice_value(
    value: ParameterValue,
    options: &BTreeMap<OptionId, OptionId>,
) -> ParameterValue {
    match value {
        ParameterValue::Choice(option) => {
            ParameterValue::Choice(options.get(&option).copied().unwrap_or(option))
        }
        other => other,
    }
}

pub fn configurations_equal(
    left: &BTreeMap<ParameterId, ParameterValue>,
    right: &BTreeMap<ParameterId, ParameterValue>,
    parameters: &[ParameterDef],
) -> bool {
    if left.len() != right.len() {
        return false;
    }
    for (id, left_value) in left {
        let Some(right_value) = right.get(id) else {
            return false;
        };
        if !parameter_values_equal(parameters, *id, left_value, right_value) {
            return false;
        }
    }
    true
}

pub fn parameter_values_equal(
    parameters: &[ParameterDef],
    id: ParameterId,
    left: &ParameterValue,
    right: &ParameterValue,
) -> bool {
    match (left, right) {
        (ParameterValue::Number(a), ParameterValue::Number(b)) => numbers_equal(*a, *b),
        (ParameterValue::Choice(a), ParameterValue::Choice(b)) => a == b,
        (ParameterValue::Boolean(a), ParameterValue::Boolean(b)) => a == b,
        (ParameterValue::Text(a), ParameterValue::Text(b)) => a == b,
        _ => {
            let _ = (parameters, id);
            false
        }
    }
}

pub fn matching_preset(
    presets: &[Preset],
    values: &BTreeMap<ParameterId, ParameterValue>,
    parameters: &[ParameterDef],
) -> Option<PresetId> {
    presets.iter().find_map(|preset| {
        configurations_equal(&preset.values, values, parameters).then_some(preset.id)
    })
}

pub fn option_usages(
    parameter: ParameterId,
    option: OptionId,
    visibility: &[VisibilityGroup],
    text_bindings: &[TextBinding],
    reflections: &[ReflectionBehavior],
    rotations: &[RotationBehavior],
    placements: &[PlacementBehavior],
    nested_inputs: &[NestedInput],
    compatibility: &[CompatibilityRule],
    presets: &[Preset],
    instances: &[(EntityId, &InstanceConfiguration)],
) -> Vec<String> {
    let mut uses = Vec::new();
    for group in visibility {
        for condition in &group.conditions {
            if let ParameterCondition::Choice {
                parameter: pid,
                options,
            } = condition
            {
                if *pid == parameter && options.contains(&option) {
                    uses.push(format!("visibility group '{}'", group.name));
                }
            }
        }
    }
    for binding in text_bindings {
        match &binding.mode {
            TextBindingMode::OptionMap {
                parameter: pid,
                texts,
            } if *pid == parameter && texts.contains_key(&option) => {
                uses.push("text binding".into());
            }
            _ => {}
        }
    }
    for behavior in reflections {
        if let ParameterCondition::Choice {
            parameter: pid,
            options,
        } = &behavior.condition
        {
            if *pid == parameter && options.contains(&option) {
                uses.push(
                    behavior
                        .name
                        .clone()
                        .unwrap_or_else(|| "flip".into()),
                );
            }
        }
    }
    for behavior in rotations {
        if let RotationSource::OptionMap {
            parameter: pid,
            angles,
        } = &behavior.source
        {
            if *pid == parameter && angles.contains_key(&option) {
                uses.push(
                    behavior
                        .name
                        .clone()
                        .unwrap_or_else(|| "rotation".into()),
                );
            }
        }
    }
    for behavior in placements {
        if behavior.parameter == parameter && behavior.destinations.contains_key(&option) {
            uses.push(
                behavior
                    .name
                    .clone()
                    .unwrap_or_else(|| "placement".into()),
            );
        }
    }
    for input in nested_inputs {
        if input.source == parameter {
            if let NestedMapping::OptionMap(map) = &input.mapping {
                if map.contains_key(&option) {
                    uses.push("nested mapping".into());
                }
            }
        }
    }
    for rule in compatibility {
        match rule {
            CompatibilityRule::ChoiceAllowsChoice {
                when,
                when_option,
                allowed,
                ..
            } => {
                if *when == parameter && (*when_option == option || allowed.contains(&option)) {
                    uses.push("compatibility rule".into());
                }
            }
            CompatibilityRule::ChoiceRestrictsNumeric {
                when, when_option, ..
            } if *when == parameter && *when_option == option => {
                uses.push("compatibility rule".into());
            }
            _ => {}
        }
    }
    for preset in presets {
        if matches!(preset.values.get(&parameter), Some(ParameterValue::Choice(id)) if *id == option)
        {
            uses.push(format!("preset '{}'", preset.name));
        }
    }
    for (entity, config) in instances {
        if matches!(config.get(parameter), Some(ParameterValue::Choice(id)) if *id == option) {
            uses.push(format!("reference #{}", entity.raw()));
        }
    }
    uses.sort();
    uses.dedup();
    uses
}

pub fn migrate_option_value(
    value: &mut ParameterValue,
    parameter: ParameterId,
    from: OptionId,
    to: OptionId,
) {
    if let ParameterValue::Choice(option) = value {
        if *option == from {
            *option = to;
        }
    }
    let _ = parameter;
}

pub fn replace_option_id(id: &mut OptionId, from: OptionId, to: OptionId) {
    if *id == from {
        *id = to;
    }
}

pub fn replace_option_list(ids: &mut Vec<OptionId>, from: OptionId, to: OptionId) {
    for id in ids.iter_mut() {
        replace_option_id(id, from, to);
    }
    ids.sort();
    ids.dedup();
}

pub fn replace_option_key<T>(map: &mut BTreeMap<OptionId, T>, from: OptionId, to: OptionId) {
    if let Some(value) = map.remove(&from) {
        map.entry(to).or_insert(value);
    }
}
