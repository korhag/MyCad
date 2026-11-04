//! Selection inspector with validated, undoable property edits.

use std::collections::BTreeMap;

use cad_core::{
    arc_length, arc_sweep, circle_area, format_angle_deg, format_area, format_length,
    polyline_length, AreaMeasurement, CadColor, Document, Entity, EntityId, EntitySpace, Geometry,
    LeaderContent, Point3, Transform2,
};
use eframe::egui::{self, RichText, Ui};

use crate::app::MyCadApp;
use crate::block_edit::insert_is_editable;

#[derive(Clone, Debug)]
pub(crate) struct PropertySummary {
    pub selection: u64,
    pub content: u64,
    pub count: usize,
    pub types: Vec<(String, usize)>,
    pub layers: Vec<(String, usize)>,
    pub shared_layer: String,
    pub shared_color: String,
    pub shared_linetype: String,
}

fn draft_id() -> egui::Id {
    egui::Id::new("entocad-property-editor-draft")
}

fn current_draft_key(app: &MyCadApp) -> Option<(usize, u64, u64)> {
    let document = app.document.as_ref()?;
    if app.selection.is_empty() {
        return None;
    }
    Some((
        std::sync::Arc::as_ptr(document) as usize,
        app.selection.generation(),
        document.content_generation(),
    ))
}

/// Unapplied properties belong to the current document and selection only.
/// A stable context ID also lets restart guards find edits while this pane is hidden.
pub(crate) fn has_pending_edits(ctx: &egui::Context, app: &MyCadApp) -> bool {
    let Some(key) = current_draft_key(app) else {
        return false;
    };
    ctx.data_mut(|data| {
        data.get_temp::<PropertyEditor>(draft_id())
            .is_some_and(|draft| draft.key == key && draft.is_dirty())
    })
}

pub fn show(ui: &mut Ui, app: &mut MyCadApp) {
    let _span = cad_core::perf::span("properties");
    ui.heading("Properties");
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("properties-scroll")
        .show(ui, |ui| show_contents(ui, app));
}

fn show_contents(ui: &mut Ui, app: &mut MyCadApp) {
    let Some(document) = app.document.as_ref() else {
        ui.weak("Open a drawing, then select an object.");
        return;
    };
    if app.selection.is_empty() {
        ui.weak("Select objects to inspect or edit their properties.");
        return;
    }
    let Some(key) = current_draft_key(app) else {
        return;
    };
    let memory = draft_id();
    let mut draft = ui.data_mut(|data| data.get_temp::<PropertyEditor>(memory));
    if draft.as_ref().is_none_or(|draft| draft.key != key) {
        draft = Some(PropertyEditor::new(document, app.selection.ids(), key));
    }
    let Some(mut draft) = draft else { return };
    let edit_block = (app.selection.len() == 1)
        .then(|| app.selection.ids().first().copied())
        .flatten()
        .and_then(|id| document.entity_by_id(id))
        .filter(|entity| insert_is_editable(entity))
        .and_then(|entity| entity.geometry.insert_block_name().map(str::to_owned));
    let edit_dynamic = edit_block.as_deref().is_some_and(|name| {
        document
            .block_by_name(name)
            .is_some_and(|block| block.is_dynamic())
    });

    ui.label(RichText::new(&draft.title).strong());
    ui.weak(format!(
        "Coordinates and lengths: {}",
        document.units.label()
    ));
    if app.block_edit.is_active() {
        ui.weak("Coordinates are local to the shared block definition.");
    }
    if app.selection.len() > MAX_PROPERTY_SELECTION {
        ui.weak(format!(
            "Select at most {MAX_PROPERTY_SELECTION} objects to edit properties."
        ));
    } else {
        let blocked = app.properties_edit_block_reason();
        let mut apply = false;
        let mut reset = false;
        ui.add_enabled_ui(blocked.is_none(), |ui| {
            draw_editor(ui, document, &mut draft);
            ui.horizontal_wrapped(|ui| {
                apply = ui
                    .add_enabled(draft.is_dirty(), egui::Button::new("Apply properties"))
                    .clicked();
                reset = ui
                    .add_enabled(draft.is_dirty(), egui::Button::new("Discard changes"))
                    .clicked();
            });
        });
        if let Some(reason) = blocked {
            ui.weak(reason);
        } else if draft.is_dirty() {
            ui.weak("Pending changes apply together as one Undo step. Changing selection discards them.");
        }
        if let Some(message) = &draft.message {
            ui.colored_label(ui.visuals().error_fg_color, message);
        }
        if reset {
            draft = PropertyEditor::new(document, app.selection.ids(), key);
        }
        if apply {
            match prepare_property_changes(document, &draft) {
                Ok(changes) => match app.commit_property_changes(changes) {
                    Ok(()) => {
                        if let Some(document) = app.document.as_ref() {
                            draft = PropertyEditor::new(
                                document,
                                app.selection.ids(),
                                (
                                    std::sync::Arc::as_ptr(document) as usize,
                                    app.selection.generation(),
                                    document.content_generation(),
                                ),
                            );
                        }
                    }
                    Err(message) => draft.message = Some(message),
                },
                Err(errors) => {
                    draft.errors = errors;
                    draft.message = Some("Correct the marked properties, then apply again.".into());
                }
            }
        }
    }
    ui.data_mut(|data| data.insert_temp(memory, draft));

    ui.add_space(8.0);
    if edit_dynamic && ui.button("Edit parameters and actions…")
        .on_hover_text("Open the guided dynamic editor for this shared definition. Existing parameters and actions are kept.")
        .clicked() {
        crate::dynamic_block::begin_create_dynamic(app);
    }
    if let Some(name) = edit_block {
        if ui.button("Edit shared block definition…")
            .on_hover_text("Definition edits affect every reference to this block. Instance properties above affect only the selected reference.")
            .clicked() {
            app.edit_named_block(&name);
        }
    }
    crate::dynamic_block::show_selected_instance_parameters(ui, app);
    egui::CollapsingHeader::new("Measured values and details").show(ui, |ui| {
        let Some(document) = app.document.as_ref() else {
            return;
        };
        if app.selection.len() == 1 {
            if let Some(entity) = app
                .selection
                .ids()
                .first()
                .and_then(|id| document.entity_by_id(*id))
            {
                let references = entity
                    .geometry
                    .insert_block_name()
                    .map(|name| app.block_tree.reference_count(name));
                show_single(ui, document, entity, references);
            }
        } else {
            let stale = app.property_summary.as_ref().is_none_or(|summary| {
                summary.selection != app.selection.generation()
                    || summary.content != document.content_generation()
            });
            if stale {
                app.property_summary = Some(summarize(
                    document,
                    app.selection.ids(),
                    app.selection.generation(),
                    document.content_generation(),
                ));
            }
            if let Some(summary) = &app.property_summary {
                paint_summary(ui, summary);
            }
        }
    });
}

const MAX_PROPERTY_SELECTION: usize = 4096;
const MAX_PROPERTY_CHOICES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PropertyKey {
    Layer,
    Color,
    Linetype,
    LinetypeScale,
    X,
    Y,
    Z,
    EndX,
    EndY,
    EndZ,
    Length,
    Radius,
    StartAngle,
    EndAngle,
    ScaleX,
    ScaleY,
    ScaleZ,
    Rotation,
}

#[derive(Clone, Debug)]
struct PropertyField {
    key: PropertyKey,
    label: &'static str,
    original: Option<String>,
    value: String,
}

impl PropertyField {
    fn new(key: PropertyKey, label: &'static str, original: Option<String>) -> Self {
        Self {
            key,
            label,
            value: original.clone().unwrap_or_default(),
            original,
        }
    }

    fn changed(&self) -> bool {
        self.value != self.original.as_deref().unwrap_or("")
    }
}

#[derive(Clone, Debug)]
struct PropertyEditor {
    key: (usize, u64, u64),
    title: String,
    ids: Vec<EntityId>,
    fields: Vec<PropertyField>,
    geometry_note: Option<String>,
    errors: BTreeMap<PropertyKey, String>,
    message: Option<String>,
}

impl PropertyEditor {
    fn new(document: &Document, ids: &[EntityId], key: (usize, u64, u64)) -> Self {
        let mut editor = Self {
            key,
            title: format!("{} selected", ids.len()),
            ids: Vec::new(),
            fields: Vec::new(),
            geometry_note: None,
            errors: BTreeMap::new(),
            message: None,
        };
        if ids.len() > MAX_PROPERTY_SELECTION {
            return editor;
        }
        editor.ids.extend_from_slice(ids);
        let entities: Vec<_> = ids
            .iter()
            .filter_map(|id| document.entity_by_id(*id))
            .collect();
        let common = |value: fn(&Entity) -> String| {
            let mut values = entities.iter().map(|entity| value(entity));
            let first = values.next()?;
            values.all(|value| value == first).then_some(first)
        };
        editor.fields = vec![
            PropertyField::new(
                PropertyKey::Layer,
                "Layer",
                common(|entity| entity.layer.clone()),
            ),
            PropertyField::new(
                PropertyKey::Color,
                "Color",
                common(|entity| entity.color.display_name()),
            ),
            PropertyField::new(
                PropertyKey::Linetype,
                "Linetype",
                common(|entity| entity.linetype.clone()),
            ),
            PropertyField::new(
                PropertyKey::LinetypeScale,
                "Linetype scale",
                common(|entity| entity.linetype_scale.to_string()),
            ),
        ];
        if let [entity] = entities.as_slice() {
            editor.title = entity.geometry.type_name().into();
            editor.add_geometry_fields(&entity.geometry);
        } else {
            editor.geometry_note = Some("Common properties apply to all selected objects. Select one object to edit coordinates and dimensions.".into());
        }
        editor
    }

    fn number(&mut self, key: PropertyKey, label: &'static str, value: f64) {
        self.fields
            .push(PropertyField::new(key, label, Some(value.to_string())));
    }

    fn position(&mut self, point: Point3) {
        self.number(PropertyKey::X, "X", point.x);
        self.number(PropertyKey::Y, "Y", point.y);
        self.number(PropertyKey::Z, "Z", point.z);
    }

    fn add_geometry_fields(&mut self, geometry: &Geometry) {
        use PropertyKey::*;
        match geometry {
            Geometry::Line { start, end } => {
                self.geometry_note = Some("X/Y/Z describe the start point. Length is measured in XY; changing length keeps the start and direction.".into());
                self.position(*start);
                self.number(EndX, "End X", end.x);
                self.number(EndY, "End Y", end.y);
                self.number(EndZ, "End Z", end.z);
                self.number(Length, "Length (XY)", start.xy().distance(end.xy()));
            }
            Geometry::Circle { center, radius, .. } | Geometry::Arc { center, radius, .. } => {
                self.geometry_note =
                    Some("X/Y/Z describe the center in the object's coordinate system.".into());
                self.position(*center);
                self.number(Radius, "Radius", *radius);
                if let Geometry::Arc {
                    start_angle,
                    end_angle,
                    ..
                } = geometry
                {
                    self.number(StartAngle, "Start angle (°)", start_angle.to_degrees());
                    self.number(EndAngle, "End angle (°)", end_angle.to_degrees());
                }
            }
            Geometry::Point { position } => self.position(*position),
            Geometry::Insert {
                insertion,
                scale,
                rotation,
                extrusion,
                ..
            } => {
                if !world_extrusion(*extrusion) {
                    self.geometry_note = Some(
                        "Transforms for blocks on tilted coordinate systems are read-only here."
                            .into(),
                    );
                    return;
                }
                self.geometry_note = Some("Position and transforms affect this block instance. Negative scale mirrors an axis; zero scale is invalid.".into());
                self.position(*insertion);
                self.number(ScaleX, "Scale X", scale.x);
                self.number(ScaleY, "Scale Y", scale.y);
                self.number(ScaleZ, "Scale Z", scale.z);
                self.number(Rotation, "Rotation (°)", rotation.to_degrees());
            }
            _ => {
                self.geometry_note = Some(
                    "Common properties are editable. Geometry details are available below.".into(),
                )
            }
        }
    }

    fn is_dirty(&self) -> bool {
        self.fields.iter().any(PropertyField::changed)
    }
}

fn world_extrusion(extrusion: Point3) -> bool {
    extrusion.x.abs() < 1e-12 && extrusion.y.abs() < 1e-12 && (extrusion.z - 1.0).abs() < 1e-12
}

fn draw_editor(ui: &mut Ui, document: &Document, editor: &mut PropertyEditor) {
    let mut changed = false;
    for (index, field) in editor.fields.iter_mut().enumerate() {
        if index == 0 || index == 4 {
            ui.add_space(6.0);
            ui.label(
                RichText::new(if index == 0 {
                    "Common properties"
                } else {
                    "Geometry"
                })
                .strong(),
            );
            if index == 4 {
                if let Some(note) = &editor.geometry_note {
                    ui.weak(note);
                }
            }
        }
        ui.push_id(field.key as u8, |ui| {
            let label_width = (ui.available_width() * 0.35).clamp(55.0, 110.0);
            ui.horizontal(|ui| {
                ui.add_sized([label_width, 20.0], egui::Label::new(field.label).wrap());
                let hint = if field.original.is_none() {
                    "Mixed values"
                } else {
                    ""
                };
                let response = ui.add(
                    egui::TextEdit::singleline(&mut field.value)
                        .desired_width((ui.available_width() - 30.0).max(60.0))
                        .hint_text(hint),
                );
                changed |= response.changed();
                if matches!(
                    field.key,
                    PropertyKey::Layer | PropertyKey::Color | PropertyKey::Linetype
                ) {
                    ui.menu_button("…", |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(240.0)
                            .show(ui, |ui| {
                                let mut choice = |label: &str, value: &str| {
                                    if ui.selectable_label(field.value == value, label).clicked() {
                                        field.value = value.to_owned();
                                        changed = true;
                                        ui.close();
                                    }
                                };
                                match field.key {
                                    PropertyKey::Layer => {
                                        for name in
                                            document.layers.keys().take(MAX_PROPERTY_CHOICES)
                                        {
                                            choice(name, name);
                                        }
                                    }
                                    PropertyKey::Linetype => {
                                        choice("ByLayer", "BYLAYER");
                                        choice("ByBlock", "BYBLOCK");
                                        for name in
                                            document.linetypes.keys().take(MAX_PROPERTY_CHOICES)
                                        {
                                            choice(name, name);
                                        }
                                    }
                                    PropertyKey::Color => {
                                        for (label, value) in [
                                            ("ByLayer", "ByLayer"),
                                            ("ByBlock", "ByBlock"),
                                            ("Red", "ACI 1"),
                                            ("Yellow", "ACI 2"),
                                            ("Green", "ACI 3"),
                                            ("Cyan", "ACI 4"),
                                            ("Blue", "ACI 5"),
                                            ("Magenta", "ACI 6"),
                                            ("White / black", "ACI 7"),
                                        ] {
                                            choice(label, value);
                                        }
                                    }
                                    _ => {}
                                }
                            });
                    });
                }
            });
            if field.key == PropertyKey::Color {
                ui.weak("ByLayer, ByBlock, ACI 1–255, or #RRGGBB");
            }
            if let Some(error) = editor.errors.get(&field.key) {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
        });
    }
    if editor.fields.len() <= 4 {
        if let Some(note) = &editor.geometry_note {
            ui.weak(note);
        }
    }
    if changed {
        editor.message = None;
        editor.errors.clear();
    }
    ui.add_space(8.0);
}

#[derive(Clone, Debug)]
enum PropertyValue {
    Layer(String),
    Color(CadColor),
    Linetype(String),
    Number(PropertyKey, f64),
}

fn parse_field(document: &Document, field: &PropertyField) -> Result<PropertyValue, String> {
    let value = field.value.trim();
    match field.key {
        PropertyKey::Layer => document
            .layers
            .keys()
            .find(|name| name.eq_ignore_ascii_case(value))
            .cloned()
            .map(PropertyValue::Layer)
            .ok_or_else(|| "Choose an existing layer.".into()),
        PropertyKey::Linetype => {
            let canonical = if value.eq_ignore_ascii_case("ByLayer") {
                Some("BYLAYER".into())
            } else if value.eq_ignore_ascii_case("ByBlock") {
                Some("BYBLOCK".into())
            } else {
                document
                    .linetypes
                    .keys()
                    .find(|name| name.eq_ignore_ascii_case(value))
                    .cloned()
            };
            canonical
                .map(PropertyValue::Linetype)
                .ok_or_else(|| "Choose an existing linetype.".into())
        }
        PropertyKey::Color => parse_color(value).map(PropertyValue::Color),
        key => {
            let number = value
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .ok_or_else(|| "Enter a finite number.".to_string())?;
            if matches!(
                key,
                PropertyKey::LinetypeScale | PropertyKey::Radius | PropertyKey::Length
            ) && number <= 0.0
            {
                return Err("Enter a value greater than zero.".into());
            }
            if matches!(
                key,
                PropertyKey::ScaleX | PropertyKey::ScaleY | PropertyKey::ScaleZ
            ) && number.abs() < 1e-12
            {
                return Err("Scale magnitude must be at least 0.000000000001.".into());
            }
            Ok(PropertyValue::Number(key, number))
        }
    }
}

fn parse_color(value: &str) -> Result<CadColor, String> {
    if value.eq_ignore_ascii_case("ByLayer") {
        return Ok(CadColor::ByLayer);
    }
    if value.eq_ignore_ascii_case("ByBlock") {
        return Ok(CadColor::ByBlock);
    }
    if let Some(hex) = value
        .strip_prefix('#')
        .filter(|hex| hex.len() == 6 && hex.is_ascii())
    {
        if let Ok(rgb) = u32::from_str_radix(hex, 16) {
            return Ok(CadColor::Rgb {
                r: (rgb >> 16) as u8,
                g: (rgb >> 8) as u8,
                b: rgb as u8,
            });
        }
    }
    let upper = value.to_ascii_uppercase();
    if let Some(rgb) = upper.strip_prefix("RGB ") {
        let channels: Vec<_> = rgb
            .split(',')
            .map(|part| part.trim().parse::<u8>())
            .collect();
        if let [Ok(r), Ok(g), Ok(b)] = channels.as_slice() {
            return Ok(CadColor::Rgb {
                r: *r,
                g: *g,
                b: *b,
            });
        }
    }
    if let Ok(index) = upper
        .strip_prefix("ACI ")
        .unwrap_or(&upper)
        .trim()
        .parse::<u8>()
    {
        if index > 0 {
            return Ok(CadColor::Aci(index));
        }
    }
    Err("Use ByLayer, ByBlock, ACI 1–255, or #RRGGBB.".into())
}

fn prepare_property_changes(
    document: &Document,
    editor: &PropertyEditor,
) -> Result<Vec<(EntitySpace, Entity, Entity)>, BTreeMap<PropertyKey, String>> {
    let mut errors = BTreeMap::new();
    let mut values = Vec::new();
    for field in editor.fields.iter().filter(|field| field.changed()) {
        match parse_field(document, field) {
            Ok(value) => values.push(value),
            Err(error) => {
                errors.insert(field.key, error);
            }
        }
    }
    let length = values
        .iter()
        .any(|value| matches!(value, PropertyValue::Number(PropertyKey::Length, _)));
    let endpoints = values.iter().any(|value| {
        matches!(
            value,
            PropertyValue::Number(
                PropertyKey::X | PropertyKey::Y | PropertyKey::EndX | PropertyKey::EndY,
                _
            )
        )
    });
    if length && endpoints {
        errors.insert(
            PropertyKey::Length,
            "Apply endpoint changes before setting length.".into(),
        );
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut changes = Vec::new();
    for id in &editor.ids {
        let Some(before) = document.entity_by_id(*id) else {
            errors.insert(
                PropertyKey::Layer,
                "The selection changed. Select the objects again.".into(),
            );
            break;
        };
        let Some((space, _)) = document.find_entity_location(*id) else {
            continue;
        };
        let mut after = before.clone();
        for value in &values {
            if let Err((key, message)) = apply_property_value(&mut after, value) {
                errors.insert(key, message);
            }
        }
        if errors.is_empty() {
            if let Err(message) = update_insert_attributes(before, &mut after) {
                errors.insert(PropertyKey::Rotation, message);
            }
        }
        if before != &after {
            changes.push((space, before.clone(), after));
        }
    }
    if errors.is_empty() {
        Ok(changes)
    } else {
        Err(errors)
    }
}

fn apply_property_value(
    entity: &mut Entity,
    value: &PropertyValue,
) -> Result<(), (PropertyKey, String)> {
    let (key, number) = match value {
        PropertyValue::Layer(name) => {
            entity.layer = name.clone();
            return Ok(());
        }
        PropertyValue::Color(color) => {
            entity.color = *color;
            return Ok(());
        }
        PropertyValue::Linetype(name) => {
            entity.linetype = name.clone();
            return Ok(());
        }
        PropertyValue::Number(key, number) => (*key, *number),
    };
    if key == PropertyKey::LinetypeScale {
        entity.linetype_scale = number;
        return Ok(());
    }
    use PropertyKey::*;
    let unsupported = || {
        (
            key,
            "This property is not editable for the selected geometry.".into(),
        )
    };
    match (&mut entity.geometry, key) {
        (Geometry::Line { start, .. }, X | Y | Z) => set_point_component(start, key, number),
        (Geometry::Line { end, .. }, EndX | EndY | EndZ) => set_point_component(end, key, number),
        (Geometry::Line { start, end }, Length) => {
            let old = start.xy().distance(end.xy());
            if !old.is_finite() || old < 1e-12 {
                return Err((
                    key,
                    "A zero-length line has no direction. Edit its endpoint first.".into(),
                ));
            }
            let x = start.x + (end.x - start.x) / old * number;
            let y = start.y + (end.y - start.y) / old * number;
            if !x.is_finite() || !y.is_finite() {
                return Err((
                    key,
                    "This length is outside the supported numeric range.".into(),
                ));
            }
            end.x = x;
            end.y = y;
        }
        (Geometry::Circle { center, .. } | Geometry::Arc { center, .. }, X | Y | Z) => {
            set_point_component(center, key, number)
        }
        (Geometry::Circle { radius, .. } | Geometry::Arc { radius, .. }, Radius) => {
            *radius = number
        }
        (Geometry::Arc { start_angle, .. }, StartAngle) => *start_angle = number.to_radians(),
        (Geometry::Arc { end_angle, .. }, EndAngle) => *end_angle = number.to_radians(),
        (Geometry::Point { position }, X | Y | Z) => set_point_component(position, key, number),
        (Geometry::Insert { insertion, .. }, X | Y | Z) => {
            set_point_component(insertion, key, number)
        }
        (Geometry::Insert { scale, .. }, ScaleX | ScaleY | ScaleZ) => {
            set_point_component(scale, key, number)
        }
        (Geometry::Insert { rotation, .. }, Rotation) => *rotation = number.to_radians(),
        _ => return Err(unsupported()),
    }
    Ok(())
}

fn set_point_component(point: &mut Point3, key: PropertyKey, value: f64) {
    match key {
        PropertyKey::X | PropertyKey::EndX | PropertyKey::ScaleX => point.x = value,
        PropertyKey::Y | PropertyKey::EndY | PropertyKey::ScaleY => point.y = value,
        PropertyKey::Z | PropertyKey::EndZ | PropertyKey::ScaleZ => point.z = value,
        _ => {}
    }
}

fn update_insert_attributes(before: &Entity, after: &mut Entity) -> Result<(), String> {
    let (
        Geometry::Insert {
            insertion: old_position,
            scale: old_scale,
            rotation: old_rotation,
            ..
        },
        Geometry::Insert {
            insertion,
            scale,
            rotation,
            attribs,
            ..
        },
    ) = (&before.geometry, &mut after.geometry)
    else {
        return Ok(());
    };
    if old_position == insertion && old_scale == scale && old_rotation == rotation {
        return Ok(());
    }
    if attribs.is_empty() {
        return Ok(());
    }
    let ratio_x = scale.x / old_scale.x;
    let ratio_y = scale.y / old_scale.y;
    if ratio_x <= 0.0
        || ratio_y <= 0.0
        || (ratio_x - ratio_y).abs() > 1e-9 * ratio_x.abs().max(ratio_y.abs()).max(1.0)
    {
        return Err(
            "Blocks with attributes require matching positive X and Y scale factors here.".into(),
        );
    }
    let old = Transform2::insert(*old_position, *old_scale, *old_rotation)
        .inverse()
        .ok_or_else(|| {
            "This block's original scale is singular; its attributes cannot be transformed."
                .to_string()
        })?;
    let delta = Transform2::insert(*insertion, *scale, *rotation).then(old);
    if ![
        delta.m00, delta.m01, delta.m10, delta.m11, delta.tx, delta.ty,
    ]
    .into_iter()
    .all(f64::is_finite)
    {
        return Err("This block transform is outside the supported numeric range.".into());
    }
    // Text attributes live in the containing space. Use the same text transform
    // as CAD commands, and carry elevation with the INSERT as well.
    for text in attribs {
        let transformed = cad_core::transform_geometry(&Geometry::Text(text.clone()), delta)
            .map_err(|error| error.to_string())?;
        if let Geometry::Text(mut result) = transformed {
            let z_ratio = if old_scale.z == scale.z {
                1.0
            } else {
                scale.z / old_scale.z
            };
            result.insertion.z = insertion.z + (text.insertion.z - old_position.z) * z_ratio;
            result.alignment.z = insertion.z + (text.alignment.z - old_position.z) * z_ratio;
            if ![
                result.insertion.x,
                result.insertion.y,
                result.insertion.z,
                result.alignment.x,
                result.alignment.y,
                result.alignment.z,
                result.height,
            ]
            .into_iter()
            .all(f64::is_finite)
            {
                return Err(
                    "The attribute transform is outside the supported numeric range.".into(),
                );
            }
            *text = result;
        }
    }
    Ok(())
}

fn show_single(ui: &mut Ui, document: &Document, entity: &Entity, references: Option<usize>) {
    ui.label(RichText::new(entity.geometry.type_name()).strong());
    ui.add_space(4.0);
    ui.label(RichText::new("Common").small().weak());
    egui::Grid::new("props-common")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            kv(ui, "Id", format!("#{}", entity.id.raw()));
            kv(ui, "Layer", entity.layer.clone());
            kv(ui, "Color", color_label(document, entity));
            kv(ui, "Linetype", entity.linetype.clone());
            kv(ui, "Linetype scale", format_num(entity.linetype_scale));
            kv(ui, "Drawing LTSCALE", format_num(document.ltscale));
            kv(ui, "Units", document.units.label().to_string());
            kv(
                ui,
                "Effective LT scale",
                format_num((document.ltscale * entity.linetype_scale).max(1e-6)),
            );
            kv(ui, "Visible", yes_no(entity.visible));
        });
    ui.add_space(10.0);
    ui.label(RichText::new("Geometry").small().weak());
    egui::Grid::new("props-geom")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            geometry_rows(ui, document, &entity.geometry, references);
        });
}

fn summarize(
    document: &Document,
    ids: &[cad_core::EntityId],
    selection: u64,
    content: u64,
) -> PropertySummary {
    let entities: Vec<&Entity> = ids
        .iter()
        .filter_map(|id| document.entity_by_id(*id))
        .collect();
    let mut types: BTreeMap<&str, usize> = BTreeMap::new();
    let mut layers: BTreeMap<&str, usize> = BTreeMap::new();
    for entity in &entities {
        *types.entry(entity.geometry.type_name()).or_insert(0) += 1;
        *layers.entry(entity.layer.as_str()).or_insert(0) += 1;
    }
    PropertySummary {
        selection,
        content,
        count: entities.len(),
        types: types
            .into_iter()
            .map(|(name, count)| (name.to_string(), count))
            .collect(),
        layers: layers
            .into_iter()
            .map(|(name, count)| (name.to_string(), count))
            .collect(),
        shared_layer: shared_text(entities.iter().map(|entity| entity.layer.as_str())),
        shared_color: shared_text(entities.iter().map(|entity| color_label(document, entity))),
        shared_linetype: shared_text(entities.iter().map(|entity| entity.linetype.as_str())),
    }
}

fn paint_summary(ui: &mut Ui, summary: &PropertySummary) {
    ui.label(RichText::new(format!("{} selected", summary.count)).strong());
    ui.add_space(6.0);
    ui.label(RichText::new("Types").small().weak());
    for (name, count) in &summary.types {
        ui.monospace(format!("{name}  {count}"));
    }
    ui.add_space(6.0);
    ui.label(RichText::new("Layers").small().weak());
    for (name, count) in &summary.layers {
        ui.monospace(format!("{name}  {count}"));
    }
    ui.add_space(10.0);
    ui.label(RichText::new("Shared").small().weak());
    egui::Grid::new("props-shared")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            kv(ui, "Layer", summary.shared_layer.clone());
            kv(ui, "Color", summary.shared_color.clone());
            kv(ui, "Linetype", summary.shared_linetype.clone());
        });
}

fn geometry_rows(ui: &mut Ui, document: &Document, geometry: &Geometry, references: Option<usize>) {
    let units = document.units;
    match geometry {
        Geometry::Line { start, end } => {
            kv(ui, "Start", point_label(*start));
            kv(ui, "End", point_label(*end));
            kv(
                ui,
                "Length",
                format_length(start.xy().distance(end.xy()), units),
            );
        }
        Geometry::Circle { center, radius, .. } => {
            kv(ui, "Center", point_label(*center));
            kv(ui, "Radius", format_length(*radius, units));
            kv(ui, "Diameter", format_length(*radius * 2.0, units));
            kv(ui, "Area", format_area(circle_area(*radius), units));
        }
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            ..
        } => {
            kv(ui, "Center", point_label(*center));
            kv(ui, "Radius", format_length(*radius, units));
            kv(ui, "Start", format_angle_deg(*start_angle));
            kv(ui, "End", format_angle_deg(*end_angle));
            kv(
                ui,
                "Included angle",
                format_angle_deg(arc_sweep(*start_angle, *end_angle)),
            );
            kv(
                ui,
                "Arc length",
                format_length(arc_length(*radius, *start_angle, *end_angle), units),
            );
        }
        Geometry::LwPolyline {
            vertices,
            closed,
            linetype_generation_continuous,
            ..
        }
        | Geometry::Polyline {
            vertices,
            closed,
            linetype_generation_continuous,
        } => {
            kv(ui, "Vertices", vertices.len().to_string());
            kv(ui, "Closed", yes_no(*closed));
            kv(
                ui,
                "Length",
                format_length(polyline_length(vertices, *closed), units),
            );
            if *closed {
                if let Ok(area) = AreaMeasurement::from_polyline(vertices, true) {
                    kv(ui, "Area", format_area(area.area, units));
                    kv(ui, "Perimeter", format_length(area.perimeter, units));
                }
            }
            kv(
                ui,
                "Linetype generation",
                if *linetype_generation_continuous {
                    "Continuous"
                } else {
                    "Per segment"
                },
            );
        }
        Geometry::Insert {
            block_name,
            insertion,
            scale,
            rotation,
            attribs,
            column_count,
            row_count,
            ..
        } => {
            kv(ui, "Block", block_name.clone());
            kv(
                ui,
                "References",
                references
                    .map(|count| count.to_string())
                    .unwrap_or_else(|| "missing".into()),
            );
            kv(ui, "Insertion", point_label(*insertion));
            kv(ui, "Scale", format!("{:.4}, {:.4}", scale.x, scale.y));
            kv(ui, "Rotation", format_angle_deg(*rotation));
            if *column_count > 1 || *row_count > 1 {
                kv(ui, "Array", format!("{column_count} × {row_count}"));
            }
            if !attribs.is_empty() {
                kv(ui, "Attributes", attribs.len().to_string());
            }
        }
        Geometry::Point { position } => kv(ui, "Position", point_label(*position)),
        Geometry::Ellipse {
            center,
            major_axis,
            axis_ratio,
            ..
        } => {
            kv(ui, "Center", point_label(*center));
            kv(ui, "Major", format_num(major_axis.length()));
            kv(ui, "Ratio", format_num(*axis_ratio));
        }
        Geometry::Spline {
            degree,
            control_points,
            closed,
            ..
        } => {
            kv(ui, "Degree", degree.to_string());
            kv(ui, "Controls", control_points.len().to_string());
            kv(ui, "Closed", yes_no(*closed));
        }
        Geometry::Text(text) => {
            kv(ui, "Insertion", point_label(text.insertion));
            kv(ui, "Height", format_num(text.height));
            kv(ui, "Rotation", format_angle_deg(text.rotation));
            if let Some(attribute) = &text.attribute {
                kv(ui, "Tag", attribute.tag.clone());
            }
            kv(ui, "Value", truncate(&text.value, 48));
        }
        Geometry::MText(text) => {
            kv(ui, "Insertion", point_label(text.insertion));
            kv(ui, "Height", format_num(text.height));
            kv(ui, "Width", format_num(text.width));
            kv(ui, "Value", truncate(&text.value, 48));
        }
        Geometry::Hatch(hatch) => {
            kv(
                ui,
                "Fill",
                if hatch.solid_fill { "Solid" } else { "Pattern" },
            );
            kv(ui, "Paths", hatch.paths.len().to_string());
        }
        Geometry::Solid { corners, .. } => {
            kv(ui, "Corner 1", point_label(corners[0]));
            kv(ui, "Corner 2", point_label(corners[1]));
        }
        Geometry::Leader { vertices } => kv(ui, "Vertices", vertices.len().to_string()),
        Geometry::Image(frame) | Geometry::Wipeout(frame) => {
            kv(ui, "Corner", point_label(frame.corner));
            if !frame.path.is_empty() {
                kv(ui, "Path", frame.path.clone());
            }
        }
        Geometry::Viewport(viewport) => {
            kv(ui, "Center", point_label(viewport.center));
            kv(ui, "Width", format_length(viewport.width, units));
            kv(ui, "Height", format_length(viewport.height, units));
            kv(
                ui,
                "View height",
                format_length(viewport.view_height, units),
            );
        }
        Geometry::MLine {
            vertices,
            closed,
            offsets,
            ..
        } => {
            kv(ui, "Vertices", vertices.len().to_string());
            kv(ui, "Closed", yes_no(*closed));
            kv(ui, "Lines", offsets.len().max(1).to_string());
        }
        Geometry::MultiLeader { lines, content, .. } => {
            kv(ui, "Leaders", lines.len().to_string());
            if let LeaderContent::Text { value, .. } = content {
                kv(ui, "Text", value.clone());
            }
        }
        Geometry::Region { wires } => kv(ui, "Wires", wires.len().to_string()),
        Geometry::Dimension(data) => {
            kv(ui, "Block", data.block_name.clone());
            kv(ui, "Style", data.dimstyle.clone());
        }
    }
}

fn kv(ui: &mut Ui, label: &str, value: impl Into<String>) {
    ui.weak(label);
    ui.monospace(value.into());
    ui.end_row();
}

fn yes_no(value: bool) -> String {
    if value { "Yes" } else { "No" }.into()
}

fn point_label(p: Point3) -> String {
    if p.z.abs() > 1e-9 {
        format!("{:.4}, {:.4}, {:.4}", p.x, p.y, p.z)
    } else {
        format!("{:.4}, {:.4}", p.x, p.y)
    }
}

fn format_num(value: f64) -> String {
    if !value.is_finite() {
        "—".into()
    } else if value.abs() >= 1000.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.4}")
    }
}

fn truncate(value: &str, max: usize) -> String {
    let cleaned = value.replace(['\r', '\n'], " ");
    if cleaned.chars().count() <= max {
        cleaned
    } else {
        let cut: String = cleaned.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

fn color_label(document: &Document, entity: &Entity) -> String {
    match entity.color {
        CadColor::ByLayer => {
            let layer = document
                .layer(&entity.layer)
                .map(|l| l.color.display_name())
                .unwrap_or_else(|| "ACI 7".into());
            format!("ByLayer ({layer})")
        }
        other => other.display_name(),
    }
}

fn shared_text<I, S>(values: I) -> String
where
    I: Iterator<Item = S>,
    S: Into<String>,
{
    let mut iter = values.map(Into::into);
    let Some(first) = iter.next() else {
        return "—".into();
    };
    if iter.all(|value| value == first) {
        first
    } else {
        "Mixed".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_core::{BlockDefinition, Layer, TextData};
    use std::sync::Arc;

    fn add(document: &mut Document, entity: Entity) -> EntityId {
        document.add_entity(entity).id
    }

    fn line(start: Point3, end: Point3) -> Entity {
        Entity::new(Geometry::Line { start, end })
    }

    fn editor(document: &Document, ids: &[EntityId]) -> PropertyEditor {
        PropertyEditor::new(document, ids, (0, 0, document.content_generation()))
    }

    fn set(editor: &mut PropertyEditor, key: PropertyKey, value: &str) {
        editor
            .fields
            .iter_mut()
            .find(|field| field.key == key)
            .unwrap()
            .value = value.into();
    }

    fn app_with(document: Document, ids: &[EntityId]) -> MyCadApp {
        let mut app = MyCadApp::for_test();
        app.document = Some(Arc::new(document));
        app.refresh_derived();
        app.selection.replace_all(ids.to_vec());
        app
    }

    #[test]
    fn pending_property_edits_follow_current_document_and_selection() {
        let mut document = Document::default();
        let first = add(
            &mut document,
            line(Point3::default(), Point3::new(10.0, 0.0, 0.0)),
        );
        let second = add(
            &mut document,
            line(Point3::default(), Point3::new(20.0, 0.0, 0.0)),
        );
        let mut app = app_with(document, &[first]);
        let ctx = egui::Context::default();
        let mut draft = PropertyEditor::new(
            app.document.as_deref().unwrap(),
            app.selection.ids(),
            current_draft_key(&app).unwrap(),
        );
        ctx.data_mut(|data| data.insert_temp(draft_id(), draft.clone()));
        assert!(!has_pending_edits(&ctx, &app));

        set(&mut draft, PropertyKey::X, "42");
        ctx.data_mut(|data| data.insert_temp(draft_id(), draft.clone()));
        assert!(has_pending_edits(&ctx, &app));

        app.selection.replace_all(vec![second]);
        assert!(
            !has_pending_edits(&ctx, &app),
            "selection changes discard drafts"
        );
        draft.key = current_draft_key(&app).unwrap();
        ctx.data_mut(|data| data.insert_temp(draft_id(), draft.clone()));
        assert!(has_pending_edits(&ctx, &app));

        let previous_document = app.document.clone().unwrap();
        app.document = Some(Arc::new((*previous_document).clone()));
        assert!(
            !has_pending_edits(&ctx, &app),
            "another document invalidates the draft"
        );
        draft.key = current_draft_key(&app).unwrap();
        ctx.data_mut(|data| data.insert_temp(draft_id(), draft.clone()));
        assert!(has_pending_edits(&ctx, &app));

        add(
            Arc::get_mut(app.document.as_mut().unwrap()).unwrap(),
            line(Point3::default(), Point3::new(30.0, 0.0, 0.0)),
        );
        assert!(
            !has_pending_edits(&ctx, &app),
            "content changes invalidate the draft"
        );
        draft.key = current_draft_key(&app).unwrap();
        ctx.data_mut(|data| data.insert_temp(draft_id(), draft));
        assert!(has_pending_edits(&ctx, &app));
        app.selection.replace_all(Vec::new());
        assert!(!has_pending_edits(&ctx, &app));
    }

    #[test]
    fn properties_mixed_common_values_apply_and_undo_together() {
        let mut document = Document::default();
        let first = add(
            &mut document,
            line(Point3::default(), Point3::new(10.0, 0.0, 0.0)),
        );
        let mut second = line(Point3::new(0.0, 5.0, 0.0), Point3::new(20.0, 5.0, 0.0));
        second.color = CadColor::Aci(1);
        let second = add(&mut document, second);
        document.layers.insert(
            "Dimensions".into(),
            Layer {
                name: "Dimensions".into(),
                ..Layer::default()
            },
        );
        let ids = [first, second];
        let mut draft = editor(&document, &ids);
        assert!(draft
            .fields
            .iter()
            .find(|field| field.key == PropertyKey::Color)
            .unwrap()
            .original
            .is_none());
        assert!(!draft.is_dirty());
        set(&mut draft, PropertyKey::Layer, "dimensions");
        set(&mut draft, PropertyKey::Color, "#1234AB");
        let replacements = prepare_property_changes(&document, &draft).unwrap();
        assert_eq!(replacements.len(), 2);
        let mut app = app_with(document, &ids);
        app.commit_property_changes(replacements).unwrap();
        assert_eq!(app.history.undo_len(), 1);
        assert_eq!(app.selection.ids(), ids);
        for id in ids {
            let entity = app.document.as_ref().unwrap().entity_by_id(id).unwrap();
            assert_eq!(entity.layer, "Dimensions");
            assert_eq!(
                entity.color,
                CadColor::Rgb {
                    r: 0x12,
                    g: 0x34,
                    b: 0xab
                }
            );
        }
        app.undo();
        let document = app.document.as_ref().unwrap();
        assert_eq!(
            document.entity_by_id(first).unwrap().color,
            CadColor::ByLayer
        );
        assert_eq!(
            document.entity_by_id(second).unwrap().color,
            CadColor::Aci(1)
        );
        assert!(ids
            .iter()
            .all(|id| document.entity_by_id(*id).unwrap().layer == "0"));
        app.redo();
        assert_eq!(
            app.document
                .as_ref()
                .unwrap()
                .entity_by_id(first)
                .unwrap()
                .layer,
            "Dimensions"
        );
    }

    #[test]
    fn properties_invalid_fields_are_atomic_and_reported_by_field() {
        let mut document = Document::default();
        let id = add(
            &mut document,
            Entity::new(Geometry::Circle {
                center: Point3::new(1.0, 2.0, 3.0),
                radius: 5.0,
                extrusion: Point3::new(0.0, 0.0, 1.0),
            }),
        );
        let mut draft = editor(&document, &[id]);
        set(&mut draft, PropertyKey::Radius, "-1");
        set(&mut draft, PropertyKey::X, "NaN");
        set(&mut draft, PropertyKey::Layer, "Missing layer");
        set(&mut draft, PropertyKey::Color, "#12345Z");
        let errors = prepare_property_changes(&document, &draft).unwrap_err();
        assert_eq!(errors.len(), 4);
        assert!(errors.contains_key(&PropertyKey::Radius));
        assert!(errors.contains_key(&PropertyKey::X));
        assert!(errors.contains_key(&PropertyKey::Layer));
        assert!(errors.contains_key(&PropertyKey::Color));
        assert!(matches!(
            document.entity_by_id(id).unwrap().geometry,
            Geometry::Circle { radius: 5.0, .. }
        ));
    }

    #[test]
    fn properties_line_length_preserves_start_direction_and_elevation() {
        let mut document = Document::default();
        let id = add(
            &mut document,
            line(Point3::new(1.0, 2.0, 3.0), Point3::new(4.0, 6.0, 7.0)),
        );
        let mut draft = editor(&document, &[id]);
        set(&mut draft, PropertyKey::Length, "10");
        let changes = prepare_property_changes(&document, &draft).unwrap();
        assert_eq!(
            changes[0].2.geometry,
            Geometry::Line {
                start: Point3::new(1.0, 2.0, 3.0),
                end: Point3::new(7.0, 10.0, 7.0),
            }
        );
        set(&mut draft, PropertyKey::EndX, "20");
        assert!(prepare_property_changes(&document, &draft)
            .unwrap_err()
            .contains_key(&PropertyKey::Length));
    }

    #[test]
    fn properties_radius_and_arc_angles_commit_in_radians() {
        let mut document = Document::default();
        let id = add(
            &mut document,
            Entity::new(Geometry::Arc {
                center: Point3::default(),
                radius: 10.0,
                start_angle: 0.0,
                end_angle: 1.0,
                extrusion: Point3::new(0.0, 0.0, 1.0),
            }),
        );
        let mut draft = editor(&document, &[id]);
        set(&mut draft, PropertyKey::Radius, "25");
        set(&mut draft, PropertyKey::StartAngle, "90");
        set(&mut draft, PropertyKey::EndAngle, "180");
        let changes = prepare_property_changes(&document, &draft).unwrap();
        if let Geometry::Arc {
            radius,
            start_angle,
            end_angle,
            ..
        } = changes[0].2.geometry
        {
            assert_eq!(radius, 25.0);
            assert_eq!(start_angle, std::f64::consts::FRAC_PI_2);
            assert_eq!(end_angle, std::f64::consts::PI);
        } else {
            panic!("arc expected");
        }
    }

    fn insert(attributes: bool) -> Entity {
        Entity::new(Geometry::Insert {
            block_name: "Part".into(),
            insertion: Point3::new(10.0, 20.0, 2.0),
            scale: Point3::new(1.0, 1.0, 1.0),
            rotation: 0.0,
            extrusion: Point3::new(0.0, 0.0, 1.0),
            attribs: if attributes {
                vec![TextData {
                    insertion: Point3::new(11.0, 20.0, 2.0),
                    alignment: Point3::new(12.0, 20.0, 2.0),
                    value: "Label".into(),
                    height: 2.0,
                    ..TextData::default()
                }]
            } else {
                Vec::new()
            },
            column_count: 1,
            row_count: 1,
            column_spacing: 0.0,
            row_spacing: 0.0,
            configuration: None,
            clip: None,
        })
    }

    #[test]
    fn properties_block_transform_carries_attributes_and_leaves_definition_unchanged() {
        let mut document = Document::default();
        document.blocks.insert(
            "Part".into(),
            BlockDefinition::plain(
                "Part",
                Point3::default(),
                vec![line(Point3::default(), Point3::new(1.0, 0.0, 0.0))],
            ),
        );
        let id = add(&mut document, insert(true));
        let definition = document.blocks["Part"].clone();
        let mut draft = editor(&document, &[id]);
        set(&mut draft, PropertyKey::X, "30");
        set(&mut draft, PropertyKey::Z, "4");
        set(&mut draft, PropertyKey::Rotation, "90");
        set(&mut draft, PropertyKey::ScaleX, "2");
        set(&mut draft, PropertyKey::ScaleY, "2");
        let changes = prepare_property_changes(&document, &draft).unwrap();
        if let Geometry::Insert {
            insertion,
            rotation,
            scale,
            attribs,
            ..
        } = &changes[0].2.geometry
        {
            assert_eq!(*insertion, Point3::new(30.0, 20.0, 4.0));
            assert_eq!(*rotation, std::f64::consts::FRAC_PI_2);
            assert_eq!(*scale, Point3::new(2.0, 2.0, 1.0));
            assert!((attribs[0].insertion.x - 30.0).abs() < 1e-9);
            assert!((attribs[0].insertion.y - 22.0).abs() < 1e-9);
            assert_eq!(attribs[0].insertion.z, 4.0);
            assert_eq!(attribs[0].value, "Label");
            assert!((attribs[0].height - 4.0).abs() < 1e-9);
        } else {
            panic!("insert expected");
        }
        assert_eq!(document.blocks["Part"], definition);
        set(&mut draft, PropertyKey::ScaleY, "3");
        assert!(prepare_property_changes(&document, &draft).is_err());
    }

    #[test]
    fn properties_locked_multiselection_leaves_every_object_unchanged() {
        let mut document = Document::default();
        document.layers.insert(
            "Locked".into(),
            Layer {
                name: "Locked".into(),
                locked: true,
                ..Layer::default()
            },
        );
        let first = add(
            &mut document,
            line(Point3::default(), Point3::new(5.0, 0.0, 0.0)),
        );
        let mut locked = line(Point3::default(), Point3::new(0.0, 5.0, 0.0));
        locked.layer = "Locked".into();
        let second = add(&mut document, locked);
        let mut draft = editor(&document, &[first, second]);
        set(&mut draft, PropertyKey::Color, "ACI 2");
        let changes = prepare_property_changes(&document, &draft).unwrap();
        let mut app = app_with(document, &[first, second]);
        assert!(app
            .commit_property_changes(changes)
            .unwrap_err()
            .contains("Unlock"));
        assert_eq!(app.history.undo_len(), 0);
        for id in [first, second] {
            assert_eq!(
                app.document
                    .as_ref()
                    .unwrap()
                    .entity_by_id(id)
                    .unwrap()
                    .color,
                CadColor::ByLayer
            );
        }
    }

    #[test]
    fn properties_reject_stale_and_shared_document_commits_without_copying() {
        let mut document = Document::default();
        let id = add(
            &mut document,
            line(Point3::default(), Point3::new(5.0, 0.0, 0.0)),
        );
        let mut draft = editor(&document, &[id]);
        set(&mut draft, PropertyKey::Color, "ACI 2");
        let changes = prepare_property_changes(&document, &draft).unwrap();
        let mut app = app_with(document, &[id]);
        let snapshot = Arc::clone(app.document.as_ref().unwrap());
        assert!(app.commit_property_changes(changes.clone()).is_err());
        assert!(Arc::ptr_eq(&snapshot, app.document.as_ref().unwrap()));
        assert_eq!(app.history.undo_len(), 0);
        drop(snapshot);
        app.commit_property_changes(changes.clone()).unwrap();
        assert!(app
            .commit_property_changes(changes)
            .unwrap_err()
            .contains("changed"));
        assert_eq!(app.history.undo_len(), 1);
    }

    #[test]
    fn properties_noop_and_unsupported_inputs_do_not_create_changes() {
        let mut document = Document::default();
        let id = add(&mut document, insert(false));
        let mut draft = editor(&document, &[id]);
        assert!(prepare_property_changes(&document, &draft)
            .unwrap()
            .is_empty());
        set(&mut draft, PropertyKey::ScaleX, "0");
        set(&mut draft, PropertyKey::Rotation, "inf");
        let errors = prepare_property_changes(&document, &draft).unwrap_err();
        assert_eq!(errors.len(), 2);
        assert!(errors.contains_key(&PropertyKey::ScaleX));
        assert!(errors.contains_key(&PropertyKey::Rotation));
        for value in ["ByLayer", "ByBlock", "ACI 255", "#AABBCC", "RGB 1,2,3"] {
            assert!(parse_color(value).is_ok());
        }
        for value in ["ACI 0", "ACI 256", "RGB 1,2,300", "#ééé"] {
            assert!(parse_color(value).is_err());
        }
    }

    fn dynamic_part_document() -> (Document, EntityId, cad_core::ParameterId) {
        use cad_core::{
            BehaviorKind, CompositionRule, DynamicBehavior, DynamicDefinition, FollowRole,
            GeometryTarget, InstanceConfiguration, NumericParameter, ParameterDef, ParameterValue,
            Point2,
        };
        let mut document = Document::default();
        let width = document.allocate_parameter_id();
        let mut edge = line(Point3::default(), Point3::new(100.0, 0.0, 0.0));
        edge.id = document.allocate_id();
        let target = edge.id;
        let mut block = BlockDefinition::plain("Part", Point3::default(), vec![edge]);
        block.dynamic = Some(DynamicDefinition {
            parameters: vec![ParameterDef::number(
                width,
                "Width",
                NumericParameter::length(100.0),
            )],
            behaviors: vec![DynamicBehavior {
                id: document.allocate_action_id(),
                kind: BehaviorKind::Stretch,
                parameter: width,
                targets: vec![GeometryTarget::LineEnd(target)],
                local_direction: Point2::new(1.0, 0.0),
                reference_value: 100.0,
                multiplier: 1.0,
                composition: CompositionRule::Additive,
                follow: FollowRole::Second,
                name: None,
            }],
            ..Default::default()
        });
        document.replace_block_definition(block);
        let mut reference = insert(false);
        let mut configuration = InstanceConfiguration::default();
        configuration.set(width, ParameterValue::Number(200.0));
        reference
            .geometry
            .set_insert_configuration(Some(configuration));
        let id = document.add_entity(reference).id;
        (document, id, width)
    }

    #[test]
    fn properties_dynamic_insert_transform_keeps_evaluated_picture_and_indexes_through_undo() {
        use cad_core::{Extents2, MeasureGeom, ParameterValue, Point2};
        fn assert_derived(app: &MyCadApp, id: EntityId, start: Point2, end: Point2) {
            let pick = app
                .display
                .pick_for(id)
                .expect("evaluated INSERT is visible");
            assert!(
                pick.bounds
                    .min
                    .distance(Point2::new(start.x.min(end.x), start.y.min(end.y)))
                    < 1e-6
            );
            assert!(
                pick.bounds
                    .max
                    .distance(Point2::new(start.x.max(end.x), start.y.max(end.y)))
                    < 1e-6
            );
            let (snaps, measures) = app.property_test_indexes();
            let mut candidates = Vec::new();
            snaps.query(
                Extents2::from_corners(
                    Point2::new(end.x - 0.01, end.y - 0.01),
                    Point2::new(end.x + 0.01, end.y + 0.01),
                ),
                &mut candidates,
            );
            assert!(candidates
                .iter()
                .any(|feature| feature.point.distance(end) < 1e-6));
            let measured: Vec<_> = measures.primitives_for_owner(id).collect();
            assert_eq!(measured.len(), 1);
            if let MeasureGeom::Straight { start: a, end: b } = measured[0].geom {
                assert!(a.distance(start) < 1e-6);
                assert!(b.distance(end) < 1e-6);
                assert!((a.distance(b) - 200.0).abs() < 1e-6);
            } else {
                panic!("line expected");
            }
        }
        let (document, id, width) = dynamic_part_document();
        let original = document.block_by_name("Part").unwrap().clone();
        let mut draft = editor(&document, &[id]);
        set(&mut draft, PropertyKey::Y, "40");
        set(&mut draft, PropertyKey::Rotation, "90");
        let replacements = prepare_property_changes(&document, &draft).unwrap();
        let mut app = app_with(document, &[id]);
        assert_derived(&app, id, Point2::new(10.0, 20.0), Point2::new(210.0, 20.0));
        app.commit_property_changes(replacements).unwrap();
        assert_derived(&app, id, Point2::new(10.0, 40.0), Point2::new(10.0, 240.0));
        let current = app.document.as_ref().unwrap();
        assert_eq!(current.block_by_name("Part").unwrap(), &original);
        assert_eq!(
            current
                .entity_by_id(id)
                .unwrap()
                .geometry
                .insert_configuration()
                .unwrap()
                .get(width),
            Some(&ParameterValue::Number(200.0))
        );
        assert_eq!(app.history.undo_len(), 1);
        app.undo();
        assert_derived(&app, id, Point2::new(10.0, 20.0), Point2::new(210.0, 20.0));
        app.redo();
        assert_derived(&app, id, Point2::new(10.0, 40.0), Point2::new(10.0, 240.0));
    }

    #[test]
    fn properties_dynamic_authoring_entry_reuses_the_existing_definition() {
        let (document, id, _) = dynamic_part_document();
        let definition = document.block_by_name("Part").unwrap().clone();
        let model = document.model_space.clone();
        let mut app = app_with(document, &[id]);
        crate::dynamic_block::begin_create_dynamic(&mut app);
        assert!(app.dynamic_convert.is_none());
        assert_eq!(app.dynamic_authoring.as_ref().unwrap().block_name, "Part");
        assert_eq!(app.block_edit.current().unwrap().block_name, "Part");
        let document = app.document.as_ref().unwrap();
        assert_eq!(document.blocks.len(), 1);
        assert_eq!(document.block_by_name("Part").unwrap(), &definition);
        assert_eq!(document.model_space, model);
        assert_eq!(app.history.undo_len(), 0);
    }
}
