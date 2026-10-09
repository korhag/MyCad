//! Native CAD document model used by import, viewport and rendering.
//!
//! This crate must stay independent of LibreDWG so future DXF import and
//! editing can share the same types.

pub mod block;
pub mod color;
pub mod compare;
pub mod curve_edit;
pub mod curves;
pub mod dash;
pub mod document;
pub mod dynamic;
pub mod dynamic_model;
pub mod entity;
pub mod entity_transform;
pub mod evaluate;
pub mod extents;
pub mod fixtures;
pub mod geom;
pub mod hatch;
mod hershey_simplex;
pub mod ids;
mod intersect;
pub mod linetype;
pub mod measure;
pub mod measure_index;
pub mod perf;
pub mod polygon;
pub mod snap;
pub mod snap_edges;
pub mod stroke_font;
pub mod thumbnail;
pub mod transform;
pub mod vectorize;

pub use block::{
    block_depends_on, clamped_array_counts, count_block_references, create_block_from_entities,
    duplicate_block_definition, identity_insert, insert_instance_ids, insert_instance_ids_in_space,
    insert_transform, is_paper_layout_block, is_system_block_name, is_user_editable_block_name,
    make_unique_block, membership_matrix, nesting_too_deep, next_user_block_name,
    paper_layout_block_order, purge_unused_user_blocks, rename_block, resolve_block_name,
    sorted_paper_layout_blocks, transfer_entity, user_block_list, validate_block_rename,
    validate_user_block_name, would_create_block_cycle, BlockError, BlockListEntry, BlockTreeChild,
    BlockTreeIndex, CreateBlockResult, MakeUniqueResult, TransferResult, MAX_BLOCK_DEPTH,
    MAX_INSERT_ARRAY_CELLS, NON_UNIFORM_MEMBERSHIP_MESSAGE,
};
pub use color::{aci_rgb, nearest_aci, CadColor, Rgb};
pub use compare::{compare_documents, CompareTol, Mismatch};
pub use curve_edit::{
    extend_geometry, offset_geometry, stretch_geometry, trim_geometry, EditError, StretchOutcome,
    TrimResult, MAX_EXTEND_EDGES, MAX_STRETCH_PREVIEW, MAX_TRIM_EDGES,
};
pub use curves::{
    arc_points, bspline_points, bulge_arc, catmull_rom_fit_points, circle_points,
    ellipse_arc_points, ellipse_points, polyline_points, polyline_points_with_tolerance,
    segments_for_arc, segments_per_turn, spline_sample_count, CIRCLE_SEGMENTS,
    POLYLINE_BULGE_SEGMENTS,
};
pub use document::{
    BlockDefinition, Document, DrawingUnits, EntityLocation, EntitySpace, ImportDiagnostics, Layer,
    PaperLayout, TextStyle,
};
pub use dynamic::{
    apply_anchor_policy, apply_size_axis, capability_for, collect_broken_bindings, dedupe_targets,
    follow_multiplier, format_display_number, increment_numeric, measure_size,
    migrate_choice_option, nearest_allowed_values, nearest_step_values, normalize_direction,
    numbers_equal, parse_allowed_value_list, proposed_configuration, resolve_values, snap_numeric,
    validate_behavior_conflicts, validate_configuration, validate_definition,
    validate_numeric_value, validate_parameter_def, validate_parameter_value, AnchorPolicy,
    BehaviorKind, BooleanParameter, ChoiceOption, ChoiceParameter, CompositionRule,
    DynamicBehavior, DynamicDefinition, DynamicError, FollowRole, GeometryTarget,
    InstanceConfiguration, MeasureMode, NumericDomain, NumericParameter, NumericQuantity,
    ParameterDef, ParameterKind, ParameterUnit, ParameterValue, ProposedConfiguration,
    SizeAuthoring, StepOrigin, StepPolicy, TextParameter, EVALUATOR_VERSION,
};
pub use dynamic_model::{
    active_compatibility_rules, conditions_match, configurations_equal, effective_visibility,
    escape_mtext_literal, evaluate_text_binding, format_parameter_display, matching_preset,
    option_usages, parameter_values_equal, remap_choice_value, rule_reason, value_allowed_by_rules,
    visibility_conditions_for, AnchorDef, AnchorFollow, CompatibilityRule, GeometryGroup,
    NestedInput, NestedMapping, OccurrencePath, ParameterCondition, PlacementBehavior, Preset,
    ReflectionBehavior, RotationBehavior, RotationSource, TextBinding, TextBindingMode,
    TextReflectPolicy, TextToken, TransformKind, VisibilityGroup,
};
pub use entity::{
    default_extrusion, AttributeInfo, DimensionData, DimensionKind, Entity, EntityId, Geometry,
    GradientStop, HatchData, HatchEdge, HatchGradient, HatchPath, HatchPatternLine, MTextData,
    PolyVertex, RasterFrame, TextData, TextHAlign, TextVAlign, ViewportData, ATTRIB_CONSTANT,
    ATTRIB_INVISIBLE, ATTRIB_PRESET, ATTRIB_VERIFY, LINEWEIGHT_BYBLOCK, LINEWEIGHT_BYLAYER,
    LINEWEIGHT_DEFAULT, MAX_HATCH_DASHES_PER_SPAN, MAX_HATCH_PATTERN_LINES,
    MAX_HATCH_PATTERN_SEGMENTS,
};
pub use entity_transform::{
    reference_radius, transform_entity, transform_entity_matrix, transform_geometry,
    validate_entities, EntityTransform, TransformError,
};
pub use evaluate::{
    apply_definition_preview, check_generation, document_has_dynamic_content, evaluate_definition,
    export_materialized, generated_block_name, is_generated_block_name, materialize_evaluated,
    materialize_evaluated_with, DynamicExportLink, EvalKey, EvaluatedBlock, EvaluationCache,
    EvaluationRequest, MaterializedExport, GENERATED_BLOCK_PREFIX,
};
pub use extents::Extents2;
pub use fixtures::{autocad_features_document, primitives_document};
pub use geom::{
    arc_from_three_points, ocs_to_wcs, ArcFromPointsError, Point2, Point3, ThreePointArc,
    GEOM_TOLERANCE,
};
pub use hatch::{
    hatch_fill_contours, hatch_path_points, hatch_path_points_with_tolerance,
    hatch_pattern_segments, GradientRamp,
};
pub use ids::{ActionId, AnchorId, BlockDefinitionId, OptionId, ParameterId, PresetId, VertexId};
pub use linetype::{
    is_byblock_name, is_bylayer_name, is_continuous_name, normalize_linetype_name, LineType,
    LineTypeShape,
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
pub use polygon::{contour_contains, contour_depths, point_in_polygon};
pub use snap::{SnapFeature, SnapIndex, SnapKind, MAX_SNAP_CANDIDATES};
pub use snap_edges::edge_snaps;
pub use stroke_font::{
    expand_cad_codes, measure_styled_width, measure_width, strip_mtext, stroke_text,
    stroke_text_styled,
};
pub use thumbnail::{
    path_is_thumbnail_folder, should_scan_asset, ContentFingerprint, SourceIdentity,
    SourceMetadata, ThumbnailRecord, ThumbnailRefreshPolicy, ThumbnailSettings, ThumbnailStatus,
    THUMBNAIL_FOLDER_NAME,
};
pub use transform::Transform2;
pub use vectorize::{
    plot_geometry, vectorize_entity, PlotFill, PlotGeometry, PlotStroke, VectorSink,
    VectorVisibility,
};
