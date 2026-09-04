//! Shared geometric measurements used by inspectors and commands.

use crate::document::DrawingUnits;
use crate::entity::PolyVertex;
use crate::geom::{Point2, GEOM_TOLERANCE};

// ------------------------------------------------------------
// Type: DistanceMeasurement
// Purpose: Two-point distance, deltas, and direction in world XY.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistanceMeasurement {
    pub start: Point2,
    pub end: Point2,
    pub distance: f64,
    pub delta_x: f64,
    pub delta_y: f64,
    pub angle: f64,
}

pub type DistanceReport = DistanceMeasurement;

impl DistanceMeasurement {
    pub fn between(start: Point2, end: Point2) -> Option<Self> {
        if !start.is_finite() || !end.is_finite() {
            return None;
        }
        let delta_x = sanitize(end.x - start.x);
        let delta_y = sanitize(end.y - start.y);
        let distance = start.distance(end);
        if !distance.is_finite() || distance <= GEOM_TOLERANCE {
            return None;
        }
        Some(Self {
            start,
            end,
            distance,
            delta_x,
            delta_y,
            angle: delta_y.atan2(delta_x),
        })
    }
}

// ------------------------------------------------------------
// Type: AngleMeasurement
// Purpose: Smaller included angle between two directions, 0–π.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AngleMeasurement {
    pub vertex: Point2,
    pub ray_a: Point2,
    pub ray_b: Point2,
    pub angle: f64,
}

impl AngleMeasurement {
    pub fn from_directions(vertex: Point2, point_a: Point2, point_b: Point2) -> Option<Self> {
        if !vertex.is_finite() || !point_a.is_finite() || !point_b.is_finite() {
            return None;
        }
        let da = point_a - vertex;
        let db = point_b - vertex;
        let la = (da.x * da.x + da.y * da.y).sqrt();
        let lb = (db.x * db.x + db.y * db.y).sqrt();
        if la <= GEOM_TOLERANCE || lb <= GEOM_TOLERANCE {
            return None;
        }
        let ua = Point2::new(da.x / la, da.y / la);
        let ub = Point2::new(db.x / lb, db.y / lb);
        let cross = ua.x * ub.y - ua.y * ub.x;
        if cross.abs() <= GEOM_TOLERANCE && (ua.x * ub.x + ua.y * ub.y).abs() > 1.0 - 1e-9 {
            // coincident directions (0° or 180° still valid if not zero-length)
        }
        let dot = (ua.x * ub.x + ua.y * ub.y).clamp(-1.0, 1.0);
        let angle = dot.acos();
        if !angle.is_finite() {
            return None;
        }
        Some(Self {
            vertex,
            ray_a: vertex + ua,
            ray_b: vertex + ub,
            angle,
        })
    }

    pub fn from_segments(a0: Point2, a1: Point2, b0: Point2, b1: Point2) -> Option<Self> {
        let da = a1 - a0;
        let db = b1 - b0;
        let la = (da.x * da.x + da.y * da.y).sqrt();
        let lb = (db.x * db.x + db.y * db.y).sqrt();
        if la <= GEOM_TOLERANCE || lb <= GEOM_TOLERANCE {
            return None;
        }
        let vertex = infinite_line_intersection(a0, a1, b0, b1).unwrap_or_else(|| {
            Point2::new(
                (a0.x + a1.x + b0.x + b1.x) * 0.25,
                (a0.y + a1.y + b0.y + b1.y) * 0.25,
            )
        });
        Self::from_directions(
            vertex,
            vertex + Point2::new(da.x / la, da.y / la),
            vertex + Point2::new(db.x / lb, db.y / lb),
        )
    }
}

// ------------------------------------------------------------
// Type: RadiusMeasurement
// Purpose: Exact circle or arc radius, never from tessellation.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadiusMeasurement {
    pub center: Point2,
    pub toward: Point2,
    pub radius: f64,
    pub included_angle: Option<f64>,
    pub arc_length: Option<f64>,
}

impl RadiusMeasurement {
    pub fn circle(center: Point2, radius: f64, toward: Point2) -> Option<Self> {
        if !center.is_finite()
            || !toward.is_finite()
            || !radius.is_finite()
            || radius <= GEOM_TOLERANCE
        {
            return None;
        }
        Some(Self {
            center,
            toward: point_on_circle(center, radius, toward),
            radius,
            included_angle: None,
            arc_length: None,
        })
    }

    pub fn arc(
        center: Point2,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
        toward: Point2,
    ) -> Option<Self> {
        let circle = Self::circle(center, radius, toward)?;
        let sweep = ccw_sweep(start_angle, end_angle);
        if !sweep.is_finite() {
            return None;
        }
        Some(Self {
            included_angle: Some(sweep),
            arc_length: Some(radius * sweep),
            ..circle
        })
    }
}

// ------------------------------------------------------------
// Type: AreaMeasurement
// Purpose: Closed-boundary area and perimeter from semantic geometry.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub struct AreaMeasurement {
    pub area: f64,
    pub perimeter: f64,
    pub centroid: Point2,
    pub vertices: Vec<PolyVertex>,
}

impl AreaMeasurement {
    pub fn from_circle(center: Point2, radius: f64) -> Option<Self> {
        if !center.is_finite() || !radius.is_finite() || radius <= GEOM_TOLERANCE {
            return None;
        }
        let area = std::f64::consts::PI * radius * radius;
        let perimeter = std::f64::consts::TAU * radius;
        if !area.is_finite() || !perimeter.is_finite() {
            return None;
        }
        Some(Self {
            area,
            perimeter,
            centroid: center,
            vertices: circle_as_loop(center, radius),
        })
    }

    pub fn from_polyline(vertices: &[PolyVertex], closed: bool) -> Result<Self, MeasureError> {
        if !closed || vertices.len() < 2 {
            return Err(MeasureError::OpenBoundary);
        }
        if vertices.len() < 3 && vertices.iter().all(|v| v.bulge.abs() <= GEOM_TOLERANCE) {
            return Err(MeasureError::OpenBoundary);
        }
        if polyline_self_intersects(vertices) {
            return Err(MeasureError::SelfIntersecting);
        }
        let perimeter = polyline_length(vertices, true);
        let (signed, centroid) = polyline_area_centroid(vertices)?;
        let area = signed.abs();
        if !area.is_finite() || area <= GEOM_TOLERANCE || !perimeter.is_finite() {
            return Err(MeasureError::InvalidGeometry);
        }
        Ok(Self {
            area,
            perimeter,
            centroid,
            vertices: vertices.to_vec(),
        })
    }

    pub fn from_points(points: &[Point2]) -> Result<Self, MeasureError> {
        if points.len() < 3 {
            return Err(MeasureError::OpenBoundary);
        }
        let vertices: Vec<PolyVertex> = points
            .iter()
            .map(|p| PolyVertex {
                point: crate::geom::Point3::from_xy(p.x, p.y),
                bulge: 0.0,
            })
            .collect();
        Self::from_polyline(&vertices, true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasureError {
    OpenBoundary,
    SelfIntersecting,
    InvalidGeometry,
    NonUniformScale,
    Unsupported,
}

impl MeasureError {
    pub fn message(self) -> &'static str {
        match self {
            Self::OpenBoundary => "Boundary is open; area requires a closed shape",
            Self::SelfIntersecting => "Boundary is self-intersecting",
            Self::InvalidGeometry => "Geometry is not valid to measure",
            Self::NonUniformScale => "Object has no single radius after non-uniform scaling",
            Self::Unsupported => "Object is not supported for this measurement",
        }
    }
}

// ------------------------------------------------------------
// Enum: MeasurementResult
// Purpose: One inspect-only measurement. Raw f64 values only.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub enum MeasurementResult {
    Distance(DistanceMeasurement),
    Angle(AngleMeasurement),
    Radius(RadiusMeasurement),
    Area(AreaMeasurement),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasurementText {
    pub primary: String,
    pub details: Vec<String>,
    pub clipboard: String,
}

impl MeasurementResult {
    pub fn format(&self, units: DrawingUnits) -> MeasurementText {
        match self {
            Self::Distance(m) => {
                let primary = format_length(m.distance, units);
                let details = vec![
                    format!("ΔX {}", format_length(m.delta_x, units)),
                    format!("ΔY {}", format_length(m.delta_y, units)),
                    format!("∠ {}", format_angle_deg(m.angle)),
                ];
                let clipboard = format!(
                    "Distance {}\n{}\n{}\n{}",
                    primary, details[0], details[1], details[2]
                );
                MeasurementText {
                    primary,
                    details,
                    clipboard,
                }
            }
            Self::Angle(m) => {
                let primary = format_angle_deg(m.angle);
                MeasurementText {
                    clipboard: format!("Angle {primary}"),
                    primary,
                    details: Vec::new(),
                }
            }
            Self::Radius(m) => {
                let primary = format!("R {}", format_length(m.radius, units));
                let mut details =
                    vec![format!("Diameter {}", format_length(m.radius * 2.0, units))];
                if let Some(length) = m.arc_length {
                    details.push(format!("Arc length {}", format_length(length, units)));
                }
                if let Some(included) = m.included_angle {
                    details.push(format!("Included angle {}", format_angle_deg(included)));
                }
                let clipboard = std::iter::once(primary.clone())
                    .chain(details.iter().cloned())
                    .collect::<Vec<_>>()
                    .join("\n");
                MeasurementText {
                    primary,
                    details,
                    clipboard,
                }
            }
            Self::Area(m) => {
                let primary = format_area(m.area, units);
                let details = vec![format!("Perimeter {}", format_length(m.perimeter, units))];
                let clipboard = format!("Area {}\n{}", primary, details[0]);
                MeasurementText {
                    primary,
                    details,
                    clipboard,
                }
            }
        }
    }

    pub fn is_valid(&self) -> bool {
        match self {
            Self::Distance(m) => m.distance.is_finite() && m.distance > GEOM_TOLERANCE,
            Self::Angle(m) => m.angle.is_finite() && m.vertex.is_finite(),
            Self::Radius(m) => m.radius.is_finite() && m.radius > GEOM_TOLERANCE,
            Self::Area(m) => {
                m.area.is_finite() && m.area > GEOM_TOLERANCE && m.perimeter.is_finite()
            }
        }
    }
}

pub fn line_length(start: Point2, end: Point2) -> f64 {
    start.distance(end)
}

pub fn polyline_length(vertices: &[PolyVertex], closed: bool) -> f64 {
    if vertices.len() < 2 {
        return 0.0;
    }
    let n = if closed {
        vertices.len()
    } else {
        vertices.len() - 1
    };
    let mut total = 0.0;
    for i in 0..n {
        let a = vertices[i];
        let b = vertices[(i + 1) % vertices.len()];
        total += segment_length(a, b);
    }
    total
}

pub fn segment_length(a: PolyVertex, b: PolyVertex) -> f64 {
    let chord = a.point.xy().distance(b.point.xy());
    if a.bulge.abs() < GEOM_TOLERANCE {
        return chord;
    }
    let theta = 4.0 * a.bulge.atan();
    if theta.abs() < GEOM_TOLERANCE {
        chord
    } else {
        let radius = chord / (2.0 * (theta * 0.5).sin().abs().max(GEOM_TOLERANCE));
        (radius * theta).abs()
    }
}

pub fn arc_sweep(start_angle: f64, end_angle: f64) -> f64 {
    ccw_sweep(start_angle, end_angle)
}

pub fn arc_length(radius: f64, start_angle: f64, end_angle: f64) -> f64 {
    radius * ccw_sweep(start_angle, end_angle)
}

pub fn circle_area(radius: f64) -> f64 {
    std::f64::consts::PI * radius * radius
}

pub fn sanitize(value: f64) -> f64 {
    if !value.is_finite() {
        value
    } else if value.abs() < 5e-13 {
        0.0
    } else {
        value
    }
}

pub fn format_number(value: f64, decimals: usize) -> String {
    let value = sanitize(value);
    if !value.is_finite() {
        return "—".into();
    }
    format!("{value:.decimals$}")
}

pub fn format_length(value: f64, units: DrawingUnits) -> String {
    format!("{} {}", format_number(value, 4), units.label())
}

pub fn format_area(value: f64, units: DrawingUnits) -> String {
    format!("{} {}", format_number(value, 4), units.area_label())
}

pub fn format_angle_deg(radians: f64) -> String {
    let mut deg = sanitize(radians).to_degrees().rem_euclid(360.0);
    if deg.abs() < 5e-13 || (360.0 - deg).abs() < 5e-13 {
        deg = 0.0;
    }
    format!("{deg:.2}°")
}

pub fn infinite_line_intersection(
    a0: Point2,
    a1: Point2,
    b0: Point2,
    b1: Point2,
) -> Option<Point2> {
    let da = a1 - a0;
    let db = b1 - b0;
    let denom = da.x * db.y - da.y * db.x;
    if denom.abs() <= GEOM_TOLERANCE {
        return None;
    }
    let t = ((b0.x - a0.x) * db.y - (b0.y - a0.y) * db.x) / denom;
    let point = Point2::new(a0.x + t * da.x, a0.y + t * da.y);
    point.is_finite().then_some(point)
}

pub fn point_segment_distance(point: Point2, start: Point2, end: Point2) -> f64 {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let len2 = dx * dx + dy * dy;
    if len2 <= GEOM_TOLERANCE * GEOM_TOLERANCE {
        return point.distance(start);
    }
    let t = ((point.x - start.x) * dx + (point.y - start.y) * dy) / len2;
    let t = t.clamp(0.0, 1.0);
    point.distance(Point2::new(start.x + t * dx, start.y + t * dy))
}

pub fn point_circle_distance(point: Point2, center: Point2, radius: f64) -> f64 {
    (point.distance(center) - radius).abs()
}

pub fn angle_on_arc(point: Point2, center: Point2, start: f64, end: f64) -> bool {
    let ang = (point.y - center.y).atan2(point.x - center.x);
    let to = wrap_tau(ang - start);
    to <= ccw_sweep(start, end) + 1e-8
}

#[derive(Debug, Clone, Copy)]
pub struct BulgeCircle {
    pub center: Point2,
    pub radius: f64,
    pub start_angle: f64,
    pub sweep: f64,
}

pub fn bulge_circle(p1: Point2, p2: Point2, bulge: f64) -> Option<BulgeCircle> {
    if bulge.abs() < GEOM_TOLERANCE {
        return None;
    }
    let chord = p1.distance(p2);
    if chord < GEOM_TOLERANCE {
        return None;
    }
    let bulge_sq = bulge * bulge;
    let sweep = 4.0 * bulge.atan();
    let radius = chord * (1.0 + bulge_sq) / (4.0 * bulge.abs());
    let offset = chord * (1.0 - bulge_sq) / (4.0 * bulge);
    let ux = (p2.x - p1.x) / chord;
    let uy = (p2.y - p1.y) / chord;
    let center = Point2::new(
        (p1.x + p2.x) * 0.5 + (-uy) * offset,
        (p1.y + p2.y) * 0.5 + ux * offset,
    );
    Some(BulgeCircle {
        center,
        radius,
        start_angle: (p1.y - center.y).atan2(p1.x - center.x),
        sweep,
    })
}

pub fn point_bulge_distance(point: Point2, p1: Point2, p2: Point2, bulge: f64) -> f64 {
    let Some(arc) = bulge_circle(p1, p2, bulge) else {
        return point_segment_distance(point, p1, p2);
    };
    let ang = (point.y - arc.center.y).atan2(point.x - arc.center.x);
    let t = if arc.sweep.abs() < GEOM_TOLERANCE {
        0.0
    } else {
        let delta = if arc.sweep >= 0.0 {
            wrap_tau(ang - arc.start_angle)
        } else {
            wrap_tau(arc.start_angle - ang)
        };
        (delta / arc.sweep.abs()).clamp(0.0, 1.0)
    };
    let angle = arc.start_angle + arc.sweep * t;
    let on = Point2::new(
        arc.center.x + arc.radius * angle.cos(),
        arc.center.y + arc.radius * angle.sin(),
    );
    point.distance(on)
}

fn wrap_tau(angle: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let wrapped = angle.rem_euclid(tau);
    if wrapped < 0.0 {
        wrapped + tau
    } else {
        wrapped
    }
}

fn ccw_sweep(start: f64, end: f64) -> f64 {
    let mut sweep = end - start;
    if sweep.abs() < 1e-15 {
        sweep = std::f64::consts::TAU;
    }
    while sweep <= 0.0 {
        sweep += std::f64::consts::TAU;
    }
    while sweep > std::f64::consts::TAU + 1e-12 {
        sweep -= std::f64::consts::TAU;
    }
    sweep
}

pub fn point_on_circle(center: Point2, radius: f64, toward: Point2) -> Point2 {
    let dx = toward.x - center.x;
    let dy = toward.y - center.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len <= GEOM_TOLERANCE {
        Point2::new(center.x + radius, center.y)
    } else {
        Point2::new(center.x + radius * dx / len, center.y + radius * dy / len)
    }
}

fn circle_as_loop(center: Point2, radius: f64) -> Vec<PolyVertex> {
    (0..32)
        .map(|i| {
            let a = std::f64::consts::TAU * (i as f64) / 32.0;
            PolyVertex {
                point: crate::geom::Point3::from_xy(
                    center.x + radius * a.cos(),
                    center.y + radius * a.sin(),
                ),
                bulge: 0.0,
            }
        })
        .collect()
}

fn polyline_area_centroid(vertices: &[PolyVertex]) -> Result<(f64, Point2), MeasureError> {
    let mut area = 0.0;
    let mut cx = 0.0;
    let mut cy = 0.0;
    for i in 0..vertices.len() {
        let a = vertices[i].point.xy();
        let b = vertices[(i + 1) % vertices.len()].point.xy();
        let cross = a.x * b.y - b.x * a.y;
        area += cross;
        cx += (a.x + b.x) * cross;
        cy += (a.y + b.y) * cross;
        if let Some(arc) = bulge_circle(a, b, vertices[i].bulge) {
            let segment = 0.5 * arc.radius * arc.radius * (arc.sweep - arc.sweep.sin());
            area += segment * 2.0;
        }
    }
    let signed = sanitize(area * 0.5);
    if !signed.is_finite() {
        return Err(MeasureError::InvalidGeometry);
    }
    let denom = area;
    let centroid = if denom.abs() > GEOM_TOLERANCE {
        Point2::new(sanitize(cx / (3.0 * denom)), sanitize(cy / (3.0 * denom)))
    } else {
        vertices[0].point.xy()
    };
    Ok((signed, centroid))
}

fn polyline_self_intersects(vertices: &[PolyVertex]) -> bool {
    let n = vertices.len();
    if n < 4 {
        return false;
    }
    let pts: Vec<Point2> = vertices.iter().map(|v| v.point.xy()).collect();
    for i in 0..n {
        let a1 = pts[i];
        let a2 = pts[(i + 1) % n];
        for j in i + 1..n {
            if j == i || (j + 1) % n == i || (i + 1) % n == j {
                continue;
            }
            let b1 = pts[j];
            let b2 = pts[(j + 1) % n];
            if segments_intersect(a1, a2, b1, b2) {
                return true;
            }
        }
    }
    false
}

fn segments_intersect(a1: Point2, a2: Point2, b1: Point2, b2: Point2) -> bool {
    fn orient(p: Point2, q: Point2, r: Point2) -> f64 {
        (q.y - p.y) * (r.x - q.x) - (q.x - p.x) * (r.y - q.y)
    }
    let o1 = orient(a1, a2, b1);
    let o2 = orient(a1, a2, b2);
    let o3 = orient(b1, b2, a1);
    let o4 = orient(b1, b2, a2);
    o1 * o2 < -GEOM_TOLERANCE && o3 * o4 < -GEOM_TOLERANCE
}

pub fn point_in_closed_polyline(point: Point2, vertices: &[PolyVertex]) -> bool {
    let mut inside = false;
    let n = vertices.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let pi = vertices[i].point.xy();
        let pj = vertices[j].point.xy();
        if (pi.y > point.y) != (pj.y > point.y) {
            let x = (pj.x - pi.x) * (point.y - pi.y) / (pj.y - pi.y + 1e-30) + pi.x;
            if point.x < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Point3;

    fn vertex(x: f64, y: f64, bulge: f64) -> PolyVertex {
        PolyVertex {
            point: Point3::from_xy(x, y),
            bulge,
        }
    }

    #[test]
    fn distance_345() {
        let report = DistanceMeasurement::between(Point2::new(0.0, 0.0), Point2::new(3.0, 4.0))
            .expect("distance");
        assert!((report.distance - 5.0).abs() < 1e-12);
        assert!((report.delta_x - 3.0).abs() < 1e-12);
        assert!((report.delta_y - 4.0).abs() < 1e-12);
    }

    #[test]
    fn distance_report_includes_deltas_and_angle() {
        let report = DistanceMeasurement::between(Point2::new(0.0, 0.0), Point2::new(3.0, 4.0))
            .expect("distance");
        assert!((report.angle - (4.0_f64).atan2(3.0)).abs() < 1e-12);
    }

    #[test]
    fn angle_90_degrees_from_perpendicular_segments() {
        let angle = AngleMeasurement::from_segments(
            Point2::new(0.0, 0.0),
            Point2::new(10.0, 0.0),
            Point2::new(0.0, 0.0),
            Point2::new(0.0, 4.0),
        )
        .expect("angle");
        assert!((angle.angle - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        let text = MeasurementResult::Angle(angle).format(DrawingUnits::Millimeters);
        assert_eq!(text.primary, "90.00°");
    }

    #[test]
    fn angle_non_intersecting_uses_theoretical_intersection() {
        let angle = AngleMeasurement::from_segments(
            Point2::new(0.0, 1.0),
            Point2::new(2.0, 1.0),
            Point2::new(1.0, 0.0),
            Point2::new(1.0, 2.0),
        )
        .expect("angle");
        assert!((angle.angle - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        assert!((angle.vertex.x - 1.0).abs() < 1e-9);
        assert!((angle.vertex.y - 1.0).abs() < 1e-9);
    }

    #[test]
    fn wrapped_arc_length_and_included_angle() {
        let start = 5.5;
        let end = 0.5;
        let sweep = arc_sweep(start, end);
        assert!((sweep - (0.5 + std::f64::consts::TAU - 5.5)).abs() < 1e-12);
        let length = arc_length(2.0, start, end);
        assert!((length - 2.0 * sweep).abs() < 1e-12);
        let radius = RadiusMeasurement::arc(
            Point2::new(0.0, 0.0),
            2.0,
            start,
            end,
            Point2::new(2.0, 0.0),
        )
        .expect("arc radius");
        assert!((radius.included_angle.unwrap() - sweep).abs() < 1e-12);
    }

    #[test]
    fn circle_area_and_perimeter() {
        let area = AreaMeasurement::from_circle(Point2::new(0.0, 0.0), 10.0).expect("circle");
        assert!((area.area - std::f64::consts::PI * 100.0).abs() < 1e-9);
        assert!((area.perimeter - std::f64::consts::TAU * 10.0).abs() < 1e-9);
    }

    #[test]
    fn reversed_polygon_order_same_area() {
        let cw = [
            vertex(0.0, 0.0, 0.0),
            vertex(0.0, 2.0, 0.0),
            vertex(3.0, 2.0, 0.0),
            vertex(3.0, 0.0, 0.0),
        ];
        let ccw = [
            vertex(0.0, 0.0, 0.0),
            vertex(3.0, 0.0, 0.0),
            vertex(3.0, 2.0, 0.0),
            vertex(0.0, 2.0, 0.0),
        ];
        let a = AreaMeasurement::from_polyline(&cw, true).expect("cw");
        let b = AreaMeasurement::from_polyline(&ccw, true).expect("ccw");
        assert!((a.area - 6.0).abs() < 1e-9);
        assert!((b.area - 6.0).abs() < 1e-9);
        assert!((a.perimeter - 10.0).abs() < 1e-9);
    }

    #[test]
    fn semicircular_bulge_area() {
        let verts = [vertex(0.0, 0.0, 1.0), vertex(2.0, 0.0, 0.0)];
        let area = AreaMeasurement::from_polyline(&verts, true).expect("semicircle");
        assert!((area.area - std::f64::consts::PI * 0.5).abs() < 1e-6);
        assert!((area.perimeter - (std::f64::consts::PI + 2.0)).abs() < 1e-6);
    }

    #[test]
    fn polyline_length_includes_bulge_arcs() {
        let straight = polyline_length(&[vertex(0.0, 0.0, 0.0), vertex(10.0, 0.0, 0.0)], false);
        assert!((straight - 10.0).abs() < 1e-12);
        let semicircle = polyline_length(&[vertex(0.0, 0.0, 1.0), vertex(2.0, 0.0, 0.0)], false);
        assert!((semicircle - std::f64::consts::PI).abs() < 1e-9);
        let closed = polyline_length(
            &[
                vertex(0.0, 0.0, 0.0),
                vertex(1.0, 0.0, 0.0),
                vertex(1.0, 1.0, 0.0),
                vertex(0.0, 1.0, 0.0),
            ],
            true,
        );
        assert!((closed - 4.0).abs() < 1e-12);
    }

    #[test]
    fn formatting_uses_insunits_without_magnitude_conversion() {
        let text = format_length(125.0, DrawingUnits::Millimeters);
        assert_eq!(text, "125.0000 mm");
        assert_eq!(
            format_area(100.0, DrawingUnits::Millimeters),
            "100.0000 mm²"
        );
        assert_eq!(format_area(2.5, DrawingUnits::Inches), "2.5000 in²");
        assert_eq!(format_number(-0.0, 4), "0.0000");
        assert_eq!(format_angle_deg(-0.0), "0.00°");
        let wrapped = format_angle_deg(-std::f64::consts::FRAC_PI_2);
        assert_eq!(wrapped, "270.00°");
    }

    #[test]
    fn open_and_bowtie_boundaries_are_rejected() {
        let open = [vertex(0.0, 0.0, 0.0), vertex(1.0, 0.0, 0.0)];
        assert_eq!(
            AreaMeasurement::from_polyline(&open, false),
            Err(MeasureError::OpenBoundary)
        );
        let bowtie = [
            vertex(0.0, 0.0, 0.0),
            vertex(1.0, 1.0, 0.0),
            vertex(1.0, 0.0, 0.0),
            vertex(0.0, 1.0, 0.0),
        ];
        assert_eq!(
            AreaMeasurement::from_polyline(&bowtie, true),
            Err(MeasureError::SelfIntersecting)
        );
    }

    #[test]
    fn parallel_and_anti_parallel_lines_report_included_angle() {
        let parallel = AngleMeasurement::from_segments(
            Point2::new(0.0, 0.0),
            Point2::new(10.0, 0.0),
            Point2::new(0.0, 4.0),
            Point2::new(8.0, 4.0),
        )
        .expect("parallel");
        assert!(parallel.angle.abs() < 1e-9);
        let anti = AngleMeasurement::from_segments(
            Point2::new(0.0, 0.0),
            Point2::new(10.0, 0.0),
            Point2::new(8.0, 4.0),
            Point2::new(0.0, 4.0),
        )
        .expect("anti-parallel");
        assert!((anti.angle - std::f64::consts::PI).abs() < 1e-9);
    }
}
