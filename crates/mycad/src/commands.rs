//! Interactive command lifecycle and point consumption.

use cad_core::{DistanceReport, Point2};

// ------------------------------------------------------------
// Enum: CommandOutput
// Purpose: Geometry or measurement produced when a command
//          consumes an accepted point.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CommandOutput {
    LineSegment([Point2; 2]),
    Distance(DistanceReport),
}

// ------------------------------------------------------------
// Type: CommandState
// Purpose: Routes viewport points to the active command before
//          idle selection sees them.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum CommandState {
    #[default]
    Idle,
    LineWaitingForFirstPoint,
    LineWaitingForNextPoint {
        last: Point2,
    },
    DistanceWaitingForFirstPoint,
    DistanceWaitingForSecondPoint {
        first: Point2,
    },
}

impl CommandState {
    pub fn is_active(self) -> bool {
        !matches!(self, Self::Idle)
    }

    pub fn requests_point(self) -> bool {
        !matches!(self, Self::Idle)
    }

    pub fn start_line(&mut self) {
        *self = Self::LineWaitingForFirstPoint;
    }

    pub fn start_distance(&mut self) {
        *self = Self::DistanceWaitingForFirstPoint;
    }

    pub fn cancel(&mut self) {
        *self = Self::Idle;
    }

    pub fn finish(&mut self) {
        *self = Self::Idle;
    }

    pub fn base_point(self) -> Option<Point2> {
        match self {
            Self::LineWaitingForNextPoint { last } => Some(last),
            Self::DistanceWaitingForSecondPoint { first } => Some(first),
            _ => None,
        }
    }

    pub fn accept_point(&mut self, point: Point2) -> Option<CommandOutput> {
        match *self {
            Self::Idle => None,
            Self::LineWaitingForFirstPoint => {
                *self = Self::LineWaitingForNextPoint { last: point };
                None
            }
            Self::LineWaitingForNextPoint { last } => {
                *self = Self::LineWaitingForNextPoint { last: point };
                (last.distance(point) > 1e-12).then_some(CommandOutput::LineSegment([last, point]))
            }
            Self::DistanceWaitingForFirstPoint => {
                *self = Self::DistanceWaitingForSecondPoint { first: point };
                None
            }
            Self::DistanceWaitingForSecondPoint { first } => {
                *self = Self::Idle;
                (first.distance(point) > 1e-12).then_some(CommandOutput::Distance(
                    DistanceReport::between(first, point),
                ))
            }
        }
    }

    pub fn preview(self, current: Option<Point2>) -> Option<[Point2; 2]> {
        Some([self.base_point()?, current?])
    }

    pub fn prompt(self) -> &'static str {
        match self {
            Self::Idle => "Command: Ready",
            Self::LineWaitingForFirstPoint => "LINE — Specify first point",
            Self::LineWaitingForNextPoint { .. } => {
                "LINE — Specify next point or press Enter to finish"
            }
            Self::DistanceWaitingForFirstPoint => "DIST — Specify first point",
            Self::DistanceWaitingForSecondPoint { .. } => "DIST — Specify second point",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_command_emits_each_completed_segment() {
        let mut command = CommandState::Idle;
        command.start_line();
        assert_eq!(command.accept_point(Point2::new(1.0, 2.0)), None);
        assert_eq!(
            command.accept_point(Point2::new(5.0, 2.0)),
            Some(CommandOutput::LineSegment([
                Point2::new(1.0, 2.0),
                Point2::new(5.0, 2.0)
            ]))
        );
        assert_eq!(command.base_point(), Some(Point2::new(5.0, 2.0)));
    }

    #[test]
    fn escape_style_cancel_returns_to_idle() {
        let mut command = CommandState::LineWaitingForFirstPoint;
        command.cancel();
        assert_eq!(command, CommandState::Idle);
    }

    #[test]
    fn distance_command_reports_two_points_then_idles() {
        let mut command = CommandState::Idle;
        command.start_distance();
        assert_eq!(command.accept_point(Point2::new(0.0, 0.0)), None);
        let Some(CommandOutput::Distance(report)) = command.accept_point(Point2::new(3.0, 4.0))
        else {
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
            Some([Point2::new(1.0, 1.0), Point2::new(2.0, 1.0)])
        );
    }
}
