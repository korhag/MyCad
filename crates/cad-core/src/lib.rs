//! Native CAD document model used by import, viewport and rendering.
//!
//! This crate must stay independent of LibreDWG so future DXF import and
//! editing can share the same types.

pub mod color;
pub mod document;
pub mod entity;
pub mod extents;
pub mod geom;
pub mod linetype;
pub mod transform;

pub use color::{aci_rgb, CadColor, Rgb};
pub use document::{BlockDefinition, Document, ImportDiagnostics, Layer};
pub use entity::{
    default_extrusion, Entity, Geometry, HatchData, HatchEdge, HatchPath, HatchPatternLine,
    MTextData, PolyVertex, TextData,
};
pub use extents::Extents2;
pub use geom::{ocs_to_wcs, Point2, Point3};
pub use linetype::LineType;
pub use transform::Transform2;
