//! CAD viewport rendering: tessellation (CPU) plus wgpu (GPU).

/// Default sample count for the offscreen viewport target. The egui
/// window stays at one sample; this value is only the starting choice.
pub const VIEWPORT_MSAA_SAMPLES: u32 = 4;

pub mod dash;
pub mod gpu;
pub mod pick;
pub mod tessellate;
mod triangulate;

pub use cad_core::{curves, stroke_font};
pub use gpu::{
    motion_samples, picture_texture_size, plan_gpu_upload, sanitize_viewport_samples,
    scene_needs_redraw, upload_for_ranges, viewport_pixel_size, CadFrame, CadGpu, GpuUpload,
    GpuUploadPlan, SceneKey,
};
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
