//! CAD viewport rendering: tessellation (CPU) plus wgpu (GPU).

pub mod curves;
pub mod gpu;
pub mod stroke_font;
pub mod tessellate;

pub use gpu::{CadFrame, CadGpu};
pub use tessellate::{tessellate_document, DisplayList, GpuVertex};

#[cfg(test)]
mod tests;
