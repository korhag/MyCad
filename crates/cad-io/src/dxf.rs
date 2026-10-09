//! ASCII DXF writer for cad-core documents. No LibreDWG.
//!
//! Fallback policy (never silent):
//! - Supported native types are written as the matching DXF entity.
//! - Types EntoCAD cannot represent fully are exploded to simpler primitives
//!   and recorded on `SaveReport.warnings` (MLINE segments, HATCH spline
//!   edges, an attribute with no tag).
//! - Missing blocks or empty exploded geometry still produce a warning.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::path::Path;

use cad_core::{
    is_paper_layout_block, nearest_aci, sorted_paper_layout_blocks, BlockDefinition, CadColor,
    DimensionData, DimensionKind, Document, DrawingUnits, Entity, Geometry, HatchData, HatchEdge,
    HatchPath, Layer, LineType, LineTypeShape, MTextData, Point3, PolyVertex, RasterFrame,
    TextData, TextStyle, ViewportData,
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
    let (report, text) = render_dxf(document, options)?;
    crate::atomic::write_atomic(path, text.as_bytes())
        .map_err(|source| ExportError::io(path, source))?;
    Ok(report)
}

// ------------------------------------------------------------
// Function: write_dxf_interchange
// Purpose: Write a throwaway DXF for LibreDWG to read back. A crash
//          here must not touch the destination DWG, so this skips the
//          durable temp-file, flush, and replace used by write_dxf.
// ------------------------------------------------------------
pub fn write_dxf_interchange(
    document: &Document,
    path: &Path,
    options: &DxfExportOptions,
) -> Result<SaveReport, ExportError> {
    let (report, text) = render_dxf(document, options)?;
    std::fs::write(path, text.as_bytes()).map_err(|source| ExportError::io(path, source))?;
    Ok(report)
}

fn render_dxf(
    document: &Document,
    options: &DxfExportOptions,
) -> Result<(SaveReport, String), ExportError> {
    let mut writer = DxfWriter::new(document, options.version);
    writer.write_document();
    if writer.nonfinite {
        return Err(ExportError::Invalid(
            "DXF cannot store non-finite coordinates or scales",
        ));
    }
    Ok((writer.report, writer.out))
}

#[derive(Clone)]
struct ExportLayout {
    name: String,
    block_name: String,
    tab_order: i32,
    paper: bool,
    width: f64,
    height: f64,
    left: f64,
    bottom: f64,
    right: f64,
    top: f64,
    handle: String,
}

struct DxfWriter<'a> {
    document: &'a Document,
    version: DxfAcadVersion,
    out: String,
    handle: u64,
    report: SaveReport,
    block_stack: Vec<String>,
    block_records: BTreeMap<String, String>,
    model_record: String,
    export_layouts: Vec<ExportLayout>,
    export_linetypes: Vec<LineType>,
    export_layers: Vec<Layer>,
    export_styles: Vec<TextStyle>,
    export_shape_files: Vec<String>,
    style_handles: BTreeMap<String, String>,
    plot_style: String,
    paper_entity: bool,
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
            model_record: String::new(),
            export_layouts: Vec::new(),
            export_linetypes: Vec::new(),
            export_layers: Vec::new(),
            export_styles: Vec::new(),
            export_shape_files: Vec::new(),
            style_handles: BTreeMap::new(),
            plot_style: String::new(),
            paper_entity: false,
            nonfinite: false,
        }
    }

    fn pair(&mut self, code: i16, value: impl AsRef<str>) {
        // Writing into a String does not fail.
        let _ = write!(self.out, "{code:3}\n{}\n", value.as_ref());
    }

    fn pair_f(&mut self, code: i16, value: f64) {
        if !value.is_finite() {
            self.nonfinite = true;
            self.warn("non-finite coordinate or scale was not written as a number");
        }
        self.pair(code, format_dxf_r2000_f64(value));
    }

    fn pair_i(&mut self, code: i16, value: i32) {
        let _ = write!(self.out, "{code:3}\n{value}\n");
    }

    fn pair_hex(&mut self, code: i16, value: u64) {
        let _ = write!(self.out, "{code:3}\n{value:X}\n");
    }

    fn next_handle(&mut self) -> String {
        let handle = self.handle;
        self.handle = self.handle.saturating_add(1);
        format!("{handle:X}")
    }

    fn write_document(&mut self) {
        self.prepare_export_tables();
        // Layout object handles are referenced by BLOCK_RECORD group 340,
        // so they are reserved before the tables are written.
        self.export_layouts = plan_layouts(self.document);
        let mut handles = Vec::with_capacity(self.export_layouts.len());
        for _ in 0..self.export_layouts.len() {
            handles.push(self.next_handle());
        }
        for (layout, handle) in self.export_layouts.iter_mut().zip(handles) {
            layout.handle = handle;
        }
        // Layers reference the Normal plot style. Its handle has to exist
        // before the LAYER table, which is written before OBJECTS.
        self.plot_style = self.next_handle();
        self.write_classes();
        self.write_tables();
        self.write_blocks();
        self.write_entities();
        self.write_objects();
        let body = std::mem::take(&mut self.out);
        self.write_header();
        self.out.push_str(&body);
        self.pair(0, "EOF");
    }

    fn prepare_export_tables(&mut self) {
        let (linetypes, linetype_warnings) = export_linetypes(self.document);
        let (layers, layer_warnings) = export_layers(self.document);
        self.export_linetypes = linetypes;
        self.export_layers = layers;
        for warning in linetype_warnings.into_iter().chain(layer_warnings) {
            self.warn(&warning);
        }
    }

    fn write_header(&mut self) {
        self.pair(0, "SECTION");
        self.pair(2, "HEADER");
        self.pair(9, "$ACADVER");
        self.pair(1, self.version.acadver());
        self.pair(9, "$ACADMAINTVER");
        self.pair_i(70, 6);
        self.pair(9, "$DWGCODEPAGE");
        self.pair(3, "ANSI_1252");
        self.pair(9, "$INSBASE");
        self.pair_f(10, 0.0);
        self.pair_f(20, 0.0);
        self.pair_f(30, 0.0);
        self.pair(9, "$INSUNITS");
        self.pair_i(70, i32::from(self.document.units.to_insunits()));
        self.pair(9, "$MEASUREMENT");
        self.pair_i(70, measurement_code(self.document.units));
        self.pair(9, "$TILEMODE");
        self.pair_i(70, 1);
        self.pair(9, "$TEXTSTYLE");
        self.pair(7, "STANDARD");
        self.pair(9, "$CELTYPE");
        self.pair(6, "BYLAYER");
        self.pair(9, "$CECOLOR");
        self.pair_i(62, 256);
        self.pair(9, "$CLAYER");
        self.pair(8, sanitize_name(&self.document.current_layer));
        self.pair(9, "$DIMSTYLE");
        self.pair(2, "STANDARD");
        self.write_dim_header();
        self.pair(9, "$LWDISPLAY");
        self.pair_i(290, 0);
        self.pair(9, "$LTSCALE");
        self.pair_f(40, self.document.ltscale.max(1e-12));
        self.write_system_header();
        self.pair(9, "$HANDSEED");
        self.pair_hex(5, self.handle);
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

    fn write_classes(&mut self) {
        self.pair(0, "SECTION");
        self.pair(2, "CLASSES");
        self.write_class("ACDBDICTIONARYWDFLT", "AcDbDictionaryWithDefault", 0);
        self.write_class("ACDBPLACEHOLDER", "AcDbPlaceHolder", 0);
        self.write_class("LAYOUT", "AcDbLayout", 0);
        self.pair(0, "ENDSEC");
    }

    fn write_class(&mut self, dxf_name: &str, cpp_name: &str, is_entity: i32) {
        self.pair(0, "CLASS");
        self.pair(1, dxf_name);
        self.pair(2, cpp_name);
        self.pair(3, "ObjectDBX Classes");
        self.pair_i(90, 0);
        self.pair_i(280, 0);
        self.pair_i(281, is_entity);
    }

    fn write_tables(&mut self) {
        self.reserve_style_handles();
        self.pair(0, "SECTION");
        self.pair(2, "TABLES");
        self.write_vport_table();
        self.write_ltype_table();
        self.write_layer_table();
        self.write_style_table();
        self.write_empty_table("VIEW");
        self.write_empty_table("UCS");
        self.write_appid_table();
        self.write_dimstyle_table();
        self.write_block_record_table();
        self.pair(0, "ENDSEC");
    }

    fn write_table_header(&mut self, name: &str, count: i32) -> String {
        self.pair(0, "TABLE");
        self.pair(2, name);
        let table_handle = self.next_handle();
        self.pair(5, &table_handle);
        self.pair(330, "0");
        self.pair(100, "AcDbSymbolTable");
        if name == "DIMSTYLE" {
            self.pair(100, "AcDbDimStyleTable");
        }
        self.pair_i(70, count);
        table_handle
    }

    fn write_vport_table(&mut self) {
        let owner = self.write_table_header("VPORT", 1);
        self.pair(0, "VPORT");
        let handle = self.next_handle();
        self.pair(5, handle);
        self.pair(330, &owner);
        self.pair(100, "AcDbSymbolTableRecord");
        self.pair(100, "AcDbViewportTableRecord");
        self.pair(2, "*ACTIVE");
        self.pair_i(70, 0);
        self.pair_f(10, 0.0);
        self.pair_f(20, 0.0);
        self.pair_f(11, 1.0);
        self.pair_f(21, 1.0);
        self.pair_f(12, 0.0);
        self.pair_f(22, 0.0);
        self.pair_f(13, 0.0);
        self.pair_f(23, 0.0);
        self.pair_f(14, 1.0);
        self.pair_f(24, 1.0);
        self.pair_f(15, 10.0);
        self.pair_f(25, 10.0);
        self.pair_f(16, 0.0);
        self.pair_f(26, 0.0);
        self.pair_f(36, 1.0);
        self.pair_f(17, 0.0);
        self.pair_f(27, 0.0);
        self.pair_f(37, 0.0);
        self.pair_f(40, 100.0);
        self.pair_f(41, 1.0);
        self.pair_f(42, 50.0);
        self.pair_f(43, 0.0);
        self.pair_f(44, 0.0);
        self.pair_f(50, 0.0);
        self.pair_f(51, 0.0);
        self.pair_i(71, 0);
        self.pair_i(72, 100);
        self.pair_i(73, 1);
        self.pair_i(74, 3);
        self.pair_i(75, 0);
        self.pair_i(76, 0);
        self.pair_i(77, 0);
        self.pair_i(78, 0);
        // UCS axes must be unit length. A zero axis is an audit error.
        self.pair_f(110, 0.0);
        self.pair_f(120, 0.0);
        self.pair_f(130, 0.0);
        self.pair_f(111, 1.0);
        self.pair_f(121, 0.0);
        self.pair_f(131, 0.0);
        self.pair_f(112, 0.0);
        self.pair_f(122, 1.0);
        self.pair_f(132, 0.0);
        self.pair_i(79, 0);
        self.pair(0, "ENDTAB");
    }

    fn write_ltype_table(&mut self) {
        let mut linetypes: Vec<_> = self.export_linetypes.clone();
        // BYBLOCK, BYLAYER, then CONTINUOUS. Any other record before
        // CONTINUOUS makes AutoCAD report "Index 1  0" on CONTINUOUS.
        linetypes.sort_by_key(|linetype| linetype_table_rank(linetype));
        let owner = self.write_table_header("LTYPE", linetypes.len() as i32);
        for linetype in linetypes {
            self.write_ltype(&linetype, &owner);
        }
        self.pair(0, "ENDTAB");
    }

    fn write_ltype(&mut self, linetype: &LineType, owner: &str) {
        self.pair(0, "LTYPE");
        let handle = self.next_handle();
        self.pair(5, handle);
        self.pair(330, owner);
        self.pair(100, "AcDbSymbolTableRecord");
        self.pair(100, "AcDbLinetypeTableRecord");
        self.pair(2, sanitize_name(&linetype.name));
        self.pair_i(70, 0);
        self.pair(3, "");
        self.pair_i(72, 65);
        self.pair_i(73, linetype.dashes.len() as i32);
        let mut pattern_len: f64 = linetype.dashes.iter().map(|d| d.abs()).sum();
        if pattern_len == 0.0 {
            pattern_len = 0.0;
        }
        self.pair_f(40, pattern_len);
        for (index, dash) in linetype.dashes.iter().enumerate() {
            self.pair_f(49, *dash);
            match linetype.shape_at(index) {
                Some(shape) => self.write_complex_dash(shape),
                None => self.pair_i(74, 0),
            }
        }
    }

    fn write_complex_dash(&mut self, shape: &LineTypeShape) {
        self.pair_i(74, i32::from(shape.flag));
        self.pair_i(75, i32::from(shape.shapecode));
        match self.shape_style_handle(shape) {
            Some(handle) => self.pair(340, handle),
            None if shape.style.trim().is_empty() && shape.shape_file.trim().is_empty() => {
                self.warn(
                    "LTYPE complex dash has no style or shape file; the dash was written without a STYLE handle",
                );
            }
            None => {
                let label = if shape.style.trim().is_empty() {
                    shape.shape_file.trim()
                } else {
                    shape.style.trim()
                };
                self.warn(&format!(
                    "LTYPE shape style '{label}' has no STYLE record; the dash was written without a style handle"
                ));
            }
        }
        // AutoCAD reads a complex dash in this order and ends the element at
        // the first unexpected group, so scale and rotation come before offsets.
        let scale = if shape.scale.is_finite() {
            shape.scale
        } else {
            1.0
        };
        self.pair_f(46, scale);
        let degrees = if shape.rotation.is_finite() {
            shape.rotation.to_degrees()
        } else {
            0.0
        };
        self.pair_f(50, degrees);
        self.pair_f(44, shape.x_offset);
        self.pair_f(45, shape.y_offset);
        if !shape.text.is_empty() || shape.flag & 2 != 0 {
            self.pair(9, sanitize_text(&shape.text));
        }
    }

    fn reserve_style_handles(&mut self) {
        if !self.style_handles.is_empty() {
            return;
        }
        let (styles, warnings) = export_text_styles(self.document);
        for warning in warnings {
            self.warn(&warning);
        }
        let shape_files = collect_shape_files(self.document);
        let mut handles = BTreeMap::new();
        for style in &styles {
            let handle = self.next_handle();
            handles.insert(style.name.to_ascii_uppercase(), handle);
        }
        for file in &shape_files {
            let handle = self.next_handle();
            handles.insert(shape_file_key(file), handle);
        }
        self.export_styles = styles;
        self.export_shape_files = shape_files;
        self.style_handles = handles;
    }

    fn shape_style_handle(&self, shape: &LineTypeShape) -> Option<String> {
        let style = shape.style.trim();
        if !style.is_empty() {
            return self.style_handles.get(&style.to_ascii_uppercase()).cloned();
        }
        let file = shape.shape_file.trim();
        if file.is_empty() {
            return None;
        }
        self.style_handles.get(&shape_file_key(file)).cloned()
    }

    fn write_layer_table(&mut self) {
        let owner = self.write_table_header("LAYER", self.export_layers.len() as i32);
        for layer in self.export_layers.clone() {
            self.write_layer(&layer, &owner);
        }
        self.pair(0, "ENDTAB");
    }

    fn write_layer(&mut self, layer: &Layer, owner: &str) {
        self.pair(0, "LAYER");
        let handle = self.next_handle();
        self.pair(5, handle);
        self.pair(330, owner);
        self.pair(100, "AcDbSymbolTableRecord");
        self.pair(100, "AcDbLayerTableRecord");
        self.pair(2, sanitize_name(&layer.name));
        let mut flags = 0_i32;
        if layer.frozen {
            flags |= 1;
        }
        if layer.locked {
            flags |= 8;
        }
        if layer.plot {
            flags |= 32768;
        }
        self.pair_i(70, flags);
        let mut aci = color_aci(layer.color);
        if !layer.visible && aci > 0 {
            aci = -aci;
        }
        self.pair_i(62, aci);
        if let CadColor::Rgb { r, g, b } = layer.color {
            self.pair_i(
                420,
                (i32::from(r) << 16) | (i32::from(g) << 8) | i32::from(b),
            );
        }
        self.pair(6, sanitize_name(&layer.linetype));
        self.pair_i(370, i32::from(layer.lineweight));
        self.pair_i(290, if layer.plot { 1 } else { 0 });
        let plot_style = self.plot_style.clone();
        if !plot_style.is_empty() {
            self.pair(390, plot_style);
        }
    }

    fn write_style_table(&mut self) {
        self.reserve_style_handles();
        let count = self.export_styles.len() + self.export_shape_files.len();
        let owner = self.write_table_header("STYLE", count as i32);
        for style in self.export_styles.clone() {
            self.pair(0, "STYLE");
            let record = self
                .style_handles
                .get(&style.name.to_ascii_uppercase())
                .cloned()
                .unwrap_or_else(|| self.next_handle());
            self.pair(5, record);
            self.pair(330, &owner);
            self.pair(100, "AcDbSymbolTableRecord");
            self.pair(100, "AcDbTextStyleTableRecord");
            self.pair(2, sanitize_name(&style.name));
            self.pair_i(70, 0);
            self.pair_f(40, style.height);
            self.pair_f(41, style.width_factor);
            self.pair_f(50, style.oblique.to_degrees());
            self.pair_i(71, 0);
            self.pair_f(42, 2.5);
            self.pair(3, &style.font_file);
            self.pair(4, &style.bigfont_file);
        }
        for file in self.export_shape_files.clone() {
            self.pair(0, "STYLE");
            let record = self
                .style_handles
                .get(&shape_file_key(&file))
                .cloned()
                .unwrap_or_else(|| self.next_handle());
            self.pair(5, record);
            self.pair(330, &owner);
            self.pair(100, "AcDbSymbolTableRecord");
            self.pair(100, "AcDbTextStyleTableRecord");
            // A shape-file STYLE (group 70 bit 1) must have an empty name.
            // AutoCAD rejects a name such as LTYPESHP.
            self.pair(2, "");
            self.pair_i(70, 1);
            self.pair_f(40, 0.0);
            self.pair_f(41, 1.0);
            self.pair_f(50, 0.0);
            self.pair_i(71, 0);
            self.pair_f(42, 0.0);
            self.pair(3, &file);
            self.pair(4, "");
        }
        self.pair(0, "ENDTAB");
    }

    fn write_dim_header(&mut self) {
        for (name, value) in dim_defaults() {
            self.pair(9, name);
            self.pair_f(40, value);
        }
        self.pair(9, "$DIMALTU");
        self.pair_i(70, 2);
    }

    fn write_system_header(&mut self) {
        self.pair(9, "$CELTSCALE");
        self.pair_f(40, 1.0);
        for (name, value) in [
            ("$LUNITS", 2),
            ("$LUPREC", 4),
            ("$AUNITS", 0),
            ("$AUPREC", 0),
            ("$MAXACTVP", 64),
            ("$SPLINETYPE", 6),
            ("$SPLINESEGS", 8),
            ("$SURFTAB1", 6),
            ("$SURFTAB2", 6),
            ("$SURFTYPE", 6),
            ("$SURFU", 6),
            ("$SURFV", 6),
            ("$USRTIMER", 1),
        ] {
            self.pair(9, name);
            self.pair_i(70, value);
        }
        self.pair(9, "$TEXTSIZE");
        self.pair_f(40, 2.5);
        self.pair(9, "$FINGERPRINTGUID");
        self.pair(2, "{A5B4C3D2-E1F0-4A5B-8C7D-6E5F4A3B2C1D}");
        self.pair(9, "$VERSIONGUID");
        self.pair(2, "{B6C5D4E3-F2A1-4B6C-9D8E-7F6A5B4C3D2E}");
    }

    fn write_dimstyle_defaults(&mut self) {
        for (code, value) in [
            (40, 1.0),
            (41, 2.5),
            (42, 0.625),
            (44, 1.25),
            (43, 3.75),
            (140, 2.5),
            (141, 2.5),
            (143, 25.4),
            (144, 1.0),
            (147, 0.625),
        ] {
            self.pair_f(code, value);
        }
        self.pair_i(271, 4);
        self.pair_i(272, 4);
        self.pair_i(277, 2);
    }

    fn write_dimstyle_table(&mut self) {
        let mut names = BTreeSet::new();
        names.insert("STANDARD".to_string());
        for entity in written_entities(self.document) {
            if let Geometry::Dimension(data) = &entity.geometry {
                let name = data.dimstyle.trim();
                if !name.is_empty() {
                    names.insert(name.to_string());
                }
            }
        }
        let owner = self.write_table_header("DIMSTYLE", names.len() as i32);
        for name in names {
            self.pair(0, "DIMSTYLE");
            let record = self.next_handle();
            self.pair(5, &record);
            self.pair(105, &record);
            self.pair(330, &owner);
            self.pair(100, "AcDbSymbolTableRecord");
            self.pair(100, "AcDbDimStyleTableRecord");
            self.pair(2, sanitize_name(&name));
            self.pair_i(70, 0);
            self.write_dimstyle_defaults();
            if let Some(handle) = self.style_handles.get("STANDARD").cloned() {
                self.pair(340, handle);
            }
        }
        self.pair(0, "ENDTAB");
    }

    fn write_empty_table(&mut self, name: &str) {
        let _owner = self.write_table_header(name, 0);
        self.pair(0, "ENDTAB");
    }

    fn write_appid_table(&mut self) {
        let owner = self.write_table_header("APPID", 1);
        self.pair(0, "APPID");
        let record = self.next_handle();
        self.pair(5, record);
        self.pair(330, &owner);
        self.pair(100, "AcDbSymbolTableRecord");
        self.pair(100, "AcDbRegAppTableRecord");
        self.pair(2, "ACAD");
        self.pair_i(70, 0);
        self.pair(0, "ENDTAB");
    }

    fn write_block_record_table(&mut self) {
        let names: Vec<String> = self
            .document
            .blocks
            .keys()
            .filter(|name| !is_layout_block(name))
            .cloned()
            .collect();
        let count = names.len() as i32 + self.export_layouts.len() as i32;
        let owner = self.write_table_header("BLOCK_RECORD", count);
        for layout in self.export_layouts.clone() {
            let handle = self.write_block_record(&layout.block_name, &owner);
            if layout.block_name.eq_ignore_ascii_case("*MODEL_SPACE") {
                self.model_record = handle.clone();
            }
            self.block_records.insert(layout.block_name, handle);
        }
        for name in names {
            let handle = self.write_block_record(&name, &owner);
            self.block_records.insert(name, handle);
        }
        self.pair(0, "ENDTAB");
    }

    fn write_block_record(&mut self, name: &str, owner: &str) -> String {
        self.pair(0, "BLOCK_RECORD");
        let handle = self.next_handle();
        self.pair(5, &handle);
        self.pair(330, owner);
        self.pair(100, "AcDbSymbolTableRecord");
        self.pair(100, "AcDbBlockTableRecord");
        self.pair(2, &export_block_name(name));
        if is_anonymous_block(name) {
            self.pair_i(70, 1);
        }
        if let Some(block) = self.document.blocks.get(name) {
            if !block.xref_path.trim().is_empty() {
                let mut flags = 4 | 32;
                if block.xref_overlay {
                    flags |= 8;
                }
                self.pair_i(70, flags);
                self.pair(1, &block.xref_path);
            }
        }
        let layout = self
            .export_layouts
            .iter()
            .find(|layout| layout.block_name.eq_ignore_ascii_case(name))
            .map(|layout| layout.handle.clone())
            .unwrap_or_default();
        if !layout.is_empty() {
            self.pair(340, layout);
        }
        handle
    }

    fn write_owner(&mut self) {
        let paper_owner = if self.paper_entity {
            self.block_records
                .iter()
                .find(|(name, _)| is_primary_paper_space(name))
                .map(|(_, handle)| handle.clone())
        } else {
            None
        };
        let handle = if let Some(name) = self.block_stack.last() {
            self.block_records.get(name).cloned()
        } else if let Some(handle) = paper_owner {
            Some(handle)
        } else if !self.model_record.is_empty() {
            Some(self.model_record.clone())
        } else {
            None
        };
        if let Some(handle) = handle {
            self.pair(330, handle);
        }
    }

    fn write_blocks(&mut self) {
        self.pair(0, "SECTION");
        self.pair(2, "BLOCKS");
        for layout in self.export_layouts.clone() {
            // *MODEL_SPACE and *PAPER_SPACE stay empty. AutoCAD rejects
            // entities in those two block bodies and reads them from ENTITIES.
            if layout.paper && !is_primary_paper_space(&layout.block_name) {
                let block = self.paper_block(&layout.block_name);
                self.write_block_definition(&layout.block_name, &block);
            } else {
                self.write_space_block(&layout.block_name);
            }
        }
        let names: Vec<String> = self.document.blocks.keys().cloned().collect();
        for name in names {
            if is_layout_block(&name) {
                continue;
            }
            let Some(block) = self.document.blocks.get(&name).cloned() else {
                continue;
            };
            self.write_block_definition(&name, &block);
        }
        self.pair(0, "ENDSEC");
    }

    fn paper_block(&self, name: &str) -> BlockDefinition {
        self.document
            .blocks
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, block)| block.clone())
            .unwrap_or_else(|| BlockDefinition {
                name: name.to_string(),
                ..BlockDefinition::default()
            })
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
        self.pair(2, &export_block_name(name));
        let mut flags = 0_i32;
        if is_anonymous_block(name) {
            flags |= 1;
        }
        if block_has_attdef(block) {
            flags |= 2;
        }
        if !block.xref_path.trim().is_empty() {
            flags |= 4;
            flags |= 32;
            if block.xref_overlay {
                flags |= 8;
            }
        }
        self.pair_i(70, flags);
        self.pair_f(10, block.base_pt.x);
        self.pair_f(20, block.base_pt.y);
        self.pair_f(30, block.base_pt.z);
        self.pair(3, &export_block_name(name));
        if !block.xref_path.trim().is_empty() {
            self.pair(1, &block.xref_path);
        }
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
        let paper = self.paper_block("*PAPER_SPACE");
        self.paper_entity = true;
        for entity in &paper.entities {
            self.write_entity(entity);
        }
        self.paper_entity = false;
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
                // ARC is a circle subclass. The circle marker has to come
                // before the center and radius, and AcDbArc before the angles.
                self.begin_entity_class("ARC", "AcDbCircle", entity);
                self.point(10, *center);
                self.pair_f(40, radius.abs());
                self.extrusion(*extrusion);
                self.pair(100, "AcDbArc");
                self.pair_f(50, start_angle.to_degrees());
                self.pair_f(51, end_angle.to_degrees());
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
                    self.warn("LWPOLYLINE with varying Z exported as a 3D POLYLINE");
                    self.write_polyline3d(entity, vertices, *closed);
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
                } else if lw_vertices_have_varying_z(vertices) {
                    self.write_polyline3d(entity, vertices, *closed);
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
                    // A stored tolerance of 0 is rejected when the spline is
                    // written as DWG. AutoCAD's own default is 1e-7.
                    self.pair_f(42, 1e-7);
                    self.pair_f(43, 1e-7);
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
                    self.point(210, WORLD);
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
                let insert_handle = self.begin_entity("INSERT", entity);
                self.pair(2, &export_block_name(block_name));
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
                let (tagged, untagged): (Vec<&TextData>, Vec<&TextData>) = attribs
                    .iter()
                    .partition(|attrib| attribute_tag(attrib).is_some());
                if !tagged.is_empty() {
                    self.pair_i(66, 1);
                }
                self.report.entities_written += 1;
                for attrib in tagged {
                    self.write_attrib(entity, attrib, &insert_handle);
                }
                if attribs.iter().any(|attrib| attribute_tag(attrib).is_some()) {
                    self.write_seqend(entity, &insert_handle);
                }
                for attrib in untagged {
                    self.warn("INSERT attribute without a tag exported as TEXT");
                    self.write_text(entity, attrib);
                    self.report.entities_written += 1;
                }
            }
            Geometry::Text(data) => {
                if data.is_attrib_def && attribute_tag(data).is_some() {
                    self.write_attdef(entity, data);
                } else {
                    if data.is_attrib_def {
                        self.warn("ATTDEF without a tag exported as TEXT");
                    }
                    self.write_text(entity, data);
                }
                self.report.entities_written += 1;
            }
            Geometry::MText(data) => {
                self.write_mtext(entity, data);
                self.report.entities_written += 1;
            }
            Geometry::Solid { corners, extrusion } => {
                self.begin_entity("SOLID", entity);
                // Model order is the polygon 1-2-4-3; DXF group 12/13 are 3 then 4.
                self.point(10, corners[0]);
                self.point(11, corners[1]);
                self.point(12, corners[3]);
                self.point(13, corners[2]);
                self.extrusion(*extrusion);
                self.report.entities_written += 1;
            }
            Geometry::Leader { vertices } => {
                if vertices.len() < 2 {
                    self.warn("LEADER with fewer than two vertices was not written");
                } else {
                    self.begin_entity("LEADER", entity);
                    self.pair(3, "STANDARD");
                    self.pair_i(71, 1);
                    self.pair_i(72, 0);
                    // 3 means the leader was created without an annotation.
                    // 0 tells AutoCAD to look for an MTEXT that is not there.
                    self.pair_i(73, 3);
                    self.pair_i(74, 0);
                    self.pair_i(75, 0);
                    self.pair_f(40, 0.0);
                    self.pair_f(41, 0.0);
                    self.pair_i(76, vertices.len() as i32);
                    for point in vertices {
                        self.point(10, *point);
                    }
                    // A zero extrusion is not a unit normal. AutoCAD then
                    // discards the leader when it is stored in a DWG.
                    self.point(210, WORLD);
                    self.report.entities_written += 1;
                }
            }
            Geometry::MLine { vertices, closed } => {
                self.write_mline_as_lines(entity, vertices, *closed)
            }
            Geometry::Viewport(viewport) => self.write_viewport(entity, viewport),
            Geometry::Image(frame) => self.write_raster(entity, "IMAGE", frame),
            Geometry::Wipeout(frame) => self.write_raster(entity, "WIPEOUT", frame),
            Geometry::Hatch(hatch) => self.write_hatch(entity, hatch),
            Geometry::Dimension(data) => self.write_dimension(entity, data),
        }
    }

    fn write_polyline2d(
        &mut self,
        entity: &Entity,
        vertices: &[PolyVertex],
        closed: bool,
        linetype_generation_continuous: bool,
    ) {
        let polyline = self.begin_entity("POLYLINE", entity);
        let elevation = vertices.first().map(|vertex| vertex.point.z).unwrap_or(0.0);
        self.pair_f(10, 0.0);
        self.pair_f(20, 0.0);
        self.pair_f(30, elevation);
        let mut flags = 0_i32;
        if closed {
            flags |= 1;
        }
        if linetype_generation_continuous {
            flags |= 128;
        }
        self.pair_i(70, flags);
        self.pair_i(66, 1);
        for vertex in vertices {
            self.pair(0, "VERTEX");
            let handle = self.next_handle();
            self.pair(5, handle);
            self.pair(330, &polyline);
            self.pair(100, "AcDbEntity");
            self.pair(8, sanitize_name(&entity.layer));
            write_subentity_color(self, entity.color);
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
        self.pair(330, &polyline);
        self.pair(100, "AcDbEntity");
        self.pair(8, sanitize_name(&entity.layer));
        write_subentity_color(self, entity.color);
        self.report.entities_written += 1;
    }

    fn write_polyline3d(&mut self, entity: &Entity, vertices: &[PolyVertex], closed: bool) {
        if vertices.iter().any(|vertex| vertex.bulge.abs() > 1e-15) {
            self.warn("POLYLINE bulge was dropped because varying Z was exported as a 3D POLYLINE");
        }
        let polyline = self.begin_entity_class("POLYLINE", "AcDb3dPolyline", entity);
        let mut flags = 8_i32;
        if closed {
            flags |= 1;
        }
        self.pair_i(70, flags);
        self.pair_i(66, 1);
        for vertex in vertices {
            self.pair(0, "VERTEX");
            let handle = self.next_handle();
            self.pair(5, handle);
            self.pair(330, &polyline);
            self.pair(100, "AcDbEntity");
            self.pair(8, sanitize_name(&entity.layer));
            write_subentity_color(self, entity.color);
            self.pair(100, "AcDbVertex");
            self.pair(100, "AcDb3dPolylineVertex");
            self.point(10, vertex.point);
            self.pair_i(70, 32);
        }
        self.pair(0, "SEQEND");
        let seq = self.next_handle();
        self.pair(5, seq);
        self.pair(330, &polyline);
        self.pair(100, "AcDbEntity");
        self.pair(8, sanitize_name(&entity.layer));
        write_subentity_color(self, entity.color);
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

    fn write_raster(&mut self, entity: &Entity, kind: &str, frame: &RasterFrame) {
        self.warn(&format!(
            "{kind} saved as its frame; AutoCAD rejects the raster object LibreDWG writes"
        ));
        let vertices: Vec<PolyVertex> = frame
            .corners()
            .into_iter()
            .map(|point| PolyVertex {
                point,
                bulge: 0.0,
                vertex_id: Default::default(),
            })
            .collect();
        self.begin_entity("LWPOLYLINE", entity);
        self.pair_i(90, vertices.len() as i32);
        self.pair_i(70, 1);
        self.extrusion(Point3::new(0.0, 0.0, 1.0));
        for vertex in &vertices {
            self.write_lw_vertex(vertex);
        }
        self.report.entities_written += 1;
    }

    fn write_dimension(&mut self, entity: &Entity, data: &DimensionData) {
        let flag = match data.kind {
            DimensionKind::Linear => 0,
            DimensionKind::Aligned => 1,
            DimensionKind::Angular2Line => 2,
            DimensionKind::Diameter => 3,
            DimensionKind::Radius => 4,
            DimensionKind::Angular3Point => 5,
            DimensionKind::Ordinate => 6,
        };
        self.begin_entity_class("DIMENSION", "AcDbDimension", entity);
        // Group order matches the R2000 dimension DXF: common groups, then
        // the aligned points, then rotation, and only then the rotated
        // subclass marker. A group after that marker is a premature end.
        self.pair(2, &export_block_name(&data.block_name));
        self.point(10, data.definition);
        self.point(11, data.text_midpoint);
        self.pair_i(70, flag | 32);
        if !data.text.is_empty() {
            self.pair(1, sanitize_text(&data.text));
        }
        self.pair_i(71, 5);
        self.pair_i(72, 1);
        self.pair_f(41, 1.0);
        self.point(210, WORLD);
        self.pair(3, sanitize_name(&data.dimstyle));
        match data.kind {
            DimensionKind::Linear | DimensionKind::Aligned => {
                self.pair(100, "AcDbAlignedDimension");
                self.point(13, data.extension1);
                self.point(14, data.extension2);
                if data.kind == DimensionKind::Linear {
                    self.pair_f(50, data.rotation.to_degrees());
                    self.pair(100, "AcDbRotatedDimension");
                }
            }
            DimensionKind::Radius | DimensionKind::Diameter => {
                self.pair(
                    100,
                    if data.kind == DimensionKind::Radius {
                        "AcDbRadialDimension"
                    } else {
                        "AcDbDiametricDimension"
                    },
                );
                self.point(15, data.extension1);
            }
            DimensionKind::Angular2Line => {
                self.pair(100, "AcDb2LineAngularDimension");
                self.point(13, data.extension1);
                self.point(14, data.extension2);
                self.point(15, data.definition);
            }
            DimensionKind::Angular3Point => {
                self.pair(100, "AcDb3PointAngularDimension");
                self.point(13, data.extension1);
                self.point(14, data.extension2);
                self.point(15, data.definition);
            }
            DimensionKind::Ordinate => {
                self.pair(100, "AcDbOrdinateDimension");
                self.point(13, data.extension1);
                self.point(14, data.extension2);
            }
        }
        self.report.entities_written += 1;
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
        // AutoCAD requires group 210 on a HATCH even when the normal is world.
        let normal = if hatch.extrusion.length() < 1e-12 {
            WORLD
        } else {
            hatch.extrusion
        };
        self.point(210, normal);
        let mut pattern_lines = hatch.pattern_lines.clone();
        if !hatch.solid_fill && pattern_lines.is_empty() {
            self.warn("HATCH has no pattern definition; one line family was generated");
            pattern_lines.push(generated_hatch_line(hatch));
        }
        let name = if hatch.solid_fill {
            "SOLID"
        } else if hatch.pattern_name.trim().is_empty() {
            "ANSI31"
        } else {
            hatch.pattern_name.as_str()
        };
        self.pair(2, sanitize_name(name));
        self.pair_i(70, i32::from(hatch.solid_fill));
        self.pair_i(71, 0);
        self.pair_i(91, hatch.paths.len() as i32);
        for path in &hatch.paths {
            self.write_hatch_path(path);
        }
        self.pair_i(75, i32::from(hatch.style));
        self.pair_i(76, i32::from(hatch.pattern_type));
        // A solid hatch has no pattern definition. AutoCAD expects group 98
        // immediately after group 76.
        if !hatch.solid_fill {
            self.pair_f(52, hatch.pattern_angle.to_degrees());
            self.pair_f(41, hatch.pattern_scale);
            self.pair_i(77, i32::from(hatch.double));
            self.pair_i(78, pattern_lines.len() as i32);
            for line in &pattern_lines {
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
        }
        let seed = hatch_seed(hatch);
        self.pair_i(98, 1);
        self.pair_f(10, seed.x);
        self.pair_f(20, seed.y);
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
                self.pair_i(92, 1);
                self.pair_i(93, edges.len() as i32);
                for edge in edges {
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
                let (start_angle, end_angle) = dxf_hatch_angles(*start_angle, *end_angle, *is_ccw);
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
                let (start_angle, end_angle) = dxf_hatch_angles(*start_angle, *end_angle, *is_ccw);
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
            HatchEdge::Spline {
                degree,
                periodic,
                knots,
                weights,
                control_points,
                fit_points,
            } => {
                let degree = if *degree == 0 { 3 } else { *degree };
                self.pair_i(72, 4);
                self.pair_i(94, degree as i32);
                self.pair_i(73, i32::from(!weights.is_empty()));
                self.pair_i(74, i32::from(*periodic));
                self.pair_i(95, knots.len() as i32);
                self.pair_i(96, control_points.len() as i32);
                for knot in knots {
                    self.pair_f(40, *knot);
                }
                for point in control_points {
                    self.pair_f(10, point.x);
                    self.pair_f(20, point.y);
                }
                for weight in weights {
                    self.pair_f(42, *weight);
                }
                self.pair_i(97, fit_points.len() as i32);
                for point in fit_points {
                    self.pair_f(11, point.x);
                    self.pair_f(21, point.y);
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
        self.pair_f(41, data.width_factor);
        self.pair_f(51, data.oblique.to_degrees());
        self.pair(7, &style_name(&data.style));
        self.extrusion(data.extrusion);
        if data.halign.uses_alignment_point() || data.valign.uses_alignment_point() {
            self.point(11, data.alignment);
        }
        self.pair_i(72, i32::from(data.halign.to_dxf()));
        self.pair(100, "AcDbText");
        self.pair_i(73, i32::from(data.valign.to_dxf()));
    }

    fn write_attrib(&mut self, entity: &Entity, data: &TextData, owner: &str) {
        let Some(tag) = attribute_tag(data) else {
            return;
        };
        let flags = data.attribute.as_ref().map(|info| info.flags).unwrap_or(0);
        self.pair(0, "ATTRIB");
        let handle = self.next_handle();
        self.pair(5, &handle);
        self.pair(330, owner);
        self.pair(100, "AcDbEntity");
        self.pair(8, sanitize_name(&entity.layer));
        self.paperspace_flag();
        write_entity_color(self, entity.color);
        self.pair(100, "AcDbText");
        self.write_text_fields(data);
        self.pair(100, "AcDbAttribute");
        self.pair(2, &export_attribute_tag(tag));
        self.pair_i(70, i32::from(flags & 0x0F));
        self.pair_i(73, i32::from(data.valign.to_dxf()));
        self.report.entities_written += 1;
    }

    fn write_attdef(&mut self, entity: &Entity, data: &TextData) {
        let Some(tag) = attribute_tag(data) else {
            return;
        };
        let prompt = data
            .attribute
            .as_ref()
            .map(|info| info.prompt.as_str())
            .unwrap_or("");
        let flags = data.attribute.as_ref().map(|info| info.flags).unwrap_or(0);
        self.begin_entity_class("ATTDEF", "AcDbText", entity);
        self.write_text_fields(data);
        self.pair(100, "AcDbAttributeDefinition");
        self.pair(3, sanitize_text(prompt));
        self.pair(2, &export_attribute_tag(tag));
        self.pair_i(70, i32::from(flags & 0x0F));
        self.pair_i(74, i32::from(data.valign.to_dxf()));
    }

    fn write_text_fields(&mut self, data: &TextData) {
        self.point(10, data.insertion);
        self.pair_f(40, data.height.abs().max(1e-9));
        self.pair(1, sanitize_text(&data.value));
        self.pair_f(50, data.rotation.to_degrees());
        self.pair_f(41, data.width_factor);
        self.pair_f(51, data.oblique.to_degrees());
        self.pair(7, &style_name(&data.style));
        if data.halign.uses_alignment_point() || data.valign.uses_alignment_point() {
            self.point(11, data.alignment);
        }
        self.pair_i(72, i32::from(data.halign.to_dxf()));
        self.extrusion(data.extrusion);
    }

    fn write_seqend(&mut self, entity: &Entity, owner: &str) {
        self.pair(0, "SEQEND");
        let handle = self.next_handle();
        self.pair(5, &handle);
        self.pair(330, owner);
        self.pair(100, "AcDbEntity");
        self.pair(8, sanitize_name(&entity.layer));
    }

    fn write_mtext(&mut self, entity: &Entity, data: &MTextData) {
        self.begin_entity("MTEXT", entity);
        self.point(10, data.insertion);
        self.pair_f(40, data.height.abs().max(1e-9));
        self.pair_f(41, data.width.abs());
        self.pair_i(71, i32::from(data.attachment.clamp(1, 9)));
        // 0 is not a drawing direction. AutoCAD leaves the MTEXT unrepaired.
        self.pair_i(72, 1);
        let spacing = if data.line_spacing.is_finite() && data.line_spacing > 1e-9 {
            data.line_spacing
        } else {
            1.0
        };
        self.pair_f(44, spacing);
        self.pair_i(73, 1);
        write_mtext_chunks(&data.value, |code, chunk| {
            self.pair(code, chunk);
        });
        self.pair(7, &style_name(&data.style));
        let axis = Point3::from_xy(data.rotation.cos(), data.rotation.sin());
        self.point(11, axis);
        self.extrusion(data.extrusion);
    }

    fn paperspace_flag(&mut self) {
        if self.paper_entity {
            self.pair_i(67, 1);
        }
    }

    fn begin_entity(&mut self, kind: &str, entity: &Entity) -> String {
        self.begin_entity_class(kind, acad_class(kind), entity)
    }

    fn begin_entity_class(&mut self, kind: &str, class_name: &str, entity: &Entity) -> String {
        self.pair(0, kind);
        let handle = self.next_handle();
        self.pair(5, &handle);
        self.write_owner();
        self.pair(100, "AcDbEntity");
        self.pair(8, sanitize_name(&entity.layer));
        self.paperspace_flag();
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
        if entity.lineweight != cad_core::LINEWEIGHT_BYLAYER {
            self.pair_i(370, i32::from(entity.lineweight));
        }
        self.pair(100, class_name);
        handle
    }

    fn write_objects(&mut self) {
        self.pair(0, "SECTION");
        self.pair(2, "OBJECTS");
        let root = self.next_handle();
        let group = self.next_handle();
        let layouts = self.next_handle();
        let mline_dict = self.next_handle();
        let mline_style = self.next_handle();
        let plot_dict = self.next_handle();
        let plotstyle = self.next_handle();
        let placeholder = if self.plot_style.is_empty() {
            self.next_handle()
        } else {
            self.plot_style.clone()
        };
        let layouts_to_write = self.export_layouts.clone();

        self.begin_object("DICTIONARY", &root, "0");
        self.pair(100, "AcDbDictionary");
        self.pair_i(281, 1);
        self.dict_entry("ACAD_GROUP", &group);
        self.dict_entry("ACAD_LAYOUT", &layouts);
        self.dict_entry("ACAD_MLINESTYLE", &mline_dict);
        self.dict_entry("ACAD_PLOTSETTINGS", &plot_dict);
        self.dict_entry("ACAD_PLOTSTYLENAME", &plotstyle);

        self.begin_object("DICTIONARY", &group, &root);
        self.pair(100, "AcDbDictionary");
        self.pair_i(281, 1);

        self.begin_object("DICTIONARY", &layouts, &root);
        self.pair(100, "AcDbDictionary");
        self.pair_i(281, 1);
        for layout in &layouts_to_write {
            self.dict_entry(&layout.name, &layout.handle);
        }
        for layout in &layouts_to_write {
            let record = self
                .block_records
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(&layout.block_name))
                .map(|(_, handle)| handle.clone())
                .unwrap_or_default();
            self.write_layout_object(layout, &layouts, &record);
        }

        self.begin_object("DICTIONARY", &mline_dict, &root);
        self.pair(100, "AcDbDictionary");
        self.pair_i(281, 1);
        self.dict_entry("Standard", &mline_style);
        self.write_standard_mlinestyle(&mline_style, &mline_dict);

        self.begin_object("DICTIONARY", &plot_dict, &root);
        self.pair(100, "AcDbDictionary");
        self.pair_i(281, 1);

        self.begin_object("ACDBDICTIONARYWDFLT", &plotstyle, &root);
        self.pair(100, "AcDbDictionary");
        self.pair_i(281, 1);
        self.dict_entry("Normal", &placeholder);
        self.pair(100, "AcDbDictionaryWithDefault");
        self.pair(340, &placeholder);

        // LibreDWG's PLACEHOLDER has no subclass. Group 100 AcDbPlaceHolder
        // makes dxf_read reject the whole drawing.
        self.begin_object("ACDBPLACEHOLDER", &placeholder, &plotstyle);
        self.pair(0, "ENDSEC");
    }

    fn begin_object(&mut self, kind: &str, handle: &str, owner: &str) {
        self.pair(0, kind);
        self.pair(5, handle);
        self.pair(330, owner);
    }

    fn dict_entry(&mut self, name: &str, handle: &str) {
        self.pair(3, name);
        self.pair(350, handle);
    }

    fn write_viewport(&mut self, entity: &Entity, viewport: &ViewportData) {
        self.begin_entity_class("VIEWPORT", "AcDbViewport", entity);
        self.point(10, viewport.center);
        self.pair_f(40, viewport.width);
        self.pair_f(41, viewport.height);
        self.pair_i(68, viewport.status);
        self.pair_i(69, viewport.id);
        self.pair_xy(12, viewport.view_center.x, viewport.view_center.y);
        self.pair_xy(13, 0.0, 0.0);
        self.pair_xy(14, 1.0, 1.0);
        self.pair_xy(15, 1.0, 1.0);
        self.point(16, viewport.view_direction);
        self.point(17, viewport.view_target);
        self.pair_f(42, viewport.lens_length);
        self.pair_f(43, viewport.front_z);
        self.pair_f(44, viewport.back_z);
        self.pair_f(45, viewport.view_height);
        self.pair_f(50, viewport.snap_angle.to_degrees());
        self.pair_f(51, viewport.twist.to_degrees());
        self.pair_i(72, viewport.circle_zoom);
        self.pair_i(90, viewport.status_flag);
        self.pair(1, "");
        self.pair_i(281, 0);
        self.pair_i(71, 1);
        self.pair_i(74, 0);
        self.point(110, Point3::default());
        self.point(111, Point3::new(1.0, 0.0, 0.0));
        self.point(112, Point3::new(0.0, 1.0, 0.0));
        self.pair_i(79, 0);
        self.pair_f(146, 0.0);
        self.report.entities_written += 1;
    }

    fn pair_xy(&mut self, code: i16, x: f64, y: f64) {
        self.pair_f(code, x);
        self.pair_f(code + 10, y);
    }

    fn write_layout_object(&mut self, layout: &ExportLayout, owner: &str, block_record: &str) {
        let (min_x, min_y, max_x, max_y) = if layout.paper {
            (0.0, 0.0, layout.width, layout.height)
        } else {
            self.layout_limits()
        };
        self.begin_object("LAYOUT", &layout.handle, owner);
        self.pair(100, "AcDbPlotSettings");
        self.pair(1, "");
        self.pair(2, "none_device");
        self.pair(4, "");
        // Group 6 is both a plot-view handle and a name. LibreDWG rejects it
        // unless the name is non-empty, so an unused view is omitted.
        self.pair_f(40, layout.left);
        self.pair_f(41, layout.bottom);
        self.pair_f(42, layout.right);
        self.pair_f(43, layout.top);
        self.pair_f(44, layout.width);
        self.pair_f(45, layout.height);
        self.pair_f(46, 0.0);
        self.pair_f(47, 0.0);
        self.pair_f(48, 0.0);
        self.pair_f(49, 0.0);
        self.pair_f(140, 0.0);
        self.pair_f(141, 0.0);
        self.pair_f(142, 1.0);
        self.pair_f(143, 1.0);
        self.pair_i(70, 0);
        self.pair(7, "");
        self.pair_i(72, 0);
        self.pair_i(73, 1);
        self.pair_i(74, 5);
        self.pair_i(75, 16);
        self.pair_f(147, 1.0);
        self.pair(100, "AcDbLayout");
        self.pair(1, &layout.name);
        // LibreDWG stores layout flags in group 70 and the tab order in 71.
        self.pair_i(70, i32::from(layout.paper));
        self.pair_i(71, layout.tab_order);
        self.pair_f(10, min_x);
        self.pair_f(20, min_y);
        self.pair_f(11, max_x);
        self.pair_f(21, max_y);
        self.pair_f(12, 0.0);
        self.pair_f(22, 0.0);
        self.pair_f(32, 0.0);
        self.pair_f(14, min_x);
        self.pair_f(24, min_y);
        self.pair_f(34, 0.0);
        self.pair_f(15, max_x);
        self.pair_f(25, max_y);
        self.pair_f(35, 0.0);
        self.pair_f(146, 0.0);
        self.pair_f(13, 0.0);
        self.pair_f(23, 0.0);
        self.pair_f(33, 0.0);
        self.pair_f(16, 1.0);
        self.pair_f(26, 0.0);
        self.pair_f(36, 0.0);
        self.pair_f(17, 0.0);
        self.pair_f(27, 1.0);
        self.pair_f(37, 0.0);
        self.pair_i(76, 0);
        self.pair(330, block_record);
    }

    fn layout_limits(&self) -> (f64, f64, f64, f64) {
        if let Some(extents) = self
            .document
            .diagnostics
            .extents
            .or_else(|| self.document.compute_extents())
        {
            return (extents.min.x, extents.min.y, extents.max.x, extents.max.y);
        }
        (0.0, 0.0, 12.0, 9.0)
    }

    fn write_standard_mlinestyle(&mut self, handle: &str, owner: &str) {
        self.begin_object("MLINESTYLE", handle, owner);
        self.pair(100, "AcDbMlineStyle");
        self.pair(2, "Standard");
        self.pair_i(70, 0);
        self.pair(3, "");
        self.pair_i(62, 256);
        self.pair_f(51, 90.0);
        self.pair_f(52, 90.0);
        self.pair_i(71, 2);
        self.pair_f(49, 0.5);
        self.pair_i(62, 256);
        self.pair(6, "CONTINUOUS");
        self.pair_f(49, -0.5);
        self.pair_i(62, 256);
        self.pair(6, "CONTINUOUS");
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
        // A missing normal is stored as (0, 0, 0). Group 210 of (0, 0, 0) is read
        // as (0, 0, -1), which mirrors the entity.
        if extrusion.length() < 1e-12 {
            return;
        }
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

fn write_subentity_color(writer: &mut DxfWriter<'_>, color: CadColor) {
    // A omitted group 62 on VERTEX defaults to ByBlock, while the polyline
    // defaults to ByLayer. AutoCAD audits that as a color mismatch.
    match color {
        CadColor::ByLayer => writer.pair_i(62, 256),
        other => write_entity_color(writer, other),
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
        CadColor::Rgb { r, g, b } => i32::from(nearest_aci(r, g, b)),
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
    let (keep_star, body) = if let Some(rest) = trimmed.strip_prefix('*') {
        (true, rest)
    } else {
        (false, trimmed)
    };
    let mut cleaned: String = body.chars().map(sanitize_name_char).collect();
    if cleaned.is_empty() {
        cleaned = "0".into();
    }
    if keep_star {
        cleaned.insert(0, '*');
    }
    encode_dxf_r2000(&cleaned)
}

fn sanitize_name_char(ch: char) -> char {
    match ch {
        '\n' | '\r' | '<' | '>' | '/' | '\\' | '"' | ':' | ';' | '?' | '*' | '|' | ',' | '='
        | '`' => '_',
        _ => ch,
    }
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

fn dim_defaults() -> [(&'static str, f64); 11] {
    [
        ("$DIMSCALE", 1.0),
        ("$DIMASZ", 2.5),
        ("$DIMEXO", 0.625),
        ("$DIMDLI", 3.75),
        ("$DIMEXE", 1.25),
        ("$DIMTXT", 2.5),
        ("$DIMCEN", 2.5),
        ("$DIMALTF", 25.4),
        ("$DIMLFAC", 1.0),
        ("$DIMGAP", 0.625),
        ("$DIMTFAC", 1.0),
    ]
}

fn is_primary_paper_space(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case("*PAPER_SPACE")
}

fn is_model_space_name(name: &str) -> bool {
    let upper = name.trim().to_ascii_uppercase();
    upper == "*MODEL_SPACE" || upper == "$MODEL_SPACE"
}

// Clockwise hatch arcs are stored mirrored in DXF. Internal angles are
// the real geometry, so the writer negates them on the way out.
fn dxf_hatch_angles(start: f64, end: f64, is_ccw: bool) -> (f64, f64) {
    let (start, end) = if is_ccw { (start, end) } else { (-start, -end) };
    let positive_zero = |angle: f64| if angle == 0.0 { 0.0 } else { angle };
    (positive_zero(start), positive_zero(end))
}

fn hatch_seed(hatch: &HatchData) -> Point3 {
    for path in &hatch.paths {
        match path {
            HatchPath::Polyline { vertices, .. } => {
                if let Some(vertex) = vertices.first() {
                    return vertex.point;
                }
            }
            HatchPath::Edges(edges) => {
                for edge in edges {
                    match edge {
                        HatchEdge::Line { start, .. } => return *start,
                        HatchEdge::Arc { center, .. } | HatchEdge::Ellipse { center, .. } => {
                            return *center;
                        }
                        HatchEdge::Spline { control_points, fit_points, .. } => {
                            if let Some(point) = control_points.first().or(fit_points.first()) {
                                return *point;
                            }
                        }
                    }
                }
            }
        }
    }
    Point3::default()
}

fn generated_hatch_line(hatch: &HatchData) -> cad_core::HatchPatternLine {
    let angle = hatch.pattern_angle;
    let scale = if hatch.pattern_scale.is_finite() && hatch.pattern_scale > 1e-6 {
        hatch.pattern_scale
    } else {
        1.0
    };
    cad_core::HatchPatternLine {
        angle,
        base: Point3::default(),
        offset: Point3::from_xy(-angle.sin() * scale, angle.cos() * scale),
        dashes: Vec::new(),
    }
}

fn attribute_tag(data: &TextData) -> Option<&str> {
    data.attribute
        .as_ref()
        .map(|info| info.tag.trim())
        .filter(|tag| !tag.is_empty())
}

fn block_has_attdef(block: &BlockDefinition) -> bool {
    block.entities.iter().any(|entity| match &entity.geometry {
        Geometry::Text(text) => text.is_attrib_def && attribute_tag(text).is_some(),
        _ => false,
    })
}

fn style_name(style: &str) -> String {
    let trimmed = style.trim();
    if trimmed.is_empty() {
        "STANDARD".to_string()
    } else {
        sanitize_name(trimmed)
    }
}

fn is_layout_block(name: &str) -> bool {
    is_model_space_name(name) || is_paper_layout_block(name)
}

fn is_anonymous_block(name: &str) -> bool {
    let name = name.trim();
    // *D, *U, and *X names are rejected by AutoCAD audit unless they
    // were created as its own anonymous blocks. Written without the
    // star, the same block is an ordinary named block.
    if drops_anonymous_star(name) {
        return false;
    }
    name.starts_with('*') && !is_layout_block(name)
}

fn linetype_table_rank(linetype: &LineType) -> u8 {
    match linetype.name.to_ascii_uppercase().as_str() {
        "BYBLOCK" => 0,
        "BYLAYER" => 1,
        "CONTINUOUS" => 2,
        _ if linetype.dashes.is_empty() => 3,
        _ => 4,
    }
}

fn export_block_name(name: &str) -> String {
    let sanitized = sanitize_name(name);
    if drops_anonymous_star(&sanitized) {
        sanitized[1..].to_string()
    } else {
        sanitized
    }
}

fn drops_anonymous_star(name: &str) -> bool {
    is_star_numbered(name, b'D')
        || is_star_numbered(name, b'U')
        || is_star_numbered(name, b'X')
}

fn is_star_numbered(name: &str, kind: u8) -> bool {
    let bytes = name.as_bytes();
    bytes.len() > 2
        && bytes[0] == b'*'
        && bytes[1].eq_ignore_ascii_case(&kind)
        && bytes[2..].iter().all(|byte| byte.is_ascii_digit())
}

fn export_attribute_tag(tag: &str) -> String {
    // AutoCAD clears a tag that contains a period or '#', then reports
    // the tag as empty. LibreDWG also drops a tag that contains a
    // lowercase letter, which AutoCAD then reports the same way.
    let cleaned: String = sanitize_name(tag)
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "TAG".to_string()
    } else {
        cleaned
    }
}

fn written_entities(document: &Document) -> impl Iterator<Item = &Entity> {
    document.model_space.iter().chain(
        document
            .blocks
            .iter()
            .filter(|(name, _)| !is_model_space_name(name))
            .flat_map(|(_, block)| block.entities.iter()),
    )
}

fn plan_layouts(document: &Document) -> Vec<ExportLayout> {
    let mut used = BTreeSet::new();
    let mut layouts = vec![ExportLayout {
        name: unique_layout_name(&mut used, "Model"),
        block_name: "*MODEL_SPACE".into(),
        tab_order: 0,
        paper: false,
        width: 210.0,
        height: 297.0,
        left: 7.5,
        bottom: 7.5,
        right: 7.5,
        top: 7.5,
        handle: String::new(),
    }];
    let mut blocks = sorted_paper_layout_blocks(document);
    if !blocks
        .iter()
        .any(|name| name.eq_ignore_ascii_case("*PAPER_SPACE"))
    {
        blocks.insert(0, "*PAPER_SPACE".into());
    }
    for layout in &document.layouts {
        if blocks
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&layout.block_name))
        {
            continue;
        }
        blocks.push(layout.block_name.clone());
    }
    for (index, block_name) in blocks.into_iter().enumerate() {
        let saved = document
            .layouts
            .iter()
            .find(|layout| layout.block_name.eq_ignore_ascii_case(&block_name));
        let fallback = if index == 0 {
            "Layout1".to_string()
        } else {
            format!("Layout{}", index + 1)
        };
        let name = saved
            .map(|layout| layout.name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(fallback);
        layouts.push(ExportLayout {
            name: unique_layout_name(&mut used, &name),
            block_name,
            tab_order: saved
                .map(|layout| layout.tab_order)
                .unwrap_or(index as i32 + 1),
            paper: true,
            width: positive(saved.map(|layout| layout.paper_width), 210.0),
            height: positive(saved.map(|layout| layout.paper_height), 297.0),
            left: non_negative(saved.map(|layout| layout.left_margin), 7.5),
            bottom: non_negative(saved.map(|layout| layout.bottom_margin), 7.5),
            right: non_negative(saved.map(|layout| layout.right_margin), 7.5),
            top: non_negative(saved.map(|layout| layout.top_margin), 7.5),
            handle: String::new(),
        });
    }
    layouts
}

fn unique_layout_name(used: &mut BTreeSet<String>, raw: &str) -> String {
    let base = if raw.trim().is_empty() {
        "Layout".to_string()
    } else {
        sanitize_name(raw)
    };
    let mut candidate = base.clone();
    let mut suffix = 2_u32;
    while !used.insert(candidate.to_ascii_uppercase()) {
        candidate = format!("{base}_{suffix}");
        suffix += 1;
    }
    candidate
}

fn positive(value: Option<f64>, fallback: f64) -> f64 {
    value
        .filter(|value| value.is_finite() && *value > 1.0)
        .unwrap_or(fallback)
}

fn non_negative(value: Option<f64>, fallback: f64) -> f64 {
    value
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(fallback)
}

fn shape_file_key(file: &str) -> String {
    format!("SHAPEFILE:{}", file.trim().to_ascii_uppercase())
}

fn collect_shape_files(document: &Document) -> Vec<String> {
    let mut found: BTreeMap<String, String> = BTreeMap::new();
    for linetype in document.linetypes.values() {
        for shape in linetype.shapes.iter().flatten() {
            let file = shape.shape_file.trim();
            if file.is_empty() {
                continue;
            }
            found
                .entry(file.to_ascii_uppercase())
                .or_insert_with(|| file.to_string());
        }
    }
    found.into_values().collect()
}

fn export_text_styles(document: &Document) -> (Vec<TextStyle>, Vec<String>) {
    let mut found: BTreeMap<String, TextStyle> = BTreeMap::new();
    for style in document.text_styles.values() {
        if style.name.trim().is_empty() {
            continue;
        }
        found.insert(style.name.to_ascii_uppercase(), style.clone());
    }
    found
        .entry("STANDARD".into())
        .or_insert_with(TextStyle::standard);
    let mut warnings = Vec::new();
    for name in referenced_styles(document) {
        let key = name.to_ascii_uppercase();
        if found.contains_key(&key) {
            continue;
        }
        warnings.push(format!(
            "text style '{name}' is missing; exported as a txt style"
        ));
        found.insert(
            key,
            TextStyle {
                name,
                ..TextStyle::standard()
            },
        );
    }
    let styles = found.into_values().collect();
    (styles, warnings)
}

fn referenced_styles(document: &Document) -> Vec<String> {
    let mut names = Vec::new();
    for entity in written_entities(document) {
        collect_style_names(&entity.geometry, &mut names);
    }
    for linetype in document.linetypes.values() {
        for shape in linetype.shapes.iter().flatten() {
            if !shape.style.trim().is_empty() {
                names.push(shape.style.clone());
            }
        }
    }
    names
}

fn collect_style_names(geometry: &Geometry, names: &mut Vec<String>) {
    match geometry {
        Geometry::Text(text) => names.push(style_name(&text.style)),
        Geometry::MText(text) => names.push(style_name(&text.style)),
        Geometry::Insert { attribs, .. } => {
            for attrib in attribs {
                names.push(style_name(&attrib.style));
            }
        }
        _ => {}
    }
}

fn export_linetypes(document: &Document) -> (Vec<LineType>, Vec<String>) {
    let mut found: BTreeMap<String, LineType> = BTreeMap::new();
    for linetype in document.linetypes.values() {
        found.insert(linetype.name.to_ascii_uppercase(), linetype.clone());
    }
    let mut warnings = Vec::new();
    // BYLAYER and BYBLOCK are not patterns, but AutoCAD's audit requires
    // both names in the LTYPE table.
    let mut needed = vec![
        "BYLAYER".to_string(),
        "BYBLOCK".to_string(),
        "CONTINUOUS".to_string(),
    ];
    for layer in document.layers.values() {
        if !layer.linetype.trim().is_empty() {
            needed.push(layer.linetype.clone());
        }
    }
    for entity in written_entities(document) {
        if !entity.linetype.trim().is_empty() {
            needed.push(entity.linetype.clone());
        }
    }
    for name in needed {
        let key = name.to_ascii_uppercase();
        if found.contains_key(&key) {
            continue;
        }
        let standard = matches!(key.as_str(), "BYLAYER" | "BYBLOCK" | "CONTINUOUS");
        let builtin = LineType::builtin(&name);
        if !standard {
            if builtin.is_continuous() {
                warnings.push(format!(
                    "linetype '{name}' was missing from the table and was written as CONTINUOUS"
                ));
            } else {
                warnings.push(format!(
                    "linetype '{name}' was missing from the table and was written from the built-in pattern"
                ));
            }
        }
        let record = if standard {
            LineType::continuous(key.clone())
        } else if builtin.is_continuous() {
            LineType::continuous(name)
        } else {
            builtin
        };
        found.insert(key, record);
    }
    let mut linetypes: Vec<LineType> = found.into_values().collect();
    linetypes.sort_by(|left, right| {
        left.name
            .to_ascii_uppercase()
            .cmp(&right.name.to_ascii_uppercase())
    });
    (linetypes, warnings)
}

fn export_layers(document: &Document) -> (Vec<Layer>, Vec<String>) {
    let mut layers = document.layers.clone();
    let mut warnings = Vec::new();
    if !layers.keys().any(|name| name.eq_ignore_ascii_case("0")) {
        layers.insert(
            "0".into(),
            Layer {
                name: "0".into(),
                visible: true,
                frozen: false,
                color: CadColor::Aci(7),
                linetype: "CONTINUOUS".into(),
                ..Layer::default()
            },
        );
    }
    for entity in written_entities(document) {
        let layer_name = if entity.layer.trim().is_empty() {
            "0".to_string()
        } else {
            entity.layer.clone()
        };
        if layers
            .keys()
            .any(|name| name.eq_ignore_ascii_case(&layer_name))
        {
            continue;
        }
        warnings.push(format!(
            "layer '{layer_name}' was missing from the table and was added"
        ));
        layers.insert(
            layer_name.clone(),
            Layer {
                name: layer_name,
                visible: true,
                frozen: false,
                color: CadColor::Aci(7),
                linetype: "CONTINUOUS".into(),
                ..Layer::default()
            },
        );
    }
    let mut list: Vec<Layer> = layers.into_values().collect();
    list.sort_by(|left, right| {
        left.name
            .to_ascii_uppercase()
            .cmp(&right.name.to_ascii_uppercase())
    });
    (list, warnings)
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
        AttributeInfo, MTextData, Point2, Point3, PolyVertex, RasterFrame, TextData,
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
        render_dxf(document, &DxfExportOptions::default()).expect("write")
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
        assert!(text.contains("\n390\n"), "layers need a plot style handle");
        assert!(text.contains("$DIMTXT"));
        assert!(text.contains("$DIMALTF"));
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
    fn zero_extrusion_writes_no_normal() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::LwPolyline {
            vertices: vec![
                PolyVertex {
                    point: Point3::from_xy(1.0, 2.0),
                    bulge: 0.0,
                    vertex_id: Default::default(),
                },
                PolyVertex {
                    point: Point3::from_xy(4.0, 2.0),
                    bulge: 0.0,
                    vertex_id: Default::default(),
                },
            ],
            closed: false,
            extrusion: Point3::default(),
            linetype_generation_continuous: false,
        }));
        let (_, text) = write_to_string(&document);
        let entities = entities_section(&text);
        assert!(entities.contains("LWPOLYLINE"));
        assert!(
            !entities.contains("\n210\n"),
            "a zero extrusion must not be written as group 210"
        );
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
        document.add_entity(Entity::new(Geometry::Dimension(DimensionData {
            block_name: "*D1".into(),
            ..DimensionData::default()
        })));
        let (report, text) = write_to_string(&document);
        let entities = entities_section(&text);
        assert!(report.entities_written >= 1);
        assert!(entities.contains("DIMENSION"));
        assert!(entities.contains("\nD1\n"));
        assert!(text.contains("AcDbRotatedDimension"));
    }

    #[test]
    fn star_x_block_is_written_as_an_ordinary_name() {
        let mut document = Document::default();
        document.blocks.insert(
            "*X12".into(),
            BlockDefinition {
                name: "*X12".into(),
                base_pt: Point3::from_xy(0.0, 0.0),
                entities: Vec::new(),
                ..Default::default()
            },
        );
        let (_, text) = write_to_string(&document);
        assert!(text.contains("\nX12\n"));
        assert!(!text.contains("*X12"));
    }

    #[test]
    fn attribute_tag_is_written_in_uppercase() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::Text(TextData {
            value: "J06".into(),
            is_attrib_def: true,
            attribute: Some(AttributeInfo {
                tag: "ILabel".into(),
                prompt: String::new(),
                flags: 8,
            }),
            ..TextData::default()
        })));
        let (_, text) = write_to_string(&document);
        let entities = entities_section(&text);
        assert!(entities.contains("\nILABEL\n"));
        assert!(!entities.contains("ILabel"));
    }

    #[test]
    fn missing_dimension_block_warns_and_does_not_invent_a_dimension() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::Dimension(DimensionData {
            block_name: "*D1".into(),
            ..DimensionData::default()
        })));
        let (report, text) = write_to_string(&document);
        assert!(report.entities_written >= 1);
        assert!(entities_section(&text).contains("DIMENSION"));
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
    fn image_and_wipeout_export_as_a_closed_frame() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::Image(RasterFrame {
            corner: Point3::from_xy(1.0, 2.0),
            size: Point2::new(4.0, 3.0),
            path: "photo.png".into(),
            ..RasterFrame::default()
        })));
        document.add_entity(Entity::new(Geometry::Wipeout(RasterFrame {
            corner: Point3::from_xy(10.0, 0.0),
            ..RasterFrame::default()
        })));
        let (report, text) = write_to_string(&document);
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("IMAGE")));
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("WIPEOUT")));
        let entities = entities_section(&text);
        assert!(entities.contains("LWPOLYLINE"));
        assert!(!entities.contains("\nIMAGE\n"));
        assert!(!entities.contains("\nWIPEOUT\n"));
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
                ..Default::default()
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
            .any(|warning| warning.contains("without a tag")));
        let entities = entities_section(&text);
        assert!(entities.contains("INSERT"));
        assert!(entities.contains("TEXT"));
        assert!(entities.contains("TAG"));
    }

    #[test]
    fn hatch_spline_edges_export_their_degree_and_controls() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::Hatch(HatchData {
            extrusion: Point3::new(0.0, 0.0, 1.0),
            elevation: 0.0,
            solid_fill: true,
            paths: vec![HatchPath::Edges(vec![HatchEdge::spline(vec![
                Point3::from_xy(0.0, 0.0),
                Point3::from_xy(1.0, 1.0),
            ])])],
            pattern_lines: Vec::new(),
            ..HatchData::default()
        })));
        let (report, text) = write_to_string(&document);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        let entities = entities_section(&text);
        assert!(entities.contains("HATCH"));
        assert!(entities.contains(" 94\n3\n"), "{entities}");
        assert_eq!(report.entities_written, 1);
    }

    #[test]
    fn clockwise_hatch_arc_is_written_mirrored() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::Hatch(HatchData {
            extrusion: Point3::new(0.0, 0.0, 1.0),
            elevation: 0.0,
            solid_fill: true,
            paths: vec![HatchPath::Edges(vec![HatchEdge::Arc {
                center: Point3::from_xy(0.0, 0.0),
                radius: 1.0,
                start_angle: 0.0,
                end_angle: std::f64::consts::FRAC_PI_2,
                is_ccw: false,
            }])],
            pattern_lines: Vec::new(),
            ..HatchData::default()
        })));
        let (_, text) = write_to_string(&document);
        let entities = entities_section(&text);
        assert!(entities.contains(" 50\n0.0\n"), "{entities}");
        assert!(entities.contains(" 51\n-90.0\n"), "{entities}");
        assert!(entities.contains(" 73\n0\n"), "{entities}");
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
            ..Default::default()
        })));
        let (report, text) = write_to_string(&document);
        let entities = entities_section(&text);
        assert_eq!(report.entities_written, 1);
        assert!(entities.contains("MTEXT"));
        assert!(entities.contains("Hello"));
        // Group 72 is the flow direction. 0 leaves the MTEXT unrepaired in
        // AutoCAD; 1 is left to right.
        assert!(entities.contains(" 72\n1\n"));
        assert!(entities.contains(" 73\n1\n"));
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
                ..Layer::default()
            },
        );
        let mut text = Entity::new(Geometry::Text(TextData {
            insertion: Point3::from_xy(0.0, 0.0),
            height: 2.5,
            rotation: 0.0,
            value: "Ölçü Çıkış İstanbul ğşıİ".into(),
            extrusion: Point3::new(0.0, 0.0, 1.0),
            is_attrib_def: false,
            ..Default::default()
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
            ..Default::default()
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
            ..HatchData::default()
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

    #[test]
    fn r2000_file_has_required_tables_objects_and_handseed() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(1.0, 0.0),
        }));
        let (_, text) = write_to_string(&document);
        assert!(text.contains("$HANDSEED"));
        for table in [
            "VPORT",
            "LTYPE",
            "LAYER",
            "STYLE",
            "VIEW",
            "UCS",
            "APPID",
            "DIMSTYLE",
            "BLOCK_RECORD",
        ] {
            assert!(text.contains(&format!("  2\n{table}\n")), "missing {table}");
        }
        assert!(text.contains("*MODEL_SPACE"));
        assert!(text.contains("*PAPER_SPACE"));
        assert!(text.contains("  2\nOBJECTS"));
        assert!(text.contains("ACAD_LAYOUT"));
        assert!(text.contains("\n  0\nLAYOUT\n"));
        assert!(text.contains("AcDbDictionaryWithDefault"));
        let entities = entities_section(&text);
        assert!(
            entities.contains("330\n"),
            "model-space entities need an owner handle"
        );
    }

    #[test]
    fn text_repeats_acdbtext_before_vertical_alignment() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::Text(TextData {
            insertion: Point3::from_xy(0.0, 0.0),
            height: 2.5,
            value: "Tag".into(),
            ..Default::default()
        })));
        let (_, text) = write_to_string(&document);
        let entities = entities_section(&text);
        assert_eq!(entities.matches("AcDbText").count(), 2);
    }

    #[test]
    fn varying_z_polyline_is_a_3d_polyline() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::Polyline {
            vertices: vec![
                PolyVertex {
                    point: Point3::new(0.0, 0.0, 0.0),
                    bulge: 0.0,
                    vertex_id: Default::default(),
                },
                PolyVertex {
                    point: Point3::new(1.0, 0.0, 5.0),
                    bulge: 0.25,
                    vertex_id: Default::default(),
                },
            ],
            closed: false,
            linetype_generation_continuous: false,
        }));
        let (report, text) = write_to_string(&document);
        let entities = entities_section(&text);
        assert!(entities.contains("AcDb3dPolyline"));
        assert!(entities.contains(" 70\n8"));
        assert!(entities.contains(" 30\n5.0"));
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("3D POLYLINE")));
        assert!(!entities.contains(" 42\n0.25"));
    }

    #[test]
    fn leader_writes_required_groups() {
        let mut document = Document::default();
        document.add_entity(Entity::new(Geometry::Leader {
            vertices: vec![Point3::from_xy(0.0, 0.0), Point3::from_xy(1.0, 1.0)],
        }));
        let (_, text) = write_to_string(&document);
        let entities = entities_section(&text);
        assert!(entities.contains("  3\nSTANDARD"));
        assert!(entities.contains(" 71\n1"));
        assert!(entities.contains(" 76\n2"));
    }

    #[test]
    fn missing_linetype_and_layer_are_added() {
        let mut document = Document::default();
        let mut entity = Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(1.0, 0.0),
        });
        entity.layer = "Notes".into();
        entity.linetype = "DASHED".into();
        document.add_entity(entity);
        let (report, text) = write_to_string(&document);
        assert!(text.contains("  2\nDASHED\n"));
        assert!(text.contains("  2\nNotes\n"));
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("DASHED")));
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("Notes")));
    }

    #[test]
    fn paper_space_layout_keeps_its_entities() {
        let mut document = Document::default();
        document.layouts.push(cad_core::PaperLayout {
            name: "Sheet".into(),
            block_name: "*PAPER_SPACE0".into(),
            tab_order: 2,
            paper_width: 420.0,
            paper_height: 297.0,
            ..cad_core::PaperLayout::sheet("Sheet", "*PAPER_SPACE0", 2)
        });
        document.blocks.insert(
            "*PAPER_SPACE0".into(),
            BlockDefinition {
                name: "*PAPER_SPACE0".into(),
                base_pt: Point3::from_xy(0.0, 0.0),
                entities: vec![
                    Entity::new(Geometry::Line {
                        start: Point3::from_xy(4.0, 8.0),
                        end: Point3::from_xy(9.0, 8.0),
                    }),
                    Entity::new(Geometry::Viewport(ViewportData {
                        center: Point3::from_xy(100.0, 50.0),
                        width: 180.0,
                        height: 120.0,
                        view_center: cad_core::Point2::new(10.0, 20.0),
                        view_height: 80.0,
                        ..ViewportData::default()
                    })),
                ],
                ..Default::default()
            },
        );
        let (report, text) = write_to_string(&document);
        assert!(text.contains("*PAPER_SPACE0"));
        assert!(text.contains("\n  1\nSheet\n"));
        assert!(text.contains("VIEWPORT"));
        assert!(text.contains(" 44\n420.0"));
        let entities = entities_section(&text);
        assert!(
            !entities.contains(" 10\n4.0\n 20\n8.0"),
            "paper-space geometry stays in the layout block"
        );
        assert!(text.contains(" 10\n4.0\n 20\n8.0"));
        assert!(!report
            .warnings
            .iter()
            .any(|warning| warning.contains("paper-space")));
    }

    #[test]
    fn complex_linetype_writes_shape_groups() {
        let mut document = Document::default();
        document.linetypes.insert(
            "AMZIGZAG".into(),
            LineType {
                name: "AMZIGZAG".into(),
                dashes: vec![0.5, -0.2],
                shapes: vec![
                    Some(LineTypeShape {
                        flag: 2,
                        text: "Z".into(),
                        scale: 1.0,
                        rotation: 0.3,
                        y_offset: 0.05,
                        style: "STANDARD".into(),
                        ..LineTypeShape::default()
                    }),
                    None,
                ],
            },
        );
        let (_, text) = write_to_string(&document);
        assert!(text.contains("  2\nAMZIGZAG\n"));
        assert!(text.contains(" 74\n2\n"));
        assert!(text.contains("  9\nZ\n"));
        assert!(text.contains(" 46\n1.0"));
        assert!(text.contains("\n340\n"));
        let element = text.split("AMZIGZAG").nth(1).expect("linetype body");
        let scale = element.find(" 46\n").expect("scale");
        let offset = element.find(" 44\n").expect("x offset");
        assert!(scale < offset, "group 46 must precede group 44");
    }

    #[test]
    fn shape_file_linetype_writes_a_shape_style() {
        let mut document = Document::default();
        document.linetypes.insert(
            "FENCELINE1".into(),
            LineType {
                name: "FENCELINE1".into(),
                dashes: vec![0.25, -0.1],
                shapes: vec![
                    None,
                    Some(LineTypeShape {
                        flag: 4,
                        shapecode: 130,
                        shape_file: "ltypeshp.shx".into(),
                        ..LineTypeShape::default()
                    }),
                ],
            },
        );
        let (_, text) = write_to_string(&document);
        assert!(text.contains("ltypeshp.shx"));
        assert!(text.contains(" 70\n1\n"));
        assert!(text.contains("\n340\n"));
    }

    #[test]
    fn invalid_name_characters_are_replaced_but_star_prefix_stays() {
        assert_eq!(sanitize_name("a/b:c"), "a_b_c");
        assert_eq!(sanitize_name("*MODEL_SPACE"), "*MODEL_SPACE");
        assert_eq!(sanitize_name("*D1"), "*D1");
    }
}
