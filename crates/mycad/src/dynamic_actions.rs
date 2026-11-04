//! Consistent, undoable inspector for native dynamic actions.

use std::sync::Arc;

use cad_core::{
    ActionId, AnchorId, BehaviorKind, Document, DynamicBehavior, DynamicDefinition, EntityId,
    Geometry, GeometryTarget, ParameterCondition, ParameterId, ParameterKind, PlacementBehavior,
    Point2, ReflectionBehavior, RotationBehavior, RotationSource, TextBinding, TextBindingMode,
    TextReflectPolicy, TextToken, VisibilityGroup,
};
use eframe::egui::{self, Ui};

use crate::app::MyCadApp;
use crate::dynamic_block::{AttachSession, AuthoringPick};
use crate::history::{Edit, History};

/// Keep the dock bounded even for a generated definition with many actions.
const ACTION_ROWS_PER_PAGE: usize = 64;

#[derive(Debug, Clone, Default)]
pub(crate) struct InspectorState {
    draft: Option<ActionDraft>,
    error: Option<String>,
    page: usize,
}

impl InspectorState {
    pub(crate) fn has_draft(&self) -> bool {
        self.draft.is_some()
    }

    pub(crate) fn cancel(&mut self) {
        self.draft = None;
        self.error = None;
    }
}

#[derive(Debug, Clone)]
struct ActionDraft {
    original: Action,
    edited: Action,
    duplicate: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Action {
    Deform(DynamicBehavior),
    Visibility(VisibilityGroup),
    Reflect(ReflectionBehavior),
    Rotate(RotationBehavior),
    Place(PlacementBehavior),
    Text(TextBinding),
}

impl Action {
    fn id(&self) -> ActionId {
        match self {
            Self::Deform(a) => a.id,
            Self::Visibility(a) => a.id,
            Self::Reflect(a) => a.id,
            Self::Rotate(a) => a.id,
            Self::Place(a) => a.id,
            Self::Text(a) => a.id,
        }
    }

    fn set_id(&mut self, id: ActionId) {
        match self {
            Self::Deform(a) => a.id = id,
            Self::Visibility(a) => a.id = id,
            Self::Reflect(a) => a.id = id,
            Self::Rotate(a) => a.id = id,
            Self::Place(a) => a.id = id,
            Self::Text(a) => a.id = id,
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Deform(a) => format!(
                "{} · {}",
                if a.kind == BehaviorKind::Stretch {
                    "Stretch"
                } else {
                    "Move"
                },
                a.name.as_deref().unwrap_or("geometry")
            ),
            Self::Visibility(a) => format!("Visibility · {}", a.name),
            Self::Reflect(a) => format!(
                "Flip / mirror · {}",
                a.name.as_deref().unwrap_or("geometry")
            ),
            Self::Rotate(a) => format!("Rotate · {}", a.name.as_deref().unwrap_or("geometry")),
            Self::Place(a) => format!("Position · {}", a.name.as_deref().unwrap_or("geometry")),
            Self::Text(a) => format!("Linked text · object #{}", a.target.raw()),
        }
    }

    fn as_ref(&self) -> ActionRef<'_> {
        match self {
            Self::Deform(a) => ActionRef::Deform(a),
            Self::Visibility(a) => ActionRef::Visibility(a),
            Self::Reflect(a) => ActionRef::Reflect(a),
            Self::Rotate(a) => ActionRef::Rotate(a),
            Self::Place(a) => ActionRef::Place(a),
            Self::Text(a) => ActionRef::Text(a),
        }
    }

    fn targets(&self) -> Vec<GeometryTarget> {
        match self {
            Self::Deform(a) => a.targets.clone(),
            Self::Visibility(a) => a
                .members
                .iter()
                .copied()
                .map(GeometryTarget::Entity)
                .collect(),
            Self::Reflect(a) => a
                .members
                .iter()
                .copied()
                .map(GeometryTarget::Entity)
                .collect(),
            Self::Rotate(a) => a
                .members
                .iter()
                .copied()
                .map(GeometryTarget::Entity)
                .collect(),
            Self::Place(a) => a
                .members
                .iter()
                .copied()
                .map(GeometryTarget::Entity)
                .collect(),
            Self::Text(a) => vec![GeometryTarget::Entity(a.target)],
        }
    }

    fn replace_targets(&mut self, ids: Vec<EntityId>) -> Result<(), String> {
        if ids.is_empty() {
            return Err("Select objects in this block first.".into());
        }
        match self {
            Self::Deform(a) if a.kind == BehaviorKind::Move => {
                a.targets = ids.into_iter().map(GeometryTarget::Entity).collect()
            }
            Self::Deform(_) => return Err("Use Edit targets to choose stretch endpoints.".into()),
            Self::Visibility(a) => a.members = ids,
            Self::Reflect(a) => a.members = ids,
            Self::Rotate(a) => a.members = ids,
            Self::Place(a) => a.members = ids,
            Self::Text(a) if ids.len() == 1 => {
                if let Some(id) = ids.first() {
                    a.target = *id;
                }
            }
            Self::Text(_) => return Err("Select one TEXT or MTEXT object for this binding.".into()),
        }
        Ok(())
    }

    fn name_duplicate(&mut self) {
        match self {
            Self::Deform(a) => {
                a.name = Some(format!("Copy of {}", a.name.as_deref().unwrap_or("action")))
            }
            Self::Visibility(a) => a.name = format!("Copy of {}", a.name),
            Self::Reflect(a) => {
                a.name = Some(format!("Copy of {}", a.name.as_deref().unwrap_or("Flip")))
            }
            Self::Rotate(a) => {
                a.name = Some(format!("Copy of {}", a.name.as_deref().unwrap_or("Rotate")))
            }
            Self::Place(a) => {
                a.name = Some(format!(
                    "Copy of {}",
                    a.name.as_deref().unwrap_or("Position")
                ))
            }
            Self::Text(_) => {}
        }
    }

    fn is_transform(&self) -> bool {
        matches!(self, Self::Reflect(_) | Self::Rotate(_) | Self::Place(_))
    }

    fn append(self, dynamic: &mut DynamicDefinition) {
        match self {
            Self::Deform(a) => dynamic.behaviors.push(a),
            Self::Visibility(a) => dynamic.visibility.push(a),
            Self::Reflect(a) => dynamic.reflections.push(a),
            Self::Rotate(a) => dynamic.rotations.push(a),
            Self::Place(a) => dynamic.placements.push(a),
            Self::Text(a) => dynamic.text_bindings.push(a),
        }
    }
}

#[derive(Clone, Copy)]
enum ActionRef<'a> {
    Deform(&'a DynamicBehavior),
    Visibility(&'a VisibilityGroup),
    Reflect(&'a ReflectionBehavior),
    Rotate(&'a RotationBehavior),
    Place(&'a PlacementBehavior),
    Text(&'a TextBinding),
}

impl ActionRef<'_> {
    fn id(self) -> ActionId {
        match self {
            Self::Deform(a) => a.id,
            Self::Visibility(a) => a.id,
            Self::Reflect(a) => a.id,
            Self::Rotate(a) => a.id,
            Self::Place(a) => a.id,
            Self::Text(a) => a.id,
        }
    }

    fn to_owned(self) -> Action {
        match self {
            Self::Deform(a) => Action::Deform(a.clone()),
            Self::Visibility(a) => Action::Visibility(a.clone()),
            Self::Reflect(a) => Action::Reflect(a.clone()),
            Self::Rotate(a) => Action::Rotate(a.clone()),
            Self::Place(a) => Action::Place(a.clone()),
            Self::Text(a) => Action::Text(a.clone()),
        }
    }

    fn label(self) -> String {
        match self {
            Self::Deform(a) => format!(
                "{} · {}",
                if a.kind == BehaviorKind::Stretch {
                    "Stretch"
                } else {
                    "Move"
                },
                a.name.as_deref().unwrap_or("geometry")
            ),
            Self::Visibility(a) => format!("Visibility · {}", a.name),
            Self::Reflect(a) => format!(
                "Flip / mirror · {}",
                a.name.as_deref().unwrap_or("geometry")
            ),
            Self::Rotate(a) => format!("Rotate · {}", a.name.as_deref().unwrap_or("geometry")),
            Self::Place(a) => format!("Position · {}", a.name.as_deref().unwrap_or("geometry")),
            Self::Text(a) => format!("Linked text · object #{}", a.target.raw()),
        }
    }

    fn uses(&self, parameter: ParameterId) -> bool {
        match self {
            Self::Deform(a) => a.parameter == parameter,
            Self::Visibility(a) => a.conditions.iter().any(|c| c.parameter() == parameter),
            Self::Reflect(a) => a.condition.parameter() == parameter,
            Self::Rotate(a) => match &a.source {
                RotationSource::AngleParameter(id)
                | RotationSource::OptionMap { parameter: id, .. } => *id == parameter,
            },
            Self::Place(a) => a.parameter == parameter,
            Self::Text(a) => match &a.mode {
                TextBindingMode::ShowValue { parameter: id }
                | TextBindingMode::OptionMap { parameter: id, .. } => *id == parameter,
                TextBindingMode::Formatted { tokens } => tokens
                    .iter()
                    .any(|t| matches!(t, TextToken::Parameter(id) if *id == parameter)),
            },
        }
    }
}

fn all_actions(dynamic: &DynamicDefinition) -> impl Iterator<Item = ActionRef<'_>> {
    dynamic
        .behaviors
        .iter()
        .map(ActionRef::Deform)
        .chain(dynamic.visibility.iter().map(ActionRef::Visibility))
        .chain(dynamic.reflections.iter().map(ActionRef::Reflect))
        .chain(dynamic.rotations.iter().map(ActionRef::Rotate))
        .chain(dynamic.placements.iter().map(ActionRef::Place))
        .chain(dynamic.text_bindings.iter().map(ActionRef::Text))
}

fn find_action(dynamic: &DynamicDefinition, id: ActionId) -> Option<Action> {
    all_actions(dynamic)
        .find(|a| a.id() == id)
        .map(ActionRef::to_owned)
}

fn remove_action(dynamic: &mut DynamicDefinition, id: ActionId) {
    dynamic.behaviors.retain(|a| a.id != id);
    dynamic.visibility.retain(|a| a.id != id);
    dynamic.reflections.retain(|a| a.id != id);
    dynamic.rotations.retain(|a| a.id != id);
    dynamic.placements.retain(|a| a.id != id);
    dynamic.text_bindings.retain(|a| a.id != id);
    dynamic.transform_order.retain(|other| *other != id);
}

fn select_action(app: &mut MyCadApp, action: &Action) {
    let targets = action.targets();
    app.selection
        .replace_all(targets.iter().copied().map(GeometryTarget::entity_id));
    if let Some(state) = app.dynamic_authoring.as_mut() {
        state.selected_behavior = Some(action.id());
        state.highlight = targets;
    }
}

pub(crate) fn show(
    ui: &mut Ui,
    app: &mut MyCadApp,
    dynamic: &DynamicDefinition,
    parameter: ParameterId,
) {
    ui.heading("Connected actions");
    let actions: Vec<_> = all_actions(dynamic).filter(|a| a.uses(parameter)).collect();
    if actions.is_empty() {
        ui.weak("No actions yet. Choose what this parameter changes below.");
        return;
    }
    let page_count = actions.len().div_ceil(ACTION_ROWS_PER_PAGE);
    let mut page = app
        .dynamic_authoring
        .as_ref()
        .map(|s| s.action_inspector.page)
        .unwrap_or(0)
        .min(page_count.saturating_sub(1));
    if page_count > 1 {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(page > 0, egui::Button::new("Previous actions"))
                .clicked()
            {
                page = page.saturating_sub(1);
            }
            ui.label(format!("{} / {page_count}", page + 1));
            if ui
                .add_enabled(page + 1 < page_count, egui::Button::new("Next actions"))
                .clicked()
            {
                page += 1;
            }
        });
    }
    if let Some(s) = app.dynamic_authoring.as_mut() {
        s.action_inspector.page = page;
    }
    for action in actions
        .iter()
        .skip(page * ACTION_ROWS_PER_PAGE)
        .take(ACTION_ROWS_PER_PAGE)
    {
        ui.push_id(action.id().raw(), |ui| {
            let selected = app
                .dynamic_authoring
                .as_ref()
                .is_some_and(|s| s.selected_behavior == Some(action.id()));
            if ui
                .selectable_label(selected, action.label())
                .on_hover_text("Highlight this action's objects and reference geometry")
                .clicked()
            {
                select_action(app, &(*action).to_owned());
            }
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("Edit").clicked() {
                    select_action(app, &(*action).to_owned());
                    if let Some(s) = app.dynamic_authoring.as_mut() {
                        s.action_inspector.draft = Some(ActionDraft {
                            original: (*action).to_owned(),
                            edited: (*action).to_owned(),
                            duplicate: false,
                        });
                        s.action_inspector.error = None;
                    }
                }
                if ui.small_button("Select targets").clicked() {
                    select_action(app, &(*action).to_owned());
                }
                if let ActionRef::Deform(a) = action {
                    if a.kind == BehaviorKind::Stretch && ui.small_button("Edit targets").clicked()
                    {
                        select_action(app, &(*action).to_owned());
                        if let Some(s) = app.dynamic_authoring.as_mut() {
                            s.attach = Some(AttachSession {
                                kind: a.kind,
                                parameter: a.parameter,
                                follow: a.follow,
                                handles: a.targets.clone(),
                                unsupported: Vec::new(),
                                window: None,
                                replace: Some(a.id),
                            });
                            s.pick = AuthoringPick::Idle;
                            s.status = Some(
                                "Choose stretch endpoints, then connect to the parameter.".into(),
                            );
                        }
                    }
                }
                if ui.small_button("Duplicate").clicked() {
                    select_action(app, &(*action).to_owned());
                    if let Some(s) = app.dynamic_authoring.as_mut() {
                        let mut edited = (*action).to_owned();
                        edited.name_duplicate();
                        s.action_inspector.draft = Some(ActionDraft {
                            original: (*action).to_owned(),
                            edited,
                            duplicate: true,
                        });
                        s.action_inspector.error = None;
                    }
                }
                if ui.small_button("Remove").clicked() {
                    match commit(app, &(*action).to_owned(), None, false) {
                        Ok(_) => {
                            if let Some(s) = app.dynamic_authoring.as_mut() {
                                s.selected_behavior = None;
                                s.highlight.clear();
                            }
                        }
                        Err(error) => report_error(app, error),
                    }
                }
            });
            ui.separator();
        });
    }
}

pub(crate) fn show_editor(ui: &mut Ui, app: &mut MyCadApp, dynamic: &DynamicDefinition) {
    let Some(mut draft) = app
        .dynamic_authoring
        .as_mut()
        .and_then(|s| s.action_inspector.draft.take())
    else {
        return;
    };
    ui.heading(if draft.duplicate {
        "Duplicate action"
    } else {
        "Edit action"
    });
    ui.label(draft.edited.label());
    ui.weak("Coordinates are local to the block, in drawing units. Changes are applied together.");
    edit_fields(ui, &mut draft.edited, dynamic);
    let target_count = match &draft.edited {
        Action::Deform(a) => a.targets.len(),
        Action::Visibility(a) => a.members.len(),
        Action::Reflect(a) => a.members.len(),
        Action::Rotate(a) => a.members.len(),
        Action::Place(a) => a.members.len(),
        Action::Text(_) => 1,
    };
    ui.label(format!("{target_count} geometry targets"));
    if !matches!(&draft.edited, Action::Deform(a) if a.kind == BehaviorKind::Stretch) {
        if ui.button("Use selected objects as targets").clicked() {
            let result = selection_in_block(app).and_then(|ids| {
                if matches!(draft.edited, Action::Text(_)) {
                    let is_text = ids.len() == 1
                        && ids
                            .first()
                            .and_then(|id| app.document.as_deref()?.entity_by_id(*id))
                            .is_some_and(|e| {
                                matches!(e.geometry, Geometry::Text(_) | Geometry::MText(_))
                            });
                    if !is_text {
                        return Err("Select one TEXT or MTEXT object in this block.".into());
                    }
                }
                draft.edited.replace_targets(ids)
            });
            if let Err(error) = result {
                report_error(app, error);
            } else if let Some(s) = app.dynamic_authoring.as_mut() {
                s.action_inspector.error = None;
                s.highlight = draft.edited.targets();
            }
        }
    } else {
        ui.weak(
            "To change stretch endpoints, apply or cancel this edit, then choose Edit targets.",
        );
    }
    if draft.duplicate && draft.edited.is_transform() {
        ui.weak("A duplicate is an additional action. Choose new targets or settings if the geometry should not transform twice.");
    }
    if draft.duplicate && matches!(draft.edited, Action::Place(_) | Action::Text(_)) {
        ui.weak("Choose different targets before saving this duplicate; these actions have one owner per object.");
    }
    if let Some(error) = app
        .dynamic_authoring
        .as_ref()
        .and_then(|s| s.action_inspector.error.as_ref())
    {
        ui.colored_label(egui::Color32::from_rgb(240, 150, 100), error);
    }
    let mut apply = false;
    let mut cancel = false;
    ui.horizontal_wrapped(|ui| {
        apply = ui
            .button(if draft.duplicate {
                "Create duplicate"
            } else {
                "Apply action changes"
            })
            .clicked();
        cancel = ui.button("Cancel action edit").clicked();
    });
    if cancel {
        if let Some(s) = app.dynamic_authoring.as_mut() {
            s.action_inspector.cancel();
            s.highlight.clear();
        }
        return;
    }
    if apply {
        match commit(
            app,
            &draft.original,
            Some(draft.edited.clone()),
            draft.duplicate,
        ) {
            Ok(id) => {
                if let Some(s) = app.dynamic_authoring.as_mut() {
                    s.action_inspector.cancel();
                    s.selected_behavior = id;
                    s.highlight = draft.edited.targets();
                }
                return;
            }
            Err(error) => report_error(app, error),
        }
    }
    if let Some(s) = app.dynamic_authoring.as_mut() {
        s.action_inspector.draft = Some(draft);
    }
}

fn report_error(app: &mut MyCadApp, error: String) {
    app.status = error.clone();
    if let Some(s) = app.dynamic_authoring.as_mut() {
        s.action_inspector.error = Some(error.clone());
        s.status = Some(error);
    }
}

fn selection_in_block(app: &MyCadApp) -> Result<Vec<EntityId>, String> {
    let name = app
        .dynamic_authoring
        .as_ref()
        .map(|s| s.block_name.as_str())
        .ok_or("Open the block definition first.")?;
    let document = app.document.as_deref().ok_or("No drawing is open.")?;
    let ids = app.selection.ids();
    if ids.is_empty() {
        return Err("Select objects in this block first.".into());
    }
    if ids.iter().any(|id| {
        document
            .entity_location(*id)
            .is_none_or(|location| !location.space.same_block(name))
    }) {
        return Err("Every target must belong to the block being edited.".into());
    }
    Ok(ids.to_vec())
}

fn commit(
    app: &mut MyCadApp,
    original: &Action,
    edited: Option<Action>,
    duplicate: bool,
) -> Result<Option<ActionId>, String> {
    if app.io_busy()
        || app
            .document
            .as_ref()
            .is_some_and(|document| Arc::strong_count(document) != 1)
    {
        return Err("Wait for the current preview or save to finish, then apply again.".into());
    }
    let name = app
        .dynamic_authoring
        .as_ref()
        .map(|s| s.block_name.clone())
        .ok_or("Open the block definition first.")?;
    let document = app
        .document
        .as_mut()
        .and_then(Arc::get_mut)
        .ok_or("The drawing is busy; try again when the current operation finishes.")?;
    let id = apply_change(
        document,
        &mut app.history,
        &name,
        original,
        edited,
        duplicate,
    )?;
    let message = if duplicate {
        "Action duplicated"
    } else if id.is_none() {
        "Action removed"
    } else {
        "Action changes applied"
    };
    app.status = message.into();
    if let Some(s) = app.dynamic_authoring.as_mut() {
        s.status = Some(message.into());
    }
    app.request_preview();
    Ok(id)
}

fn apply_change(
    document: &mut Document,
    history: &mut History,
    name: &str,
    original: &Action,
    mut edited: Option<Action>,
    duplicate: bool,
) -> Result<Option<ActionId>, String> {
    let before = document
        .block_by_name(name)
        .and_then(|b| b.dynamic.clone())
        .ok_or("The dynamic definition no longer exists.")?;
    if find_action(&before, original.id()).as_ref() != Some(original) {
        return Err("This action changed while the editor was open. Cancel and reopen it to edit the latest version.".into());
    }
    if !duplicate && edited.as_ref() == Some(original) {
        return Ok(Some(original.id()));
    }
    let mut after = before.clone();
    let mut id = None;
    if let Some(mut action) = edited.take() {
        if action.targets().is_empty() {
            return Err("An action needs at least one geometry target.".into());
        }
        let assigned = if duplicate {
            document.allocate_action_id()
        } else {
            original.id()
        };
        action.set_id(assigned);
        if duplicate {
            if action.is_transform() {
                if after.transform_order.is_empty() {
                    after
                        .transform_order
                        .extend(before.reflections.iter().map(|a| a.id));
                    after
                        .transform_order
                        .extend(before.rotations.iter().map(|a| a.id));
                    after
                        .transform_order
                        .extend(before.placements.iter().map(|a| a.id));
                }
                let at = after
                    .transform_order
                    .iter()
                    .position(|item| *item == original.id())
                    .map(|index| index + 1)
                    .unwrap_or(after.transform_order.len());
                after.transform_order.insert(at, assigned);
            }
            action.append(&mut after);
        } else {
            // Preserve vector order as well as transform_order: an empty explicit
            // order means the vectors themselves define the transform sequence.
            replace_action(&mut after, action)?;
        }
        id = Some(assigned);
    } else {
        remove_action(&mut after, original.id());
    }
    let entities = &document
        .block_by_name(name)
        .ok_or("The block no longer exists.")?
        .entities;
    cad_core::validate_definition(&after, entities).map_err(|error| error.to_string())?;
    if !document.replace_dynamic_definition(name, Some(after.clone())) {
        return Ok(id);
    }
    history.begin();
    history.record(Edit::ReplaceDynamicDefinition {
        name: name.into(),
        before: Some(before),
        after: Some(after),
    });
    history.commit_open();
    Ok(id)
}

fn replace_action(dynamic: &mut DynamicDefinition, action: Action) -> Result<(), String> {
    macro_rules! replace {
        ($items:expr, $value:expr) => {{
            let value = $value;
            let slot = $items
                .iter_mut()
                .find(|a| a.id == value.id)
                .ok_or("The action no longer exists.")?;
            *slot = value;
        }};
    }
    match action {
        Action::Deform(a) => replace!(dynamic.behaviors, a),
        Action::Visibility(a) => replace!(dynamic.visibility, a),
        Action::Reflect(a) => replace!(dynamic.reflections, a),
        Action::Rotate(a) => replace!(dynamic.rotations, a),
        Action::Place(a) => replace!(dynamic.placements, a),
        Action::Text(a) => replace!(dynamic.text_bindings, a),
    }
    Ok(())
}

fn edit_fields(ui: &mut Ui, action: &mut Action, dynamic: &DynamicDefinition) {
    match action {
        Action::Deform(a) => {
            optional_name(ui, &mut a.name);
            parameter_picker(ui, "deform-param", dynamic, &mut a.parameter, |kind| {
                matches!(kind, ParameterKind::Number(_))
            });
            point_editor(ui, "Direction", &mut a.local_direction);
            ui.horizontal_wrapped(|ui| {
                ui.label("Reference value");
                ui.add(egui::DragValue::new(&mut a.reference_value).speed(1.0));
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Multiplier");
                if ui
                    .add(egui::DragValue::new(&mut a.multiplier).speed(0.1))
                    .changed()
                {
                    a.follow = cad_core::FollowRole::Custom;
                }
            });
            ui.weak("Displacement = (value − reference) × direction × multiplier.");
        }
        Action::Visibility(a) => {
            ui.label("Action name");
            ui.text_edit_singleline(&mut a.name);
            ui.weak("Show these objects only when every condition below matches.");
            let mut remove = None;
            let len = a.conditions.len();
            for (index, condition) in a.conditions.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    condition_editor(ui, dynamic, condition);
                    if ui
                        .add_enabled(len > 1, egui::Button::new("Remove condition"))
                        .clicked()
                    {
                        remove = Some(index);
                    }
                });
            }
            if let Some(index) = remove {
                a.conditions.remove(index);
            }
            if ui.button("Add condition").clicked() {
                if let Some(condition) = dynamic
                    .parameters
                    .iter()
                    .find_map(|p| default_condition(p.id, &p.kind))
                {
                    a.conditions.push(condition);
                }
            }
        }
        Action::Reflect(a) => {
            optional_name(ui, &mut a.name);
            condition_editor(ui, dynamic, &mut a.condition);
            point_editor(ui, "Axis start", &mut a.axis_a);
            point_editor(ui, "Axis end", &mut a.axis_b);
            ui.radio_value(
                &mut a.text_policy,
                TextReflectPolicy::KeepReadable,
                "Keep text readable",
            );
            ui.radio_value(
                &mut a.text_policy,
                TextReflectPolicy::KeepUpright,
                "Keep text upright",
            );
            ui.radio_value(
                &mut a.text_policy,
                TextReflectPolicy::Mirror,
                "Mirror text glyphs",
            );
        }
        Action::Rotate(a) => {
            optional_name(ui, &mut a.name);
            point_editor(ui, "Pivot", &mut a.pivot);
            match &mut a.source {
                RotationSource::AngleParameter(parameter) => {
                    parameter_picker(ui, "rotate-angle", dynamic, parameter, |kind| {
                        matches!(kind, ParameterKind::Number(_))
                    });
                    ui.weak("Uses the selected parameter's stored angle unit.");
                }
                RotationSource::OptionMap { parameter, angles } => {
                    parameter_picker(ui, "rotate-choice", dynamic, parameter, |kind| {
                        matches!(kind, ParameterKind::Choice(_))
                    });
                    if let Some(ParameterKind::Choice(choice)) =
                        dynamic.parameter(*parameter).map(|p| &p.kind)
                    {
                        angles.retain(|id, _| choice.options.iter().any(|option| option.id == *id));
                        for option in &choice.options {
                            angle_editor(ui, &option.label, angles.entry(option.id).or_insert(0.0));
                        }
                    }
                }
            }
        }
        Action::Place(a) => {
            optional_name(ui, &mut a.name);
            point_editor(ui, "Attachment", &mut a.attachment);
            angle_editor(ui, "Attachment angle", &mut a.attachment_angle);
            // Preserve the source and mappings, including Boolean placement.
            ui.label(format!(
                "Controlled by {}",
                parameter_label(dynamic, a.parameter)
            ));
            if let Some(ParameterKind::Choice(choice)) =
                dynamic.parameter(a.parameter).map(|p| &p.kind)
            {
                for option in &choice.options {
                    if let Some(anchor) = a.destinations.get_mut(&option.id) {
                        anchor_picker(ui, &option.label, dynamic, anchor);
                    }
                }
            }
            if let Some((off, on)) = a.boolean_destinations.as_mut() {
                anchor_picker(ui, "Off destination", dynamic, off);
                anchor_picker(ui, "On destination", dynamic, on);
            }
            ui.weak("Destinations reference the block's named anchors; other actions using an anchor are unchanged.");
        }
        Action::Text(a) => {
            ui.weak("The target text identifies this action.");
            match &mut a.mode {
                TextBindingMode::ShowValue { parameter } => {
                    parameter_picker(ui, "text-value", dynamic, parameter, |_| true)
                }
                TextBindingMode::OptionMap { parameter, texts } => {
                    parameter_picker(ui, "text-option", dynamic, parameter, |kind| {
                        matches!(kind, ParameterKind::Choice(_))
                    });
                    if let Some(ParameterKind::Choice(choice)) =
                        dynamic.parameter(*parameter).map(|p| &p.kind)
                    {
                        texts.retain(|id, _| choice.options.iter().any(|option| option.id == *id));
                        for option in &choice.options {
                            ui.label(&option.label);
                            ui.text_edit_singleline(
                                texts
                                    .entry(option.id)
                                    .or_insert_with(|| option.label.clone()),
                            );
                        }
                    }
                }
                TextBindingMode::Formatted { tokens } => {
                    ui.weak("Text pieces are joined in this order. Parameter references remain linked when renamed.");
                    let mut remove = None;
                    for (index, token) in tokens.iter_mut().enumerate() {
                        ui.push_id(index, |ui| {
                            match token {
                                TextToken::Literal(text) => {
                                    ui.text_edit_singleline(text);
                                }
                                TextToken::Parameter(parameter) => {
                                    parameter_picker(ui, "token", dynamic, parameter, |_| true)
                                }
                            }
                            if ui.small_button("Remove piece").clicked() {
                                remove = Some(index);
                            }
                        });
                    }
                    if let Some(index) = remove {
                        tokens.remove(index);
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Add text").clicked() {
                            tokens.push(TextToken::Literal(String::new()));
                        }
                        if ui.button("Add parameter").clicked() {
                            if let Some(parameter) = dynamic.parameters.first() {
                                tokens.push(TextToken::Parameter(parameter.id));
                            }
                        }
                    });
                }
            }
            ui.label("On label");
            ui.text_edit_singleline(&mut a.boolean_true);
            ui.label("Off label");
            ui.text_edit_singleline(&mut a.boolean_false);
            let mut custom = a.number_precision.is_some();
            if ui
                .checkbox(&mut custom, "Override number precision")
                .changed()
            {
                a.number_precision = custom.then_some(2);
            }
            if let Some(precision) = a.number_precision.as_mut() {
                ui.add(egui::DragValue::new(precision).range(0..=12));
            }
        }
    }
}

fn optional_name(ui: &mut Ui, name: &mut Option<String>) {
    ui.label("Action name");
    let mut text = name.clone().unwrap_or_default();
    if ui.text_edit_singleline(&mut text).changed() {
        *name = (!text.trim().is_empty()).then_some(text);
    }
}

fn point_editor(ui: &mut Ui, label: &str, point: &mut Point2) {
    ui.label(label);
    ui.push_id(label, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::DragValue::new(&mut point.x).prefix("X: ").speed(1.0));
            ui.add(egui::DragValue::new(&mut point.y).prefix("Y: ").speed(1.0));
        });
    });
}

fn angle_editor(ui: &mut Ui, label: &str, radians: &mut f64) {
    let mut degrees = radians.to_degrees();
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        if ui
            .add(egui::DragValue::new(&mut degrees).suffix("°").speed(1.0))
            .changed()
        {
            *radians = degrees.to_radians();
        }
    });
}

fn parameter_label(dynamic: &DynamicDefinition, id: ParameterId) -> String {
    dynamic
        .parameter(id)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "Missing parameter".into())
}

fn parameter_picker(
    ui: &mut Ui,
    salt: &str,
    dynamic: &DynamicDefinition,
    id: &mut ParameterId,
    valid: impl Fn(&ParameterKind) -> bool,
) {
    egui::ComboBox::from_id_salt(salt)
        .selected_text(parameter_label(dynamic, *id))
        .show_ui(ui, |ui| {
            for parameter in dynamic
                .sorted_parameters()
                .into_iter()
                .filter(|p| valid(&p.kind))
            {
                ui.selectable_value(id, parameter.id, &parameter.name);
            }
        });
}

fn default_condition(id: ParameterId, kind: &ParameterKind) -> Option<ParameterCondition> {
    match kind {
        ParameterKind::Boolean(_) => Some(ParameterCondition::Boolean {
            parameter: id,
            state: true,
        }),
        ParameterKind::Choice(choice) => Some(ParameterCondition::Choice {
            parameter: id,
            options: vec![choice.default],
        }),
        _ => None,
    }
}

fn condition_editor(ui: &mut Ui, dynamic: &DynamicDefinition, condition: &mut ParameterCondition) {
    let mut id = condition.parameter();
    parameter_picker(ui, "condition-param", dynamic, &mut id, |kind| {
        matches!(kind, ParameterKind::Choice(_) | ParameterKind::Boolean(_))
    });
    if id != condition.parameter() {
        if let Some(replacement) = dynamic
            .parameter(id)
            .and_then(|p| default_condition(p.id, &p.kind))
        {
            *condition = replacement;
        }
    }
    match condition {
        ParameterCondition::Boolean { state, .. } => {
            ui.checkbox(state, "When On (unchecked means Off)");
        }
        ParameterCondition::Choice { parameter, options } => {
            if let Some(ParameterKind::Choice(choice)) =
                dynamic.parameter(*parameter).map(|p| &p.kind)
            {
                for option in &choice.options {
                    let mut included = options.contains(&option.id);
                    if ui.checkbox(&mut included, &option.label).changed() {
                        if included {
                            options.push(option.id);
                        } else {
                            options.retain(|id| *id != option.id);
                        }
                    }
                }
            }
        }
    }
}

fn anchor_picker(ui: &mut Ui, label: &str, dynamic: &DynamicDefinition, anchor: &mut AnchorId) {
    ui.label(label);
    let text = dynamic
        .anchor(*anchor)
        .map(|a| format!("{} ({:.2}, {:.2})", a.name, a.position.x, a.position.y))
        .unwrap_or_else(|| "Missing anchor".into());
    egui::ComboBox::from_id_salt(label)
        .selected_text(text)
        .show_ui(ui, |ui| {
            for a in &dynamic.anchors {
                ui.selectable_value(anchor, a.id, &a.name);
            }
        });
}

/// Pivot / axis / attachment guides for the selected or currently edited action.
pub(crate) fn reference_guides(app: &MyCadApp) -> Vec<(Point2, Option<Point2>)> {
    let Some(state) = app.dynamic_authoring.as_ref() else {
        return Vec::new();
    };
    let action = state
        .action_inspector
        .draft
        .as_ref()
        .map(|draft| draft.edited.as_ref())
        .or_else(|| {
            let dynamic = app
                .document
                .as_deref()?
                .block_by_name(&state.block_name)?
                .dynamic
                .as_ref()?;
            all_actions(dynamic).find(|action| Some(action.id()) == state.selected_behavior)
        });
    match action {
        Some(ActionRef::Reflect(a)) => vec![(a.axis_a, Some(a.axis_b))],
        Some(ActionRef::Rotate(a)) => vec![(a.pivot, None)],
        Some(ActionRef::Place(a)) => vec![(a.attachment, None)],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_core::{
        AnchorDef, BlockDefinition, BooleanParameter, ChoiceOption, ChoiceParameter, Entity,
        NumericParameter, OptionId, ParameterDef, Point3,
    };
    use std::collections::BTreeMap;

    fn fixture() -> Document {
        let mut document = Document::default();
        let mut first = Entity::new(Geometry::Line {
            start: Point3::default(),
            end: Point3::from_xy(10.0, 0.0),
        });
        first.id = EntityId(1);
        let mut second = first.clone();
        second.id = EntityId(2);
        let mut text = Entity::new(Geometry::Text(cad_core::TextData::default()));
        text.id = EntityId(3);
        let mut dynamic = DynamicDefinition {
            parameters: vec![
                ParameterDef::number(ParameterId(1), "Angle", NumericParameter::length(0.0)),
                ParameterDef::choice(
                    ParameterId(2),
                    "Hand",
                    ChoiceParameter {
                        options: vec![
                            ChoiceOption {
                                id: OptionId(1),
                                label: "Left".into(),
                            },
                            ChoiceOption {
                                id: OptionId(2),
                                label: "Right".into(),
                            },
                        ],
                        default: OptionId(1),
                    },
                ),
                ParameterDef::boolean(ParameterId(3), "Enabled", BooleanParameter::default()),
            ],
            ..Default::default()
        };
        dynamic.rotations = vec![
            RotationBehavior {
                id: ActionId(1),
                name: Some("First".into()),
                members: vec![EntityId(1)],
                pivot: Point2::new(0.0, 0.0),
                source: RotationSource::OptionMap {
                    parameter: ParameterId(2),
                    angles: BTreeMap::from([(OptionId(1), 0.0), (OptionId(2), 1.5)]),
                },
            },
            RotationBehavior {
                id: ActionId(2),
                name: Some("Second".into()),
                members: vec![EntityId(1)],
                pivot: Point2::new(5.0, 0.0),
                source: RotationSource::AngleParameter(ParameterId(1)),
            },
        ];
        dynamic.visibility.push(VisibilityGroup {
            id: ActionId(3),
            name: "Parts".into(),
            members: vec![EntityId(1)],
            conditions: vec![
                ParameterCondition::Choice {
                    parameter: ParameterId(2),
                    options: vec![OptionId(1)],
                },
                ParameterCondition::Boolean {
                    parameter: ParameterId(3),
                    state: true,
                },
            ],
        });
        dynamic.anchors = vec![
            AnchorDef {
                id: AnchorId(1),
                name: "Off".into(),
                position: Point2::new(0.0, 0.0),
                orientation: Some(0.2),
                follow: None,
            },
            AnchorDef {
                id: AnchorId(2),
                name: "On".into(),
                position: Point2::new(8.0, 2.0),
                orientation: Some(0.3),
                follow: None,
            },
        ];
        dynamic.placements.push(PlacementBehavior {
            id: ActionId(4),
            name: Some("Boolean placement".into()),
            members: vec![EntityId(2)],
            attachment: Point2::new(1.0, 2.0),
            attachment_angle: 0.4,
            parameter: ParameterId(3),
            destinations: BTreeMap::new(),
            boolean_destinations: Some((AnchorId(1), AnchorId(2))),
        });
        dynamic.text_bindings.push(TextBinding {
            id: ActionId(5),
            target: EntityId(3),
            mode: TextBindingMode::Formatted {
                tokens: vec![
                    TextToken::Literal("Type: ".into()),
                    TextToken::Parameter(ParameterId(2)),
                    TextToken::Literal(" / ".into()),
                    TextToken::Parameter(ParameterId(3)),
                ],
            },
            number_precision: Some(3),
            ..Default::default()
        });
        let mut block =
            BlockDefinition::plain("Part", Point3::default(), vec![first, second, text]);
        block.dynamic = Some(dynamic);
        document.blocks.insert("Part".into(), block);
        document.assign_missing_ids();
        let block = document.block_by_name("Part").unwrap();
        cad_core::validate_definition(block.dynamic.as_ref().unwrap(), &block.entities).unwrap();
        document
    }

    fn definition(document: &Document) -> &DynamicDefinition {
        document
            .block_by_name("Part")
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
    }

    #[test]
    fn action_edit_preserves_implicit_order_option_mapping_and_geometry_storage() {
        let mut document = fixture();
        let mut history = History::default();
        let storage = document.block_by_name("Part").unwrap().entities.as_ptr();
        let generation = document.content_generation();
        let revision = document.block_by_name("Part").unwrap().content_revision;
        let original = find_action(definition(&document), ActionId(1)).unwrap();
        let mut edited = original.clone();
        if let Action::Rotate(action) = &mut edited {
            action.name = Some("Door swing".into());
            action.pivot.x = 3.0;
        }
        apply_change(
            &mut document,
            &mut history,
            "part",
            &original,
            Some(edited.clone()),
            false,
        )
        .unwrap();
        assert_eq!(
            find_action(definition(&document), ActionId(1)),
            Some(edited)
        );
        assert_eq!(
            definition(&document)
                .rotations
                .iter()
                .map(|a| a.id)
                .collect::<Vec<_>>(),
            vec![ActionId(1), ActionId(2)]
        );
        assert!(definition(&document).transform_order.is_empty());
        assert_eq!(
            document.block_by_name("Part").unwrap().entities.as_ptr(),
            storage
        );
        assert_eq!(document.content_generation(), generation + 1);
        assert_eq!(
            document.block_by_name("Part").unwrap().content_revision,
            revision + 1
        );
        assert!(document.entity_index_is_consistent());
        assert_eq!(history.undo_len(), 1);
        assert!(history.undo(&mut document));
        assert_eq!(
            find_action(definition(&document), ActionId(1)),
            Some(original)
        );
        assert_eq!(
            document.block_by_name("Part").unwrap().entities.as_ptr(),
            storage
        );
        assert!(history.redo(&mut document));
        assert_eq!(definition(&document).rotations[0].pivot.x, 3.0);
    }

    #[test]
    fn duplicate_and_remove_keep_transform_order_valid_and_undoable() {
        let mut document = fixture();
        let mut history = History::default();
        let original = find_action(definition(&document), ActionId(1)).unwrap();
        let copy = apply_change(
            &mut document,
            &mut history,
            "Part",
            &original,
            Some(original.clone()),
            true,
        )
        .unwrap()
        .unwrap();
        assert_ne!(copy, original.id());
        assert_eq!(
            definition(&document).transform_order,
            vec![ActionId(1), copy, ActionId(2), ActionId(4)]
        );
        let duplicate = find_action(definition(&document), copy).unwrap();
        apply_change(&mut document, &mut history, "Part", &duplicate, None, false).unwrap();
        assert!(!definition(&document).transform_order.contains(&copy));
        assert!(history.undo(&mut document));
        assert!(find_action(definition(&document), copy).is_some());
        assert!(history.undo(&mut document));
        assert!(find_action(definition(&document), copy).is_none());
        assert!(definition(&document).transform_order.is_empty());
    }

    #[test]
    fn action_editor_preserves_multicondition_visibility_boolean_placement_and_text_tokens() {
        let mut document = fixture();
        let mut history = History::default();
        let original = definition(&document).clone();
        for id in [ActionId(3), ActionId(4), ActionId(5)] {
            let before = find_action(definition(&document), id).unwrap();
            let mut after = before.clone();
            match &mut after {
                Action::Visibility(a) => a.name = "Selected parts".into(),
                Action::Place(a) => a.name = Some("Gate".into()),
                Action::Text(a) => a.boolean_true = "Enabled".into(),
                _ => unreachable!(),
            }
            apply_change(
                &mut document,
                &mut history,
                "Part",
                &before,
                Some(after),
                false,
            )
            .unwrap();
        }
        let after = definition(&document);
        assert_eq!(
            after.visibility[0].conditions,
            original.visibility[0].conditions
        );
        assert_eq!(
            after.placements[0].boolean_destinations,
            original.placements[0].boolean_destinations
        );
        assert_eq!(
            after.placements[0].attachment_angle,
            original.placements[0].attachment_angle
        );
        assert_eq!(after.anchors, original.anchors);
        assert_eq!(after.text_bindings[0].mode, original.text_bindings[0].mode);
        assert_eq!(after.text_bindings[0].number_precision, Some(3));
        assert_eq!(after.rotations, original.rotations);
    }

    #[test]
    fn invalid_or_stale_action_edit_leaves_definition_and_history_unchanged() {
        let mut document = fixture();
        let mut history = History::default();
        let before = definition(&document).clone();
        let original = find_action(&before, ActionId(1)).unwrap();
        let mut invalid = original.clone();
        if let Action::Rotate(a) = &mut invalid {
            a.pivot.x = f64::NAN;
        }
        assert!(apply_change(
            &mut document,
            &mut history,
            "Part",
            &original,
            Some(invalid),
            false
        )
        .is_err());
        assert_eq!(definition(&document), &before);
        assert_eq!(history.undo_len(), 0);
        let mut changed = original.clone();
        if let Action::Rotate(a) = &mut changed {
            a.pivot.x = 1.0;
        }
        apply_change(
            &mut document,
            &mut history,
            "Part",
            &original,
            Some(changed),
            false,
        )
        .unwrap();
        let latest = definition(&document).clone();
        assert!(apply_change(&mut document, &mut history, "Part", &original, None, false).is_err());
        assert_eq!(definition(&document), &latest);
        assert_eq!(history.undo_len(), 1);
    }

    #[test]
    fn conflicting_text_duplicate_is_rejected_without_losing_existing_binding() {
        let mut document = fixture();
        let mut history = History::default();
        let before = definition(&document).clone();
        let original = find_action(&before, ActionId(5)).unwrap();
        assert!(apply_change(
            &mut document,
            &mut history,
            "Part",
            &original,
            Some(original.clone()),
            true
        )
        .is_err());
        assert_eq!(definition(&document), &before);
        assert_eq!(history.undo_len(), 0);
    }
}
