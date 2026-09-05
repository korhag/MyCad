//! Native save/export for cad-core documents.
//!
//! DXF and PDF are written here from the native document model.
//! DWG conversion stays in `dwg-import` so every LibreDWG call shares
//! that crate's existing process-wide mutex.

mod atomic;
mod dxf;
mod error;
mod native;
mod options;
mod pdf;
mod r2000;

pub use atomic::{
    replace_atomic, sibling_temp_in_dir, sibling_temp_path, sibling_temp_with_extension,
};
pub use dxf::write_dxf;
pub use error::ExportError;
pub use native::{
    import_block_asset, parse_mycad_bytes, read_mycad, read_mycadblock, write_mycad,
    write_mycadblock, BlockAsset, MYCAD_BLOCK_FORMAT, MYCAD_FORMAT, MYCAD_SCHEMA,
};
pub use options::{
    mm_to_pt, CadFileFormat, DxfAcadVersion, DxfExportOptions, PdfExportOptions, PdfOrientation,
    PdfPaperSize, PdfPlotArea, PdfPlotStyle, SaveReport, PDF_MARGIN_MM, PDF_STROKE_HEAVY_PT,
    PDF_STROKE_NORMAL_PT, PDF_STROKE_THIN_PT, PDF_STROKE_WEIGHTS,
};
pub use pdf::export_pdf;
pub use r2000::{decode_dxf_r2000, encode_dxf_r2000};
