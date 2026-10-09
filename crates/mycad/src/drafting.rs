//! Drafting aids shared by point-based commands and the status bar.

use cad_core::{
    edge_snaps, Extents2, MeasureIndex, Point2, Point3, SnapFeature, SnapIndex, SnapKind,
    GEOM_TOLERANCE,
};
use cad_viewport::Camera2;
use eframe::egui::{self, Color32, Pos2, Rect, Stroke};
use serde::{Deserialize, Serialize};

use crate::commands::PreviewGeometry;
use crate::dynamic_input::AlongPath;

pub const SNAP_APERTURE_PX: f64 = 9.0;
const SNAP_MARKER_RADIUS: f32 = 6.0;
const TRACK_DWELL_SECS: f64 = 0.35;
const MAX_TRACKED: usize = 7;
const TRACK_POINT_EPS: f64 = 1e-6;

pub const POLAR_INCREMENTS_DEG: [f64; 7] = [90.0, 45.0, 30.0, 22.5, 15.0, 10.0, 5.0];

fn default_enabled() -> bool {
    true
}

fn default_polar_increment() -> f64 {
    45.0
}

pub fn sanitize_polar_increment(value: f64) -> f64 {
    if !value.is_finite() {
        return 45.0;
    }
    POLAR_INCREMENTS_DEG
        .into_iter()
        .min_by(|left, right| (left - value).abs().total_cmp(&(right - value).abs()))
        .unwrap_or(45.0)
}

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
    #[serde(default = "default_enabled")]
    pub intersection: bool,
    #[serde(default = "default_enabled")]
    pub quadrant: bool,
    #[serde(default)]
    pub tangent: bool,
    #[serde(default)]
    pub perpendicular: bool,
    #[serde(default)]
    pub nearest: bool,
    #[serde(default)]
    pub node: bool,
    #[serde(default)]
    pub insertion: bool,
}

impl Default for RunningSnaps {
    fn default() -> Self {
        Self {
            endpoint: true,
            midpoint: true,
            center: true,
            intersection: true,
            quadrant: true,
            tangent: false,
            perpendicular: false,
            nearest: false,
            node: false,
            insertion: false,
        }
    }
}

impl RunningSnaps {
    pub fn allows(self, kind: SnapKind) -> bool {
        match kind {
            SnapKind::Endpoint => self.endpoint,
            SnapKind::Midpoint => self.midpoint,
            SnapKind::Center => self.center,
            SnapKind::Quadrant => self.quadrant,
            SnapKind::Intersection => self.intersection,
            SnapKind::Tangent => self.tangent,
            SnapKind::Perpendicular => self.perpendicular,
            SnapKind::Nearest => self.nearest,
            SnapKind::Node => self.node,
            SnapKind::Insertion => self.insertion,
        }
    }
}

// ------------------------------------------------------------
// Type: DraftingPreferences
// Purpose: Drafting switches persisted with AppSettings.
//          POLAR and ORTHO cannot both be on.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DraftingPreferences {
    pub ortho_enabled: bool,
    pub osnap_enabled: bool,
    pub running_snaps: RunningSnaps,
    #[serde(default)]
    pub polar_enabled: bool,
    #[serde(default = "default_polar_increment")]
    pub polar_increment_deg: f64,
    #[serde(default)]
    pub otrack_enabled: bool,
}

impl Default for DraftingPreferences {
    fn default() -> Self {
        Self {
            ortho_enabled: false,
            osnap_enabled: true,
            running_snaps: RunningSnaps::default(),
            polar_enabled: false,
            polar_increment_deg: 45.0,
            otrack_enabled: false,
        }
    }
}

impl DraftingPreferences {
    pub fn sanitize(&mut self) {
        if self.ortho_enabled && self.polar_enabled {
            self.polar_enabled = false;
        }
        self.polar_increment_deg = sanitize_polar_increment(self.polar_increment_deg);
    }

    pub fn toggle_ortho(&mut self) {
        self.ortho_enabled = !self.ortho_enabled;
        if self.ortho_enabled {
            self.polar_enabled = false;
        }
    }

    pub fn toggle_polar(&mut self) {
        self.polar_enabled = !self.polar_enabled;
        if self.polar_enabled {
            self.ortho_enabled = false;
        }
    }
}

// ------------------------------------------------------------
// Type: OneShotSnap
// Purpose: Shift+right-click override for the next accepted point.
//          None disables object snap for that pick only.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OneShotSnap {
    Kind(SnapKind),
    None,
}

// ------------------------------------------------------------
// Type: SnapSources
// Purpose: Point index, edge primitives, and in-command snaps
//          passed together into point resolution.
// ------------------------------------------------------------
#[derive(Clone, Copy)]
pub struct SnapSources<'a> {
    pub points: &'a SnapIndex,
    pub edges: &'a MeasureIndex,
    pub extra: &'a [SnapFeature],
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackedSnap {
    pub feature: SnapFeature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackSource {
    Polar,
    Object(SnapKind),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackRay {
    pub origin: Point2,
    pub angle_deg: f64,
    pub source: TrackSource,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackingHint {
    pub rays: Vec<TrackRay>,
    pub point: Point2,
}

struct AlignPath {
    origin: Point2,
    line_deg: f64,
    source: TrackSource,
    perp: f64,
    projected: Point2,
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
    pub snap_override: Option<OneShotSnap>,
    pub tracked_points: Vec<TrackedSnap>,
    pub tracking_hint: Option<TrackingHint>,
    nearby: Vec<SnapFeature>,
    edge_slots: Vec<usize>,
    candidates: Vec<SnapFeature>,
    edge_snaps_buf: Vec<SnapFeature>,
    hover_snap: Option<SnapFeature>,
    hover_since: f64,
    dwell_armed: bool,
    snap_cache: Option<SnapResolveCache>,
}

#[derive(Debug, Clone)]
struct SnapResolveCache {
    raw_x: u64,
    raw_y: u64,
    base: Option<Point2>,
    shift: bool,
    pixels_per_world: u64,
    height: u64,
    generation: u64,
    extra: u64,
    preferences: DraftingPreferences,
    snap_override: Option<OneShotSnap>,
    result: Point2,
    acquired: Option<SnapFeature>,
    tracking: Option<TrackingHint>,
}

impl SnapResolveCache {
    fn capture(
        raw: Point2,
        base: Option<Point2>,
        shift: bool,
        camera: &Camera2,
        height: f64,
        sources: &SnapSources<'_>,
        drafting: &DraftingState,
        result: Point2,
    ) -> Self {
        Self {
            raw_x: raw.x.to_bits(),
            raw_y: raw.y.to_bits(),
            base,
            shift,
            pixels_per_world: camera.pixels_per_world(height).to_bits(),
            height: height.to_bits(),
            generation: sources.generation,
            extra: snap_extra_fingerprint(sources.extra),
            preferences: drafting.preferences,
            snap_override: drafting.snap_override,
            result,
            acquired: drafting.acquired_snap,
            tracking: drafting.tracking_hint.clone(),
        }
    }

    fn matches(
        &self,
        raw: Point2,
        base: Option<Point2>,
        shift: bool,
        camera: &Camera2,
        height: f64,
        sources: &SnapSources<'_>,
        preferences: DraftingPreferences,
        snap_override: Option<OneShotSnap>,
    ) -> bool {
        self.raw_x == raw.x.to_bits()
            && self.raw_y == raw.y.to_bits()
            && self.base == base
            && self.shift == shift
            && self.pixels_per_world == camera.pixels_per_world(height).to_bits()
            && self.height == height.to_bits()
            && self.generation == sources.generation
            && self.extra == snap_extra_fingerprint(sources.extra)
            && self.preferences == preferences
            && self.snap_override == snap_override
    }
}

fn snap_extra_fingerprint(extra: &[SnapFeature]) -> u64 {
    let mut hash = extra.len() as u64;
    for feature in extra.iter().take(8) {
        hash = hash
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(feature.point.x.to_bits());
        hash = hash
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(feature.point.y.to_bits())
            .wrapping_add(feature.kind as u64);
    }
    hash
}

impl DraftingState {
    pub fn new(preferences: DraftingPreferences) -> Self {
        Self {
            preferences,
            acquired_snap: None,
            current_point: None,
            command_base_point: None,
            snap_override: None,
            tracked_points: Vec::new(),
            tracking_hint: None,
            nearby: Vec::new(),
            edge_slots: Vec::new(),
            candidates: Vec::new(),
            edge_snaps_buf: Vec::new(),
            hover_snap: None,
            hover_since: 0.0,
            dwell_armed: false,
            snap_cache: None,
        }
    }

    pub fn invalidate_snap_cache(&mut self) {
        self.snap_cache = None;
    }

    pub fn clear_acquisition(&mut self) {
        self.acquired_snap = None;
        self.current_point = None;
        self.command_base_point = None;
        self.snap_cache = None;
        self.consume_pick();
    }

    pub fn consume_pick(&mut self) {
        self.snap_override = None;
        self.tracked_points.clear();
        self.tracking_hint = None;
        self.hover_snap = None;
        self.dwell_armed = false;
    }

    pub fn dwell_remaining(&self, now: f64) -> Option<f64> {
        if !self.dwell_armed {
            return None;
        }
        let left = TRACK_DWELL_SECS - (now - self.hover_since);
        (left > 0.0).then_some(left)
    }

    pub fn along_path(&self) -> Option<AlongPath> {
        let ray = self.tracking_hint.as_ref()?.rays.first()?;
        Some(AlongPath {
            origin: ray.origin,
            angle_rad: ray.angle_deg.to_radians(),
        })
    }

    pub fn resolve_point(
        &mut self,
        raw: Point2,
        base: Option<Point2>,
        shift_held: bool,
        camera: &Camera2,
        viewport_height: f64,
        sources: SnapSources<'_>,
        time: f64,
    ) -> Point2 {
        let preferences = self.preferences;
        let snap_override = self.snap_override;
        let reused = self
            .snap_cache
            .as_ref()
            .filter(|cache| {
                cache.matches(
                    raw,
                    base,
                    shift_held,
                    camera,
                    viewport_height,
                    &sources,
                    preferences,
                    snap_override,
                )
            })
            .map(|cache| (cache.result, cache.acquired, cache.tracking.clone()));
        if let Some((result, acquired, tracking)) = reused {
            self.command_base_point = base;
            self.acquired_snap = acquired;
            self.tracking_hint = tracking;
            let armed = self.dwell_armed;
            self.update_dwell(time);
            let dwell_finished = armed && !self.dwell_armed;
            if !dwell_finished {
                self.current_point = Some(result);
                return result;
            }
            self.tracking_hint = None;
            let resolved = self.resolve_from_acquisition(
                raw,
                base,
                shift_held,
                world_aperture(camera, viewport_height),
            );
            self.current_point = Some(resolved);
            self.snap_cache = Some(SnapResolveCache::capture(
                raw,
                base,
                shift_held,
                camera,
                viewport_height,
                &sources,
                self,
                resolved,
            ));
            return resolved;
        }
        self.command_base_point = base;
        self.acquired_snap = None;
        self.tracking_hint = None;
        let aperture = world_aperture(camera, viewport_height);
        if self.snaps_enabled() {
            self.collect_snaps(raw, base, aperture, sources);
        }
        self.update_dwell(time);
        let resolved = self.resolve_from_acquisition(raw, base, shift_held, aperture);
        self.current_point = Some(resolved);
        self.snap_cache = Some(SnapResolveCache::capture(
            raw,
            base,
            shift_held,
            camera,
            viewport_height,
            &sources,
            self,
            resolved,
        ));
        resolved
    }

    fn snaps_enabled(&self) -> bool {
        match self.snap_override {
            Some(OneShotSnap::None) => false,
            Some(OneShotSnap::Kind(_)) => true,
            None => self.preferences.osnap_enabled,
        }
    }

    fn kind_allowed(&self, kind: SnapKind) -> bool {
        match self.snap_override {
            Some(OneShotSnap::Kind(only)) => only == kind,
            Some(OneShotSnap::None) => false,
            None => self.preferences.running_snaps.allows(kind),
        }
    }

    fn collect_snaps(
        &mut self,
        raw: Point2,
        base: Option<Point2>,
        world_aperture: f64,
        sources: SnapSources<'_>,
    ) {
        let region = Extents2::from_corners(
            Point2::new(raw.x - world_aperture, raw.y - world_aperture),
            Point2::new(raw.x + world_aperture, raw.y + world_aperture),
        );
        sources.points.query(region, &mut self.nearby);
        for feature in sources.extra {
            if region.contains(feature.point) {
                self.nearby.push(*feature);
            }
        }
        self.nearby
            .retain(|feature| !is_current_base(feature.point, base));
        self.candidates.clear();
        for feature in self.nearby.iter().copied() {
            if self.kind_allowed(feature.kind) && raw.distance(feature.point) <= world_aperture {
                self.candidates.push(feature);
            }
        }

        sources.edges.query_slots(region, &mut self.edge_slots);
        let slots = std::mem::take(&mut self.edge_slots);
        edge_snaps(
            slots
                .iter()
                .filter_map(|slot| sources.edges.primitive(*slot)),
            raw,
            base,
            world_aperture,
            &mut self.edge_snaps_buf,
        );
        self.edge_slots = slots;
        for feature in self.edge_snaps_buf.iter().copied() {
            if self.kind_allowed(feature.kind) && !is_current_base(feature.point, base) {
                self.candidates.push(feature);
            }
        }

        if self.snap_override != Some(OneShotSnap::Kind(SnapKind::Nearest)) {
            let stronger = self
                .candidates
                .iter()
                .any(|feature| feature.kind != SnapKind::Nearest);
            if stronger {
                self.candidates
                    .retain(|feature| feature.kind != SnapKind::Nearest);
            }
        }
        self.acquired_snap = self.candidates.iter().copied().min_by(|left, right| {
            raw.distance(left.point)
                .total_cmp(&raw.distance(right.point))
                .then(snap_rank(left.kind).cmp(&snap_rank(right.kind)))
        });
    }

    fn resolve_from_acquisition(
        &mut self,
        raw: Point2,
        base: Option<Point2>,
        shift_held: bool,
        world_aperture: f64,
    ) -> Point2 {
        let ortho_now = self.preferences.ortho_enabled ^ shift_held;
        if let Some(feature) = self.acquired_snap {
            feature.point
        } else if !ortho_now {
            self.acquire_tracking(raw, base, world_aperture)
                .unwrap_or(raw)
        } else {
            base.map(|base| constrain_ortho(base, raw)).unwrap_or(raw)
        }
    }

    fn update_dwell(&mut self, time: f64) {
        if !self.preferences.otrack_enabled {
            self.hover_snap = None;
            self.dwell_armed = false;
            return;
        }
        let Some(snap) = self.acquired_snap else {
            self.hover_snap = None;
            self.dwell_armed = false;
            return;
        };
        let same = self
            .hover_snap
            .is_some_and(|held| held.point.distance(snap.point) <= TRACK_POINT_EPS);
        if !same {
            self.hover_snap = Some(snap);
            self.hover_since = time;
            self.dwell_armed = true;
            return;
        }
        if self.dwell_armed && time - self.hover_since >= TRACK_DWELL_SECS {
            self.toggle_tracked(snap);
            self.dwell_armed = false;
        }
    }

    fn toggle_tracked(&mut self, snap: SnapFeature) {
        if let Some(index) = self
            .tracked_points
            .iter()
            .position(|tracked| tracked.feature.point.distance(snap.point) <= TRACK_POINT_EPS)
        {
            self.tracked_points.remove(index);
            return;
        }
        if self.tracked_points.len() >= MAX_TRACKED {
            self.tracked_points.remove(0);
        }
        self.tracked_points.push(TrackedSnap { feature: snap });
    }

    fn acquire_tracking(
        &mut self,
        raw: Point2,
        base: Option<Point2>,
        aperture: f64,
    ) -> Option<Point2> {
        let mut paths = Vec::new();
        if self.preferences.polar_enabled {
            if let Some(base) = base {
                if let Some(path) =
                    polar_path(base, raw, self.preferences.polar_increment_deg, aperture)
                {
                    paths.push(path);
                }
            }
        }
        if self.preferences.otrack_enabled {
            let angles = alignment_angles(&self.preferences);
            for tracked in &self.tracked_points {
                for angle in &angles {
                    if let Some(path) = line_path(
                        tracked.feature.point,
                        *angle,
                        TrackSource::Object(tracked.feature.kind),
                        raw,
                        aperture,
                    ) {
                        paths.push(path);
                    }
                }
            }
        }
        if paths.is_empty() {
            return None;
        }

        let mut best_cross: Option<(f64, Point2, usize, usize)> = None;
        for left in 0..paths.len() {
            for right in (left + 1)..paths.len() {
                let Some(point) = path_intersection(&paths[left], &paths[right]) else {
                    continue;
                };
                let distance = raw.distance(point);
                if distance > aperture {
                    continue;
                }
                match best_cross {
                    Some((best, _, _, _)) if distance >= best => {}
                    _ => best_cross = Some((distance, point, left, right)),
                }
            }
        }
        if let Some((_, point, left, right)) = best_cross {
            let mut rays = vec![
                ray_toward(&paths[left], point),
                ray_toward(&paths[right], point),
            ];
            if matches!(rays[0].source, TrackSource::Polar)
                && matches!(rays[1].source, TrackSource::Object(_))
            {
                rays.swap(0, 1);
            }
            self.tracking_hint = Some(TrackingHint { rays, point });
            return Some(point);
        }

        let best = paths
            .iter()
            .min_by(|left, right| left.perp.total_cmp(&right.perp))?;
        let point = best.projected;
        self.tracking_hint = Some(TrackingHint {
            rays: vec![ray_toward(best, point)],
            point,
        });
        Some(point)
    }
}

fn world_aperture(camera: &Camera2, viewport_height: f64) -> f64 {
    SNAP_APERTURE_PX / camera.pixels_per_world(viewport_height).max(1e-15)
}

fn is_current_base(point: Point2, base: Option<Point2>) -> bool {
    base.is_some_and(|base| point.distance(base) <= GEOM_TOLERANCE)
}

fn snap_rank(kind: SnapKind) -> u8 {
    match kind {
        SnapKind::Endpoint => 0,
        SnapKind::Midpoint => 1,
        SnapKind::Center => 2,
        SnapKind::Node => 3,
        SnapKind::Quadrant => 4,
        SnapKind::Intersection => 5,
        SnapKind::Insertion => 6,
        SnapKind::Perpendicular => 7,
        SnapKind::Tangent => 8,
        SnapKind::Nearest => 9,
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

fn alignment_angles(preferences: &DraftingPreferences) -> Vec<f64> {
    if !preferences.polar_enabled {
        return vec![0.0, 90.0];
    }
    let increment = sanitize_polar_increment(preferences.polar_increment_deg);
    let mut angles = Vec::new();
    let mut angle = 0.0;
    while angle < 180.0 - 1e-6 {
        angles.push(angle);
        angle += increment;
    }
    if angles.is_empty() {
        angles.push(0.0);
    }
    angles
}

fn polar_path(base: Point2, raw: Point2, increment_deg: f64, aperture: f64) -> Option<AlignPath> {
    let dx = raw.x - base.x;
    let dy = raw.y - base.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len <= GEOM_TOLERANCE {
        return None;
    }
    let increment = sanitize_polar_increment(increment_deg).to_radians();
    let angle = dy.atan2(dx);
    let snapped = (angle / increment).round() * increment;
    let delta = angle - snapped;
    let perp = (len * delta.sin()).abs();
    if perp > aperture {
        return None;
    }
    let along = len * delta.cos();
    if along <= GEOM_TOLERANCE {
        return None;
    }
    let projected = Point2::new(
        base.x + along * snapped.cos(),
        base.y + along * snapped.sin(),
    );
    Some(AlignPath {
        origin: base,
        line_deg: snapped.to_degrees().rem_euclid(180.0),
        source: TrackSource::Polar,
        perp,
        projected,
    })
}

fn line_path(
    origin: Point2,
    line_deg: f64,
    source: TrackSource,
    raw: Point2,
    aperture: f64,
) -> Option<AlignPath> {
    let rad = line_deg.to_radians();
    let ux = rad.cos();
    let uy = rad.sin();
    let dx = raw.x - origin.x;
    let dy = raw.y - origin.y;
    let along = dx * ux + dy * uy;
    let projected = Point2::new(origin.x + along * ux, origin.y + along * uy);
    let perp = raw.distance(projected);
    if perp > aperture || raw.distance(origin) <= GEOM_TOLERANCE {
        return None;
    }
    Some(AlignPath {
        origin,
        line_deg,
        source,
        perp,
        projected,
    })
}

fn path_intersection(left: &AlignPath, right: &AlignPath) -> Option<Point2> {
    let left_dir = dir_from_deg(left.line_deg);
    let right_dir = dir_from_deg(right.line_deg);
    let cross = left_dir.x * right_dir.y - left_dir.y * right_dir.x;
    if cross.abs() < 1e-8 {
        return None;
    }
    cad_core::measure::infinite_line_intersection(
        left.origin,
        left.origin + left_dir,
        right.origin,
        right.origin + right_dir,
    )
}

fn dir_from_deg(deg: f64) -> Point2 {
    let rad = deg.to_radians();
    Point2::new(rad.cos(), rad.sin())
}

fn ray_toward(path: &AlignPath, point: Point2) -> TrackRay {
    TrackRay {
        origin: path.origin,
        angle_deg: direction_deg(path.origin, point),
        source: path.source,
    }
}

fn direction_deg(origin: Point2, point: Point2) -> f64 {
    let mut deg = (point.y - origin.y).atan2(point.x - origin.x).to_degrees();
    if deg < 0.0 {
        deg += 360.0;
    }
    if deg.is_finite() {
        deg
    } else {
        0.0
    }
}

pub fn paint_overlay(
    painter: &egui::Painter,
    rect: Rect,
    camera: Camera2,
    preview: Option<PreviewGeometry<'_>>,
    acquired_snap: Option<SnapFeature>,
    start_marker: Option<Point2>,
    close_hint: bool,
    tracked: &[TrackedSnap],
    tracking: Option<&TrackingHint>,
) {
    let origin = Point2::new(rect.min.x as f64, rect.min.y as f64);
    let size = Point2::new(rect.width() as f64, rect.height() as f64);
    let to_screen = |point: Point2| {
        let point = camera.world_to_screen(point, origin, size);
        Pos2::new(point.x as f32, point.y as f32)
    };
    let stroke = Stroke::new(1.5_f32, Color32::from_rgb(235, 235, 235));

    if let Some(preview) = preview {
        paint_preview(painter, &to_screen, preview, stroke);
    }
    if let Some(hint) = tracking {
        paint_tracking(painter, &to_screen, camera, rect, hint);
    }
    for tracked in tracked {
        paint_track_cross(painter, to_screen(tracked.feature.point));
    }
    if let Some(start) = start_marker {
        paint_snap_marker(painter, to_screen(start), SnapKind::Endpoint, false);
    }
    if let Some(feature) = acquired_snap {
        let screen = to_screen(feature.point);
        paint_snap_marker(painter, screen, feature.kind, true);
        if close_hint {
            painter.text(
                screen + egui::vec2(10.0, -8.0),
                egui::Align2::LEFT_CENTER,
                "Close",
                egui::FontId::proportional(11.0),
                Color32::from_rgb(80, 230, 220),
            );
        }
    }
}

fn paint_tracking(
    painter: &egui::Painter,
    to_screen: &impl Fn(Point2) -> Pos2,
    camera: Camera2,
    viewport: Rect,
    hint: &TrackingHint,
) {
    let color = Color32::from_rgb(120, 200, 190);
    let stroke = Stroke::new(1.0_f32, color);
    for ray in &hint.rays {
        let span = camera.view_height.max(1.0) * 4.0;
        let dir = dir_from_deg(ray.angle_deg);
        let start = Point2::new(ray.origin.x - dir.x * span, ray.origin.y - dir.y * span);
        let end = Point2::new(ray.origin.x + dir.x * span, ray.origin.y + dir.y * span);
        let Some((from, to)) = clip_segment(to_screen(start), to_screen(end), viewport) else {
            continue;
        };
        paint_dashed(painter, from, to, stroke);
    }
    let screen = to_screen(hint.point);
    for (index, ray) in hint.rays.iter().enumerate() {
        painter.text(
            screen + egui::vec2(12.0, -8.0 - index as f32 * 14.0),
            egui::Align2::LEFT_BOTTOM,
            format_tracking(ray, hint.point),
            egui::FontId::proportional(11.0),
            color,
        );
    }
}

fn format_tracking(ray: &TrackRay, point: Point2) -> String {
    let name = match ray.source {
        TrackSource::Polar => "Polar",
        TrackSource::Object(kind) => kind.label(),
    };
    format!(
        "{name}: {:.3} < {}°",
        ray.origin.distance(point),
        format_angle(ray.angle_deg)
    )
}

fn format_angle(deg: f64) -> String {
    if (deg - deg.round()).abs() < 0.05 {
        format!("{:.0}", deg.round())
    } else {
        format!("{deg:.1}")
    }
}

fn clip_segment(start: Pos2, end: Pos2, bounds: Rect) -> Option<(Pos2, Pos2)> {
    const INSIDE: u8 = 0;
    const LEFT: u8 = 1;
    const RIGHT: u8 = 2;
    const BOTTOM: u8 = 4;
    const TOP: u8 = 8;
    let code = |point: Pos2| -> u8 {
        let mut bits = INSIDE;
        if point.x < bounds.min.x {
            bits |= LEFT;
        } else if point.x > bounds.max.x {
            bits |= RIGHT;
        }
        if point.y < bounds.min.y {
            bits |= BOTTOM;
        } else if point.y > bounds.max.y {
            bits |= TOP;
        }
        bits
    };
    let mut a = start;
    let mut b = end;
    let mut code_a = code(a);
    let mut code_b = code(b);
    for _ in 0..8 {
        if code_a | code_b == 0 {
            return Some((a, b));
        }
        if code_a & code_b != 0 {
            return None;
        }
        let out = if code_a != 0 { code_a } else { code_b };
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let point = if out & TOP != 0 && dy.abs() > f32::EPSILON {
            let t = (bounds.max.y - a.y) / dy;
            Pos2::new(a.x + t * dx, bounds.max.y)
        } else if out & BOTTOM != 0 && dy.abs() > f32::EPSILON {
            let t = (bounds.min.y - a.y) / dy;
            Pos2::new(a.x + t * dx, bounds.min.y)
        } else if out & RIGHT != 0 && dx.abs() > f32::EPSILON {
            let t = (bounds.max.x - a.x) / dx;
            Pos2::new(bounds.max.x, a.y + t * dy)
        } else if out & LEFT != 0 && dx.abs() > f32::EPSILON {
            let t = (bounds.min.x - a.x) / dx;
            Pos2::new(bounds.min.x, a.y + t * dy)
        } else {
            return None;
        };
        if out == code_a {
            a = point;
            code_a = code(a);
        } else {
            b = point;
            code_b = code(b);
        }
    }
    None
}

fn paint_dashed(painter: &egui::Painter, start: Pos2, end: Pos2, stroke: Stroke) {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let ux = dx / len;
    let uy = dy / len;
    let dash = 8.0;
    let gap = 5.0;
    let mut traveled = 0.0;
    while traveled < len {
        let next = (traveled + dash).min(len);
        painter.line_segment(
            [
                Pos2::new(start.x + ux * traveled, start.y + uy * traveled),
                Pos2::new(start.x + ux * next, start.y + uy * next),
            ],
            stroke,
        );
        traveled += dash + gap;
    }
}

fn paint_track_cross(painter: &egui::Painter, center: Pos2) {
    let color = Color32::from_rgb(80, 230, 220);
    let stroke = Stroke::new(1.25_f32, color);
    let arm = 5.0;
    painter.line_segment(
        [
            center + egui::vec2(-arm, 0.0),
            center + egui::vec2(arm, 0.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            center + egui::vec2(0.0, -arm),
            center + egui::vec2(0.0, arm),
        ],
        stroke,
    );
}

pub fn paint_world_axis(
    painter: &egui::Painter,
    rect: Rect,
    camera: Camera2,
    start: Point2,
    end: Point2,
) {
    let dir = Point2::new(end.x - start.x, end.y - start.y);
    let length = (dir.x * dir.x + dir.y * dir.y).sqrt();
    if length <= GEOM_TOLERANCE {
        return;
    }
    let origin = Point2::new(rect.min.x as f64, rect.min.y as f64);
    let size = Point2::new(rect.width() as f64, rect.height() as f64);
    let to_screen = |point: Point2| {
        let point = camera.world_to_screen(point, origin, size);
        Pos2::new(point.x as f32, point.y as f32)
    };
    let span = camera.view_height.max(1.0) * 8.0;
    let ux = dir.x / length;
    let uy = dir.y / length;
    let a = Point2::new(start.x - ux * span, start.y - uy * span);
    let b = Point2::new(start.x + ux * span, start.y + uy * span);
    painter.line_segment(
        [to_screen(a), to_screen(b)],
        Stroke::new(1.0_f32, Color32::from_rgb(90, 140, 150)),
    );
}

pub fn paint_segments(
    painter: &egui::Painter,
    rect: Rect,
    camera: Camera2,
    segments: &[[Point2; 2]],
) {
    if segments.is_empty() {
        return;
    }
    let origin = Point2::new(rect.min.x as f64, rect.min.y as f64);
    let size = Point2::new(rect.width() as f64, rect.height() as f64);
    let stroke = Stroke::new(1.25_f32, Color32::from_rgb(120, 220, 190));
    for segment in segments {
        let a = camera.world_to_screen(segment[0], origin, size);
        let b = camera.world_to_screen(segment[1], origin, size);
        painter.line_segment(
            [
                Pos2::new(a.x as f32, a.y as f32),
                Pos2::new(b.x as f32, b.y as f32),
            ],
            stroke,
        );
    }
}

pub fn paint_crossing_window(
    painter: &egui::Painter,
    rect: Rect,
    camera: Camera2,
    first: Point2,
    opposite: Point2,
    border: Color32,
    fill: Color32,
) {
    let origin = Point2::new(rect.min.x as f64, rect.min.y as f64);
    let size = Point2::new(rect.width() as f64, rect.height() as f64);
    let first = camera.world_to_screen(first, origin, size);
    let opposite = camera.world_to_screen(opposite, origin, size);
    let window = Rect::from_two_pos(
        Pos2::new(first.x as f32, first.y as f32),
        Pos2::new(opposite.x as f32, opposite.y as f32),
    );
    if window.width() < 1.0 && window.height() < 1.0 {
        return;
    }
    painter.rect(
        window,
        0.0,
        fill,
        Stroke::new(1.5_f32, border),
        egui::StrokeKind::Inside,
    );
}

fn paint_preview(
    painter: &egui::Painter,
    to_screen: &impl Fn(Point2) -> Pos2,
    preview: PreviewGeometry<'_>,
    stroke: Stroke,
) {
    match preview {
        PreviewGeometry::LineSegment([start, end]) => {
            painter.line_segment([to_screen(start), to_screen(end)], stroke);
        }
        PreviewGeometry::Polyline {
            vertices,
            next,
            closed,
        } => {
            let mut points: Vec<Pos2> = vertices.iter().copied().map(to_screen).collect();
            if let Some(next) = next {
                points.push(to_screen(next));
            }
            paint_polyline(painter, &points, closed, stroke);
        }
        PreviewGeometry::Circle { center, radius } => {
            let screen_center = to_screen(center);
            let rim = to_screen(Point2::new(center.x + radius, center.y));
            let screen_radius = screen_center.distance(rim).max(1.0);
            painter.circle_stroke(screen_center, screen_radius, stroke);
        }
        PreviewGeometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
        } => {
            let samples = cad_render::curves::arc_points(
                Point3::from_xy(center.x, center.y),
                radius,
                start_angle,
                end_angle,
                true,
                Point3::new(0.0, 0.0, 1.0),
                48,
            );
            let points: Vec<Pos2> = samples.into_iter().map(to_screen).collect();
            paint_polyline(painter, &points, false, stroke);
        }
        PreviewGeometry::Rectangle { corners } => {
            let points: Vec<Pos2> = corners.iter().copied().map(to_screen).collect();
            paint_polyline(painter, &points, true, stroke);
        }
    }
}

fn paint_polyline(painter: &egui::Painter, points: &[Pos2], closed: bool, stroke: Stroke) {
    if points.len() < 2 {
        return;
    }
    if closed {
        painter.add(egui::Shape::closed_line(points.to_vec(), stroke));
    } else {
        for pair in points.windows(2) {
            painter.line_segment([pair[0], pair[1]], stroke);
        }
    }
}

fn paint_snap_marker(painter: &egui::Painter, center: Pos2, kind: SnapKind, labeled: bool) {
    let color = Color32::from_rgb(80, 230, 220);
    let stroke = Stroke::new(1.5_f32, color);
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
        SnapKind::Quadrant => {
            let points = vec![
                Pos2::new(center.x, center.y - radius),
                Pos2::new(center.x + radius, center.y),
                Pos2::new(center.x, center.y + radius),
                Pos2::new(center.x - radius, center.y),
            ];
            painter.add(egui::Shape::closed_line(points, stroke));
        }
        SnapKind::Intersection | SnapKind::Node => {
            if matches!(kind, SnapKind::Node) {
                painter.circle_stroke(center, radius, stroke);
            }
            let arm = radius * 0.8;
            painter.line_segment(
                [
                    center + egui::vec2(-arm, -arm),
                    center + egui::vec2(arm, arm),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    center + egui::vec2(-arm, arm),
                    center + egui::vec2(arm, -arm),
                ],
                stroke,
            );
        }
        SnapKind::Nearest => {
            painter.line_segment(
                [
                    center + egui::vec2(-radius, -radius),
                    center + egui::vec2(radius, -radius),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    center + egui::vec2(radius, -radius),
                    center + egui::vec2(-radius, radius),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    center + egui::vec2(-radius, radius),
                    center + egui::vec2(radius, radius),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    center + egui::vec2(radius, radius),
                    center + egui::vec2(-radius, -radius),
                ],
                stroke,
            );
        }
        SnapKind::Perpendicular => {
            painter.line_segment(
                [
                    center + egui::vec2(-radius, radius),
                    center + egui::vec2(-radius, -radius),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    center + egui::vec2(-radius, -radius),
                    center + egui::vec2(radius, -radius),
                ],
                stroke,
            );
        }
        SnapKind::Tangent => {
            painter.circle_stroke(center, radius * 0.7, stroke);
            painter.line_segment(
                [
                    center + egui::vec2(-radius, -radius * 0.15),
                    center + egui::vec2(radius, -radius * 0.15),
                ],
                stroke,
            );
        }
        SnapKind::Insertion => {
            let size = egui::vec2(radius * 1.4, radius * 1.4);
            painter.rect_stroke(
                Rect::from_center_size(center + egui::vec2(-2.0, -2.0), size),
                0.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.rect_stroke(
                Rect::from_center_size(center + egui::vec2(2.0, 2.0), size),
                0.0,
                stroke,
                egui::StrokeKind::Inside,
            );
        }
    }
    if labeled {
        painter.text(
            center + egui::vec2(radius + 4.0, radius + 2.0),
            egui::Align2::LEFT_TOP,
            kind.label(),
            egui::FontId::proportional(11.0),
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_core::{Document, Entity, Geometry};

    fn resolve(
        drafting: &mut DraftingState,
        raw: Point2,
        base: Option<Point2>,
        shift: bool,
        camera: &Camera2,
        height: f64,
        points: &SnapIndex,
        extra: &[SnapFeature],
    ) -> Point2 {
        let edges = MeasureIndex::default();
        drafting.resolve_point(
            raw,
            base,
            shift,
            camera,
            height,
            SnapSources {
                points,
                edges: &edges,
                extra,
                generation: 0,
            },
            0.0,
        )
    }

    fn zoomed_out() -> Camera2 {
        Camera2 {
            center: Point2::new(0.0, 0.0),
            view_height: 100.0,
        }
    }

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
        let constrained = resolve(
            &mut drafting,
            raw,
            Some(base),
            false,
            &camera,
            600.0,
            &SnapIndex::default(),
            &[],
        );
        let reversed = resolve(
            &mut drafting,
            raw,
            Some(base),
            true,
            &camera,
            600.0,
            &SnapIndex::default(),
            &[],
        );
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
        let far_camera = zoomed_out();
        let near_camera = Camera2 {
            center: Point2::new(0.0, 0.0),
            view_height: 10.0,
        };
        assert_eq!(
            resolve(
                &mut drafting,
                Point2::new(0.8, 0.0),
                None,
                false,
                &far_camera,
                1000.0,
                &index,
                &[],
            ),
            snap.point
        );
        assert_eq!(
            resolve(
                &mut drafting,
                Point2::new(0.08, 0.0),
                None,
                false,
                &near_camera,
                1000.0,
                &index,
                &[],
            ),
            snap.point
        );
    }

    #[test]
    fn transient_command_snaps_merge_when_document_index_is_empty() {
        let extra = [SnapFeature {
            point: Point2::new(4.0, 0.0),
            kind: SnapKind::Endpoint,
        }];
        let mut drafting = DraftingState::new(DraftingPreferences::default());
        let resolved = resolve(
            &mut drafting,
            Point2::new(4.2, 0.0),
            Some(Point2::new(0.0, 0.0)),
            false,
            &zoomed_out(),
            1000.0,
            &SnapIndex::default(),
            &extra,
        );
        assert_eq!(resolved, extra[0].point);
        assert_eq!(
            drafting.acquired_snap.map(|feature| feature.kind),
            Some(SnapKind::Endpoint)
        );
    }

    #[test]
    fn current_base_and_live_preview_cannot_self_snap() {
        let base = Point2::new(10.0, 0.0);
        let extra = [
            SnapFeature {
                point: base,
                kind: SnapKind::Endpoint,
            },
            SnapFeature {
                point: Point2::new(5.0, 0.0),
                kind: SnapKind::Midpoint,
            },
        ];
        let document = SnapIndex::from_features(vec![SnapFeature {
            point: base,
            kind: SnapKind::Endpoint,
        }]);
        let mut drafting = DraftingState::new(DraftingPreferences::default());
        let resolved = resolve(
            &mut drafting,
            Point2::new(10.05, 0.0),
            Some(base),
            false,
            &zoomed_out(),
            1000.0,
            &document,
            &extra,
        );
        assert!(drafting.acquired_snap.is_none());
        assert_eq!(resolved, Point2::new(10.05, 0.0));
    }

    #[test]
    fn osnap_off_ignores_transient_snaps_and_explicit_close_is_separate() {
        let extra = [SnapFeature {
            point: Point2::new(0.0, 0.0),
            kind: SnapKind::Endpoint,
        }];
        let mut drafting = DraftingState::new(DraftingPreferences {
            osnap_enabled: false,
            ..DraftingPreferences::default()
        });
        let resolved = resolve(
            &mut drafting,
            Point2::new(0.1, 0.0),
            Some(Point2::new(4.0, 0.0)),
            false,
            &Camera2::default(),
            600.0,
            &SnapIndex::default(),
            &extra,
        );
        assert!(drafting.acquired_snap.is_none());
        assert_eq!(resolved, Point2::new(0.1, 0.0));
    }

    #[test]
    fn transient_endpoint_overrides_ortho() {
        let extra = [SnapFeature {
            point: Point2::new(3.0, 4.0),
            kind: SnapKind::Endpoint,
        }];
        let mut drafting = DraftingState::new(DraftingPreferences {
            ortho_enabled: true,
            osnap_enabled: true,
            ..DraftingPreferences::default()
        });
        let resolved = resolve(
            &mut drafting,
            Point2::new(3.05, 4.0),
            Some(Point2::new(0.0, 0.0)),
            false,
            &zoomed_out(),
            1000.0,
            &SnapIndex::default(),
            &extra,
        );
        assert_eq!(resolved, extra[0].point);
        assert_ne!(resolved, Point2::new(3.05, 0.0));
    }

    #[test]
    fn first_vertex_transient_snap_uses_nine_pixel_aperture_at_any_zoom() {
        let extra = [SnapFeature {
            point: Point2::new(0.0, 0.0),
            kind: SnapKind::Endpoint,
        }];
        let mut drafting = DraftingState::new(DraftingPreferences::default());
        let far_camera = zoomed_out();
        let near_camera = Camera2 {
            center: Point2::new(0.0, 0.0),
            view_height: 10.0,
        };
        let base = Some(Point2::new(10.0, 0.0));
        assert_eq!(
            resolve(
                &mut drafting,
                Point2::new(0.8, 0.0),
                base,
                false,
                &far_camera,
                1000.0,
                &SnapIndex::default(),
                &extra,
            ),
            extra[0].point
        );
        assert_eq!(
            resolve(
                &mut drafting,
                Point2::new(0.08, 0.0),
                base,
                false,
                &near_camera,
                1000.0,
                &SnapIndex::default(),
                &extra,
            ),
            extra[0].point
        );
    }

    #[test]
    fn running_endpoint_off_ignores_transient_endpoint() {
        let extra = [SnapFeature {
            point: Point2::new(0.0, 0.0),
            kind: SnapKind::Endpoint,
        }];
        let mut drafting = DraftingState::new(DraftingPreferences {
            running_snaps: RunningSnaps {
                endpoint: false,
                ..RunningSnaps::default()
            },
            ..DraftingPreferences::default()
        });
        let resolved = resolve(
            &mut drafting,
            Point2::new(0.2, 0.0),
            Some(Point2::new(4.0, 0.0)),
            false,
            &zoomed_out(),
            1000.0,
            &SnapIndex::default(),
            &extra,
        );
        assert!(drafting.acquired_snap.is_none());
        assert_eq!(resolved, Point2::new(0.2, 0.0));
    }

    #[test]
    fn override_restricts_kinds_and_clears_after_a_pick() {
        let extra = [
            SnapFeature {
                point: Point2::new(0.0, 0.0),
                kind: SnapKind::Endpoint,
            },
            SnapFeature {
                point: Point2::new(0.3, 0.0),
                kind: SnapKind::Midpoint,
            },
        ];
        let mut drafting = DraftingState::new(DraftingPreferences {
            osnap_enabled: false,
            ..DraftingPreferences::default()
        });
        drafting.snap_override = Some(OneShotSnap::Kind(SnapKind::Midpoint));
        let resolved = resolve(
            &mut drafting,
            Point2::new(0.1, 0.0),
            None,
            false,
            &zoomed_out(),
            1000.0,
            &SnapIndex::default(),
            &extra,
        );
        assert_eq!(resolved, extra[1].point);
        assert_eq!(
            drafting.acquired_snap.map(|feature| feature.kind),
            Some(SnapKind::Midpoint)
        );
        drafting.consume_pick();
        let after = resolve(
            &mut drafting,
            Point2::new(0.1, 0.0),
            None,
            false,
            &zoomed_out(),
            1000.0,
            &SnapIndex::default(),
            &extra,
        );
        assert!(drafting.acquired_snap.is_none());
        assert_eq!(after, Point2::new(0.1, 0.0));
        assert!(drafting.snap_override.is_none());
    }

    #[test]
    fn nearest_loses_to_a_farther_endpoint() {
        let mut document = Document::default();
        document.model_space.push(Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(10.0, 0.0),
        }));
        let edges = MeasureIndex::build(&document);
        let points = SnapIndex::from_features(vec![SnapFeature {
            point: Point2::new(5.4, 0.0),
            kind: SnapKind::Endpoint,
        }]);
        let mut drafting = DraftingState::new(DraftingPreferences {
            running_snaps: RunningSnaps {
                nearest: true,
                midpoint: false,
                ..RunningSnaps::default()
            },
            ..DraftingPreferences::default()
        });
        let resolved = drafting.resolve_point(
            Point2::new(5.0, 0.01),
            None,
            false,
            &zoomed_out(),
            1000.0,
            SnapSources {
                points: &points,
                edges: &edges,
                extra: &[],
                generation: 0,
            },
            0.0,
        );
        assert_eq!(
            drafting.acquired_snap.map(|feature| feature.kind),
            Some(SnapKind::Endpoint)
        );
        assert!((resolved.x - 5.4).abs() < 1e-9);
    }

    #[test]
    fn polar_projects_onto_the_45_degree_ray() {
        let mut drafting = DraftingState::new(DraftingPreferences {
            osnap_enabled: false,
            polar_enabled: true,
            polar_increment_deg: 45.0,
            ..DraftingPreferences::default()
        });
        let raw = Point2::new(
            10.0 * 40.0_f64.to_radians().cos(),
            10.0 * 40.0_f64.to_radians().sin(),
        );
        let resolved = resolve(
            &mut drafting,
            raw,
            Some(Point2::new(0.0, 0.0)),
            false,
            &zoomed_out(),
            1000.0,
            &SnapIndex::default(),
            &[],
        );
        let along = 10.0 * 5.0_f64.to_radians().cos();
        let expected = Point2::new(
            along * 45.0_f64.to_radians().cos(),
            along * 45.0_f64.to_radians().sin(),
        );
        assert!(resolved.distance(expected) < 1e-6);
        let missed = Point2::new(
            10.0 * 20.0_f64.to_radians().cos(),
            10.0 * 20.0_f64.to_radians().sin(),
        );
        let free = resolve(
            &mut drafting,
            missed,
            Some(Point2::new(0.0, 0.0)),
            false,
            &zoomed_out(),
            1000.0,
            &SnapIndex::default(),
            &[],
        );
        assert!(free.distance(missed) < 1e-9);
    }

    #[test]
    fn crossing_tracking_paths_win_over_a_single_path() {
        let mut drafting = DraftingState::new(DraftingPreferences {
            osnap_enabled: false,
            otrack_enabled: true,
            ..DraftingPreferences::default()
        });
        drafting.tracked_points = vec![
            TrackedSnap {
                feature: SnapFeature {
                    point: Point2::new(0.0, 0.0),
                    kind: SnapKind::Endpoint,
                },
            },
            TrackedSnap {
                feature: SnapFeature {
                    point: Point2::new(10.0, 10.0),
                    kind: SnapKind::Endpoint,
                },
            },
        ];
        let resolved = resolve(
            &mut drafting,
            Point2::new(10.2, 0.1),
            None,
            false,
            &zoomed_out(),
            1000.0,
            &SnapIndex::default(),
            &[],
        );
        assert!(resolved.distance(Point2::new(10.0, 0.0)) < 1e-6);
        assert_eq!(
            drafting.tracking_hint.as_ref().map(|hint| hint.rays.len()),
            Some(2)
        );
    }

    #[test]
    fn old_running_snaps_json_still_deserializes() {
        let snaps: RunningSnaps =
            serde_json::from_str(r#"{"endpoint":true,"midpoint":false,"center":true}"#).unwrap();
        assert!(snaps.endpoint);
        assert!(!snaps.midpoint);
        assert!(snaps.center);
        assert!(snaps.intersection);
        assert!(snaps.quadrant);
        assert!(!snaps.nearest);
        assert!(!snaps.tangent);
    }

    #[test]
    fn polar_and_ortho_are_exclusive() {
        let mut preferences = DraftingPreferences {
            ortho_enabled: true,
            polar_enabled: false,
            ..DraftingPreferences::default()
        };
        preferences.toggle_polar();
        assert!(preferences.polar_enabled);
        assert!(!preferences.ortho_enabled);
        preferences.toggle_ortho();
        assert!(preferences.ortho_enabled);
        assert!(!preferences.polar_enabled);
        preferences.polar_enabled = true;
        preferences.sanitize();
        assert!(preferences.ortho_enabled);
        assert!(!preferences.polar_enabled);
    }
}
