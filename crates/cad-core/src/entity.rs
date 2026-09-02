//! Native CAD entities. These types must not mention LibreDWG.

use crate::color::CadColor;
use crate::geom::Point3;

// ------------------------------------------------------------
// Type: Entity
// Purpose: One drawable object in the native document model.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Entity {
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
            layer: "0".to_string(),
            color: CadColor::ByLayer,
            linetype: "BYLAYER".to_string(),
            linetype_scale: 1.0,
            visible: true,
            geometry,
        }
    }
}

#[derive(Debug, Clone)]
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
    },
    Polyline {
        vertices: Vec<PolyVertex>,
        closed: bool,
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

#[derive(Debug, Clone, Copy)]
pub struct PolyVertex {
    pub point: Point3,
    pub bulge: f64,
}

#[derive(Debug, Clone)]
pub struct TextData {
    pub insertion: Point3,
    pub height: f64,
    pub rotation: f64,
    pub value: String,
    pub extrusion: Point3,
    pub is_attrib_def: bool,
}

#[derive(Debug, Clone)]
pub struct MTextData {
    pub insertion: Point3,
    pub height: f64,
    pub rotation: f64,
    pub width: f64,
    pub value: String,
    pub extrusion: Point3,
}

#[derive(Debug, Clone)]
pub struct HatchData {
    pub extrusion: Point3,
    pub elevation: f64,
    pub solid_fill: bool,
    pub paths: Vec<HatchPath>,
    pub pattern_lines: Vec<HatchPatternLine>,
}

#[derive(Debug, Clone)]
pub enum HatchPath {
    Polyline {
        vertices: Vec<PolyVertex>,
        closed: bool,
    },
    Edges(Vec<HatchEdge>),
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct HatchPatternLine {
    pub angle: f64,
    pub base: Point3,
    pub offset: Point3,
    pub dashes: Vec<f64>,
}

pub fn default_extrusion() -> Point3 {
    Point3::new(0.0, 0.0, 1.0)
}
