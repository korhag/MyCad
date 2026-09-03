//! Drafting aids shared by point-based commands and the status bar.

use cad_core::{Extents2, Point2, SnapFeature, SnapIndex, SnapKind};
use cad_viewport::Camera2;
use eframe::egui::{self, Color32, Pos2, Rect, Stroke};
use serde::{Deserialize, Serialize};

pub const SNAP_APERTURE_PX: f64 = 9.0;
const SNAP_MARKER_RADIUS: f32 = 6.0;

// ------------------------------------------------------------
// Type: RunningSnaps
// Purpose: Persistent set of semantic object-snap modes.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunningSnaps {
    pub endpoint: bool,
    pub midpoint: bool,
    pub center: bool,
}

impl Default for RunningSnaps {
    fn default() -> Self {
        Self {
            endpoint: true,
            midpoint: true,
            center: true,
        }
    }
}

impl RunningSnaps {
    fn contains(self, kind: SnapKind) -> bool {
        match kind {
            SnapKind::Endpoint => self.endpoint,
            SnapKind::Midpoint => self.midpoint,
            SnapKind::Center => self.center,
        }
    }
}

// ------------------------------------------------------------
// Type: DraftingPreferences
// Purpose: Drafting switches persisted with AppSettings.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DraftingPreferences {
    pub ortho_enabled: bool,
    pub osnap_enabled: bool,
    pub running_snaps: RunningSnaps,
}

impl Default for DraftingPreferences {
    fn default() -> Self {
        Self {
            ortho_enabled: false,
            osnap_enabled: true,
            running_snaps: RunningSnaps::default(),
        }
    }
}

// ------------------------------------------------------------
// Type: DraftingState
// Purpose: Runtime point acquisition state for active commands.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct DraftingState {
    pub preferences: DraftingPreferences,
    pub acquired_snap: Option<SnapFeature>,
    pub current_point: Option<Point2>,
    pub command_base_point: Option<Point2>,
    nearby: Vec<SnapFeature>,
}

impl DraftingState {
    pub fn new(preferences: DraftingPreferences) -> Self {
        Self {
            preferences,
            acquired_snap: None,
            current_point: None,
            command_base_point: None,
            nearby: Vec::new(),
        }
    }

    pub fn clear_acquisition(&mut self) {
        self.acquired_snap = None;
        self.current_point = None;
        self.command_base_point = None;
    }

    pub fn resolve_point(
        &mut self,
        raw: Point2,
        base: Option<Point2>,
        shift_held: bool,
        camera: &Camera2,
        viewport_height: f64,
        snaps: &SnapIndex,
    ) -> Point2 {
        self.command_base_point = base;
        self.acquired_snap = None;
        let world_aperture = SNAP_APERTURE_PX / camera.pixels_per_world(viewport_height).max(1e-15);
        if self.preferences.osnap_enabled && !snaps.is_empty() {
            let region = Extents2::from_corners(
                Point2::new(raw.x - world_aperture, raw.y - world_aperture),
                Point2::new(raw.x + world_aperture, raw.y + world_aperture),
            );
            snaps.query(region, &mut self.nearby);
            self.acquired_snap = self
                .nearby
                .iter()
                .filter(|feature| self.preferences.running_snaps.contains(feature.kind))
                .filter_map(|feature| {
                    let distance = raw.distance(feature.point);
                    (distance <= world_aperture).then_some((*feature, distance))
                })
                .min_by(|(_, left), (_, right)| left.total_cmp(right))
                .map(|(feature, _)| feature);
        }

        // An exact semantic snap takes precedence. Otherwise ORTHO constrains
        // to the dominant axis, with Shift temporarily reversing stored state.
        let resolved = if let Some(feature) = self.acquired_snap {
            feature.point
        } else if self.preferences.ortho_enabled ^ shift_held {
            base.map(|base| constrain_ortho(base, raw)).unwrap_or(raw)
        } else {
            raw
        };
        self.current_point = Some(resolved);
        resolved
    }
}

pub fn constrain_ortho(base: Point2, point: Point2) -> Point2 {
    let dx = (point.x - base.x).abs();
    let dy = (point.y - base.y).abs();
    if dx >= dy {
        Point2::new(point.x, base.y)
    } else {
        Point2::new(base.x, point.y)
    }
}

pub fn paint_overlay(
    painter: &egui::Painter,
    rect: Rect,
    camera: Camera2,
    preview: Option<[Point2; 2]>,
    acquired_snap: Option<SnapFeature>,
) {
    let origin = Point2::new(rect.min.x as f64, rect.min.y as f64);
    let size = Point2::new(rect.width() as f64, rect.height() as f64);
    let to_screen = |point: Point2| {
        let point = camera.world_to_screen(point, origin, size);
        Pos2::new(point.x as f32, point.y as f32)
    };

    if let Some([start, end]) = preview {
        painter.line_segment(
            [to_screen(start), to_screen(end)],
            Stroke::new(1.5, Color32::from_rgb(235, 235, 235)),
        );
    }
    if let Some(feature) = acquired_snap {
        paint_snap_marker(painter, to_screen(feature.point), feature.kind);
    }
}

fn paint_snap_marker(painter: &egui::Painter, center: Pos2, kind: SnapKind) {
    let color = Color32::from_rgb(80, 230, 220);
    let stroke = Stroke::new(1.5, color);
    let radius = SNAP_MARKER_RADIUS;
    match kind {
        SnapKind::Endpoint => {
            painter.rect_stroke(
                Rect::from_center_size(center, egui::vec2(radius * 2.0, radius * 2.0)),
                0.0,
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        SnapKind::Midpoint => {
            let points = vec![
                Pos2::new(center.x, center.y - radius),
                Pos2::new(center.x - radius, center.y + radius),
                Pos2::new(center.x + radius, center.y + radius),
            ];
            painter.add(egui::Shape::closed_line(points, stroke));
        }
        SnapKind::Center => {
            painter.circle_stroke(center, radius, stroke);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ortho_uses_dominant_axis() {
        let base = Point2::new(1.0, 2.0);
        assert_eq!(
            constrain_ortho(base, Point2::new(10.0, 5.0)),
            Point2::new(10.0, 2.0)
        );
        assert_eq!(
            constrain_ortho(base, Point2::new(3.0, 20.0)),
            Point2::new(1.0, 20.0)
        );
    }

    #[test]
    fn shift_reverses_ortho_only_during_resolution() {
        let mut drafting = DraftingState::new(DraftingPreferences {
            ortho_enabled: true,
            osnap_enabled: false,
            ..DraftingPreferences::default()
        });
        let camera = Camera2::default();
        let base = Point2::new(0.0, 0.0);
        let raw = Point2::new(10.0, 3.0);
        let constrained = drafting.resolve_point(
            raw,
            Some(base),
            false,
            &camera,
            600.0,
            &SnapIndex::default(),
        );
        let reversed =
            drafting.resolve_point(raw, Some(base), true, &camera, 600.0, &SnapIndex::default());
        assert_eq!(constrained, Point2::new(10.0, 0.0));
        assert_eq!(reversed, raw);
    }

    #[test]
    fn snap_aperture_stays_constant_in_screen_pixels() {
        let snap = SnapFeature {
            point: Point2::new(0.0, 0.0),
            kind: SnapKind::Endpoint,
        };
        let index = SnapIndex::from_features(vec![snap]);
        let mut drafting = DraftingState::new(DraftingPreferences::default());
        let far_camera = Camera2 {
            center: Point2::new(0.0, 0.0),
            view_height: 100.0,
        };
        let near_camera = Camera2 {
            center: Point2::new(0.0, 0.0),
            view_height: 10.0,
        };
        assert_eq!(
            drafting.resolve_point(
                Point2::new(0.8, 0.0),
                None,
                false,
                &far_camera,
                1000.0,
                &index,
            ),
            snap.point
        );
        assert_eq!(
            drafting.resolve_point(
                Point2::new(0.08, 0.0),
                None,
                false,
                &near_camera,
                1000.0,
                &index,
            ),
            snap.point
        );
    }
}
