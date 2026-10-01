//! CAD viewport rendering: tessellation (CPU) plus wgpu (GPU).

/// Sample count for the viewport color target. Pipelines and the eframe
/// window must agree, or wgpu rejects the render pass.
pub const VIEWPORT_MSAA_SAMPLES: u32 = 4;

pub mod dash;
pub mod gpu;
pub mod pick;
pub mod tessellate;
mod triangulate;

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
