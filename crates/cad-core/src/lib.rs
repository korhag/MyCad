//! Native CAD document model used by import, viewport and rendering.
//!
//! This crate must stay independent of LibreDWG so future DXF import and
//! editing can share the same types.

pub mod block;
pub mod color;
pub mod compare;
pub mod curves;
pub mod dash;
pub mod document;
pub mod entity;
pub mod entity_transform;
pub mod extents;
pub mod fixtures;
pub mod geom;
pub mod hatch;
pub mod linetype;
pub mod measure;
pub mod measure_index;
pub mod snap;
pub mod stroke_font;
pub mod transform;
pub mod vectorize;

pub use block::{
    block_depends_on, count_block_references, create_block_from_entities,
    duplicate_block_definition, identity_insert, insert_instance_ids, insert_transform,
    is_system_block_name, is_user_editable_block_name, make_unique_block, membership_matrix,
    next_user_block_name, purge_unused_user_blocks, rename_block, resolve_block_name,
    transfer_entity, user_block_list, validate_block_rename, validate_user_block_name,
    would_create_block_cycle, BlockError, BlockListEntry, BlockTreeChild, BlockTreeIndex,
    CreateBlockResult, MakeUniqueResult, TransferResult, NON_UNIFORM_MEMBERSHIP_MESSAGE,
};
pub use color::{aci_rgb, CadColor, Rgb};
pub use compare::{compare_documents, CompareTol, Mismatch};
pub use curves::{
    arc_points, bspline_points, bulge_arc, circle_points, ellipse_arc_points, ellipse_points,
    polyline_points, CIRCLE_SEGMENTS, POLYLINE_BULGE_SEGMENTS,
};
pub use document::{
    BlockDefinition, Document, DrawingUnits, EntitySpace, ImportDiagnostics, Layer,
};
pub use entity::{
    default_extrusion, Entity, EntityId, Geometry, HatchData, HatchEdge, HatchPath,
    HatchPatternLine, MTextData, PolyVertex, TextData,
};
pub use entity_transform::{
    reference_radius, transform_entity, transform_entity_matrix, transform_geometry,
    validate_entities, EntityTransform, TransformError,
};
pub use extents::Extents2;
pub use fixtures::primitives_document;
pub use geom::{
    arc_from_three_points, ocs_to_wcs, ArcFromPointsError, Point2, Point3, ThreePointArc,
    GEOM_TOLERANCE,
};
pub use hatch::hatch_path_points;
pub use linetype::{
    is_byblock_name, is_bylayer_name, is_continuous_name, normalize_linetype_name, LineType,
};
pub use measure::{
    arc_length, arc_sweep, bulge_circle, circle_area, format_angle_deg, format_area, format_length,
    format_number, line_length, polyline_length, segment_length, AngleMeasurement, AreaMeasurement,
    DistanceMeasurement, DistanceReport, MeasureError, MeasurementResult, MeasurementText,
    RadiusMeasurement,
};
pub use measure_index::{
    area_from_primitive, radius_from_primitive, straight_of, MeasureGeom, MeasureIndex,
    MeasurePrimitive, MeasureRole, MEASURE_APERTURE_PX,
};
pub use snap::{SnapFeature, SnapIndex, SnapKind};
pub use stroke_font::{measure_width, strip_mtext, stroke_text};
pub use transform::Transform2;
pub use vectorize::{
    plot_geometry, vectorize_entity, PlotFill, PlotGeometry, PlotStroke, VectorSink,
    VectorVisibility,
};
