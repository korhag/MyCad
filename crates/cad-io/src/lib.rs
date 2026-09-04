//! Native save/export for cad-core documents.
//!
//! DXF and PDF are written here from the native document model.
//! DWG conversion stays in `dwg-import` so every LibreDWG call shares
//! that crate's existing process-wide mutex.

mod atomic;
mod dxf;
mod error;
mod options;
mod pdf;

pub use atomic::{
    replace_atomic, sibling_temp_in_dir, sibling_temp_path, sibling_temp_with_extension,
};
pub use dxf::write_dxf;
pub use error::ExportError;
pub use options::{
    mm_to_pt, CadFileFormat, DxfAcadVersion, DxfExportOptions, PdfExportOptions, PdfOrientation,
    PdfPaperSize, PdfPlotStyle, SaveReport, PDF_MARGIN_MM,
};
pub use pdf::export_pdf;
