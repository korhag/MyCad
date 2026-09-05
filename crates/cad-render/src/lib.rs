//! CAD viewport rendering: tessellation (CPU) plus wgpu (GPU).

pub mod dash;
pub mod gpu;
pub mod pick;
pub mod tessellate;

pub use cad_core::{curves, stroke_font};
pub use gpu::{plan_gpu_upload, CadFrame, CadGpu, GpuUpload, GpuUploadPlan};
pub use pick::{
    box_select, box_select_into, hit_test, stroke_edges, EntityPick, PickKind, PickPrimitive,
    SelectBoxMode, SpatialIndex, DEFAULT_PICK_TOLERANCE_PX,
};
pub use tessellate::{
    merge_vertex_ranges, overlay_batches, tessellate_document, tessellate_document_for_block_edit,
    AppendedGeometry, BlockEditView, BlockEditViewFrame, DisplayList, EntityDrawRange, GpuVertex,
    OverlayBatches,
};

#[cfg(test)]
mod tests;
