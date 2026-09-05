//! Shared typed configuration form for Properties, Test mode,
//! Configure Block, and preset editing.

use std::collections::{BTreeMap, BTreeSet};

use cad_core::{
    active_compatibility_rules, format_display_number, matching_preset, proposed_configuration,
    resolve_values, rule_reason, value_allowed_by_rules, DynamicDefinition, ParameterDef,
    ParameterId, ParameterKind, ParameterValue, PresetId,
};
use eframe::egui::{self, RichText, Ui};

use crate::dynamic_block::{numeric_field, NumericEdit};

// ------------------------------------------------------------
// Type: ConfigFormAction
// Purpose: Proposed edits returned by the form. The form never
//          mutates source geometry.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigFormAction {
    None,
    Values(BTreeMap<ParameterId, ParameterValue>),
    Reset(ParameterId),
    ResetAll,
    ApplyPreset(PresetId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingRelated {
    pub values: BTreeMap<ParameterId, ParameterValue>,
    pub extras: Vec<(String, String)>,
}

pub fn show_configuration_form(
    ui: &mut Ui,
    id_salt: &str,
    dynamic: &DynamicDefinition,
    current: &BTreeMap<ParameterId, ParameterValue>,
    mixed: &BTreeSet<ParameterId>,
    drafts: &mut BTreeMap<ParameterId, String>,
    errors: &mut BTreeMap<ParameterId, String>,
    pending: &mut Option<PendingRelated>,
) -> ConfigFormAction {
    let mut action = ConfigFormAction::None;
    if let Some(related) = pending.clone() {
        ui.colored_label(
            egui::Color32::from_rgb(220, 160, 80),
            "This change requires related values:",
        );
        for (name, value) in &related.extras {
            ui.label(format!("{name} → {value}"));
        }
        ui.horizontal(|ui| {
            if ui.button("Apply together").clicked() {
                action = ConfigFormAction::Values(related.values);
                *pending = None;
            }
            if ui.button("Cancel").clicked() {
                *pending = None;
            }
        });
        if !matches!(action, ConfigFormAction::None) {
            return action;
        }
    }

    let resolved = resolve_values(
        dynamic,
        Some(&cad_core::InstanceConfiguration {
            values: current.clone(),
        }),
    )
    .unwrap_or_else(|_| current.clone());
    let active = active_compatibility_rules(&dynamic.compatibility, &resolved);
    let matched = matching_preset(&dynamic.presets, &resolved, &dynamic.parameters);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Configuration").small().weak());
        if let Some(id) = matched {
            if let Some(preset) = dynamic.preset(id) {
                ui.weak(format!("Preset: {}", preset.name));
            }
        } else if !dynamic.presets.is_empty() {
            ui.weak("Custom");
        }
    });
    if !dynamic.presets.is_empty() {
        egui::ComboBox::from_id_salt(format!("{id_salt}-preset"))
            .selected_text(matched.map(|id| {
                dynamic
                    .preset(id)
                    .map(|preset| preset.name.clone())
                    .unwrap_or_else(|| "Custom".into())
            }).unwrap_or_else(|| "Custom".into()))
            .show_ui(ui, |ui| {
                if ui.selectable_label(matched.is_none(), "Custom").clicked() {
                    // Keep current values; do not apply a preset.
                }
                for preset in &dynamic.presets {
                    if ui
                        .selectable_label(matched == Some(preset.id), &preset.name)
                        .clicked()
                    {
                        action = ConfigFormAction::ApplyPreset(preset.id);
                    }
                }
            });
    }

    let parameters: Vec<ParameterDef> = dynamic.sorted_parameters().into_iter().cloned().collect();
    for parameter in &parameters {
        ui.add_space(4.0);
        ui.label(&parameter.name);
        if let Some(description) = &parameter.description {
            ui.weak(description);
        }
        if mixed.contains(&parameter.id) {
            ui.weak("Mixed");
        }
        let current_value = current
            .get(&parameter.id)
            .cloned()
            .unwrap_or_else(|| parameter.default_value());
        match &parameter.kind {
            ParameterKind::Number(numeric) => {
                let committed = current_value.as_number();
                let mut draft = drafts.remove(&parameter.id).unwrap_or_else(|| {
                    committed
                        .map(|value| format_display_number(value, numeric.display_precision))
                        .unwrap_or_default()
                });
                let outcome = numeric_field(
                    ui,
                    &format!("{id_salt}-n-{}", parameter.id.raw()),
                    &mut draft,
                    numeric,
                    committed,
                );
                drafts.insert(parameter.id, draft);
                match outcome {
                    NumericEdit::Commit(value) | NumericEdit::Preview(value) => {
                        propose(
                            dynamic,
                            current,
                            parameter.id,
                            ParameterValue::Number(value),
                            pending,
                            errors,
                            &mut action,
                        );
                    }
                    NumericEdit::Invalid(message) => {
                        errors.insert(parameter.id, message);
                    }
                    NumericEdit::Cancel | NumericEdit::None => {}
                }
            }
            ParameterKind::Choice(choice) => {
                let selected = match current_value {
                    ParameterValue::Choice(id) => id,
                    _ => choice.default,
                };
                let label = if mixed.contains(&parameter.id) {
                    "Mixed".into()
                } else {
                    choice
                        .options
                        .iter()
                        .find(|option| option.id == selected)
                        .map(|option| option.label.clone())
                        .unwrap_or_else(|| "Choose…".into())
                };
                let mut picked = None;
                egui::ComboBox::from_id_salt(format!("{id_salt}-c-{}", parameter.id.raw()))
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        for option in &choice.options {
                            let mut trial = resolved.clone();
                            trial.insert(parameter.id, ParameterValue::Choice(option.id));
                            let trial_active =
                                active_compatibility_rules(&dynamic.compatibility, &trial);
                            let allowed = value_allowed_by_rules(
                                parameter,
                                &ParameterValue::Choice(option.id),
                                &trial_active,
                            )
                            .is_ok();
                            let response = ui.add_enabled(
                                allowed || option.id == selected,
                                egui::Button::selectable(option.id == selected, &option.label),
                            );
                            if !allowed {
                                response.clone().on_hover_text(
                                    dynamic
                                        .compatibility
                                        .iter()
                                        .map(|rule| rule_reason(rule, &dynamic.parameters))
                                        .next()
                                        .unwrap_or_else(|| "Not allowed by a compatibility rule".into()),
                                );
                            }
                            if response.clicked() {
                                picked = Some(option.id);
                            }
                        }
                    });
                if let Some(option) = picked {
                    propose(
                        dynamic,
                        current,
                        parameter.id,
                        ParameterValue::Choice(option),
                        pending,
                        errors,
                        &mut action,
                    );
                }
            }
            ParameterKind::Boolean(flag) => {
                let mut on = matches!(current_value, ParameterValue::Boolean(true))
                    && !mixed.contains(&parameter.id);
                let label = if mixed.contains(&parameter.id) {
                    "Mixed"
                } else if on {
                    flag.true_label.as_str()
                } else {
                    flag.false_label.as_str()
                };
                if ui.checkbox(&mut on, label).changed() {
                    propose(
                        dynamic,
                        current,
                        parameter.id,
                        ParameterValue::Boolean(on),
                        pending,
                        errors,
                        &mut action,
                    );
                }
            }
            ParameterKind::Text(text) => {
                let mut draft = drafts.remove(&parameter.id).unwrap_or_else(|| {
                    match &current_value {
                        ParameterValue::Text(value) if !mixed.contains(&parameter.id) => {
                            value.clone()
                        }
                        _ => String::new(),
                    }
                });
                let response = if text.multiline {
                    ui.text_edit_multiline(&mut draft)
                } else {
                    ui.text_edit_singleline(&mut draft)
                };
                if response.lost_focus() || (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !text.multiline)
                {
                    propose(
                        dynamic,
                        current,
                        parameter.id,
                        ParameterValue::Text(draft.clone()),
                        pending,
                        errors,
                        &mut action,
                    );
                }
                drafts.insert(parameter.id, draft);
            }
        }
        if let Some(err) = errors.get(&parameter.id) {
            ui.colored_label(egui::Color32::from_rgb(220, 90, 70), err);
        }
        if ui.small_button("Reset").clicked() {
            action = ConfigFormAction::Reset(parameter.id);
        }
    }
    ui.horizontal(|ui| {
        if ui.button("Reset all").clicked() {
            action = ConfigFormAction::ResetAll;
        }
    });
    let _ = active;
    action
}

fn propose(
    dynamic: &DynamicDefinition,
    current: &BTreeMap<ParameterId, ParameterValue>,
    parameter: ParameterId,
    value: ParameterValue,
    pending: &mut Option<PendingRelated>,
    errors: &mut BTreeMap<ParameterId, String>,
    action: &mut ConfigFormAction,
) {
    let mut patch = BTreeMap::new();
    patch.insert(parameter, value);
    match proposed_configuration(dynamic, current, &patch) {
        Ok(proposed) => {
            errors.remove(&parameter);
            if proposed.related.is_empty() {
                *action = ConfigFormAction::Values(proposed.values);
            } else {
                let extras = proposed
                    .related
                    .iter()
                    .filter_map(|(id, value)| {
                        let name = dynamic.parameter(*id)?.name.clone();
                        Some((name, display_value(dynamic, *id, value)))
                    })
                    .collect();
                *pending = Some(PendingRelated {
                    values: proposed.values,
                    extras,
                });
            }
        }
        Err(err) => {
            errors.insert(parameter, err.to_string());
        }
    }
}

fn display_value(dynamic: &DynamicDefinition, id: ParameterId, value: &ParameterValue) -> String {
    let Some(parameter) = dynamic.parameter(id) else {
        return value.type_name().into();
    };
    match (value, &parameter.kind) {
        (ParameterValue::Number(number), ParameterKind::Number(numeric)) => {
            format_display_number(*number, numeric.display_precision)
        }
        (ParameterValue::Choice(option), ParameterKind::Choice(choice)) => choice
            .options
            .iter()
            .find(|item| item.id == *option)
            .map(|item| item.label.clone())
            .unwrap_or_else(|| "option".into()),
        (ParameterValue::Boolean(flag), ParameterKind::Boolean(boolean)) => {
            if *flag {
                boolean.true_label.clone()
            } else {
                boolean.false_label.clone()
            }
        }
        (ParameterValue::Text(text), _) => text.clone(),
        _ => value.type_name().into(),
    }
}
