//! ASCII DXF writer for cad-core documents. No LibreDWG.
//!
//! Fallback policy (never silent):
//! - Supported native types are written as the matching DXF entity.
//! - Types MyCAD cannot represent fully are exploded to simpler primitives
//!   and recorded on `SaveReport.warnings` (DIMENSION block geometry, MLINE
//!   segments, INSERT attributes, ATTDEF, HATCH spline edges, varying-Z
//!   LWPOLYLINE).
//! - Missing blocks or empty exploded geometry still produce a warning.

use std::collections::BTreeMap;
use std::path::Path;

use cad_core::{
    BlockDefinition, CadColor, Document, DrawingUnits, Entity, Geometry, HatchData, HatchEdge,
    HatchPath, Layer, LineType, MTextData, Point3, PolyVertex, TextData,
};

use crate::error::ExportError;
use crate::options::{DxfAcadVersion, DxfExportOptions, SaveReport};
use crate::r2000::{encode_dxf_r2000, mtext_group_chunks};

const WORLD: Point3 = Point3 {
    x: 0.0,
    y: 0.0,
    z: 1.0,
};

// ------------------------------------------------------------
// Function: write_dxf
// Purpose: Write an ASCII DXF from a native cad-core document.
// ------------------------------------------------------------
pub fn write_dxf(
    document: &Document,
    path: &Path,
    options: &DxfExportOptions,
) -> Result<SaveReport, ExportError> {
    let mut writer = DxfWriter::new(document, options.version);
    writer.write_document();
    if writer.nonfinite {
        return Err(ExportError::Invalid(
            "DXF cannot store non-finite coordinates or scales",
        ));
    }
    crate::atomic::write_atomic(path, writer.out.as_bytes())
        .map_err(|source| ExportError::io(path, source))?;
    Ok(writer.report)
}

struct DxfWriter<'a> {
    document: &'a Document,
    version: DxfAcadVersion,
    out: String,
    handle: u64,
    report: SaveReport,
    block_stack: Vec<String>,
    block_records: BTreeMap<String, String>,
    nonfinite: bool,
}

impl<'a> DxfWriter<'a> {
    fn new(document: &'a Document, version: DxfAcadVersion) -> Self {
        Self {
            document,
            version,
            out: String::with_capacity(16 * 1024),
            // LibreDWG reserves low handles for layout BLOCK_HEADERs
            // (*MODEL_SPACE is typically 0x1F). Colliding with those IDs
            // overwrites the model-space object and truncates ENTITIES.
            handle: 0x100,
            report: SaveReport::default(),
            block_stack: Vec::new(),
            block_records: BTreeMap::new(),
            nonfinite: false,
        }
    }

    fn pair(&mut self, code: i16, value: impl AsRef<str>) {
        self.out
            .push_str(&format!("{code:3}\n{}\n", value.as_ref()));
    }

    fn pair_f(&mut self, code: i16, value: f64) {
        if !value.is_finite() {
            self.nonfinite = true;
            self.warn("non-finite coordinate or scale was not written as a number");
        }
        self.pair(code, format_dxf_r2000_f64(value));
    }

    fn pair_i(&mut self, code: i16, value: i32) {
        self.pair(code, value.to_string());
    }

    fn next_handle(&mut self) -> String {
        let handle = self.handle;
        self.handle = self.handle.saturating_add(1);
        format!("{handle:X}")
    }

    fn write_document(&mut self) {
        self.write_header();
        self.write_tables();
        self.write_blocks();
        self.write_entities();
        self.pair(0, "EOF");
    }

    fn write_header(&mut self) {
        self.pair(0, "SECTION");
        self.pair(2, "HEADER");
        self.pair(9, "$ACADVER");
        self.pair(1, self.version.acadver());
        self.pair(9, "$DWGCODEPAGE");
        self.pair(3, "ANSI_1252");
        self.pair(9, "$INSUNITS");
        self.pair_i(70, i32::from(self.document.units.to_insunits()));
        self.pair(9, "$MEASUREMENT");
        self.pair_i(70, measurement_code(self.document.units));
        self.pair(9, "$CLAYER");
        self.pair(8, sanitize_name(&self.document.current_layer));
        self.pair(9, "$LTSCALE");
        self.pair_f(40, self.document.ltscale.max(1e-12));
        // LibreDWG's R2000 decoder copies TDUCREATE into TDCREATE and always
        // calls strftime() with STRFTIME_DATE. A zero Julian day produces
        // tm_mday=0, which MSVC treats as an invalid parameter and aborts
        // with STATUS_STACK_BUFFER_OVERRUN before dwg_read_file returns.
        let created = autocad_julian_date();
        for name in ["$TDCREATE", "$TDUPDATE", "$TDUCREATE", "$TDUUPDATE"] {
            self.pair(9, name);
            self.pair_f(40, created);
        }
        if let Some(extents) = self
            .document
            .diagnostics
            .extents
            .or_else(|| self.document.compute_extents())
        {
            self.pair(9, "$EXTMIN");
            self.pair_f(10, extents.min.x);
            self.pair_f(20, extents.min.y);
            self.pair_f(30, 0.0);
            self.pair(9, "$EXTMAX");
            self.pair_f(10, extents.max.x);
            self.pair_f(20, extents.max.y);
            self.pair_f(30, 0.0);
        }
        self.pair(0, "ENDSEC");
    }

    fn write_tables(&mut self) {
        self.pair(0, "SECTION");
        self.pair(2, "TABLES");
        self.write_ltype_table();
        self.write_layer_table();
        self.write_style_table();
        self.write_block_record_table();
        self.pair(0, "ENDSEC");
    }

    fn write_ltype_table(&mut self) {
        self.pair(0, "TABLE");
        self.pair(2, "LTYPE");
        let table_handle = self.next_handle();
        self.pair(5, &table_handle);
        self.pair(100, "AcDbSymbolTable");
        let mut names: Vec<String> = self.document.linetypes.keys().cloned().collect();
        for required in ["BYLAYER", "BYBLOCK", "CONTINUOUS"] {
            if !names.iter().any(|n| n.eq_ignore_ascii_case(required)) {
                names.push(required.into());
            }
        }
        names.sort();
        names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        self.pair_i(70, names.len() as i32);
        for name in names {
            let linetype = self
                .document
                .linetypes
                .get(&name)
                .cloned()
                .unwrap_or_else(|| LineType::continuous(&name));
            self.write_ltype(&linetype);
        }
        self.pair(0, "ENDTAB");
    }

    fn write_ltype(&mut self, linetype: &LineType) {
        self.pair(0, "LTYPE");
        let handle = self.next_handle();
        self.pair(5, handle);
        self.pair(100, "AcDbSymbolTableRecord");
        self.pair(100, "AcDbLinetypeTableRecord");
        self.pair(2, sanitize_name(&linetype.name));
        self.pair_i(70, 0);
        self.pair(3, "");
        self.pair_i(72, 65);
        self.pair_i(73, linetype.dashes.len() as i32);
        let pattern_len: f64 = linetype.dashes.iter().map(|d| d.abs()).sum();
        self.pair_f(40, pattern_len);
        for dash in &linetype.dashes {
            self.pair_f(49, *dash);
            self.pair_i(74, 0);
        }
    }

    fn write_layer_table(&mut self) {
        self.pair(0, "TABLE");
        self.pair(2, "LAYER");
        let table_handle = self.next_handle();
        self.pair(5, &table_handle);
        self.pair(100, "AcDbSymbolTable");
        self.pair_i(70, self.document.layers.len() as i32);
        let layers: Vec<&Layer> = self.document.layers.values().collect();
        for layer in layers {
            self.write_layer(layer);
        }
        self.pair(0, "ENDTAB");
    }

    fn write_layer(&mut self, layer: &Layer) {
        self.pair(0, "LAYER");
        let handle = self.next_handle();
        self.pair(5, handle);
        self.pair(100, "AcDbSymbolTableRecord");
        self.pair(100, "AcDbLayerTableRecord");
        self.pair(2, sanitize_name(&layer.name));
        let mut flags = 0_i32;
        if layer.frozen {
            flags |= 1;
        }
        self.pair_i(70, flags);
        let mut aci = color_aci(layer.color);
        if !layer.visible && aci > 0 {
            aci = -aci;
        }
        self.pair_i(62, aci);
        self.pair(6, sanitize_name(&layer.linetype));
        self.pair_i(370, -3);
    }

    fn write_style_table(&mut self) {
        self.pair(0, "TABLE");
        self.pair(2, "STYLE");
        let handle = self.next_handle();
        self.pair(5, handle);
        self.pair(100, "AcDbSymbolTable");
        self.pair_i(70, 1);
        self.pair(0, "STYLE");
        let record = self.next_handle();
        self.pair(5, record);
        self.pair(100, "AcDbSymbolTableRecord");
        self.pair(100, "AcDbTextStyleTableRecord");
        self.pair(2, "STANDARD");
        self.pair_i(70, 0);
        self.pair_f(40, 0.0);
        self.pair_f(41, 1.0);
        self.pair_f(50, 0.0);
        self.pair_i(71, 0);
        self.pair_f(42, 2.5);
        self.pair(3, "txt");
        self.pair(4, "");
        self.pair(0, "ENDTAB");
    }

    fn write_block_record_table(&mut self) {
        let names: Vec<String> = self
            .document
            .blocks
            .keys()
            .filter(|name| !is_model_or_paper(name))
            .cloned()
            .collect();
        if names.is_empty() {
            return;
        }
        self.pair(0, "TABLE");
        self.pair(2, "BLOCK_RECORD");
        let table_handle = self.next_handle();
        self.pair(5, &table_handle);
        self.pair(100, "AcDbSymbolTable");
        self.pair_i(70, names.len() as i32);
        for name in names {
            self.pair(0, "BLOCK_RECORD");
            let handle = self.next_handle();
            self.pair(5, &handle);
            self.pair(100, "AcDbSymbolTableRecord");
            self.pair(100, "AcDbBlockTableRecord");
            let sanitized = sanitize_name(&name);
            self.pair(2, &sanitized);
            self.block_records.insert(name, handle);
        }
        self.pair(0, "ENDTAB");
    }

    fn write_owner(&mut self) {
        let handle = self
            .block_stack
            .last()
            .and_then(|name| self.block_records.get(name).cloned());
        if let Some(handle) = handle {
            self.pair(330, handle);
        }
    }

    fn write_blocks(&mut self) {
        self.pair(0, "SECTION");
        self.pair(2, "BLOCKS");
        self.write_space_block("*MODEL_SPACE");
        self.write_space_block("*PAPER_SPACE");
        let names: Vec<String> = self.document.blocks.keys().cloned().collect();
        for name in names {
            if is_model_or_paper(&name) {
                continue;
            }
            let Some(block) = self.document.blocks.get(&name).cloned() else {
                continue;
            };
            self.write_block_definition(&name, &block);
        }
        self.pair(0, "ENDSEC");
    }

    fn write_space_block(&mut self, name: &str) {
        self.write_block_definition(
            name,
            &BlockDefinition {
                name: name.into(),
                base_pt: Point3::from_xy(0.0, 0.0),
                entities: Vec::new(),
                ..Default::default()
            },
        );
    }

    fn write_block_definition(&mut self, name: &str, block: &BlockDefinition) {
        self.block_stack.push(name.to_string());
        self.pair(0, "BLOCK");
        let handle = self.next_handle();
        self.pair(5, handle);
        self.write_owner();
        self.pair(100, "AcDbEntity");
        self.pair(8, "0");
        self.pair(100, "AcDbBlockBegin");
        self.pair(2, sanitize_name(name));
        let mut flags = 0_i32;
        if name.starts_with('*') && !is_model_or_paper(name) {
            flags |= 1;
        }
        if block
            .entities
            .iter()
            .any(|entity| matches!(&entity.geometry, Geometry::Text(data) if data.is_attrib_def))
        {
            flags |= 2;
        }
        self.pair_i(70, flags);
        self.pair_f(10, block.base_pt.x);
        self.pair_f(20, block.base_pt.y);
        self.pair_f(30, block.base_pt.z);
        self.pair(3, sanitize_name(name));
        for entity in &block.entities {
            self.write_entity(entity);
        }
        self.pair(0, "ENDBLK");
        let end = self.next_handle();
        self.pair(5, end);
        self.write_owner();
        self.pair(100, "AcDbEntity");
        self.pair(8, "0");
        self.pair(100, "AcDbBlockEnd");
        self.block_stack.pop();
    }

    fn write_entities(&mut self) {
        self.pair(0, "SECTION");
        self.pair(2, "ENTITIES");
        for entity in &self.document.model_space {
            self.write_entity(entity);
        }
        self.pair(0, "ENDSEC");
    }

    fn write_entity(&mut self, entity: &Entity) {
        match &entity.geometry {
            Geometry::Line { start, end } => {
                self.begin_entity("LINE", entity);
                self.point(10, *start);
                self.point(11, *end);
                self.report.entities_written += 1;
            }
            Geometry::Point { position } => {
                self.begin_entity("POINT", entity);
                self.point(10, *position);
                self.report.entities_written += 1;
            }
            Geometry::Circle {
                center,
                radius,
                extrusion,
            } => {
                self.begin_entity("CIRCLE", entity);
                self.point(10, *center);
                self.pair_f(40, radius.abs());
                self.extrusion(*extrusion);
                self.report.entities_written += 1;
            }
            Geometry::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                extrusion,
            } => {
                self.begin_entity("ARC", entity);
                self.point(10, *center);
                self.pair_f(40, radius.abs());
                self.pair_f(50, start_angle.to_degrees());
                self.pair_f(51, end_angle.to_degrees());
                self.extrusion(*extrusion);
                self.report.entities_written += 1;
            }
            Geometry::Ellipse {
                center,
                major_axis,
                axis_ratio,
                start_param,
                end_param,
                extrusion,
            } => {
                self.begin_entity("ELLIPSE", entity);
                self.point(10, *center);
                self.point(11, *major_axis);
                self.pair_f(40, *axis_ratio);
                self.pair_f(41, *start_param);
                self.pair_f(42, *end_param);
                self.extrusion(*extrusion);
                self.report.entities_written += 1;
            }
            Geometry::LwPolyline {
                vertices,
                closed,
                extrusion,
                linetype_generation_continuous,
            } => {
                if vertices.is_empty() {
                    self.warn("empty LWPOLYLINE was not written");
                } else if lw_vertices_have_varying_z(vertices) {
                    self.warn("LWPOLYLINE with varying Z exported as POLYLINE");
                    self.write_polyline2d(
                        entity,
                        vertices,
                        *closed,
                        *linetype_generation_continuous,
                    );
                } else {
                    self.begin_entity("LWPOLYLINE", entity);
                    self.pair_i(90, vertices.len() as i32);
                    let mut flags = 0_i32;
                    if *closed {
                        flags |= 1;
                    }
                    if *linetype_generation_continuous {
                        flags |= 128;
                    }
                    self.pair_i(70, flags);
                    let elevation = vertices[0].point.z;
                    if elevation.abs() > 1e-15 {
                        self.pair_f(38, elevation);
                    }
                    self.extrusion(*extrusion);
                    for vertex in vertices {
                        self.write_lw_vertex(vertex);
                    }
                    self.report.entities_written += 1;
                }
            }
            Geometry::Polyline {
                vertices,
                closed,
                linetype_generation_continuous,
            } => {
                if vertices.is_empty() {
                    self.warn("empty POLYLINE was not written");
                } else {
                    self.write_polyline2d(
                        entity,
                        vertices,
                        *closed,
                        *linetype_generation_continuous,
                    );
                }
            }
            Geometry::Spline {
                degree,
                control_points,
                fit_points,
                knots,
                weights,
                closed,
            } => {
                if control_points.is_empty() && fit_points.is_empty() {
                    self.warn("SPLINE with no control or fit points was not written");
                } else {
                    self.begin_entity("SPLINE", entity);
                    let rational = weights.iter().any(|w| (w - 1.0).abs() > 1e-12);
                    let mut flags = 8_i32;
                    if *closed {
                        flags |= 1;
                    }
                    if rational {
                        flags |= 4;
                    }
                    if !fit_points.is_empty() {
                        flags |= 2;
                    }
                    self.pair_i(70, flags);
                    self.pair_i(71, *degree as i32);
                    self.pair_i(72, knots.len() as i32);
                    self.pair_i(73, control_points.len() as i32);
                    if !fit_points.is_empty() {
                        self.pair_i(74, fit_points.len() as i32);
                    }
                    for knot in knots {
                        self.pair_f(40, *knot);
                    }
                    for point in control_points {
                        self.point(10, *point);
                    }
                    if rational {
                        for weight in weights {
                            self.pair_f(41, *weight);
                        }
                    }
                    for point in fit_points {
                        self.point(11, *point);
                    }
                    self.report.entities_written += 1;
                }
            }
            Geometry::Insert {
                block_name,
                insertion,
                scale,
                rotation,
                extrusion,
                column_count,
                row_count,
                column_spacing,
                row_spacing,
                attribs,
                configuration: _,
            } => {
                if self.block_by_name(block_name).is_none() {
                    self.warn(&format!(
                        "INSERT '{block_name}' references a missing block; the INSERT was still written"
                    ));
                }
                self.begin_entity("INSERT", entity);
                self.pair(2, sanitize_name(block_name));
                self.point(10, *insertion);
                self.pair_f(41, scale.x);
                self.pair_f(42, scale.y);
                self.pair_f(43, scale.z);
                self.pair_f(50, rotation.to_degrees());
                if *column_count > 1 {
                    self.pair_i(70, *column_count as i32);
                    self.pair_f(44, *column_spacing);
                }
                if *row_count > 1 {
                    self.pair_i(71, *row_count as i32);
                    self.pair_f(45, *row_spacing);
                }
                self.extrusion(*extrusion);
                self.report.entities_written += 1;
                if !attribs.is_empty() {
                    self.warn(
                        "INSERT attributes exported as TEXT; attribute tags are not stored natively",
                    );
                    for attrib in attribs {
                        self.write_text(entity, attrib);
                        self.report.entities_written += 1;
                    }
                }
            }
            Geometry::Text(data) => {
                if data.is_attrib_def {
                    self.warn("ATTDEF exported as TEXT; attribute tags are not stored natively");
                }
                self.write_text(entity, data);
                self.report.entities_written += 1;
            }
            Geometry::MText(data) => {
                self.write_mtext(entity, data);
                self.report.entities_written += 1;
            }
            Geometry::Solid { corners, extrusion } => {
                self.begin_entity("SOLID", entity);
                self.point(10, corners[0]);
                self.point(11, corners[1]);
                self.point(12, corners[2]);
                self.point(13, corners[3]);
                self.extrusion(*extrusion);
                self.report.entities_written += 1;
            }
            Geometry::Leader { vertices } => {
                if vertices.len() < 2 {
                    self.warn("LEADER with fewer than two vertices was not written");
                } else {
                    self.begin_entity("LEADER", entity);
                    self.pair_i(76, vertices.len() as i32);
                    for point in vertices {
                        self.point(10, *point);
                    }
                    self.report.entities_written += 1;
                }
            }
            Geometry::MLine { vertices, closed } => {
                self.write_mline_as_lines(entity, vertices, *closed)
            }
            Geometry::Hatch(hatch) => self.write_hatch(entity, hatch),
            Geometry::Dimension { block_name } => self.explode_dimension(block_name),
        }
    }

    fn write_polyline2d(
        &mut self,
        entity: &Entity,
        vertices: &[PolyVertex],
        closed: bool,
        linetype_generation_continuous: bool,
    ) {
        self.begin_entity("POLYLINE", entity);
        let mut flags = 0_i32;
        if closed {
            flags |= 1;
        }
        if linetype_generation_continuous {
            flags |= 128;
        }
        self.pair_i(70, flags);
        for vertex in vertices {
            self.pair(0, "VERTEX");
            let handle = self.next_handle();
            self.pair(5, handle);
            self.write_owner();
            self.pair(100, "AcDbEntity");
            self.pair(8, sanitize_name(&entity.layer));
            self.pair(100, "AcDbVertex");
            self.pair(100, "AcDb2dVertex");
            self.point(10, vertex.point);
            if vertex.bulge.abs() > 1e-15 {
                self.pair_f(42, vertex.bulge);
            }
        }
        self.pair(0, "SEQEND");
        let seq = self.next_handle();
        self.pair(5, seq);
        self.write_owner();
        self.pair(100, "AcDbEntity");
        self.pair(8, sanitize_name(&entity.layer));
        self.report.entities_written += 1;
    }

    fn write_mline_as_lines(&mut self, entity: &Entity, vertices: &[Point3], closed: bool) {
        if vertices.len() < 2 {
            self.warn("MLINE with fewer than two vertices was not written");
            return;
        }
        let count = if closed {
            vertices.len()
        } else {
            vertices.len().saturating_sub(1)
        };
        self.warn("MLINE exported as LINE segments");
        for i in 0..count {
            let start = vertices[i];
            let end = vertices[(i + 1) % vertices.len()];
            self.begin_entity("LINE", entity);
            self.point(10, start);
            self.point(11, end);
            self.report.entities_written += 1;
        }
    }

    fn explode_dimension(&mut self, block_name: &str) {
        if block_name.trim().is_empty() {
            self.warn("DIMENSION skipped; native model stores no block name or geometry");
            return;
        }
        if self
            .block_stack
            .iter()
            .any(|name| name.eq_ignore_ascii_case(block_name))
        {
            self.warn(&format!(
                "DIMENSION '{block_name}' skipped; its block is already being written"
            ));
            return;
        }
        let Some(block) = self.block_by_name(block_name) else {
            self.warn(&format!(
                "DIMENSION '{block_name}' skipped; visible block geometry is missing"
            ));
            return;
        };
        let entities = block.entities.clone();
        if entities.is_empty() {
            self.warn(&format!(
                "DIMENSION '{block_name}' exported no geometry; its anonymous block is empty"
            ));
            return;
        }
        self.warn(&format!(
            "DIMENSION '{block_name}' exported as visible block geometry, not a DIMENSION entity"
        ));
        self.block_stack.push(block_name.to_string());
        for child in &entities {
            self.write_entity(child);
        }
        self.block_stack.pop();
    }

    fn block_by_name(&self, name: &str) -> Option<&BlockDefinition> {
        self.document.blocks.get(name).or_else(|| {
            self.document
                .blocks
                .iter()
                .find(|(existing, _)| existing.eq_ignore_ascii_case(name))
                .map(|(_, block)| block)
        })
    }

    fn write_hatch(&mut self, entity: &Entity, hatch: &HatchData) {
        self.begin_entity("HATCH", entity);
        self.pair_f(10, 0.0);
        self.pair_f(20, 0.0);
        self.pair_f(30, hatch.elevation);
        self.extrusion(hatch.extrusion);
        if hatch.solid_fill {
            self.pair(2, "SOLID");
            self.pair_i(70, 1);
        } else {
            self.pair(2, "ANSI31");
            self.pair_i(70, 0);
        }
        self.pair_i(71, 0);
        self.pair_i(91, hatch.paths.len() as i32);
        for path in &hatch.paths {
            self.write_hatch_path(path);
        }
        self.pair_i(75, 1);
        self.pair_i(76, 1);
        self.pair_f(52, 0.0);
        self.pair_f(41, 1.0);
        self.pair_i(77, 0);
        self.pair_i(78, hatch.pattern_lines.len() as i32);
        for line in &hatch.pattern_lines {
            self.pair_f(53, line.angle.to_degrees());
            self.pair_f(43, line.base.x);
            self.pair_f(44, line.base.y);
            self.pair_f(45, line.offset.x);
            self.pair_f(46, line.offset.y);
            self.pair_i(79, line.dashes.len() as i32);
            for dash in &line.dashes {
                self.pair_f(49, *dash);
            }
        }
        self.pair_f(47, 0.0);
        self.pair_i(98, 0);
        self.report.entities_written += 1;
    }

    fn write_hatch_path(&mut self, path: &HatchPath) {
        match path {
            HatchPath::Polyline { vertices, closed } => {
                self.pair_i(92, 2);
                self.pair_i(72, 1);
                self.pair_i(73, i32::from(*closed));
                self.pair_i(93, vertices.len() as i32);
                for vertex in vertices {
                    self.pair_f(10, vertex.point.x);
                    self.pair_f(20, vertex.point.y);
                    self.pair_f(42, vertex.bulge);
                }
                self.pair_i(97, 0);
            }
            HatchPath::Edges(edges) => {
                let mut flat = Vec::new();
                for edge in edges {
                    match edge {
                        HatchEdge::Spline { control_points } => {
                            self.warn("HATCH spline edges exported as LINE segments");
                            for pair in control_points.windows(2) {
                                flat.push(HatchEdge::Line {
                                    start: pair[0],
                                    end: pair[1],
                                });
                            }
                        }
                        other => flat.push(other.clone()),
                    }
                }
                self.pair_i(92, 1);
                self.pair_i(93, flat.len() as i32);
                for edge in &flat {
                    self.write_hatch_edge(edge);
                }
                self.pair_i(97, 0);
            }
        }
    }

    fn write_hatch_edge(&mut self, edge: &HatchEdge) {
        match edge {
            HatchEdge::Line { start, end } => {
                self.pair_i(72, 1);
                self.pair_f(10, start.x);
                self.pair_f(20, start.y);
                self.pair_f(11, end.x);
                self.pair_f(21, end.y);
            }
            HatchEdge::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                is_ccw,
            } => {
                self.pair_i(72, 2);
                self.pair_f(10, center.x);
                self.pair_f(20, center.y);
                self.pair_f(40, *radius);
                self.pair_f(50, start_angle.to_degrees());
                self.pair_f(51, end_angle.to_degrees());
                self.pair_i(73, i32::from(*is_ccw));
            }
            HatchEdge::Ellipse {
                center,
                major_endpoint,
                axis_ratio,
                start_angle,
                end_angle,
                is_ccw,
            } => {
                self.pair_i(72, 3);
                self.pair_f(10, center.x);
                self.pair_f(20, center.y);
                self.pair_f(11, major_endpoint.x);
                self.pair_f(21, major_endpoint.y);
                self.pair_f(40, *axis_ratio);
                self.pair_f(50, start_angle.to_degrees());
                self.pair_f(51, end_angle.to_degrees());
                self.pair_i(73, i32::from(*is_ccw));
            }
            HatchEdge::Spline { control_points } => {
                self.warn("HATCH spline edges exported as LINE segments");
                for pair in control_points.windows(2) {
                    self.write_hatch_edge(&HatchEdge::Line {
                        start: pair[0],
                        end: pair[1],
                    });
                }
            }
        }
    }

    fn write_text(&mut self, entity: &Entity, data: &TextData) {
        self.begin_entity("TEXT", entity);
        self.point(10, data.insertion);
        self.pair_f(40, data.height.abs().max(1e-9));
        self.pair(1, sanitize_text(&data.value));
        self.pair_f(50, data.rotation.to_degrees());
        self.pair(7, "STANDARD");
        self.extrusion(data.extrusion);
    }

    fn write_mtext(&mut self, entity: &Entity, data: &MTextData) {
        self.begin_entity("MTEXT", entity);
        self.point(10, data.insertion);
        self.pair_f(40, data.height.abs().max(1e-9));
        self.pair_f(41, data.width.abs());
        self.pair_i(71, 1);
        write_mtext_chunks(&data.value, |code, chunk| {
            self.pair(code, chunk);
        });
        self.pair(7, "STANDARD");
        let axis = Point3::from_xy(data.rotation.cos(), data.rotation.sin());
        self.point(11, axis);
        self.extrusion(data.extrusion);
    }

    fn begin_entity(&mut self, kind: &str, entity: &Entity) {
        self.pair(0, kind);
        let handle = self.next_handle();
        self.pair(5, handle);
        self.write_owner();
        self.pair(100, "AcDbEntity");
        self.pair(8, sanitize_name(&entity.layer));
        write_entity_color(self, entity.color);
        if !cad_core::is_bylayer_name(&entity.linetype) {
            self.pair(6, sanitize_name(&entity.linetype));
        }
        if (entity.linetype_scale - 1.0).abs() > 1e-12 {
            self.pair_f(48, entity.linetype_scale);
        }
        if !entity.visible {
            self.pair_i(60, 1);
        }
        self.pair(100, acad_class(kind));
    }

    fn write_lw_vertex(&mut self, vertex: &PolyVertex) {
        self.pair_f(10, vertex.point.x);
        self.pair_f(20, vertex.point.y);
        if vertex.bulge.abs() > 1e-15 {
            self.pair_f(42, vertex.bulge);
        }
    }

    fn point(&mut self, code: i16, point: Point3) {
        self.pair_f(code, point.x);
        self.pair_f(code + 10, point.y);
        self.pair_f(code + 20, point.z);
    }

    fn extrusion(&mut self, extrusion: Point3) {
        if (extrusion.x - WORLD.x).abs() > 1e-12
            || (extrusion.y - WORLD.y).abs() > 1e-12
            || (extrusion.z - WORLD.z).abs() > 1e-12
        {
            self.point(210, extrusion);
        }
    }

    fn warn(&mut self, message: &str) {
        if !self
            .report
            .warnings
            .iter()
            .any(|existing| existing == message)
        {
            self.report.warnings.push(message.to_string());
        }
    }
}

fn write_entity_color(writer: &mut DxfWriter<'_>, color: CadColor) {
    match color {
        CadColor::ByLayer => {}
        CadColor::ByBlock => writer.pair_i(62, 0),
        CadColor::Aci(index) => writer.pair_i(62, i32::from(index)),
        CadColor::Rgb { r, g, b } => {
            writer.pair_i(62, 256);
            writer.pair_i(
                420,
                (i32::from(r) << 16) | (i32::from(g) << 8) | i32::from(b),
            );
        }
    }
}

fn color_aci(color: CadColor) -> i32 {
    match color {
        CadColor::ByLayer => 7,
        CadColor::ByBlock => 0,
        CadColor::Aci(index) => i32::from(index),
        CadColor::Rgb { .. } => 7,
    }
}

fn acad_class(kind: &str) -> &'static str {
    match kind {
        "LINE" => "AcDbLine",
        "POINT" => "AcDbPoint",
        "CIRCLE" => "AcDbCircle",
        "ARC" => "AcDbArc",
        "ELLIPSE" => "AcDbEllipse",
        "LWPOLYLINE" => "AcDbPolyline",
        "POLYLINE" => "AcDb2dPolyline",
        "SPLINE" => "AcDbSpline",
        "INSERT" => "AcDbBlockReference",
        "TEXT" => "AcDbText",
        "MTEXT" => "AcDbMText",
        "SOLID" => "AcDbTrace",
        "LEADER" => "AcDbLeader",
        "HATCH" => "AcDbHatch",
        _ => "AcDbEntity",
    }
}

fn sanitize_name(name: &str) -> String {
    let trimmed = name.trim();
    let cleaned = if trimmed.is_empty() {
        "0".into()
    } else {
        trimmed.replace(['\n', '\r'], "_")
    };
    encode_dxf_r2000(&cleaned)
}

fn sanitize_text(value: &str) -> String {
    encode_dxf_r2000(&value.replace('\r', "").replace('\n', "\\P"))
}

fn write_mtext_chunks(value: &str, mut write: impl FnMut(i16, &str)) {
    let text = sanitize_text(value);
    for (code, chunk) in mtext_group_chunks(&text) {
        write(code, chunk);
    }
}

fn is_model_or_paper(name: &str) -> bool {
    let upper = name.trim().to_ascii_uppercase();
    upper == "*MODEL_SPACE" || upper == "*PAPER_SPACE" || upper == "$MODEL_SPACE"
}

fn lw_vertices_have_varying_z(vertices: &[PolyVertex]) -> bool {
    let Some(first) = vertices.first() else {
        return false;
    };
    vertices
        .iter()
        .any(|vertex| (vertex.point.z - first.point.z).abs() > 1e-12)
}

fn measurement_code(units: DrawingUnits) -> i32 {
    match units {
        DrawingUnits::Inches
        | DrawingUnits::Feet
        | DrawingUnits::Miles
        | DrawingUnits::Microinches
        | DrawingUnits::Mils
        | DrawingUnits::Yards => 0,
        _ => 1,
    }
}

/// Julian day AutoCAD stores in `$TDUCREATE` / `$TDUUPDATE`.
/// 2451545.0 is 2000-01-01, well above LibreDWG's calendar-date threshold.
fn autocad_julian_date() -> f64 {
    2_451_545.0
}

fn format_dxf_r2000_f64(value: f64) -> String {
    if !value.is_finite() {
        "0.0".into()
    } else if value.fract().abs() < 1e-12 {
        format!("{:.1}", value.round())
    } else {
        format!("{value:.16}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_core::{
        BlockDefinition, CadColor, DrawingUnits, Entity, Geometry, HatchData, HatchEdge, HatchPath,
        MTextData, Point3, PolyVertex, TextData,
    };
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("mycad-cad-io-{stamp}-{name}"))
    }

    fn write_to_string(document: &Document) -> (SaveReport, String) {
        let path = temp_path("out.dxf");
        let report = write_dxf(document, &path, &DxfExportOptions::default()).expect("write");
        let text = fs::read_to_string(&path).expect("read");
        let _ = fs::remove_file(&path);
        (report, text)
    }

    fn entities_section(text: &str) -> &str {
        let start = text.find("  2\nENTITIES").expect("ENTITIES");
        let rest = &text[start..];
        rest.split("\n  0\nENDSEC").next().unwrap_or(rest)
    }

    #[test]
    fn writes_r2000_sections_and_a_line() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(1.0, 2.0),
            end: Point3::from_xy(3.0, 4.0),
        }));
        let (report, text) = write_to_string(&document);
        assert_eq!(report.entities_written, 1);
        assert!(text.contains("$ACADVER"));
        assert!(text.contains("AC1015"));
        assert!(text.contains("$TDUCREATE"));
        assert!(text.contains("$TDUUPDATE"));
        assert!(text.contains("  2\nLTYPE"));
        assert!(text.contains("  2\nLAYER"));
        assert!(text.contains("  2\nBLOCKS"));
        assert!(text.contains("*MODEL_SPACE"));
        assert!(text.contains("  2\nENTITIES"));
        assert!(text.contains("LINE"));
        assert!(!text.contains("libredwg"));
    }

    #[test]
    fn preserves_layer_color_linetype_scale_visibility_and_z() {
        let mut document = Document::default();
        document.ltscale = 2.5;
        document.units = DrawingUnits::Millimeters;
        let mut entity = Entity::new(Geometry::Line {
            start: Point3::new(1.0, 2.0, 3.0),
            end: Point3::new(4.0, 5.0, 6.0),
        });
        entity.layer = "0".into();
        entity.color = CadColor::Aci(1);
        entity.linetype = "DASHED".into();
        entity.linetype_scale = 0.5;
        entity.visible = false;
        document.add_entity(entity);
        let (_, text) = write_to_string(&document);
        assert!(text.contains("$INSUNITS"));
        assert!(text.contains("$LTSCALE"));
        assert!(text.contains("DASHED"));
        let entities = entities_section(&text);
        assert!(entities.contains(" 62\n1"));
        assert!(entities.contains(" 48\n0.5"));
        assert!(entities.contains(" 60\n1"));
        assert!(entities.contains(" 30\n3.0"));
        assert!(entities.contains(" 31\n6.0"));
    }

    #[test]
    fn preserves_closed_polyline_bulge_and_truecolor() {
        let mut document = Document::default();
        let mut entity = Entity::new(Geometry::LwPolyline {
            vertices: vec![
                PolyVertex {
                    point: Point3::from_xy(0.0, 0.0),
                    bulge: 0.5,
                vertex_id: Default::default(),
        },
                PolyVertex {
                    point: Point3::from_xy(2.0, 0.0),
                    bulge: 0.0,
                vertex_id: Default::default(),
        },
            ],
            closed: true,
            extrusion: Point3::new(0.0, 0.0, 1.0),
            linetype_generation_continuous: true,
        });
        entity.color = CadColor::Rgb {
            r: 10,
            g: 20,
            b: 30,
        };
        document.add_entity(entity);
        let (_, text) = write_to_string(&document);
        let entities = entities_section(&text);
        assert!(entities.contains("LWPOLYLINE"));
        assert!(entities.contains(" 70\n129"));
        assert!(entities.contains(" 42\n0.5"));
        assert!(entities.contains("420\n"));
    }

    #[test]
    fn dimension_exports_visible_block_geometry_not_a_dimension_entity() {
        let mut document = Document::default();
        document.blocks.insert(
            "*D1".into(),
            BlockDefinition {
                name: "*D1".into(),
                base_pt: Point3::from_xy(0.0, 0.0),
                entities: vec![Entity::new(Geometry::Line {
                    start: Point3::from_xy(10.0, 20.0),
                    end: Point3::from_xy(30.0, 20.0),
                })],
                ..Default::default()
            },
        );
        document.add_entity(Entity::new(Geometry::Dimension {
            block_name: "*D1".into(),
        }));
        let (report, text) = write_to_string(&document);
        let entities = entities_section(&text);
        assert!(report.entities_written >= 1);
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("visible block geometry")));
        assert!(entities.contains("LINE"));
        assert!(entities.contains("10.0"));
        assert!(entities.contains("30.0"));
        assert!(!entities.contains("  0\nDIMENSION"));
    }

    #[test]
    fn missing_dimension_block_warns_and_does_not_invent_a_dimension() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::Dimension {
            block_name: "*D1".into(),
        }));
        let (report, text) = write_to_string(&document);
        assert_eq!(report.entities_written, 0);
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("missing")));
        assert!(!entities_section(&text).contains("  0\nDIMENSION"));
    }

    #[test]
    fn mline_explodes_to_lines_with_a_warning() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::MLine {
            vertices: vec![Point3::from_xy(0.0, 0.0), Point3::from_xy(1.0, 0.0)],
            closed: false,
        }));
        let (report, text) = write_to_string(&document);
        assert_eq!(report.entities_written, 1);
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("MLINE")));
        assert!(entities_section(&text).contains("LINE"));
        assert!(!entities_section(&text).contains("  0\nMLINE"));
    }

    #[test]
    fn insert_attributes_export_as_text() {
        let mut document = Document::default();
        document.blocks.insert(
            "SYM".into(),
            BlockDefinition {
                name: "SYM".into(),
                base_pt: Point3::from_xy(0.0, 0.0),
                entities: Vec::new(),
                ..Default::default()
            },
        );
        document.add_entity(Entity::new(Geometry::Insert {
            block_name: "SYM".into(),
            insertion: Point3::from_xy(5.0, 6.0),
            scale: Point3::new(1.0, 1.0, 1.0),
            rotation: 0.0,
            extrusion: Point3::new(0.0, 0.0, 1.0),
            attribs: vec![TextData {
                insertion: Point3::from_xy(5.0, 7.0),
                height: 2.5,
                rotation: 0.0,
                value: "TAG".into(),
                extrusion: Point3::new(0.0, 0.0, 1.0),
                is_attrib_def: false,
            }],
            column_count: 1,
            row_count: 1,
            column_spacing: 0.0,
            row_spacing: 0.0,
            configuration: None,
        }));
        let (report, text) = write_to_string(&document);
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("attributes")));
        let entities = entities_section(&text);
        assert!(entities.contains("INSERT"));
        assert!(entities.contains("TEXT"));
        assert!(entities.contains("TAG"));
    }

    #[test]
    fn hatch_spline_edges_export_as_line_edges() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::Hatch(HatchData {
            extrusion: Point3::new(0.0, 0.0, 1.0),
            elevation: 0.0,
            solid_fill: true,
            paths: vec![HatchPath::Edges(vec![HatchEdge::Spline {
                control_points: vec![Point3::from_xy(0.0, 0.0), Point3::from_xy(1.0, 1.0)],
            }])],
            pattern_lines: Vec::new(),
        })));
        let (report, text) = write_to_string(&document);
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("HATCH spline")));
        assert!(entities_section(&text).contains("HATCH"));
        assert_eq!(report.entities_written, 1);
    }

    #[test]
    fn mtext_writes_ltr_flow_and_x_axis_not_column_count() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::MText(MTextData {
            insertion: Point3::from_xy(1.0, 2.0),
            height: 2.5,
            rotation: std::f64::consts::FRAC_PI_2,
            width: 40.0,
            value: "Hello".into(),
            extrusion: Point3::new(0.0, 0.0, 1.0),
        })));
        let (report, text) = write_to_string(&document);
        let entities = entities_section(&text);
        assert_eq!(report.entities_written, 1);
        assert!(entities.contains("MTEXT"));
        assert!(entities.contains("Hello"));
        // Group 72 is flow direction in DXF but LibreDWG also binds it to
        // column counts; omit it rather than write a value that aborts read.
        assert!(!entities.contains(" 72\n"));
        assert!(entities.contains(" 11\n0.0\n 21\n1.0\n"));
        assert!(!entities.contains(" 50\n90"));
    }

    #[test]
    fn r2000_encodes_turkish_text_and_layer_names() {
        let mut document = Document::default();
        document.layers.insert(
            "Şase".into(),
            cad_core::Layer {
                name: "Şase".into(),
                visible: true,
                frozen: false,
                color: CadColor::Aci(1),
                linetype: "CONTINUOUS".into(),
            },
        );
        let mut text = Entity::new(Geometry::Text(TextData {
            insertion: Point3::from_xy(0.0, 0.0),
            height: 2.5,
            rotation: 0.0,
            value: "Ölçü Çıkış İstanbul ğşıİ".into(),
            extrusion: Point3::new(0.0, 0.0, 1.0),
            is_attrib_def: false,
        }));
        text.layer = "Şase".into();
        document.add_entity(text);
        let (_, dxf) = write_to_string(&document);
        assert!(dxf.is_ascii(), "R2000 DXF must be ASCII plus \\U+ escapes");
        assert!(dxf.contains("\\U+00D6"));
        assert!(dxf.contains("\\U+015E"));
        assert!(dxf.contains("\\U+0131"));
        assert!(!dxf.contains("Ölçü"));
    }

    #[test]
    fn long_mtext_writes_group_3_then_group_1() {
        let mut document = Document::default();
        let value = format!("{}İstanbul", "A".repeat(500));
        document.add_entity(Entity::new(Geometry::MText(MTextData {
            insertion: Point3::from_xy(0.0, 0.0),
            height: 2.5,
            rotation: 0.0,
            width: 80.0,
            value,
            extrusion: Point3::new(0.0, 0.0, 1.0),
        })));
        let (_, dxf) = write_to_string(&document);
        let entities = entities_section(&dxf);
        let first_3 = entities.find("\n  3\n").expect("group 3");
        let last_1 = entities.rfind("\n  1\n").expect("group 1");
        assert!(
            first_3 < last_1,
            "group 3 chunks must precede the last group 1"
        );
        assert!(dxf.contains("\\U+0130"));
        let group_3_count = entities.matches("\n  3\n").count();
        assert!(group_3_count >= 2);
    }

    #[test]
    fn non_finite_coordinate_fails_the_save() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(f64::NAN, 0.0),
            end: Point3::from_xy(1.0, 0.0),
        }));
        let path = temp_path("nan.dxf");
        let err = write_dxf(&document, &path, &DxfExportOptions::default()).unwrap_err();
        let _ = fs::remove_file(&path);
        assert!(err.to_string().contains("non-finite"));
        assert!(!path.exists());
    }

    #[test]
    fn hatch_writes_acdbhatch_once() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::Hatch(HatchData {
            extrusion: Point3::new(0.0, 0.0, 1.0),
            elevation: 0.0,
            solid_fill: true,
            paths: vec![HatchPath::Polyline {
                vertices: vec![
                    PolyVertex {
                        point: Point3::from_xy(0.0, 0.0),
                        bulge: 0.0,
                    vertex_id: Default::default(),
        },
                    PolyVertex {
                        point: Point3::from_xy(1.0, 0.0),
                        bulge: 0.0,
                    vertex_id: Default::default(),
        },
                    PolyVertex {
                        point: Point3::from_xy(1.0, 1.0),
                        bulge: 0.0,
                    vertex_id: Default::default(),
        },
                ],
                closed: true,
            }],
            pattern_lines: Vec::new(),
        })));
        let (_, text) = write_to_string(&document);
        assert_eq!(text.matches("AcDbHatch").count(), 1);
    }

    #[test]
    fn create_block_writes_definition_and_insert() {
        use cad_core::{create_block_from_entities, EntitySpace, Point2};
        let mut document = Document::default();
        let a = document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(10.0, 0.0),
        }));
        let b = document.add_entity(Entity::new(Geometry::Circle {
            center: Point3::from_xy(5.0, 0.0),
            radius: 2.0,
            extrusion: cad_core::default_extrusion(),
        }));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[a.id, b.id],
            "TestBlock",
            Point2::new(5.0, 0.0),
            true,
        )
        .unwrap();
        let (_, text) = write_to_string(&document);
        assert!(text.contains("TestBlock"));
        assert!(text.contains("  0\nBLOCK"));
        assert!(text.contains("  0\nINSERT"));
        let entities = entities_section(&text);
        assert!(entities.contains("INSERT"));
        assert!(!entities.contains("  0\nLINE"));
        assert!(!entities.contains("  0\nCIRCLE"));
        assert!(text.contains("  0\nLINE"));
        assert!(text.contains("  0\nCIRCLE"));
    }

    #[test]
    fn nested_block_writes_both_definitions() {
        use cad_core::{create_block_from_entities, EntitySpace, Point2};
        let mut document = Document::default();
        let inner = document.add_entity(Entity::new(Geometry::Circle {
            center: Point3::from_xy(0.0, 0.0),
            radius: 1.0,
            extrusion: cad_core::default_extrusion(),
        }));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[inner.id],
            "B",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let outer = document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(8.0, 0.0),
            end: Point3::from_xy(10.0, 0.0),
        }));
        let b_id = document.model_space[0].id;
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[outer.id, b_id],
            "A",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let (_, text) = write_to_string(&document);
        assert!(text.contains("  2\nA\n"));
        assert!(text.contains("  2\nB\n"));
        assert!(text.contains("  0\nINSERT"));
        let entities = entities_section(&text);
        assert!(entities.contains("INSERT"));
        assert!(!entities.contains("  0\nLINE"));
        assert!(!entities.contains("  0\nCIRCLE"));
    }

    #[test]
    fn renamed_block_writes_new_definition_and_insert_name() {
        use cad_core::{create_block_from_entities, EntitySpace, Point2};
        let mut document = Document::default();
        let line = document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(10.0, 0.0),
        }));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[line.id],
            "Motor",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        document.rename_block("Motor", "Motor Drive").unwrap();
        let (_, text) = write_to_string(&document);
        assert!(text.contains("Motor Drive"));
        assert!(!text.contains("  2\nMotor\n"));
        match &document.model_space[0].geometry {
            Geometry::Insert { block_name, .. } => assert_eq!(block_name, "Motor Drive"),
            other => panic!("{other:?}"),
        }
    }
}
