//! Sidecar data EntoCAD keeps next to a DWG or DXF.
//!
//! `Plant.dwg` stays a normal static-block drawing. `Plant.dwg.mycad` stores
//! the dynamic block definitions and which flattened block stands for which
//! instance. Opening the DWG without the companion still shows the geometry.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cad_core::{
    BlockDefinition, Document, DynamicExportLink, Entity, Geometry, HatchEdge, HatchPath,
    InstanceConfiguration, ParameterKind,
};
use serde::{Deserialize, Serialize};

use crate::error::ExportError;
use crate::native::{
    from_config, from_wire_block, to_config, to_wire_block, WireBlock, WireConfig,
};
use crate::options::SaveReport;

pub const COMPANION_FORMAT: &str = "mycad-companion";
pub const COMPANION_SCHEMA: u32 = 1;

const FINGERPRINT_SCALE: f64 = 10_000.0;

// ------------------------------------------------------------
// Type: Companion
// Purpose: Parsed sidecar. The DWG remains the geometry master.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Companion {
    blocks: Vec<BlockDefinition>,
    instances: Vec<CompanionInstance>,
    fingerprints: BTreeMap<String, BlockFingerprint>,
    generated: Vec<String>,
}

#[derive(Debug, Clone)]
struct CompanionInstance {
    export_block: String,
    source_block: String,
    configuration: InstanceConfiguration,
    fingerprint: BlockFingerprint,
}

// ------------------------------------------------------------
// Type: CompanionReport
// Purpose: How many inserts became dynamic again, and how many
//          flattened blocks were left alone because they no longer
//          match the file EntoCAD wrote.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionReport {
    pub restored: usize,
    pub skipped: usize,
    pub warning: Option<String>,
}

impl CompanionReport {
    pub fn status_line(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.restored > 0 {
            let noun = if self.restored == 1 {
                "block"
            } else {
                "blocks"
            };
            parts.push(format!("Restored {} dynamic {noun}", self.restored));
        }
        if self.skipped == 1 {
            parts.push("1 block edited outside EntoCAD stays static".into());
        } else if self.skipped > 1 {
            parts.push(format!(
                "{} blocks edited outside EntoCAD stay static",
                self.skipped
            ));
        }
        if let Some(warning) = &self.warning {
            parts.push(warning.clone());
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" • "))
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct BlockFingerprint {
    entities: u64,
    min: Option<[f64; 2]>,
    max: Option<[f64; 2]>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireCompanion {
    format: String,
    schema: u32,
    blocks: Vec<WireBlock>,
    instances: Vec<WireInstance>,
    #[serde(default)]
    block_fingerprints: BTreeMap<String, WireFingerprint>,
    #[serde(default)]
    generated: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireInstance {
    export_block: String,
    source_block: String,
    configuration: WireConfig,
    fingerprint: WireFingerprint,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct WireFingerprint {
    entities: u64,
    #[serde(default)]
    min: Option<[f64; 2]>,
    #[serde(default)]
    max: Option<[f64; 2]>,
}

struct IdMaps {
    parameters: BTreeMap<cad_core::ParameterId, cad_core::ParameterId>,
    options: BTreeMap<cad_core::OptionId, cad_core::OptionId>,
}

struct PreparedBlock {
    block: BlockDefinition,
    anchors: BTreeMap<cad_core::AnchorId, cad_core::AnchorId>,
    presets: BTreeMap<cad_core::PresetId, cad_core::PresetId>,
    vertices: BTreeMap<cad_core::VertexId, cad_core::VertexId>,
}

// ------------------------------------------------------------
// Function: companion_path
// Purpose: Plant.dwg → Plant.dwg.mycad. Appending keeps the DWG
//          extension, so the sidecar cannot be mistaken for an
//          older full .mycad drawing.
// ------------------------------------------------------------
pub fn companion_path(cad_path: &Path) -> PathBuf {
    let mut name = cad_path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".mycad");
    cad_path.with_file_name(name)
}

// ------------------------------------------------------------
// Function: drawing_for_companion
// Purpose: Opening Plant.dwg.mycad loads Plant.dwg instead of
//          treating the sidecar as a full drawing.
// ------------------------------------------------------------
pub fn drawing_for_companion(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let lower = name.to_ascii_lowercase();
    if !(lower.ends_with(".dwg.mycad") || lower.ends_with(".dxf.mycad")) {
        return None;
    }
    let drawing = &name[..name.len() - ".mycad".len()];
    if drawing.is_empty() {
        return None;
    }
    Some(path.with_file_name(drawing))
}

// ------------------------------------------------------------
// Function: write_companion
// Purpose: Store dynamic definitions, instance links, and the
//          fingerprints of the blocks the DWG actually contains.
// ------------------------------------------------------------
pub fn write_companion(
    path: &Path,
    source: &Document,
    exported: &Document,
    links: &[DynamicExportLink],
    generated_blocks: &[String],
) -> Result<SaveReport, ExportError> {
    let blocks = source_blocks(source);
    let mut fingerprints = BTreeMap::new();
    for name in generated_blocks {
        if let Some(block) = exported.block_by_name(name) {
            fingerprints.insert(block.name.clone(), fingerprint_block(block));
        }
    }
    for block in &blocks {
        if let Some(exported_block) = exported.block_by_name(&block.name) {
            fingerprints.insert(
                exported_block.name.clone(),
                fingerprint_block(exported_block),
            );
        }
    }
    let instances = links
        .iter()
        .filter_map(|link| {
            let fingerprint = lookup_fingerprint(&fingerprints, &link.export_block)?.clone();
            Some(WireInstance {
                export_block: link.export_block.clone(),
                source_block: link.source_block.clone(),
                configuration: to_config(&link.configuration),
                fingerprint: to_wire_fingerprint(&fingerprint),
            })
        })
        .collect();
    let wire = WireCompanion {
        format: COMPANION_FORMAT.into(),
        schema: COMPANION_SCHEMA,
        blocks: blocks.iter().map(to_wire_block).collect(),
        instances,
        block_fingerprints: fingerprints
            .iter()
            .map(|(name, fingerprint)| (name.clone(), to_wire_fingerprint(fingerprint)))
            .collect(),
        generated: generated_blocks.to_vec(),
    };
    let bytes = serde_json::to_vec_pretty(&wire)
        .map_err(|err| ExportError::Validation(format!("serialize failed: {err}")))?;
    crate::atomic::write_atomic(path, &bytes).map_err(|source| ExportError::io(path, source))?;
    Ok(SaveReport::default())
}

pub fn read_companion(path: &Path) -> Result<Companion, ExportError> {
    let bytes = std::fs::read(path).map_err(|source| ExportError::io(path, source))?;
    parse_companion_bytes(&bytes)
}

// ------------------------------------------------------------
// Function: read_companion_optional
// Purpose: A missing sidecar is not an error. The drawing opens
//          with static blocks.
// ------------------------------------------------------------
pub fn read_companion_optional(path: &Path) -> Result<Option<Companion>, ExportError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(parse_companion_bytes(&bytes)?)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(ExportError::io(path, err)),
    }
}

fn parse_companion_bytes(bytes: &[u8]) -> Result<Companion, ExportError> {
    if bytes.len() > 512 * 1024 * 1024 {
        return Err(ExportError::Validation(
            "companion file is too large".into(),
        ));
    }
    let wire: WireCompanion = serde_json::from_slice(bytes)
        .map_err(|err| ExportError::Validation(format!("invalid EntoCAD companion: {err}")))?;
    if wire.format != COMPANION_FORMAT {
        return Err(ExportError::Unsupported(format!(
            "expected format '{COMPANION_FORMAT}', found '{}'",
            wire.format
        )));
    }
    if wire.schema != COMPANION_SCHEMA {
        return Err(ExportError::Unsupported(format!(
            "unsupported EntoCAD companion schema {} (this build reads {COMPANION_SCHEMA})",
            wire.schema
        )));
    }
    let mut blocks = Vec::with_capacity(wire.blocks.len());
    for block in wire.blocks {
        blocks.push(from_wire_block(block)?);
    }
    let mut instances = Vec::with_capacity(wire.instances.len());
    for instance in wire.instances {
        instances.push(CompanionInstance {
            export_block: instance.export_block,
            source_block: instance.source_block,
            configuration: from_config(instance.configuration),
            fingerprint: from_wire_fingerprint(instance.fingerprint),
        });
    }
    Ok(Companion {
        blocks,
        instances,
        fingerprints: wire
            .block_fingerprints
            .into_iter()
            .map(|(name, fingerprint)| (name, from_wire_fingerprint(fingerprint)))
            .collect(),
        generated: wire.generated,
    })
}

// ------------------------------------------------------------
// Function: apply_companion
// Purpose: Put dynamic definitions back and point flattened
//          inserts at them. A block whose geometry no longer
//          matches the fingerprint stays as AutoCAD left it.
// ------------------------------------------------------------
pub fn apply_companion(document: &mut Document, companion: Companion) -> CompanionReport {
    let mut blocked = fingerprint_conflicts(document, &companion);
    expand_blocked(&companion.blocks, &mut blocked);
    let install: Vec<BlockDefinition> = companion
        .blocks
        .iter()
        .filter(|block| !blocked.contains(&block.name.to_ascii_lowercase()))
        .filter(|block| block.is_dynamic() || document.block_by_name(&block.name).is_none())
        .cloned()
        .collect();
    let installed_dynamic = install.iter().filter(|block| block.is_dynamic()).count();
    let maps = match prepare_blocks(document, install) {
        Ok(maps) => maps,
        Err(err) => {
            return CompanionReport {
                restored: 0,
                skipped: 0,
                warning: Some(err),
            };
        }
    };
    let generated: BTreeSet<String> = companion
        .generated
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    let mut restored_inserts = 0;
    let mut skipped = 0;
    for instance in &companion.instances {
        let export_present = document.block_by_name(&instance.export_block).is_some();
        let export_mismatch = document
            .block_by_name(&instance.export_block)
            .is_some_and(|block| fingerprint_block(block) != instance.fingerprint);
        let tree_mismatch = export_present
            && generated_tree_mismatch(
                document,
                &instance.export_block,
                &generated,
                &companion.fingerprints,
            );
        let source_blocked = blocked.contains(&instance.source_block.to_ascii_lowercase());
        if !export_present || export_mismatch || tree_mismatch || source_blocked {
            if export_present && (export_mismatch || tree_mismatch || source_blocked) {
                skipped += 1;
            }
            continue;
        }
        let configuration =
            remap_configuration(&instance.configuration, &maps.parameters, &maps.options);
        restored_inserts += retarget_inserts(
            document,
            &instance.export_block,
            &instance.source_block,
            &configuration,
            &generated,
        );
    }
    prune_generated(document, &companion.generated);
    let restored = if restored_inserts > 0 {
        restored_inserts
    } else {
        installed_dynamic
    };
    CompanionReport {
        restored,
        skipped,
        warning: None,
    }
}

fn fingerprint_conflicts(document: &Document, companion: &Companion) -> BTreeSet<String> {
    let mut blocked = BTreeSet::new();
    for block in &companion.blocks {
        let Some(existing) = document.block_by_name(&block.name) else {
            continue;
        };
        let Some(expected) = lookup_fingerprint(&companion.fingerprints, &block.name) else {
            continue;
        };
        if fingerprint_block(existing) != *expected {
            blocked.insert(block.name.to_ascii_lowercase());
        }
    }
    blocked
}

fn expand_blocked(blocks: &[BlockDefinition], blocked: &mut BTreeSet<String>) {
    loop {
        let mut grew = false;
        for block in blocks {
            let name = block.name.to_ascii_lowercase();
            if blocked.contains(&name) {
                continue;
            }
            let depends = block.entities.iter().any(|entity| {
                entity
                    .geometry
                    .insert_block_name()
                    .is_some_and(|child| blocked.contains(&child.to_ascii_lowercase()))
            });
            if depends {
                blocked.insert(name);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
}

fn generated_tree_mismatch(
    document: &Document,
    root: &str,
    generated: &BTreeSet<String>,
    fingerprints: &BTreeMap<String, BlockFingerprint>,
) -> bool {
    let mut stack = vec![root.to_string()];
    let mut seen = BTreeSet::new();
    while let Some(name) = stack.pop() {
        let key = name.to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        if !name.eq_ignore_ascii_case(root) && generated.contains(&name.to_ascii_lowercase()) {
            let Some(definition) = document.block_by_name(&name) else {
                return true;
            };
            let Some(expected) = lookup_fingerprint(fingerprints, &name) else {
                return true;
            };
            if fingerprint_block(definition) != *expected {
                return true;
            }
        }
        if let Some(definition) = document.block_by_name(&name) {
            for entity in &definition.entities {
                if let Some(child) = entity.geometry.insert_block_name() {
                    stack.push(child.to_string());
                }
            }
        }
    }
    false
}

fn prepare_blocks(document: &mut Document, blocks: Vec<BlockDefinition>) -> Result<IdMaps, String> {
    let mut parameters = BTreeMap::new();
    let mut options = BTreeMap::new();
    let mut actions = BTreeMap::new();
    let mut entities = BTreeMap::new();
    let mut prepared = Vec::with_capacity(blocks.len());
    for mut block in blocks {
        block.id = document.allocate_definition_id();
        for entity in &mut block.entities {
            let old = entity.id;
            entity.id = document.allocate_id();
            if old.is_assigned() {
                entities.insert(old, entity.id);
            }
        }
        let vertices = document.remap_entity_vertex_ids(&mut block.entities);
        let mut anchors = BTreeMap::new();
        let mut presets = BTreeMap::new();
        if let Some(dynamic) = block.dynamic.as_ref() {
            for parameter in &dynamic.parameters {
                parameters.insert(parameter.id, document.allocate_parameter_id());
                if let ParameterKind::Choice(choice) = &parameter.kind {
                    for option in &choice.options {
                        options.insert(option.id, document.allocate_option_id());
                    }
                }
            }
            for id in dynamic.collect_action_ids() {
                actions.insert(id, document.allocate_action_id());
            }
            for anchor in &dynamic.anchors {
                anchors.insert(anchor.id, document.allocate_anchor_id());
            }
            for preset in &dynamic.presets {
                presets.insert(preset.id, document.allocate_preset_id());
            }
        }
        prepared.push(PreparedBlock {
            block,
            anchors,
            presets,
            vertices,
        });
    }
    let mut ready = Vec::with_capacity(prepared.len());
    for mut item in prepared {
        if let Some(dynamic) = item.block.dynamic.as_mut() {
            dynamic
                .remap_ids_with(
                    &parameters,
                    &options,
                    &actions,
                    &item.anchors,
                    &item.presets,
                    &entities,
                    &item.vertices,
                )
                .map_err(|err| err.to_string())?;
        }
        for entity in &mut item.block.entities {
            if let Some(config) = entity.geometry.insert_configuration_mut() {
                if let Some(values) = config.as_mut() {
                    values.remap_identities(&parameters, &options);
                }
            }
        }
        ready.push(item.block);
    }
    for block in ready {
        document.replace_block_definition(block);
    }
    Ok(IdMaps {
        parameters,
        options,
    })
}

fn remap_configuration(
    configuration: &InstanceConfiguration,
    parameters: &BTreeMap<cad_core::ParameterId, cad_core::ParameterId>,
    options: &BTreeMap<cad_core::OptionId, cad_core::OptionId>,
) -> InstanceConfiguration {
    let mut remapped = configuration.clone();
    remapped.remap_identities(parameters, options);
    remapped
}

fn retarget_inserts(
    document: &mut Document,
    from: &str,
    to: &str,
    configuration: &InstanceConfiguration,
    generated: &BTreeSet<String>,
) -> usize {
    let mut restored = retarget_entities(&mut document.model_space, from, to, configuration);
    let keys: Vec<String> = document.blocks.keys().cloned().collect();
    for key in keys {
        if generated.contains(&key.to_ascii_lowercase()) {
            continue;
        }
        if let Some(block) = document.blocks.get_mut(&key) {
            restored += retarget_entities(&mut block.entities, from, to, configuration);
        }
    }
    restored
}

fn retarget_entities(
    entities: &mut [Entity],
    from: &str,
    to: &str,
    configuration: &InstanceConfiguration,
) -> usize {
    let mut count = 0;
    for entity in entities.iter_mut() {
        let Some(name) = entity.geometry.insert_block_name() else {
            continue;
        };
        if !name.eq_ignore_ascii_case(from) {
            continue;
        }
        if let Some(block_name) = entity.geometry.insert_block_name_mut() {
            *block_name = to.to_string();
        }
        entity
            .geometry
            .set_insert_configuration(Some(configuration.clone()));
        count += 1;
    }
    count
}

fn prune_generated(document: &mut Document, generated: &[String]) {
    loop {
        let mut removed = false;
        for name in generated {
            let Some(key) = document.block_key(name) else {
                continue;
            };
            if block_is_referenced(document, &key) {
                continue;
            }
            document.remove_block_definition(&key);
            removed = true;
        }
        if !removed {
            break;
        }
    }
}

fn block_is_referenced(document: &Document, name: &str) -> bool {
    if space_references(&document.model_space, name) {
        return true;
    }
    document.blocks.values().any(|block| {
        !block.name.eq_ignore_ascii_case(name) && space_references(&block.entities, name)
    })
}

fn space_references(entities: &[Entity], name: &str) -> bool {
    entities.iter().any(|entity| {
        entity
            .geometry
            .insert_block_name()
            .is_some_and(|block| block.eq_ignore_ascii_case(name))
    })
}

fn source_blocks(source: &Document) -> Vec<BlockDefinition> {
    let mut ordered = Vec::new();
    let mut seen = BTreeSet::new();
    let names: Vec<String> = source.blocks.keys().cloned().collect();
    for name in names {
        let Some(block) = source.block_by_name(&name) else {
            continue;
        };
        if block.is_dynamic() {
            push_block_tree(source, block, &mut ordered, &mut seen);
        }
    }
    ordered
}

fn push_block_tree(
    source: &Document,
    block: &BlockDefinition,
    ordered: &mut Vec<BlockDefinition>,
    seen: &mut BTreeSet<String>,
) {
    if !seen.insert(block.name.to_ascii_lowercase()) {
        return;
    }
    let children: Vec<String> = block
        .entities
        .iter()
        .filter_map(|entity| entity.geometry.insert_block_name().map(str::to_string))
        .collect();
    for child_name in children {
        if let Some(child) = source.block_by_name(&child_name) {
            push_block_tree(source, child, ordered, seen);
        }
    }
    ordered.push(block.clone());
}

fn lookup_fingerprint<'a>(
    map: &'a BTreeMap<String, BlockFingerprint>,
    name: &str,
) -> Option<&'a BlockFingerprint> {
    if let Some(found) = map.get(name) {
        return Some(found);
    }
    map.iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, fingerprint)| fingerprint)
}

fn fingerprint_block(block: &BlockDefinition) -> BlockFingerprint {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut any = false;
    for entity in &block.entities {
        for (x, y) in entity_points(&entity.geometry) {
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            any = true;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    BlockFingerprint {
        entities: block.entities.len() as u64,
        min: any.then(|| [quantize(min_x), quantize(min_y)]),
        max: any.then(|| [quantize(max_x), quantize(max_y)]),
    }
}

fn entity_points(geometry: &Geometry) -> Vec<(f64, f64)> {
    let mut points = Vec::new();
    macro_rules! add {
        ($point:expr) => {
            points.push(($point.x, $point.y))
        };
    }
    macro_rules! add_xy {
        ($x:expr, $y:expr $(,)?) => {
            points.push(($x, $y))
        };
    }
    match geometry {
        Geometry::Line { start, end } => {
            add!(*start);
            add!(*end);
        }
        Geometry::Point { position } => add!(*position),
        Geometry::Circle { center, radius, .. } => {
            add!(*center);
            add_xy!(center.x + radius, center.y);
            add_xy!(center.x - radius, center.y);
            add_xy!(center.x, center.y + radius);
            add_xy!(center.x, center.y - radius);
        }
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            ..
        } => {
            add!(*center);
            add_xy!(
                center.x + radius * start_angle.cos(),
                center.y + radius * start_angle.sin(),
            );
            add_xy!(
                center.x + radius * end_angle.cos(),
                center.y + radius * end_angle.sin(),
            );
        }
        Geometry::Ellipse {
            center, major_axis, ..
        } => {
            add!(*center);
            add_xy!(center.x + major_axis.x, center.y + major_axis.y);
        }
        Geometry::LwPolyline { vertices, .. } | Geometry::Polyline { vertices, .. } => {
            for vertex in vertices {
                add!(vertex.point);
            }
        }
        Geometry::Spline {
            control_points,
            fit_points,
            ..
        } => {
            for point in control_points.iter().chain(fit_points.iter()) {
                add!(*point);
            }
        }
        Geometry::Insert {
            insertion,
            scale,
            rotation,
            attribs,
            ..
        } => {
            add!(*insertion);
            add_xy!(
                insertion.x + rotation.cos() * scale.x,
                insertion.y + rotation.sin() * scale.y,
            );
            for attrib in attribs {
                add!(attrib.insertion);
            }
        }
        Geometry::Text(data) => {
            add!(data.insertion);
            add_xy!(
                data.insertion.x + data.height,
                data.insertion.y + data.height
            );
        }
        Geometry::MText(data) => {
            add!(data.insertion);
            add_xy!(
                data.insertion.x + data.width,
                data.insertion.y + data.height
            );
        }
        Geometry::Hatch(hatch) => {
            for path in &hatch.paths {
                match path {
                    HatchPath::Polyline { vertices, .. } => {
                        for vertex in vertices {
                            add!(vertex.point);
                        }
                    }
                    HatchPath::Edges(edges) => {
                        for edge in edges {
                            match edge {
                                HatchEdge::Line { start, end } => {
                                    add!(*start);
                                    add!(*end);
                                }
                                HatchEdge::Arc { center, radius, .. } => {
                                    add!(*center);
                                    add_xy!(center.x + radius, center.y);
                                }
                                HatchEdge::Ellipse {
                                    center,
                                    major_endpoint,
                                    ..
                                } => {
                                    add!(*center);
                                    add!(*major_endpoint);
                                }
                                HatchEdge::Spline {
                                    control_points,
                                    fit_points,
                                    ..
                                } => {
                                    for point in control_points.iter().chain(fit_points.iter()) {
                                        add!(*point);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Geometry::Dimension(_) => {}
        Geometry::Image(frame) | Geometry::Wipeout(frame) => {
            for corner in frame.corners() {
                add!(corner);
            }
        }
        Geometry::Solid { corners, .. } => {
            for corner in corners {
                add!(*corner);
            }
        }
        Geometry::Leader { vertices } | Geometry::MLine { vertices, .. } => {
            for vertex in vertices {
                add!(*vertex);
            }
        }
        Geometry::Viewport(viewport) => {
            for corner in viewport.corners() {
                add!(corner);
            }
        }
    }
    points
}

fn quantize(value: f64) -> f64 {
    (value * FINGERPRINT_SCALE).round() / FINGERPRINT_SCALE
}

fn to_wire_fingerprint(fingerprint: &BlockFingerprint) -> WireFingerprint {
    WireFingerprint {
        entities: fingerprint.entities,
        min: fingerprint.min,
        max: fingerprint.max,
    }
}

fn from_wire_fingerprint(fingerprint: WireFingerprint) -> BlockFingerprint {
    BlockFingerprint {
        entities: fingerprint.entities,
        min: fingerprint.min,
        max: fingerprint.max,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_core::{
        export_materialized, identity_insert, BehaviorKind, CompositionRule, DynamicBehavior,
        DynamicDefinition, EvaluationCache, EvaluationRequest, FollowRole, GeometryTarget,
        NumericParameter, ParameterDef, ParameterValue, Point2, Point3,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("mycad-companion-{stamp}-{name}"))
    }

    fn span_document() -> (Document, cad_core::ParameterId) {
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
        (document, param)
    }

    fn materialize(source: &Document) -> cad_core::MaterializedExport {
        let request = EvaluationRequest {
            generation: source.content_generation(),
        };
        export_materialized(source, &mut EvaluationCache::default(), request).unwrap()
    }

    fn write_and_read(source: &Document, exported: &cad_core::MaterializedExport) -> Companion {
        let path = temp_path("plant.dwg.mycad");
        write_companion(
            &path,
            source,
            &exported.document,
            &exported.links,
            &exported.generated_blocks,
        )
        .unwrap();
        let companion = read_companion(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        companion
    }

    fn simulate_import(document: &mut Document) {
        for block in document.blocks.values_mut() {
            block.dynamic = None;
        }
        clear_configurations(&mut document.model_space);
        let keys: Vec<String> = document.blocks.keys().cloned().collect();
        for key in keys {
            if let Some(block) = document.blocks.get_mut(&key) {
                clear_configurations(&mut block.entities);
            }
        }
    }

    fn clear_configurations(entities: &mut [Entity]) {
        for entity in entities {
            entity.geometry.set_insert_configuration(None);
        }
    }

    fn insert_name(document: &Document) -> String {
        document.model_space[0]
            .geometry
            .insert_block_name()
            .unwrap()
            .to_string()
    }

    fn span_of(document: &Document) -> f64 {
        let definition = document.block_by_name("AdjustableFrame").unwrap();
        let param = definition.dynamic.as_ref().unwrap().parameters[0].id;
        document.model_space[0]
            .geometry
            .insert_configuration()
            .unwrap()
            .get(param)
            .unwrap()
            .as_number()
            .unwrap()
    }

    #[test]
    fn companion_path_keeps_the_drawing_extension() {
        assert_eq!(
            companion_path(Path::new("drawings/Plant.dwg")),
            PathBuf::from("drawings/Plant.dwg.mycad")
        );
        assert_eq!(
            companion_path(Path::new("Plant.dxf")),
            PathBuf::from("Plant.dxf.mycad")
        );
    }

    #[test]
    fn drawing_for_companion_opens_the_sibling_drawing() {
        assert_eq!(
            drawing_for_companion(Path::new("drawings/Plant.dwg.mycad")).unwrap(),
            PathBuf::from("drawings/Plant.dwg")
        );
        assert_eq!(
            drawing_for_companion(Path::new("Plant.DXF.MYCAD")).unwrap(),
            PathBuf::from("Plant.DXF")
        );
        assert!(drawing_for_companion(Path::new("Plant.mycad")).is_none());
        assert!(drawing_for_companion(Path::new("Plant.dwg")).is_none());
    }

    #[test]
    fn companion_roundtrip_restores_configuration_and_prunes_the_static_block() {
        let (source, _) = span_document();
        let exported = materialize(&source);
        let companion = write_and_read(&source, &exported);
        let mut imported = exported.document.clone();
        simulate_import(&mut imported);
        let export_name = insert_name(&imported);
        assert_ne!(export_name, "AdjustableFrame");
        let report = apply_companion(&mut imported, companion);
        assert_eq!(report.restored, 1);
        assert_eq!(report.skipped, 0);
        assert_eq!(
            report.status_line().as_deref(),
            Some("Restored 1 dynamic block")
        );
        assert!(imported
            .block_by_name("AdjustableFrame")
            .unwrap()
            .is_dynamic());
        assert_eq!(insert_name(&imported), "AdjustableFrame");
        assert!((span_of(&imported) - 1200.0).abs() < 1e-9);
        assert!(imported.block_by_name(&export_name).is_none());
    }

    #[test]
    fn moved_and_copied_inserts_stay_dynamic_at_the_drawing_position() {
        let (source, _) = span_document();
        let exported = materialize(&source);
        let companion = write_and_read(&source, &exported);
        let mut imported = exported.document.clone();
        simulate_import(&mut imported);
        if let Geometry::Insert { insertion, .. } = &mut imported.model_space[0].geometry {
            *insertion = Point3::from_xy(50.0, 10.0);
        }
        let mut copy = imported.model_space[0].clone();
        if let Geometry::Insert { insertion, .. } = &mut copy.geometry {
            *insertion = Point3::from_xy(100.0, 0.0);
        }
        imported.model_space.push(copy);
        let report = apply_companion(&mut imported, companion);
        assert_eq!(report.restored, 2);
        assert_eq!(report.skipped, 0);
        for (index, expected) in [(50.0, 10.0), (100.0, 0.0)].into_iter().enumerate() {
            let Geometry::Insert {
                block_name,
                insertion,
                ..
            } = &imported.model_space[index].geometry
            else {
                panic!("insert");
            };
            assert_eq!(block_name, "AdjustableFrame");
            assert!((insertion.x - expected.0).abs() < 1e-9);
            assert!((insertion.y - expected.1).abs() < 1e-9);
            assert!(imported.model_space[index]
                .geometry
                .insert_configuration()
                .is_some());
        }
    }

    #[test]
    fn fingerprint_mismatch_leaves_the_block_static() {
        let (source, _) = span_document();
        let exported = materialize(&source);
        let companion = write_and_read(&source, &exported);
        let mut imported = exported.document.clone();
        simulate_import(&mut imported);
        let export_name = insert_name(&imported);
        let block = imported.block_by_name_mut(&export_name).unwrap();
        if let Geometry::Line { end, .. } = &mut block.entities[0].geometry {
            end.x += 25.0;
        }
        let report = apply_companion(&mut imported, companion);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.restored, 1);
        assert_eq!(insert_name(&imported), export_name);
        assert!(imported.model_space[0]
            .geometry
            .insert_configuration()
            .is_none());
        assert!(imported.block_by_name(&export_name).is_some());
        assert_eq!(
            report.status_line().as_deref(),
            Some("Restored 1 dynamic block • 1 block edited outside EntoCAD stays static")
        );
    }

    #[test]
    fn missing_companion_is_a_no_op() {
        let path = temp_path("absent.dwg.mycad");
        let _ = std::fs::remove_file(&path);
        assert!(read_companion_optional(&path).unwrap().is_none());
    }

    #[test]
    fn unsupported_schema_is_rejected() {
        let json = r#"{"format":"mycad-companion","schema":99,"blocks":[],"instances":[],"block_fingerprints":{},"generated":[]}"#;
        let err = parse_companion_bytes(json.as_bytes()).unwrap_err();
        assert!(err
            .to_string()
            .contains("unsupported EntoCAD companion schema"));
    }
}
