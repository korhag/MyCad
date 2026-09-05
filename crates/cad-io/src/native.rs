//! Versioned native MyCAD drawing and block-asset persistence.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use cad_core::{
    validate_definition, ActionId, AnchorDef, AnchorFollow, AnchorId, AnchorPolicy, BlockDefinition,
    BlockDefinitionId, BooleanParameter, CadColor, ChoiceOption, ChoiceParameter, CompatibilityRule,
    CompositionRule, Document, DrawingUnits, DynamicBehavior, DynamicDefinition, Entity, EntityId,
    FollowRole, Geometry, GeometryGroup, GeometryTarget, HatchData, HatchEdge, HatchPath,
    HatchPatternLine, InstanceConfiguration, Layer, LineType, MeasureMode, MTextData, NestedInput,
    NestedMapping, NumericDomain, NumericParameter, NumericQuantity, OccurrencePath, OptionId,
    ParameterCondition, ParameterDef, ParameterId, ParameterKind, ParameterUnit, ParameterValue,
    PlacementBehavior, Point2, Point3, PolyVertex, Preset, PresetId, ReflectionBehavior, RotationBehavior,
    RotationSource, SizeAuthoring, StepOrigin, StepPolicy, TextBinding, TextBindingMode, TextData,
    TextParameter, TextReflectPolicy, TextToken, VertexId, VisibilityGroup,
};
use serde::{Deserialize, Serialize};

use crate::error::ExportError;
use crate::options::SaveReport;

pub const MYCAD_FORMAT: &str = "mycad";
pub const MYCAD_BLOCK_FORMAT: &str = "mycadblock";
pub const MYCAD_SCHEMA: u32 = 1;
const MAX_ENTITIES: usize = 2_000_000;
const MAX_NESTING: usize = 64;

#[derive(Debug, Serialize, Deserialize)]
struct WireFile {
    format: String,
    schema: u32,
    document: WireDocument,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireBlockFile {
    format: String,
    schema: u32,
    block: WireBlock,
    dependencies: Vec<WireBlock>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireDocument {
    units: u16,
    ltscale: f64,
    current_layer: String,
    next_entity_id: u64,
    next_definition_id: u64,
    next_parameter_id: u64,
    next_option_id: u64,
    next_action_id: u64,
    #[serde(default)]
    next_vertex_id: u64,
    #[serde(default)]
    next_anchor_id: u64,
    #[serde(default)]
    next_preset_id: u64,
    content_generation: u64,
    saved_revision: u64,
    layers: Vec<WireLayer>,
    linetypes: Vec<WireLineType>,
    blocks: Vec<WireBlock>,
    model_space: Vec<WireEntity>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireLayer {
    name: String,
    visible: bool,
    frozen: bool,
    color: WireColor,
    linetype: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireLineType {
    name: String,
    dashes: Vec<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum WireColor {
    ByLayer,
    ByBlock,
    Aci { index: u8 },
    Rgb { r: u8, g: u8, b: u8 },
}

#[derive(Debug, Serialize, Deserialize)]
struct WireBlock {
    id: u64,
    name: String,
    base_pt: [f64; 3],
    content_revision: u64,
    entities: Vec<WireEntity>,
    dynamic: Option<WireDynamic>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireEntity {
    id: u64,
    layer: String,
    color: WireColor,
    linetype: String,
    linetype_scale: f64,
    visible: bool,
    geometry: WireGeometry,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum WireGeometry {
    Line {
        start: [f64; 3],
        end: [f64; 3],
    },
    Point {
        position: [f64; 3],
    },
    Circle {
        center: [f64; 3],
        radius: f64,
        extrusion: [f64; 3],
    },
    Arc {
        center: [f64; 3],
        radius: f64,
        start_angle: f64,
        end_angle: f64,
        extrusion: [f64; 3],
    },
    Ellipse {
        center: [f64; 3],
        major_axis: [f64; 3],
        axis_ratio: f64,
        start_param: f64,
        end_param: f64,
        extrusion: [f64; 3],
    },
    LwPolyline {
        vertices: Vec<WireVertex>,
        closed: bool,
        extrusion: [f64; 3],
        linetype_generation_continuous: bool,
    },
    Polyline {
        vertices: Vec<WireVertex>,
        closed: bool,
        linetype_generation_continuous: bool,
    },
    Spline {
        degree: u32,
        control_points: Vec<[f64; 3]>,
        fit_points: Vec<[f64; 3]>,
        knots: Vec<f64>,
        weights: Vec<f64>,
        closed: bool,
    },
    Insert {
        block_name: String,
        insertion: [f64; 3],
        scale: [f64; 3],
        rotation: f64,
        extrusion: [f64; 3],
        attribs: Vec<WireText>,
        column_count: u32,
        row_count: u32,
        column_spacing: f64,
        row_spacing: f64,
        configuration: Option<WireConfig>,
    },
    Text(WireText),
    MText {
        insertion: [f64; 3],
        height: f64,
        rotation: f64,
        width: f64,
        value: String,
        extrusion: [f64; 3],
    },
    Hatch {
        extrusion: [f64; 3],
        elevation: f64,
        solid_fill: bool,
        paths: Vec<WireHatchPath>,
        pattern_lines: Vec<WirePatternLine>,
    },
    Dimension {
        block_name: String,
    },
    Solid {
        corners: [[f64; 3]; 4],
        extrusion: [f64; 3],
    },
    Leader {
        vertices: Vec<[f64; 3]>,
    },
    MLine {
        vertices: Vec<[f64; 3]>,
        closed: bool,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct WireVertex {
    point: [f64; 3],
    bulge: f64,
    #[serde(default)]
    vertex_id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireText {
    insertion: [f64; 3],
    height: f64,
    rotation: f64,
    value: String,
    extrusion: [f64; 3],
    is_attrib_def: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum WireHatchPath {
    Polyline {
        vertices: Vec<WireVertex>,
        closed: bool,
    },
    Edges {
        edges: Vec<WireHatchEdge>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum WireHatchEdge {
    Line {
        start: [f64; 3],
        end: [f64; 3],
    },
    Arc {
        center: [f64; 3],
        radius: f64,
        start_angle: f64,
        end_angle: f64,
        is_ccw: bool,
    },
    Ellipse {
        center: [f64; 3],
        major_endpoint: [f64; 3],
        axis_ratio: f64,
        start_angle: f64,
        end_angle: f64,
        is_ccw: bool,
    },
    Spline {
        control_points: Vec<[f64; 3]>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct WirePatternLine {
    angle: f64,
    base: [f64; 3],
    offset: [f64; 3],
    dashes: Vec<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireConfig {
    values: Vec<WireInstanceValue>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireInstanceValue {
    parameter: u64,
    value: WireValue,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum WireValue {
    Number { value: f64 },
    Choice { option: u64 },
    Boolean { value: bool },
    Text { value: String },
}

#[derive(Debug, Serialize, Deserialize)]
struct WireDynamic {
    parameters: Vec<WireParameter>,
    behaviors: Vec<WireBehavior>,
    #[serde(default)]
    groups: Vec<WireGroup>,
    #[serde(default)]
    anchors: Vec<WireAnchor>,
    #[serde(default)]
    visibility: Vec<WireVisibility>,
    #[serde(default)]
    text_bindings: Vec<WireTextBinding>,
    #[serde(default)]
    reflections: Vec<WireReflection>,
    #[serde(default)]
    rotations: Vec<WireRotation>,
    #[serde(default)]
    placements: Vec<WirePlacement>,
    #[serde(default)]
    nested_inputs: Vec<WireNestedInput>,
    #[serde(default)]
    compatibility: Vec<WireCompatRule>,
    #[serde(default)]
    presets: Vec<WirePreset>,
    #[serde(default)]
    transform_order: Vec<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireParameter {
    id: u64,
    name: String,
    description: Option<String>,
    #[serde(default)]
    display_order: i32,
    kind: WireParameterKind,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum WireParameterKind {
    Number {
        quantity: String,
        unit: WireUnit,
        default: f64,
        reference: f64,
        min: Option<f64>,
        max: Option<f64>,
        step: Option<f64>,
        step_policy: String,
        step_origin: String,
        display_precision: u8,
        display_order: i32,
        #[serde(default)]
        domain: Option<WireDomain>,
        #[serde(default)]
        size: Option<WireSizeAuthoring>,
    },
    Choice {
        options: Vec<WireChoiceOption>,
        default: u64,
    },
    Boolean {
        default: bool,
        #[serde(default)]
        true_label: Option<String>,
        #[serde(default)]
        false_label: Option<String>,
    },
    Text {
        default: String,
        #[serde(default)]
        multiline: bool,
        #[serde(default)]
        max_length: Option<u32>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum WireUnit {
    Drawing { code: u16 },
    Degrees,
    Radians,
    Count,
    None,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireChoiceOption {
    id: u64,
    label: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireBehavior {
    id: u64,
    kind: String,
    parameter: u64,
    targets: Vec<WireTarget>,
    local_direction: [f64; 2],
    reference_value: f64,
    multiplier: f64,
    composition: String,
    #[serde(default)]
    follow: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum WireTarget {
    Entity { id: u64 },
    LineStart { id: u64 },
    LineEnd { id: u64 },
    Vertex { entity: u64, vertex: u64 },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum WireDomain {
    Continuous,
    AllowedValues { values: Vec<f64> },
}

#[derive(Debug, Serialize, Deserialize)]
struct WireSizeAuthoring {
    point_a: [f64; 2],
    point_b: [f64; 2],
    measure: String,
    direction: [f64; 2],
    anchor: String,
    label_offset: [f64; 2],
    #[serde(default)]
    bound_anchor: Option<WireTarget>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireGroup {
    id: u64,
    name: String,
    members: Vec<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireAnchor {
    id: u64,
    name: String,
    position: [f64; 2],
    orientation: Option<f64>,
    #[serde(default)]
    follow: Option<WireAnchorFollow>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum WireAnchorFollow {
    Size { parameter: u64, role: String },
    Geometry { target: WireTarget },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum WireConditionInner {
    Choice { parameter: u64, options: Vec<u64> },
    Boolean { parameter: u64, state: bool },
}

#[derive(Debug, Serialize, Deserialize)]
struct WireVisibility {
    id: u64,
    name: String,
    members: Vec<u64>,
    conditions: Vec<WireConditionInner>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireTextBinding {
    id: u64,
    target: u64,
    mode: WireTextMode,
    #[serde(default)]
    boolean_true: Option<String>,
    #[serde(default)]
    boolean_false: Option<String>,
    #[serde(default)]
    number_precision: Option<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum WireTextMode {
    ShowValue { parameter: u64 },
    OptionMap { parameter: u64, texts: Vec<WireOptionText> },
    Formatted { tokens: Vec<WireTextToken> },
}

#[derive(Debug, Serialize, Deserialize)]
struct WireOptionText {
    option: u64,
    text: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum WireTextToken {
    Literal { text: String },
    Parameter { id: u64 },
}

#[derive(Debug, Serialize, Deserialize)]
struct WireReflection {
    id: u64,
    name: Option<String>,
    members: Vec<u64>,
    axis_a: [f64; 2],
    axis_b: [f64; 2],
    condition: WireConditionInner,
    #[serde(default)]
    text_policy: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireRotation {
    id: u64,
    name: Option<String>,
    members: Vec<u64>,
    pivot: [f64; 2],
    source: WireRotationSource,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum WireRotationSource {
    Angle { parameter: u64 },
    OptionMap { parameter: u64, angles: Vec<WireOptionAngle> },
}

#[derive(Debug, Serialize, Deserialize)]
struct WireOptionAngle {
    option: u64,
    radians: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct WirePlacement {
    id: u64,
    name: Option<String>,
    members: Vec<u64>,
    attachment: [f64; 2],
    attachment_angle: f64,
    parameter: u64,
    destinations: Vec<WireOptionAnchor>,
    #[serde(default)]
    boolean_destinations: Option<[u64; 2]>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireOptionAnchor {
    option: u64,
    anchor: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireNestedInput {
    id: u64,
    source: u64,
    occurrence: Vec<u64>,
    target_parameter: u64,
    mapping: WireNestedMapping,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum WireNestedMapping {
    Direct,
    NumericScale { factor: f64 },
    OptionMap { values: Vec<WireOptionValue> },
}

#[derive(Debug, Serialize, Deserialize)]
struct WireOptionValue {
    option: u64,
    value: WireValue,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum WireCompatRule {
    ChoiceAllowsChoice {
        id: u64,
        when: u64,
        when_option: u64,
        target: u64,
        allowed: Vec<u64>,
    },
    ChoiceRestrictsNumeric {
        id: u64,
        when: u64,
        when_option: u64,
        target: u64,
        min: Option<f64>,
        max: Option<f64>,
        allowed: Option<Vec<f64>>,
    },
    BooleanPermits {
        id: u64,
        when: u64,
        when_state: bool,
        target: u64,
        allowed_options: Option<Vec<u64>>,
        required_boolean: Option<bool>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct WirePreset {
    id: u64,
    name: String,
    values: Vec<WireInstanceValue>,
}

pub fn write_mycad(document: &Document, path: &Path) -> Result<SaveReport, ExportError> {
    let wire = WireFile {
        format: MYCAD_FORMAT.into(),
        schema: MYCAD_SCHEMA,
        document: to_wire_document(document),
    };
    write_json(path, &wire)
}

pub fn read_mycad(path: &Path) -> Result<Document, ExportError> {
    let bytes = std::fs::read(path).map_err(|source| ExportError::io(path, source))?;
    parse_mycad_bytes(&bytes)
}

pub fn parse_mycad_bytes(bytes: &[u8]) -> Result<Document, ExportError> {
    if bytes.len() > 512 * 1024 * 1024 {
        return Err(ExportError::Validation("drawing is too large".into()));
    }
    let wire: WireFile = serde_json::from_slice(bytes)
        .map_err(|err| ExportError::Validation(format!("invalid MyCAD drawing: {err}")))?;
    if wire.format != MYCAD_FORMAT {
        return Err(ExportError::Unsupported(format!(
            "expected format '{MYCAD_FORMAT}', found '{}'",
            wire.format
        )));
    }
    if wire.schema != MYCAD_SCHEMA {
        return Err(ExportError::Unsupported(format!(
            "unsupported MyCAD schema {} (this build reads {MYCAD_SCHEMA})",
            wire.schema
        )));
    }
    from_wire_document(wire.document)
}

pub fn write_mycadblock(
    document: &Document,
    block_name: &str,
    path: &Path,
) -> Result<SaveReport, ExportError> {
    let Some(root) = document.block_by_name(block_name) else {
        return Err(ExportError::Validation(format!(
            "block '{block_name}' was not found"
        )));
    };
    let mut dependencies = Vec::new();
    collect_dependencies(document, root, &mut dependencies, &mut Vec::new())?;
    let wire = WireBlockFile {
        format: MYCAD_BLOCK_FORMAT.into(),
        schema: MYCAD_SCHEMA,
        block: to_wire_block(root),
        dependencies,
    };
    write_json(path, &wire)
}

pub fn read_mycadblock(path: &Path) -> Result<BlockAsset, ExportError> {
    let bytes = std::fs::read(path).map_err(|source| ExportError::io(path, source))?;
    let wire: WireBlockFile = serde_json::from_slice(&bytes)
        .map_err(|err| ExportError::Validation(format!("invalid MyCAD block: {err}")))?;
    if wire.format != MYCAD_BLOCK_FORMAT {
        return Err(ExportError::Unsupported(format!(
            "expected format '{MYCAD_BLOCK_FORMAT}', found '{}'",
            wire.format
        )));
    }
    if wire.schema != MYCAD_SCHEMA {
        return Err(ExportError::Unsupported(format!(
            "unsupported MyCAD block schema {}",
            wire.schema
        )));
    }
    let mut blocks = Vec::new();
    for dep in wire.dependencies {
        blocks.push(from_wire_block(dep)?);
    }
    blocks.push(from_wire_block(wire.block)?);
    Ok(BlockAsset { blocks })
}

pub struct BlockAsset {
    pub blocks: Vec<BlockDefinition>,
}

pub fn import_block_asset(
    document: &mut Document,
    asset: BlockAsset,
) -> Result<String, ExportError> {
    let mut definition_map = BTreeMap::new();
    let mut entity_map = BTreeMap::new();
    let mut parameter_map = BTreeMap::new();
    let mut option_map = BTreeMap::new();
    let mut action_map = BTreeMap::new();
    let mut last_name = String::new();
    for mut block in asset.blocks {
        let old_id = block.id;
        block.id = document.allocate_definition_id();
        definition_map.insert(old_id, block.id);
        for entity in &mut block.entities {
            let old = entity.id;
            entity.id = document.allocate_id();
            if old.is_assigned() {
                entity_map.insert(old, entity.id);
            }
        }
        let vertex_map = document.remap_entity_vertex_ids(&mut block.entities);
        if let Some(dynamic) = block.dynamic.as_mut() {
            let mut anchors = BTreeMap::new();
            let mut presets = BTreeMap::new();
            for parameter in &dynamic.parameters {
                parameter_map.insert(parameter.id, document.allocate_parameter_id());
                if let ParameterKind::Choice(choice) = &parameter.kind {
                    for option in &choice.options {
                        option_map.insert(option.id, document.allocate_option_id());
                    }
                }
            }
            for id in dynamic.collect_action_ids() {
                action_map.insert(id, document.allocate_action_id());
            }
            for anchor in &dynamic.anchors {
                anchors.insert(anchor.id, document.allocate_anchor_id());
            }
            for preset in &dynamic.presets {
                presets.insert(preset.id, document.allocate_preset_id());
            }
            dynamic
                .remap_ids_with(
                    &parameter_map,
                    &option_map,
                    &action_map,
                    &anchors,
                    &presets,
                    &entity_map,
                    &vertex_map,
                )
                .map_err(|err| ExportError::Validation(err.to_string()))?;
            for entity in &mut block.entities {
                if let Some(config) = entity.geometry.insert_configuration_mut() {
                    if let Some(values) = config.as_mut() {
                        values.remap_identities(&parameter_map, &option_map);
                    }
                }
            }
        }
        if document.block_key(&block.name).is_some() {
            block.name = unique_imported_name(document, &block.name);
        }
        last_name = block.name.clone();
        document.replace_block_definition(block);
    }
    let _ = definition_map;
    Ok(last_name)
}

fn unique_imported_name(document: &Document, source: &str) -> String {
    for index in 1..10_000 {
        let candidate = format!("{source}_{index:03}");
        if document.block_key(&candidate).is_none() {
            return candidate;
        }
    }
    format!("{source}_imported")
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<SaveReport, ExportError> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|err| ExportError::Validation(format!("serialize failed: {err}")))?;
    crate::atomic::write_atomic(path, &bytes).map_err(|source| ExportError::io(path, source))?;
    Ok(SaveReport::default())
}

fn collect_dependencies(
    document: &Document,
    block: &BlockDefinition,
    out: &mut Vec<WireBlock>,
    stack: &mut Vec<String>,
) -> Result<(), ExportError> {
    if stack.len() > MAX_NESTING {
        return Err(ExportError::Validation("block nesting is too deep".into()));
    }
    if stack
        .iter()
        .any(|name| name.eq_ignore_ascii_case(&block.name))
    {
        return Ok(());
    }
    stack.push(block.name.clone());
    for entity in &block.entities {
        if let Some(name) = entity.geometry.insert_block_name() {
            if let Some(child) = document.block_by_name(name) {
                collect_dependencies(document, child, out, stack)?;
                if !out
                    .iter()
                    .any(|item| item.name.eq_ignore_ascii_case(&child.name))
                {
                    out.push(to_wire_block(child));
                }
            }
        }
    }
    stack.pop();
    Ok(())
}

fn to_wire_document(document: &Document) -> WireDocument {
    let (
        next_entity_id,
        next_definition_id,
        next_parameter_id,
        next_option_id,
        next_action_id,
        content_generation,
        saved_revision,
    ) = document.identity_counters();
    WireDocument {
        units: document.units.to_insunits(),
        ltscale: document.ltscale,
        current_layer: document.current_layer.clone(),
        next_entity_id,
        next_definition_id,
        next_parameter_id,
        next_option_id,
        next_action_id,
        next_vertex_id: document.next_vertex_id(),
        next_anchor_id: document.next_anchor_id(),
        next_preset_id: document.next_preset_id(),
        content_generation,
        saved_revision,
        layers: document
            .layers
            .values()
            .map(|layer| WireLayer {
                name: layer.name.clone(),
                visible: layer.visible,
                frozen: layer.frozen,
                color: to_color(layer.color),
                linetype: layer.linetype.clone(),
            })
            .collect(),
        linetypes: document
            .linetypes
            .values()
            .map(|lt| WireLineType {
                name: lt.name.clone(),
                dashes: lt.dashes.clone(),
            })
            .collect(),
        blocks: document.blocks.values().map(to_wire_block).collect(),
        model_space: document.model_space.iter().map(to_wire_entity).collect(),
    }
}

fn from_wire_document(wire: WireDocument) -> Result<Document, ExportError> {
    let mut document = Document::default();
    document.units = DrawingUnits::from_insunits(wire.units);
    document.ltscale = wire.ltscale;
    document.current_layer = wire.current_layer;
    document.layers.clear();
    for layer in wire.layers {
        document.layers.insert(
            layer.name.clone(),
            Layer {
                name: layer.name,
                visible: layer.visible,
                frozen: layer.frozen,
                color: from_color(layer.color),
                linetype: layer.linetype,
            },
        );
    }
    document.ensure_layer_zero();
    for lt in wire.linetypes {
        document.linetypes.insert(
            lt.name.clone(),
            LineType {
                name: lt.name,
                dashes: lt.dashes,
            },
        );
    }
    let mut seen_defs = BTreeSet::new();
    let mut entity_total = wire.model_space.len();
    for block in &wire.blocks {
        entity_total += block.entities.len();
        if entity_total > MAX_ENTITIES {
            return Err(ExportError::Validation("too many entities".into()));
        }
        if !seen_defs.insert(block.id) && block.id != 0 {
            return Err(ExportError::Validation(format!(
                "duplicate block identity {}",
                block.id
            )));
        }
    }
    for block in wire.blocks {
        let definition = from_wire_block(block)?;
        document.blocks.insert(definition.name.clone(), definition);
    }
    let mut seen_entities = BTreeSet::new();
    for entity in wire.model_space {
        let entity = from_wire_entity(entity)?;
        if entity.id.is_assigned() && !seen_entities.insert(entity.id.raw()) {
            return Err(ExportError::Validation(format!(
                "duplicate entity identity {}",
                entity.id.raw()
            )));
        }
        document.model_space.push(entity);
    }
    document.set_identity_counters(
        wire.next_entity_id,
        wire.next_definition_id,
        wire.next_parameter_id,
        wire.next_option_id,
        wire.next_action_id,
        wire.content_generation,
        wire.saved_revision,
    );
    document.set_next_vertex_id(wire.next_vertex_id);
    document.set_next_anchor_id(wire.next_anchor_id);
    document.set_next_preset_id(wire.next_preset_id);
    document.assign_missing_ids();
    for block in document.blocks.values() {
        if let Some(dynamic) = &block.dynamic {
            validate_definition(dynamic, &block.entities)
                .map_err(|err| ExportError::Validation(err.to_string()))?;
        }
    }
    Ok(document)
}

fn to_wire_block(block: &BlockDefinition) -> WireBlock {
    WireBlock {
        id: block.id.raw(),
        name: block.name.clone(),
        base_pt: pt(block.base_pt),
        content_revision: block.content_revision,
        entities: block.entities.iter().map(to_wire_entity).collect(),
        dynamic: block.dynamic.as_ref().map(to_wire_dynamic),
    }
}

fn from_wire_block(block: WireBlock) -> Result<BlockDefinition, ExportError> {
    let mut definition = BlockDefinition::plain(block.name, from_pt(block.base_pt), Vec::new());
    definition.id = BlockDefinitionId(block.id);
    definition.content_revision = block.content_revision;
    for entity in block.entities {
        definition.entities.push(from_wire_entity(entity)?);
    }
    if let Some(dynamic) = block.dynamic {
        definition.dynamic = Some(from_wire_dynamic(dynamic)?);
        if let Some(dyn_def) = &definition.dynamic {
            validate_definition(dyn_def, &definition.entities)
                .map_err(|err| ExportError::Validation(err.to_string()))?;
        }
    }
    Ok(definition)
}

fn to_wire_entity(entity: &Entity) -> WireEntity {
    WireEntity {
        id: entity.id.raw(),
        layer: entity.layer.clone(),
        color: to_color(entity.color),
        linetype: entity.linetype.clone(),
        linetype_scale: entity.linetype_scale,
        visible: entity.visible,
        geometry: to_wire_geometry(&entity.geometry),
    }
}

fn from_wire_entity(entity: WireEntity) -> Result<Entity, ExportError> {
    Ok(Entity {
        id: EntityId(entity.id),
        layer: entity.layer,
        color: from_color(entity.color),
        linetype: entity.linetype,
        linetype_scale: entity.linetype_scale,
        visible: entity.visible,
        geometry: from_wire_geometry(entity.geometry)?,
    })
}

fn to_wire_geometry(geometry: &Geometry) -> WireGeometry {
    match geometry {
        Geometry::Line { start, end } => WireGeometry::Line {
            start: pt(*start),
            end: pt(*end),
        },
        Geometry::Point { position } => WireGeometry::Point {
            position: pt(*position),
        },
        Geometry::Circle {
            center,
            radius,
            extrusion,
        } => WireGeometry::Circle {
            center: pt(*center),
            radius: *radius,
            extrusion: pt(*extrusion),
        },
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            extrusion,
        } => WireGeometry::Arc {
            center: pt(*center),
            radius: *radius,
            start_angle: *start_angle,
            end_angle: *end_angle,
            extrusion: pt(*extrusion),
        },
        Geometry::Ellipse {
            center,
            major_axis,
            axis_ratio,
            start_param,
            end_param,
            extrusion,
        } => WireGeometry::Ellipse {
            center: pt(*center),
            major_axis: pt(*major_axis),
            axis_ratio: *axis_ratio,
            start_param: *start_param,
            end_param: *end_param,
            extrusion: pt(*extrusion),
        },
        Geometry::LwPolyline {
            vertices,
            closed,
            extrusion,
            linetype_generation_continuous,
        } => WireGeometry::LwPolyline {
            vertices: vertices.iter().map(to_vertex).collect(),
            closed: *closed,
            extrusion: pt(*extrusion),
            linetype_generation_continuous: *linetype_generation_continuous,
        },
        Geometry::Polyline {
            vertices,
            closed,
            linetype_generation_continuous,
        } => WireGeometry::Polyline {
            vertices: vertices.iter().map(to_vertex).collect(),
            closed: *closed,
            linetype_generation_continuous: *linetype_generation_continuous,
        },
        Geometry::Spline {
            degree,
            control_points,
            fit_points,
            knots,
            weights,
            closed,
        } => WireGeometry::Spline {
            degree: *degree,
            control_points: control_points.iter().copied().map(pt).collect(),
            fit_points: fit_points.iter().copied().map(pt).collect(),
            knots: knots.clone(),
            weights: weights.clone(),
            closed: *closed,
        },
        Geometry::Insert {
            block_name,
            insertion,
            scale,
            rotation,
            extrusion,
            attribs,
            column_count,
            row_count,
            column_spacing,
            row_spacing,
            configuration,
        } => WireGeometry::Insert {
            block_name: block_name.clone(),
            insertion: pt(*insertion),
            scale: pt(*scale),
            rotation: *rotation,
            extrusion: pt(*extrusion),
            attribs: attribs.iter().map(to_text).collect(),
            column_count: *column_count,
            row_count: *row_count,
            column_spacing: *column_spacing,
            row_spacing: *row_spacing,
            configuration: configuration.as_ref().map(to_config),
        },
        Geometry::Text(data) => WireGeometry::Text(to_text(data)),
        Geometry::MText(data) => WireGeometry::MText {
            insertion: pt(data.insertion),
            height: data.height,
            rotation: data.rotation,
            width: data.width,
            value: data.value.clone(),
            extrusion: pt(data.extrusion),
        },
        Geometry::Hatch(hatch) => WireGeometry::Hatch {
            extrusion: pt(hatch.extrusion),
            elevation: hatch.elevation,
            solid_fill: hatch.solid_fill,
            paths: hatch.paths.iter().map(to_hatch_path).collect(),
            pattern_lines: hatch.pattern_lines.iter().map(to_pattern).collect(),
        },
        Geometry::Dimension { block_name } => WireGeometry::Dimension {
            block_name: block_name.clone(),
        },
        Geometry::Solid { corners, extrusion } => WireGeometry::Solid {
            corners: corners.map(pt),
            extrusion: pt(*extrusion),
        },
        Geometry::Leader { vertices } => WireGeometry::Leader {
            vertices: vertices.iter().copied().map(pt).collect(),
        },
        Geometry::MLine { vertices, closed } => WireGeometry::MLine {
            vertices: vertices.iter().copied().map(pt).collect(),
            closed: *closed,
        },
    }
}

fn from_wire_geometry(geometry: WireGeometry) -> Result<Geometry, ExportError> {
    Ok(match geometry {
        WireGeometry::Line { start, end } => Geometry::Line {
            start: from_pt(start),
            end: from_pt(end),
        },
        WireGeometry::Point { position } => Geometry::Point {
            position: from_pt(position),
        },
        WireGeometry::Circle {
            center,
            radius,
            extrusion,
        } => Geometry::Circle {
            center: from_pt(center),
            radius,
            extrusion: from_pt(extrusion),
        },
        WireGeometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            extrusion,
        } => Geometry::Arc {
            center: from_pt(center),
            radius,
            start_angle,
            end_angle,
            extrusion: from_pt(extrusion),
        },
        WireGeometry::Ellipse {
            center,
            major_axis,
            axis_ratio,
            start_param,
            end_param,
            extrusion,
        } => Geometry::Ellipse {
            center: from_pt(center),
            major_axis: from_pt(major_axis),
            axis_ratio,
            start_param,
            end_param,
            extrusion: from_pt(extrusion),
        },
        WireGeometry::LwPolyline {
            vertices,
            closed,
            extrusion,
            linetype_generation_continuous,
        } => Geometry::LwPolyline {
            vertices: vertices.into_iter().map(from_vertex).collect(),
            closed,
            extrusion: from_pt(extrusion),
            linetype_generation_continuous,
        },
        WireGeometry::Polyline {
            vertices,
            closed,
            linetype_generation_continuous,
        } => Geometry::Polyline {
            vertices: vertices.into_iter().map(from_vertex).collect(),
            closed,
            linetype_generation_continuous,
        },
        WireGeometry::Spline {
            degree,
            control_points,
            fit_points,
            knots,
            weights,
            closed,
        } => Geometry::Spline {
            degree,
            control_points: control_points.into_iter().map(from_pt).collect(),
            fit_points: fit_points.into_iter().map(from_pt).collect(),
            knots,
            weights,
            closed,
        },
        WireGeometry::Insert {
            block_name,
            insertion,
            scale,
            rotation,
            extrusion,
            attribs,
            column_count,
            row_count,
            column_spacing,
            row_spacing,
            configuration,
        } => Geometry::Insert {
            block_name,
            insertion: from_pt(insertion),
            scale: from_pt(scale),
            rotation,
            extrusion: from_pt(extrusion),
            attribs: attribs.into_iter().map(from_text).collect(),
            column_count,
            row_count,
            column_spacing,
            row_spacing,
            configuration: configuration.map(from_config),
        },
        WireGeometry::Text(data) => Geometry::Text(from_text(data)),
        WireGeometry::MText {
            insertion,
            height,
            rotation,
            width,
            value,
            extrusion,
        } => Geometry::MText(MTextData {
            insertion: from_pt(insertion),
            height,
            rotation,
            width,
            value,
            extrusion: from_pt(extrusion),
        }),
        WireGeometry::Hatch {
            extrusion,
            elevation,
            solid_fill,
            paths,
            pattern_lines,
        } => Geometry::Hatch(HatchData {
            extrusion: from_pt(extrusion),
            elevation,
            solid_fill,
            paths: paths.into_iter().map(from_hatch_path).collect(),
            pattern_lines: pattern_lines.into_iter().map(from_pattern).collect(),
        }),
        WireGeometry::Dimension { block_name } => Geometry::Dimension { block_name },
        WireGeometry::Solid { corners, extrusion } => Geometry::Solid {
            corners: corners.map(from_pt),
            extrusion: from_pt(extrusion),
        },
        WireGeometry::Leader { vertices } => Geometry::Leader {
            vertices: vertices.into_iter().map(from_pt).collect(),
        },
        WireGeometry::MLine { vertices, closed } => Geometry::MLine {
            vertices: vertices.into_iter().map(from_pt).collect(),
            closed,
        },
    })
}

fn to_wire_dynamic(dynamic: &DynamicDefinition) -> WireDynamic {
    WireDynamic {
        parameters: dynamic.parameters.iter().map(to_parameter).collect(),
        behaviors: dynamic.behaviors.iter().map(to_behavior).collect(),
        groups: dynamic
            .groups
            .iter()
            .map(|group| WireGroup {
                id: group.id.raw(),
                name: group.name.clone(),
                members: group.members.iter().map(|id| id.raw()).collect(),
            })
            .collect(),
        anchors: dynamic.anchors.iter().map(to_anchor).collect(),
        visibility: dynamic.visibility.iter().map(to_visibility).collect(),
        text_bindings: dynamic.text_bindings.iter().map(to_text_binding).collect(),
        reflections: dynamic.reflections.iter().map(to_reflection).collect(),
        rotations: dynamic.rotations.iter().map(to_rotation).collect(),
        placements: dynamic.placements.iter().map(to_placement).collect(),
        nested_inputs: dynamic.nested_inputs.iter().map(to_nested_input).collect(),
        compatibility: dynamic.compatibility.iter().map(to_compat).collect(),
        presets: dynamic.presets.iter().map(to_preset).collect(),
        transform_order: dynamic.transform_order.iter().map(|id| id.raw()).collect(),
    }
}

fn from_wire_dynamic(dynamic: WireDynamic) -> Result<DynamicDefinition, ExportError> {
    Ok(DynamicDefinition {
        parameters: dynamic
            .parameters
            .into_iter()
            .map(from_parameter)
            .collect::<Result<_, _>>()?,
        behaviors: dynamic
            .behaviors
            .into_iter()
            .map(from_behavior)
            .collect::<Result<_, _>>()?,
        groups: dynamic
            .groups
            .into_iter()
            .map(|group| GeometryGroup {
                id: ActionId(group.id),
                name: group.name,
                members: group.members.into_iter().map(EntityId).collect(),
            })
            .collect(),
        anchors: dynamic
            .anchors
            .into_iter()
            .map(from_anchor)
            .collect::<Result<_, _>>()?,
        visibility: dynamic
            .visibility
            .into_iter()
            .map(from_visibility)
            .collect::<Result<_, _>>()?,
        text_bindings: dynamic
            .text_bindings
            .into_iter()
            .map(from_text_binding)
            .collect::<Result<_, _>>()?,
        reflections: dynamic
            .reflections
            .into_iter()
            .map(from_reflection)
            .collect::<Result<_, _>>()?,
        rotations: dynamic
            .rotations
            .into_iter()
            .map(from_rotation)
            .collect::<Result<_, _>>()?,
        placements: dynamic
            .placements
            .into_iter()
            .map(from_placement)
            .collect::<Result<_, _>>()?,
        nested_inputs: dynamic
            .nested_inputs
            .into_iter()
            .map(from_nested_input)
            .collect::<Result<_, _>>()?,
        compatibility: dynamic.compatibility.into_iter().map(from_compat).collect(),
        presets: dynamic.presets.into_iter().map(from_preset).collect(),
        transform_order: dynamic.transform_order.into_iter().map(ActionId).collect(),
    })
}

fn to_parameter(parameter: &ParameterDef) -> WireParameter {
    WireParameter {
        id: parameter.id.raw(),
        name: parameter.name.clone(),
        description: parameter.description.clone(),
        display_order: parameter.display_order,
        kind: match &parameter.kind {
            ParameterKind::Number(numeric) => WireParameterKind::Number {
                quantity: quantity_name(numeric.quantity).into(),
                unit: to_unit(numeric.unit),
                default: numeric.default,
                reference: numeric.reference,
                min: numeric.min,
                max: numeric.max,
                step: numeric.step,
                step_policy: match numeric.step_policy {
                    StepPolicy::IncrementOnly => "increment_only".into(),
                    StepPolicy::RequiredIncrement => "required_increment".into(),
                },
                step_origin: match numeric.step_origin {
                    StepOrigin::Minimum => "minimum".into(),
                    StepOrigin::Zero => "zero".into(),
                },
                display_precision: numeric.display_precision,
                display_order: numeric.display_order,
                domain: Some(to_domain(&numeric.domain)),
                size: numeric.size.as_ref().map(to_size),
            },
            ParameterKind::Choice(choice) => WireParameterKind::Choice {
                options: choice
                    .options
                    .iter()
                    .map(|option| WireChoiceOption {
                        id: option.id.raw(),
                        label: option.label.clone(),
                    })
                    .collect(),
                default: choice.default.raw(),
            },
            ParameterKind::Boolean(flag) => WireParameterKind::Boolean {
                default: flag.default,
                true_label: Some(flag.true_label.clone()),
                false_label: Some(flag.false_label.clone()),
            },
            ParameterKind::Text(text) => WireParameterKind::Text {
                default: text.default.clone(),
                multiline: text.multiline,
                max_length: text.max_length,
            },
        },
    }
}

fn from_parameter(parameter: WireParameter) -> Result<ParameterDef, ExportError> {
    let display_order = parameter.display_order;
    let kind = match parameter.kind {
        WireParameterKind::Number {
            quantity,
            unit,
            default,
            reference,
            min,
            max,
            step,
            step_policy,
            step_origin,
            display_precision,
            display_order: numeric_order,
            domain,
            size,
        } => ParameterKind::Number(NumericParameter {
            quantity: parse_quantity(&quantity)?,
            unit: from_unit(unit),
            default,
            reference,
            min,
            max,
            step,
            step_policy: parse_step_policy(&step_policy)?,
            step_origin: parse_step_origin(&step_origin)?,
            display_precision,
            display_order: numeric_order,
            domain: from_domain(domain),
            size: size.map(from_size).transpose()?,
        }),
        WireParameterKind::Choice { options, default } => ParameterKind::Choice(ChoiceParameter {
            options: options
                .into_iter()
                .map(|option| ChoiceOption {
                    id: OptionId(option.id),
                    label: option.label,
                })
                .collect(),
            default: OptionId(default),
        }),
        WireParameterKind::Boolean {
            default,
            true_label,
            false_label,
        } => ParameterKind::Boolean(BooleanParameter {
            default,
            true_label: true_label.unwrap_or_else(|| "On".into()),
            false_label: false_label.unwrap_or_else(|| "Off".into()),
        }),
        WireParameterKind::Text {
            default,
            multiline,
            max_length,
        } => ParameterKind::Text(TextParameter {
            default,
            multiline,
            max_length,
        }),
    };
    let display_order = if display_order != 0 {
        display_order
    } else if let ParameterKind::Number(numeric) = &kind {
        numeric.display_order
    } else {
        0
    };
    Ok(ParameterDef {
        id: ParameterId(parameter.id),
        name: parameter.name,
        description: parameter.description,
        display_order,
        kind,
    })
}

fn to_anchor(anchor: &AnchorDef) -> WireAnchor {
    WireAnchor {
        id: anchor.id.raw(),
        name: anchor.name.clone(),
        position: [anchor.position.x, anchor.position.y],
        orientation: anchor.orientation,
        follow: anchor.follow.as_ref().map(|follow| match follow {
            AnchorFollow::Size { parameter, role } => WireAnchorFollow::Size {
                parameter: parameter.raw(),
                role: follow_name(*role).into(),
            },
            AnchorFollow::Geometry(target) => WireAnchorFollow::Geometry {
                target: to_target(target),
            },
        }),
    }
}

fn from_anchor(anchor: WireAnchor) -> Result<AnchorDef, ExportError> {
    Ok(AnchorDef {
        id: AnchorId(anchor.id),
        name: anchor.name,
        position: Point2::new(anchor.position[0], anchor.position[1]),
        orientation: anchor.orientation,
        follow: match anchor.follow {
            Some(WireAnchorFollow::Size { parameter, role }) => Some(AnchorFollow::Size {
                parameter: ParameterId(parameter),
                role: parse_follow(&role)?,
            }),
            Some(WireAnchorFollow::Geometry { target }) => {
                Some(AnchorFollow::Geometry(from_target(target)))
            }
            None => None,
        },
    })
}

fn to_condition(condition: &ParameterCondition) -> WireConditionInner {
    match condition {
        ParameterCondition::Choice { parameter, options } => WireConditionInner::Choice {
            parameter: parameter.raw(),
            options: options.iter().map(|id| id.raw()).collect(),
        },
        ParameterCondition::Boolean { parameter, state } => WireConditionInner::Boolean {
            parameter: parameter.raw(),
            state: *state,
        },
    }
}

fn from_condition(condition: WireConditionInner) -> ParameterCondition {
    match condition {
        WireConditionInner::Choice { parameter, options } => ParameterCondition::Choice {
            parameter: ParameterId(parameter),
            options: options.into_iter().map(OptionId).collect(),
        },
        WireConditionInner::Boolean { parameter, state } => ParameterCondition::Boolean {
            parameter: ParameterId(parameter),
            state,
        },
    }
}

fn to_visibility(group: &VisibilityGroup) -> WireVisibility {
    WireVisibility {
        id: group.id.raw(),
        name: group.name.clone(),
        members: group.members.iter().map(|id| id.raw()).collect(),
        conditions: group.conditions.iter().map(to_condition).collect(),
    }
}

fn from_visibility(group: WireVisibility) -> Result<VisibilityGroup, ExportError> {
    Ok(VisibilityGroup {
        id: ActionId(group.id),
        name: group.name,
        members: group.members.into_iter().map(EntityId).collect(),
        conditions: group.conditions.into_iter().map(from_condition).collect(),
    })
}

fn to_text_binding(binding: &TextBinding) -> WireTextBinding {
    WireTextBinding {
        id: binding.id.raw(),
        target: binding.target.raw(),
        mode: match &binding.mode {
            TextBindingMode::ShowValue { parameter } => WireTextMode::ShowValue {
                parameter: parameter.raw(),
            },
            TextBindingMode::OptionMap { parameter, texts } => WireTextMode::OptionMap {
                parameter: parameter.raw(),
                texts: texts
                    .iter()
                    .map(|(option, text)| WireOptionText {
                        option: option.raw(),
                        text: text.clone(),
                    })
                    .collect(),
            },
            TextBindingMode::Formatted { tokens } => WireTextMode::Formatted {
                tokens: tokens
                    .iter()
                    .map(|token| match token {
                        TextToken::Literal(text) => WireTextToken::Literal { text: text.clone() },
                        TextToken::Parameter(id) => WireTextToken::Parameter { id: id.raw() },
                    })
                    .collect(),
            },
        },
        boolean_true: Some(binding.boolean_true.clone()),
        boolean_false: Some(binding.boolean_false.clone()),
        number_precision: binding.number_precision,
    }
}

fn from_text_binding(binding: WireTextBinding) -> Result<TextBinding, ExportError> {
    Ok(TextBinding {
        id: ActionId(binding.id),
        target: EntityId(binding.target),
        mode: match binding.mode {
            WireTextMode::ShowValue { parameter } => TextBindingMode::ShowValue {
                parameter: ParameterId(parameter),
            },
            WireTextMode::OptionMap { parameter, texts } => TextBindingMode::OptionMap {
                parameter: ParameterId(parameter),
                texts: texts
                    .into_iter()
                    .map(|item| (OptionId(item.option), item.text))
                    .collect(),
            },
            WireTextMode::Formatted { tokens } => TextBindingMode::Formatted {
                tokens: tokens
                    .into_iter()
                    .map(|token| match token {
                        WireTextToken::Literal { text } => TextToken::Literal(text),
                        WireTextToken::Parameter { id } => TextToken::Parameter(ParameterId(id)),
                    })
                    .collect(),
            },
        },
        boolean_true: binding.boolean_true.unwrap_or_else(|| "On".into()),
        boolean_false: binding.boolean_false.unwrap_or_else(|| "Off".into()),
        number_precision: binding.number_precision,
    })
}

fn to_reflection(behavior: &ReflectionBehavior) -> WireReflection {
    WireReflection {
        id: behavior.id.raw(),
        name: behavior.name.clone(),
        members: behavior.members.iter().map(|id| id.raw()).collect(),
        axis_a: [behavior.axis_a.x, behavior.axis_a.y],
        axis_b: [behavior.axis_b.x, behavior.axis_b.y],
        condition: to_condition(&behavior.condition),
        text_policy: Some(match behavior.text_policy {
            TextReflectPolicy::KeepReadable => "keep_readable".into(),
            TextReflectPolicy::KeepUpright => "keep_upright".into(),
            TextReflectPolicy::Mirror => "mirror".into(),
        }),
    }
}

fn from_reflection(behavior: WireReflection) -> Result<ReflectionBehavior, ExportError> {
    Ok(ReflectionBehavior {
        id: ActionId(behavior.id),
        name: behavior.name,
        members: behavior.members.into_iter().map(EntityId).collect(),
        axis_a: Point2::new(behavior.axis_a[0], behavior.axis_a[1]),
        axis_b: Point2::new(behavior.axis_b[0], behavior.axis_b[1]),
        condition: from_condition(behavior.condition),
        text_policy: match behavior.text_policy.as_deref() {
            Some("keep_upright") => TextReflectPolicy::KeepUpright,
            Some("mirror") => TextReflectPolicy::Mirror,
            _ => TextReflectPolicy::KeepReadable,
        },
    })
}

fn to_rotation(behavior: &RotationBehavior) -> WireRotation {
    WireRotation {
        id: behavior.id.raw(),
        name: behavior.name.clone(),
        members: behavior.members.iter().map(|id| id.raw()).collect(),
        pivot: [behavior.pivot.x, behavior.pivot.y],
        source: match &behavior.source {
            RotationSource::AngleParameter(parameter) => WireRotationSource::Angle {
                parameter: parameter.raw(),
            },
            RotationSource::OptionMap { parameter, angles } => WireRotationSource::OptionMap {
                parameter: parameter.raw(),
                angles: angles
                    .iter()
                    .map(|(option, radians)| WireOptionAngle {
                        option: option.raw(),
                        radians: *radians,
                    })
                    .collect(),
            },
        },
    }
}

fn from_rotation(behavior: WireRotation) -> Result<RotationBehavior, ExportError> {
    Ok(RotationBehavior {
        id: ActionId(behavior.id),
        name: behavior.name,
        members: behavior.members.into_iter().map(EntityId).collect(),
        pivot: Point2::new(behavior.pivot[0], behavior.pivot[1]),
        source: match behavior.source {
            WireRotationSource::Angle { parameter } => {
                RotationSource::AngleParameter(ParameterId(parameter))
            }
            WireRotationSource::OptionMap { parameter, angles } => RotationSource::OptionMap {
                parameter: ParameterId(parameter),
                angles: angles
                    .into_iter()
                    .map(|item| (OptionId(item.option), item.radians))
                    .collect(),
            },
        },
    })
}

fn to_placement(behavior: &PlacementBehavior) -> WirePlacement {
    WirePlacement {
        id: behavior.id.raw(),
        name: behavior.name.clone(),
        members: behavior.members.iter().map(|id| id.raw()).collect(),
        attachment: [behavior.attachment.x, behavior.attachment.y],
        attachment_angle: behavior.attachment_angle,
        parameter: behavior.parameter.raw(),
        destinations: behavior
            .destinations
            .iter()
            .map(|(option, anchor)| WireOptionAnchor {
                option: option.raw(),
                anchor: anchor.raw(),
            })
            .collect(),
        boolean_destinations: behavior
            .boolean_destinations
            .map(|(off, on)| [off.raw(), on.raw()]),
    }
}

fn from_placement(behavior: WirePlacement) -> Result<PlacementBehavior, ExportError> {
    Ok(PlacementBehavior {
        id: ActionId(behavior.id),
        name: behavior.name,
        members: behavior.members.into_iter().map(EntityId).collect(),
        attachment: Point2::new(behavior.attachment[0], behavior.attachment[1]),
        attachment_angle: behavior.attachment_angle,
        parameter: ParameterId(behavior.parameter),
        destinations: behavior
            .destinations
            .into_iter()
            .map(|item| (OptionId(item.option), AnchorId(item.anchor)))
            .collect(),
        boolean_destinations: behavior
            .boolean_destinations
            .map(|ids| (AnchorId(ids[0]), AnchorId(ids[1]))),
    })
}

fn to_nested_input(input: &NestedInput) -> WireNestedInput {
    WireNestedInput {
        id: input.id.raw(),
        source: input.source.raw(),
        occurrence: input
            .target_occurrence
            .inserts
            .iter()
            .map(|id| id.raw())
            .collect(),
        target_parameter: input.target_parameter.raw(),
        mapping: match &input.mapping {
            NestedMapping::Direct => WireNestedMapping::Direct,
            NestedMapping::NumericScale { factor } => WireNestedMapping::NumericScale {
                factor: *factor,
            },
            NestedMapping::OptionMap(map) => WireNestedMapping::OptionMap {
                values: map
                    .iter()
                    .map(|(option, value)| WireOptionValue {
                        option: option.raw(),
                        value: to_wire_value(value),
                    })
                    .collect(),
            },
        },
    }
}

fn from_nested_input(input: WireNestedInput) -> Result<NestedInput, ExportError> {
    Ok(NestedInput {
        id: ActionId(input.id),
        source: ParameterId(input.source),
        target_occurrence: OccurrencePath {
            inserts: input.occurrence.into_iter().map(EntityId).collect(),
        },
        target_parameter: ParameterId(input.target_parameter),
        mapping: match input.mapping {
            WireNestedMapping::Direct => NestedMapping::Direct,
            WireNestedMapping::NumericScale { factor } => NestedMapping::NumericScale { factor },
            WireNestedMapping::OptionMap { values } => NestedMapping::OptionMap(
                values
                    .into_iter()
                    .map(|item| (OptionId(item.option), from_wire_value(item.value)))
                    .collect(),
            ),
        },
    })
}

fn to_compat(rule: &CompatibilityRule) -> WireCompatRule {
    match rule {
        CompatibilityRule::ChoiceAllowsChoice {
            id,
            when,
            when_option,
            target,
            allowed,
        } => WireCompatRule::ChoiceAllowsChoice {
            id: id.raw(),
            when: when.raw(),
            when_option: when_option.raw(),
            target: target.raw(),
            allowed: allowed.iter().map(|id| id.raw()).collect(),
        },
        CompatibilityRule::ChoiceRestrictsNumeric {
            id,
            when,
            when_option,
            target,
            min,
            max,
            allowed,
        } => WireCompatRule::ChoiceRestrictsNumeric {
            id: id.raw(),
            when: when.raw(),
            when_option: when_option.raw(),
            target: target.raw(),
            min: *min,
            max: *max,
            allowed: allowed.clone(),
        },
        CompatibilityRule::BooleanPermits {
            id,
            when,
            when_state,
            target,
            allowed_options,
            required_boolean,
        } => WireCompatRule::BooleanPermits {
            id: id.raw(),
            when: when.raw(),
            when_state: *when_state,
            target: target.raw(),
            allowed_options: allowed_options
                .as_ref()
                .map(|items| items.iter().map(|id| id.raw()).collect()),
            required_boolean: *required_boolean,
        },
    }
}

fn from_compat(rule: WireCompatRule) -> CompatibilityRule {
    match rule {
        WireCompatRule::ChoiceAllowsChoice {
            id,
            when,
            when_option,
            target,
            allowed,
        } => CompatibilityRule::ChoiceAllowsChoice {
            id: ActionId(id),
            when: ParameterId(when),
            when_option: OptionId(when_option),
            target: ParameterId(target),
            allowed: allowed.into_iter().map(OptionId).collect(),
        },
        WireCompatRule::ChoiceRestrictsNumeric {
            id,
            when,
            when_option,
            target,
            min,
            max,
            allowed,
        } => CompatibilityRule::ChoiceRestrictsNumeric {
            id: ActionId(id),
            when: ParameterId(when),
            when_option: OptionId(when_option),
            target: ParameterId(target),
            min,
            max,
            allowed,
        },
        WireCompatRule::BooleanPermits {
            id,
            when,
            when_state,
            target,
            allowed_options,
            required_boolean,
        } => CompatibilityRule::BooleanPermits {
            id: ActionId(id),
            when: ParameterId(when),
            when_state,
            target: ParameterId(target),
            allowed_options: allowed_options.map(|items| items.into_iter().map(OptionId).collect()),
            required_boolean,
        },
    }
}

fn to_preset(preset: &Preset) -> WirePreset {
    WirePreset {
        id: preset.id.raw(),
        name: preset.name.clone(),
        values: preset
            .values
            .iter()
            .map(|(id, value)| WireInstanceValue {
                parameter: id.raw(),
                value: to_wire_value(value),
            })
            .collect(),
    }
}

fn from_preset(preset: WirePreset) -> Preset {
    Preset {
        id: PresetId(preset.id),
        name: preset.name,
        values: preset
            .values
            .into_iter()
            .map(|item| (ParameterId(item.parameter), from_wire_value(item.value)))
            .collect(),
    }
}

fn to_wire_value(value: &ParameterValue) -> WireValue {
    match value {
        ParameterValue::Number(v) => WireValue::Number { value: *v },
        ParameterValue::Choice(option) => WireValue::Choice {
            option: option.raw(),
        },
        ParameterValue::Boolean(v) => WireValue::Boolean { value: *v },
        ParameterValue::Text(v) => WireValue::Text { value: v.clone() },
    }
}

fn from_wire_value(value: WireValue) -> ParameterValue {
    match value {
        WireValue::Number { value } => ParameterValue::Number(value),
        WireValue::Choice { option } => ParameterValue::Choice(OptionId(option)),
        WireValue::Boolean { value } => ParameterValue::Boolean(value),
        WireValue::Text { value } => ParameterValue::Text(value),
    }
}

fn follow_name(role: FollowRole) -> &'static str {
    match role {
        FollowRole::First => "first",
        FollowRole::Second => "second",
        FollowRole::Center => "center",
        FollowRole::Custom => "custom",
    }
}

fn parse_follow(role: &str) -> Result<FollowRole, ExportError> {
    match role {
        "first" => Ok(FollowRole::First),
        "second" => Ok(FollowRole::Second),
        "center" => Ok(FollowRole::Center),
        "custom" | "" => Ok(FollowRole::Custom),
        other => Err(ExportError::Validation(format!(
            "unknown follow role '{other}'"
        ))),
    }
}

fn to_behavior(behavior: &DynamicBehavior) -> WireBehavior {
    WireBehavior {
        id: behavior.id.raw(),
        kind: match behavior.kind {
            cad_core::BehaviorKind::Move => "move".into(),
            cad_core::BehaviorKind::Stretch => "stretch".into(),
        },
        parameter: behavior.parameter.raw(),
        targets: behavior.targets.iter().map(to_target).collect(),
        local_direction: [behavior.local_direction.x, behavior.local_direction.y],
        reference_value: behavior.reference_value,
        multiplier: behavior.multiplier,
        composition: "additive".into(),
        follow: Some(match behavior.follow {
            FollowRole::First => "first".into(),
            FollowRole::Second => "second".into(),
            FollowRole::Center => "center".into(),
            FollowRole::Custom => "custom".into(),
        }),
        name: behavior.name.clone(),
    }
}

fn from_behavior(behavior: WireBehavior) -> Result<DynamicBehavior, ExportError> {
    Ok(DynamicBehavior {
        id: ActionId(behavior.id),
        kind: match behavior.kind.as_str() {
            "move" => cad_core::BehaviorKind::Move,
            "stretch" => cad_core::BehaviorKind::Stretch,
            other => {
                return Err(ExportError::Validation(format!(
                    "unknown behavior '{other}'"
                )))
            }
        },
        parameter: ParameterId(behavior.parameter),
        targets: behavior.targets.into_iter().map(from_target).collect(),
        local_direction: cad_core::Point2::new(
            behavior.local_direction[0],
            behavior.local_direction[1],
        ),
        reference_value: behavior.reference_value,
        multiplier: behavior.multiplier,
        composition: match behavior.composition.as_str() {
            "additive" | "" => CompositionRule::Additive,
            other => {
                return Err(ExportError::Validation(format!(
                    "unknown composition '{other}'"
                )))
            }
        },
        follow: match behavior.follow.as_deref() {
            Some("first") => FollowRole::First,
            Some("second") => FollowRole::Second,
            Some("center") => FollowRole::Center,
            Some("custom") | None | Some("") => FollowRole::Custom,
            Some(other) => {
                return Err(ExportError::Validation(format!(
                    "unknown follow role '{other}'"
                )))
            }
        },
        name: behavior.name,
    })
}

fn to_target(target: &GeometryTarget) -> WireTarget {
    match *target {
        GeometryTarget::Entity(id) => WireTarget::Entity { id: id.raw() },
        GeometryTarget::LineStart(id) => WireTarget::LineStart { id: id.raw() },
        GeometryTarget::LineEnd(id) => WireTarget::LineEnd { id: id.raw() },
        GeometryTarget::Vertex { entity, vertex } => WireTarget::Vertex {
            entity: entity.raw(),
            vertex: vertex.raw(),
        },
    }
}

fn from_target(target: WireTarget) -> GeometryTarget {
    match target {
        WireTarget::Entity { id } => GeometryTarget::Entity(EntityId(id)),
        WireTarget::LineStart { id } => GeometryTarget::LineStart(EntityId(id)),
        WireTarget::LineEnd { id } => GeometryTarget::LineEnd(EntityId(id)),
        WireTarget::Vertex { entity, vertex } => GeometryTarget::Vertex {
            entity: EntityId(entity),
            vertex: VertexId(vertex),
        },
    }
}

fn to_domain(domain: &NumericDomain) -> WireDomain {
    match domain {
        NumericDomain::Continuous => WireDomain::Continuous,
        NumericDomain::AllowedValues(values) => WireDomain::AllowedValues {
            values: values.clone(),
        },
    }
}

fn from_domain(domain: Option<WireDomain>) -> NumericDomain {
    match domain {
        None | Some(WireDomain::Continuous) => NumericDomain::Continuous,
        Some(WireDomain::AllowedValues { values }) => NumericDomain::AllowedValues(values),
    }
}

fn to_size(size: &SizeAuthoring) -> WireSizeAuthoring {
    WireSizeAuthoring {
        point_a: [size.point_a.x, size.point_a.y],
        point_b: [size.point_b.x, size.point_b.y],
        measure: match size.measure {
            MeasureMode::AlongPicked => "along".into(),
            MeasureMode::LocalX => "local_x".into(),
            MeasureMode::LocalY => "local_y".into(),
        },
        direction: [size.direction.x, size.direction.y],
        anchor: match size.anchor {
            AnchorPolicy::FirstFixed => "first".into(),
            AnchorPolicy::SecondFixed => "second".into(),
            AnchorPolicy::CenterFixed => "center".into(),
        },
        label_offset: [size.label_offset.x, size.label_offset.y],
        bound_anchor: size.bound_anchor.as_ref().map(to_target),
    }
}

fn from_size(size: WireSizeAuthoring) -> Result<SizeAuthoring, ExportError> {
    Ok(SizeAuthoring {
        point_a: cad_core::Point2::new(size.point_a[0], size.point_a[1]),
        point_b: cad_core::Point2::new(size.point_b[0], size.point_b[1]),
        measure: match size.measure.as_str() {
            "along" | "along_picked" => MeasureMode::AlongPicked,
            "local_x" | "x" => MeasureMode::LocalX,
            "local_y" | "y" => MeasureMode::LocalY,
            other => {
                return Err(ExportError::Validation(format!(
                    "unknown measure mode '{other}'"
                )))
            }
        },
        direction: cad_core::Point2::new(size.direction[0], size.direction[1]),
        anchor: match size.anchor.as_str() {
            "first" => AnchorPolicy::FirstFixed,
            "second" => AnchorPolicy::SecondFixed,
            "center" => AnchorPolicy::CenterFixed,
            other => {
                return Err(ExportError::Validation(format!(
                    "unknown anchor '{other}'"
                )))
            }
        },
        label_offset: cad_core::Point2::new(size.label_offset[0], size.label_offset[1]),
        bound_anchor: size.bound_anchor.map(from_target),
    })
}

fn to_config(config: &InstanceConfiguration) -> WireConfig {
    WireConfig {
        values: config
            .values
            .iter()
            .map(|(id, value)| WireInstanceValue {
                parameter: id.raw(),
                value: match value {
                    ParameterValue::Number(v) => WireValue::Number { value: *v },
                    ParameterValue::Choice(option) => WireValue::Choice {
                        option: option.raw(),
                    },
                    ParameterValue::Boolean(v) => WireValue::Boolean { value: *v },
                    ParameterValue::Text(v) => WireValue::Text { value: v.clone() },
                },
            })
            .collect(),
    }
}

fn from_config(config: WireConfig) -> InstanceConfiguration {
    let mut values = BTreeMap::new();
    for item in config.values {
        values.insert(
            ParameterId(item.parameter),
            match item.value {
                WireValue::Number { value } => ParameterValue::Number(value),
                WireValue::Choice { option } => ParameterValue::Choice(OptionId(option)),
                WireValue::Boolean { value } => ParameterValue::Boolean(value),
                WireValue::Text { value } => ParameterValue::Text(value),
            },
        );
    }
    InstanceConfiguration { values }
}

fn to_color(color: CadColor) -> WireColor {
    match color {
        CadColor::ByLayer => WireColor::ByLayer,
        CadColor::ByBlock => WireColor::ByBlock,
        CadColor::Aci(index) => WireColor::Aci { index },
        CadColor::Rgb { r, g, b } => WireColor::Rgb { r, g, b },
    }
}

fn from_color(color: WireColor) -> CadColor {
    match color {
        WireColor::ByLayer => CadColor::ByLayer,
        WireColor::ByBlock => CadColor::ByBlock,
        WireColor::Aci { index } => CadColor::Aci(index),
        WireColor::Rgb { r, g, b } => CadColor::Rgb { r, g, b },
    }
}

fn to_text(data: &TextData) -> WireText {
    WireText {
        insertion: pt(data.insertion),
        height: data.height,
        rotation: data.rotation,
        value: data.value.clone(),
        extrusion: pt(data.extrusion),
        is_attrib_def: data.is_attrib_def,
    }
}

fn from_text(data: WireText) -> TextData {
    TextData {
        insertion: from_pt(data.insertion),
        height: data.height,
        rotation: data.rotation,
        value: data.value,
        extrusion: from_pt(data.extrusion),
        is_attrib_def: data.is_attrib_def,
    }
}

fn to_vertex(vertex: &PolyVertex) -> WireVertex {
    WireVertex {
        point: pt(vertex.point),
        bulge: vertex.bulge,
        vertex_id: vertex.vertex_id.raw(),
    }
}

fn from_vertex(vertex: WireVertex) -> PolyVertex {
    PolyVertex {
        point: from_pt(vertex.point),
        bulge: vertex.bulge,
        vertex_id: VertexId(vertex.vertex_id),
    }
}

fn to_hatch_path(path: &HatchPath) -> WireHatchPath {
    match path {
        HatchPath::Polyline { vertices, closed } => WireHatchPath::Polyline {
            vertices: vertices.iter().map(to_vertex).collect(),
            closed: *closed,
        },
        HatchPath::Edges(edges) => WireHatchPath::Edges {
            edges: edges.iter().map(to_hatch_edge).collect(),
        },
    }
}

fn from_hatch_path(path: WireHatchPath) -> HatchPath {
    match path {
        WireHatchPath::Polyline { vertices, closed } => HatchPath::Polyline {
            vertices: vertices.into_iter().map(from_vertex).collect(),
            closed,
        },
        WireHatchPath::Edges { edges } => {
            HatchPath::Edges(edges.into_iter().map(from_hatch_edge).collect())
        }
    }
}

fn to_hatch_edge(edge: &HatchEdge) -> WireHatchEdge {
    match edge {
        HatchEdge::Line { start, end } => WireHatchEdge::Line {
            start: pt(*start),
            end: pt(*end),
        },
        HatchEdge::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            is_ccw,
        } => WireHatchEdge::Arc {
            center: pt(*center),
            radius: *radius,
            start_angle: *start_angle,
            end_angle: *end_angle,
            is_ccw: *is_ccw,
        },
        HatchEdge::Ellipse {
            center,
            major_endpoint,
            axis_ratio,
            start_angle,
            end_angle,
            is_ccw,
        } => WireHatchEdge::Ellipse {
            center: pt(*center),
            major_endpoint: pt(*major_endpoint),
            axis_ratio: *axis_ratio,
            start_angle: *start_angle,
            end_angle: *end_angle,
            is_ccw: *is_ccw,
        },
        HatchEdge::Spline { control_points } => WireHatchEdge::Spline {
            control_points: control_points.iter().copied().map(pt).collect(),
        },
    }
}

fn from_hatch_edge(edge: WireHatchEdge) -> HatchEdge {
    match edge {
        WireHatchEdge::Line { start, end } => HatchEdge::Line {
            start: from_pt(start),
            end: from_pt(end),
        },
        WireHatchEdge::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            is_ccw,
        } => HatchEdge::Arc {
            center: from_pt(center),
            radius,
            start_angle,
            end_angle,
            is_ccw,
        },
        WireHatchEdge::Ellipse {
            center,
            major_endpoint,
            axis_ratio,
            start_angle,
            end_angle,
            is_ccw,
        } => HatchEdge::Ellipse {
            center: from_pt(center),
            major_endpoint: from_pt(major_endpoint),
            axis_ratio,
            start_angle,
            end_angle,
            is_ccw,
        },
        WireHatchEdge::Spline { control_points } => HatchEdge::Spline {
            control_points: control_points.into_iter().map(from_pt).collect(),
        },
    }
}

fn to_pattern(line: &HatchPatternLine) -> WirePatternLine {
    WirePatternLine {
        angle: line.angle,
        base: pt(line.base),
        offset: pt(line.offset),
        dashes: line.dashes.clone(),
    }
}

fn from_pattern(line: WirePatternLine) -> HatchPatternLine {
    HatchPatternLine {
        angle: line.angle,
        base: from_pt(line.base),
        offset: from_pt(line.offset),
        dashes: line.dashes,
    }
}

fn to_unit(unit: ParameterUnit) -> WireUnit {
    match unit {
        ParameterUnit::Drawing(units) => WireUnit::Drawing {
            code: units.to_insunits(),
        },
        ParameterUnit::Degrees => WireUnit::Degrees,
        ParameterUnit::Radians => WireUnit::Radians,
        ParameterUnit::Count => WireUnit::Count,
        ParameterUnit::None => WireUnit::None,
    }
}

fn from_unit(unit: WireUnit) -> ParameterUnit {
    match unit {
        WireUnit::Drawing { code } => ParameterUnit::Drawing(DrawingUnits::from_insunits(code)),
        WireUnit::Degrees => ParameterUnit::Degrees,
        WireUnit::Radians => ParameterUnit::Radians,
        WireUnit::Count => ParameterUnit::Count,
        WireUnit::None => ParameterUnit::None,
    }
}

fn quantity_name(quantity: NumericQuantity) -> &'static str {
    match quantity {
        NumericQuantity::Length => "length",
        NumericQuantity::Distance => "distance",
        NumericQuantity::Angle => "angle",
        NumericQuantity::Count => "count",
        NumericQuantity::Dimensionless => "dimensionless",
    }
}

fn parse_quantity(name: &str) -> Result<NumericQuantity, ExportError> {
    match name {
        "length" => Ok(NumericQuantity::Length),
        "distance" => Ok(NumericQuantity::Distance),
        "angle" => Ok(NumericQuantity::Angle),
        "count" => Ok(NumericQuantity::Count),
        "dimensionless" => Ok(NumericQuantity::Dimensionless),
        other => Err(ExportError::Validation(format!(
            "unknown quantity '{other}'"
        ))),
    }
}

fn parse_step_policy(name: &str) -> Result<StepPolicy, ExportError> {
    match name {
        "increment_only" => Ok(StepPolicy::IncrementOnly),
        "required_increment" => Ok(StepPolicy::RequiredIncrement),
        other => Err(ExportError::Validation(format!(
            "unknown step policy '{other}'"
        ))),
    }
}

fn parse_step_origin(name: &str) -> Result<StepOrigin, ExportError> {
    match name {
        "minimum" => Ok(StepOrigin::Minimum),
        "zero" => Ok(StepOrigin::Zero),
        other => Err(ExportError::Validation(format!(
            "unknown step origin '{other}'"
        ))),
    }
}

fn pt(point: Point3) -> [f64; 3] {
    [point.x, point.y, point.z]
}

fn from_pt(point: [f64; 3]) -> Point3 {
    Point3::new(point[0], point[1], point[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_core::{
        identity_insert, primitives_document, AnchorPolicy, BehaviorKind, FollowRole, GeometryTarget,
        MeasureMode, NumericDomain, NumericParameter, ParameterDef, ParameterKind, Point2,
        SizeAuthoring,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp(name: &str) -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("mycad-native-{stamp}-{name}"))
    }

    #[test]
    fn primitives_roundtrip_preserves_identities() {
        let mut document = primitives_document();
        document.assign_missing_ids();
        let path = temp("primitives.mycad");
        write_mycad(&document, &path).expect("write");
        let loaded = read_mycad(&path).expect("read");
        let _ = std::fs::remove_file(&path);
        assert_eq!(loaded.model_space.len(), document.model_space.len());
        assert_eq!(loaded.blocks.len(), document.blocks.len());
        assert_eq!(loaded.model_space[0].id, document.model_space[0].id);
    }

    #[test]
    fn dynamic_values_survive_save_and_reopen() {
        let mut document = Document::default();
        let param = document.allocate_parameter_id();
        let mut line = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(800.0, 0.0),
        });
        line.id = document.allocate_id();
        let mut numeric = NumericParameter::length(800.0);
        numeric.reference = 800.0;
        let mut definition = BlockDefinition::plain(
            "AdjustableFrame",
            Point3::from_xy(0.0, 0.0),
            vec![line.clone()],
        );
        definition.dynamic = Some(DynamicDefinition {
            parameters: vec![ParameterDef::number(param, "Span", numeric)],
            behaviors: vec![DynamicBehavior {
                id: document.allocate_action_id(),
                kind: BehaviorKind::Stretch,
                parameter: param,
                targets: vec![GeometryTarget::LineEnd(line.id)],
                local_direction: Point2::new(1.0, 0.0),
                reference_value: 800.0,
                multiplier: 1.0,
                composition: CompositionRule::Additive,
                follow: FollowRole::Second,
                name: None,
            }],
            ..Default::default()
        });
        document.replace_block_definition(definition);
        let mut insert = Entity::new(identity_insert(
            "AdjustableFrame".into(),
            Point3::from_xy(0.0, 0.0),
        ));
        let mut config = InstanceConfiguration::default();
        config.set(param, ParameterValue::Number(1200.0));
        insert.geometry.set_insert_configuration(Some(config));
        document.add_entity(insert);
        let path = temp("frame.mycad");
        write_mycad(&document, &path).expect("write");
        let loaded = read_mycad(&path).expect("read");
        let _ = std::fs::remove_file(&path);
        let def = loaded.block_by_name("AdjustableFrame").unwrap();
        assert_eq!(def.dynamic.as_ref().unwrap().parameters[0].name, "Span");
        assert_eq!(def.dynamic.as_ref().unwrap().parameters[0].id, param);
        let value = loaded.model_space[0]
            .geometry
            .insert_configuration()
            .unwrap()
            .get(param)
            .unwrap()
            .as_number()
            .unwrap();
        assert!((value - 1200.0).abs() < 1e-12);
    }

    #[test]
    fn phase3_choice_visibility_text_and_preset_roundtrip() {
        let mut document = Document::default();
        let style = document.allocate_parameter_id();
        let first = document.allocate_option_id();
        let second = document.allocate_option_id();
        let mut alt_a = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(10.0, 0.0),
        });
        let mut alt_b = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 20.0),
            end: Point3::from_xy(10.0, 20.0),
        });
        let mut label = Entity::new(Geometry::Text(TextData {
            insertion: Point3::from_xy(0.0, 5.0),
            height: 2.5,
            rotation: 0.0,
            value: "source".into(),
            extrusion: cad_core::default_extrusion(),
            is_attrib_def: false,
        }));
        alt_a.id = document.allocate_id();
        alt_b.id = document.allocate_id();
        label.id = document.allocate_id();
        let vis = document.allocate_action_id();
        let bind = document.allocate_action_id();
        let preset = document.allocate_preset_id();
        let mut definition = BlockDefinition::plain(
            "Assembly",
            Point3::from_xy(0.0, 0.0),
            vec![alt_a.clone(), alt_b.clone(), label.clone()],
        );
        definition.dynamic = Some(DynamicDefinition {
            parameters: vec![ParameterDef::choice(
                style,
                "Style",
                ChoiceParameter {
                    options: vec![
                        ChoiceOption {
                            id: first,
                            label: "Standard".into(),
                        },
                        ChoiceOption {
                            id: second,
                            label: "Reinforced".into(),
                        },
                    ],
                    default: first,
                },
            )],
            visibility: vec![VisibilityGroup {
                id: vis,
                name: "Alt B".into(),
                members: vec![alt_b.id],
                conditions: vec![ParameterCondition::Choice {
                    parameter: style,
                    options: vec![second],
                }],
            }],
            text_bindings: vec![TextBinding {
                id: bind,
                target: label.id,
                mode: TextBindingMode::ShowValue { parameter: style },
                boolean_true: "On".into(),
                boolean_false: "Off".into(),
                number_precision: None,
            }],
            presets: vec![Preset {
                id: preset,
                name: "Reinforced".into(),
                values: [(style, ParameterValue::Choice(second))]
                    .into_iter()
                    .collect(),
            }],
            ..Default::default()
        });
        document.replace_block_definition(definition);
        let mut insert = Entity::new(identity_insert("Assembly".into(), Point3::from_xy(0.0, 0.0)));
        let mut config = InstanceConfiguration::default();
        config.set(style, ParameterValue::Choice(second));
        insert.geometry.set_insert_configuration(Some(config));
        document.add_entity(insert);
        let path = temp("phase3.mycad");
        write_mycad(&document, &path).expect("write");
        let loaded = read_mycad(&path).expect("read");
        let _ = std::fs::remove_file(&path);
        let def = loaded.block_by_name("Assembly").unwrap().dynamic.as_ref().unwrap();
        assert_eq!(def.parameters[0].id, style);
        match &def.parameters[0].kind {
            ParameterKind::Choice(choice) => {
                assert_eq!(choice.options[0].id, first);
                assert_eq!(choice.options[1].label, "Reinforced");
            }
            _ => panic!("choice"),
        }
        assert_eq!(def.visibility[0].members[0], alt_b.id);
        assert_eq!(def.text_bindings[0].target, label.id);
        assert_eq!(def.presets[0].id, preset);
        assert_eq!(
            loaded.model_space[0]
                .geometry
                .insert_configuration()
                .unwrap()
                .get(style),
            Some(&ParameterValue::Choice(second))
        );
    }

    #[test]
    fn size_metadata_domain_and_vertex_targets_roundtrip() {
        let mut document = Document::default();
        let param = document.allocate_parameter_id();
        let vertex = document.allocate_vertex_id();
        let mut polyline = Entity::new(Geometry::LwPolyline {
            vertices: vec![
                cad_core::PolyVertex {
                    point: Point3::from_xy(0.0, 0.0),
                    bulge: 0.0,
                    vertex_id: vertex,
                },
                cad_core::PolyVertex {
                    point: Point3::from_xy(800.0, 0.0),
                    bulge: 0.0,
                    vertex_id: document.allocate_vertex_id(),
                },
            ],
            closed: false,
            extrusion: Point3::new(0.0, 0.0, 1.0),
            linetype_generation_continuous: false,
        });
        polyline.id = document.allocate_id();
        let mut numeric = NumericParameter::length(800.0);
        numeric.reference = 800.0;
        numeric.default = 800.0;
        numeric.domain = NumericDomain::AllowedValues(vec![250.0, 400.0, 800.0]);
        numeric.size = Some(SizeAuthoring {
            point_a: Point2::new(0.0, 0.0),
            point_b: Point2::new(800.0, 0.0),
            measure: MeasureMode::LocalX,
            direction: Point2::new(1.0, 0.0),
            anchor: AnchorPolicy::FirstFixed,
            label_offset: Point2::new(0.0, 12.0),
            bound_anchor: None,
        });
        let mut definition = BlockDefinition::plain(
            "AdjustableFrame",
            Point3::from_xy(0.0, 0.0),
            vec![polyline.clone()],
        );
        definition.dynamic = Some(DynamicDefinition {
            parameters: vec![ParameterDef::number(param, "Span", numeric)],
            behaviors: vec![DynamicBehavior {
                id: document.allocate_action_id(),
                kind: BehaviorKind::Stretch,
                parameter: param,
                targets: vec![GeometryTarget::Vertex {
                    entity: polyline.id,
                    vertex,
                }],
                local_direction: Point2::new(1.0, 0.0),
                reference_value: 800.0,
                multiplier: 1.0,
                composition: CompositionRule::Additive,
                follow: FollowRole::Second,
                name: Some("right rail".into()),
            }],
            ..Default::default()
        });
        document.replace_block_definition(definition);
        let path = temp("size-meta.mycad");
        write_mycad(&document, &path).expect("write");
        let loaded = read_mycad(&path).expect("read");
        let _ = std::fs::remove_file(&path);
        let def = loaded.block_by_name("AdjustableFrame").unwrap();
        let numeric = match &def.dynamic.as_ref().unwrap().parameters[0].kind {
            ParameterKind::Number(numeric) => numeric,
            _ => panic!("number"),
        };
        assert!(matches!(numeric.domain, NumericDomain::AllowedValues(ref values) if values.len() == 3));
        let size = numeric.size.as_ref().expect("size metadata");
        assert_eq!(size.measure, MeasureMode::LocalX);
        assert_eq!(size.anchor, AnchorPolicy::FirstFixed);
        assert!((size.point_b.x - 800.0).abs() < 1e-12);
        assert!((size.label_offset.y - 12.0).abs() < 1e-12);
        assert_eq!(
            def.dynamic.as_ref().unwrap().behaviors[0].targets[0],
            GeometryTarget::Vertex {
                entity: polyline.id,
                vertex,
            }
        );
        assert_eq!(
            def.dynamic.as_ref().unwrap().behaviors[0].name.as_deref(),
            Some("right rail")
        );
    }

    #[test]
    fn block_asset_roundtrip_remaps_identities_on_import() {
        let mut document = Document::default();
        let param = document.allocate_parameter_id();
        let mut line = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(10.0, 0.0),
        });
        line.id = document.allocate_id();
        let mut numeric = NumericParameter::length(10.0);
        numeric.reference = 10.0;
        let mut definition = BlockDefinition::plain(
            "OffsetSymbol",
            Point3::from_xy(0.0, 0.0),
            vec![line.clone()],
        );
        definition.dynamic = Some(DynamicDefinition {
            parameters: vec![ParameterDef::number(param, "Offset", numeric)],
            behaviors: vec![DynamicBehavior {
                id: document.allocate_action_id(),
                kind: BehaviorKind::Stretch,
                parameter: param,
                targets: vec![GeometryTarget::LineEnd(line.id)],
                local_direction: Point2::new(1.0, 0.0),
                reference_value: 10.0,
                multiplier: 1.0,
                composition: CompositionRule::Additive,
                follow: FollowRole::Second,
                name: None,
            }],
            ..Default::default()
        });
        document.replace_block_definition(definition);
        let original_id = document.block_by_name("OffsetSymbol").unwrap().id;
        let original_param = document
            .block_by_name("OffsetSymbol")
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
            .parameters[0]
            .id;
        let original_entity = document.block_by_name("OffsetSymbol").unwrap().entities[0].id;
        let path = temp("symbol.mycadblock");
        write_mycadblock(&document, "OffsetSymbol", &path).expect("write");
        let asset = read_mycadblock(&path).expect("read");
        let _ = std::fs::remove_file(&path);
        let mut host = document.clone();
        let name = import_block_asset(&mut host, asset).expect("import");
        let imported = host.block_by_name(&name).unwrap();
        assert_ne!(imported.id, original_id);
        assert_ne!(name, "OffsetSymbol");
        assert_eq!(imported.dynamic.as_ref().unwrap().parameters[0].name, "Offset");
        assert_ne!(imported.dynamic.as_ref().unwrap().parameters[0].id, original_param);
        assert_ne!(imported.entities[0].id, original_entity);
        assert_eq!(
            imported.dynamic.as_ref().unwrap().behaviors[0].targets[0].entity_id(),
            imported.entities[0].id
        );
    }

    #[test]
    fn schema_1_numeric_without_domain_loads_as_continuous() {
        let json = r#"{"format":"mycad","schema":1,"document":{"units":0,"ltscale":1,"current_layer":"0","next_entity_id":3,"next_definition_id":2,"next_parameter_id":2,"next_option_id":1,"next_action_id":2,"content_generation":1,"saved_revision":0,"layers":[],"linetypes":[],"blocks":[{"id":1,"name":"Frame","base_pt":[0,0,0],"content_revision":1,"entities":[{"id":1,"layer":"0","color":{"kind":"ByLayer"},"linetype":"BYLAYER","linetype_scale":1,"visible":true,"geometry":{"type":"Line","start":[0,0,0],"end":[10,0,0]}}],"dynamic":{"parameters":[{"id":1,"name":"Span","description":null,"kind":{"type":"Number","quantity":"length","unit":{"kind":"None"},"default":10,"reference":10,"min":null,"max":null,"step":null,"step_policy":"increment_only","step_origin":"minimum","display_precision":4,"display_order":0}}],"behaviors":[{"id":1,"kind":"stretch","parameter":1,"targets":[{"kind":"LineEnd","id":1}],"local_direction":[1,0],"reference_value":10,"multiplier":1,"composition":"additive"}]}}],"model_space":[]}}"#;
        let loaded = parse_mycad_bytes(json.as_bytes()).expect("schema-1");
        let def = loaded.block_by_name("Frame").unwrap();
        let numeric = match &def.dynamic.as_ref().unwrap().parameters[0].kind {
            ParameterKind::Number(numeric) => numeric,
            _ => panic!("number"),
        };
        assert!(matches!(numeric.domain, NumericDomain::Continuous));
        assert!(numeric.size.is_none());
        assert_eq!(def.dynamic.as_ref().unwrap().behaviors[0].follow, FollowRole::Custom);
    }

    #[test]
    fn unsupported_schema_is_rejected() {
        let json = r#"{"format":"mycad","schema":99,"document":{"units":0,"ltscale":1,"current_layer":"0","next_entity_id":1,"next_definition_id":1,"next_parameter_id":1,"next_option_id":1,"next_action_id":1,"content_generation":1,"saved_revision":0,"layers":[],"linetypes":[],"blocks":[],"model_space":[]}}"#;
        let err = parse_mycad_bytes(json.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("unsupported MyCAD schema"));
    }

    #[test]
    fn failed_save_leaves_the_previous_file() {
        let path = temp("keep.mycad");
        std::fs::write(&path, b"PREVIOUS").unwrap();
        let as_dir = path.clone();
        // write_atomic cannot replace a path that is currently a directory.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&as_dir).unwrap();
        let document = Document::default();
        assert!(write_mycad(&document, &as_dir).is_err());
        assert!(as_dir.is_dir());
        let _ = std::fs::remove_dir(&as_dir);
    }
}
