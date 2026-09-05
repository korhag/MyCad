//! Native CAD entities. These types must not mention LibreDWG.

use crate::color::CadColor;
use crate::geom::Point3;

// ------------------------------------------------------------
// Type: EntityId
// Purpose: Stable identity for a drawable entity. Indices into
//          model_space are not durable across insert, erase, or undo.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct EntityId(pub u64);

impl EntityId {
    pub const UNASSIGNED: Self = Self(0);

    pub fn is_assigned(self) -> bool {
        self.0 != 0
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

// ------------------------------------------------------------
// Type: Entity
// Purpose: One drawable object in the native document model.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub id: EntityId,
    pub layer: String,
    pub color: CadColor,
    pub linetype: String,
    pub linetype_scale: f64,
    pub visible: bool,
    pub geometry: Geometry,
}

impl Entity {
    pub fn new(geometry: Geometry) -> Self {
        Self {
            id: EntityId::UNASSIGNED,
            layer: "0".to_string(),
            color: CadColor::ByLayer,
            linetype: "BYLAYER".to_string(),
            linetype_scale: 1.0,
            visible: true,
            geometry,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    Line {
        start: Point3,
        end: Point3,
    },
    Point {
        position: Point3,
    },
    Circle {
        center: Point3,
        radius: f64,
        extrusion: Point3,
    },
    Arc {
        center: Point3,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
        extrusion: Point3,
    },
    Ellipse {
        center: Point3,
        major_axis: Point3,
        axis_ratio: f64,
        start_param: f64,
        end_param: f64,
        extrusion: Point3,
    },
    LwPolyline {
        vertices: Vec<PolyVertex>,
        closed: bool,
        extrusion: Point3,
        linetype_generation_continuous: bool,
    },
    Polyline {
        vertices: Vec<PolyVertex>,
        closed: bool,
        linetype_generation_continuous: bool,
    },
    Spline {
        degree: u32,
        control_points: Vec<Point3>,
        fit_points: Vec<Point3>,
        knots: Vec<f64>,
        weights: Vec<f64>,
        closed: bool,
    },
    Insert {
        block_name: String,
        insertion: Point3,
        scale: Point3,
        rotation: f64,
        extrusion: Point3,
        attribs: Vec<TextData>,
        column_count: u32,
        row_count: u32,
        column_spacing: f64,
        row_spacing: f64,
    },
    Text(TextData),
    MText(MTextData),
    Hatch(HatchData),
    Dimension {
        block_name: String,
    },
    Solid {
        corners: [Point3; 4],
        extrusion: Point3,
    },
    Leader {
        vertices: Vec<Point3>,
    },
    MLine {
        vertices: Vec<Point3>,
        closed: bool,
    },
}

impl Geometry {
    // --------------------------------------------------------
    // Method: type_name
    // Purpose: Stable, user-facing name for inspectors and diagnostics.
    // --------------------------------------------------------
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Line { .. } => "Line",
            Self::Point { .. } => "Point",
            Self::Circle { .. } => "Circle",
            Self::Arc { .. } => "Arc",
            Self::Ellipse { .. } => "Ellipse",
            Self::LwPolyline { .. } => "Polyline",
            Self::Polyline { .. } => "Polyline",
            Self::Spline { .. } => "Spline",
            Self::Insert { .. } => "Block",
            Self::Text(_) => "Text",
            Self::MText(_) => "MText",
            Self::Hatch(_) => "Hatch",
            Self::Dimension { .. } => "Dimension",
            Self::Solid { .. } => "Solid",
            Self::Leader { .. } => "Leader",
            Self::MLine { .. } => "MLine",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolyVertex {
    pub point: Point3,
    pub bulge: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextData {
    pub insertion: Point3,
    pub height: f64,
    pub rotation: f64,
    pub value: String,
    pub extrusion: Point3,
    pub is_attrib_def: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MTextData {
    pub insertion: Point3,
    pub height: f64,
    pub rotation: f64,
    pub width: f64,
    pub value: String,
    pub extrusion: Point3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HatchData {
    pub extrusion: Point3,
    pub elevation: f64,
    pub solid_fill: bool,
    pub paths: Vec<HatchPath>,
    pub pattern_lines: Vec<HatchPatternLine>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HatchPath {
    Polyline {
        vertices: Vec<PolyVertex>,
        closed: bool,
    },
    Edges(Vec<HatchEdge>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum HatchEdge {
    Line {
        start: Point3,
        end: Point3,
    },
    Arc {
        center: Point3,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
        is_ccw: bool,
    },
    Ellipse {
        center: Point3,
        major_endpoint: Point3,
        axis_ratio: f64,
        start_angle: f64,
        end_angle: f64,
        is_ccw: bool,
    },
    Spline {
        control_points: Vec<Point3>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HatchPatternLine {
    pub angle: f64,
    pub base: Point3,
    pub offset: Point3,
    pub dashes: Vec<f64>,
}

pub fn default_extrusion() -> Point3 {
    Point3::new(0.0, 0.0, 1.0)
}
