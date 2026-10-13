//! CAD viewport rendering: tessellation (CPU) plus wgpu (GPU).

/// Default sample count for the offscreen viewport target. The egui
/// window stays at one sample; this value is only the starting choice.
pub const VIEWPORT_MSAA_SAMPLES: u32 = 4;

/// AutoCAD Dark theme, 2D model space, uniform background (RGB 33, 40, 48).
pub const VIEWPORT_CLEAR_RGB: [u8; 3] = [33, 40, 48];

/// Model-space clear color. Wipeouts fill with this so they cover what is behind them.
pub const VIEWPORT_CLEAR: [f32; 4] = [
    VIEWPORT_CLEAR_RGB[0] as f32 / 255.0,
    VIEWPORT_CLEAR_RGB[1] as f32 / 255.0,
    VIEWPORT_CLEAR_RGB[2] as f32 / 255.0,
    1.0,
];

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
    tessellate_document_for_block_edit_on, tessellate_document_on, AppendedGeometry, BlockEditView,
    BlockEditViewFrame, DisplayList, EntityDrawRange, GpuVertex, OverlayBatches, WeightVertex,
};

#[cfg(test)]
mod tests;
