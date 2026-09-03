//! CAD viewport rendering: tessellation (CPU) plus wgpu (GPU).

pub mod curves;
pub mod dash;
pub mod gpu;
pub mod pick;
pub mod stroke_font;
pub mod tessellate;

pub use gpu::{CadFrame, CadGpu};
pub use pick::{
    box_select, box_select_into, hit_test, stroke_edges, EntityPick, PickKind, PickPrimitive,
    SelectBoxMode, SpatialIndex, DEFAULT_PICK_TOLERANCE_PX,
};
pub use tessellate::{
    merge_vertex_ranges, overlay_batches, tessellate_document, DisplayList, EntityDrawRange,
    GpuVertex, OverlayBatches,
};

#[cfg(test)]
mod tests;
