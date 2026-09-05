//! Dynamic-block conversion, authoring panel, and numeric instance editor.

use cad_core::{
    count_block_references, increment_numeric, make_unique_block, snap_numeric,
    validate_parameter_value, ActionId, BehaviorKind, CompositionRule, Document, DynamicBehavior,
    DynamicDefinition, Entity, EntityId, Geometry, GeometryTarget, InstanceConfiguration,
    NumericParameter, ParameterDef, ParameterId, ParameterKind, ParameterValue, Point2, StepPolicy,
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
pub enum AuthoringPick {
    Idle,
    DirectionStart,
    DirectionEnd,
}

#[derive(Debug, Clone)]
pub struct AuthoringState {
    pub block_name: String,
    pub instance_id: EntityId,
    pub created_unique: bool,
    pub previous_name: Option<String>,
    pub selected_parameter: Option<ParameterId>,
    pub test_draft: String,
    pub test_committed: Option<f64>,
    pub pick: AuthoringPick,
    pub direction_start: Option<Point2>,
    pub direction: Point2,
    pub highlight: Vec<GeometryTarget>,
    pub status: Option<String>,
}

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
            test_draft: String::new(),
            test_committed: None,
            pick: AuthoringPick::Idle,
            direction_start: None,
            direction: Point2::new(1.0, 0.0),
            highlight: Vec::new(),
            status: None,
        }
    }

    pub fn test_config(&self, document: &Document) -> Option<InstanceConfiguration> {
        let value = self.test_committed?;
        let parameter = self.selected_parameter?;
        let definition = document.block_by_name(&self.block_name)?;
        let _ = definition.dynamic.as_ref()?.parameter(parameter)?;
        let mut config = InstanceConfiguration::default();
        config.set(parameter, ParameterValue::Number(value));
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

pub fn add_number_parameter(document: &mut Document, history: &mut History, block_name: &str) {
    let Some(block) = document.block_by_name(block_name).cloned() else {
        return;
    };
    let mut after = block.clone();
    let id = document.allocate_parameter_id();
    let mut numeric = NumericParameter::length(0.0);
    numeric.display_order = after
        .dynamic
        .as_ref()
        .map(|dynamic| dynamic.parameters.len() as i32)
        .unwrap_or(0);
    let name = unique_parameter_name(after.dynamic.as_ref(), "Length");
    let dynamic = after.dynamic.get_or_insert_with(DynamicDefinition::default);
    dynamic
        .parameters
        .push(ParameterDef::number(id, name, numeric));
    document.replace_block_definition(after.clone());
    history.record(Edit::ReplaceBlockDefinition {
        name: block_name.to_string(),
        before: Some(block),
        after: Some(after),
    });
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
    value: f64,
) -> Result<(), String> {
    let (space, index) = document
        .find_entity_location(entity_id)
        .ok_or("Block reference was not found")?;
    let before = document
        .entity_by_id(entity_id)
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
    let param = definition
        .dynamic
        .as_ref()
        .and_then(|dynamic| dynamic.parameter(parameter))
        .ok_or("Parameter was not found")?;
    let value = ParameterValue::Number(value);
    validate_parameter_value(param, &value).map_err(|err| err.to_string())?;
    let mut after = before.clone();
    let config = after
        .geometry
        .insert_configuration_mut()
        .ok_or("Select a block reference")?;
    let mut values = config.take().unwrap_or_default();
    values.set(parameter, value);
    *config = Some(values);
    document
        .replace_entity_in(&space, entity_id, after.clone())
        .ok_or("Block reference was not found")?;
    history.record(Edit::ReplaceEntity {
        space,
        index,
        before,
        after,
    });
    Ok(())
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
    let Some(document) = app.document.as_deref() else {
        return;
    };
    let Some(block) = document.block_by_name(&state.block_name) else {
        ui.weak("The definition is no longer in the drawing.");
        return;
    };
    if let Some(status) = &state.status {
        ui.colored_label(egui::Color32::from_rgb(220, 160, 80), status);
    }
    ui.label(RichText::new(&state.block_name).strong());
    ui.add_space(6.0);
    if ui.button("Add Number").clicked() {
        let name = state.block_name.clone();
        if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
            app.history.begin();
            add_number_parameter(document, &mut app.history, &name);
            app.history.commit_open();
            app.refresh_derived();
        }
        return;
    }
    let dynamic = block.dynamic.clone().unwrap_or_default();
    ui.add_space(6.0);
    ui.label(RichText::new("Parameters").small().weak());
    let mut selected = state.selected_parameter;
    for parameter in dynamic.sorted_parameters() {
        let checked = selected == Some(parameter.id);
        if ui.selectable_label(checked, &parameter.name).clicked() {
            selected = Some(parameter.id);
        }
        if let ParameterKind::Number(numeric) = &parameter.kind {
            ui.indent(parameter.id.raw(), |ui| {
                show_numeric_fields(ui, numeric);
            });
        }
    }
    if let Some(authoring) = app.dynamic_authoring.as_mut() {
        authoring.selected_parameter = selected;
    }
    if let Some(parameter) = selected.and_then(|id| dynamic.parameter(id)).cloned() {
        if let ParameterKind::Number(numeric) = parameter.kind {
            show_parameter_editor(ui, app, &state.block_name, parameter.id, &parameter.name, numeric);
        }
    }
    ui.add_space(8.0);
    ui.label(RichText::new("Direction").small().weak());
    ui.label(format!(
        "Local axis  {:.3}, {:.3}",
        state.direction.x, state.direction.y
    ));
    ui.horizontal(|ui| {
        if ui.button("Pick direction").clicked() {
            if let Some(authoring) = app.dynamic_authoring.as_mut() {
                authoring.pick = AuthoringPick::DirectionStart;
                authoring.status = Some("Specify direction start point".into());
            }
        }
        if ui.button("X").clicked() {
            if let Some(authoring) = app.dynamic_authoring.as_mut() {
                authoring.direction = Point2::new(1.0, 0.0);
            }
        }
        if ui.button("Y").clicked() {
            if let Some(authoring) = app.dynamic_authoring.as_mut() {
                authoring.direction = Point2::new(0.0, 1.0);
            }
        }
    });
    ui.add_space(8.0);
    ui.label(RichText::new("Attach").small().weak());
    ui.label("Select geometry, then attach Move or Stretch.");
    ui.horizontal(|ui| {
        if ui.button("Attach Move").clicked() {
            attach_from_selection(app, BehaviorKind::Move, 1.0);
        }
        if ui.button("Stretch start").clicked() {
            attach_stretch_targets(app, true, false);
        }
        if ui.button("Stretch end").clicked() {
            attach_stretch_targets(app, false, true);
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Follow half").clicked() {
            attach_from_selection(app, BehaviorKind::Move, 0.5);
        }
        if ui.button("Opposite half").clicked() {
            attach_from_selection(app, BehaviorKind::Move, -0.5);
        }
    });
    if !dynamic.behaviors.is_empty() {
        ui.add_space(6.0);
        ui.label(RichText::new("Behaviors").small().weak());
        let mut remove = None;
        for behavior in &dynamic.behaviors {
            let label = match behavior.kind {
                BehaviorKind::Move => "Move",
                BehaviorKind::Stretch => "Stretch",
            };
            ui.horizontal(|ui| {
                ui.label(format!("{label} × {}", behavior.multiplier));
                if ui.small_button("Remove").clicked() {
                    remove = Some(behavior.id);
                }
            });
        }
        if let Some(id) = remove {
            let name = state.block_name.clone();
            if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
                app.history.begin();
                remove_behavior(document, &mut app.history, &name, id);
                app.history.commit_open();
                app.refresh_derived();
            }
        }
    }
    ui.add_space(8.0);
    ui.label(RichText::new("Test").small().weak());
    if let Some(parameter) = selected.and_then(|id| dynamic.parameter(id)) {
        if let ParameterKind::Number(numeric) = &parameter.kind {
            let mut draft = state.test_draft.clone();
            let outcome = numeric_field(ui, "test-value", &mut draft, numeric, state.test_committed);
            if let Some(authoring) = app.dynamic_authoring.as_mut() {
                authoring.test_draft = draft;
                match outcome {
                    NumericEdit::Preview(value) | NumericEdit::Commit(value) => {
                        authoring.test_committed = Some(value);
                    }
                    NumericEdit::Cancel => {
                        authoring.test_committed = None;
                        authoring.test_draft.clear();
                    }
                    NumericEdit::None => {}
                }
            }
            if matches!(outcome, NumericEdit::Preview(_) | NumericEdit::Commit(_) | NumericEdit::Cancel)
            {
                app.refresh_derived();
            }
        }
    } else {
        ui.weak("Select a parameter to test a value.");
    }
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.button("Save").clicked() {
            app.save_active_block();
        }
        if ui.button("Discard").clicked() {
            discard_authoring(app);
            app.block_edit.pop();
            app.refresh_derived();
        }
    });
}

fn show_parameter_editor(
    ui: &mut Ui,
    app: &mut MyCadApp,
    block_name: &str,
    id: ParameterId,
    name: &str,
    numeric: NumericParameter,
) {
    ui.add_space(4.0);
    ui.label(RichText::new("Selected parameter").small().weak());
    let mut name = name.to_string();
    let mut numeric = numeric;
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut name);
    });
    ui.horizontal(|ui| {
        ui.label("Default");
        ui.add(egui::DragValue::new(&mut numeric.default).speed(1.0));
        ui.label("Min");
        let mut min = numeric.min.unwrap_or(0.0);
        let mut has_min = numeric.min.is_some();
        ui.checkbox(&mut has_min, "");
        ui.add_enabled(has_min, egui::DragValue::new(&mut min).speed(1.0));
        numeric.min = has_min.then_some(min);
        ui.label("Max");
        let mut max = numeric.max.unwrap_or(0.0);
        let mut has_max = numeric.max.is_some();
        ui.checkbox(&mut has_max, "");
        ui.add_enabled(has_max, egui::DragValue::new(&mut max).speed(1.0));
        numeric.max = has_max.then_some(max);
    });
    ui.horizontal(|ui| {
        ui.label("Step");
        let mut step = numeric.step.unwrap_or(1.0);
        let mut has_step = numeric.step.is_some();
        ui.checkbox(&mut has_step, "");
        ui.add_enabled(has_step, egui::DragValue::new(&mut step).speed(0.1));
        numeric.step = has_step.then_some(step.max(1e-9));
        let mut required = numeric.step_policy == StepPolicy::RequiredIncrement;
        ui.checkbox(&mut required, "Required increment");
        numeric.step_policy = if required {
            StepPolicy::RequiredIncrement
        } else {
            StepPolicy::IncrementOnly
        };
    });
    if ui.button("Apply parameter").clicked() {
        let block_name = block_name.to_string();
        if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
            app.history.begin();
            apply_parameter_settings(document, &mut app.history, &block_name, id, name, numeric);
            app.history.commit_open();
            app.refresh_derived();
        }
    }
}

fn apply_parameter_settings(
    document: &mut Document,
    history: &mut History,
    block_name: &str,
    id: ParameterId,
    name: String,
    numeric: NumericParameter,
) {
    let Some(block) = document.block_by_name(block_name).cloned() else {
        return;
    };
    let mut after = block.clone();
    let Some(parameter) = after
        .dynamic
        .as_mut()
        .and_then(|dynamic| dynamic.parameters.iter_mut().find(|parameter| parameter.id == id))
    else {
        return;
    };
    parameter.name = name;
    parameter.kind = ParameterKind::Number(numeric);
    if let Some(dynamic) = after.dynamic.as_ref() {
        if cad_core::validate_definition(dynamic, &after.entities).is_err() {
            return;
        }
    }
    document.replace_block_definition(after.clone());
    history.record(Edit::ReplaceBlockDefinition {
        name: block_name.to_string(),
        before: Some(block),
        after: Some(after),
    });
}

fn show_numeric_fields(ui: &mut Ui, numeric: &NumericParameter) {
    ui.monospace(format!(
        "default {}  reference {}  {}  step {:?}",
        numeric.default,
        numeric.reference,
        numeric.unit.label(),
        numeric.step
    ));
}

pub fn attach_from_selection(app: &mut MyCadApp, kind: BehaviorKind, multiplier: f64) {
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
    if ids.is_empty() {
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.status = Some("Select target geometry first".into());
        }
        return;
    }
    let Some(document) = app.document.as_deref() else {
        return;
    };
    let Some(block) = document.block_by_name(&state.block_name) else {
        return;
    };
    let mut targets = Vec::new();
    for id in ids {
        let Some(entity) = block.entities.iter().find(|entity| entity.id == id) else {
            continue;
        };
        match kind {
            BehaviorKind::Move => targets.push(GeometryTarget::Entity(id)),
            BehaviorKind::Stretch => match &entity.geometry {
                Geometry::Line { .. } => {
                    targets.push(GeometryTarget::LineEnd(id));
                }
                _ => {
                    if let Some(authoring) = app.dynamic_authoring.as_mut() {
                        authoring.status =
                            Some("Stretch currently supports LINE endpoints".into());
                    }
                    return;
                }
            },
        }
    }
    if targets.is_empty() {
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.status = Some("Select geometry that belongs to this definition".into());
        }
        return;
    }
    let name = state.block_name.clone();
    let direction = state.direction;
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        app.history.begin();
        match attach_behavior(
            document,
            &mut app.history,
            &name,
            parameter,
            kind,
            targets.clone(),
            direction,
            multiplier,
        ) {
            Ok(()) => {
                app.history.commit_open();
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    authoring.highlight = targets;
                    authoring.status = Some("Behavior attached".into());
                }
                app.refresh_derived();
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

pub fn attach_stretch_targets(app: &mut MyCadApp, start: bool, end: bool) {
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    let ids = app.selection.ids().to_vec();
    let Some(document) = app.document.as_deref() else {
        return;
    };
    let Some(block) = document.block_by_name(&state.block_name) else {
        return;
    };
    let mut targets = Vec::new();
    for id in ids {
        let Some(entity) = block.entities.iter().find(|entity| entity.id == id) else {
            continue;
        };
        match &entity.geometry {
            Geometry::Line { .. } => {
                if start {
                    targets.push(GeometryTarget::LineStart(id));
                }
                if end {
                    targets.push(GeometryTarget::LineEnd(id));
                }
            }
            _ => {
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    authoring.status = Some("Stretch currently supports LINE endpoints".into());
                }
                return;
            }
        }
    }
    commit_targets(app, BehaviorKind::Stretch, targets, 1.0);
}

pub fn attach_stretch_from_click(app: &mut MyCadApp, world: Option<Point2>) {
    let Some(world) = world else {
        attach_stretch_targets(app, false, true);
        return;
    };
    let local = app.block_edit.local_from_world().apply(world);
    let Some(state) = app.dynamic_authoring.clone() else {
        return;
    };
    let ids = app.selection.ids().to_vec();
    let Some(document) = app.document.as_deref() else {
        return;
    };
    let Some(block) = document.block_by_name(&state.block_name) else {
        return;
    };
    let mut targets = Vec::new();
    for id in ids {
        let Some(entity) = block.entities.iter().find(|entity| entity.id == id) else {
            continue;
        };
        match &entity.geometry {
            Geometry::Line { start, end } => {
                let start_pt = Point2::new(start.x, start.y);
                let end_pt = Point2::new(end.x, end.y);
                if start_pt.distance(local) <= end_pt.distance(local) {
                    targets.push(GeometryTarget::LineStart(id));
                } else {
                    targets.push(GeometryTarget::LineEnd(id));
                }
            }
            _ => {
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    authoring.status = Some("Stretch currently supports LINE endpoints".into());
                }
                return;
            }
        }
    }
    commit_targets(app, BehaviorKind::Stretch, targets, 1.0);
}

fn commit_targets(
    app: &mut MyCadApp,
    kind: BehaviorKind,
    targets: Vec<GeometryTarget>,
    multiplier: f64,
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
    if targets.is_empty() {
        if let Some(authoring) = app.dynamic_authoring.as_mut() {
            authoring.status = Some("Select geometry that belongs to this definition".into());
        }
        return;
    }
    let name = state.block_name.clone();
    let direction = state.direction;
    if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
        app.history.begin();
        match attach_behavior(
            document,
            &mut app.history,
            &name,
            parameter,
            kind,
            targets.clone(),
            direction,
            multiplier,
        ) {
            Ok(()) => {
                app.history.commit_open();
                if let Some(authoring) = app.dynamic_authoring.as_mut() {
                    authoring.highlight = targets;
                    authoring.status = Some("Behavior attached".into());
                }
                app.refresh_derived();
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

pub fn handle_authoring_pick(app: &mut MyCadApp, local: Point2) {
    let Some(authoring) = app.dynamic_authoring.as_mut() else {
        return;
    };
    match authoring.pick {
        AuthoringPick::DirectionStart => {
            authoring.direction_start = Some(local);
            authoring.pick = AuthoringPick::DirectionEnd;
            authoring.status = Some("Specify direction end point".into());
        }
        AuthoringPick::DirectionEnd => {
            if let Some(start) = authoring.direction_start {
                let delta = Point2::new(local.x - start.x, local.y - start.y);
                if let Some(dir) = cad_core::normalize_direction(delta) {
                    authoring.direction = dir;
                    authoring.status = Some("Direction set".into());
                } else {
                    authoring.status = Some("Direction must not be zero".into());
                }
            }
            authoring.pick = AuthoringPick::Idle;
            authoring.direction_start = None;
        }
        AuthoringPick::Idle => {}
    }
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumericEdit {
    None,
    Preview(f64),
    Commit(f64),
    Cancel,
}

pub fn numeric_field(
    ui: &mut Ui,
    id: &str,
    draft: &mut String,
    numeric: &NumericParameter,
    committed: Option<f64>,
) -> NumericEdit {
    let mut outcome = NumericEdit::None;
    ui.horizontal(|ui| {
        if ui.button("−").clicked() {
            let current = committed.unwrap_or(numeric.default);
            let next = increment_numeric(numeric, current, -1);
            *draft = format_number(next, numeric.display_precision);
            outcome = NumericEdit::Commit(next);
        }
        let response = ui.add(
            egui::TextEdit::singleline(draft)
                .id_source(id)
                .desired_width(88.0),
        );
        if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            *draft = committed
                .map(|value| format_number(value, numeric.display_precision))
                .unwrap_or_default();
            outcome = NumericEdit::Cancel;
        }
        if response.lost_focus() || (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
        {
            match parse_numeric_draft(draft, numeric) {
                Ok(value) => {
                    *draft = format_number(value, numeric.display_precision);
                    outcome = NumericEdit::Commit(value);
                }
                Err(err) => {
                    ui.colored_label(egui::Color32::from_rgb(220, 90, 70), err);
                }
            }
        } else if response.changed() {
            if let Ok(value) = parse_numeric_draft(draft, numeric) {
                outcome = NumericEdit::Preview(value);
            }
        }
        if ui.button("+").clicked() {
            let current = committed.unwrap_or(numeric.default);
            let next = increment_numeric(numeric, current, 1);
            *draft = format_number(next, numeric.display_precision);
            outcome = NumericEdit::Commit(next);
        }
        let unit = numeric.unit.label();
        if !unit.is_empty() {
            ui.weak(unit);
        }
    });
    outcome
}

pub fn parse_numeric_draft(draft: &str, numeric: &NumericParameter) -> Result<f64, &'static str> {
    let trimmed = draft.trim();
    if trimmed.is_empty() || trimmed == "-" || trimmed == "." || trimmed == "-." {
        return Err("incomplete");
    }
    let value: f64 = trimmed.parse().map_err(|_| "not a number")?;
    if !value.is_finite() {
        return Err("not finite");
    }
    let value = if numeric.step_policy == StepPolicy::RequiredIncrement {
        snap_numeric(numeric, value)
    } else {
        value
    };
    cad_core::validate_numeric_value(ParameterId(0), numeric, value).map_err(|_| "out of range")?;
    Ok(value)
}

fn format_number(value: f64, precision: u8) -> String {
    format!("{value:.prec$}", prec = precision as usize)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

pub fn instance_numeric_value(entity: &Entity, parameter: &ParameterDef) -> f64 {
    entity
        .geometry
        .insert_configuration()
        .and_then(|config| config.get(parameter.id))
        .and_then(ParameterValue::as_number)
        .unwrap_or_else(|| match parameter.default_value() {
            ParameterValue::Number(value) => value,
            _ => 0.0,
        })
}

pub fn show_instance_parameters(ui: &mut Ui, app: &mut MyCadApp, entity: &Entity) {
    let parameters: Vec<ParameterDef> = {
        let Some(document) = app.document.as_deref() else {
            return;
        };
        let Some(name) = entity.geometry.insert_block_name() else {
            return;
        };
        let Some(definition) = document.block_by_name(name) else {
            return;
        };
        let Some(dynamic) = &definition.dynamic else {
            return;
        };
        if dynamic.parameters.is_empty() {
            return;
        }
        dynamic.sorted_parameters().into_iter().cloned().collect()
    };
    ui.add_space(10.0);
    ui.label(RichText::new("Parameters").small().weak());
    let entity_id = entity.id;
    for parameter in &parameters {
        let ParameterKind::Number(numeric) = &parameter.kind else {
            continue;
        };
        ui.label(&parameter.name);
        let current = instance_numeric_value(entity, parameter);
        let draft_key = (entity_id, parameter.id);
        let mut draft = app
            .parameter_drafts
            .get(&draft_key)
            .cloned()
            .unwrap_or_else(|| format_number(current, numeric.display_precision));
        let outcome = numeric_field(
            ui,
            &format!("inst-{}-{}", entity_id.raw(), parameter.id.raw()),
            &mut draft,
            numeric,
            Some(current),
        );
        match outcome {
            NumericEdit::None => {
                app.parameter_drafts.insert(draft_key, draft);
            }
            NumericEdit::Preview(value) => {
                app.parameter_drafts.insert(draft_key, draft);
                let mut config = entity
                    .geometry
                    .insert_configuration()
                    .cloned()
                    .unwrap_or_default();
                config.set(parameter.id, ParameterValue::Number(value));
                app.parameter_previews.insert(entity_id, config);
                app.refresh_derived();
            }
            NumericEdit::Cancel => {
                app.parameter_drafts.remove(&draft_key);
                app.parameter_previews.remove(&entity_id);
                app.refresh_derived();
            }
            NumericEdit::Commit(value) => {
                app.parameter_drafts
                    .insert(draft_key, format_number(value, numeric.display_precision));
                app.parameter_previews.remove(&entity_id);
                if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
                    app.history.begin();
                    match commit_instance_value(
                        document,
                        &mut app.history,
                        entity_id,
                        parameter.id,
                        value,
                    ) {
                        Ok(()) => {
                            app.history.commit_open();
                            app.refresh_derived();
                        }
                        Err(err) => {
                            app.history.commit_open();
                            app.status = err;
                        }
                    }
                }
            }
        }
        if ui.button("Reset to default").clicked() {
            if let ParameterKind::Number(numeric) = &parameter.kind {
                app.parameter_previews.remove(&entity_id);
                if let Some(document) = app.document.as_mut().map(std::sync::Arc::make_mut) {
                    app.history.begin();
                    let _ = commit_instance_value(
                        document,
                        &mut app.history,
                        entity_id,
                        parameter.id,
                        numeric.default,
                    );
                    app.history.commit_open();
                    app.parameter_drafts.remove(&draft_key);
                    app.refresh_derived();
                }
            }
        }
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
            }],
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
        commit_instance_value(&mut document, &mut history, entity_id, param, 5.0).unwrap();
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
}
