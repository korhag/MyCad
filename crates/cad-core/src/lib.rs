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
pub mod measure;
pub mod snap;
pub mod transform;

pub use color::{aci_rgb, CadColor, Rgb};
pub use document::{BlockDefinition, Document, DrawingUnits, ImportDiagnostics, Layer};
pub use entity::{
    default_extrusion, Entity, EntityId, Geometry, HatchData, HatchEdge, HatchPath,
    HatchPatternLine, MTextData, PolyVertex, TextData,
};
pub use extents::Extents2;
pub use geom::{ocs_to_wcs, Point2, Point3};
pub use linetype::{
    is_byblock_name, is_bylayer_name, is_continuous_name, normalize_linetype_name, LineType,
};
pub use measure::{line_length, polyline_length, segment_length, DistanceReport};
pub use snap::{SnapFeature, SnapIndex, SnapKind};
pub use transform::Transform2;
