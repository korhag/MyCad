//! Interactive command lifecycle and point consumption.

use cad_core::{
    arc_from_three_points, default_extrusion, DistanceReport, EntityId, EntityTransform, Geometry,
    MeasurementResult, Point2, Point3, PolyVertex, SnapFeature, SnapKind, GEOM_TOLERANCE,
};

use crate::dynamic_input::DynamicLayout;

// ------------------------------------------------------------
// Enum: ModifyKind
// Purpose: One shared modification command, not six boolean flags.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifyKind {
    Move,
    Copy,
    Rotate,
    Mirror,
    Scale,
    Erase,
}

impl ModifyKind {
    pub fn command_kind(self) -> CommandKind {
        match self {
            Self::Move => CommandKind::Move,
            Self::Copy => CommandKind::Copy,
            Self::Rotate => CommandKind::Rotate,
            Self::Mirror => CommandKind::Mirror,
            Self::Scale => CommandKind::Scale,
            Self::Erase => CommandKind::Erase,
        }
    }

    pub fn creates_copies(self) -> bool {
        matches!(self, Self::Copy | Self::Mirror)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifyPhase {
    Selecting,
    BasePoint,
    Destination,
    Angle,
    MirrorSecond,
    ScaleFactor,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModifyState {
    pub kind: ModifyKind,
    pub phase: ModifyPhase,
    pub targets: Vec<EntityId>,
    pub base: Option<Point2>,
    pub reference_radius: f64,
}

// ------------------------------------------------------------
// Enum: CommandKind
// Purpose: Ribbon and menu identity for the active command without
//          a separate `is_line` / `is_circle` predicate per tool.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Idle,
    Line,
    Polyline,
    Circle,
    Arc,
    Rectangle,
    Distance,
    Angle,
    Radius,
    Area,
    Move,
    Copy,
    Rotate,
    Mirror,
    Scale,
    Erase,
}

impl CommandKind {
    pub fn is_idle(self) -> bool {
        matches!(self, Self::Idle)
    }

    pub fn is_active(self) -> bool {
        !self.is_idle()
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Ready",
            Self::Line => "LINE",
            Self::Polyline => "PLINE",
            Self::Circle => "CIRCLE",
            Self::Arc => "ARC",
            Self::Rectangle => "RECTANGLE",
            Self::Distance => "DIST",
            Self::Angle => "ANGLE",
            Self::Radius => "RADIUS",
            Self::Area => "AREA",
            Self::Move => "MOVE",
            Self::Copy => "COPY",
            Self::Rotate => "ROTATE",
            Self::Mirror => "MIRROR",
            Self::Scale => "SCALE",
            Self::Erase => "ERASE",
        }
    }

    pub fn is_measure(self) -> bool {
        matches!(
            self,
            Self::Distance | Self::Angle | Self::Radius | Self::Area
        )
    }

    pub fn is_modify(self) -> bool {
        matches!(
            self,
            Self::Move | Self::Copy | Self::Rotate | Self::Mirror | Self::Scale | Self::Erase
        )
    }

    pub fn modify_kind(self) -> Option<ModifyKind> {
        match self {
            Self::Move => Some(ModifyKind::Move),
            Self::Copy => Some(ModifyKind::Copy),
            Self::Rotate => Some(ModifyKind::Rotate),
            Self::Mirror => Some(ModifyKind::Mirror),
            Self::Scale => Some(ModifyKind::Scale),
            Self::Erase => Some(ModifyKind::Erase),
            _ => None,
        }
    }
}

// ------------------------------------------------------------
// Enum: CommandOutput
// Purpose: Geometry or measurement produced when a command
//          consumes an accepted point.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
pub enum CommandOutput {
    None,
    Geometry(Geometry),
    Distance(DistanceReport),
    Measurement(MeasurementResult),
    Modify {
        transform: EntityTransform,
        copies: bool,
    },
    Rejected(&'static str),
}

// ------------------------------------------------------------
// Enum: PreviewGeometry
// Purpose: Live rubber-band drawn while the pointer moves. Never
//          written into the document.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub enum PreviewGeometry {
    LineSegment([Point2; 2]),
    Polyline {
        vertices: Vec<Point2>,
        next: Option<Point2>,
        closed: bool,
    },
    Circle {
        center: Point2,
        radius: f64,
    },
    Arc {
        center: Point2,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    },
    Rectangle {
        corners: [Point2; 4],
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineState {
    pub first: Option<Point2>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolylineState {
    pub vertices: Vec<Point2>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CircleState {
    pub center: Option<Point2>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArcState {
    pub start: Option<Point2>,
    pub mid: Option<Point2>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RectangleState {
    pub first: Option<Point2>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DistanceState {
    pub first: Option<Point2>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AngleState {
    Prompt,
    FirstSegment {
        start: Point2,
        end: Point2,
    },
    ThreePoint {
        vertex: Option<Point2>,
        ray: Option<Point2>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum AreaState {
    Prompt,
    Points { vertices: Vec<Point2> },
}

// ------------------------------------------------------------
// Enum: CommandState
// Purpose: Routes viewport points to the active command before
//          idle selection sees them.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Default)]
pub enum CommandState {
    #[default]
    Idle,
    Line(LineState),
    Polyline(PolylineState),
    Circle(CircleState),
    Arc(ArcState),
    Rectangle(RectangleState),
    Distance(DistanceState),
    Angle(AngleState),
    Radius,
    Area(AreaState),
    Modify(ModifyState),
}

impl CommandState {
    pub fn kind(&self) -> CommandKind {
        match self {
            Self::Idle => CommandKind::Idle,
            Self::Line(_) => CommandKind::Line,
            Self::Polyline(_) => CommandKind::Polyline,
            Self::Circle(_) => CommandKind::Circle,
            Self::Arc(_) => CommandKind::Arc,
            Self::Rectangle(_) => CommandKind::Rectangle,
            Self::Distance(_) => CommandKind::Distance,
            Self::Angle(_) => CommandKind::Angle,
            Self::Radius => CommandKind::Radius,
            Self::Area(_) => CommandKind::Area,
            Self::Modify(state) => state.kind.command_kind(),
        }
    }

    pub fn is_active(&self) -> bool {
        self.kind().is_active()
    }

    pub fn is_idle(&self) -> bool {
        self.kind().is_idle()
    }

    pub fn is_selecting_objects(&self) -> bool {
        matches!(
            self,
            Self::Modify(ModifyState {
                phase: ModifyPhase::Selecting,
                ..
            })
        )
    }

    pub fn is_erase_picking(&self) -> bool {
        matches!(
            self,
            Self::Modify(ModifyState {
                kind: ModifyKind::Erase,
                phase: ModifyPhase::Selecting,
                ..
            })
        )
    }

    pub fn modify_targets(&self) -> &[EntityId] {
        match self {
            Self::Modify(state) => &state.targets,
            _ => &[],
        }
    }

    pub fn requests_point(&self) -> bool {
        self.is_active() && !self.is_selecting_objects()
    }

    pub fn uses_osnap(&self) -> bool {
        match self {
            Self::Angle(AngleState::Prompt | AngleState::FirstSegment { .. })
            | Self::Radius
            | Self::Area(AreaState::Prompt)
            | Self::Modify(ModifyState {
                phase: ModifyPhase::Selecting,
                ..
            }) => false,
            Self::Idle => false,
            _ => true,
        }
    }

    pub fn start_line(&mut self) {
        *self = Self::Line(LineState { first: None });
    }

    pub fn start_polyline(&mut self) {
        *self = Self::Polyline(PolylineState {
            vertices: Vec::new(),
        });
    }

    pub fn start_circle(&mut self) {
        *self = Self::Circle(CircleState { center: None });
    }

    pub fn start_arc(&mut self) {
        *self = Self::Arc(ArcState {
            start: None,
            mid: None,
        });
    }

    pub fn start_rectangle(&mut self) {
        *self = Self::Rectangle(RectangleState { first: None });
    }

    pub fn start_distance(&mut self) {
        *self = Self::Distance(DistanceState { first: None });
    }

    pub fn start_angle(&mut self) {
        *self = Self::Angle(AngleState::Prompt);
    }

    pub fn start_radius(&mut self) {
        *self = Self::Radius;
    }

    pub fn start_area(&mut self) {
        *self = Self::Area(AreaState::Prompt);
    }

    pub fn start_modify(&mut self, kind: ModifyKind, selected: Vec<EntityId>) {
        let phase = if kind == ModifyKind::Erase || selected.is_empty() {
            ModifyPhase::Selecting
        } else {
            ModifyPhase::BasePoint
        };
        *self = Self::Modify(ModifyState {
            kind,
            phase,
            targets: selected,
            base: None,
            reference_radius: 1.0,
        });
    }

    pub fn confirm_modify_selection(&mut self, selected: Vec<EntityId>, reference_radius: f64) {
        let Self::Modify(state) = self else {
            return;
        };
        state.targets = selected;
        state.reference_radius = reference_radius.max(1.0);
        if state.kind == ModifyKind::Erase {
            return;
        }
        state.phase = ModifyPhase::BasePoint;
        state.base = None;
    }

    pub fn set_scale_reference(&mut self, radius: f64) {
        if let Self::Modify(state) = self {
            state.reference_radius = radius.max(1.0);
        }
    }

    pub fn cancel(&mut self) {
        *self = Self::Idle;
    }

    pub fn finish(&mut self) {
        *self = Self::Idle;
    }

    pub fn start_vertex(&self) -> Option<Point2> {
        match self {
            Self::Line(state) => state.first,
            Self::Polyline(state) => state.vertices.first().copied(),
            _ => None,
        }
    }

    pub fn write_snap_features(&self, out: &mut Vec<SnapFeature>) {
        out.clear();
        let points: &[Point2] = match self {
            Self::Polyline(state) => &state.vertices,
            _ => return,
        };
        if points.len() < 2 {
            return;
        }
        let base = *points.last().expect("accepted points");
        for point in &points[..points.len() - 1] {
            if point.distance(base) > GEOM_TOLERANCE {
                out.push(SnapFeature {
                    point: *point,
                    kind: SnapKind::Endpoint,
                });
            }
        }
        for pair in points.windows(2) {
            let mid = pair[0].lerp(pair[1], 0.5);
            if mid.distance(base) > GEOM_TOLERANCE {
                out.push(SnapFeature {
                    point: mid,
                    kind: SnapKind::Midpoint,
                });
            }
        }
    }

    pub fn base_point(&self) -> Option<Point2> {
        match self {
            Self::Line(state) => state.first,
            Self::Polyline(state) => state.vertices.last().copied(),
            Self::Circle(state) => state.center,
            Self::Arc(state) => state.mid.or(state.start),
            Self::Rectangle(state) => state.first,
            Self::Distance(state) => state.first,
            Self::Angle(AngleState::ThreePoint { vertex, ray }) => ray.or(*vertex),
            Self::Area(AreaState::Points { vertices }) => vertices.last().copied(),
            Self::Modify(state) => state.base,
            Self::Idle | Self::Angle(_) | Self::Radius | Self::Area(_) => None,
        }
    }

    pub fn dynamic_layout(&self) -> DynamicLayout {
        match self {
            Self::Line(state) if state.first.is_some() => DynamicLayout::LengthAngle,
            Self::Polyline(state) if !state.vertices.is_empty() => DynamicLayout::LengthAngle,
            Self::Circle(state) if state.center.is_some() => DynamicLayout::Radius,
            Self::Rectangle(state) if state.first.is_some() => DynamicLayout::WidthHeight,
            Self::Modify(state) => match state.phase {
                ModifyPhase::Destination => DynamicLayout::LengthAngle,
                ModifyPhase::Angle => DynamicLayout::Angle,
                ModifyPhase::ScaleFactor => DynamicLayout::Factor,
                _ => DynamicLayout::Hidden,
            },
            _ => DynamicLayout::Hidden,
        }
    }

    pub fn can_finish(&self) -> bool {
        match self {
            Self::Line(state) => state.first.is_some(),
            Self::Polyline(state) => state.vertices.len() >= 2,
            Self::Area(AreaState::Points { vertices }) => vertices.len() >= 3,
            Self::Circle(_) | Self::Arc(_) | Self::Rectangle(_) => true,
            Self::Modify(ModifyState {
                kind: ModifyKind::Erase,
                phase: ModifyPhase::Selecting,
                ..
            }) => true,
            Self::Modify(ModifyState {
                phase: ModifyPhase::Selecting,
                targets,
                ..
            }) => !targets.is_empty(),
            _ => false,
        }
    }

    pub fn can_close(&self) -> bool {
        match self {
            Self::Polyline(state) => polyline_can_close(&state.vertices),
            _ => false,
        }
    }

    pub fn can_undo_last(&self) -> bool {
        match self {
            Self::Line(state) => state.first.is_some(),
            Self::Polyline(state) => !state.vertices.is_empty(),
            Self::Area(AreaState::Points { vertices }) => !vertices.is_empty(),
            Self::Modify(state) => state.phase != ModifyPhase::Selecting,
            _ => false,
        }
    }

    pub fn can_back(&self) -> bool {
        match self {
            Self::Circle(state) => state.center.is_some(),
            Self::Arc(state) => state.start.is_some(),
            Self::Rectangle(state) => state.first.is_some(),
            Self::Distance(state) => state.first.is_some(),
            Self::Angle(AngleState::FirstSegment { .. }) => true,
            Self::Angle(AngleState::ThreePoint { vertex, ray }) => {
                vertex.is_some() || ray.is_some()
            }
            Self::Area(AreaState::Points { .. }) => true,
            Self::Modify(state) => state.phase != ModifyPhase::Selecting,
            _ => false,
        }
    }

    pub fn undo_last(&mut self) -> bool {
        match self {
            Self::Line(state) if state.first.is_some() => {
                state.first = None;
                true
            }
            Self::Polyline(state) if !state.vertices.is_empty() => {
                state.vertices.pop();
                true
            }
            Self::Area(AreaState::Points { vertices }) if !vertices.is_empty() => {
                vertices.pop();
                if vertices.is_empty() {
                    *self = Self::Area(AreaState::Prompt);
                }
                true
            }
            Self::Modify(state) => back_modify(state),
            _ => false,
        }
    }

    pub fn back(&mut self) -> bool {
        match self {
            Self::Circle(state) if state.center.is_some() => {
                state.center = None;
                true
            }
            Self::Arc(state) if state.mid.is_some() => {
                state.mid = None;
                true
            }
            Self::Arc(state) if state.start.is_some() => {
                state.start = None;
                true
            }
            Self::Rectangle(state) if state.first.is_some() => {
                state.first = None;
                true
            }
            Self::Distance(state) if state.first.is_some() => {
                state.first = None;
                true
            }
            Self::Angle(AngleState::FirstSegment { .. }) => {
                *self = Self::Angle(AngleState::Prompt);
                true
            }
            Self::Angle(AngleState::ThreePoint { ray, vertex }) if ray.is_some() => {
                *ray = None;
                true
            }
            Self::Angle(AngleState::ThreePoint { vertex, .. }) if vertex.is_some() => {
                *self = Self::Angle(AngleState::Prompt);
                true
            }
            Self::Area(AreaState::Points { .. }) => {
                *self = Self::Area(AreaState::Prompt);
                true
            }
            Self::Modify(state) => back_modify(state),
            _ => false,
        }
    }

    pub fn accept_point(&mut self, point: Point2) -> CommandOutput {
        if !point.is_finite() {
            return CommandOutput::Rejected("Point is not finite");
        }
        let completes = matches!(
            self,
            Self::Circle(_)
                | Self::Arc(_)
                | Self::Rectangle(_)
                | Self::Distance(_)
                | Self::Angle(AngleState::ThreePoint { ray: Some(_), .. })
                | Self::Polyline(_)
        );
        let output = match self {
            Self::Idle => CommandOutput::None,
            Self::Line(state) => accept_line_point(state, point),
            Self::Polyline(state) => accept_polyline_point(state, point),
            Self::Circle(state) => accept_circle_point(state, point),
            Self::Arc(state) => accept_arc_point(state, point),
            Self::Rectangle(state) => accept_rectangle_point(state, point),
            Self::Distance(state) => accept_distance_point(state, point),
            Self::Angle(state) => accept_angle_point(state, point),
            Self::Area(state) => accept_area_point(state, point),
            Self::Modify(state) => accept_modify_point(state, point, None, None),
            Self::Radius => CommandOutput::Rejected("Select a Circle or Arc"),
        };
        if completes
            && matches!(
                output,
                CommandOutput::Geometry(_)
                    | CommandOutput::Distance(_)
                    | CommandOutput::Measurement(_)
            )
        {
            *self = Self::Idle;
        }
        output
    }

    pub fn accept_modify_point(
        &mut self,
        point: Point2,
        typed_angle_deg: Option<f64>,
        typed_factor: Option<f64>,
    ) -> CommandOutput {
        let Self::Modify(state) = self else {
            return self.accept_point(point);
        };
        accept_modify_point(state, point, typed_angle_deg, typed_factor)
    }

    pub fn preview_transform(
        &self,
        current: Option<Point2>,
        typed_angle_deg: Option<f64>,
        typed_factor: Option<f64>,
    ) -> Option<EntityTransform> {
        let Self::Modify(state) = self else {
            return None;
        };
        modify_preview_transform(state, current?, typed_angle_deg, typed_factor)
    }

    pub fn preview_mirror_axis(&self, current: Option<Point2>) -> Option<[Point2; 2]> {
        let Self::Modify(state) = self else {
            return None;
        };
        if state.kind != ModifyKind::Mirror || state.phase != ModifyPhase::MirrorSecond {
            return None;
        }
        Some([state.base?, current?])
    }

    pub fn accept_straight_segment(&mut self, start: Point2, end: Point2) -> CommandOutput {
        let Self::Angle(state) = self else {
            return CommandOutput::Rejected("Angle is not active");
        };
        match state {
            AngleState::Prompt => {
                *state = AngleState::FirstSegment { start, end };
                CommandOutput::None
            }
            AngleState::FirstSegment { start: a0, end: a1 } => {
                let (a0, a1) = (*a0, *a1);
                match cad_core::AngleMeasurement::from_segments(a0, a1, start, end) {
                    Some(angle) => {
                        *self = Self::Idle;
                        CommandOutput::Measurement(MeasurementResult::Angle(angle))
                    }
                    None => CommandOutput::Rejected("Directions are coincident or zero-length"),
                }
            }
            AngleState::ThreePoint { .. } => {
                CommandOutput::Rejected("Specify a point for the current ray")
            }
        }
    }

    pub fn begin_three_point_angle(&mut self, vertex: Point2) -> CommandOutput {
        if !matches!(self, Self::Angle(AngleState::Prompt)) {
            return CommandOutput::Rejected("Angle is not waiting for a vertex");
        }
        *self = Self::Angle(AngleState::ThreePoint {
            vertex: Some(vertex),
            ray: None,
        });
        CommandOutput::None
    }

    pub fn begin_area_points(&mut self, first: Point2) -> CommandOutput {
        if !matches!(self, Self::Area(AreaState::Prompt)) {
            return CommandOutput::Rejected("Area is not waiting for a point");
        }
        *self = Self::Area(AreaState::Points {
            vertices: vec![first],
        });
        CommandOutput::None
    }

    pub fn finish_measurement(
        &mut self,
    ) -> Option<Result<MeasurementResult, cad_core::MeasureError>> {
        match self {
            Self::Area(AreaState::Points { vertices }) if vertices.len() >= 3 => {
                match cad_core::AreaMeasurement::from_points(vertices) {
                    Ok(area) => {
                        *self = Self::Idle;
                        Some(Ok(MeasurementResult::Area(area)))
                    }
                    Err(err) => Some(Err(err)),
                }
            }
            _ => None,
        }
    }

    pub fn live_measurement(&self, current: Option<Point2>) -> Option<MeasurementResult> {
        match self {
            Self::Distance(DistanceState { first: Some(first) }) => {
                cad_core::DistanceMeasurement::between(*first, current?)
                    .map(MeasurementResult::Distance)
            }
            Self::Angle(AngleState::ThreePoint {
                vertex: Some(vertex),
                ray: Some(ray),
            }) => cad_core::AngleMeasurement::from_directions(*vertex, *ray, current?)
                .map(MeasurementResult::Angle),
            Self::Area(AreaState::Points { vertices }) if vertices.len() >= 2 => {
                let mut pts = vertices.clone();
                if let Some(current) = current {
                    pts.push(current);
                }
                cad_core::AreaMeasurement::from_points(&pts)
                    .ok()
                    .map(MeasurementResult::Area)
            }
            _ => None,
        }
    }

    pub fn finish_geometry(&self) -> Option<Geometry> {
        match self {
            Self::Polyline(state) if state.vertices.len() >= 2 => {
                Some(lwpolyline_geometry(&state.vertices, false))
            }
            _ => None,
        }
    }

    pub fn close_geometry(&self) -> Option<Geometry> {
        match self {
            Self::Polyline(state) if polyline_can_close(&state.vertices) => {
                Some(lwpolyline_geometry(&state.vertices, true))
            }
            _ => None,
        }
    }

    pub fn preview(&self, current: Option<Point2>) -> Option<PreviewGeometry> {
        match self {
            Self::Idle
            | Self::Distance(_)
            | Self::Angle(_)
            | Self::Radius
            | Self::Area(_)
            | Self::Modify(_) => None,
            Self::Line(state) => {
                let first = state.first?;
                let current = current?;
                Some(PreviewGeometry::LineSegment([first, current]))
            }
            Self::Polyline(state) => {
                if state.vertices.is_empty() {
                    return None;
                }
                Some(PreviewGeometry::Polyline {
                    vertices: state.vertices.clone(),
                    next: current,
                    closed: false,
                })
            }
            Self::Circle(state) => {
                let center = state.center?;
                let current = current?;
                let radius = center.distance(current);
                (radius > GEOM_TOLERANCE).then_some(PreviewGeometry::Circle { center, radius })
            }
            Self::Arc(state) => preview_arc(state, current),
            Self::Rectangle(state) => {
                let first = state.first?;
                let current = current?;
                rectangle_preview(first, current)
            }
        }
    }

    pub fn prompt(&self) -> &'static str {
        match self {
            Self::Idle => "Command: Ready",
            Self::Line(state) if state.first.is_none() => "LINE • Specify first point",
            Self::Line(_) => "LINE • Specify second point",
            Self::Polyline(state) if state.vertices.is_empty() => "PLINE • Specify start point",
            Self::Polyline(_) => "PLINE • Specify next vertex or press Enter to finish",
            Self::Circle(state) if state.center.is_none() => "CIRCLE • Specify center point",
            Self::Circle(_) => "CIRCLE • Specify radius",
            Self::Arc(state) if state.start.is_none() => "ARC • Specify start point",
            Self::Arc(state) if state.mid.is_none() => "ARC • Specify point on arc",
            Self::Arc(_) => "ARC • Specify end point",
            Self::Rectangle(state) if state.first.is_none() => "RECTANGLE • Specify first corner",
            Self::Rectangle(_) => "RECTANGLE • Specify opposite corner",
            Self::Distance(state) if state.first.is_none() => "DIST • Specify first point",
            Self::Distance(_) => "DIST • Specify second point",
            Self::Angle(AngleState::Prompt) => {
                "ANGLE • Select first line, or click empty space to specify the vertex"
            }
            Self::Angle(AngleState::FirstSegment { .. }) => "ANGLE • Select second line",
            Self::Angle(AngleState::ThreePoint { vertex: None, .. }) => {
                "ANGLE • Specify the vertex"
            }
            Self::Angle(AngleState::ThreePoint { ray: None, .. }) => {
                "ANGLE • Specify the first ray"
            }
            Self::Angle(AngleState::ThreePoint { .. }) => "ANGLE • Specify the second ray",
            Self::Radius => "RADIUS • Select a Circle or Arc",
            Self::Area(AreaState::Prompt) => "AREA • Select a closed object or specify first point",
            Self::Area(AreaState::Points { vertices }) if vertices.len() < 3 => {
                "AREA • Specify next boundary point"
            }
            Self::Area(AreaState::Points { .. }) => "AREA • Specify next point, or Enter to finish",
            Self::Modify(state) => modify_prompt(state),
        }
    }
}

fn accept_line_point(state: &mut LineState, point: Point2) -> CommandOutput {
    match state.first {
        None => {
            state.first = Some(point);
            CommandOutput::None
        }
        Some(first) => {
            if first.distance(point) <= GEOM_TOLERANCE {
                return CommandOutput::Rejected("Length must be greater than zero");
            }
            state.first = None;
            CommandOutput::Geometry(line_geometry(first, point))
        }
    }
}

fn accept_polyline_point(state: &mut PolylineState, point: Point2) -> CommandOutput {
    if let Some(last) = state.vertices.last() {
        if last.distance(point) <= GEOM_TOLERANCE {
            return CommandOutput::Rejected("Length must be greater than zero");
        }
    }
    if polyline_can_close(&state.vertices) {
        if let Some(first) = state.vertices.first().copied() {
            if first.distance(point) <= GEOM_TOLERANCE {
                return CommandOutput::Geometry(lwpolyline_geometry(&state.vertices, true));
            }
        }
    }
    state.vertices.push(point);
    CommandOutput::None
}

fn polyline_can_close(points: &[Point2]) -> bool {
    if points.len() < 3 {
        return false;
    }
    let first = points[0];
    let last = *points.last().expect("polyline has vertices");
    first.distance(last) > GEOM_TOLERANCE
}

fn back_modify(state: &mut ModifyState) -> bool {
    match state.phase {
        ModifyPhase::Selecting => false,
        ModifyPhase::BasePoint => {
            state.phase = ModifyPhase::Selecting;
            state.base = None;
            true
        }
        ModifyPhase::Destination
        | ModifyPhase::Angle
        | ModifyPhase::MirrorSecond
        | ModifyPhase::ScaleFactor => {
            state.phase = ModifyPhase::BasePoint;
            state.base = None;
            true
        }
    }
}

fn accept_modify_point(
    state: &mut ModifyState,
    point: Point2,
    typed_angle_deg: Option<f64>,
    typed_factor: Option<f64>,
) -> CommandOutput {
    match state.phase {
        ModifyPhase::Selecting => CommandOutput::Rejected("Select objects, then press Enter"),
        ModifyPhase::BasePoint => {
            state.base = Some(point);
            state.phase = match state.kind {
                ModifyKind::Move | ModifyKind::Copy => ModifyPhase::Destination,
                ModifyKind::Rotate => ModifyPhase::Angle,
                ModifyKind::Mirror => ModifyPhase::MirrorSecond,
                ModifyKind::Scale => ModifyPhase::ScaleFactor,
                ModifyKind::Erase => ModifyPhase::Selecting,
            };
            CommandOutput::None
        }
        ModifyPhase::Destination => {
            let base = state.base.expect("base point");
            CommandOutput::Modify {
                transform: EntityTransform::Translate {
                    dx: point.x - base.x,
                    dy: point.y - base.y,
                },
                copies: state.kind.creates_copies(),
            }
        }
        ModifyPhase::Angle => {
            let base = state.base.expect("base point");
            let radians = typed_angle_deg
                .map(|deg| deg.to_radians())
                .unwrap_or_else(|| (point.y - base.y).atan2(point.x - base.x));
            CommandOutput::Modify {
                transform: EntityTransform::Rotate { base, radians },
                copies: false,
            }
        }
        ModifyPhase::MirrorSecond => {
            let axis_start = state.base.expect("axis start");
            CommandOutput::Modify {
                transform: EntityTransform::Mirror {
                    axis_start,
                    axis_end: point,
                },
                copies: true,
            }
        }
        ModifyPhase::ScaleFactor => {
            let base = state.base.expect("base point");
            let factor = typed_factor
                .unwrap_or_else(|| base.distance(point) / state.reference_radius.max(1.0));
            CommandOutput::Modify {
                transform: EntityTransform::UniformScale { base, factor },
                copies: false,
            }
        }
    }
}

fn modify_preview_transform(
    state: &ModifyState,
    current: Point2,
    typed_angle_deg: Option<f64>,
    typed_factor: Option<f64>,
) -> Option<EntityTransform> {
    let base = state.base?;
    match state.phase {
        ModifyPhase::Destination => Some(EntityTransform::Translate {
            dx: current.x - base.x,
            dy: current.y - base.y,
        }),
        ModifyPhase::Angle => {
            let radians = typed_angle_deg
                .map(|deg| deg.to_radians())
                .unwrap_or_else(|| (current.y - base.y).atan2(current.x - base.x));
            Some(EntityTransform::Rotate { base, radians })
        }
        ModifyPhase::MirrorSecond => Some(EntityTransform::Mirror {
            axis_start: base,
            axis_end: current,
        }),
        ModifyPhase::ScaleFactor => {
            let factor = typed_factor
                .unwrap_or_else(|| current.distance(base) / state.reference_radius.max(1.0));
            Some(EntityTransform::UniformScale { base, factor })
        }
        ModifyPhase::Selecting | ModifyPhase::BasePoint => None,
    }
}

fn modify_prompt(state: &ModifyState) -> &'static str {
    match (state.kind, state.phase) {
        (ModifyKind::Erase, _) => "ERASE • Click objects to erase • Esc to finish",
        (ModifyKind::Move, ModifyPhase::Selecting) => "MOVE • Select objects",
        (ModifyKind::Move, ModifyPhase::BasePoint) => "MOVE • Specify base point",
        (ModifyKind::Move, _) => "MOVE • Specify destination point",
        (ModifyKind::Copy, ModifyPhase::Selecting) => "COPY • Select objects",
        (ModifyKind::Copy, ModifyPhase::BasePoint) => "COPY • Specify base point",
        (ModifyKind::Copy, _) => "COPY • Specify destination point",
        (ModifyKind::Rotate, ModifyPhase::Selecting) => "ROTATE • Select objects",
        (ModifyKind::Rotate, ModifyPhase::BasePoint) => "ROTATE • Specify base point",
        (ModifyKind::Rotate, _) => "ROTATE • Specify rotation angle",
        (ModifyKind::Mirror, ModifyPhase::Selecting) => "MIRROR • Select objects",
        (ModifyKind::Mirror, ModifyPhase::BasePoint) => {
            "MIRROR • Specify first point of mirror line"
        }
        (ModifyKind::Mirror, _) => "MIRROR • Specify second point of mirror line",
        (ModifyKind::Scale, ModifyPhase::Selecting) => "SCALE • Select objects",
        (ModifyKind::Scale, ModifyPhase::BasePoint) => "SCALE • Specify base point",
        (ModifyKind::Scale, _) => "SCALE • Specify scale factor",
    }
}

fn accept_circle_point(state: &mut CircleState, point: Point2) -> CommandOutput {
    match state.center {
        None => {
            state.center = Some(point);
            CommandOutput::None
        }
        Some(center) => {
            let radius = center.distance(point);
            if radius <= GEOM_TOLERANCE {
                return CommandOutput::Rejected("Radius is below the geometry tolerance");
            }
            CommandOutput::Geometry(circle_geometry(center, radius))
        }
    }
}

fn accept_arc_point(state: &mut ArcState, point: Point2) -> CommandOutput {
    match (state.start, state.mid) {
        (None, _) => {
            state.start = Some(point);
            CommandOutput::None
        }
        (Some(start), None) => {
            if start.distance(point) <= GEOM_TOLERANCE {
                return CommandOutput::Rejected("Points are too close to define an arc");
            }
            state.mid = Some(point);
            CommandOutput::None
        }
        (Some(start), Some(mid)) => match arc_from_three_points(start, mid, point) {
            Ok(arc) => CommandOutput::Geometry(arc_geometry(arc)),
            Err(err) => CommandOutput::Rejected(err.message()),
        },
    }
}

fn accept_rectangle_point(state: &mut RectangleState, point: Point2) -> CommandOutput {
    match state.first {
        None => {
            state.first = Some(point);
            CommandOutput::None
        }
        Some(first) => match rectangle_geometry(first, point) {
            Some(geometry) => CommandOutput::Geometry(geometry),
            None => CommandOutput::Rejected("Width and height must be greater than zero"),
        },
    }
}

fn accept_distance_point(state: &mut DistanceState, point: Point2) -> CommandOutput {
    match state.first {
        None => {
            state.first = Some(point);
            CommandOutput::None
        }
        Some(first) => match DistanceReport::between(first, point) {
            Some(report) => CommandOutput::Distance(report),
            None => CommandOutput::Rejected("Distance must be greater than zero"),
        },
    }
}

fn accept_angle_point(state: &mut AngleState, point: Point2) -> CommandOutput {
    match state {
        AngleState::Prompt | AngleState::FirstSegment { .. } => {
            CommandOutput::Rejected("Select a straight segment, or click empty space")
        }
        AngleState::ThreePoint {
            vertex: vertex @ None,
            ..
        } => {
            *vertex = Some(point);
            CommandOutput::None
        }
        AngleState::ThreePoint {
            vertex: Some(_),
            ray: ray @ None,
        } => {
            *ray = Some(point);
            CommandOutput::None
        }
        AngleState::ThreePoint {
            vertex: Some(vertex),
            ray: Some(ray),
        } => match cad_core::AngleMeasurement::from_directions(*vertex, *ray, point) {
            Some(angle) => CommandOutput::Measurement(MeasurementResult::Angle(angle)),
            None => CommandOutput::Rejected("Directions are coincident or zero-length"),
        },
    }
}

fn accept_area_point(state: &mut AreaState, point: Point2) -> CommandOutput {
    match state {
        AreaState::Prompt => CommandOutput::Rejected("Select a closed object or specify a point"),
        AreaState::Points { vertices } => {
            if vertices
                .last()
                .is_some_and(|last| last.distance(point) <= GEOM_TOLERANCE)
            {
                return CommandOutput::Rejected("Points must be distinct");
            }
            vertices.push(point);
            CommandOutput::None
        }
    }
}

fn preview_arc(state: &ArcState, current: Option<Point2>) -> Option<PreviewGeometry> {
    let start = state.start?;
    let mid = state.mid?;
    let end = current?;
    match arc_from_three_points(start, mid, end) {
        Ok(arc) => Some(PreviewGeometry::Arc {
            center: arc.center,
            radius: arc.radius,
            start_angle: arc.start_angle,
            end_angle: arc.end_angle,
        }),
        Err(_) => Some(PreviewGeometry::Polyline {
            vertices: vec![start, mid],
            next: Some(end),
            closed: false,
        }),
    }
}

pub fn line_geometry(start: Point2, end: Point2) -> Geometry {
    Geometry::Line {
        start: Point3::from_xy(start.x, start.y),
        end: Point3::from_xy(end.x, end.y),
    }
}

pub fn circle_geometry(center: Point2, radius: f64) -> Geometry {
    Geometry::Circle {
        center: Point3::from_xy(center.x, center.y),
        radius,
        extrusion: default_extrusion(),
    }
}

pub fn arc_geometry(arc: cad_core::ThreePointArc) -> Geometry {
    Geometry::Arc {
        center: Point3::from_xy(arc.center.x, arc.center.y),
        radius: arc.radius,
        start_angle: arc.start_angle,
        end_angle: arc.end_angle,
        extrusion: default_extrusion(),
    }
}

pub fn lwpolyline_geometry(points: &[Point2], closed: bool) -> Geometry {
    Geometry::LwPolyline {
        vertices: points
            .iter()
            .map(|point| PolyVertex {
                point: Point3::from_xy(point.x, point.y),
                bulge: 0.0,
            vertex_id: Default::default(),
        })
            .collect(),
        closed,
        extrusion: default_extrusion(),
        linetype_generation_continuous: false,
    }
}

pub fn rectangle_corners(first: Point2, opposite: Point2) -> [Point2; 4] {
    [
        first,
        Point2::new(opposite.x, first.y),
        opposite,
        Point2::new(first.x, opposite.y),
    ]
}

fn rectangle_preview(first: Point2, opposite: Point2) -> Option<PreviewGeometry> {
    if (opposite.x - first.x).abs() <= GEOM_TOLERANCE
        || (opposite.y - first.y).abs() <= GEOM_TOLERANCE
    {
        return None;
    }
    Some(PreviewGeometry::Rectangle {
        corners: rectangle_corners(first, opposite),
    })
}

pub fn rectangle_geometry(first: Point2, opposite: Point2) -> Option<Geometry> {
    if (opposite.x - first.x).abs() <= GEOM_TOLERANCE
        || (opposite.y - first.y).abs() <= GEOM_TOLERANCE
    {
        return None;
    }
    Some(lwpolyline_geometry(
        &rectangle_corners(first, opposite),
        true,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_point_arc_command_accepts_cw_and_ccw() {
        for mid in [Point2::new(0.0, 1.0), Point2::new(0.0, -1.0)] {
            let mut command = CommandState::Idle;
            command.start_arc();
            command.accept_point(Point2::new(1.0, 0.0));
            command.accept_point(mid);
            let CommandOutput::Geometry(Geometry::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                extrusion,
            }) = command.accept_point(Point2::new(-1.0, 0.0))
            else {
                panic!("expected arc for mid {mid:?}");
            };
            assert!((center.x).abs() < 1e-8);
            assert!((center.y).abs() < 1e-8);
            assert!((radius - 1.0).abs() < 1e-8);
            assert_eq!(extrusion, default_extrusion());
            let arc = cad_core::ThreePointArc {
                center: center.xy(),
                radius,
                start_angle,
                end_angle,
            };
            assert!(arc.contains_point(mid, 1e-6));
            assert_eq!(command, CommandState::Idle);
        }
    }

    fn is_line(output: &CommandOutput, start: Point2, end: Point2) -> bool {
        matches!(
            output,
            CommandOutput::Geometry(Geometry::Line { start: s, end: e })
                if s.xy() == start && e.xy() == end
        )
    }

    #[test]
    fn line_command_emits_each_completed_segment() {
        let mut command = CommandState::Idle;
        command.start_line();
        assert_eq!(command.prompt(), "LINE • Specify first point");
        assert!(matches!(
            command.accept_point(Point2::new(1.0, 2.0)),
            CommandOutput::None
        ));
        assert_eq!(command.prompt(), "LINE • Specify second point");
        assert!(is_line(
            &command.accept_point(Point2::new(5.0, 2.0)),
            Point2::new(1.0, 2.0),
            Point2::new(5.0, 2.0)
        ));
        assert_eq!(command.base_point(), None);
        assert_eq!(command.kind(), CommandKind::Line);
        assert_eq!(command.prompt(), "LINE • Specify first point");
        assert!(command.preview(Some(Point2::new(9.0, 2.0))).is_none());
    }

    #[test]
    fn line_resets_after_commit_and_does_not_chain() {
        let mut command = CommandState::Idle;
        command.start_line();
        assert!(matches!(
            command.accept_point(Point2::new(0.0, 0.0)),
            CommandOutput::None
        ));
        assert!(is_line(
            &command.accept_point(Point2::new(10.0, 0.0)),
            Point2::new(0.0, 0.0),
            Point2::new(10.0, 0.0)
        ));
        assert_eq!(command.base_point(), None);
        assert!(matches!(
            command.accept_point(Point2::new(4.0, 6.0)),
            CommandOutput::None
        ));
        assert_eq!(command.base_point(), Some(Point2::new(4.0, 6.0)));
        assert!(is_line(
            &command.accept_point(Point2::new(4.0, 9.0)),
            Point2::new(4.0, 6.0),
            Point2::new(4.0, 9.0)
        ));
        assert_eq!(command.kind(), CommandKind::Line);
        assert_eq!(command.base_point(), None);
    }

    #[test]
    fn polyline_stays_continuous_after_each_vertex() {
        let mut command = CommandState::Idle;
        command.start_polyline();
        command.accept_point(Point2::new(0.0, 0.0));
        command.accept_point(Point2::new(3.0, 0.0));
        assert_eq!(command.base_point(), Some(Point2::new(3.0, 0.0)));
        command.accept_point(Point2::new(3.0, 4.0));
        assert_eq!(command.base_point(), Some(Point2::new(3.0, 4.0)));
        assert_eq!(command.kind(), CommandKind::Polyline);
        match command.finish_geometry() {
            Some(Geometry::LwPolyline {
                vertices, closed, ..
            }) => {
                assert!(!closed);
                assert_eq!(vertices.len(), 3);
            }
            other => panic!("expected polyline, got {other:?}"),
        }
    }

    #[test]
    fn escape_style_cancel_returns_to_idle() {
        let mut command = CommandState::Idle;
        command.start_line();
        command.cancel();
        assert_eq!(command, CommandState::Idle);
        assert_eq!(command.kind(), CommandKind::Idle);
    }

    #[test]
    fn distance_command_reports_two_points_then_idles() {
        let mut command = CommandState::Idle;
        command.start_distance();
        assert!(matches!(
            command.accept_point(Point2::new(0.0, 0.0)),
            CommandOutput::None
        ));
        let CommandOutput::Distance(report) = command.accept_point(Point2::new(3.0, 4.0)) else {
            panic!("expected distance report");
        };
        assert!((report.distance - 5.0).abs() < 1e-12);
        assert!((report.delta_x - 3.0).abs() < 1e-12);
        assert!((report.delta_y - 4.0).abs() < 1e-12);
        assert_eq!(command, CommandState::Idle);
    }

    #[test]
    fn line_preview_uses_last_accepted_point() {
        let mut command = CommandState::Idle;
        command.start_line();
        command.accept_point(Point2::new(1.0, 1.0));
        assert_eq!(
            command.preview(Some(Point2::new(2.0, 1.0))),
            Some(PreviewGeometry::LineSegment([
                Point2::new(1.0, 1.0),
                Point2::new(2.0, 1.0)
            ]))
        );
    }

    #[test]
    fn polyline_close_and_undo_keep_one_object() {
        let mut command = CommandState::Idle;
        command.start_polyline();
        assert!(matches!(
            command.accept_point(Point2::new(0.0, 0.0)),
            CommandOutput::None
        ));
        command.accept_point(Point2::new(2.0, 0.0));
        command.accept_point(Point2::new(2.0, 2.0));
        assert!(command.can_finish());
        assert!(command.can_close());
        assert!(command.can_undo_last());
        assert!(command.undo_last());
        assert!(!command.can_close());
        command.accept_point(Point2::new(0.0, 2.0));
        let geometry = command.close_geometry().expect("closed polyline");
        match geometry {
            Geometry::LwPolyline {
                vertices, closed, ..
            } => {
                assert!(closed);
                assert_eq!(vertices.len(), 3);
                assert!(vertices.iter().all(|vertex| vertex.bulge.abs() < 1e-15));
            }
            other => panic!("expected lwpolyline, got {other:?}"),
        }
        assert!(command.finish_geometry().is_some());
        command.cancel();
        assert_eq!(command, CommandState::Idle);
    }

    #[test]
    fn rectangle_commits_closed_four_vertex_polyline() {
        let mut command = CommandState::Idle;
        command.start_rectangle();
        command.accept_point(Point2::new(1.0, 2.0));
        let CommandOutput::Geometry(geometry) = command.accept_point(Point2::new(4.0, 6.0)) else {
            panic!("expected rectangle");
        };
        match geometry {
            Geometry::LwPolyline {
                vertices, closed, ..
            } => {
                assert!(closed);
                assert_eq!(vertices.len(), 4);
                assert_eq!(vertices[0].point.xy(), Point2::new(1.0, 2.0));
                assert_eq!(vertices[2].point.xy(), Point2::new(4.0, 6.0));
            }
            other => panic!("expected lwpolyline, got {other:?}"),
        }
        assert_eq!(command, CommandState::Idle);
    }

    #[test]
    fn rectangle_rejects_zero_height() {
        let mut command = CommandState::Idle;
        command.start_rectangle();
        command.accept_point(Point2::new(0.0, 0.0));
        assert!(matches!(
            command.accept_point(Point2::new(4.0, 0.0)),
            CommandOutput::Rejected(_)
        ));
        assert_eq!(command.kind(), CommandKind::Rectangle);
    }

    #[test]
    fn circle_center_radius_commits_world_z_circle() {
        let mut command = CommandState::Idle;
        command.start_circle();
        command.accept_point(Point2::new(3.0, 4.0));
        let CommandOutput::Geometry(Geometry::Circle {
            center,
            radius,
            extrusion,
        }) = command.accept_point(Point2::new(6.0, 4.0))
        else {
            panic!("expected circle");
        };
        assert_eq!(center.xy(), Point2::new(3.0, 4.0));
        assert!((radius - 3.0).abs() < 1e-12);
        assert_eq!(extrusion, default_extrusion());
        assert_eq!(command, CommandState::Idle);
    }

    #[test]
    fn circle_rejects_tiny_radius() {
        let mut command = CommandState::Idle;
        command.start_circle();
        command.accept_point(Point2::new(0.0, 0.0));
        assert!(matches!(
            command.accept_point(Point2::new(0.0, 0.0)),
            CommandOutput::Rejected(_)
        ));
        assert_eq!(command.kind(), CommandKind::Circle);
    }

    #[test]
    fn arc_back_and_reject_keep_command_active() {
        let mut command = CommandState::Idle;
        command.start_arc();
        command.accept_point(Point2::new(0.0, 0.0));
        command.accept_point(Point2::new(1.0, 0.0));
        assert!(command.can_back());
        assert!(matches!(
            command.accept_point(Point2::new(2.0, 0.0)),
            CommandOutput::Rejected(_)
        ));
        assert_eq!(command.kind(), CommandKind::Arc);
        assert!(command.back());
        assert!(command.back());
        assert!(!command.can_back());
    }

    #[test]
    fn preview_does_not_mutate_command_state() {
        let mut command = CommandState::Idle;
        command.start_line();
        command.accept_point(Point2::new(0.0, 0.0));
        let before = command.clone();
        let _ = command.preview(Some(Point2::new(4.0, 0.0)));
        assert_eq!(command, before);
    }

    #[test]
    fn angle_two_segments_then_idles() {
        let mut command = CommandState::Idle;
        command.start_angle();
        assert!(matches!(
            command.accept_straight_segment(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0)),
            CommandOutput::None
        ));
        let CommandOutput::Measurement(MeasurementResult::Angle(angle)) =
            command.accept_straight_segment(Point2::new(0.0, 0.0), Point2::new(0.0, 4.0))
        else {
            panic!("expected angle");
        };
        assert!((angle.angle - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        assert_eq!(command, CommandState::Idle);
    }

    #[test]
    fn area_point_boundary_finishes_without_geometry() {
        let mut command = CommandState::Idle;
        command.start_area();
        command.begin_area_points(Point2::new(0.0, 0.0));
        command.accept_point(Point2::new(4.0, 0.0));
        command.accept_point(Point2::new(4.0, 3.0));
        command.accept_point(Point2::new(0.0, 3.0));
        let result = command
            .finish_measurement()
            .expect("area finish")
            .expect("valid area");
        let MeasurementResult::Area(area) = result else {
            panic!("expected area");
        };
        assert!((area.area - 12.0).abs() < 1e-9);
        assert_eq!(command, CommandState::Idle);
    }

    #[test]
    fn command_snaps_exclude_base_and_include_completed_midpoints() {
        let mut command = CommandState::Idle;
        command.start_polyline();
        command.accept_point(Point2::new(0.0, 0.0));
        command.accept_point(Point2::new(4.0, 0.0));
        command.accept_point(Point2::new(4.0, 4.0));
        let mut snaps = vec![SnapFeature {
            point: Point2::new(99.0, 99.0),
            kind: SnapKind::Center,
        }];
        command.write_snap_features(&mut snaps);
        let base = command.base_point().expect("base");
        assert!(snaps
            .iter()
            .all(|feature| feature.point.distance(base) > GEOM_TOLERANCE));
        assert!(snaps.iter().any(|feature| {
            feature.kind == SnapKind::Endpoint && feature.point == Point2::new(0.0, 0.0)
        }));
        assert!(snaps.iter().any(|feature| {
            feature.kind == SnapKind::Endpoint && feature.point == Point2::new(4.0, 0.0)
        }));
        assert!(snaps.iter().any(|feature| {
            feature.kind == SnapKind::Midpoint && feature.point == Point2::new(2.0, 0.0)
        }));
        assert!(snaps.iter().any(|feature| {
            feature.kind == SnapKind::Midpoint && feature.point == Point2::new(4.0, 2.0)
        }));
        assert_eq!(snaps.len(), 4);
    }

    #[test]
    fn command_snaps_are_empty_before_a_completed_segment() {
        let mut command = CommandState::Idle;
        command.start_line();
        command.accept_point(Point2::new(1.0, 1.0));
        let mut snaps = vec![SnapFeature {
            point: Point2::new(1.0, 1.0),
            kind: SnapKind::Endpoint,
        }];
        command.write_snap_features(&mut snaps);
        assert!(snaps.is_empty());
        command.accept_point(Point2::new(4.0, 1.0));
        command.write_snap_features(&mut snaps);
        assert!(snaps.is_empty());
        assert_eq!(command.base_point(), None);
    }

    #[test]
    fn snapping_to_first_polyline_vertex_closes_without_duplicating() {
        let mut command = CommandState::Idle;
        command.start_polyline();
        let first = Point2::new(0.0, 0.0);
        command.accept_point(first);
        command.accept_point(Point2::new(4.0, 0.0));
        command.accept_point(Point2::new(4.0, 3.0));
        let CommandOutput::Geometry(Geometry::LwPolyline {
            vertices, closed, ..
        }) = command.accept_point(first)
        else {
            panic!("expected closed polyline");
        };
        assert!(closed);
        assert_eq!(vertices.len(), 3);
        assert_eq!(vertices[0].point.xy(), first);
        assert_eq!(command, CommandState::Idle);
    }

    #[test]
    fn two_vertex_polyline_does_not_close_on_start_snap() {
        let mut command = CommandState::Idle;
        command.start_polyline();
        let first = Point2::new(0.0, 0.0);
        command.accept_point(first);
        command.accept_point(Point2::new(5.0, 0.0));
        assert!(matches!(command.accept_point(first), CommandOutput::None));
        assert_eq!(command.kind(), CommandKind::Polyline);
        match command.finish_geometry() {
            Some(Geometry::LwPolyline {
                vertices, closed, ..
            }) => {
                assert!(!closed);
                assert_eq!(vertices.len(), 3);
            }
            other => panic!("expected open polyline, got {other:?}"),
        }
        assert!(!command.can_close());
    }

    #[test]
    fn explicit_close_sets_flag_without_appending_start() {
        let mut command = CommandState::Idle;
        command.start_polyline();
        command.accept_point(Point2::new(0.0, 0.0));
        command.accept_point(Point2::new(1.0, 0.0));
        command.accept_point(Point2::new(1.0, 1.0));
        match command.close_geometry() {
            Some(Geometry::LwPolyline {
                vertices, closed, ..
            }) => {
                assert!(closed);
                assert_eq!(vertices.len(), 3);
                assert_eq!(vertices[0].point.xy(), Point2::new(0.0, 0.0));
                assert_ne!(vertices.last().unwrap().point.xy(), vertices[0].point.xy());
            }
            other => panic!("expected closed polyline, got {other:?}"),
        }
        assert_eq!(command.kind(), CommandKind::Polyline);
    }

    #[test]
    fn modify_selection_first_asks_for_base_point() {
        let mut command = CommandState::Idle;
        command.start_modify(ModifyKind::Move, vec![EntityId(3)]);
        assert!(!command.is_selecting_objects());
        assert!(command.requests_point());
        assert_eq!(command.modify_targets(), &[EntityId(3)]);
        assert!(matches!(
            command.accept_modify_point(Point2::new(1.0, 1.0), None, None),
            CommandOutput::None
        ));
        let CommandOutput::Modify {
            transform: EntityTransform::Translate { dx, dy },
            copies,
        } = command.accept_modify_point(Point2::new(4.0, 5.0), None, None)
        else {
            panic!("expected move");
        };
        assert!((dx - 3.0).abs() < 1e-12);
        assert!((dy - 4.0).abs() < 1e-12);
        assert!(!copies);
        assert_eq!(command.kind(), CommandKind::Move);
    }

    #[test]
    fn modify_command_first_enters_selection_phase() {
        let mut command = CommandState::Idle;
        command.start_modify(ModifyKind::Copy, Vec::new());
        assert!(command.is_selecting_objects());
        assert!(!command.requests_point());
        command.confirm_modify_selection(vec![EntityId(1), EntityId(2)], 10.0);
        assert!(!command.is_selecting_objects());
        assert!(command.requests_point());
    }

    #[test]
    fn copy_and_mirror_request_copies() {
        let mut command = CommandState::Idle;
        command.start_modify(ModifyKind::Copy, vec![EntityId(1)]);
        command.accept_modify_point(Point2::new(0.0, 0.0), None, None);
        let CommandOutput::Modify { copies, .. } =
            command.accept_modify_point(Point2::new(1.0, 0.0), None, None)
        else {
            panic!("copy");
        };
        assert!(copies);
        command.start_modify(ModifyKind::Mirror, vec![EntityId(1)]);
        command.accept_modify_point(Point2::new(0.0, 0.0), None, None);
        let CommandOutput::Modify { copies, .. } =
            command.accept_modify_point(Point2::new(0.0, 1.0), None, None)
        else {
            panic!("mirror");
        };
        assert!(copies);
    }

    #[test]
    fn esc_style_modify_cancel_returns_idle() {
        let mut command = CommandState::Idle;
        command.start_modify(ModifyKind::Rotate, vec![EntityId(8)]);
        command.cancel();
        assert_eq!(command, CommandState::Idle);
    }

    #[test]
    fn erase_stays_in_selection_phase_even_with_preselection() {
        let mut command = CommandState::Idle;
        command.start_modify(ModifyKind::Erase, vec![EntityId(4)]);
        assert!(command.is_selecting_objects());
        assert!(!command.requests_point());
    }

    #[test]
    fn rotate_uses_signed_typed_angle() {
        let mut command = CommandState::Idle;
        command.start_modify(ModifyKind::Rotate, vec![EntityId(1)]);
        command.accept_modify_point(Point2::new(0.0, 0.0), None, None);
        let CommandOutput::Modify {
            transform: EntityTransform::Rotate { radians, .. },
            copies,
        } = command.accept_modify_point(Point2::new(10.0, 0.0), Some(-90.0), None)
        else {
            panic!("rotate");
        };
        assert!((radians + std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        assert!(!copies);
    }

    #[test]
    fn scale_prefers_typed_factor_over_cursor() {
        let mut command = CommandState::Idle;
        command.start_modify(ModifyKind::Scale, vec![EntityId(1)]);
        command.accept_modify_point(Point2::new(0.0, 0.0), None, None);
        let CommandOutput::Modify {
            transform: EntityTransform::UniformScale { factor, .. },
            copies,
        } = command.accept_modify_point(Point2::new(100.0, 0.0), None, Some(2.5))
        else {
            panic!("scale");
        };
        assert!((factor - 2.5).abs() < 1e-12);
        assert!(!copies);
    }
}
