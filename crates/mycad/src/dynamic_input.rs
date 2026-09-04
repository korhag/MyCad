//! Compact numeric fields painted beside the cursor during drawing.

use cad_core::{Point2, GEOM_TOLERANCE};
use eframe::egui::{self, Color32, FontId, Pos2, Rect, Stroke, Vec2};

// ------------------------------------------------------------
// Enum: DynamicLayout
// Purpose: Which numeric fields a drawing stage exposes.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DynamicLayout {
    #[default]
    Hidden,
    LengthAngle,
    Radius,
    WidthHeight,
    Angle,
    Factor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    Length,
    Angle,
    Radius,
    Width,
    Height,
    Factor,
}

impl FieldKind {
    fn label(self) -> &'static str {
        match self {
            Self::Length => "Length",
            Self::Angle => "Angle",
            Self::Radius => "Radius",
            Self::Width => "Width",
            Self::Height => "Height",
            Self::Factor => "Factor",
        }
    }

    fn allows_minus(self) -> bool {
        matches!(self, Self::Angle)
    }

    fn must_be_positive(self) -> bool {
        !matches!(self, Self::Angle)
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Field {
    kind: FieldKind,
    buffer: String,
    locked: bool,
    typed: bool,
}

impl Field {
    fn new(kind: FieldKind) -> Self {
        Self {
            kind,
            buffer: String::new(),
            locked: false,
            typed: false,
        }
    }

    fn reset(&mut self) {
        self.buffer.clear();
        self.locked = false;
        self.typed = false;
    }

    fn parsed(&self) -> Option<f64> {
        parse_number(&self.buffer).ok()
    }

    fn constrained_value(&self) -> Option<f64> {
        if self.locked || self.typed {
            self.parsed()
        } else {
            None
        }
    }

    fn display_text(&self, live: f64) -> String {
        if self.typed || self.locked {
            if self.buffer.is_empty() {
                String::new()
            } else {
                self.buffer.clone()
            }
        } else {
            format_live(live)
        }
    }
}

// ------------------------------------------------------------
// Enum: DynamicKeyResult
// Purpose: Outcome of routing a keystroke through numeric input
//          before command shortcuts see it.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicKeyResult {
    None,
    Handled,
    Submit,
    FinishEmpty,
    Invalid(&'static str),
}

// ------------------------------------------------------------
// Type: DynamicInput
// Purpose: Numeric state plus the compact viewport overlay.
// ------------------------------------------------------------
#[derive(Debug, Clone, Default)]
pub struct DynamicInput {
    layout: DynamicLayout,
    fields: Vec<Field>,
    active: usize,
}

impl DynamicInput {
    pub fn layout(&self) -> DynamicLayout {
        self.layout
    }

    pub fn is_active(&self) -> bool {
        !matches!(self.layout, DynamicLayout::Hidden)
    }

    pub fn set_layout(&mut self, layout: DynamicLayout) {
        if self.layout == layout {
            return;
        }
        self.layout = layout;
        self.fields = fields_for(layout);
        self.active = 0;
    }

    pub fn reset_values(&mut self) {
        for field in &mut self.fields {
            field.reset();
        }
        self.active = 0;
    }

    pub fn has_typed_or_locked(&self) -> bool {
        self.fields
            .iter()
            .any(|field| field.locked || (field.typed && !field.buffer.is_empty()))
    }

    pub fn constrain(&self, base: Option<Point2>, resolved: Point2) -> Point2 {
        let Some(base) = base else {
            return resolved;
        };
        match self.layout {
            DynamicLayout::Hidden => resolved,
            DynamicLayout::LengthAngle => constrain_length_angle(&self.fields, base, resolved),
            DynamicLayout::Radius => constrain_radius(&self.fields, base, resolved),
            DynamicLayout::WidthHeight => constrain_width_height(&self.fields, base, resolved),
            DynamicLayout::Angle => constrain_angle_only(&self.fields, base, resolved),
            DynamicLayout::Factor => resolved,
        }
    }

    pub fn typed_angle_deg(&self) -> Option<f64> {
        field_value(&self.fields, FieldKind::Angle)
    }

    pub fn typed_factor(&self) -> Option<f64> {
        field_value(&self.fields, FieldKind::Factor)
    }

    pub fn consume(
        &mut self,
        input: &mut egui::InputState,
        live: LiveValues,
        finish_on_empty_enter: bool,
    ) -> DynamicKeyResult {
        if !self.is_active() {
            return DynamicKeyResult::None;
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
            || input.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab)
        {
            return self.tab(live);
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
            return self.enter(finish_on_empty_enter);
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace) {
            self.backspace();
            return DynamicKeyResult::Handled;
        }

        let mut handled = false;
        let mut invalid = None;
        input.events.retain(|event| {
            let egui::Event::Text(text) = event else {
                return true;
            };
            match self.insert_text(text) {
                Ok(consumed) => {
                    if consumed {
                        handled = true;
                        false
                    } else {
                        true
                    }
                }
                Err(message) => {
                    invalid = Some(message);
                    handled = true;
                    false
                }
            }
        });
        if let Some(message) = invalid {
            DynamicKeyResult::Invalid(message)
        } else if handled {
            DynamicKeyResult::Handled
        } else {
            DynamicKeyResult::None
        }
    }

    fn tab(&mut self, live: LiveValues) -> DynamicKeyResult {
        if self.fields.is_empty() {
            return DynamicKeyResult::Handled;
        }
        match self.lock_active(live) {
            Ok(()) => {
                self.active = (self.active + 1) % self.fields.len();
                DynamicKeyResult::Handled
            }
            Err(message) => DynamicKeyResult::Invalid(message),
        }
    }

    fn enter(&mut self, finish_on_empty_enter: bool) -> DynamicKeyResult {
        if !self.has_typed_or_locked() {
            if finish_on_empty_enter {
                return DynamicKeyResult::FinishEmpty;
            }
            return DynamicKeyResult::Submit;
        }
        for field in &self.fields {
            if field.typed || field.locked {
                if let Err(message) = validate_field(field) {
                    return DynamicKeyResult::Invalid(message);
                }
            }
        }
        DynamicKeyResult::Submit
    }

    fn lock_active(&mut self, live: LiveValues) -> Result<(), &'static str> {
        let Some(field) = self.fields.get_mut(self.active) else {
            return Ok(());
        };
        if field.buffer.trim().is_empty() && !field.typed {
            let value = live_for(field.kind, live);
            if field.kind.must_be_positive() && value <= GEOM_TOLERANCE {
                return Err(invalid_message(field.kind));
            }
            field.buffer = format_live(value);
            field.typed = true;
        }
        validate_field(field)?;
        field.locked = true;
        field.typed = true;
        Ok(())
    }

    fn backspace(&mut self) {
        let Some(field) = self.fields.get_mut(self.active) else {
            return;
        };
        field.typed = true;
        field.locked = false;
        field.buffer.pop();
    }

    fn insert_text(&mut self, text: &str) -> Result<bool, &'static str> {
        let Some(field) = self.fields.get_mut(self.active) else {
            return Ok(false);
        };
        let mut consumed = false;
        for ch in text.chars() {
            if insert_char(field, ch) {
                consumed = true;
            } else if ch.is_ascii_alphabetic() {
                return Ok(false);
            }
        }
        Ok(consumed)
    }

    pub fn paint(&self, painter: &egui::Painter, viewport: Rect, cursor: Pos2, live: LiveValues) {
        if !self.is_active() || self.fields.is_empty() {
            return;
        }
        let box_size = Vec2::new(92.0, 28.0);
        let gap = 4.0;
        let n = self.fields.len() as f32;
        let total = Vec2::new(n * box_size.x + (n - 1.0).max(0.0) * gap, box_size.y);
        let origin = clamp_overlay(viewport, cursor + Vec2::new(16.0, 16.0), total);
        for (index, field) in self.fields.iter().enumerate() {
            let min = Pos2::new(origin.x + index as f32 * (box_size.x + gap), origin.y);
            let rect = Rect::from_min_size(min, box_size);
            let active = index == self.active;
            let fill = if field.locked {
                Color32::from_rgb(28, 52, 48)
            } else if active {
                Color32::from_rgb(24, 36, 44)
            } else {
                Color32::from_rgb(18, 24, 28)
            };
            let border = if active {
                Color32::from_rgb(90, 210, 200)
            } else if field.locked {
                Color32::from_rgb(70, 150, 130)
            } else {
                Color32::from_rgb(70, 90, 90)
            };
            painter.rect_filled(rect, 3.0, fill);
            painter.rect_stroke(
                rect,
                3.0,
                Stroke::new(1.0, border),
                egui::StrokeKind::Inside,
            );
            let live_value = live_for(field.kind, live);
            let caption = format!("{} {}", field.kind.label(), field.display_text(live_value));
            let mut text_color = Color32::from_rgb(230, 235, 230);
            if field.locked {
                text_color = Color32::from_rgb(170, 230, 210);
            }
            painter.text(
                rect.left_center() + Vec2::new(6.0, 0.0),
                egui::Align2::LEFT_CENTER,
                caption,
                FontId::monospace(11.0),
                text_color,
            );
            if field.locked {
                painter.text(
                    rect.right_center() - Vec2::new(6.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    "L",
                    FontId::monospace(10.0),
                    Color32::from_rgb(120, 200, 180),
                );
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LiveValues {
    pub length: f64,
    pub angle_deg: f64,
    pub radius: f64,
    pub width: f64,
    pub height: f64,
    pub factor: f64,
}

impl LiveValues {
    pub fn from_points(base: Option<Point2>, current: Option<Point2>) -> Self {
        let (length, angle_deg, width, height) = match (base, current) {
            (Some(base), Some(current)) => {
                let dx = current.x - base.x;
                let dy = current.y - base.y;
                (
                    base.distance(current),
                    dy.atan2(dx).to_degrees(),
                    dx.abs(),
                    dy.abs(),
                )
            }
            _ => (0.0, 0.0, 0.0, 0.0),
        };
        Self {
            length,
            angle_deg,
            radius: length,
            width,
            height,
            factor: 1.0,
        }
    }

    pub fn with_factor(mut self, factor: f64) -> Self {
        self.factor = factor;
        self
    }
}

fn fields_for(layout: DynamicLayout) -> Vec<Field> {
    match layout {
        DynamicLayout::Hidden => Vec::new(),
        DynamicLayout::LengthAngle => {
            vec![Field::new(FieldKind::Length), Field::new(FieldKind::Angle)]
        }
        DynamicLayout::Radius => vec![Field::new(FieldKind::Radius)],
        DynamicLayout::WidthHeight => {
            vec![Field::new(FieldKind::Width), Field::new(FieldKind::Height)]
        }
        DynamicLayout::Angle => vec![Field::new(FieldKind::Angle)],
        DynamicLayout::Factor => vec![Field::new(FieldKind::Factor)],
    }
}

fn live_for(kind: FieldKind, live: LiveValues) -> f64 {
    match kind {
        FieldKind::Length => live.length,
        FieldKind::Angle => live.angle_deg,
        FieldKind::Radius => live.radius,
        FieldKind::Width => live.width,
        FieldKind::Height => live.height,
        FieldKind::Factor => live.factor,
    }
}

fn field_value(fields: &[Field], kind: FieldKind) -> Option<f64> {
    fields
        .iter()
        .find(|field| field.kind == kind)
        .and_then(Field::constrained_value)
}

fn constrain_length_angle(fields: &[Field], base: Point2, resolved: Point2) -> Point2 {
    let raw_dx = resolved.x - base.x;
    let raw_dy = resolved.y - base.y;
    let raw_len = base.distance(resolved);
    let raw_angle = raw_dy.atan2(raw_dx);
    let length = field_value(fields, FieldKind::Length);
    let angle = field_value(fields, FieldKind::Angle);
    if length.is_none() && angle.is_none() {
        return resolved;
    }
    let length = length.unwrap_or(raw_len);
    let angle = angle.map(|deg| deg.to_radians()).unwrap_or(raw_angle);
    if !length.is_finite() || length <= GEOM_TOLERANCE || !angle.is_finite() {
        return resolved;
    }
    Point2::new(base.x + length * angle.cos(), base.y + length * angle.sin())
}

fn constrain_angle_only(fields: &[Field], base: Point2, resolved: Point2) -> Point2 {
    let Some(deg) = field_value(fields, FieldKind::Angle) else {
        return resolved;
    };
    let length = base.distance(resolved);
    if !deg.is_finite() || length <= GEOM_TOLERANCE {
        return resolved;
    }
    let angle = deg.to_radians();
    Point2::new(base.x + length * angle.cos(), base.y + length * angle.sin())
}

fn constrain_radius(fields: &[Field], center: Point2, resolved: Point2) -> Point2 {
    let raw = center.distance(resolved);
    let radius = field_value(fields, FieldKind::Radius).unwrap_or(raw);
    if !radius.is_finite() || radius <= GEOM_TOLERANCE {
        return resolved;
    }
    if raw <= GEOM_TOLERANCE {
        return Point2::new(center.x + radius, center.y);
    }
    let scale = radius / raw;
    Point2::new(
        center.x + (resolved.x - center.x) * scale,
        center.y + (resolved.y - center.y) * scale,
    )
}

fn constrain_width_height(fields: &[Field], first: Point2, resolved: Point2) -> Point2 {
    let sign_x = if resolved.x >= first.x { 1.0 } else { -1.0 };
    let sign_y = if resolved.y >= first.y { 1.0 } else { -1.0 };
    let width = field_value(fields, FieldKind::Width)
        .unwrap_or((resolved.x - first.x).abs())
        .abs();
    let height = field_value(fields, FieldKind::Height)
        .unwrap_or((resolved.y - first.y).abs())
        .abs();
    if width <= GEOM_TOLERANCE && field_value(fields, FieldKind::Width).is_some()
        || height <= GEOM_TOLERANCE && field_value(fields, FieldKind::Height).is_some()
    {
        return resolved;
    }
    Point2::new(first.x + sign_x * width, first.y + sign_y * height)
}

fn insert_char(field: &mut Field, ch: char) -> bool {
    let ch = if ch == ',' { '.' } else { ch };
    if ch == '-' {
        if !field.kind.allows_minus() || !field.buffer.is_empty() {
            return false;
        }
        field.typed = true;
        field.locked = false;
        field.buffer.push('-');
        return true;
    }
    if ch == '.' {
        let normalized = field.buffer.replace(',', ".");
        if normalized.contains('.') {
            return false;
        }
        field.typed = true;
        field.locked = false;
        field.buffer.push('.');
        return true;
    }
    if ch.is_ascii_digit() {
        field.typed = true;
        field.locked = false;
        field.buffer.push(ch);
        return true;
    }
    false
}

fn validate_field(field: &Field) -> Result<(), &'static str> {
    let value = parse_number(&field.buffer).map_err(|_| invalid_message(field.kind))?;
    if field.kind.must_be_positive() && value.abs() <= GEOM_TOLERANCE {
        return Err(invalid_message(field.kind));
    }
    Ok(())
}

fn invalid_message(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Length => "Length must be a positive number",
        FieldKind::Angle => "Angle must be a finite number",
        FieldKind::Radius => "Radius must be a positive number",
        FieldKind::Width => "Width must be a positive number",
        FieldKind::Height => "Height must be a positive number",
        FieldKind::Factor => "Scale factor must be greater than zero",
    }
}

pub fn parse_number(text: &str) -> Result<f64, ()> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(());
    }
    let normalized = trimmed.replace(',', ".");
    let value: f64 = normalized.parse().map_err(|_| ())?;
    if !value.is_finite() {
        return Err(());
    }
    Ok(value)
}

fn format_live(value: f64) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    format!("{value:.4}")
}

fn clamp_overlay(viewport: Rect, preferred: Pos2, size: Vec2) -> Pos2 {
    let pad = 6.0;
    let max = Pos2::new(
        (viewport.max.x - size.x - pad).max(viewport.min.x + pad),
        (viewport.max.y - size.y - pad).max(viewport.min.y + pad),
    );
    Pos2::new(
        preferred.x.clamp(viewport.min.x + pad, max.x),
        preferred.y.clamp(viewport.min.y + pad, max.y),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn length_angle() -> DynamicInput {
        let mut input = DynamicInput::default();
        input.set_layout(DynamicLayout::LengthAngle);
        input
    }

    #[test]
    fn parse_number_accepts_decimal_separator_variants() {
        assert_eq!(parse_number("100"), Ok(100.0));
        assert_eq!(parse_number("12.5"), Ok(12.5));
        assert_eq!(parse_number("12,5"), Ok(12.5));
        assert_eq!(parse_number("-45"), Ok(-45.0));
        assert!(parse_number("").is_err());
        assert!(parse_number("abc").is_err());
        assert!(parse_number("inf").is_err());
        assert!(parse_number("nan").is_err());
    }

    #[test]
    fn length_and_angle_example_locks_polar_point() {
        let mut input = length_angle();
        let length = input.fields.get_mut(0).expect("length");
        length.buffer = "100".into();
        length.typed = true;
        length.locked = true;
        let angle = input.fields.get_mut(1).expect("angle");
        angle.buffer = "45".into();
        angle.typed = true;
        let base = Point2::new(10.0, 20.0);
        let cursor = Point2::new(80.0, 90.0);
        let point = input.constrain(Some(base), cursor);
        let expected = Point2::new(
            10.0 + 100.0 * 45f64.to_radians().cos(),
            20.0 + 100.0 * 45f64.to_radians().sin(),
        );
        assert!((point.x - expected.x).abs() < 1e-9);
        assert!((point.y - expected.y).abs() < 1e-9);
    }

    #[test]
    fn unlocked_angle_follows_cursor_or_ortho_direction() {
        let mut input = length_angle();
        let length = input.fields.get_mut(0).expect("length");
        length.buffer = "10".into();
        length.typed = true;
        let base = Point2::new(0.0, 0.0);
        let along_x = input.constrain(Some(base), Point2::new(4.0, 0.0));
        assert!((along_x.x - 10.0).abs() < 1e-9);
        assert!(along_x.y.abs() < 1e-9);
        let along_y = input.constrain(Some(base), Point2::new(0.0, 8.0));
        assert!(along_y.x.abs() < 1e-9);
        assert!((along_y.y - 10.0).abs() < 1e-9);
    }

    #[test]
    fn numeric_length_overrides_osnap_distance_only() {
        let mut input = length_angle();
        let length = input.fields.get_mut(0).expect("length");
        length.buffer = "5".into();
        length.typed = true;
        let base = Point2::new(0.0, 0.0);
        let snap = Point2::new(20.0, 0.0);
        let point = input.constrain(Some(base), snap);
        assert!((point.x - 5.0).abs() < 1e-12);
        assert!(point.y.abs() < 1e-12);
        assert!(point.distance(snap) > 1.0);
    }

    #[test]
    fn rectangle_width_height_stay_positive_in_all_quadrants() {
        let mut input = DynamicInput::default();
        input.set_layout(DynamicLayout::WidthHeight);
        input.fields[0].buffer = "8".into();
        input.fields[0].typed = true;
        input.fields[1].buffer = "3".into();
        input.fields[1].typed = true;
        let first = Point2::new(10.0, 10.0);
        let lower_left = input.constrain(Some(first), Point2::new(1.0, 2.0));
        assert_eq!(lower_left, Point2::new(2.0, 7.0));
        let upper_right = input.constrain(Some(first), Point2::new(12.0, 14.0));
        assert_eq!(upper_right, Point2::new(18.0, 13.0));
    }

    #[test]
    fn zero_and_non_finite_values_are_rejected() {
        assert!(parse_number("inf").is_err());
        let mut field = Field::new(FieldKind::Length);
        field.buffer = "0".into();
        field.typed = true;
        assert!(validate_field(&field).is_err());
        field.buffer = "1".into();
        assert!(validate_field(&field).is_ok());
        let mut angle = Field::new(FieldKind::Angle);
        angle.buffer = "-90".into();
        angle.typed = true;
        assert!(validate_field(&angle).is_ok());
    }

    #[test]
    fn unlocked_length_angle_keeps_snapped_point() {
        let input = length_angle();
        let base = Point2::new(0.0, 0.0);
        let snap = Point2::new(3.0, 4.0);
        assert_eq!(input.constrain(Some(base), snap), snap);
        let live = LiveValues::from_points(Some(base), Some(snap));
        assert!((live.length - 5.0).abs() < 1e-12);
        assert!((live.angle_deg - 4.0_f64.atan2(3.0).to_degrees()).abs() < 1e-12);
    }

    #[test]
    fn compatible_locked_length_keeps_snap_point() {
        let mut input = length_angle();
        let length = input.fields.get_mut(0).expect("length");
        length.buffer = "5".into();
        length.typed = true;
        let base = Point2::new(0.0, 0.0);
        let snap = Point2::new(5.0, 0.0);
        let point = input.constrain(Some(base), snap);
        assert!(point.distance(snap) <= GEOM_TOLERANCE);
    }

    #[test]
    fn overlay_stays_inside_viewport_near_edges() {
        let viewport = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(200.0, 100.0));
        let size = Vec2::new(188.0, 28.0);
        let clamped = clamp_overlay(viewport, Pos2::new(195.0, 95.0), size);
        assert!(clamped.x + size.x <= viewport.max.x);
        assert!(clamped.y + size.y <= viewport.max.y);
        assert!(clamped.x >= viewport.min.x);
        assert!(clamped.y >= viewport.min.y);
    }

    #[test]
    fn factor_layout_owns_keyboard_so_delete_hotkey_stays_gated() {
        let mut input = DynamicInput::default();
        input.set_layout(DynamicLayout::Factor);
        assert!(input.is_active());
        assert_eq!(input.layout(), DynamicLayout::Factor);
    }
}
