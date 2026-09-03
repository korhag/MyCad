//! CAD viewport rendering: tessellation (CPU) plus wgpu (GPU).

pub mod curves;
pub mod dash;
pub mod gpu;
pub mod pick;
pub mod stroke_font;
pub mod tessellate;

pub use gpu::{CadFrame, CadGpu};
pub use pick::{
    box_select, hit_test, stroke_edges, EntityPick, PickKind, PickPrimitive, SelectBoxMode,
    DEFAULT_PICK_TOLERANCE_PX,
};
pub use tessellate::{tessellate_document, DisplayList, GpuVertex};

#[cfg(test)]
mod tests;
