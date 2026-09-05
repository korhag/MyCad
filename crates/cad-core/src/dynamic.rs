//! Generic dynamic-block definition, instance values, and validation.
//!
//! Parameter names, equipment types, and product-specific behaviors do not
//! belong here. The engine only understands typed parameters and generic
//! Move/Stretch actions bound to durable targets.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::document::DrawingUnits;
use crate::entity::{Entity, EntityId, Geometry};
use crate::geom::{Point2, Point3};
use crate::ids::{ActionId, OptionId, ParameterId, VertexId};

pub const EVALUATOR_VERSION: u32 = 1;

// ------------------------------------------------------------
// Enum: GeometryTarget
// Purpose: Durable binding to a source entity, LINE endpoint, or
//          polyline vertex. Vertex targets use VertexId, not index.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GeometryTarget {
    Entity(EntityId),
    LineStart(EntityId),
    LineEnd(EntityId),
    Vertex {
        entity: EntityId,
        vertex: VertexId,
    },
}

impl GeometryTarget {
    pub fn entity_id(self) -> EntityId {
        match self {
            Self::Entity(id) | Self::LineStart(id) | Self::LineEnd(id) => id,
            Self::Vertex { entity, .. } => entity,
        }
    }

    pub fn vertex_id(self) -> Option<VertexId> {
        match self {
            Self::Vertex { vertex, .. } => Some(vertex),
            _ => None,
        }
    }

    pub fn remap(
        self,
        entities: &BTreeMap<EntityId, EntityId>,
        vertices: &BTreeMap<VertexId, VertexId>,
    ) -> Option<Self> {
        let mapped = *entities.get(&self.entity_id())?;
        Some(match self {
            Self::Entity(_) => Self::Entity(mapped),
            Self::LineStart(_) => Self::LineStart(mapped),
            Self::LineEnd(_) => Self::LineEnd(mapped),
            Self::Vertex { vertex, .. } => Self::Vertex {
                entity: mapped,
                vertex: vertices.get(&vertex).copied().unwrap_or(vertex),
            },
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Entity(_) => "entity",
            Self::LineStart(_) => "line start",
            Self::LineEnd(_) => "line end",
            Self::Vertex { .. } => "vertex",
        }
    }
}

// ------------------------------------------------------------
// Enum: NumericQuantity
// Purpose: Physical meaning of a numeric parameter.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumericQuantity {
    Length,
    Distance,
    Angle,
    Count,
    Dimensionless,
}

impl NumericQuantity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Length => "length",
            Self::Distance => "distance",
            Self::Angle => "angle",
            Self::Count => "count",
            Self::Dimensionless => "number",
        }
    }
}

// ------------------------------------------------------------
// Enum: ParameterUnit
// Purpose: Display unit stored on the parameter, independent of
//          the drawing's current $INSUNITS.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParameterUnit {
    Drawing(DrawingUnits),
    Degrees,
    Radians,
    Count,
    None,
}

impl ParameterUnit {
    pub fn label(self) -> &'static str {
        match self {
            Self::Drawing(units) => units.label(),
            Self::Degrees => "deg",
            Self::Radians => "rad",
            Self::Count => "count",
            Self::None => "",
        }
    }

    pub fn compatible_with(self, quantity: NumericQuantity) -> bool {
        match (quantity, self) {
            (NumericQuantity::Length | NumericQuantity::Distance, Self::Drawing(_)) => true,
            (NumericQuantity::Length | NumericQuantity::Distance, Self::None) => true,
            (NumericQuantity::Angle, Self::Degrees | Self::Radians) => true,
            (NumericQuantity::Count, Self::Count | Self::None) => true,
            (NumericQuantity::Dimensionless, Self::None | Self::Count) => true,
            _ => false,
        }
    }
}

// ------------------------------------------------------------
// Enum: StepPolicy
// Purpose: How an optional numeric step is enforced.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepPolicy {
    /// Step only drives increment/decrement controls.
    IncrementOnly,
    /// Accepted values must fall on the configured step grid.
    RequiredIncrement,
}

// ------------------------------------------------------------
// Enum: StepOrigin
// Purpose: Grid origin for required increments. `Minimum` is
//          `minimum + n × step`; `Zero` is `n × step`.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepOrigin {
    Minimum,
    Zero,
}

// ------------------------------------------------------------
// Enum: NumericDomain
// Purpose: Continuous range (with optional min/max/step) or an
//          explicit list of permitted numeric sizes.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub enum NumericDomain {
    Continuous,
    AllowedValues(Vec<f64>),
}

impl Default for NumericDomain {
    fn default() -> Self {
        Self::Continuous
    }
}

impl NumericDomain {
    pub fn is_list(&self) -> bool {
        matches!(self, Self::AllowedValues(_))
    }

    pub fn contains(&self, value: f64) -> bool {
        match self {
            Self::Continuous => true,
            Self::AllowedValues(values) => values.iter().any(|item| numbers_equal(*item, value)),
        }
    }
}

// ------------------------------------------------------------
// Enum: MeasureMode / AnchorPolicy / FollowRole / SizeAuthoring
// Purpose: Persisted meaning of a measured size parameter. Picked
//          points are fixed block-local references unless an anchor
//          is explicitly geometry-bound.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeasureMode {
    AlongPicked,
    LocalX,
    LocalY,
}

impl MeasureMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::AlongPicked => "Along picked points",
            Self::LocalX => "Local X",
            Self::LocalY => "Local Y",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnchorPolicy {
    FirstFixed,
    SecondFixed,
    CenterFixed,
}

impl AnchorPolicy {
    pub fn label(self) -> &'static str {
        match self {
            Self::FirstFixed => "Keep first side fixed",
            Self::SecondFixed => "Keep second side fixed",
            Self::CenterFixed => "Keep center fixed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FollowRole {
    First,
    Second,
    Center,
    Custom,
}

impl FollowRole {
    pub fn label(self) -> &'static str {
        match self {
            Self::First => "first side",
            Self::Second => "second side",
            Self::Center => "center",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SizeAuthoring {
    /// First picked reference in block-local source coordinates.
    pub point_a: Point2,
    /// Second picked reference in block-local source coordinates.
    pub point_b: Point2,
    pub measure: MeasureMode,
    pub direction: Point2,
    pub anchor: AnchorPolicy,
    /// Overlay-only offset for the displayed dimension.
    pub label_offset: Point2,
    /// Optional geometry-bound anchor. Invalid when the target disappears.
    pub bound_anchor: Option<GeometryTarget>,
}

impl SizeAuthoring {
    pub fn points_are_fixed_local(&self) -> bool {
        self.bound_anchor.is_none()
    }
}

pub fn follow_multiplier(anchor: AnchorPolicy, follow: FollowRole) -> f64 {
    match (anchor, follow) {
        (_, FollowRole::Custom) => 1.0,
        (AnchorPolicy::FirstFixed, FollowRole::First) => 0.0,
        (AnchorPolicy::FirstFixed, FollowRole::Second) => 1.0,
        (AnchorPolicy::FirstFixed, FollowRole::Center) => 0.5,
        (AnchorPolicy::SecondFixed, FollowRole::First) => -1.0,
        (AnchorPolicy::SecondFixed, FollowRole::Second) => 0.0,
        (AnchorPolicy::SecondFixed, FollowRole::Center) => -0.5,
        (AnchorPolicy::CenterFixed, FollowRole::First) => -0.5,
        (AnchorPolicy::CenterFixed, FollowRole::Second) => 0.5,
        (AnchorPolicy::CenterFixed, FollowRole::Center) => 0.0,
    }
}

pub fn measure_size(
    point_a: Point2,
    point_b: Point2,
    mode: MeasureMode,
) -> Result<(Point2, f64), &'static str> {
    let delta = point_b - point_a;
    match mode {
        MeasureMode::AlongPicked => {
            let direction = normalize_direction(delta).ok_or("points coincide")?;
            Ok((direction, point_a.distance(point_b)))
        }
        MeasureMode::LocalX => projected_size(delta, Point2::new(1.0, 0.0)),
        MeasureMode::LocalY => projected_size(delta, Point2::new(0.0, 1.0)),
    }
}

fn projected_size(delta: Point2, axis: Point2) -> Result<(Point2, f64), &'static str> {
    let projection = delta.x * axis.x + delta.y * axis.y;
    if projection.abs() <= 1e-12 {
        return Err("projected distance is zero");
    }
    let direction = if projection >= 0.0 { axis } else { axis * -1.0 };
    Ok((direction, projection.abs()))
}

/// Compile a parameter's measured axis into every behavior that uses it.
pub fn apply_size_axis(
    dynamic: &mut DynamicDefinition,
    parameter: ParameterId,
    direction: Point2,
    reference: f64,
) {
    if let Some(def) = dynamic.parameter_mut(parameter) {
        if let ParameterKind::Number(numeric) = &mut def.kind {
            numeric.reference = reference;
            if let Some(size) = &mut numeric.size {
                size.direction = direction;
            }
        }
    }
    for behavior in &mut dynamic.behaviors {
        if behavior.parameter == parameter {
            behavior.local_direction = direction;
            behavior.reference_value = reference;
        }
    }
}

/// Rebuild multipliers for non-custom follow roles after an anchor change.
pub fn apply_anchor_policy(dynamic: &mut DynamicDefinition, parameter: ParameterId, anchor: AnchorPolicy) {
    if let Some(def) = dynamic.parameter_mut(parameter) {
        if let ParameterKind::Number(numeric) = &mut def.kind {
            if let Some(size) = &mut numeric.size {
                size.anchor = anchor;
            }
        }
    }
    for behavior in &mut dynamic.behaviors {
        if behavior.parameter == parameter && behavior.follow != FollowRole::Custom {
            behavior.multiplier = follow_multiplier(anchor, behavior.follow);
        }
    }
}

pub fn numbers_equal(left: f64, right: f64) -> bool {
    if left == right {
        return true;
    }
    let scale = left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= 1e-9 * scale
}

pub fn parse_allowed_value_list(text: &str) -> Result<Vec<f64>, String> {
    let mut values = Vec::new();
    let normalized = text.replace('\r', "\n").replace(';', "\n");
    for raw in normalized.split('\n') {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        let parsed = parse_pasted_number(token)?;
        if values.iter().any(|existing| numbers_equal(*existing, parsed)) {
            return Err(format!("duplicate value {parsed}"));
        }
        values.push(parsed);
    }
    if values.is_empty() {
        return Err("no values".into());
    }
    Ok(values)
}

fn parse_pasted_number(token: &str) -> Result<f64, String> {
    let trimmed = token.trim();
    let candidate = if trimmed.contains(',') && !trimmed.contains('.') {
        trimmed.replace(',', ".")
    } else {
        trimmed.to_string()
    };
    let value: f64 = candidate
        .parse()
        .map_err(|_| format!("not a number: {token}"))?;
    if !value.is_finite() {
        return Err(format!("not finite: {token}"));
    }
    Ok(value)
}

pub fn format_display_number(value: f64, precision: u8) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let text = format!("{value:.prec$}", prec = precision as usize);
    if precision == 0 {
        return text;
    }
    let Some(dot) = text.find('.') else {
        return text;
    };
    let (integer, fraction) = text.split_at(dot);
    let fraction = fraction.trim_end_matches('0');
    if fraction == "." {
        integer.to_string()
    } else {
        format!("{integer}{fraction}")
    }
}

pub fn nearest_step_values(numeric: &NumericParameter, value: f64) -> Option<(f64, f64)> {
    let step = numeric.step?;
    if !step.is_finite() || step <= 0.0 {
        return None;
    }
    let origin = match numeric.step_origin {
        StepOrigin::Minimum => numeric.min.unwrap_or(0.0),
        StepOrigin::Zero => 0.0,
    };
    let n = ((value - origin) / step).floor();
    let mut lo = origin + n * step;
    let mut hi = origin + (n + 1.0) * step;
    if let Some(min) = numeric.min {
        lo = lo.max(min);
        hi = hi.max(min);
    }
    if let Some(max) = numeric.max {
        lo = lo.min(max);
        hi = hi.min(max);
    }
    Some((lo, hi))
}

pub fn nearest_allowed_values(values: &[f64], value: f64) -> Option<(f64, f64)> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut below = sorted[0];
    let mut above = *sorted.last().unwrap();
    for item in &sorted {
        if *item <= value {
            below = *item;
        }
        if *item >= value {
            above = *item;
            break;
        }
    }
    Some((below, above))
}

// ------------------------------------------------------------
// Type: NumericParameter
// Purpose: Length, distance, angle, count, or dimensionless number.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub struct NumericParameter {
    pub quantity: NumericQuantity,
    pub unit: ParameterUnit,
    pub default: f64,
    /// Geometric baseline represented by the source geometry.
    pub reference: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub step_policy: StepPolicy,
    pub step_origin: StepOrigin,
    pub display_precision: u8,
    pub display_order: i32,
    pub domain: NumericDomain,
    pub size: Option<SizeAuthoring>,
}

impl NumericParameter {
    pub fn length(default: f64) -> Self {
        Self {
            quantity: NumericQuantity::Length,
            unit: ParameterUnit::None,
            default,
            reference: default,
            min: None,
            max: None,
            step: None,
            step_policy: StepPolicy::IncrementOnly,
            step_origin: StepOrigin::Minimum,
            display_precision: 4,
            display_order: 0,
            domain: NumericDomain::Continuous,
            size: None,
        }
    }
}

// ------------------------------------------------------------
// Type: ChoiceOption / ChoiceParameter / Boolean / Text
// Purpose: Validated model for later authoring phases. Not exposed
//          as unfinished UI controls in this milestone.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceOption {
    pub id: OptionId,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceParameter {
    pub options: Vec<ChoiceOption>,
    pub default: OptionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BooleanParameter {
    pub default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextParameter {
    pub default: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParameterKind {
    Number(NumericParameter),
    Choice(ChoiceParameter),
    Boolean(BooleanParameter),
    Text(TextParameter),
}

impl ParameterKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Number(_) => "number",
            Self::Choice(_) => "choice",
            Self::Boolean(_) => "boolean",
            Self::Text(_) => "text",
        }
    }
}

// ------------------------------------------------------------
// Type: ParameterDef
// Purpose: Named, typed parameter owned by a dynamic definition.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub struct ParameterDef {
    pub id: ParameterId,
    pub name: String,
    pub description: Option<String>,
    pub kind: ParameterKind,
}

impl ParameterDef {
    pub fn number(id: ParameterId, name: impl Into<String>, numeric: NumericParameter) -> Self {
        Self {
            id,
            name: name.into(),
            description: None,
            kind: ParameterKind::Number(numeric),
        }
    }

    pub fn default_value(&self) -> ParameterValue {
        match &self.kind {
            ParameterKind::Number(numeric) => ParameterValue::Number(numeric.default),
            ParameterKind::Choice(choice) => ParameterValue::Choice(choice.default),
            ParameterKind::Boolean(flag) => ParameterValue::Boolean(flag.default),
            ParameterKind::Text(text) => ParameterValue::Text(text.default.clone()),
        }
    }
}

// ------------------------------------------------------------
// Enum: ParameterValue
// Purpose: Instance or default value for one parameter.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub enum ParameterValue {
    Number(f64),
    Choice(OptionId),
    Boolean(bool),
    Text(String),
}

impl ParameterValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Number(_) => "number",
            Self::Choice(_) => "choice",
            Self::Boolean(_) => "boolean",
            Self::Text(_) => "text",
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(*value),
            _ => None,
        }
    }

    pub fn normalize(&self) -> NormalizedValue {
        match self {
            Self::Number(value) => NormalizedValue::Number(value.to_bits()),
            Self::Choice(id) => NormalizedValue::Choice(id.raw()),
            Self::Boolean(flag) => NormalizedValue::Boolean(*flag),
            Self::Text(text) => NormalizedValue::Text(text.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NormalizedValue {
    Number(u64),
    Choice(u64),
    Boolean(bool),
    Text(String),
}

// ------------------------------------------------------------
// Type: InstanceConfiguration
// Purpose: Canonical instance state attached to one INSERT.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InstanceConfiguration {
    pub values: BTreeMap<ParameterId, ParameterValue>,
}

impl InstanceConfiguration {
    pub fn get(&self, id: ParameterId) -> Option<&ParameterValue> {
        self.values.get(&id)
    }

    pub fn set(&mut self, id: ParameterId, value: ParameterValue) {
        self.values.insert(id, value);
    }

    pub fn remap_parameters(&mut self, parameters: &BTreeMap<ParameterId, ParameterId>) {
        let mut remapped = BTreeMap::new();
        for (id, value) in std::mem::take(&mut self.values) {
            if let Some(new_id) = parameters.get(&id) {
                remapped.insert(*new_id, value);
            } else {
                remapped.insert(id, value);
            }
        }
        self.values = remapped;
    }
}

// ------------------------------------------------------------
// Enum: BehaviorKind / CompositionRule / DynamicBehavior
// Purpose: Generic Move and Stretch driven by a numeric parameter.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BehaviorKind {
    Move,
    Stretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompositionRule {
    Additive,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DynamicBehavior {
    pub id: ActionId,
    pub kind: BehaviorKind,
    pub parameter: ParameterId,
    pub targets: Vec<GeometryTarget>,
    pub local_direction: Point2,
    pub reference_value: f64,
    pub multiplier: f64,
    pub composition: CompositionRule,
    pub follow: FollowRole,
    pub name: Option<String>,
}

impl DynamicBehavior {
    pub fn displacement(&self, current: f64) -> Point2 {
        let delta = current - self.reference_value;
        Point2::new(
            self.local_direction.x * delta * self.multiplier,
            self.local_direction.y * delta * self.multiplier,
        )
    }

    pub fn describe(&self, dynamic: &DynamicDefinition) -> String {
        let kind = match self.kind {
            BehaviorKind::Move => "Move",
            BehaviorKind::Stretch => "Stretch",
        };
        let follow = match self.follow {
            FollowRole::Custom => format!("× {}", self.multiplier),
            other => format!("follow {}", other.label()),
        };
        let count = self.targets.len();
        let noun = match self.kind {
            BehaviorKind::Move => {
                if count == 1 {
                    "object"
                } else {
                    "objects"
                }
            }
            BehaviorKind::Stretch => {
                if count == 1 {
                    "vertex"
                } else {
                    "vertices"
                }
            }
        };
        let label = self
            .name
            .as_deref()
            .or_else(|| dynamic.parameter(self.parameter).map(|p| p.name.as_str()))
            .unwrap_or("parameter");
        format!("{kind} — {follow} — {count} {noun} ({label})")
    }

    pub fn remap(
        &mut self,
        parameters: &BTreeMap<ParameterId, ParameterId>,
        actions: &BTreeMap<ActionId, ActionId>,
        entities: &BTreeMap<EntityId, EntityId>,
        vertices: &BTreeMap<VertexId, VertexId>,
    ) -> Result<(), DynamicError> {
        if let Some(mapped) = actions.get(&self.id) {
            self.id = *mapped;
        }
        if let Some(mapped) = parameters.get(&self.parameter) {
            self.parameter = *mapped;
        }
        let mut remapped = Vec::with_capacity(self.targets.len());
        for target in &self.targets {
            let Some(mapped) = target.remap(entities, vertices) else {
                return Err(DynamicError::BrokenBinding {
                    action: self.id,
                    target: *target,
                });
            };
            remapped.push(mapped);
        }
        self.targets = remapped;
        Ok(())
    }
}

// ------------------------------------------------------------
// Type: DynamicDefinition
// Purpose: Parameters and behaviors stored with source geometry.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DynamicDefinition {
    pub parameters: Vec<ParameterDef>,
    pub behaviors: Vec<DynamicBehavior>,
}

impl DynamicDefinition {
    pub fn is_empty(&self) -> bool {
        self.parameters.is_empty() && self.behaviors.is_empty()
    }

    pub fn parameter(&self, id: ParameterId) -> Option<&ParameterDef> {
        self.parameters.iter().find(|parameter| parameter.id == id)
    }

    pub fn parameter_mut(&mut self, id: ParameterId) -> Option<&mut ParameterDef> {
        self.parameters
            .iter_mut()
            .find(|parameter| parameter.id == id)
    }

    pub fn sorted_parameters(&self) -> Vec<&ParameterDef> {
        let mut parameters: Vec<&ParameterDef> = self.parameters.iter().collect();
        parameters.sort_by(|left, right| {
            let left_order = match &left.kind {
                ParameterKind::Number(numeric) => numeric.display_order,
                _ => 0,
            };
            let right_order = match &right.kind {
                ParameterKind::Number(numeric) => numeric.display_order,
                _ => 0,
            };
            left_order
                .cmp(&right_order)
                .then_with(|| {
                    left.name
                        .to_ascii_lowercase()
                        .cmp(&right.name.to_ascii_lowercase())
                })
                .then_with(|| left.id.cmp(&right.id))
        });
        parameters
    }

    pub fn remap_ids(
        &mut self,
        parameters: &BTreeMap<ParameterId, ParameterId>,
        options: &BTreeMap<OptionId, OptionId>,
        actions: &BTreeMap<ActionId, ActionId>,
        entities: &BTreeMap<EntityId, EntityId>,
        vertices: &BTreeMap<VertexId, VertexId>,
    ) -> Result<(), DynamicError> {
        for parameter in &mut self.parameters {
            if let Some(mapped) = parameters.get(&parameter.id) {
                parameter.id = *mapped;
            }
            if let ParameterKind::Choice(choice) = &mut parameter.kind {
                if let Some(mapped) = options.get(&choice.default) {
                    choice.default = *mapped;
                }
                for option in &mut choice.options {
                    if let Some(mapped) = options.get(&option.id) {
                        option.id = *mapped;
                    }
                }
            }
            if let ParameterKind::Number(numeric) = &mut parameter.kind {
                if let Some(size) = &mut numeric.size {
                    if let Some(bound) = size.bound_anchor {
                        size.bound_anchor = bound.remap(entities, vertices);
                    }
                }
            }
        }
        for behavior in &mut self.behaviors {
            behavior.remap(parameters, actions, entities, vertices)?;
        }
        Ok(())
    }
}

// ------------------------------------------------------------
// Enum: DynamicError
// Purpose: One validation or evaluation failure with enough
//          identity to name the responsible parameter or action.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub enum DynamicError {
    NonFinite {
        parameter: ParameterId,
    },
    ValueType {
        parameter: ParameterId,
        expected: &'static str,
        actual: &'static str,
    },
    Range {
        parameter: ParameterId,
        value: f64,
    },
    NonPositiveStep {
        parameter: ParameterId,
    },
    DefaultOutOfDomain {
        parameter: ParameterId,
    },
    UnknownChoice {
        parameter: ParameterId,
        option: OptionId,
    },
    IncompatibleQuantity {
        parameter: ParameterId,
    },
    DuplicateParameter(ParameterId),
    DuplicateOption(OptionId),
    DuplicateAction(ActionId),
    UnknownParameter {
        action: ActionId,
        parameter: ParameterId,
    },
    MissingEntity {
        target: GeometryTarget,
    },
    BrokenBinding {
        action: ActionId,
        target: GeometryTarget,
    },
    UnsupportedTarget {
        action: ActionId,
        target: GeometryTarget,
        reason: &'static str,
    },
    ZeroDirection {
        action: ActionId,
    },
    ConflictingWrite {
        target: GeometryTarget,
        actions: Vec<ActionId>,
    },
    OverlappingContribution {
        parameter: ParameterId,
        action: ActionId,
        other: ActionId,
        target: GeometryTarget,
        reason: &'static str,
    },
    ValueNotInList {
        parameter: ParameterId,
        value: f64,
    },
    OffStep {
        parameter: ParameterId,
        value: f64,
        nearest: [f64; 2],
    },
    MissingVertex {
        target: GeometryTarget,
    },
    InvalidGeometry {
        entity: EntityId,
        reason: &'static str,
    },
    MissingDefinition,
    StaleGeneration {
        expected: u64,
        actual: u64,
    },
    NestedEditUnsupported,
}

impl fmt::Display for DynamicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite { parameter } => {
                write!(f, "Parameter {parameter} has a non-finite value")
            }
            Self::ValueType {
                parameter,
                expected,
                actual,
            } => write!(
                f,
                "Parameter {parameter} expected a {expected} value, got {actual}"
            ),
            Self::Range { parameter, value } => {
                write!(
                    f,
                    "Parameter {parameter} value {value} is outside its limits"
                )
            }
            Self::NonPositiveStep { parameter } => {
                write!(f, "Parameter {parameter} step must be positive")
            }
            Self::DefaultOutOfDomain { parameter } => {
                write!(
                    f,
                    "Parameter {parameter} default is outside its permitted domain"
                )
            }
            Self::UnknownChoice { parameter, option } => {
                write!(f, "Parameter {parameter} has unknown choice {option}")
            }
            Self::IncompatibleQuantity { parameter } => {
                write!(f, "Parameter {parameter} unit does not match its quantity")
            }
            Self::DuplicateParameter(id) => write!(f, "Duplicate parameter {id}"),
            Self::DuplicateOption(id) => write!(f, "Duplicate option {id}"),
            Self::DuplicateAction(id) => write!(f, "Duplicate action {id}"),
            Self::UnknownParameter { action, parameter } => {
                write!(
                    f,
                    "Action {action} references unknown parameter {parameter}"
                )
            }
            Self::MissingEntity { target } => {
                write!(
                    f,
                    "Bound {} #{} is missing",
                    target.label(),
                    target.entity_id().raw()
                )
            }
            Self::BrokenBinding { action, target } => write!(
                f,
                "Action {action} binding to {} #{} is broken",
                target.label(),
                target.entity_id().raw()
            ),
            Self::UnsupportedTarget {
                action,
                target,
                reason,
            } => write!(
                f,
                "Action {action} cannot {reason} {} #{}",
                target.label(),
                target.entity_id().raw()
            ),
            Self::ZeroDirection { action } => {
                write!(f, "Action {action} has a zero local direction")
            }
            Self::ConflictingWrite { target, actions } => {
                write!(
                    f,
                    "Conflicting writes to {} #{} from actions {:?}",
                    target.label(),
                    target.entity_id().raw(),
                    actions
                )
            }
            Self::OverlappingContribution {
                parameter,
                action,
                other,
                target,
                reason,
            } => write!(
                f,
                "Parameter {parameter} behavior {action} overlaps {other} on {} #{} ({reason})",
                target.label(),
                target.entity_id().raw()
            ),
            Self::ValueNotInList { parameter, value } => {
                write!(
                    f,
                    "Parameter {parameter} value {value} is not in the allowed list"
                )
            }
            Self::OffStep {
                parameter,
                value,
                nearest,
            } => write!(
                f,
                "Parameter {parameter} value {value} is off-step (nearest {} and {})",
                nearest[0], nearest[1]
            ),
            Self::MissingVertex { target } => write!(
                f,
                "Bound {} #{} is missing",
                target.label(),
                target.entity_id().raw()
            ),
            Self::InvalidGeometry { entity, reason } => {
                write!(
                    f,
                    "Entity #{} is invalid after evaluation ({reason})",
                    entity.raw()
                )
            }
            Self::MissingDefinition => f.write_str("Block definition was not found"),
            Self::StaleGeneration { expected, actual } => {
                write!(
                    f,
                    "Stale evaluation for generation {expected} (now {actual})"
                )
            }
            Self::NestedEditUnsupported => {
                f.write_str("Editing a nested dynamic reference independently is not supported yet")
            }
        }
    }
}

impl std::error::Error for DynamicError {}

// ------------------------------------------------------------
// Function: validate_definition
// Purpose: One validation path for UI, file loading, commands,
//          and evaluation. Rejects silently broken bindings.
// ------------------------------------------------------------
pub fn validate_definition(
    dynamic: &DynamicDefinition,
    entities: &[Entity],
) -> Result<(), DynamicError> {
    let mut parameter_ids = BTreeSet::new();
    let mut option_ids = BTreeSet::new();
    for parameter in &dynamic.parameters {
        if !parameter.id.is_assigned() || !parameter_ids.insert(parameter.id) {
            return Err(DynamicError::DuplicateParameter(parameter.id));
        }
        validate_parameter_def(parameter)?;
        if let ParameterKind::Choice(choice) = &parameter.kind {
            for option in &choice.options {
                if !option.id.is_assigned() || !option_ids.insert(option.id) {
                    return Err(DynamicError::DuplicateOption(option.id));
                }
            }
        }
    }
    let entity_ids: BTreeSet<EntityId> = entities
        .iter()
        .filter(|entity| entity.id.is_assigned())
        .map(|entity| entity.id)
        .collect();
    let mut action_ids = BTreeSet::new();
    for behavior in &dynamic.behaviors {
        if !behavior.id.is_assigned() || !action_ids.insert(behavior.id) {
            return Err(DynamicError::DuplicateAction(behavior.id));
        }
        if dynamic.parameter(behavior.parameter).is_none() {
            return Err(DynamicError::UnknownParameter {
                action: behavior.id,
                parameter: behavior.parameter,
            });
        }
        if !direction_is_valid(behavior.local_direction) {
            return Err(DynamicError::ZeroDirection {
                action: behavior.id,
            });
        }
        if !behavior.multiplier.is_finite() || !behavior.reference_value.is_finite() {
            return Err(DynamicError::NonFinite {
                parameter: behavior.parameter,
            });
        }
        for target in &behavior.targets {
            if !entity_ids.contains(&target.entity_id()) {
                return Err(DynamicError::BrokenBinding {
                    action: behavior.id,
                    target: *target,
                });
            }
            let entity = entities
                .iter()
                .find(|entity| entity.id == target.entity_id())
                .ok_or(DynamicError::BrokenBinding {
                    action: behavior.id,
                    target: *target,
                })?;
            if let GeometryTarget::Vertex { vertex, .. } = *target {
                let found = entity
                    .geometry
                    .polyline_vertices()
                    .is_some_and(|vertices| vertices.iter().any(|item| item.vertex_id == vertex));
                if !found {
                    return Err(DynamicError::MissingVertex { target: *target });
                }
            }
            capability_for(behavior.kind, &entity.geometry, *target).map_err(|reason| {
                DynamicError::UnsupportedTarget {
                    action: behavior.id,
                    target: *target,
                    reason,
                }
            })?;
        }
    }
    for parameter in &dynamic.parameters {
        if let ParameterKind::Number(numeric) = &parameter.kind {
            if let Some(size) = &numeric.size {
                if let Some(bound) = size.bound_anchor {
                    if !entity_ids.contains(&bound.entity_id()) {
                        return Err(DynamicError::BrokenBinding {
                            action: ActionId::UNASSIGNED,
                            target: bound,
                        });
                    }
                }
            }
        }
    }
    validate_behavior_conflicts(dynamic)?;
    Ok(())
}

pub fn collect_broken_bindings(
    dynamic: &DynamicDefinition,
    entities: &[Entity],
) -> Vec<(ActionId, GeometryTarget, &'static str)> {
    let mut broken = Vec::new();
    for behavior in &dynamic.behaviors {
        for target in &behavior.targets {
            let Some(entity) = entities.iter().find(|entity| entity.id == target.entity_id()) else {
                broken.push((behavior.id, *target, "missing entity"));
                continue;
            };
            if let GeometryTarget::Vertex { vertex, .. } = target {
                let found = entity
                    .geometry
                    .polyline_vertices()
                    .is_some_and(|vertices| vertices.iter().any(|item| item.vertex_id == *vertex));
                if !found {
                    broken.push((behavior.id, *target, "deleted vertex"));
                }
            }
        }
    }
    for parameter in &dynamic.parameters {
        if let ParameterKind::Number(numeric) = &parameter.kind {
            if let Some(size) = &numeric.size {
                if let Some(bound) = size.bound_anchor {
                    if entities.iter().all(|entity| entity.id != bound.entity_id()) {
                        broken.push((ActionId::UNASSIGNED, bound, "missing geometry-bound anchor"));
                    }
                }
            }
        }
    }
    broken
}

pub fn validate_parameter_def(parameter: &ParameterDef) -> Result<(), DynamicError> {
    match &parameter.kind {
        ParameterKind::Number(numeric) => validate_numeric_def(parameter.id, numeric),
        ParameterKind::Choice(choice) => {
            if choice.options.is_empty()
                || !choice
                    .options
                    .iter()
                    .any(|option| option.id == choice.default)
            {
                return Err(DynamicError::UnknownChoice {
                    parameter: parameter.id,
                    option: choice.default,
                });
            }
            Ok(())
        }
        ParameterKind::Boolean(_) | ParameterKind::Text(_) => Ok(()),
    }
}

fn validate_numeric_def(id: ParameterId, numeric: &NumericParameter) -> Result<(), DynamicError> {
    if !numeric.unit.compatible_with(numeric.quantity) {
        return Err(DynamicError::IncompatibleQuantity { parameter: id });
    }
    if !numeric.default.is_finite() || !numeric.reference.is_finite() {
        return Err(DynamicError::NonFinite { parameter: id });
    }
    if let Some(step) = numeric.step {
        if !step.is_finite() || step <= 0.0 {
            return Err(DynamicError::NonPositiveStep { parameter: id });
        }
    }
    if let (Some(min), Some(max)) = (numeric.min, numeric.max) {
        if !min.is_finite() || !max.is_finite() || min > max {
            return Err(DynamicError::Range {
                parameter: id,
                value: numeric.default,
            });
        }
    }
    if let NumericDomain::AllowedValues(values) = &numeric.domain {
        if values.iter().any(|item| !item.is_finite()) {
            return Err(DynamicError::NonFinite { parameter: id });
        }
        if !numeric.domain.contains(numeric.default) {
            return Err(DynamicError::DefaultOutOfDomain { parameter: id });
        }
    }
    validate_numeric_value(id, numeric, numeric.default)?;
    if let Some(min) = numeric.min {
        if numeric.reference < min && numeric.step_policy == StepPolicy::RequiredIncrement {
            // Reference is the geometric baseline and may be stored even when
            // a later limit would reject new instance values. Still require
            // it to be finite; domain checks apply to current/default values.
            let _ = min;
        }
    }
    Ok(())
}

pub fn validate_parameter_value(
    parameter: &ParameterDef,
    value: &ParameterValue,
) -> Result<(), DynamicError> {
    match (&parameter.kind, value) {
        (ParameterKind::Number(numeric), ParameterValue::Number(number)) => {
            validate_numeric_value(parameter.id, numeric, *number)
        }
        (ParameterKind::Choice(choice), ParameterValue::Choice(option)) => {
            if choice.options.iter().any(|item| item.id == *option) {
                Ok(())
            } else {
                Err(DynamicError::UnknownChoice {
                    parameter: parameter.id,
                    option: *option,
                })
            }
        }
        (ParameterKind::Boolean(_), ParameterValue::Boolean(_)) => Ok(()),
        (ParameterKind::Text(_), ParameterValue::Text(_)) => Ok(()),
        (expected, actual) => Err(DynamicError::ValueType {
            parameter: parameter.id,
            expected: expected.type_name(),
            actual: actual.type_name(),
        }),
    }
}

pub fn validate_numeric_value(
    id: ParameterId,
    numeric: &NumericParameter,
    value: f64,
) -> Result<(), DynamicError> {
    if !value.is_finite() {
        return Err(DynamicError::NonFinite { parameter: id });
    }
    if let Some(min) = numeric.min {
        if value < min {
            return Err(DynamicError::Range {
                parameter: id,
                value,
            });
        }
    }
    if let Some(max) = numeric.max {
        if value > max {
            return Err(DynamicError::Range {
                parameter: id,
                value,
            });
        }
    }
    if let NumericDomain::AllowedValues(_) = &numeric.domain {
        if !numeric.domain.contains(value) {
            return Err(DynamicError::ValueNotInList {
                parameter: id,
                value,
            });
        }
        return Ok(());
    }
    if numeric.step_policy == StepPolicy::RequiredIncrement {
        if let Some(step) = numeric.step {
            let origin = match numeric.step_origin {
                StepOrigin::Minimum => numeric.min.unwrap_or(0.0),
                StepOrigin::Zero => 0.0,
            };
            let n = ((value - origin) / step).round();
            let snapped = origin + n * step;
            if (snapped - value).abs() > step * 1e-9 && (snapped - value).abs() > 1e-9 {
                let nearest = nearest_step_values(numeric, value).unwrap_or((snapped, snapped));
                return Err(DynamicError::OffStep {
                    parameter: id,
                    value,
                    nearest: [nearest.0, nearest.1],
                });
            }
        }
    }
    Ok(())
}

pub fn snap_numeric(numeric: &NumericParameter, value: f64) -> f64 {
    let Some(step) = numeric.step else {
        return value;
    };
    if !step.is_finite() || step <= 0.0 || !value.is_finite() {
        return value;
    }
    let origin = match numeric.step_origin {
        StepOrigin::Minimum => numeric.min.unwrap_or(0.0),
        StepOrigin::Zero => 0.0,
    };
    let n = ((value - origin) / step).round();
    let mut snapped = origin + n * step;
    if let Some(min) = numeric.min {
        snapped = snapped.max(min);
    }
    if let Some(max) = numeric.max {
        snapped = snapped.min(max);
    }
    snapped
}

pub fn increment_numeric(numeric: &NumericParameter, value: f64, steps: i32) -> f64 {
    if let NumericDomain::AllowedValues(values) = &numeric.domain {
        return increment_allowed(values, value, steps);
    }
    let step = numeric.step.unwrap_or(1.0);
    snap_numeric(numeric, value + step * f64::from(steps))
}

fn increment_allowed(values: &[f64], value: f64, steps: i32) -> f64 {
    if values.is_empty() {
        return value;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let current = sorted
        .iter()
        .position(|item| numbers_equal(*item, value))
        .unwrap_or_else(|| {
            sorted
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    (*a - value)
                        .abs()
                        .partial_cmp(&(*b - value).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(index, _)| index)
                .unwrap_or(0)
        });
    let next = (current as i32 + steps).clamp(0, sorted.len() as i32 - 1) as usize;
    sorted[next]
}

fn direction_is_valid(direction: Point2) -> bool {
    direction.is_finite() && (direction.x * direction.x + direction.y * direction.y) > 1e-24
}

pub fn normalize_direction(direction: Point2) -> Option<Point2> {
    if !direction.is_finite() {
        return None;
    }
    let length = (direction.x * direction.x + direction.y * direction.y).sqrt();
    if length <= 1e-12 {
        return None;
    }
    Some(Point2::new(direction.x / length, direction.y / length))
}

// ------------------------------------------------------------
// Capability matrix
// Purpose: Supported whole-entity movement and LINE endpoint
//          stretch. Unsupported cases are rejected explicitly.
// ------------------------------------------------------------
pub fn capability_for(
    kind: BehaviorKind,
    geometry: &Geometry,
    target: GeometryTarget,
) -> Result<(), &'static str> {
    match (kind, target, geometry) {
        (BehaviorKind::Move, GeometryTarget::Entity(_), geometry) => {
            if geometry_can_move(geometry) {
                Ok(())
            } else {
                Err("move")
            }
        }
        (BehaviorKind::Move, GeometryTarget::LineStart(_) | GeometryTarget::LineEnd(_), _) => {
            Err("move a partial target; use Stretch")
        }
        (BehaviorKind::Move, GeometryTarget::Vertex { .. }, _) => {
            Err("move a vertex; use Stretch")
        }
        (
            BehaviorKind::Stretch,
            GeometryTarget::LineStart(_) | GeometryTarget::LineEnd(_),
            Geometry::Line { .. },
        ) => Ok(()),
        (
            BehaviorKind::Stretch,
            GeometryTarget::Vertex { .. },
            Geometry::LwPolyline { .. } | Geometry::Polyline { .. },
        ) => {
            if geometry.polyline_has_curves() {
                Err("stretch polylines that contain curved segments")
            } else {
                Ok(())
            }
        }
        (BehaviorKind::Stretch, GeometryTarget::Entity(_), Geometry::Line { .. }) => {
            Err("stretch a whole line; select endpoints or use Move")
        }
        (BehaviorKind::Stretch, _, Geometry::LwPolyline { .. } | Geometry::Polyline { .. }) => {
            Err("stretch this polyline; select straight vertices")
        }
        (BehaviorKind::Stretch, _, Geometry::Arc { .. }) => Err("stretch arcs"),
        (BehaviorKind::Stretch, _, Geometry::Spline { .. }) => Err("stretch splines"),
        (BehaviorKind::Stretch, _, Geometry::Hatch(_)) => Err("stretch hatches"),
        (BehaviorKind::Stretch, _, _) => Err("stretch this geometry"),
    }
}

fn geometry_can_move(geometry: &Geometry) -> bool {
    matches!(
        geometry,
        Geometry::Line { .. }
            | Geometry::Point { .. }
            | Geometry::Circle { .. }
            | Geometry::Arc { .. }
            | Geometry::Ellipse { .. }
            | Geometry::LwPolyline { .. }
            | Geometry::Polyline { .. }
            | Geometry::Spline { .. }
            | Geometry::Insert { .. }
            | Geometry::Text(_)
            | Geometry::MText(_)
            | Geometry::Solid { .. }
            | Geometry::Leader { .. }
            | Geometry::MLine { .. }
    )
}

pub fn dedupe_targets(targets: Vec<GeometryTarget>) -> Vec<GeometryTarget> {
    let mut seen = BTreeSet::new();
    let mut unique = Vec::new();
    for target in targets {
        if seen.insert(target) {
            unique.push(target);
        }
    }
    unique
}

pub fn directions_parallel(a: Point2, b: Point2) -> bool {
    let Some(na) = normalize_direction(a) else {
        return false;
    };
    let Some(nb) = normalize_direction(b) else {
        return false;
    };
    let cross = (na.x * nb.y - na.y * nb.x).abs();
    cross <= 1e-6
}

pub fn validate_behavior_conflicts(dynamic: &DynamicDefinition) -> Result<(), DynamicError> {
    let mut occupancy: BTreeMap<GeometryTarget, Vec<&DynamicBehavior>> = BTreeMap::new();
    let mut entity_move: BTreeMap<EntityId, Vec<&DynamicBehavior>> = BTreeMap::new();
    let mut entity_stretch: BTreeMap<EntityId, Vec<&DynamicBehavior>> = BTreeMap::new();
    for behavior in &dynamic.behaviors {
        let mut seen = BTreeSet::new();
        for target in &behavior.targets {
            if !seen.insert(*target) {
                continue;
            }
            occupancy.entry(*target).or_default().push(behavior);
            match target {
                GeometryTarget::Entity(id) if behavior.kind == BehaviorKind::Move => {
                    entity_move.entry(*id).or_default().push(behavior);
                }
                GeometryTarget::LineStart(id)
                | GeometryTarget::LineEnd(id)
                | GeometryTarget::Vertex { entity: id, .. }
                    if behavior.kind == BehaviorKind::Stretch =>
                {
                    entity_stretch.entry(*id).or_default().push(behavior);
                }
                _ => {}
            }
        }
    }
    for (target, owners) in occupancy {
        if owners.len() < 2 {
            continue;
        }
        for i in 0..owners.len() {
            for j in (i + 1)..owners.len() {
                let left = owners[i];
                let right = owners[j];
                if behaviors_equivalent(left, right) {
                    return Err(DynamicError::OverlappingContribution {
                        parameter: left.parameter,
                        action: left.id,
                        other: right.id,
                        target,
                        reason: "duplicate equivalent behavior",
                    });
                }
                if left.parameter == right.parameter && directions_parallel(left.local_direction, right.local_direction)
                {
                    return Err(DynamicError::OverlappingContribution {
                        parameter: left.parameter,
                        action: left.id,
                        other: right.id,
                        target,
                        reason: "same parameter writes this target twice",
                    });
                }
            }
        }
    }
    for (entity, movers) in entity_move {
        let Some(stretchers) = entity_stretch.get(&entity) else {
            continue;
        };
        for mover in movers {
            for stretcher in stretchers {
                if mover.parameter == stretcher.parameter
                    || directions_parallel(mover.local_direction, stretcher.local_direction)
                {
                    return Err(DynamicError::OverlappingContribution {
                        parameter: mover.parameter,
                        action: mover.id,
                        other: stretcher.id,
                        target: GeometryTarget::Entity(entity),
                        reason: "whole-object move plus endpoint stretch",
                    });
                }
            }
        }
    }
    Ok(())
}

fn behaviors_equivalent(left: &DynamicBehavior, right: &DynamicBehavior) -> bool {
    left.kind == right.kind
        && left.parameter == right.parameter
        && directions_parallel(left.local_direction, right.local_direction)
        && numbers_equal(left.multiplier, right.multiplier)
        && left.follow == right.follow
}

pub fn resolve_values(
    dynamic: &DynamicDefinition,
    instance: Option<&InstanceConfiguration>,
) -> Result<BTreeMap<ParameterId, ParameterValue>, DynamicError> {
    let mut values = BTreeMap::new();
    for parameter in &dynamic.parameters {
        let value = instance
            .and_then(|config| config.get(parameter.id).cloned())
            .unwrap_or_else(|| parameter.default_value());
        validate_parameter_value(parameter, &value)?;
        values.insert(parameter.id, value);
    }
    Ok(values)
}

pub fn numeric_current(
    dynamic: &DynamicDefinition,
    values: &BTreeMap<ParameterId, ParameterValue>,
    id: ParameterId,
) -> Result<f64, DynamicError> {
    let parameter = dynamic
        .parameter(id)
        .ok_or(DynamicError::UnknownParameter {
            action: ActionId::UNASSIGNED,
            parameter: id,
        })?;
    match values.get(&id) {
        Some(ParameterValue::Number(value)) => Ok(*value),
        Some(other) => Err(DynamicError::ValueType {
            parameter: id,
            expected: "number",
            actual: other.type_name(),
        }),
        None => match parameter.default_value() {
            ParameterValue::Number(value) => Ok(value),
            other => Err(DynamicError::ValueType {
                parameter: id,
                expected: "number",
                actual: other.type_name(),
            }),
        },
    }
}

pub fn translate_point(point: Point3, delta: Point2) -> Point3 {
    Point3::new(point.x + delta.x, point.y + delta.y, point.z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::Entity;

    fn line_entity(id: u64, x0: f64, y0: f64, x1: f64, y1: f64) -> Entity {
        let mut entity = Entity::new(Geometry::Line {
            start: Point3::from_xy(x0, y0),
            end: Point3::from_xy(x1, y1),
        });
        entity.id = EntityId(id);
        entity
    }

    #[test]
    fn required_increment_uses_explicit_origin() {
        let mut numeric = NumericParameter::length(10.0);
        numeric.min = Some(5.0);
        numeric.step = Some(2.0);
        numeric.step_policy = StepPolicy::RequiredIncrement;
        numeric.step_origin = StepOrigin::Minimum;
        assert!(validate_numeric_value(ParameterId(1), &numeric, 5.0).is_ok());
        assert!(validate_numeric_value(ParameterId(1), &numeric, 7.0).is_ok());
        assert!(validate_numeric_value(ParameterId(1), &numeric, 6.0).is_err());
        numeric.step_origin = StepOrigin::Zero;
        assert!(validate_numeric_value(ParameterId(1), &numeric, 6.0).is_ok());
        assert!(validate_numeric_value(ParameterId(1), &numeric, 5.0).is_err());
    }

    #[test]
    fn changing_default_does_not_change_reference() {
        let mut numeric = NumericParameter::length(800.0);
        numeric.reference = 800.0;
        numeric.default = 1200.0;
        assert_eq!(numeric.reference, 800.0);
        assert_eq!(numeric.default, 1200.0);
    }

    #[test]
    fn polyline_stretch_requires_straight_vertex_targets() {
        let entity = Entity::new(Geometry::LwPolyline {
            vertices: Vec::new(),
            closed: false,
            extrusion: Point3::new(0.0, 0.0, 1.0),
            linetype_generation_continuous: false,
        });
        let err = capability_for(
            BehaviorKind::Stretch,
            &entity.geometry,
            GeometryTarget::Entity(EntityId(1)),
        );
        assert!(err.is_err());
        let ok = capability_for(
            BehaviorKind::Stretch,
            &entity.geometry,
            GeometryTarget::Vertex {
                entity: EntityId(1),
                vertex: VertexId(1),
            },
        );
        assert!(ok.is_ok());
        let curved = Entity::new(Geometry::LwPolyline {
            vertices: vec![crate::entity::PolyVertex {
                point: Point3::from_xy(0.0, 0.0),
                bulge: 0.5,
                vertex_id: VertexId(1),
            }],
            closed: false,
            extrusion: Point3::new(0.0, 0.0, 1.0),
            linetype_generation_continuous: false,
        });
        let curved_err = capability_for(
            BehaviorKind::Stretch,
            &curved.geometry,
            GeometryTarget::Vertex {
                entity: EntityId(1),
                vertex: VertexId(1),
            },
        );
        assert!(curved_err.unwrap_err().contains("curved"));
    }

    #[test]
    fn broken_binding_is_rejected() {
        let numeric = NumericParameter::length(10.0);
        let dynamic = DynamicDefinition {
            parameters: vec![ParameterDef::number(ParameterId(1), "Span", numeric)],
            behaviors: vec![DynamicBehavior {
                id: ActionId(1),
                kind: BehaviorKind::Move,
                parameter: ParameterId(1),
                targets: vec![GeometryTarget::Entity(EntityId(99))],
                local_direction: Point2::new(1.0, 0.0),
                reference_value: 10.0,
                multiplier: 1.0,
                composition: CompositionRule::Additive,
                follow: FollowRole::Second,
                name: None,
            }],
        };
        let err =
            validate_definition(&dynamic, &[line_entity(1, 0.0, 0.0, 10.0, 0.0)]).unwrap_err();
        assert!(matches!(err, DynamicError::BrokenBinding { .. }));
    }

    #[test]
    fn unknown_choice_is_rejected() {
        let parameter = ParameterDef {
            id: ParameterId(1),
            name: "Side".into(),
            description: None,
            kind: ParameterKind::Choice(ChoiceParameter {
                options: vec![ChoiceOption {
                    id: OptionId(2),
                    label: "Left".into(),
                }],
                default: OptionId(9),
            }),
        };
        assert!(matches!(
            validate_parameter_def(&parameter),
            Err(DynamicError::UnknownChoice { .. })
        ));
    }

    #[test]
    fn format_display_number_keeps_integer_zeros_at_precision_zero() {
        assert_eq!(format_display_number(100.0, 0), "100");
        assert_eq!(format_display_number(100.0, 4), "100");
        assert_eq!(format_display_number(100.5, 4), "100.5");
    }

    #[test]
    fn reverse_pick_order_flips_direction_and_keeps_positive_size() {
        let a = Point2::new(0.0, 0.0);
        let b = Point2::new(800.0, 0.0);
        let (dir, size) = measure_size(a, b, MeasureMode::LocalX).unwrap();
        let (rev, rev_size) = measure_size(b, a, MeasureMode::LocalX).unwrap();
        assert!((size - 800.0).abs() < 1e-12);
        assert!((rev_size - 800.0).abs() < 1e-12);
        assert!((dir.x + rev.x).abs() < 1e-12);
        assert!((dir.y + rev.y).abs() < 1e-12);
    }

    #[test]
    fn allowed_list_rejects_duplicates_and_requires_default() {
        let mut numeric = NumericParameter::length(400.0);
        numeric.domain = NumericDomain::AllowedValues(vec![250.0, 400.0, 600.0]);
        assert!(validate_numeric_def(ParameterId(1), &numeric).is_ok());
        assert!(validate_numeric_value(ParameterId(1), &numeric, 250.0).is_ok());
        assert!(matches!(
            validate_numeric_value(ParameterId(1), &numeric, 300.0),
            Err(DynamicError::ValueNotInList { .. })
        ));
        numeric.default = 300.0;
        assert!(matches!(
            validate_numeric_def(ParameterId(1), &numeric),
            Err(DynamicError::DefaultOutOfDomain { .. })
        ));
        assert!(parse_allowed_value_list("250;400;600").unwrap().len() == 3);
        assert!(parse_allowed_value_list("250\n250").is_err());
        assert!((parse_allowed_value_list("12,5").unwrap()[0] - 12.5).abs() < 1e-12);
    }

    #[test]
    fn follow_center_depends_on_anchor_policy() {
        assert_eq!(follow_multiplier(AnchorPolicy::FirstFixed, FollowRole::Center), 0.5);
        assert_eq!(follow_multiplier(AnchorPolicy::SecondFixed, FollowRole::Center), -0.5);
        assert_eq!(follow_multiplier(AnchorPolicy::CenterFixed, FollowRole::Center), 0.0);
        assert_eq!(follow_multiplier(AnchorPolicy::CenterFixed, FollowRole::First), -0.5);
        assert_eq!(follow_multiplier(AnchorPolicy::CenterFixed, FollowRole::Second), 0.5);
    }

    #[test]
    fn move_plus_stretch_on_same_entity_is_a_conflict() {
        let numeric = NumericParameter::length(10.0);
        let dynamic = DynamicDefinition {
            parameters: vec![ParameterDef::number(ParameterId(1), "Span", numeric)],
            behaviors: vec![
                DynamicBehavior {
                    id: ActionId(1),
                    kind: BehaviorKind::Move,
                    parameter: ParameterId(1),
                    targets: vec![GeometryTarget::Entity(EntityId(1))],
                    local_direction: Point2::new(1.0, 0.0),
                    reference_value: 10.0,
                    multiplier: 1.0,
                    composition: CompositionRule::Additive,
                    follow: FollowRole::Second,
                    name: None,
                },
                DynamicBehavior {
                    id: ActionId(2),
                    kind: BehaviorKind::Stretch,
                    parameter: ParameterId(1),
                    targets: vec![GeometryTarget::LineEnd(EntityId(1))],
                    local_direction: Point2::new(1.0, 0.0),
                    reference_value: 10.0,
                    multiplier: 1.0,
                    composition: CompositionRule::Additive,
                    follow: FollowRole::Second,
                    name: None,
                },
            ],
        };
        let err =
            validate_definition(&dynamic, &[line_entity(1, 0.0, 0.0, 10.0, 0.0)]).unwrap_err();
        assert!(matches!(err, DynamicError::OverlappingContribution { .. }));
    }

    #[test]
    fn orthogonal_parameters_may_share_a_corner() {
        let span = NumericParameter::length(800.0);
        let depth = NumericParameter::length(10.0);
        let dynamic = DynamicDefinition {
            parameters: vec![
                ParameterDef::number(ParameterId(1), "Span", span),
                ParameterDef::number(ParameterId(2), "Depth", depth),
            ],
            behaviors: vec![
                DynamicBehavior {
                    id: ActionId(1),
                    kind: BehaviorKind::Stretch,
                    parameter: ParameterId(1),
                    targets: vec![GeometryTarget::LineEnd(EntityId(1))],
                    local_direction: Point2::new(1.0, 0.0),
                    reference_value: 800.0,
                    multiplier: 1.0,
                    composition: CompositionRule::Additive,
                    follow: FollowRole::Second,
                    name: None,
                },
                DynamicBehavior {
                    id: ActionId(2),
                    kind: BehaviorKind::Stretch,
                    parameter: ParameterId(2),
                    targets: vec![GeometryTarget::LineEnd(EntityId(1))],
                    local_direction: Point2::new(0.0, 1.0),
                    reference_value: 10.0,
                    multiplier: 1.0,
                    composition: CompositionRule::Additive,
                    follow: FollowRole::Second,
                    name: None,
                },
            ],
        };
        validate_definition(&dynamic, &[line_entity(1, 0.0, 0.0, 800.0, 10.0)]).unwrap();
    }

    #[test]
    fn deleted_vertex_is_reported_for_repair() {
        let mut numeric = NumericParameter::length(10.0);
        numeric.reference = 10.0;
        let dynamic = DynamicDefinition {
            parameters: vec![ParameterDef::number(ParameterId(1), "Span", numeric)],
            behaviors: vec![DynamicBehavior {
                id: ActionId(1),
                kind: BehaviorKind::Stretch,
                parameter: ParameterId(1),
                targets: vec![GeometryTarget::Vertex {
                    entity: EntityId(1),
                    vertex: VertexId(99),
                }],
                local_direction: Point2::new(1.0, 0.0),
                reference_value: 10.0,
                multiplier: 1.0,
                composition: CompositionRule::Additive,
                follow: FollowRole::Second,
                name: None,
            }],
        };
        let mut entity = Entity::new(Geometry::LwPolyline {
            vertices: vec![
                crate::entity::PolyVertex {
                    point: Point3::from_xy(0.0, 0.0),
                    bulge: 0.0,
                    vertex_id: VertexId(1),
                },
                crate::entity::PolyVertex {
                    point: Point3::from_xy(10.0, 0.0),
                    bulge: 0.0,
                    vertex_id: VertexId(2),
                },
            ],
            closed: false,
            extrusion: Point3::new(0.0, 0.0, 1.0),
            linetype_generation_continuous: false,
        });
        entity.id = EntityId(1);
        let broken = collect_broken_bindings(&dynamic, &[entity]);
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].2, "deleted vertex");
    }
}
