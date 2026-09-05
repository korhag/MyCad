//! Dynamic-block conversion, authoring panel, and numeric instance editor.

use std::collections::BTreeMap;

use cad_core::{
    apply_anchor_policy, apply_size_axis, collect_broken_bindings, count_block_references,
    dedupe_targets, follow_multiplier, format_display_number, increment_numeric, make_unique_block,
    measure_size, nearest_allowed_values, parse_allowed_value_list, validate_parameter_value,
    ActionId, AnchorPolicy, BehaviorKind, BooleanParameter, ChoiceOption, ChoiceParameter,
    CompositionRule, Document, DynamicBehavior, DynamicDefinition, DynamicError, Entity, EntityId,
    FollowRole, Geometry, GeometryTarget, InstanceConfiguration, MeasureMode, NumericDomain,
    NumericParameter, OptionId, ParameterDef, ParameterId, ParameterKind, ParameterValue, Point2,
    SizeAuthoring, StepPolicy, TextParameter,
};
use eframe::egui::{self, RichText, Ui};

use crate::app::MyCadApp;
use crate::history::{Edit, History};

// ------------------------------------------------------------
// Type: ConvertMode
// Purpose: Shared vs unique conversion when creating a dynamic block.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvertMode {
    Unique,
    Shared,
}

#[derive(Debug, Clone)]
pub struct ConvertDialog {
    pub instance_id: EntityId,
    pub block_name: String,
    pub references: usize,
    pub mode: ConvertMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthoringMode {
    Author,
    Test,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthoringPick {
    Idle,
    SizePointA,
    SizePointB,
    StretchWindowA,
    StretchWindowB,
    LabelOffset,
    MirrorAxisA,
    MirrorAxisB,
    PlaceAttachment,
    RotatePivot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeWizardStage {
    Name,
    PickA,
    PickB,
    Measure,
    Anchor,
    Domain,
    Review,
}

#[derive(Debug, Clone)]
pub struct SizeWizard {
    pub name: String,
    pub stage: SizeWizardStage,
    pub point_a: Option<Point2>,
    pub point_b: Option<Point2>,
    pub hover: Option<Point2>,
    pub measure: MeasureMode,
    pub anchor: AnchorPolicy,
    pub list_mode: bool,
    pub allowed_text: String,
    pub error: Option<String>,
    pub replace: Option<ParameterId>,
}

impl SizeWizard {
    fn new(name: String) -> Self {
        Self {
            name,
            stage: SizeWizardStage::Name,
            point_a: None,
            point_b: None,
            hover: None,
            measure: MeasureMode::AlongPicked,
            anchor: AnchorPolicy::FirstFixed,
            list_mode: false,
            allowed_text: String::new(),
            error: None,
            replace: None,
        }
    }

    fn preview_measure(&self) -> Option<(Point2, f64)> {
        measure_size(self.point_a?, self.point_b.or(self.hover)?, self.measure).ok()
    }

    fn measured(&self) -> Option<(Point2, f64)> {
        measure_size(self.point_a?, self.point_b?, self.measure).ok()
    }
}

#[derive(Debug, Clone)]
pub struct ParameterDraft {
    pub name: String,
    pub numeric: NumericParameter,
    pub original_name: String,
    pub original: NumericParameter,
    pub errors: BTreeMap<String, String>,
    pub allowed_paste: String,
    pub dirty: bool,
}

#[derive(Debug, Clone)]
pub struct KindDraft {
    pub name: String,
    pub description: String,
    pub kind: ParameterKind,
    pub original_name: String,
    pub original: ParameterKind,
    pub original_description: String,
    pub dirty: bool,
    pub errors: BTreeMap<String, String>,
}

impl KindDraft {
    fn from_parameter(parameter: &ParameterDef) -> Self {
        Self {
            name: parameter.name.clone(),
            description: parameter.description.clone().unwrap_or_default(),
            kind: parameter.kind.clone(),
            original_name: parameter.name.clone(),
            original: parameter.kind.clone(),
            original_description: parameter.description.clone().unwrap_or_default(),
            dirty: false,
            errors: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum BehaviorWizard {
    LinkText {
        target: EntityId,
        parameter: ParameterId,
        mode: usize,
        option_texts: BTreeMap<OptionId, String>,
        formatted: String,
        boolean_true: String,
        boolean_false: String,
    },
    ShowWhen {
        members: Vec<EntityId>,
        parameter: ParameterId,
        options: Vec<OptionId>,
        boolean: bool,
    },
    Mirror {
        members: Vec<EntityId>,
        axis_a: Option<Point2>,
        axis_b: Option<Point2>,
        parameter: ParameterId,
        use_visibility: bool,
        policy: cad_core::TextReflectPolicy,
    },
    Position {
        members: Vec<EntityId>,
        attachment: Option<Point2>,
        parameter: ParameterId,
    },
    Rotate {
        members: Vec<EntityId>,
        pivot: Option<Point2>,
        parameter: ParameterId,
        degrees: bool,
    },
}

#[derive(Debug, Clone)]
pub struct OptionMigration {
    pub parameter: ParameterId,
    pub from: OptionId,
    pub replacement: OptionId,
    pub usages: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ConfigureDialog {
    pub entity_ids: Vec<EntityId>,
    pub drafts: BTreeMap<ParameterId, String>,
    pub errors: BTreeMap<ParameterId, String>,
    pub pending: Option<crate::config_form::PendingRelated>,
}

impl ParameterDraft {
    fn from_parameter(name: &str, numeric: &NumericParameter) -> Self {
        Self {
            name: name.to_string(),
            numeric: numeric.clone(),
            original_name: name.to_string(),
            original: numeric.clone(),
            errors: BTreeMap::new(),
            allowed_paste: match &numeric.domain {
                NumericDomain::AllowedValues(values) => values
                    .iter()
                    .map(|value| format_display_number(*value, numeric.display_precision))
                    .collect::<Vec<_>>()
                    .join("\n"),
                NumericDomain::Continuous => String::new(),
            },
            dirty: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AttachSession {
    pub kind: BehaviorKind,
    pub parameter: ParameterId,
    pub follow: FollowRole,
    pub handles: Vec<GeometryTarget>,
    pub unsupported: Vec<String>,
    pub window: Option<(Point2, Point2)>,
    pub replace: Option<ActionId>,
}

#[derive(Debug, Clone)]
pub struct AuthoringState {
    pub block_name: String,
    pub instance_id: EntityId,
    pub created_unique: bool,
    pub previous_name: Option<String>,
    pub selected_parameter: Option<ParameterId>,
    pub selected_behavior: Option<ActionId>,
    pub mode: AuthoringMode,
    pub test_values: BTreeMap<ParameterId, ParameterValue>,
    pub test_drafts: BTreeMap<ParameterId, String>,
    pub test_initial: BTreeMap<ParameterId, ParameterValue>,
    pub test_errors: BTreeMap<ParameterId, String>,
    pub parameter_drafts: BTreeMap<ParameterId, ParameterDraft>,
    pub kind_drafts: BTreeMap<ParameterId, KindDraft>,
    pub size_wizard: Option<SizeWizard>,
    pub wizard: Option<BehaviorWizard>,
    pub option_migration: Option<OptionMigration>,
    pub attach: Option<AttachSession>,
    pub pick: AuthoringPick,
    pub direction_start: Option<Point2>,
    pub direction: Point2,
    pub highlight: Vec<GeometryTarget>,
    pub status: Option<String>,
    pub session_generation: u64,
    pub preview_generation: u64,
    pub preview_dirty: bool,
    pub test_compare: bool,
    pub show_inactive: bool,
    pub test_pending: Option<crate::config_form::PendingRelated>,
}

pub const HANDLE_PICK_PX: f64 = 9.0;

impl AuthoringState {
    pub fn new(
        block_name: String,
        instance_id: EntityId,
        created_unique: bool,
        previous_name: Option<String>,
    ) -> Self {
        Self {
            block_name,
            instance_id,
            created_unique,
            previous_name,
            selected_parameter: None,
            selected_behavior: None,
            mode: AuthoringMode::Author,
            test_values: BTreeMap::new(),
            test_drafts: BTreeMap::new(),
            test_initial: BTreeMap::new(),
            test_errors: BTreeMap::new(),
            parameter_drafts: BTreeMap::new(),
            kind_drafts: BTreeMap::new(),
            size_wizard: None,
            wizard: None,
            option_migration: None,
            attach: None,
            pick: AuthoringPick::Idle,
            direction_start: None,
            direction: Point2::new(1.0, 0.0),
            highlight: Vec::new(),
            status: None,
            session_generation: 1,
            preview_generation: 0,
            preview_dirty: false,
            test_compare: false,
            show_inactive: false,
            test_pending: None,
        }
    }

    pub fn mark_preview_dirty(&mut self) {
        self.preview_dirty = true;
        self.preview_generation = self.preview_generation.wrapping_add(1);
    }

    pub fn enter_test(&mut self, document: &Document) {
        self.mode = AuthoringMode::Test;
        self.pick = AuthoringPick::Idle;
        self.size_wizard = None;
        self.attach = None;
        if let Some(definition) = document.block_by_name(&self.block_name) {
            if let Some(entity) = document.entity_by_id(self.instance_id) {
                self.test_values.clear();
                self.test_drafts.clear();
                self.test_errors.clear();
                if let Some(dynamic) = &definition.dynamic {
                    for parameter in &dynamic.parameters {
                        let value = entity
                            .geometry
                            .insert_configuration()
                            .and_then(|config| config.get(parameter.id).cloned())
                            .unwrap_or_else(|| parameter.default_value());
                        self.test_values.insert(parameter.id, value.clone());
                        if let (ParameterKind::Number(numeric), ParameterValue::Number(number)) =
                            (&parameter.kind, &value)
                        {
                            self.test_drafts.insert(
                                parameter.id,
                                format_display_number(*number, numeric.display_precision),
                            );
                        } else if let ParameterValue::Text(text) = &value {
                            self.test_drafts.insert(parameter.id, text.clone());
                        }
                    }
                }
            }
        }
        self.test_initial = self.test_values.clone();
        self.test_compare = false;
        self.mark_preview_dirty();
    }

    pub fn leave_test(&mut self) {
        self.mode = AuthoringMode::Author;
        self.test_errors.clear();
        self.test_compare = false;
        self.mark_preview_dirty();
    }

    pub fn test_config(&self, document: &Document) -> Option<InstanceConfiguration> {
        if self.mode != AuthoringMode::Test {
            return None;
        }
        let definition = document.block_by_name(&self.block_name)?;
        let dynamic = definition.dynamic.as_ref()?;
        let mut config = InstanceConfiguration::default();
        for parameter in &dynamic.parameters {
            if let Some(value) = self.test_values.get(&parameter.id) {
                config.set(parameter.id, value.clone());
            } else {
                config.set(parameter.id, parameter.default_value());
            }
        }
        Some(config)
    }
}

pub fn selected_insert_id(app: &MyCadApp) -> Option<EntityId> {
    let ids = app.selection.ids();
    if ids.len() != 1 {
        return None;
    }
    let document = app.document.as_deref()?;
    let entity = document.entity_by_id(ids[0])?;
    matches!(entity.geometry, Geometry::Insert { .. }).then_some(ids[0])
}

pub fn begin_new_size_from_menu(app: &mut MyCadApp) {
    if app.dynamic_authoring.is_none() {
        begin_create_dynamic(app);
    }
    start_size_wizard(app);
}

pub fn begin_create_dynamic(app: &mut MyCadApp) {
    let Some(id) = selected_insert_id(app) else {
        app.status = "Select a block reference first".into();
        return;
    };
    let (name, already_dynamic, references) = {
        let Some(document) = app.document.as_deref() else {
            return;
        };
        let Some(entity) = document.entity_by_id(id) else {
            return;
        };
        let Some(name) = entity.geometry.insert_block_name() else {
            app.status = "Select a block reference first".into();
            return;
        };
        if !cad_core::is_user_editable_block_name(name) {
            app.status = "Cannot convert a system block".into();
            return;
        }
        let already_dynamic = document
            .block_by_name(name)
            .is_some_and(|block| block.is_dynamic());
        let references = count_block_references(document, name);
        (name.to_string(), already_dynamic, references)
    };
    if already_dynamic {
        app.enter_block_edit(id);
        app.dynamic_authoring = Some(AuthoringState::new(name, id, false, None));
        crate::workspace::ensure_tab(
            &mut app.dock_state,
            crate::workspace::WorkspaceTab::DynamicBlock,
        );
        app.status = "Editing dynamic block".into();
        return;
    }
    if references <= 1 {
        convert_and_enter(app, id, ConvertMode::Shared);
        return;
    }
    app.dynamic_convert = Some(ConvertDialog {
        instance_id: id,
        block_name: name,
        references,
        mode: ConvertMode::Unique,
    });
}

pub fn convert_and_enter(app: &mut MyCadApp, instance_id: EntityId, mode: ConvertMode) {
    app.history.begin();
    let created = {
        let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) else {
            return;
        };
        match mode {
            ConvertMode::Unique => match make_unique_block(document, instance_id) {
                Ok(result) => {
                    let previous = result
                        .insert_before
                        .geometry
                        .insert_block_name()
                        .map(str::to_string);
                    app.history.record(Edit::ReplaceBlockDefinition {
                        name: result.new_name.clone(),
                        before: None,
                        after: Some(result.definition.clone()),
                    });
                    app.history.record(Edit::ReplaceEntity {
                        space: result.insert_space,
                        index: result.insert_index,
                        before: result.insert_before,
                        after: result.insert_after.clone(),
                    });
                    Some((result.new_name, true, previous))
                }
                Err(err) => {
                    app.status = err.to_string();
                    None
                }
            },
            ConvertMode::Shared => document
                .entity_by_id(instance_id)
                .and_then(|entity| entity.geometry.insert_block_name().map(str::to_string))
                .map(|name| (name, false, None)),
        }
    };
    let Some((name, unique, previous)) = created else {
        app.history.commit_open();
        return;
    };
    {
        let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) else {
            return;
        };
        if let Some(block) = document.block_by_name_mut(&name) {
            if block.dynamic.is_none() {
                let before = block.clone();
                block.dynamic = Some(DynamicDefinition::default());
                let after = block.clone();
                app.history.record(Edit::ReplaceBlockDefinition {
                    name: name.clone(),
                    before: Some(before),
                    after: Some(after),
                });
            }
        }
    }
    app.history.commit_open();
    app.enter_block_edit(instance_id);
    app.dynamic_authoring = Some(AuthoringState::new(name, instance_id, unique, previous));
    crate::workspace::ensure_tab(
        &mut app.dock_state,
        crate::workspace::WorkspaceTab::DynamicBlock,
    );
    app.status = "Editing dynamic block".into();
    app.refresh_derived();
}

pub fn revert_unique_conversion(app: &mut MyCadApp) {
    if !app
        .dynamic_authoring
        .as_ref()
        .is_some_and(|state| state.created_unique)
    {
        return;
    }
    let Some(state) = app.dynamic_authoring.take() else {
        return;
    };
    app.history.begin();
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        if let Some(previous) = &state.previous_name {
            if let Some((space, index)) = document.find_entity_location(state.instance_id) {
                if let Some(before) = document.entity_by_id(state.instance_id).cloned() {
                    let mut after = before.clone();
                    if let Some(name) = after.geometry.insert_block_name_mut() {
                        *name = previous.clone();
                    }
                    let _ = document.replace_entity_in(&space, state.instance_id, after.clone());
                    app.history.record(Edit::ReplaceEntity {
                        space,
                        index,
                        before,
                        after,
                    });
                }
            }
        }
        if let Some(definition) = document.remove_block_definition(&state.block_name) {
            app.history.record(Edit::ReplaceBlockDefinition {
                name: state.block_name,
                before: Some(definition),
                after: None,
            });
        }
    }
    app.history.commit_open();
}

pub fn discard_authoring(app: &mut MyCadApp) {
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        app.block_edit.discard_current(document, &mut app.history);
    }
    revert_unique_conversion(app);
    app.status = "Discarded dynamic-block authoring".into();
    app.refresh_derived();
}

pub fn add_size_parameter(
    document: &mut Document,
    history: &mut History,
    block_name: &str,
    name: String,
    numeric: NumericParameter,
) -> Result<ParameterId, String> {
    let Some(block) = document.block_by_name(block_name).cloned() else {
        return Err("Block definition was not found".into());
    };
    let mut after = block.clone();
    let id = document.allocate_parameter_id();
    let mut numeric = numeric;
    numeric.display_order = after
        .dynamic
        .as_ref()
        .map(|dynamic| dynamic.parameters.len() as i32)
        .unwrap_or(0);
    let name = unique_parameter_name(after.dynamic.as_ref(), &name);
    let dynamic = after.dynamic.get_or_insert_with(DynamicDefinition::default);
    dynamic
        .parameters
        .push(ParameterDef::number(id, name, numeric));
    cad_core::validate_definition(dynamic, &after.entities).map_err(|err| err.to_string())?;
    document.replace_block_definition(after.clone());
    history.record(Edit::ReplaceBlockDefinition {
        name: block_name.to_string(),
        before: Some(block),
        after: Some(after),
    });
    Ok(id)
}

pub fn add_number_parameter(document: &mut Document, history: &mut History, block_name: &str) {
    let mut numeric = NumericParameter::length(0.0);
    numeric.reference = 0.0;
    let _ = add_size_parameter(document, history, block_name, "Offset".into(), numeric);
}

fn unique_parameter_name(dynamic: Option<&DynamicDefinition>, base: &str) -> String {
    let Some(dynamic) = dynamic else {
        return base.into();
    };
    if !dynamic.parameters.iter().any(|parameter| parameter.name == base) {
        return base.into();
    }
    for index in 2..1000 {
        let candidate = format!("{base} {index}");
        if !dynamic
            .parameters
            .iter()
            .any(|parameter| parameter.name == candidate)
        {
            return candidate;
        }
    }
    format!("{base} new")
}

fn add_choice_parameter(app: &mut MyCadApp) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        app.history.begin();
        let name = unique_parameter_name(
            document
                .block_by_name(&state.block_name)
                .and_then(|block| block.dynamic.as_ref()),
            "Choice",
        );
        let id = document.allocate_parameter_id();
        let first = document.allocate_option_id();
        let second = document.allocate_option_id();
        let order = next_display_order(document, &state.block_name);
        let mut parameter = ParameterDef::choice(
            id,
            name,
            ChoiceParameter {
                options: vec![
                    ChoiceOption {
                        id: first,
                        label: "Option A".into(),
                    },
                    ChoiceOption {
                        id: second,
                        label: "Option B".into(),
                    },
                ],
                default: first,
            },
        );
        parameter.display_order = order;
        push_parameter(document, &mut app.history, &state.block_name, parameter);
        app.history.commit_open();
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.selected_parameter = Some(id);
        }
        app.request_preview();
    }
}

fn add_boolean_parameter(app: &mut MyCadApp) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        app.history.begin();
        let name = unique_parameter_name(
            document
                .block_by_name(&state.block_name)
                .and_then(|block| block.dynamic.as_ref()),
            "On/Off",
        );
        let id = document.allocate_parameter_id();
        let mut parameter = ParameterDef::boolean(id, name, BooleanParameter::default());
        parameter.display_order = next_display_order(document, &state.block_name);
        push_parameter(document, &mut app.history, &state.block_name, parameter);
        app.history.commit_open();
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.selected_parameter = Some(id);
        }
        app.request_preview();
    }
}

fn add_text_parameter(app: &mut MyCadApp) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        app.history.begin();
        let name = unique_parameter_name(
            document
                .block_by_name(&state.block_name)
                .and_then(|block| block.dynamic.as_ref()),
            "Text",
        );
        let id = document.allocate_parameter_id();
        let mut parameter = ParameterDef::text(id, name, TextParameter::default());
        parameter.display_order = next_display_order(document, &state.block_name);
        push_parameter(document, &mut app.history, &state.block_name, parameter);
        app.history.commit_open();
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.selected_parameter = Some(id);
        }
        app.request_preview();
    }
}

fn next_display_order(document: &Document, block_name: &str) -> i32 {
    document
        .block_by_name(block_name)
        .and_then(|block| block.dynamic.as_ref())
        .map(|dynamic| {
            dynamic
                .parameters
                .iter()
                .map(ParameterDef::presentation_order)
                .max()
                .unwrap_or(0)
                + 1
        })
        .unwrap_or(0)
}

fn push_parameter(
    document: &mut Document,
    history: &mut History,
    block_name: &str,
    parameter: ParameterDef,
) {
    let Some(block) = document.block_by_name(block_name).cloned() else {
        return;
    };
    let mut after = block.clone();
    after
        .dynamic
        .get_or_insert_with(DynamicDefinition::default)
        .parameters
        .push(parameter);
    document.replace_block_definition(after.clone());
    history.record(Edit::ReplaceBlockDefinition {
        name: block_name.to_string(),
        before: Some(block),
        after: Some(after),
    });
}

fn replace_parameter_kind(
    document: &mut Document,
    history: &mut History,
    block_name: &str,
    id: ParameterId,
    name: String,
    kind: ParameterKind,
    display_order: i32,
    description: Option<String>,
) -> Result<(), String> {
    let Some(block) = document.block_by_name(block_name).cloned() else {
        return Err("Block definition was not found".into());
    };
    let mut after = block.clone();
    let Some(parameter) = after
        .dynamic
        .as_mut()
        .and_then(|dynamic| dynamic.parameters.iter_mut().find(|item| item.id == id))
    else {
        return Err("Parameter was not found".into());
    };
    parameter.name = name;
    parameter.kind = kind;
    parameter.display_order = display_order;
    parameter.description = description;
    cad_core::validate_parameter_def(parameter).map_err(|err| err.to_string())?;
    cad_core::validate_definition(after.dynamic.as_ref().unwrap(), &after.entities)
        .map_err(|err| err.to_string())?;
    document.replace_block_definition(after.clone());
    history.record(Edit::ReplaceBlockDefinition {
        name: block_name.to_string(),
        before: Some(block),
        after: Some(after),
    });
    Ok(())
}

fn follow_from_multiplier(multiplier: f64) -> FollowRole {
    if (multiplier - 1.0).abs() < 1e-9 {
        FollowRole::Second
    } else if (multiplier + 1.0).abs() < 1e-9 {
        FollowRole::First
    } else if (multiplier - 0.5).abs() < 1e-9 || (multiplier + 0.5).abs() < 1e-9 {
        FollowRole::Center
    } else if multiplier.abs() < 1e-9 {
        FollowRole::First
    } else {
        FollowRole::Custom
    }
}

pub fn attach_behavior_with_follow(
    document: &mut Document,
    history: &mut History,
    block_name: &str,
    parameter: ParameterId,
    kind: BehaviorKind,
    targets: Vec<GeometryTarget>,
    follow: FollowRole,
) -> Result<(), String> {
    let Some(block) = document.block_by_name(block_name) else {
        return Err("Block definition was not found".into());
    };
    let Some(dynamic) = block.dynamic.as_ref() else {
        return Err("Block is not dynamic".into());
    };
    let Some(param) = dynamic.parameter(parameter) else {
        return Err("Select a parameter first".into());
    };
    let ParameterKind::Number(numeric) = &param.kind else {
        return Err("Move and Stretch require a numeric parameter".into());
    };
    let direction = numeric
        .size
        .as_ref()
        .map(|size| size.direction)
        .filter(|dir| cad_core::normalize_direction(*dir).is_some())
        .unwrap_or(Point2::new(1.0, 0.0));
    let anchor = numeric
        .size
        .as_ref()
        .map(|size| size.anchor)
        .unwrap_or(AnchorPolicy::FirstFixed);
    let multiplier = if follow == FollowRole::Custom {
        1.0
    } else {
        follow_multiplier(anchor, follow)
    };
    attach_behavior(
        document,
        history,
        block_name,
        parameter,
        kind,
        targets,
        direction,
        multiplier,
    )?;
    Ok(())
}

pub fn attach_behavior(
    document: &mut Document,
    history: &mut History,
    block_name: &str,
    parameter: ParameterId,
    kind: BehaviorKind,
    targets: Vec<GeometryTarget>,
    direction: Point2,
    multiplier: f64,
) -> Result<(), String> {
    let Some(block) = document.block_by_name(block_name).cloned() else {
        return Err("Block definition was not found".into());
    };
    let Some(dynamic) = block.dynamic.as_ref() else {
        return Err("Block is not dynamic".into());
    };
    let Some(param) = dynamic.parameter(parameter) else {
        return Err("Select a parameter first".into());
    };
    let ParameterKind::Number(numeric) = &param.kind else {
        return Err("Move and Stretch require a numeric parameter".into());
    };
    let targets = dedupe_targets(targets);
    if targets.is_empty() {
        return Err("Select target geometry first".into());
    }
    if let Some(existing) = dynamic.behaviors.iter().find(|behavior| {
        behavior.parameter == parameter
            && behavior.kind == kind
            && behavior.targets.iter().any(|target| targets.contains(target))
    }) {
        return Err(format!(
            "Those targets are already attached to {}",
            existing.describe(dynamic)
        ));
    }
    for target in &targets {
        let entity = block
            .entities
            .iter()
            .find(|entity| entity.id == target.entity_id())
            .ok_or("Bound geometry is missing")?;
        cad_core::capability_for(kind, &entity.geometry, *target).map_err(|reason| {
            format!("Cannot {reason}")
        })?;
    }
    let mut after = block.clone();
    let action = document.allocate_action_id();
    after
        .dynamic
        .get_or_insert_with(DynamicDefinition::default)
        .behaviors
        .push(DynamicBehavior {
            id: action,
            kind,
            parameter,
            targets,
            local_direction: cad_core::normalize_direction(direction)
                .unwrap_or(Point2::new(1.0, 0.0)),
            reference_value: numeric.reference,
            multiplier,
            composition: CompositionRule::Additive,
            follow: follow_from_multiplier(multiplier),
            name: None,
        });
    cad_core::validate_definition(
        after.dynamic.as_ref().unwrap(),
        &after.entities,
    )
    .map_err(|err| err.to_string())?;
    document.replace_block_definition(after.clone());
    history.record(Edit::ReplaceBlockDefinition {
        name: block_name.to_string(),
        before: Some(block),
        after: Some(after),
    });
    Ok(())
}

pub fn commit_instance_value(
    document: &mut Document,
    history: &mut History,
    entity_id: EntityId,
    parameter: ParameterId,
    value: ParameterValue,
) -> Result<(), String> {
    let mut patch = BTreeMap::new();
    patch.insert(parameter, value);
    commit_instance_values(document, history, &[entity_id], &patch)
}

pub fn commit_instance_values(
    document: &mut Document,
    history: &mut History,
    entity_ids: &[EntityId],
    patch: &BTreeMap<ParameterId, ParameterValue>,
) -> Result<(), String> {
    let mut planned = Vec::new();
    for entity_id in entity_ids {
        let (space, index) = document
            .find_entity_location(*entity_id)
            .ok_or("Block reference was not found")?;
        let before = document
            .entity_by_id(*entity_id)
            .cloned()
            .ok_or("Block reference was not found")?;
        let block_name = before
            .geometry
            .insert_block_name()
            .ok_or("Select a block reference")?
            .to_string();
        let definition = document
            .block_by_name(&block_name)
            .ok_or("Block definition was not found")?;
        let dynamic = definition
            .dynamic
            .as_ref()
            .ok_or("Parameter was not found")?;
        let mut after = before.clone();
        let config = after
            .geometry
            .insert_configuration_mut()
            .ok_or("Select a block reference")?;
        let mut values = config.take().unwrap_or_default();
        for (parameter, value) in patch {
            let param = dynamic
                .parameter(*parameter)
                .ok_or("Parameter was not found")?;
            validate_parameter_value(param, value).map_err(|err| err.to_string())?;
            values.set(*parameter, value.clone());
        }
        cad_core::validate_configuration(dynamic, &values.values).map_err(|err| err.to_string())?;
        *config = Some(values);
        planned.push((space, index, before, after));
    }
    for (space, index, before, after) in planned {
        document
            .replace_entity_in(&space, before.id, after.clone())
            .ok_or("Block reference was not found")?;
        history.record(Edit::ReplaceEntity {
            space,
            index,
            before,
            after,
        });
    }
    Ok(())
}

pub fn apply_preset_to_instances(
    document: &mut Document,
    history: &mut History,
    entity_ids: &[EntityId],
    preset_id: cad_core::PresetId,
) -> Result<(), String> {
    let first = entity_ids.first().copied().ok_or("Select a block reference")?;
    let name = document
        .entity_by_id(first)
        .and_then(|entity| entity.geometry.insert_block_name().map(str::to_string))
        .ok_or("Select a block reference")?;
    let definition = document
        .block_by_name(&name)
        .ok_or("Block definition was not found")?;
    let preset = definition
        .dynamic
        .as_ref()
        .and_then(|dynamic| dynamic.preset(preset_id))
        .ok_or("Preset was not found")?;
    let patch = preset.values.clone();
    commit_instance_values(document, history, entity_ids, &patch)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvertDialogResult {
    Open,
    Cancel,
    Continue(ConvertMode),
}

pub fn show_convert_dialog(ctx: &egui::Context, dialog: &mut ConvertDialog) -> ConvertDialogResult {
    let mut result = ConvertDialogResult::Open;
    let mut open = true;
    egui::Window::new("Create a Dynamic Block")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(format!(
                "“{}” is used by {} references.",
                dialog.block_name, dialog.references
            ));
            ui.add_space(8.0);
            ui.radio_value(
                &mut dialog.mode,
                ConvertMode::Unique,
                "Create a unique dynamic definition for this reference (recommended)",
            );
            ui.radio_value(
                &mut dialog.mode,
                ConvertMode::Shared,
                format!(
                    "Convert the shared definition (affects {} references)",
                    dialog.references
                ),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Continue").clicked() {
                    result = ConvertDialogResult::Continue(dialog.mode);
                }
                if ui.button("Cancel").clicked() {
                    result = ConvertDialogResult::Cancel;
                }
            });
        });
    if !open {
        ConvertDialogResult::Cancel
    } else {
        result
    }
}

pub fn show_authoring(ui: &mut Ui, app: &mut MyCadApp) {
    ui.heading("Dynamic Block");
    ui.separator();
    let Some(state) = app.dynamic_authoring.clone() else {
        ui.weak("Use Create a Dynamic Block… on a block reference.");
        return;
    };
    let references = app
        .document
        .as_deref()
        .map(|document| count_block_references(document, &state.block_name))
        .unwrap_or(0);
    let mode_text = match state.mode {
        AuthoringMode::Author => format!("Authoring definition — affects {references} references"),
        AuthoringMode::Test => "Testing configuration".to_string(),
    };
    ui.colored_label(egui::Color32::from_rgb(180, 210, 255), RichText::new(mode_text).strong());
    ui.label(RichText::new(&state.block_name).strong());
    if let Some(status) = &state.status {
        ui.colored_label(egui::Color32::from_rgb(220, 160, 80), status);
    }
    ui.add_space(6.0);
    ui.vertical(|ui| {
        match state.mode {
            AuthoringMode::Author => {
                if ui.button("Test mode").clicked() {
                    if let Some(document) = app.document.as_deref() {
                        if let Some(authoring) = app.dynamic_authoring.as_mut() {
                            authoring.enter_test(document);
                        }
                    }
                }
            }
            AuthoringMode::Test => {
                if ui.button("Return to authoring").clicked() {
                    if let Some(authoring) = app.dynamic_authoring.as_mut() {
                        authoring.leave_test();
                    }
                    app.request_preview();
                }
            }
        }
        if ui.button("Add Size…").clicked() {
            start_size_wizard(app);
        }
        if ui.button("Add Choice…").clicked() {
            add_choice_parameter(app);
        }
        if ui.button("Add On/Off…").clicked() {
            add_boolean_parameter(app);
        }
        if ui.button("Add Text…").clicked() {
            add_text_parameter(app);
        }
        ui.collapsing("Advanced", |ui| {
            if ui.button("Add zero-based offset").clicked() {
                let name = state.block_name.clone();
                if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
                    app.history.begin();
                    add_number_parameter(document, &mut app.history, &name);
                    app.history.commit_open();
                    app.request_preview();
                }
            }
        });
    });
    if app.dynamic_authoring.as_ref().and_then(|s| s.size_wizard.as_ref()).is_some() {
        show_size_wizard(ui, app);
        show_save_discard(ui, app);
        return;
    }
    let (dynamic, entities) = {
        let Some(document) = app.document.as_deref() else {
            return;
        };
        let Some(block) = document.block_by_name(&state.block_name) else {
            ui.weak("The definition is no longer in the drawing.");
            return;
        };
        (
            block.dynamic.clone().unwrap_or_default(),
            block.entities.clone(),
        )
    };
    ui.add_space(6.0);
    ui.label(RichText::new("Parameters").small().weak());
    let mut selected = state.selected_parameter;
    for parameter in dynamic.sorted_parameters() {
        if ui.selectable_label(selected == Some(parameter.id), &parameter.name).clicked() {
            selected = Some(parameter.id);
        }
    }
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.selected_parameter = selected;
    }
    if state.mode == AuthoringMode::Author {
        show_repair_bindings(ui, app, &dynamic, &entities);
        show_selection_bindings(ui, app, &dynamic);
        if let Some(parameter) = selected.and_then(|id| dynamic.parameter(id)).cloned() {
            match &parameter.kind {
                ParameterKind::Number(numeric) => {
                    show_parameter_editor(ui, app, &state.block_name, parameter.id, &parameter.name, numeric);
                    show_behavior_list(ui, app, &dynamic, parameter.id);
                }
                ParameterKind::Choice(choice) => {
                    show_choice_editor(ui, app, &state.block_name, &parameter, choice);
                }
                ParameterKind::Boolean(flag) => {
                    show_boolean_editor(ui, app, &state.block_name, &parameter, flag);
                }
                ParameterKind::Text(text) => {
                    show_text_editor(ui, app, &state.block_name, &parameter, text);
                }
            }
        }
        show_attach_controls(ui, app);
        show_behavior_wizard(ui, app, &dynamic);
        show_option_migration(ui, app);
        show_phase3_authoring(ui, app, &dynamic);
    } else {
        show_test_controls(ui, app, &dynamic);
    }
    show_save_discard(ui, app);
}

fn show_save_discard(ui: &mut Ui, app: &mut MyCadApp) {
    ui.add_space(12.0);
    ui.vertical(|ui| {
        if ui.button("Save").clicked() {
            app.save_active_block();
        }
        if ui.button("Discard").clicked() {
            discard_authoring(app);
            app.block_edit.pop();
            app.request_preview();
        }
    });
}

fn start_size_wizard(app: &mut MyCadApp) {
    let Some(state) = app.dynamic_authoring.as_mut() else {
        return;
    };
    let name = unique_parameter_name(
        app.document
            .as_deref()
            .and_then(|document| document.block_by_name(&state.block_name))
            .and_then(|block| block.dynamic.as_ref()),
        "Size",
    );
    state.size_wizard = Some(SizeWizard::new(name));
    state.status = Some("Name the size, then pick two reference points.".into());
}

fn show_size_wizard(ui: &mut Ui, app: &mut MyCadApp) {
    let Some(mut wizard) = app
        .dynamic_authoring
        .as_ref()
        .and_then(|state| state.size_wizard.clone())
    else {
        return;
    };
    ui.add_space(6.0);
    ui.label(RichText::new(if wizard.replace.is_some() {
        "Change reference points"
    } else {
        "Add Size"
    }).strong());
    if let Some(error) = &wizard.error {
        ui.colored_label(egui::Color32::from_rgb(220, 90, 70), error);
    }
    ui.vertical(|ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut wizard.name);
        let stage = match wizard.stage {
            SizeWizardStage::Name => "Enter a name, then pick the first point.",
            SizeWizardStage::PickA => "Pick the first reference point.",
            SizeWizardStage::PickB => "Pick the second reference point.",
            SizeWizardStage::Measure => "Choose the measurement direction.",
            SizeWizardStage::Anchor => "Choose which side stays fixed.",
            SizeWizardStage::Domain => "Any valid size, or a list of available sizes.",
            SizeWizardStage::Review => "Review the measured size and confirm.",
        };
        ui.weak(stage);
        match wizard.stage {
            SizeWizardStage::Measure => {
                ui.radio_value(&mut wizard.measure, MeasureMode::AlongPicked, "Along picked points");
                ui.radio_value(&mut wizard.measure, MeasureMode::LocalX, "Local X");
                ui.radio_value(&mut wizard.measure, MeasureMode::LocalY, "Local Y");
            }
            SizeWizardStage::Anchor => {
                ui.radio_value(&mut wizard.anchor, AnchorPolicy::FirstFixed, "Keep first side fixed");
                ui.radio_value(&mut wizard.anchor, AnchorPolicy::SecondFixed, "Keep second side fixed");
                ui.radio_value(&mut wizard.anchor, AnchorPolicy::CenterFixed, "Keep center fixed");
            }
            SizeWizardStage::Domain => {
                ui.checkbox(&mut wizard.list_mode, "Choose from available sizes");
                if wizard.list_mode {
                    ui.label("Available sizes (one per line, or semicolon-separated)");
                    ui.text_edit_multiline(&mut wizard.allowed_text);
                } else {
                    ui.label("Any valid size");
                }
            }
            SizeWizardStage::Review => {
                ui.label(wizard.measure.label());
                ui.label(wizard.anchor.label());
                if wizard.list_mode {
                    ui.label("Available sizes:");
                    ui.monospace(&wizard.allowed_text);
                } else {
                    ui.label("Input: any valid size");
                }
            }
            _ => {}
        }
        if let Some((direction, size)) = wizard.preview_measure() {
            ui.label(format!(
                "Measured {} along {:.3}, {:.3}",
                format_display_number(size, 4),
                direction.x,
                direction.y
            ));
        }
        ui.horizontal(|ui| {
            if wizard.stage == SizeWizardStage::Name && ui.button("Pick points").clicked() {
                wizard.stage = SizeWizardStage::PickA;
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    authoring.pick = AuthoringPick::SizePointA;
                    authoring.status = Some("Specify first reference point".into());
                }
            }
            if matches!(
                wizard.stage,
                SizeWizardStage::Measure | SizeWizardStage::Anchor | SizeWizardStage::Domain
            ) && ui.button("Next").clicked()
            {
                wizard.stage = match wizard.stage {
                    SizeWizardStage::Measure => SizeWizardStage::Anchor,
                    SizeWizardStage::Anchor => SizeWizardStage::Domain,
                    _ => SizeWizardStage::Review,
                };
            }
            if wizard.stage == SizeWizardStage::Review && ui.button("Confirm").clicked() {
                confirm_size_wizard(app, &wizard);
                return;
            }
            if !matches!(
                wizard.stage,
                SizeWizardStage::Name | SizeWizardStage::PickA | SizeWizardStage::PickB
            ) && ui.button("Back").clicked()
            {
                wizard.stage = match wizard.stage {
                    SizeWizardStage::Review => SizeWizardStage::Domain,
                    SizeWizardStage::Domain => SizeWizardStage::Anchor,
                    SizeWizardStage::Anchor => SizeWizardStage::Measure,
                    _ => SizeWizardStage::Name,
                };
            }
            if ui.button("Cancel").clicked() {
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    authoring.size_wizard = None;
                    authoring.pick = AuthoringPick::Idle;
                    authoring.status = Some("Size creation cancelled".into());
                }
                return;
            }
        });
    });
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.size_wizard = Some(wizard);
    }
}

fn start_retarget_wizard(app: &mut MyCadApp, id: ParameterId, name: &str, numeric: &NumericParameter) {
    let Some(size) = numeric.size.clone() else {
        return;
    };
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.size_wizard = Some(SizeWizard {
            name: name.to_string(),
            stage: SizeWizardStage::PickA,
            point_a: None,
            point_b: None,
            hover: None,
            measure: size.measure,
            anchor: size.anchor,
            list_mode: numeric.domain.is_list(),
            allowed_text: match &numeric.domain {
                NumericDomain::AllowedValues(values) => values
                    .iter()
                    .map(|value| format_display_number(*value, numeric.display_precision))
                    .collect::<Vec<_>>()
                    .join("\n"),
                NumericDomain::Continuous => String::new(),
            },
            error: None,
            replace: Some(id),
        });
        authoring.pick = AuthoringPick::SizePointA;
        authoring.status = Some("Pick new first reference point. This redefines the size.".into());
    }
}

fn confirm_size_wizard(app: &mut MyCadApp, wizard: &SizeWizard) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    let Some(point_a) = wizard.point_a else {
        return;
    };
    let Some(point_b) = wizard.point_b else {
        return;
    };
    let Ok((direction, reference)) = measure_size(point_a, point_b, wizard.measure) else {
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            if let Some(wizard) = authoring.size_wizard.as_mut() {
                wizard.error = Some("Points must have a measurable distance.".into());
            }
        }
        return;
    };
    let mut numeric = NumericParameter::length(reference);
    numeric.reference = reference;
    numeric.default = reference;
    if wizard.list_mode {
        match parse_allowed_value_list(&wizard.allowed_text) {
            Ok(values) => {
                if !values.iter().any(|value| cad_core::numbers_equal(*value, reference)) {
                    if let Some(authoring) = app.dynamic_authoring.as_mut() {
                        if let Some(wizard) = authoring.size_wizard.as_mut() {
                            wizard.error = Some(format!(
                                "Measured size {} is not in the list. Add it or cancel — the block will not be deformed.",
                                format_display_number(reference, 4)
                            ));
                        }
                    }
                    return;
                }
                numeric.domain = NumericDomain::AllowedValues(values);
            }
            Err(err) => {
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    if let Some(wizard) = authoring.size_wizard.as_mut() {
                        wizard.error = Some(err);
                    }
                }
                return;
            }
        }
    }
    numeric.size = Some(SizeAuthoring {
        point_a,
        point_b,
        measure: wizard.measure,
        direction,
        anchor: wizard.anchor,
        label_offset: Point2::new(0.0, 0.0),
        bound_anchor: None,
    });
    let name = wizard.name.clone();
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        app.history.begin();
        let result = if let Some(id) = wizard.replace {
            apply_parameter_settings(
                document,
                &mut app.history,
                &state.block_name,
                id,
                name,
                numeric,
            )
            .map(|_| id)
            .map_err(|err| err.message)
        } else {
            add_size_parameter(document, &mut app.history, &state.block_name, name, numeric)
        };
        match result {
            Ok(id) => {
                app.history.commit_open();
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    authoring.selected_parameter = Some(id);
                    authoring.direction = direction;
                    authoring.size_wizard = None;
                    authoring.pick = AuthoringPick::Idle;
                    authoring.status = Some(if wizard.replace.is_some() {
                        "Reference points updated".into()
                    } else {
                        "Size created from measured geometry".into()
                    });
                }
                app.request_preview();
            }
            Err(err) => {
                app.history.commit_open();
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    if let Some(wizard) = authoring.size_wizard.as_mut() {
                        wizard.error = Some(err);
                    }
                }
            }
        }
    }
}

fn show_choice_editor(
    ui: &mut Ui,
    app: &mut MyCadApp,
    block_name: &str,
    parameter: &ParameterDef,
    choice: &ChoiceParameter,
) {
    ui.add_space(4.0);
    ui.label(RichText::new("Choice").small().weak());
    let mut draft = app
        .dynamic_authoring
        .as_ref()
        .and_then(|state| state.kind_drafts.get(&parameter.id).cloned())
        .unwrap_or_else(|| KindDraft::from_parameter(parameter));
    if !matches!(draft.kind, ParameterKind::Choice(_)) {
        draft = KindDraft::from_parameter(parameter);
    }
    ui.label("Name");
    if ui.text_edit_singleline(&mut draft.name).changed() {
        draft.dirty = true;
    }
    ui.label("Description");
    if ui.text_edit_singleline(&mut draft.description).changed() {
        draft.dirty = true;
    }
    ui.label("Options");
    let mut dirty = draft.dirty;
    let mut choice_draft = match &draft.kind {
        ParameterKind::Choice(choice) => choice.clone(),
        _ => return,
    };
    let mut remove = None;
    let option_count = choice_draft.options.len();
    for index in 0..option_count {
        ui.horizontal(|ui| {
            if ui.radio(choice_draft.default == choice_draft.options[index].id, "").clicked() {
                choice_draft.default = choice_draft.options[index].id;
                dirty = true;
            }
            if ui.text_edit_singleline(&mut choice_draft.options[index].label).changed() {
                dirty = true;
            }
            if ui.small_button("↑").clicked() && index > 0 {
                choice_draft.options.swap(index - 1, index);
                dirty = true;
            }
            if ui.small_button("↓").clicked() && index + 1 < choice_draft.options.len() {
                choice_draft.options.swap(index, index + 1);
                dirty = true;
            }
            if ui.small_button("Remove").clicked() {
                remove = Some(choice_draft.options[index].id);
            }
        });
    }
    if let Some(id) = remove {
        if choice_draft.options.len() > 1 {
            begin_option_migration(app, parameter.id, id, choice);
            return;
        } else if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.status = Some("A choice needs at least one option.".into());
        }
    }
    if ui.button("Add option").clicked() {
        if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
            let id = document.allocate_option_id();
            choice_draft.options.push(ChoiceOption {
                id,
                label: format!("Option {}", choice_draft.options.len() + 1),
            });
            dirty = true;
        }
    }
    draft.kind = ParameterKind::Choice(choice_draft);
    draft.dirty = dirty;
    if let Some(err) = draft.errors.get("label") {
        ui.colored_label(egui::Color32::from_rgb(220, 90, 70), err);
    }
    ui.horizontal(|ui| {
        if ui.button("Apply").clicked() {
            apply_kind_draft(app, block_name, parameter.id, &mut draft);
        }
        if ui.button("Cancel").clicked() {
            draft = KindDraft::from_parameter(parameter);
        }
    });
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.kind_drafts.insert(parameter.id, draft);
    }
}

fn begin_option_migration(
    app: &mut MyCadApp,
    parameter: ParameterId,
    from: OptionId,
    choice: &ChoiceParameter,
) {
    let Some(state) = app.dynamic_authoring.as_ref() else {
        return;
    };
    let Some(document) = app.document.as_deref() else {
        return;
    };
    let Some(block) = document.block_by_name(&state.block_name) else {
        return;
    };
    let Some(dynamic) = block.dynamic.as_ref() else {
        return;
    };
    let instances: Vec<(EntityId, InstanceConfiguration)> = document
        .model_space
        .iter()
        .filter(|entity| entity.geometry.insert_block_name() == Some(state.block_name.as_str()))
        .filter_map(|entity| {
            entity
                .geometry
                .insert_configuration()
                .cloned()
                .map(|config| (entity.id, config))
        })
        .collect();
    let refs: Vec<(EntityId, &InstanceConfiguration)> = instances
        .iter()
        .map(|(id, config)| (*id, config))
        .collect();
    let usages = cad_core::option_usages(
        parameter,
        from,
        &dynamic.visibility,
        &dynamic.text_bindings,
        &dynamic.reflections,
        &dynamic.rotations,
        &dynamic.placements,
        &dynamic.nested_inputs,
        &dynamic.compatibility,
        &dynamic.presets,
        &refs,
    );
    let replacement = choice
        .options
        .iter()
        .find(|option| option.id != from)
        .map(|option| option.id)
        .unwrap_or(from);
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.option_migration = Some(OptionMigration {
            parameter,
            from,
            replacement,
            usages,
        });
        authoring.status = Some("Review option removal before applying.".into());
    }
}

fn show_boolean_editor(
    ui: &mut Ui,
    app: &mut MyCadApp,
    block_name: &str,
    parameter: &ParameterDef,
    _flag: &BooleanParameter,
) {
    ui.add_space(4.0);
    ui.label(RichText::new("On/Off").small().weak());
    let mut draft = app
        .dynamic_authoring
        .as_ref()
        .and_then(|state| state.kind_drafts.get(&parameter.id).cloned())
        .unwrap_or_else(|| KindDraft::from_parameter(parameter));
    ui.label("Name");
    if ui.text_edit_singleline(&mut draft.name).changed() {
        draft.dirty = true;
    }
    ui.label("Description");
    if ui.text_edit_singleline(&mut draft.description).changed() {
        draft.dirty = true;
    }
    let mut flag = match &draft.kind {
        ParameterKind::Boolean(flag) => flag.clone(),
        _ => return,
    };
    if ui.checkbox(&mut flag.default, "Default on").changed() {
        draft.dirty = true;
    }
    ui.label("On label");
    if ui.text_edit_singleline(&mut flag.true_label).changed() {
        draft.dirty = true;
    }
    ui.label("Off label");
    if ui.text_edit_singleline(&mut flag.false_label).changed() {
        draft.dirty = true;
    }
    draft.kind = ParameterKind::Boolean(flag);
    ui.horizontal(|ui| {
        if ui.button("Apply").clicked() {
            apply_kind_draft(app, block_name, parameter.id, &mut draft);
        }
        if ui.button("Cancel").clicked() {
            draft = KindDraft::from_parameter(parameter);
        }
    });
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.kind_drafts.insert(parameter.id, draft);
    }
}

fn show_text_editor(
    ui: &mut Ui,
    app: &mut MyCadApp,
    block_name: &str,
    parameter: &ParameterDef,
    _text: &TextParameter,
) {
    ui.add_space(4.0);
    ui.label(RichText::new("Text").small().weak());
    let mut draft = app
        .dynamic_authoring
        .as_ref()
        .and_then(|state| state.kind_drafts.get(&parameter.id).cloned())
        .unwrap_or_else(|| KindDraft::from_parameter(parameter));
    ui.label("Name");
    if ui.text_edit_singleline(&mut draft.name).changed() {
        draft.dirty = true;
    }
    ui.label("Description");
    if ui.text_edit_singleline(&mut draft.description).changed() {
        draft.dirty = true;
    }
    let mut text = match &draft.kind {
        ParameterKind::Text(text) => text.clone(),
        _ => return,
    };
    ui.label("Default");
    let default_changed = if text.multiline {
        ui.text_edit_multiline(&mut text.default).changed()
    } else {
        ui.text_edit_singleline(&mut text.default).changed()
    };
    if default_changed {
        draft.dirty = true;
    }
    if ui.checkbox(&mut text.multiline, "Multiline").changed() {
        draft.dirty = true;
    }
    let mut limited = text.max_length.is_some();
    if ui.checkbox(&mut limited, "Limit length").changed() {
        text.max_length = limited.then_some(64);
        draft.dirty = true;
    }
    if let Some(limit) = &mut text.max_length {
        let mut value = *limit as i32;
        if ui.add(egui::DragValue::new(&mut value).range(1..=1000)).changed() {
            *limit = value.max(1) as u32;
            draft.dirty = true;
        }
    }
    draft.kind = ParameterKind::Text(text);
    ui.horizontal(|ui| {
        if ui.button("Apply").clicked() {
            apply_kind_draft(app, block_name, parameter.id, &mut draft);
        }
        if ui.button("Cancel").clicked() {
            draft = KindDraft::from_parameter(parameter);
        }
    });
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.kind_drafts.insert(parameter.id, draft);
    }
}

fn apply_kind_draft(app: &mut MyCadApp, block_name: &str, id: ParameterId, draft: &mut KindDraft) {
    draft.errors.clear();
    if draft.name.trim().is_empty() {
        draft.errors.insert("name".into(), "Name is required".into());
        return;
    }
    if let ParameterKind::Choice(choice) = &draft.kind {
        let mut labels = BTreeMap::new();
        for option in &choice.options {
            if option.label.trim().is_empty() {
                draft.errors.insert("label".into(), "Option labels cannot be empty".into());
                return;
            }
            if labels.insert(option.label.to_ascii_lowercase(), option.id).is_some() {
                draft.errors.insert("label".into(), "Option labels must be unique".into());
                return;
            }
        }
    }
    apply_kind_edit(
        app,
        block_name,
        id,
        draft.name.clone(),
        draft.kind.clone(),
        app.document
            .as_deref()
            .and_then(|document| document.block_by_name(block_name))
            .and_then(|block| block.dynamic.as_ref())
            .and_then(|dynamic| dynamic.parameter(id))
            .map(|parameter| parameter.display_order)
            .unwrap_or(0),
        draft.description.clone(),
    );
    draft.original = draft.kind.clone();
    draft.original_name = draft.name.clone();
    draft.original_description = draft.description.clone();
    draft.dirty = false;
}

fn apply_kind_edit(
    app: &mut MyCadApp,
    block_name: &str,
    id: ParameterId,
    name: String,
    kind: ParameterKind,
    display_order: i32,
    description: String,
) {
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        app.history.begin();
        match replace_parameter_kind(
            document,
            &mut app.history,
            block_name,
            id,
            name,
            kind,
            display_order,
            if description.trim().is_empty() {
                None
            } else {
                Some(description)
            },
        ) {
            Ok(()) => {
                app.history.commit_open();
                app.request_preview();
            }
            Err(err) => {
                app.history.commit_open();
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    authoring.status = Some(err);
                }
            }
        }
    }
}

fn show_parameter_editor(
    ui: &mut Ui,
    app: &mut MyCadApp,
    block_name: &str,
    id: ParameterId,
    name: &str,
    numeric: &NumericParameter,
) {
    ui.add_space(4.0);
    ui.label(RichText::new("Selected parameter").small().weak());
    let mut draft = app
        .dynamic_authoring
        .as_ref()
        .and_then(|state| state.parameter_drafts.get(&id).cloned())
        .unwrap_or_else(|| ParameterDraft::from_parameter(name, numeric));
    ui.vertical(|ui| {
        ui.label("Name");
        if ui.text_edit_singleline(&mut draft.name).changed() {
            draft.dirty = true;
        }
        if let Some(err) = draft.errors.get("name") {
            ui.colored_label(egui::Color32::from_rgb(220, 90, 70), err);
        }
        ui.label("Default");
        if ui.add(egui::DragValue::new(&mut draft.numeric.default).speed(1.0)).changed() {
            draft.dirty = true;
        }
        if let Some(err) = draft.errors.get("default") {
            ui.colored_label(egui::Color32::from_rgb(220, 90, 70), err);
        }
        ui.label(format!(
            "Reference {} (source geometry)",
            format_display_number(draft.numeric.reference, draft.numeric.display_precision)
        ));
        if let Some(size) = &draft.numeric.size {
            ui.label(size.anchor.label());
            ui.label(size.measure.label());
            ui.weak("Picked points are fixed block-local references unless a geometry-bound anchor is set.");
            let mut anchor = size.anchor;
            ui.radio_value(&mut anchor, AnchorPolicy::FirstFixed, "Keep first side fixed");
            ui.radio_value(&mut anchor, AnchorPolicy::SecondFixed, "Keep second side fixed");
            ui.radio_value(&mut anchor, AnchorPolicy::CenterFixed, "Keep center fixed");
            if anchor != size.anchor {
                draft.numeric.size.as_mut().unwrap().anchor = anchor;
                draft.dirty = true;
            }
            if ui.small_button("Move dimension label").clicked() {
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    authoring.pick = AuthoringPick::LabelOffset;
                    authoring.status = Some("Click to place the dimension label".into());
                }
            }
            if ui.small_button("Change reference points").clicked() {
                start_retarget_wizard(app, id, &draft.name, &draft.numeric);
            }
        }
        let list_mode = draft.numeric.domain.is_list();
        let mut list_mode = list_mode;
        if ui.radio_value(&mut list_mode, false, "Any valid size").clicked() && draft.numeric.domain.is_list()
        {
            draft.numeric.domain = NumericDomain::Continuous;
            draft.dirty = true;
        }
        if ui
            .radio_value(&mut list_mode, true, "Choose from available sizes")
            .clicked()
            && !draft.numeric.domain.is_list()
        {
            draft.numeric.domain = NumericDomain::AllowedValues(vec![draft.numeric.default]);
            draft.allowed_paste = format_display_number(draft.numeric.default, draft.numeric.display_precision);
            draft.dirty = true;
        }
        if draft.numeric.domain.is_list() {
            ui.label("Available sizes (one per line or semicolon-separated)");
            if ui.text_edit_multiline(&mut draft.allowed_paste).changed() {
                draft.dirty = true;
            }
            if let Some(err) = draft.errors.get("domain") {
                ui.colored_label(egui::Color32::from_rgb(220, 90, 70), err);
            }
        } else {
            let mut has_min = draft.numeric.min.is_some();
            let mut min = draft.numeric.min.unwrap_or(0.0);
            ui.checkbox(&mut has_min, "Minimum");
            ui.add_enabled(has_min, egui::DragValue::new(&mut min).speed(1.0));
            draft.numeric.min = has_min.then_some(min);
            let mut has_max = draft.numeric.max.is_some();
            let mut max = draft.numeric.max.unwrap_or(0.0);
            ui.checkbox(&mut has_max, "Maximum");
            ui.add_enabled(has_max, egui::DragValue::new(&mut max).speed(1.0));
            draft.numeric.max = has_max.then_some(max);
            let mut has_step = draft.numeric.step.is_some();
            let mut step = draft.numeric.step.unwrap_or(1.0);
            ui.checkbox(&mut has_step, "Step");
            ui.add_enabled(has_step, egui::DragValue::new(&mut step).speed(0.1));
            draft.numeric.step = has_step.then_some(step.max(1e-9));
            let mut required = draft.numeric.step_policy == StepPolicy::RequiredIncrement;
            ui.checkbox(&mut required, "Required increment");
            draft.numeric.step_policy = if required {
                StepPolicy::RequiredIncrement
            } else {
                StepPolicy::IncrementOnly
            };
        }
        ui.horizontal(|ui| {
            if ui.button("Apply parameter").clicked() {
                apply_draft(app, block_name, id, &mut draft);
            }
            if ui.button("Cancel").clicked() {
                draft = ParameterDraft::from_parameter(name, numeric);
            }
        });
    });
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.parameter_drafts.insert(id, draft);
    }
}

fn apply_draft(app: &mut MyCadApp, block_name: &str, id: ParameterId, draft: &mut ParameterDraft) {
    draft.errors.clear();
    if draft.name.trim().is_empty() {
        draft.errors.insert("name".into(), "Name is required".into());
        return;
    }
    if draft.numeric.domain.is_list() {
        match parse_allowed_value_list(&draft.allowed_paste) {
            Ok(values) => draft.numeric.domain = NumericDomain::AllowedValues(values),
            Err(err) => {
                draft.errors.insert("domain".into(), err);
                return;
            }
        }
    }
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        app.history.begin();
        match apply_parameter_settings(
            document,
            &mut app.history,
            block_name,
            id,
            draft.name.clone(),
            draft.numeric.clone(),
        ) {
            Ok(()) => {
                app.history.commit_open();
                draft.original = draft.numeric.clone();
                draft.original_name = draft.name.clone();
                draft.dirty = false;
                app.request_preview();
            }
            Err(err) => {
                app.history.commit_open();
                draft.errors.insert(err.field, err.message);
            }
        }
    }
}

#[derive(Debug)]
struct SettingsError {
    field: String,
    message: String,
}

fn apply_parameter_settings(
    document: &mut Document,
    history: &mut History,
    block_name: &str,
    id: ParameterId,
    name: String,
    numeric: NumericParameter,
) -> Result<(), SettingsError> {
    let Some(block) = document.block_by_name(block_name).cloned() else {
        return Err(SettingsError {
            field: "name".into(),
            message: "Block definition was not found".into(),
        });
    };
    let mut after = block.clone();
    if after
        .dynamic
        .as_ref()
        .is_some_and(|dynamic| dynamic.parameters.iter().any(|item| item.id != id && item.name == name))
    {
        return Err(SettingsError {
            field: "name".into(),
            message: "Another parameter already uses this name".into(),
        });
    }
    let Some(parameter) = after
        .dynamic
        .as_mut()
        .and_then(|dynamic| dynamic.parameters.iter_mut().find(|parameter| parameter.id == id))
    else {
        return Err(SettingsError {
            field: "name".into(),
            message: "Parameter was not found".into(),
        });
    };
    parameter.name = name;
    if let (ParameterKind::Number(previous), Some(size)) = (&parameter.kind, numeric.size.as_ref()) {
        let prev_dir = previous.size.as_ref().map(|item| item.direction);
        let prev_anchor = previous.size.as_ref().map(|item| item.anchor);
        parameter.kind = ParameterKind::Number(numeric.clone());
        if let Some(dynamic) = after.dynamic.as_mut() {
            if prev_dir.is_some_and(|dir| dir != size.direction) {
                apply_size_axis(dynamic, id, size.direction, numeric.reference);
            }
            if prev_anchor.is_some_and(|anchor| anchor != size.anchor) {
                apply_anchor_policy(dynamic, id, size.anchor);
            }
        }
    } else {
        parameter.kind = ParameterKind::Number(numeric.clone());
    }
    if let Some(dynamic) = after.dynamic.as_ref() {
        if let Err(err) = cad_core::validate_definition(dynamic, &after.entities) {
            return Err(settings_error_from_dynamic(err));
        }
    }
    if let NumericDomain::AllowedValues(values) = &numeric.domain {
        let affected = affected_instance_values(document, block_name, id, values);
        if !affected.is_empty() {
            return Err(SettingsError {
                field: "domain".into(),
                message: format!(
                    "These instance values are not in the new list: {}. Map or cancel.",
                    affected
                        .iter()
                        .map(|value| format_display_number(*value, numeric.display_precision))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }
    }
    document.replace_block_definition(after.clone());
    history.record(Edit::ReplaceBlockDefinition {
        name: block_name.to_string(),
        before: Some(block),
        after: Some(after),
    });
    Ok(())
}

fn settings_error_from_dynamic(err: DynamicError) -> SettingsError {
    let field = match err {
        DynamicError::DefaultOutOfDomain { .. } | DynamicError::ValueNotInList { .. } => "default",
        DynamicError::Range { .. } | DynamicError::OffStep { .. } => "default",
        _ => "name",
    };
    SettingsError {
        field: field.into(),
        message: err.to_string(),
    }
}

fn affected_instance_values(
    document: &Document,
    block_name: &str,
    parameter: ParameterId,
    allowed: &[f64],
) -> Vec<f64> {
    let mut affected = Vec::new();
    for entity in document.model_space.iter().chain(
        document
            .blocks
            .values()
            .flat_map(|block| block.entities.iter()),
    ) {
        if entity.geometry.insert_block_name() != Some(block_name) {
            continue;
        }
        if let Some(value) = entity
            .geometry
            .insert_configuration()
            .and_then(|config| config.get(parameter))
            .and_then(ParameterValue::as_number)
        {
            if !allowed.iter().any(|item| cad_core::numbers_equal(*item, value))
                && !affected.iter().any(|item| cad_core::numbers_equal(*item, value))
            {
                affected.push(value);
            }
        }
    }
    affected
}

fn show_behavior_list(ui: &mut Ui, app: &mut MyCadApp, dynamic: &DynamicDefinition, parameter: ParameterId) {
    ui.add_space(6.0);
    ui.label(RichText::new("Behaviors").small().weak());
    let mut remove = None;
    let mut select = None;
    for behavior in dynamic.behaviors.iter().filter(|behavior| behavior.parameter == parameter) {
        let selected = app
            .dynamic_authoring
            .as_ref()
            .is_some_and(|state| state.selected_behavior == Some(behavior.id));
        ui.vertical(|ui| {
            if ui.selectable_label(selected, behavior.describe(dynamic)).clicked() {
                select = Some(behavior.id);
            }
            ui.horizontal(|ui| {
                if ui.small_button("Follow first side").clicked() {
                    set_behavior_follow(app, behavior.id, FollowRole::First);
                }
                if ui.small_button("Follow second side").clicked() {
                    set_behavior_follow(app, behavior.id, FollowRole::Second);
                }
                if ui.small_button("Follow center").clicked() {
                    set_behavior_follow(app, behavior.id, FollowRole::Center);
                }
                if ui.small_button("Edit targets").clicked() {
                    if let Some(authoring) = app.dynamic_authoring.as_mut() {
                        authoring.attach = Some(AttachSession {
                            kind: behavior.kind,
                            parameter: behavior.parameter,
                            follow: behavior.follow,
                            handles: behavior.targets.clone(),
                            unsupported: Vec::new(),
                            window: None,
                            replace: Some(behavior.id),
                        });
                        authoring.selected_behavior = Some(behavior.id);
                        authoring.highlight = behavior.targets.clone();
                        authoring.status = Some("Correct targets, then confirm to update this behavior.".into());
                    }
                }
                if ui.small_button("Remove").clicked() {
                    remove = Some(behavior.id);
                }
            });
            ui.collapsing("Advanced", |ui| {
                ui.label(format!("multiplier × {}", behavior.multiplier));
            });
        });
    }
    if let Some(id) = select {
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.selected_behavior = Some(id);
            if let Some(behavior) = dynamic.behaviors.iter().find(|item| item.id == id) {
                authoring.highlight = behavior.targets.clone();
            }
        }
    }
    if let Some(id) = remove {
        let name = app
            .dynamic_authoring
            .as_ref()
            .map(|state| state.block_name.clone())
            .unwrap_or_default();
        if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
            app.history.begin();
            remove_behavior(document, &mut app.history, &name, id);
            app.history.commit_open();
            app.request_preview();
        }
    }
}

fn set_behavior_follow(app: &mut MyCadApp, id: ActionId, follow: FollowRole) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) else {
        return;
    };
    let Some(block) = document.block_by_name(&state.block_name).cloned() else {
        return;
    };
    let mut after = block.clone();
    let Some(dynamic) = after.dynamic.as_mut() else {
        return;
    };
    let Some(parameter) = dynamic.behaviors.iter().find(|item| item.id == id).map(|item| item.parameter) else {
        return;
    };
    let anchor = dynamic
        .parameter(parameter)
        .and_then(|item| match &item.kind {
            ParameterKind::Number(numeric) => numeric.size.as_ref().map(|size| size.anchor),
            _ => None,
        })
        .unwrap_or(AnchorPolicy::FirstFixed);
    let Some(behavior) = dynamic.behaviors.iter_mut().find(|item| item.id == id) else {
        return;
    };
    behavior.follow = follow;
    behavior.multiplier = follow_multiplier(anchor, follow);
    if cad_core::validate_definition(dynamic, &after.entities).is_err() {
        return;
    }
    app.history.begin();
    document.replace_block_definition(after.clone());
    app.history.record(Edit::ReplaceBlockDefinition {
        name: state.block_name,
        before: Some(block),
        after: Some(after),
    });
    app.history.commit_open();
    app.request_preview();
}

fn show_attach_controls(ui: &mut Ui, app: &mut MyCadApp) {
    ui.add_space(8.0);
    ui.label(RichText::new("Attach").small().weak());
    ui.label("Select geometry, then attach Move or Stretch. Stretch uses handles or a window.");
    ui.vertical(|ui| {
        if ui.button("Attach Move — follow first side").clicked() {
            attach_move_from_selection(app, FollowRole::First);
        }
        if ui.button("Attach Move — follow second side").clicked() {
            attach_move_from_selection(app, FollowRole::Second);
        }
        if ui.button("Attach Move — follow center").clicked() {
            attach_move_from_selection(app, FollowRole::Center);
        }
        if ui.button("Select stretch handles").clicked() {
            begin_stretch_handles(app);
        }
        if ui.button("Stretch window").clicked() {
            if let Some(authoring) = app.dynamic_authoring.as_mut() {
                authoring.pick = AuthoringPick::StretchWindowA;
                authoring.status = Some("Draw a stretch window first corner".into());
            }
        }
        if app.dynamic_authoring.as_ref().and_then(|state| state.attach.as_ref()).is_some() {
            let mut follow = app
                .dynamic_authoring
                .as_ref()
                .and_then(|state| state.attach.as_ref().map(|attach| attach.follow))
                .unwrap_or(FollowRole::First);
            let handle_count = app
                .dynamic_authoring
                .as_ref()
                .and_then(|state| state.attach.as_ref().map(|attach| attach.handles.len()))
                .unwrap_or(0);
            ui.label(format!("{handle_count} targets selected"));
            if let Some(unsupported) = app
                .dynamic_authoring
                .as_ref()
                .and_then(|state| state.attach.as_ref())
                .map(|attach| attach.unsupported.clone())
                .filter(|items| !items.is_empty())
            {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 90, 70),
                    format!("Unsupported: {}", unsupported.join("; ")),
                );
                if ui.button("Remove unsupported targets").clicked() {
                    if let Some(attach) = app.dynamic_authoring.as_mut().and_then(|state| state.attach.as_mut())
                    {
                        attach.unsupported.clear();
                    }
                }
            }
            ui.radio_value(&mut follow, FollowRole::First, "Follow first side");
            ui.radio_value(&mut follow, FollowRole::Second, "Follow second side");
            ui.radio_value(&mut follow, FollowRole::Center, "Follow center");
            if let Some(attach) = app.dynamic_authoring.as_mut().and_then(|state| state.attach.as_mut()) {
                attach.follow = follow;
            }
            if ui.button("Confirm attachment").clicked() {
                if let Some(attach) = app.dynamic_authoring.as_ref().and_then(|state| state.attach.clone()) {
                    if !attach.unsupported.is_empty() {
                        if let Some(authoring) = app.dynamic_authoring.as_mut() {
                            authoring.status = Some(
                                "Remove unsupported targets or cancel. Partial commit is not allowed.".into(),
                            );
                        }
                    } else {
                        if let Some(authoring) = app.dynamic_authoring.as_mut() {
                            authoring.selected_parameter = Some(attach.parameter);
                        }
                        commit_follow_targets(app, attach.kind, attach.handles, attach.follow);
                    }
                }
            }
            if ui.button("Cancel attachment").clicked() {
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    authoring.attach = None;
                    authoring.highlight.clear();
                }
            }
        }
    });
}

fn show_test_controls(ui: &mut Ui, app: &mut MyCadApp, dynamic: &DynamicDefinition) {
    ui.add_space(8.0);
    ui.label(RichText::new("Test configuration").small().weak());
    ui.label("Source geometry stays at definition defaults. These values preview together.");
    let current = app
        .dynamic_authoring
        .as_ref()
        .map(|state| state.test_values.clone())
        .unwrap_or_default();
    let mut drafts = app
        .dynamic_authoring
        .as_ref()
        .map(|state| state.test_drafts.clone())
        .unwrap_or_default();
    let mut errors = app
        .dynamic_authoring
        .as_ref()
        .map(|state| state.test_errors.clone())
        .unwrap_or_default();
    let mut pending = app
        .dynamic_authoring
        .as_ref()
        .and_then(|state| state.test_pending.clone());
    let mixed = std::collections::BTreeSet::new();
    let action = crate::config_form::show_configuration_form(
        ui,
        "test-config",
        dynamic,
        &current,
        &mixed,
        &mut drafts,
        &mut errors,
        &mut pending,
    );
    let mut request_preview = false;
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.test_drafts = drafts;
        authoring.test_errors = errors;
        authoring.test_pending = pending;
        match action {
            crate::config_form::ConfigFormAction::None => {}
            crate::config_form::ConfigFormAction::Values(values) => {
                authoring.test_values = values;
                authoring.mark_preview_dirty();
                request_preview = true;
            }
            crate::config_form::ConfigFormAction::Reset(id) => {
                if let Some(parameter) = dynamic.parameter(id) {
                    authoring.test_values.insert(id, parameter.default_value());
                    authoring.mark_preview_dirty();
                    request_preview = true;
                }
            }
            crate::config_form::ConfigFormAction::ResetAll => {
                for parameter in &dynamic.parameters {
                    authoring
                        .test_values
                        .insert(parameter.id, parameter.default_value());
                }
                authoring.mark_preview_dirty();
                request_preview = true;
            }
            crate::config_form::ConfigFormAction::ApplyPreset(id) => {
                if let Some(preset) = dynamic.preset(id) {
                    authoring.test_values = preset.values.clone();
                    authoring.mark_preview_dirty();
                    request_preview = true;
                }
            }
        }
    }
    if request_preview {
        app.request_preview();
    }
    ui.horizontal(|ui| {
        if ui.button("Return to initial test").clicked() {
            if let Some(authoring) = app.dynamic_authoring.as_mut() {
                authoring.test_values = authoring.test_initial.clone();
                authoring.mark_preview_dirty();
            }
            app.request_preview();
        }
        let mut compare = app
            .dynamic_authoring
            .as_ref()
            .is_some_and(|state| state.test_compare);
        if ui.checkbox(&mut compare, "Compare with reference geometry").changed() {
            if let Some(authoring) = app.dynamic_authoring.as_mut() {
                authoring.test_compare = compare;
            }
        }
        let mut show_inactive = app
            .dynamic_authoring
            .as_ref()
            .is_some_and(|state| state.show_inactive);
        if ui.checkbox(&mut show_inactive, "Show inactive parts").changed() {
            if let Some(authoring) = app.dynamic_authoring.as_mut() {
                authoring.show_inactive = show_inactive;
            }
        }
        if ui.button("Use these values as defaults").clicked() {
            apply_test_values_as_defaults(app, dynamic);
        }
        if ui.button("Save current configuration as preset…").clicked() {
            save_preset_from_test(app);
        }
    });
}

fn attach_move_from_selection(app: &mut MyCadApp, follow: FollowRole) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    let Some(parameter) = state.selected_parameter else {
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.status = Some("Select a parameter first".into());
        }
        return;
    };
    let ids = app.selection.ids().to_vec();
    let mut targets = Vec::new();
    let mut unsupported = Vec::new();
    if let Some(document) = app.document.as_deref() {
        if let Some(block) = document.block_by_name(&state.block_name) {
            for id in ids {
                let Some(entity) = block.entities.iter().find(|entity| entity.id == id) else {
                    continue;
                };
                match cad_core::capability_for(BehaviorKind::Move, &entity.geometry, GeometryTarget::Entity(id)) {
                    Ok(()) => targets.push(GeometryTarget::Entity(id)),
                    Err(reason) => unsupported.push(format!("{}: cannot {reason}", entity.geometry.type_name())),
                }
            }
        }
    }
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.attach = Some(AttachSession {
            kind: BehaviorKind::Move,
            parameter,
            follow,
            handles: targets.clone(),
            unsupported: unsupported.clone(),
            window: None,
            replace: None,
        });
        authoring.highlight = targets;
        if unsupported.is_empty() {
            authoring.status = Some("Choose the followed side, then confirm attachment.".into());
        } else {
            authoring.status = Some(format!(
                "Unsupported targets: {}. Remove them or cancel.",
                unsupported.join("; ")
            ));
        }
    }
}

fn begin_stretch_handles(app: &mut MyCadApp) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    let Some(parameter) = state.selected_parameter else {
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.status = Some("Select a parameter first".into());
        }
        return;
    };
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.attach = Some(AttachSession {
            kind: BehaviorKind::Stretch,
            parameter,
            follow: FollowRole::Second,
            handles: Vec::new(),
            unsupported: Vec::new(),
            window: None,
            replace: None,
        });
        authoring.status = Some("Click handles to add (Shift) or remove (Ctrl), then confirm.".into());
    }
}

pub fn attach_from_selection(app: &mut MyCadApp, kind: BehaviorKind, multiplier: f64) {
    match kind {
        BehaviorKind::Move => attach_move_from_selection(app, follow_from_multiplier(multiplier)),
        BehaviorKind::Stretch => begin_stretch_handles(app),
    }
}

pub fn attach_stretch_from_click(app: &mut MyCadApp, _world: Option<Point2>) {
    begin_stretch_handles(app);
}

fn toggle_nearest_handle(
    app: &mut MyCadApp,
    local: Point2,
    screen: Point2,
    origin: Point2,
    size: Point2,
    add: bool,
    remove: bool,
) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    if state.attach.is_none() {
        return;
    }
    let Some(document) = app.document.as_deref() else {
        return;
    };
    let Some(block) = document.block_by_name(&state.block_name) else {
        return;
    };
    let mut best: Option<(f64, GeometryTarget, String)> = None;
    let ids = if app.selection.is_empty() {
        block.entities.iter().map(|entity| entity.id).collect()
    } else {
        app.selection.ids().to_vec()
    };
    for id in ids {
        let Some(entity) = block.entities.iter().find(|entity| entity.id == id) else {
            continue;
        };
        for (point, target) in stretch_handles(entity) {
            let world = app.block_edit.world_from_local().apply(point);
            let handle_screen = app.camera.world_to_screen(world, origin, size);
            let distance = ((handle_screen.x - screen.x).powi(2) + (handle_screen.y - screen.y).powi(2)).sqrt();
            if best
                .as_ref()
                .is_none_or(|(best_distance, _, _)| distance < *best_distance)
            {
                best = Some((distance, target, entity.geometry.type_name().to_string()));
            }
        }
    }
    let Some((distance, target, owner)) = best else {
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.status = Some("No stretch handle near the click".into());
        }
        return;
    };
    if distance > HANDLE_PICK_PX {
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.status = Some("No stretch handle within picking tolerance".into());
        }
        return;
    }
    let _ = local;
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        let session = authoring.attach.get_or_insert_with(|| AttachSession {
            kind: BehaviorKind::Stretch,
            parameter: authoring.selected_parameter.unwrap_or(ParameterId(0)),
            follow: FollowRole::Second,
            handles: Vec::new(),
            unsupported: Vec::new(),
            window: None,
            replace: None,
        });
        if remove || (session.handles.contains(&target) && !add) {
            session.handles.retain(|item| *item != target);
        } else if !session.handles.contains(&target) {
            session.handles.push(target);
        }
        authoring.highlight = session.handles.clone();
        authoring.status = Some(format!(
            "{} — {} handles selected — confirm in the panel",
            owner,
            session.handles.len()
        ));
    }
}

fn stretch_handles(entity: &Entity) -> Vec<(Point2, GeometryTarget)> {
    match &entity.geometry {
        Geometry::Line { start, end } => vec![
            (Point2::new(start.x, start.y), GeometryTarget::LineStart(entity.id)),
            (Point2::new(end.x, end.y), GeometryTarget::LineEnd(entity.id)),
        ],
        Geometry::LwPolyline { vertices, .. } | Geometry::Polyline { vertices, .. } => {
            if entity.geometry.polyline_has_curves() {
                Vec::new()
            } else {
                vertices
                    .iter()
                    .filter(|vertex| vertex.vertex_id.is_assigned())
                    .map(|vertex| {
                        (
                            Point2::new(vertex.point.x, vertex.point.y),
                            GeometryTarget::Vertex {
                                entity: entity.id,
                                vertex: vertex.vertex_id,
                            },
                        )
                    })
                    .collect()
            }
        }
        _ => Vec::new(),
    }
}

fn commit_follow_targets(
    app: &mut MyCadApp,
    kind: BehaviorKind,
    targets: Vec<GeometryTarget>,
    follow: FollowRole,
) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    let Some(parameter) = state.selected_parameter else {
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.status = Some("Select a parameter first".into());
        }
        return;
    };
    let targets = dedupe_targets(targets);
    if targets.is_empty() {
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.status = Some("Select geometry that belongs to this definition".into());
        }
        return;
    }
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        if let Some(id) = state.attach.as_ref().and_then(|attach| attach.replace) {
            app.history.begin();
            match replace_behavior_targets(
                document,
                &mut app.history,
                &state.block_name,
                id,
                targets.clone(),
                follow,
            ) {
                Ok(()) => {
                    app.history.commit_open();
                    if let Some(authoring) = app.dynamic_authoring.as_mut() {
                        authoring.highlight = targets;
                        authoring.attach = None;
                        authoring.status = Some("Behavior targets updated".into());
                    }
                    app.request_preview();
                }
                Err(err) => {
                    app.history.commit_open();
                    if let Some(authoring) = app.dynamic_authoring.as_mut() {
                        authoring.status = Some(err);
                    }
                }
            }
            return;
        }
        app.history.begin();
        match attach_behavior_with_follow(
            document,
            &mut app.history,
            &state.block_name,
            parameter,
            kind,
            targets.clone(),
            follow,
        ) {
            Ok(()) => {
                app.history.commit_open();
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    authoring.highlight = targets;
                    authoring.attach = None;
                    authoring.status = Some("Behavior attached".into());
                }
                app.request_preview();
            }
            Err(err) => {
                app.history.commit_open();
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    if err.contains("already attached") {
                        if let Some(document) = app.document.as_deref() {
                            if let Some(dynamic) = document
                                .block_by_name(&state.block_name)
                                .and_then(|block| block.dynamic.as_ref())
                            {
                                if let Some(existing) = dynamic.behaviors.iter().find(|behavior| {
                                    behavior.parameter == parameter
                                        && behavior.kind == kind
                                        && behavior.targets.iter().any(|target| targets.contains(target))
                                }) {
                                    authoring.selected_behavior = Some(existing.id);
                                    authoring.highlight = existing.targets.clone();
                                }
                            }
                        }
                    }
                    authoring.status = Some(err);
                }
            }
        }
    }
}

pub fn handle_authoring_pick(
    app: &mut MyCadApp,
    local: Point2,
    screen: Point2,
    origin: Point2,
    size: Point2,
    add: bool,
    remove: bool,
) {
    let pick = app
        .dynamic_authoring
        .as_ref()
        .map(|state| state.pick)
        .unwrap_or(AuthoringPick::Idle);
    match pick {
        AuthoringPick::SizePointA => {
            if let Some(authoring) = app.dynamic_authoring.as_mut() {
                if let Some(wizard) = authoring.size_wizard.as_mut() {
                    wizard.point_a = Some(local);
                    wizard.stage = SizeWizardStage::PickB;
                }
                authoring.pick = AuthoringPick::SizePointB;
                authoring.status = Some("Specify second reference point".into());
            }
        }
        AuthoringPick::SizePointB => {
            let mut error = None;
            if let Some(authoring) = app.dynamic_authoring.as_mut() {
                if let Some(wizard) = authoring.size_wizard.as_mut() {
                    wizard.point_b = Some(local);
                    if wizard.measured().is_none() {
                        error = Some("Points coincide or projected distance is zero".into());
                        wizard.point_b = None;
                    } else {
                        wizard.stage = SizeWizardStage::Measure;
                    }
                }
                if error.is_none() {
                    authoring.pick = AuthoringPick::Idle;
                    authoring.status = Some("Choose the measurement direction".into());
                } else {
                    authoring.status = error;
                }
            }
        }
        AuthoringPick::StretchWindowA => {
            if let Some(authoring) = app.dynamic_authoring.as_mut() {
                authoring.direction_start = Some(local);
                authoring.pick = AuthoringPick::StretchWindowB;
                authoring.status = Some("Specify opposite window corner".into());
            }
        }
        AuthoringPick::StretchWindowB => {
            apply_stretch_window(app, local);
        }
        AuthoringPick::LabelOffset => {
            apply_label_offset(app, local);
        }
        AuthoringPick::MirrorAxisA => {
            if let Some(authoring) = app.dynamic_authoring.as_mut() {
                if let Some(BehaviorWizard::Mirror { axis_a, .. }) = authoring.wizard.as_mut() {
                    *axis_a = Some(local);
                }
                authoring.pick = AuthoringPick::MirrorAxisB;
                authoring.status = Some("Specify second axis point".into());
            }
        }
        AuthoringPick::MirrorAxisB => {
            if let Some(authoring) = app.dynamic_authoring.as_mut() {
                if let Some(BehaviorWizard::Mirror { axis_b, .. }) = authoring.wizard.as_mut() {
                    *axis_b = Some(local);
                }
                authoring.pick = AuthoringPick::Idle;
                authoring.status = Some("Preview both states, then Confirm".into());
            }
        }
        AuthoringPick::PlaceAttachment => {
            if let Some(authoring) = app.dynamic_authoring.as_mut() {
                if let Some(BehaviorWizard::Position { attachment, .. }) = authoring.wizard.as_mut() {
                    *attachment = Some(local);
                }
                authoring.pick = AuthoringPick::Idle;
                authoring.status = Some("Map destinations, then Confirm".into());
            }
        }
        AuthoringPick::RotatePivot => {
            if let Some(authoring) = app.dynamic_authoring.as_mut() {
                if let Some(BehaviorWizard::Rotate { pivot, .. }) = authoring.wizard.as_mut() {
                    *pivot = Some(local);
                }
                authoring.pick = AuthoringPick::Idle;
                authoring.status = Some("Confirm rotation".into());
            }
        }
        AuthoringPick::Idle => {
            toggle_nearest_handle(app, local, screen, origin, size, add, remove);
        }
    }
}

pub fn update_authoring_hover(app: &mut MyCadApp, local: Point2) {
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        if let Some(wizard) = authoring.size_wizard.as_mut() {
            if matches!(authoring.pick, AuthoringPick::SizePointA | AuthoringPick::SizePointB) {
                wizard.hover = Some(local);
            }
        }
    }
}

fn apply_label_offset(app: &mut MyCadApp, local: Point2) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    let Some(parameter_id) = state.selected_parameter else {
        return;
    };
    let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) else {
        return;
    };
    let Some(block) = document.block_by_name(&state.block_name).cloned() else {
        return;
    };
    let mut after = block.clone();
    let Some(parameter) = after
        .dynamic
        .as_mut()
        .and_then(|dynamic| dynamic.parameters.iter_mut().find(|item| item.id == parameter_id))
    else {
        return;
    };
    if let ParameterKind::Number(numeric) = &mut parameter.kind {
        if let Some(size) = numeric.size.as_mut() {
            let mid = Point2::new(
                (size.point_a.x + size.point_b.x) * 0.5,
                (size.point_a.y + size.point_b.y) * 0.5,
            );
            size.label_offset = Point2::new(local.x - mid.x, local.y - mid.y);
        }
    }
    app.history.begin();
    document.replace_block_definition(after.clone());
    app.history.record(Edit::ReplaceBlockDefinition {
        name: state.block_name,
        before: Some(block),
        after: Some(after),
    });
    app.history.commit_open();
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.pick = AuthoringPick::Idle;
        authoring.status = Some("Dimension label moved".into());
    }
}

fn apply_stretch_window(app: &mut MyCadApp, local: Point2) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    let Some(start) = state.direction_start else {
        return;
    };
    let min_x = start.x.min(local.x);
    let max_x = start.x.max(local.x);
    let min_y = start.y.min(local.y);
    let max_y = start.y.max(local.y);
    let Some(document) = app.document.as_deref() else {
        return;
    };
    let Some(block) = document.block_by_name(&state.block_name) else {
        return;
    };
    let mut proposed = Vec::new();
    for entity in &block.entities {
        for (point, target) in stretch_handles(entity) {
            if point.x >= min_x && point.x <= max_x && point.y >= min_y && point.y <= max_y {
                proposed.push(target);
            }
        }
    }
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        let parameter = authoring.selected_parameter.unwrap_or(ParameterId(0));
        authoring.attach = Some(AttachSession {
            kind: BehaviorKind::Stretch,
            parameter,
            follow: FollowRole::Second,
            handles: dedupe_targets(proposed.clone()),
            unsupported: Vec::new(),
            window: Some((start, local)),
            replace: None,
        });
        authoring.highlight = proposed;
        authoring.pick = AuthoringPick::Idle;
        authoring.direction_start = None;
        authoring.status = Some("Window proposed handles. Correct individually, then confirm.".into());
    }
}

pub fn cancel_authoring_pick(app: &mut MyCadApp) -> bool {
    let Some(authoring) = app.dynamic_authoring.as_mut() else {
        return false;
    };
    if authoring.pick == AuthoringPick::Idle && authoring.size_wizard.is_none() {
        if authoring.attach.take().is_some() {
            authoring.highlight.clear();
            authoring.status = Some("Attachment cancelled".into());
            return true;
        }
        return false;
    }
    match authoring.pick {
        AuthoringPick::SizePointB => {
            authoring.pick = AuthoringPick::SizePointA;
            if let Some(wizard) = authoring.size_wizard.as_mut() {
                wizard.point_b = None;
                wizard.stage = SizeWizardStage::PickA;
            }
            authoring.status = Some("Specify first reference point".into());
        }
        AuthoringPick::SizePointA => {
            authoring.pick = AuthoringPick::Idle;
            authoring.size_wizard = None;
            authoring.status = Some("Pick cancelled".into());
        }
        AuthoringPick::StretchWindowA
        | AuthoringPick::StretchWindowB
        | AuthoringPick::LabelOffset
        | AuthoringPick::MirrorAxisA
        | AuthoringPick::MirrorAxisB
        | AuthoringPick::PlaceAttachment
        | AuthoringPick::RotatePivot => {
            authoring.pick = AuthoringPick::Idle;
            authoring.direction_start = None;
            authoring.status = Some("Pick cancelled".into());
        }
        AuthoringPick::Idle => {
            authoring.size_wizard = None;
            authoring.status = Some("Size creation cancelled".into());
        }
    }
    true
}

fn replace_behavior_targets(
    document: &mut Document,
    history: &mut History,
    block_name: &str,
    id: ActionId,
    targets: Vec<GeometryTarget>,
    follow: FollowRole,
) -> Result<(), String> {
    let Some(block) = document.block_by_name(block_name).cloned() else {
        return Err("Block definition was not found".into());
    };
    let mut after = block.clone();
    let Some(dynamic) = after.dynamic.as_mut() else {
        return Err("Block is not dynamic".into());
    };
    let parameter = dynamic
        .behaviors
        .iter()
        .find(|item| item.id == id)
        .map(|item| item.parameter);
    let Some(parameter) = parameter else {
        return Err("Behavior was not found".into());
    };
    let anchor = dynamic
        .parameter(parameter)
        .and_then(|item| match &item.kind {
            ParameterKind::Number(numeric) => numeric.size.as_ref().map(|size| size.anchor),
            _ => None,
        })
        .unwrap_or(AnchorPolicy::FirstFixed);
    let Some(behavior) = dynamic.behaviors.iter_mut().find(|item| item.id == id) else {
        return Err("Behavior was not found".into());
    };
    behavior.targets = dedupe_targets(targets);
    behavior.follow = follow;
    if follow != FollowRole::Custom {
        behavior.multiplier = follow_multiplier(anchor, follow);
    }
    cad_core::validate_definition(dynamic, &after.entities).map_err(|err| err.to_string())?;
    document.replace_block_definition(after.clone());
    history.record(Edit::ReplaceBlockDefinition {
        name: block_name.to_string(),
        before: Some(block),
        after: Some(after),
    });
    Ok(())
}

fn apply_test_values_as_defaults(app: &mut MyCadApp, _dynamic: &DynamicDefinition) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) else {
        return;
    };
    let Some(block) = document.block_by_name(&state.block_name).cloned() else {
        return;
    };
    let affected = count_block_references(document, &state.block_name);
    let mut after = block.clone();
    let Some(after_dynamic) = after.dynamic.as_mut() else {
        return;
    };
    for parameter in &mut after_dynamic.parameters {
        if let Some(value) = state.test_values.get(&parameter.id) {
            match (&mut parameter.kind, value) {
                (ParameterKind::Number(numeric), ParameterValue::Number(number)) => {
                    numeric.default = *number;
                }
                (ParameterKind::Choice(choice), ParameterValue::Choice(option)) => {
                    choice.default = *option;
                }
                (ParameterKind::Boolean(flag), ParameterValue::Boolean(on)) => {
                    flag.default = *on;
                }
                (ParameterKind::Text(text), ParameterValue::Text(content)) => {
                    text.default = content.clone();
                }
                _ => {}
            }
        }
    }
    if let Err(err) = cad_core::validate_definition(after_dynamic, &after.entities) {
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.status = Some(err.to_string());
        }
        return;
    }
    app.history.begin();
    document.replace_block_definition(after.clone());
    app.history.record(Edit::ReplaceBlockDefinition {
        name: state.block_name,
        before: Some(block),
        after: Some(after),
    });
    app.history.commit_open();
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.status = Some(format!(
            "Defaults updated. {affected} references use this definition; stored instance values are unchanged."
        ));
    }
    app.request_preview();
}

fn show_repair_bindings(ui: &mut Ui, app: &mut MyCadApp, dynamic: &DynamicDefinition, entities: &[Entity]) {
    let broken = collect_broken_bindings(dynamic, entities);
    if broken.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.colored_label(
        egui::Color32::from_rgb(220, 90, 70),
        RichText::new("Broken bindings — repair or remove").strong(),
    );
    for (action, target, reason) in &broken {
        ui.label(format!("{reason}: {target:?}"));
        if action.is_assigned() {
            ui.push_id(action.raw(), |ui| {
                if ui.small_button("Remove behavior").clicked() {
                    let name = app
                        .dynamic_authoring
                        .as_ref()
                        .map(|state| state.block_name.clone())
                        .unwrap_or_default();
                    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
                        app.history.begin();
                        remove_behavior(document, &mut app.history, &name, *action);
                        app.history.commit_open();
                        app.request_preview();
                    }
                }
            });
        }
    }
}

fn show_selection_bindings(ui: &mut Ui, app: &mut MyCadApp, dynamic: &DynamicDefinition) {
    let ids = app.selection.ids().to_vec();
    if ids.is_empty() {
        return;
    }
    let mut hits = Vec::new();
    for behavior in &dynamic.behaviors {
        if behavior
            .targets
            .iter()
            .any(|target| ids.contains(&target.entity_id()))
        {
            hits.push(behavior.describe(dynamic));
        }
    }
    if hits.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.label(RichText::new("Affects selected geometry").small().weak());
    for hit in hits {
        ui.label(hit);
    }
}


fn show_behavior_wizard(ui: &mut Ui, app: &mut MyCadApp, dynamic: &DynamicDefinition) {
    let Some(mut wizard) = app.dynamic_authoring.as_ref().and_then(|state| state.wizard.clone()) else {
        return;
    };
    ui.add_space(8.0);
    ui.label(RichText::new("Behavior").small().weak());
    let mut confirm = false;
    let mut cancel = false;
    match &mut wizard {
        BehaviorWizard::LinkText {
            target,
            parameter,
            mode,
            option_texts,
            formatted,
            boolean_true,
            boolean_false,
        } => {
            ui.label("Link Text to Parameter");
            ui.label(format!("Target entity #{}", target.raw()));
            egui::ComboBox::from_id_salt("link-text-param")
                .selected_text(
                    dynamic
                        .parameter(*parameter)
                        .map(|item| item.name.clone())
                        .unwrap_or_else(|| "Parameter".into()),
                )
                .show_ui(ui, |ui| {
                    for item in dynamic.sorted_parameters() {
                        if ui.selectable_label(*parameter == item.id, &item.name).clicked() {
                            *parameter = item.id;
                        }
                    }
                });
            ui.radio_value(mode, 0usize, "Show parameter value");
            ui.radio_value(mode, 1usize, "Text for each option");
            ui.radio_value(mode, 2usize, "Formatted text");
            if *mode == 1 {
                if let Some(ParameterKind::Choice(choice)) = dynamic.parameter(*parameter).map(|item| &item.kind) {
                    for option in &choice.options {
                        let text = option_texts.entry(option.id).or_insert_with(|| option.label.clone());
                        ui.horizontal(|ui| {
                            ui.label(&option.label);
                            ui.text_edit_singleline(text);
                        });
                    }
                }
            }
            if *mode == 2 {
                ui.text_edit_singleline(formatted);
                ui.weak("Insert a parameter:");
                for item in dynamic.sorted_parameters() {
                    if ui.small_button(&item.name).clicked() {
                        formatted.push_str(&format!("{{{}}}", item.name));
                    }
                }
                ui.weak("Tokens are stored by parameter ID after confirm.");
            }
            if matches!(dynamic.parameter(*parameter).map(|item| &item.kind), Some(ParameterKind::Boolean(_))) {
                ui.label("On label");
                ui.text_edit_singleline(boolean_true);
                ui.label("Off label");
                ui.text_edit_singleline(boolean_false);
            }
            let preview = preview_text_binding(dynamic, &wizard);
            ui.label(format!("Preview: {preview}"));
        }
        BehaviorWizard::ShowWhen {
            members,
            parameter,
            options,
            boolean,
        } => {
            ui.label(format!("Show When — {} objects", members.len()));
            parameter_picker(ui, dynamic, parameter);
            match dynamic.parameter(*parameter).map(|item| &item.kind) {
                Some(ParameterKind::Choice(choice)) => {
                    for option in &choice.options {
                        let mut on = options.contains(&option.id);
                        if ui.checkbox(&mut on, &option.label).changed() {
                            if on {
                                if !options.contains(&option.id) {
                                    options.push(option.id);
                                }
                            } else {
                                options.retain(|id| *id != option.id);
                            }
                        }
                    }
                }
                Some(ParameterKind::Boolean(flag)) => {
                    ui.checkbox(boolean, &format!("Required {}", flag.true_label));
                }
                _ => {
                    ui.weak("Select a Choice or On/Off parameter.");
                }
            }
        }
        BehaviorWizard::Mirror {
            members,
            axis_a,
            axis_b,
            parameter,
            use_visibility,
            policy,
        } => {
            ui.label(format!("Mirror for Option — {} objects", members.len()));
            parameter_picker(ui, dynamic, parameter);
            ui.label(format!(
                "Axis: {} → {}",
                axis_a.map(|p| format!("{:.2},{:.2}", p.x, p.y)).unwrap_or_else(|| "pick first".into()),
                axis_b.map(|p| format!("{:.2},{:.2}", p.x, p.y)).unwrap_or_else(|| "pick second".into())
            ));
            if ui.button("Pick mirror axis").clicked() {
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    authoring.pick = AuthoringPick::MirrorAxisA;
                    authoring.status = Some("Specify first axis point".into());
                }
            }
            ui.radio_value(use_visibility, false, "Mirror these parts");
            ui.radio_value(use_visibility, true, "Use different parts");
            ui.radio_value(policy, cad_core::TextReflectPolicy::KeepReadable, "Keep text readable");
            ui.radio_value(policy, cad_core::TextReflectPolicy::KeepUpright, "Keep upright");
            ui.radio_value(policy, cad_core::TextReflectPolicy::Mirror, "Mirror glyphs");
        }
        BehaviorWizard::Position {
            members,
            attachment,
            parameter,
        } => {
            ui.label(format!("Position by Option — {} objects", members.len()));
            parameter_picker(ui, dynamic, parameter);
            ui.label(format!(
                "Attachment: {}",
                attachment.map(|p| format!("{:.2},{:.2}", p.x, p.y)).unwrap_or_else(|| "pick point".into())
            ));
            if ui.button("Pick attachment point").clicked() {
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    authoring.pick = AuthoringPick::PlaceAttachment;
                    authoring.status = Some("Specify the group's attachment point".into());
                }
            }
        }
        BehaviorWizard::Rotate {
            members,
            pivot,
            parameter,
            degrees,
        } => {
            ui.label(format!("Rotate by Parameter — {} objects", members.len()));
            parameter_picker(ui, dynamic, parameter);
            ui.label(format!(
                "Pivot: {}",
                pivot.map(|p| format!("{:.2},{:.2}", p.x, p.y)).unwrap_or_else(|| "pick pivot".into())
            ));
            if ui.button("Pick pivot").clicked() {
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    authoring.pick = AuthoringPick::RotatePivot;
                    authoring.status = Some("Specify rotation pivot".into());
                }
            }
            ui.checkbox(degrees, "Angle parameter is in degrees");
        }
    }
    ui.horizontal(|ui| {
        if ui.button("Confirm").clicked() {
            confirm = true;
        }
        if ui.button("Cancel").clicked() {
            cancel = true;
        }
    });
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.wizard = Some(wizard.clone());
    }
    if confirm {
        confirm_behavior_wizard(app, dynamic);
    }
    if cancel {
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.wizard = None;
            authoring.pick = AuthoringPick::Idle;
        }
    }
}

fn parameter_picker(ui: &mut Ui, dynamic: &DynamicDefinition, parameter: &mut ParameterId) {
    egui::ComboBox::from_id_salt(format!("wiz-param-{}", parameter.raw()))
        .selected_text(
            dynamic
                .parameter(*parameter)
                .map(|item| item.name.clone())
                .unwrap_or_else(|| "Parameter".into()),
        )
        .show_ui(ui, |ui| {
            for item in dynamic.sorted_parameters() {
                if ui.selectable_label(*parameter == item.id, &item.name).clicked() {
                    *parameter = item.id;
                }
            }
        });
}

fn preview_text_binding(dynamic: &DynamicDefinition, wizard: &BehaviorWizard) -> String {
    let BehaviorWizard::LinkText {
        parameter,
        mode,
        option_texts,
        formatted,
        boolean_true,
        boolean_false,
        ..
    } = wizard
    else {
        return String::new();
    };
    let values = cad_core::resolve_values(dynamic, None).unwrap_or_default();
    let binding = cad_core::TextBinding {
        id: cad_core::ActionId::UNASSIGNED,
        target: EntityId(0),
        mode: match *mode {
            1 => cad_core::TextBindingMode::OptionMap {
                parameter: *parameter,
                texts: option_texts.clone(),
            },
            2 => cad_core::TextBindingMode::Formatted {
                tokens: tokens_from_formatted(dynamic, formatted),
            },
            _ => cad_core::TextBindingMode::ShowValue {
                parameter: *parameter,
            },
        },
        boolean_true: boolean_true.clone(),
        boolean_false: boolean_false.clone(),
        number_precision: None,
    };
    cad_core::evaluate_text_binding(&binding, &dynamic.parameters, &values, false)
        .unwrap_or_else(|err| err.to_string())
}

fn tokens_from_formatted(dynamic: &DynamicDefinition, formatted: &str) -> Vec<cad_core::TextToken> {
    let mut tokens = Vec::new();
    let mut rest = formatted;
    while let Some(start) = rest.find('{') {
        if start > 0 {
            tokens.push(cad_core::TextToken::Literal(rest[..start].to_string()));
        }
        rest = &rest[start + 1..];
        let Some(end) = rest.find('}') else {
            tokens.push(cad_core::TextToken::Literal(format!("{{{rest}")));
            return tokens;
        };
        let name = &rest[..end];
        if let Some(parameter) = dynamic.parameters.iter().find(|item| item.name == name) {
            tokens.push(cad_core::TextToken::Parameter(parameter.id));
        } else {
            tokens.push(cad_core::TextToken::Literal(format!("{{{name}}}")));
        }
        rest = &rest[end + 1..];
    }
    if !rest.is_empty() {
        tokens.push(cad_core::TextToken::Literal(rest.to_string()));
    }
    tokens
}

fn confirm_behavior_wizard(app: &mut MyCadApp, _dynamic: &DynamicDefinition) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    let Some(wizard) = state.wizard.clone() else {
        return;
    };
    attach_named_behavior(app, |document, dynamic, _members, _parameter| {
        match wizard {
            BehaviorWizard::LinkText {
                target,
                parameter,
                mode,
                option_texts,
                formatted,
                boolean_true,
                boolean_false,
            } => {
                dynamic.text_bindings.retain(|binding| binding.target != target);
                let binding_mode = match mode {
                    1 => cad_core::TextBindingMode::OptionMap {
                        parameter,
                        texts: option_texts,
                    },
                    2 => cad_core::TextBindingMode::Formatted {
                        tokens: tokens_from_formatted(dynamic, &formatted),
                    },
                    _ => cad_core::TextBindingMode::ShowValue { parameter },
                };
                dynamic.text_bindings.push(cad_core::TextBinding {
                    id: document.allocate_action_id(),
                    target,
                    mode: binding_mode,
                    boolean_true,
                    boolean_false,
                    number_precision: None,
                });
                Ok("Linked text to parameter".into())
            }
            BehaviorWizard::ShowWhen {
                members,
                parameter,
                options,
                boolean,
            } => {
                let condition = match &dynamic.parameter(parameter).ok_or("Select a parameter")?.kind {
                    ParameterKind::Choice(_) => cad_core::ParameterCondition::Choice {
                        parameter,
                        options,
                    },
                    ParameterKind::Boolean(_) => cad_core::ParameterCondition::Boolean {
                        parameter,
                        state: boolean,
                    },
                    _ => return Err("Show When needs a Choice or On/Off parameter".into()),
                };
                dynamic.visibility.push(cad_core::VisibilityGroup {
                    id: document.allocate_action_id(),
                    name: dynamic
                        .parameter(parameter)
                        .map(|item| item.name.clone())
                        .unwrap_or_else(|| "Visibility".into()),
                    members,
                    conditions: vec![condition],
                });
                Ok("Show When condition added".into())
            }
            BehaviorWizard::Mirror {
                members,
                axis_a,
                axis_b,
                parameter,
                use_visibility,
                policy,
            } => {
                if use_visibility {
                    return Err("Use Show When to assign different parts instead of mirroring".into());
                }
                let Some(a) = axis_a else {
                    return Err("Pick two axis points".into());
                };
                let Some(b) = axis_b else {
                    return Err("Pick two axis points".into());
                };
                let condition = condition_for_parameter(dynamic, parameter)?;
                let id = document.allocate_action_id();
                dynamic.reflections.push(cad_core::ReflectionBehavior {
                    id,
                    name: Some("Flip".into()),
                    members,
                    axis_a: a,
                    axis_b: b,
                    condition,
                    text_policy: policy,
                });
                dynamic.transform_order.push(id);
                Ok("Mirror for option added".into())
            }
            BehaviorWizard::Position {
                members,
                attachment,
                parameter,
            } => {
                let options = match &dynamic.parameter(parameter).ok_or("Select a Choice")?.kind {
                    ParameterKind::Choice(choice) => choice
                        .options
                        .iter()
                        .map(|option| (option.id, option.label.clone()))
                        .collect::<Vec<_>>(),
                    _ => return Err("Position by Option needs a Choice parameter".into()),
                };
                let mut destinations = std::collections::BTreeMap::new();
                for (index, (option_id, label)) in options.into_iter().enumerate() {
                    let id = document.allocate_anchor_id();
                    dynamic.anchors.push(cad_core::AnchorDef {
                        id,
                        name: label,
                        position: Point2::new(index as f64 * 100.0, 0.0),
                        orientation: Some(0.0),
                        follow: None,
                    });
                    destinations.insert(option_id, id);
                }
                let action = document.allocate_action_id();
                dynamic.placements.push(cad_core::PlacementBehavior {
                    id: action,
                    name: Some("Position".into()),
                    members,
                    attachment: attachment.unwrap_or(Point2::new(0.0, 0.0)),
                    attachment_angle: 0.0,
                    parameter,
                    destinations,
                    boolean_destinations: None,
                });
                dynamic.transform_order.push(action);
                Ok("Position by Option added".into())
            }
            BehaviorWizard::Rotate {
                members,
                pivot,
                parameter,
                ..
            } => {
                let action = document.allocate_action_id();
                dynamic.rotations.push(cad_core::RotationBehavior {
                    id: action,
                    name: Some("Rotate".into()),
                    members,
                    pivot: pivot.unwrap_or(Point2::new(0.0, 0.0)),
                    source: cad_core::RotationSource::AngleParameter(parameter),
                });
                dynamic.transform_order.push(action);
                Ok("Rotate by Parameter added".into())
            }
        }
    });
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.wizard = None;
        authoring.pick = AuthoringPick::Idle;
    }
}

fn show_option_migration(ui: &mut Ui, app: &mut MyCadApp) {
    let Some(mut migration) = app.dynamic_authoring.as_ref().and_then(|state| state.option_migration.clone()) else {
        return;
    };
    ui.add_space(8.0);
    ui.colored_label(egui::Color32::from_rgb(220, 160, 80), "Remove option");
    if migration.usages.is_empty() {
        ui.label("This option is unused.");
    } else {
        ui.label("Used by:");
        for usage in &migration.usages {
            ui.label(usage);
        }
    }
    if let Some(document) = app.document.as_deref() {
        if let Some(authoring) = app.dynamic_authoring.as_ref() {
            if let Some(dynamic) = document.block_by_name(&authoring.block_name).and_then(|block| block.dynamic.as_ref()) {
                if let Some(ParameterKind::Choice(choice)) = dynamic.parameter(migration.parameter).map(|item| &item.kind) {
                    ui.label("Replace with");
                    for option in &choice.options {
                        if option.id == migration.from {
                            continue;
                        }
                        ui.radio_value(&mut migration.replacement, option.id, &option.label);
                    }
                }
            }
        }
    }
    let mut apply = false;
    let mut cancel = false;
    ui.horizontal(|ui| {
        if ui.button("Apply replacement").clicked() {
            apply = true;
        }
        if ui.button("Cancel").clicked() {
            cancel = true;
        }
    });
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.option_migration = Some(migration.clone());
    }
    if apply {
        let Some(state) = app.dynamic_authoring.clone() else {
            return;
        };
        if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
            app.history.begin();
            let before = document.block_by_name(&state.block_name).cloned();
            match cad_core::migrate_choice_option(
                document,
                &state.block_name,
                migration.parameter,
                migration.from,
                migration.replacement,
            ) {
                Ok(()) => {
                    if let Some(after) = document.block_by_name(&state.block_name).cloned() {
                        app.history.record(Edit::ReplaceBlockDefinition {
                            name: state.block_name,
                            before,
                            after: Some(after),
                        });
                    }
                    app.history.commit_open();
                    app.request_preview();
                }
                Err(err) => {
                    app.history.commit_open();
                    app.status = err.to_string();
                }
            }
        }
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.option_migration = None;
        }
    }
    if cancel {
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.option_migration = None;
        }
    }
}

fn show_phase3_authoring(ui: &mut Ui, app: &mut MyCadApp, dynamic: &DynamicDefinition) {
    ui.add_space(8.0);
    let mut show_inactive = app.dynamic_authoring.as_ref().is_some_and(|state| state.show_inactive);
    if ui.checkbox(&mut show_inactive, "Show inactive parts").changed() {
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.show_inactive = show_inactive;
        }
    }
    if !dynamic.visibility.is_empty() {
        ui.label(RichText::new("Visibility").small().weak());
        for group in &dynamic.visibility {
            ui.label(format!("{} ({} objects)", group.name, group.members.len()));
        }
    }
    if !dynamic.text_bindings.is_empty() {
        ui.label(RichText::new("Text bindings").small().weak());
        ui.label(format!("{} linked text entities", dynamic.text_bindings.len()));
    }
    if !dynamic.transform_order.is_empty() || !dynamic.reflections.is_empty() || !dynamic.rotations.is_empty() || !dynamic.placements.is_empty() {
        ui.label(RichText::new("Transform order").small().weak());
        ui.weak("Default: flip → rotate → place. Later pivots use the declared space.");
        let mut order = dynamic.transform_order.clone();
        if order.is_empty() {
            order.extend(dynamic.reflections.iter().map(|item| item.id));
            order.extend(dynamic.rotations.iter().map(|item| item.id));
            order.extend(dynamic.placements.iter().map(|item| item.id));
        }
        let mut changed = false;
        for index in 0..order.len() {
            let id = order[index];
            let name = transform_name(dynamic, id);
            ui.horizontal(|ui| {
                ui.label(name);
                if ui.small_button("↑").clicked() && index > 0 {
                    order.swap(index - 1, index);
                    changed = true;
                }
                if ui.small_button("↓").clicked() && index + 1 < order.len() {
                    order.swap(index, index + 1);
                    changed = true;
                }
            });
        }
        if changed {
            apply_transform_order(app, order);
        }
    }
    ui.label(RichText::new("Presets").small().weak());
    for preset in &dynamic.presets {
        ui.horizontal(|ui| {
            ui.label(&preset.name);
            if ui.small_button("Apply in Test").clicked() {
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    authoring.test_values = preset.values.clone();
                    authoring.mode = AuthoringMode::Test;
                    authoring.mark_preview_dirty();
                }
                app.request_preview();
            }
            if ui.small_button("Replace from Test").clicked() {
                replace_preset_from_test(app, preset.id);
            }
            if ui.small_button("Delete").clicked() {
                delete_preset(app, preset.id);
            }
        });
    }
    if ui.button("Save current configuration as preset…").clicked() {
        save_preset_from_test(app);
    }
    if ui.button("Add compatibility rule").clicked() {
        add_compatibility_rule(app);
    }
    for rule in &dynamic.compatibility {
        ui.label(cad_core::rule_reason(rule, &dynamic.parameters));
    }
    let nested_selected = app.selection.ids().iter().copied().find(|id| {
        app.document.as_deref().and_then(|document| document.entity_by_id(*id)).is_some_and(|entity| {
            matches!(entity.geometry, Geometry::Insert { .. })
        })
    });
    if let Some(nested) = nested_selected {
        if ui.button("Map selected nested parameter…").clicked() {
            add_nested_mapping(app, nested);
        }
    }
}

fn transform_name(dynamic: &DynamicDefinition, id: cad_core::ActionId) -> String {
    if let Some(item) = dynamic.reflections.iter().find(|item| item.id == id) {
        return item.name.clone().unwrap_or_else(|| "Flip".into());
    }
    if let Some(item) = dynamic.rotations.iter().find(|item| item.id == id) {
        return item.name.clone().unwrap_or_else(|| "Rotate".into());
    }
    if let Some(item) = dynamic.placements.iter().find(|item| item.id == id) {
        return item.name.clone().unwrap_or_else(|| "Place".into());
    }
    format!("#{}", id.raw())
}

fn apply_transform_order(app: &mut MyCadApp, order: Vec<cad_core::ActionId>) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        app.history.begin();
        let Some(block) = document.block_by_name(&state.block_name).cloned() else {
            return;
        };
        let mut after = block.clone();
        if let Some(dynamic) = after.dynamic.as_mut() {
            dynamic.transform_order = order;
        }
        document.replace_block_definition(after.clone());
        app.history.record(Edit::ReplaceBlockDefinition {
            name: state.block_name,
            before: Some(block),
            after: Some(after),
        });
        app.history.commit_open();
        app.request_preview();
    }
}

fn replace_preset_from_test(app: &mut MyCadApp, id: cad_core::PresetId) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        app.history.begin();
        let Some(block) = document.block_by_name(&state.block_name).cloned() else {
            return;
        };
        let mut after = block.clone();
        if let Some(dynamic) = after.dynamic.as_mut() {
            if let Some(preset) = dynamic.presets.iter_mut().find(|preset| preset.id == id) {
                preset.values = state.test_values.clone();
            }
        }
        document.replace_block_definition(after.clone());
        app.history.record(Edit::ReplaceBlockDefinition {
            name: state.block_name,
            before: Some(block),
            after: Some(after),
        });
        app.history.commit_open();
        app.request_preview();
    }
}

fn delete_preset(app: &mut MyCadApp, id: cad_core::PresetId) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        app.history.begin();
        let Some(block) = document.block_by_name(&state.block_name).cloned() else {
            return;
        };
        let mut after = block.clone();
        if let Some(dynamic) = after.dynamic.as_mut() {
            dynamic.presets.retain(|preset| preset.id != id);
        }
        document.replace_block_definition(after.clone());
        app.history.record(Edit::ReplaceBlockDefinition {
            name: state.block_name,
            before: Some(block),
            after: Some(after),
        });
        app.history.commit_open();
        app.request_preview();
    }
}

fn add_compatibility_rule(app: &mut MyCadApp) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        app.history.begin();
        let Some(block) = document.block_by_name(&state.block_name).cloned() else {
            return;
        };
        let mut after = block.clone();
        let Some(dynamic) = after.dynamic.as_mut() else {
            return;
        };
        let Some((when_id, when_option, target_id, kind)) = dynamic.parameters.iter().find_map(|left| {
            let ParameterKind::Choice(choice) = &left.kind else {
                return None;
            };
            let right = dynamic
                .parameters
                .iter()
                .find(|item| matches!(item.kind, ParameterKind::Number(_)))?;
            Some((
                left.id,
                choice.options.first()?.id,
                right.id,
                right.kind.clone(),
            ))
        }) else {
            app.history.commit_open();
            app.status = "Add a Choice and a numeric parameter first".into();
            return;
        };
        let ParameterKind::Number(numeric) = kind else {
            return;
        };
        let allowed = match numeric.domain {
            NumericDomain::AllowedValues(values) => Some(values),
            NumericDomain::Continuous => None,
        };
        dynamic.compatibility.push(cad_core::CompatibilityRule::ChoiceRestrictsNumeric {
            id: document.allocate_action_id(),
            when: when_id,
            when_option,
            target: target_id,
            min: numeric.min,
            max: numeric.max,
            allowed,
        });
        document.replace_block_definition(after.clone());
        app.history.record(Edit::ReplaceBlockDefinition {
            name: state.block_name,
            before: Some(block),
            after: Some(after),
        });
        app.history.commit_open();
        app.request_preview();
    }
}

fn add_nested_mapping(app: &mut MyCadApp, nested: EntityId) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    let Some(parameter) = state.selected_parameter else {
        app.status = "Select a parent parameter first".into();
        return;
    };
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        let child_param = document
            .entity_by_id(nested)
            .and_then(|entity| entity.geometry.insert_block_name().map(str::to_string))
            .and_then(|name| document.block_by_name(&name).cloned())
            .and_then(|block| block.dynamic.and_then(|dynamic| dynamic.parameters.first().map(|item| item.id)));
        let Some(child_param) = child_param else {
            app.status = "Nested block has no parameters".into();
            return;
        };
        app.history.begin();
        let Some(block) = document.block_by_name(&state.block_name).cloned() else {
            return;
        };
        let mut after = block.clone();
        if let Some(dynamic) = after.dynamic.as_mut() {
            dynamic.nested_inputs.push(cad_core::NestedInput {
                id: document.allocate_action_id(),
                source: parameter,
                target_occurrence: cad_core::OccurrencePath { inserts: vec![nested] },
                target_parameter: child_param,
                mapping: cad_core::NestedMapping::Direct,
            });
        }
        if let Err(err) = cad_core::validate_definition(after.dynamic.as_ref().unwrap(), &after.entities) {
            app.history.commit_open();
            app.status = err.to_string();
            return;
        }
        document.replace_block_definition(after.clone());
        app.history.record(Edit::ReplaceBlockDefinition {
            name: state.block_name,
            before: Some(block),
            after: Some(after),
        });
        app.history.commit_open();
        app.request_preview();
    }
}

pub fn link_text_from_selection(app: &mut MyCadApp) {
    let Some(state) = app.dynamic_authoring.as_ref() else {
        app.status = "Open Edit Dynamic Block first".into();
        return;
    };
    let Some(parameter) = state.selected_parameter else {
        app.status = "Select a parameter first".into();
        return;
    };
    let Some(target) = app.selection.ids().iter().copied().find(|id| {
        app.document.as_deref().and_then(|document| document.entity_by_id(*id)).is_some_and(|entity| {
            matches!(entity.geometry, Geometry::Text(_) | Geometry::MText(_))
        })
    }) else {
        app.status = "Select TEXT or MTEXT".into();
        return;
    };
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.wizard = Some(BehaviorWizard::LinkText {
            target,
            parameter,
            mode: 0,
            option_texts: BTreeMap::new(),
            formatted: String::new(),
            boolean_true: "On".into(),
            boolean_false: "Off".into(),
        });
        authoring.status = Some("Choose how the text should follow the parameter".into());
    }
}

pub fn show_when_from_selection(app: &mut MyCadApp) {
    start_member_wizard(app, |members, parameter| BehaviorWizard::ShowWhen {
        members,
        parameter,
        options: Vec::new(),
        boolean: true,
    });
}

pub fn always_visible_from_selection(app: &mut MyCadApp) {
    attach_named_behavior(app, |_document, dynamic, members, _parameter| {
        for group in &mut dynamic.visibility {
            group.members.retain(|id| !members.contains(id));
        }
        dynamic.visibility.retain(|group| !group.members.is_empty());
        Ok("Removed dynamic visibility".into())
    });
}

pub fn mirror_from_selection(app: &mut MyCadApp) {
    start_member_wizard(app, |members, parameter| BehaviorWizard::Mirror {
        members,
        axis_a: None,
        axis_b: None,
        parameter,
        use_visibility: false,
        policy: cad_core::TextReflectPolicy::KeepReadable,
    });
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.pick = AuthoringPick::MirrorAxisA;
        authoring.status = Some("Specify first axis point".into());
    }
}

pub fn position_from_selection(app: &mut MyCadApp) {
    start_member_wizard(app, |members, parameter| BehaviorWizard::Position {
        members,
        attachment: None,
        parameter,
    });
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.pick = AuthoringPick::PlaceAttachment;
        authoring.status = Some("Specify the group's attachment point".into());
    }
}

pub fn rotate_from_selection(app: &mut MyCadApp) {
    start_member_wizard(app, |members, parameter| BehaviorWizard::Rotate {
        members,
        pivot: None,
        parameter,
        degrees: true,
    });
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.pick = AuthoringPick::RotatePivot;
        authoring.status = Some("Specify rotation pivot".into());
    }
}

fn start_member_wizard(
    app: &mut MyCadApp,
    build: impl FnOnce(Vec<EntityId>, ParameterId) -> BehaviorWizard,
) {
    let Some(state) = app.dynamic_authoring.as_ref() else {
        app.status = "Open Edit Dynamic Block first".into();
        return;
    };
    let Some(parameter) = state.selected_parameter else {
        app.status = "Select a parameter first".into();
        return;
    };
    let members = app.selection.ids().to_vec();
    if members.is_empty() {
        app.status = "Select objects first".into();
        return;
    }
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.wizard = Some(build(members, parameter));
    }
}

pub fn save_preset_from_test(app: &mut MyCadApp) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        app.history.begin();
        let Some(block) = document.block_by_name(&state.block_name).cloned() else {
            return;
        };
        let mut after = block.clone();
        let Some(dynamic) = after.dynamic.as_mut() else {
            return;
        };
        let values = if state.mode == AuthoringMode::Test {
            state.test_values.clone()
        } else {
            cad_core::resolve_values(dynamic, None).unwrap_or_default()
        };
        let id = document.allocate_preset_id();
        dynamic.presets.push(cad_core::Preset {
            id,
            name: format!("Preset {}", dynamic.presets.len() + 1),
            values,
        });
        document.replace_block_definition(after.clone());
        app.history.record(Edit::ReplaceBlockDefinition {
            name: state.block_name,
            before: Some(block),
            after: Some(after),
        });
        app.history.commit_open();
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.status = Some("Saved preset".into());
        }
        app.request_preview();
    }
}

fn condition_for_parameter(
    dynamic: &DynamicDefinition,
    parameter: ParameterId,
) -> Result<cad_core::ParameterCondition, String> {
    match &dynamic.parameter(parameter).ok_or("Select a parameter")?.kind {
        ParameterKind::Choice(choice) => Ok(cad_core::ParameterCondition::Choice {
            parameter,
            options: vec![choice.default],
        }),
        ParameterKind::Boolean(_) => Ok(cad_core::ParameterCondition::Boolean {
            parameter,
            state: true,
        }),
        _ => Err("Show When / Mirror needs a Choice or On/Off parameter".into()),
    }
}

fn attach_named_behavior(
    app: &mut MyCadApp,
    write: impl FnOnce(
        &mut Document,
        &mut DynamicDefinition,
        Vec<EntityId>,
        ParameterId,
    ) -> Result<String, String>,
) {
    let Some(state) = app.dynamic_authoring.clone() else {
        app.status = "Open Edit Dynamic Block first".into();
        return;
    };
    let Some(parameter) = state.selected_parameter else {
        app.status = "Select a parameter first".into();
        return;
    };
    let members: Vec<EntityId> = app.selection.ids().to_vec();
    if members.is_empty() {
        app.status = "Select objects first".into();
        return;
    }
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        app.history.begin();
        let Some(block) = document.block_by_name(&state.block_name).cloned() else {
            return;
        };
        let mut after = block.clone();
        let Some(dynamic) = after.dynamic.as_mut() else {
            return;
        };
        match write(document, dynamic, members, parameter) {
            Ok(status) => {
                if let Err(err) = cad_core::validate_definition(dynamic, &after.entities) {
                    app.history.commit_open();
                    app.status = err.to_string();
                    return;
                }
                document.replace_block_definition(after.clone());
                app.history.record(Edit::ReplaceBlockDefinition {
                    name: state.block_name,
                    before: Some(block),
                    after: Some(after),
                });
                app.history.commit_open();
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    authoring.status = Some(status);
                }
                app.request_preview();
            }
            Err(err) => {
                app.history.commit_open();
                app.status = err;
            }
        }
    }
}

pub fn authoring_size_parameters(app: &MyCadApp) -> Vec<(ParameterId, String)> {
    let Some(state) = app.dynamic_authoring.as_ref() else {
        return Vec::new();
    };
    let Some(document) = app.document.as_deref() else {
        return Vec::new();
    };
    let Some(dynamic) = document
        .block_by_name(&state.block_name)
        .and_then(|block| block.dynamic.as_ref())
    else {
        return Vec::new();
    };
    dynamic
        .parameters
        .iter()
        .filter(|parameter| matches!(parameter.kind, ParameterKind::Number(_)))
        .map(|parameter| (parameter.id, parameter.name.clone()))
        .collect()
}

pub fn select_authoring_parameter(app: &mut MyCadApp, id: ParameterId) {
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.selected_parameter = Some(id);
    }
}

pub fn preview_result_is_current(
    message_document: u64,
    message_session: u64,
    message_preview: u64,
    document: u64,
    session: u64,
    preview: u64,
) -> bool {
    message_document == document && message_session == session && message_preview == preview
}

fn remove_behavior(document: &mut Document, history: &mut History, block_name: &str, id: ActionId) {
    let Some(block) = document.block_by_name(block_name).cloned() else {
        return;
    };
    let mut after = block.clone();
    if let Some(dynamic) = after.dynamic.as_mut() {
        dynamic.behaviors.retain(|behavior| behavior.id != id);
    }
    document.replace_block_definition(after.clone());
    history.record(Edit::ReplaceBlockDefinition {
        name: block_name.to_string(),
        before: Some(block),
        after: Some(after),
    });
}

#[derive(Debug, Clone, PartialEq)]
pub enum NumericEdit {
    None,
    Preview(f64),
    Commit(f64),
    Cancel,
    Invalid(String),
}

pub fn numeric_field(
    ui: &mut Ui,
    id: &str,
    draft: &mut String,
    numeric: &NumericParameter,
    committed: Option<f64>,
) -> NumericEdit {
    let mut outcome = NumericEdit::None;
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            if ui.button("−").clicked() {
                let current = committed.unwrap_or(numeric.default);
                let next = increment_numeric(numeric, current, -1);
                *draft = format_display_number(next, numeric.display_precision);
                outcome = NumericEdit::Commit(next);
            }
            if let NumericDomain::AllowedValues(values) = &numeric.domain {
                let current = committed.unwrap_or(numeric.default);
                let label = format_display_number(current, numeric.display_precision);
                egui::ComboBox::from_id_salt(id)
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        for value in values {
                            let text = format_display_number(*value, numeric.display_precision);
                            if ui.selectable_label(cad_core::numbers_equal(*value, current), text).clicked()
                            {
                                *draft = format_display_number(*value, numeric.display_precision);
                                outcome = NumericEdit::Commit(*value);
                            }
                        }
                    });
            } else {
                let response = ui.add(egui::TextEdit::singleline(draft).id_source(id).desired_width(88.0));
                let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if escape && (response.has_focus() || response.lost_focus()) {
                    *draft = committed
                        .map(|value| format_display_number(value, numeric.display_precision))
                        .unwrap_or_default();
                    outcome = NumericEdit::Cancel;
                } else if !escape
                    && (response.lost_focus()
                        || (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))))
                {
                    match parse_numeric_draft(draft, numeric) {
                        Ok(ParseOutcome::Exact(value)) => {
                            *draft = format_display_number(value, numeric.display_precision);
                            outcome = NumericEdit::Commit(value);
                        }
                        Err(err) => outcome = NumericEdit::Invalid(err),
                    }
                } else if response.changed() {
                    if let Ok(ParseOutcome::Exact(value)) = parse_numeric_draft(draft, numeric) {
                        outcome = NumericEdit::Preview(value);
                    }
                }
            }
            if ui.button("+").clicked() {
                let current = committed.unwrap_or(numeric.default);
                let next = increment_numeric(numeric, current, 1);
                *draft = format_display_number(next, numeric.display_precision);
                outcome = NumericEdit::Commit(next);
            }
            let unit = numeric.unit.label();
            if !unit.is_empty() {
                ui.weak(unit);
            }
        });
    });
    outcome
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParseOutcome {
    Exact(f64),
}

pub fn parse_numeric_draft(draft: &str, numeric: &NumericParameter) -> Result<ParseOutcome, String> {
    let trimmed = draft.trim();
    if trimmed.is_empty() || trimmed == "-" || trimmed == "." || trimmed == "-." {
        return Err("incomplete".into());
    }
    let value: f64 = trimmed.parse().map_err(|_| "not a number".to_string())?;
    if !value.is_finite() {
        return Err("not finite".into());
    }
    match cad_core::validate_numeric_value(ParameterId(0), numeric, value) {
        Ok(()) => Ok(ParseOutcome::Exact(value)),
        Err(DynamicError::OffStep { nearest, .. }) => Err(format!(
            "off-step; nearest {} and {}",
            format_display_number(nearest[0], numeric.display_precision),
            format_display_number(nearest[1], numeric.display_precision)
        )),
        Err(DynamicError::ValueNotInList { .. }) => {
            let nearest = match &numeric.domain {
                NumericDomain::AllowedValues(values) => nearest_allowed_values(values, value),
                _ => None,
            };
            Err(match nearest {
                Some((lo, hi)) => format!(
                    "not in list; nearest {} and {}",
                    format_display_number(lo, numeric.display_precision),
                    format_display_number(hi, numeric.display_precision)
                ),
                None => "not in list".into(),
            })
        }
        Err(err) => Err(err.to_string()),
    }
}

pub fn paint_authoring_overlays(
    painter: &egui::Painter,
    app: &MyCadApp,
    rect: egui::Rect,
) {
    let Some(state) = app.dynamic_authoring.as_ref() else {
        return;
    };
    let Some(document) = app.document.as_deref() else {
        return;
    };
    let Some(block) = document.block_by_name(&state.block_name) else {
        return;
    };
    let to_screen = |local: Point2| -> egui::Pos2 {
        let world = app.block_edit.world_from_local().apply(local);
        let origin = Point2::new(rect.left() as f64, rect.top() as f64);
        let size = Point2::new(rect.width() as f64, rect.height() as f64);
        let screen = app.camera.world_to_screen(world, origin, size);
        egui::pos2(screen.x as f32, screen.y as f32)
    };
    if let Some(wizard) = &state.size_wizard {
        if let (Some(a), Some(b)) = (wizard.point_a, wizard.point_b.or(wizard.hover)) {
            let pa = to_screen(a);
            let pb = to_screen(b);
            painter.line_segment([pa, pb], egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(80, 180, 255)));
            painter.circle_filled(pa, 4.0, egui::Color32::from_rgb(240, 200, 80));
            painter.circle_filled(pb, 4.0, egui::Color32::from_rgb(80, 220, 140));
            if let Some((_, size)) = wizard.preview_measure() {
                painter.text(
                    pa.lerp(pb, 0.5),
                    egui::Align2::CENTER_BOTTOM,
                    format_display_number(size, 4),
                    egui::FontId::proportional(12.0),
                    egui::Color32::WHITE,
                );
            }
        }
    }
    if state.test_compare {
        for entity in &block.entities {
            if let Geometry::Line { start, end } = &entity.geometry {
                painter.line_segment(
                    [
                        to_screen(Point2::new(start.x, start.y)),
                        to_screen(Point2::new(end.x, end.y)),
                    ],
                    egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(160, 160, 160)),
                );
            }
        }
    }
    if state.show_inactive {
        if let Some(dynamic) = &block.dynamic {
            let values = state.test_values.clone();
            for group in &dynamic.visibility {
                if cad_core::conditions_match(&group.conditions, &values) {
                    continue;
                }
                for id in &group.members {
                    if let Some(entity) = block.entities.iter().find(|entity| entity.id == *id) {
                        if let Geometry::Line { start, end } = &entity.geometry {
                            painter.line_segment(
                                [
                                    to_screen(Point2::new(start.x, start.y)),
                                    to_screen(Point2::new(end.x, end.y)),
                                ],
                                egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(90, 90, 140)),
                            );
                        }
                    }
                }
            }
        }
    }
    if let Some(wizard) = &state.wizard {
        match wizard {
            BehaviorWizard::Mirror { axis_a, axis_b, .. } => {
                if let (Some(a), Some(b)) = (*axis_a, *axis_b) {
                    painter.line_segment(
                        [to_screen(a), to_screen(b)],
                        egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(220, 120, 255)),
                    );
                }
            }
            BehaviorWizard::Position { attachment, .. } => {
                if let Some(point) = *attachment {
                    painter.circle_filled(to_screen(point), 5.0, egui::Color32::from_rgb(80, 220, 140));
                }
            }
            BehaviorWizard::Rotate { pivot, .. } => {
                if let Some(point) = *pivot {
                    painter.circle_filled(to_screen(point), 5.0, egui::Color32::from_rgb(240, 180, 80));
                }
            }
            _ => {}
        }
    }
    if let Some(dynamic) = &block.dynamic {
        for anchor in &dynamic.anchors {
            let p = to_screen(anchor.position);
            painter.circle_filled(p, 4.0, egui::Color32::from_rgb(80, 180, 255));
            if let Some(angle) = anchor.orientation {
                let tip = to_screen(Point2::new(
                    anchor.position.x + 12.0 * angle.cos(),
                    anchor.position.y + 12.0 * angle.sin(),
                ));
                painter.line_segment([p, tip], egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(80, 180, 255)));
            }
        }
    }
    if let Some(dynamic) = &block.dynamic {
        for parameter in &dynamic.parameters {
            let ParameterKind::Number(numeric) = &parameter.kind else {
                continue;
            };
            let Some(size) = &numeric.size else {
                continue;
            };
            let pa = to_screen(size.point_a + size.label_offset);
            let pb = to_screen(size.point_b + size.label_offset);
            painter.line_segment([pa, pb], egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(120, 180, 255)));
            let fixed = match size.anchor {
                AnchorPolicy::FirstFixed => pa,
                AnchorPolicy::SecondFixed => pb,
                AnchorPolicy::CenterFixed => pa.lerp(pb, 0.5),
            };
            painter.rect_stroke(
                egui::Rect::from_center_size(fixed, egui::vec2(10.0, 10.0)),
                0.0,
                egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(240, 200, 80)),
                egui::StrokeKind::Middle,
            );
        }
        if let Some(id) = state.selected_behavior {
            if let Some(behavior) = dynamic.behaviors.iter().find(|item| item.id == id) {
                for target in &behavior.targets {
                    if let Some(entity) = block.entities.iter().find(|entity| entity.id == target.entity_id()) {
                        for (point, handle) in stretch_handles(entity) {
                            if handle == *target {
                                painter.circle_filled(
                                    to_screen(point),
                                    5.0,
                                    egui::Color32::from_rgb(80, 220, 140),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(attach) = &state.attach {
        for target in &attach.handles {
            if let Some(entity) = block.entities.iter().find(|entity| entity.id == target.entity_id()) {
                for (point, handle) in stretch_handles(entity) {
                    if handle == *target {
                        painter.circle_stroke(
                            to_screen(point),
                            6.0,
                            egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(255, 180, 80)),
                        );
                    }
                }
            }
        }
        if let Some((a, b)) = attach.window {
            let rect = egui::Rect::from_two_pos(to_screen(a), to_screen(b));
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(80, 180, 220)),
                egui::StrokeKind::Middle,
            );
        }
    }
    if let Some(pointer) = app.last_pointer {
        let mut best: Option<(f64, Point2, String)> = None;
        for entity in &block.entities {
            for (point, _) in stretch_handles(entity) {
                let screen = to_screen(point);
                let distance = ((screen.x as f64 - pointer.x as f64).powi(2)
                    + (screen.y as f64 - pointer.y as f64).powi(2))
                .sqrt();
                if best.as_ref().is_none_or(|(best_distance, _, _)| distance < *best_distance) {
                    best = Some((distance, point, entity.geometry.type_name().to_string()));
                }
            }
        }
        if let Some((distance, point, owner)) = best {
            if distance <= HANDLE_PICK_PX {
                painter.text(
                    to_screen(point),
                    egui::Align2::LEFT_BOTTOM,
                    owner,
                    egui::FontId::proportional(11.0),
                    egui::Color32::from_rgb(240, 220, 160),
                );
            }
        }
    }
}

pub fn show_instance_parameters(ui: &mut Ui, app: &mut MyCadApp, _entity: &Entity) {
    show_selected_instance_parameters(ui, app);
}

pub fn show_selected_instance_parameters(ui: &mut Ui, app: &mut MyCadApp) {
    let Some((dynamic, ids, current, mixed)) = selected_same_definition_config(app) else {
        return;
    };
    ui.add_space(10.0);
    let mut drafts = app.config_form_drafts.clone();
    let mut errors = app.config_form_errors.clone();
    let mut pending = app.config_pending.clone();
    let action = crate::config_form::show_configuration_form(
        ui,
        "props-config",
        &dynamic,
        &current,
        &mixed,
        &mut drafts,
        &mut errors,
        &mut pending,
    );
    app.config_form_drafts = drafts;
    app.config_form_errors = errors;
    app.config_pending = pending;
    apply_config_action(app, &dynamic, &ids, &current, action);
}

fn selected_same_definition_config(
    app: &MyCadApp,
) -> Option<(
    DynamicDefinition,
    Vec<EntityId>,
    BTreeMap<ParameterId, ParameterValue>,
    std::collections::BTreeSet<ParameterId>,
)> {
    let document = app.document.as_deref()?;
    let inserts: Vec<&Entity> = app
        .selection
        .ids()
        .iter()
        .filter_map(|id| document.entity_by_id(*id))
        .filter(|entity| matches!(entity.geometry, Geometry::Insert { .. }))
        .collect();
    if inserts.is_empty() {
        return None;
    }
    let name = inserts[0].geometry.insert_block_name()?.to_string();
    if inserts
        .iter()
        .any(|entity| entity.geometry.insert_block_name() != Some(name.as_str()))
    {
        return None;
    }
    let dynamic = document.block_by_name(&name)?.dynamic.clone()?;
    if dynamic.parameters.is_empty() {
        return None;
    }
    let ids: Vec<EntityId> = inserts.iter().map(|entity| entity.id).collect();
    let mut current = BTreeMap::new();
    let mut mixed = std::collections::BTreeSet::new();
    for parameter in dynamic.sorted_parameters() {
        let mut values = Vec::new();
        for entity in &inserts {
            let value = entity
                .geometry
                .insert_configuration()
                .and_then(|config| config.get(parameter.id).cloned())
                .unwrap_or_else(|| parameter.default_value());
            values.push(value);
        }
        let first = values[0].clone();
        if values.iter().any(|value| {
            !cad_core::parameter_values_equal(&dynamic.parameters, parameter.id, value, &first)
        }) {
            mixed.insert(parameter.id);
        }
        current.insert(parameter.id, first);
    }
    Some((dynamic, ids, current, mixed))
}

fn apply_config_action(
    app: &mut MyCadApp,
    dynamic: &DynamicDefinition,
    ids: &[EntityId],
    current: &BTreeMap<ParameterId, ParameterValue>,
    action: crate::config_form::ConfigFormAction,
) {
    match action {
        crate::config_form::ConfigFormAction::None => {}
        crate::config_form::ConfigFormAction::Values(values) => {
            let mut patch = BTreeMap::new();
            for (id, value) in values {
                if current.get(&id) != Some(&value) {
                    patch.insert(id, value);
                }
            }
            if !patch.is_empty() {
                commit_patch(app, ids, &patch);
            }
        }
        crate::config_form::ConfigFormAction::Reset(id) => {
            if let Some(parameter) = dynamic.parameter(id) {
                let mut patch = BTreeMap::new();
                patch.insert(id, parameter.default_value());
                commit_patch(app, ids, &patch);
            }
        }
        crate::config_form::ConfigFormAction::ResetAll => {
            let mut patch = BTreeMap::new();
            for parameter in &dynamic.parameters {
                patch.insert(parameter.id, parameter.default_value());
            }
            commit_patch(app, ids, &patch);
        }
        crate::config_form::ConfigFormAction::ApplyPreset(id) => {
            if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
                app.history.begin();
                match apply_preset_to_instances(document, &mut app.history, ids, id) {
                    Ok(()) => {
                        app.history.commit_open();
                        app.request_preview();
                    }
                    Err(err) => {
                        app.history.commit_open();
                        app.status = err;
                    }
                }
            }
        }
    }
}

fn commit_patch(
    app: &mut MyCadApp,
    ids: &[EntityId],
    patch: &BTreeMap<ParameterId, ParameterValue>,
) {
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        app.history.begin();
        let result = if ids.len() == 1 {
            if let Some((parameter, value)) = patch.iter().next() {
                if patch.len() == 1 {
                    commit_instance_value(document, &mut app.history, ids[0], *parameter, value.clone())
                } else {
                    commit_instance_values(document, &mut app.history, ids, patch)
                }
            } else {
                Ok(())
            }
        } else {
            commit_instance_values(document, &mut app.history, ids, patch)
        };
        match result {
            Ok(()) => {
                app.history.commit_open();
                app.request_preview();
            }
            Err(err) => {
                app.history.commit_open();
                app.status = err;
            }
        }
    }
}

pub fn begin_configure_block(app: &mut MyCadApp) {
    let Some((_, ids, _, _)) = selected_same_definition_config(app) else {
        app.status = "Select references of one dynamic block".into();
        return;
    };
    app.configure_dialog = Some(ConfigureDialog {
        entity_ids: ids,
        drafts: BTreeMap::new(),
        errors: BTreeMap::new(),
        pending: None,
    });
}

pub fn show_configure_dialog(ctx: &egui::Context, app: &mut MyCadApp) {
    let Some(mut dialog) = app.configure_dialog.clone() else {
        return;
    };
    let Some((dynamic, _, current, mixed)) = selected_same_definition_config(app) else {
        app.configure_dialog = None;
        return;
    };
    let mut open = true;
    let mut close = false;
    egui::Window::new("Configure Block")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .show(ctx, |ui| {
            let action = crate::config_form::show_configuration_form(
                ui,
                "configure-block",
                &dynamic,
                &current,
                &mixed,
                &mut dialog.drafts,
                &mut dialog.errors,
                &mut dialog.pending,
            );
            apply_config_action(app, &dynamic, &dialog.entity_ids, &current, action);
            if ui.button("Close").clicked() {
                close = true;
            }
        });
    if !open || close {
        app.configure_dialog = None;
    } else {
        app.configure_dialog = Some(dialog);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_core::{
        identity_insert, BlockDefinition, GeometryTarget, NumericParameter, ParameterValue, Point3,
    };

    fn offset_document() -> (Document, EntityId, ParameterId) {
        let mut document = Document::default();
        let param = document.allocate_parameter_id();
        let mut line = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(10.0, 0.0),
        });
        line.id = document.allocate_id();
        let mut numeric = NumericParameter::length(0.0);
        numeric.reference = 0.0;
        let mut definition =
            BlockDefinition::plain("Offset", Point3::from_xy(0.0, 0.0), vec![line.clone()]);
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
                follow: cad_core::FollowRole::Second,
                name: None,
            }],
            ..Default::default()
        });
        document.replace_block_definition(definition);
        let mut insert = Entity::new(identity_insert("Offset".into(), Point3::from_xy(0.0, 0.0)));
        insert.geometry.set_insert_configuration(Some({
            let mut config = InstanceConfiguration::default();
            config.set(param, ParameterValue::Number(0.0));
            config
        }));
        let insert = document.add_entity(insert);
        (document, insert.id, param)
    }

    #[test]
    fn committed_value_undoes_with_geometry() {
        let (mut document, entity_id, param) = offset_document();
        let mut history = History::default();
        history.begin();
        commit_instance_value(&mut document, &mut history, entity_id, param, ParameterValue::Number(5.0)).unwrap();
        history.commit_open();
        let value = document
            .entity_by_id(entity_id)
            .unwrap()
            .geometry
            .insert_configuration()
            .unwrap()
            .get(param)
            .unwrap()
            .as_number()
            .unwrap();
        assert!((value - 5.0).abs() < 1e-12);
        history.undo(&mut document);
        let restored = document
            .entity_by_id(entity_id)
            .unwrap()
            .geometry
            .insert_configuration()
            .and_then(|config| config.get(param))
            .and_then(ParameterValue::as_number)
            .unwrap_or(0.0);
        assert!((restored - 0.0).abs() < 1e-12);
        history.redo(&mut document);
        let redone = document
            .entity_by_id(entity_id)
            .unwrap()
            .geometry
            .insert_configuration()
            .unwrap()
            .get(param)
            .unwrap()
            .as_number()
            .unwrap();
        assert!((redone - 5.0).abs() < 1e-12);
    }

    #[test]
    fn format_number_keeps_integer_zeros_at_precision_zero() {
        assert_eq!(format_display_number(100.0, 0), "100");
        assert_eq!(format_display_number(100.0, 4), "100");
    }

    #[test]
    fn required_increment_does_not_silently_snap() {
        let mut numeric = NumericParameter::length(10.0);
        numeric.step = Some(5.0);
        numeric.step_policy = StepPolicy::RequiredIncrement;
        numeric.step_origin = cad_core::StepOrigin::Zero;
        let err = parse_numeric_draft("12", &numeric).unwrap_err();
        assert!(err.contains("off-step"));
        assert!(matches!(
            parse_numeric_draft("10", &numeric),
            Ok(ParseOutcome::Exact(value)) if (value - 10.0).abs() < 1e-12
        ));
    }

    #[test]
    fn test_config_keeps_every_parameter_value() {
        let (document, _, param) = offset_document();
        let mut state = AuthoringState::new("Offset".into(), EntityId(1), false, None);
        state.mode = AuthoringMode::Test;
        state.test_values.insert(param, ParameterValue::Number(7.0));
        state.selected_parameter = None;
        let config = state.test_config(&document).unwrap();
        assert!((config.get(param).unwrap().as_number().unwrap() - 7.0).abs() < 1e-12);
    }

    #[test]
    fn parameter_draft_survives_rebuilding_from_definition() {
        let draft = ParameterDraft::from_parameter("Span", &NumericParameter::length(800.0));
        let mut name = draft.name.clone();
        name.push_str(" Width");
        assert_eq!(draft.original_name, "Span");
        assert_ne!(name, draft.original_name);
    }

    #[test]
    fn apply_parameter_settings_reports_validation_errors() {
        let (mut document, _, param) = offset_document();
        let mut history = History::default();
        let mut numeric = NumericParameter::length(10.0);
        numeric.min = Some(20.0);
        numeric.max = Some(10.0);
        let err = apply_parameter_settings(
            &mut document,
            &mut history,
            "Offset",
            param,
            "Offset".into(),
            numeric,
        )
        .unwrap_err();
        assert!(!err.message.is_empty());
    }

    #[test]
    fn superseded_preview_generation_is_discarded() {
        assert!(!preview_result_is_current(1, 1, 1, 1, 1, 2));
        assert!(!preview_result_is_current(1, 1, 4, 2, 1, 4));
        assert!(preview_result_is_current(3, 2, 9, 3, 2, 9));
    }

    #[test]
    fn size_wizard_keeps_points_as_fixed_local_references() {
        let mut wizard = SizeWizard::new("Span".into());
        wizard.point_a = Some(Point2::new(0.0, 0.0));
        wizard.point_b = Some(Point2::new(800.0, 0.0));
        wizard.measure = MeasureMode::LocalX;
        let (direction, size) = wizard.measured().unwrap();
        assert!((size - 800.0).abs() < 1e-12);
        assert!((direction.x - 1.0).abs() < 1e-12);
        let reversed = measure_size(
            wizard.point_b.unwrap(),
            wizard.point_a.unwrap(),
            MeasureMode::LocalX,
        )
        .unwrap();
        assert!((reversed.1 - 800.0).abs() < 1e-12);
        assert!((direction.x + reversed.0.x).abs() < 1e-12);
    }

    #[test]
    fn handle_pick_tolerance_is_screen_pixels() {
        assert_eq!(HANDLE_PICK_PX, 9.0);
    }

    #[test]
    fn multi_selection_rejects_when_any_result_is_invalid() {
        let (mut document, first, param) = offset_document();
        let mut second = Entity::new(identity_insert("Offset".into(), Point3::from_xy(20.0, 0.0)));
        second.geometry.set_insert_configuration(Some({
            let mut config = InstanceConfiguration::default();
            config.set(param, ParameterValue::Number(0.0));
            config
        }));
        let second = document.add_entity(second).id;
        {
            let definition = document.block_by_name_mut("Offset").unwrap();
            if let ParameterKind::Number(numeric) = &mut definition.dynamic.as_mut().unwrap().parameters[0].kind {
                numeric.max = Some(10.0);
            }
        }
        let mut history = History::default();
        history.begin();
        let mut patch = BTreeMap::new();
        patch.insert(param, ParameterValue::Number(25.0));
        let err = commit_instance_values(&mut document, &mut history, &[first, second], &patch).unwrap_err();
        assert!(!err.is_empty());
        history.commit_open();
        for id in [first, second] {
            let value = document
                .entity_by_id(id)
                .unwrap()
                .geometry
                .insert_configuration()
                .and_then(|config| config.get(param))
                .and_then(ParameterValue::as_number)
                .unwrap();
            assert!((value - 0.0).abs() < 1e-12);
        }
    }

    #[test]
    fn applying_a_preset_undoes_as_one_transaction() {
        let (mut document, entity_id, param) = offset_document();
        let preset = document.allocate_preset_id();
        {
            let block = document.block_by_name_mut("Offset").unwrap();
            let mut values = BTreeMap::new();
            values.insert(param, ParameterValue::Number(5.0));
            block.dynamic.as_mut().unwrap().presets.push(cad_core::Preset {
                id: preset,
                name: "Five".into(),
                values,
            });
        }
        let mut history = History::default();
        history.begin();
        apply_preset_to_instances(&mut document, &mut history, &[entity_id], preset).unwrap();
        history.commit_open();
        let value = document
            .entity_by_id(entity_id)
            .unwrap()
            .geometry
            .insert_configuration()
            .unwrap()
            .get(param)
            .unwrap()
            .as_number()
            .unwrap();
        assert!((value - 5.0).abs() < 1e-12);
        history.undo(&mut document);
        let restored = document
            .entity_by_id(entity_id)
            .unwrap()
            .geometry
            .insert_configuration()
            .and_then(|config| config.get(param))
            .and_then(ParameterValue::as_number)
            .unwrap_or(0.0);
        assert!((restored - 0.0).abs() < 1e-12);
    }
}
