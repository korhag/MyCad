//! Transient inspection overlay shared by Distance, Angle, Radius, and Area.

use cad_core::{
    bulge_circle, DrawingUnits, MeasurementResult, Point2, PolyVertex,
};
use cad_viewport::Camera2;
use eframe::egui::{
    self, Align2, Area, Color32, FontId, Frame, Id, Order, Pos2, Rect, Shape, Stroke, Vec2,
};

pub const ACCENT: Color32 = Color32::from_rgb(232, 176, 64);
const STROKE_PX: f32 = 1.5;
const MARKER_PX: f32 = 5.0;
const ARC_PX: f32 = 28.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardAction {
    None,
    Copy,
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeasurementOverlay {
    pub result: MeasurementResult,
    pub live: bool,
}

impl MeasurementOverlay {
    pub fn final_result(result: MeasurementResult) -> Self {
        Self {
            result,
            live: false,
        }
    }

    pub fn live(result: MeasurementResult) -> Self {
        Self { result, live: true }
    }
}

pub fn world_aperture(camera: &Camera2, viewport_height: f64) -> f64 {
    cad_core::MEASURE_APERTURE_PX / camera.pixels_per_world(viewport_height).max(1e-9)
}

pub fn paint(
    painter: &egui::Painter,
    rect: Rect,
    camera: Camera2,
    overlay: &MeasurementOverlay,
    units: DrawingUnits,
) {
    let origin = Point2::new(rect.min.x as f64, rect.min.y as f64);
    let size = Point2::new(rect.width() as f64, rect.height() as f64);
    let to_screen = |point: Point2| {
        let point = camera.world_to_screen(point, origin, size);
        Pos2::new(point.x as f32, point.y as f32)
    };
    let text = overlay.result.format(units);
    let stroke = Stroke::new(STROKE_PX, ACCENT);
    match &overlay.result {
        MeasurementResult::Distance(m) => {
            let a = to_screen(m.start);
            let b = to_screen(m.end);
            painter.line_segment([a, b], stroke);
            paint_marker(painter, a);
            paint_marker(painter, b);
            if !overlay.live {
                paint_label(painter, rect, a.lerp(b, 0.5), &text.primary);
            }
        }
        MeasurementResult::Angle(m) => {
            let vertex = to_screen(m.vertex);
            let dir_a = screen_dir(vertex, to_screen(m.ray_a));
            let dir_b = screen_dir(vertex, to_screen(m.ray_b));
            painter.line_segment([vertex, vertex + dir_a * 72.0], stroke);
            painter.line_segment([vertex, vertex + dir_b * 72.0], stroke);
            paint_marker(painter, vertex);
            paint_angle_arc(painter, vertex, dir_a, dir_b, m.angle);
            let mid = (dir_a + dir_b).normalized() * (ARC_PX + 10.0);
            let fallback = rotate(dir_a, m.angle as f32 * 0.5) * (ARC_PX + 10.0);
            let label_off = if mid.length() > 1.0 { mid } else { fallback };
            paint_label(painter, rect, vertex + label_off, &text.primary);
        }
        MeasurementResult::Radius(m) => {
            let center = to_screen(m.center);
            let rim = to_screen(m.toward);
            let screen_radius = center.distance(rim).max(1.0);
            painter.circle_stroke(center, screen_radius, stroke);
            painter.line_segment([center, rim], stroke);
            paint_marker(painter, center);
            painter.circle_stroke(center, MARKER_PX * 0.7, stroke);
            paint_label(painter, rect, center.lerp(rim, 0.62), &text.primary);
        }
        MeasurementResult::Area(m) => {
            let points = sample_loop(&m.vertices, to_screen);
            if points.len() >= 3 {
                painter.add(Shape::Path(egui::epaint::PathShape {
                    points: points.clone(),
                    closed: true,
                    fill: Color32::from_rgba_unmultiplied(232, 176, 64, 36),
                    stroke: egui::epaint::PathStroke::new(STROKE_PX, ACCENT),
                }));
            }
            paint_label(painter, rect, to_screen(m.centroid), &text.primary);
        }
    }
}

pub fn show_card(
    ctx: &egui::Context,
    viewport: Rect,
    overlay: &MeasurementOverlay,
    units: DrawingUnits,
) -> (CardAction, bool) {
    if overlay.live {
        return (CardAction::None, false);
    }
    let text = overlay.result.format(units);
    let mut action = CardAction::None;
    let response = Area::new(Id::new("mycad-measure-card"))
        .order(Order::Foreground)
        .fixed_pos(Pos2::new(viewport.min.x + 10.0, viewport.max.y - 12.0))
        .pivot(Align2::LEFT_BOTTOM)
        .constrain_to(viewport)
        .show(ctx, |ui| {
            Frame::popup(ui.style())
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.set_max_width(260.0);
                    ui.label(egui::RichText::new(&text.primary).strong().color(ACCENT));
                    for line in &text.details {
                        ui.label(egui::RichText::new(line).small());
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.button("Copy").clicked() {
                            ui.ctx().copy_text(text.clipboard.clone());
                            action = CardAction::Copy;
                        }
                        if ui.button("Close").clicked() {
                            action = CardAction::Close;
                        }
                    });
                });
        });
    (action, response.response.contains_pointer())
}

pub fn live_cursor_label(painter: &egui::Painter, rect: Rect, cursor: Pos2, text: &str) {
    paint_label(painter, rect, cursor + Vec2::new(14.0, -10.0), text);
}

fn paint_marker(painter: &egui::Painter, center: Pos2) {
    let r = MARKER_PX;
    painter.line_segment(
        [Pos2::new(center.x - r, center.y), Pos2::new(center.x + r, center.y)],
        Stroke::new(STROKE_PX, ACCENT),
    );
    painter.line_segment(
        [Pos2::new(center.x, center.y - r), Pos2::new(center.x, center.y + r)],
        Stroke::new(STROKE_PX, ACCENT),
    );
}

fn paint_label(painter: &egui::Painter, viewport: Rect, pos: Pos2, text: &str) {
    let font = FontId::monospace(12.0);
    let galley = painter.layout_no_wrap(text.to_string(), font, Color32::from_rgb(250, 236, 200));
    let size = galley.size() + Vec2::new(10.0, 6.0);
    let mut min = pos - Vec2::new(size.x * 0.5, size.y * 0.5);
    min.x = min.x.clamp(viewport.min.x + 4.0, (viewport.max.x - size.x - 4.0).max(viewport.min.x + 4.0));
    min.y = min.y.clamp(viewport.min.y + 4.0, (viewport.max.y - size.y - 4.0).max(viewport.min.y + 4.0));
    let rect = Rect::from_min_size(min, size);
    painter.rect_filled(rect, 3.0, Color32::from_rgba_unmultiplied(12, 16, 14, 200));
    painter.galley(rect.center() - galley.size() * 0.5, galley, Color32::WHITE);
}

fn paint_angle_arc(painter: &egui::Painter, vertex: Pos2, dir_a: Vec2, dir_b: Vec2, included: f64) {
    let start = dir_a.angle();
    let cross = dir_a.x * dir_b.y - dir_a.y * dir_b.x;
    let sign = if cross < 0.0 { -1.0 } else { 1.0 };
    let sweep = included as f32 * sign;
    let mut points = Vec::new();
    let steps = 16;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let a = start + sweep * t;
        points.push(vertex + Vec2::angled(a) * ARC_PX);
    }
    for pair in points.windows(2) {
        painter.line_segment([pair[0], pair[1]], Stroke::new(STROKE_PX, ACCENT));
    }
}

fn screen_dir(from: Pos2, to: Pos2) -> Vec2 {
    let delta = to - from;
    if delta.length() <= 1e-3 {
        Vec2::X
    } else {
        delta.normalized()
    }
}

fn rotate(v: Vec2, radians: f32) -> Vec2 {
    let (s, c) = radians.sin_cos();
    Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

fn sample_loop(vertices: &[PolyVertex], to_screen: impl Fn(Point2) -> Pos2) -> Vec<Pos2> {
    let n = vertices.len();
    if n < 2 {
        return Vec::new();
    }
    let mut points = Vec::new();
    for i in 0..n {
        let a = vertices[i];
        let b = vertices[(i + 1) % n];
        let start = a.point.xy();
        let end = b.point.xy();
        points.push(to_screen(start));
        if let Some(arc) = bulge_circle(start, end, a.bulge) {
            let steps = 12.max((arc.sweep.abs() / 0.25).ceil() as usize);
            for s in 1..steps {
                let t = s as f64 / steps as f64;
                let ang = arc.start_angle + arc.sweep * t;
                points.push(to_screen(Point2::new(
                    arc.center.x + arc.radius * ang.cos(),
                    arc.center.y + arc.radius * ang.sin(),
                )));
            }
        }
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_core::{DistanceMeasurement, Point2};

    #[test]
    fn overlay_keeps_raw_result_not_preformatted_strings() {
        let overlay = MeasurementOverlay::final_result(MeasurementResult::Distance(
            DistanceMeasurement::between(Point2::new(0.0, 0.0), Point2::new(3.0, 4.0)).unwrap(),
        ));
        let text = overlay.result.format(DrawingUnits::Millimeters);
        assert_eq!(text.primary, "5.0000 mm");
        assert!(text.clipboard.contains("ΔX 3.0000 mm"));
    }
}
