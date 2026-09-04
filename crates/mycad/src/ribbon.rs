//! Adaptive ribbon: density, overflow packing, and horizontal command chrome.
//!
//! Height chooses Micro / Compact / Normal / Expanded. Width hides lower-priority
//! commands into each group's dropdown. Buttons are content-sized `[icon] Label`.

use eframe::egui::{
    self, Align, Color32, FontId, Layout, Pos2, RichText, Sense, Stroke, StrokeKind, Ui, Vec2,
};

const MICRO_ENTER: f32 = 29.0;
const MICRO_LEAVE: f32 = 31.0;
const COMPACT_ENTER: f32 = 41.0;
const COMPACT_LEAVE: f32 = 43.0;
const NORMAL_ENTER: f32 = 62.0;
const NORMAL_LEAVE: f32 = 66.0;

const MICRO_MAX: f32 = 30.0;
const COMPACT_MAX: f32 = 42.0;
const NORMAL_MAX: f32 = 64.0;

// ------------------------------------------------------------
// Enum: RibbonDensity
// Purpose: Discrete height breakpoint for ribbon chrome.
// ------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RibbonDensity {
    Micro,
    Compact,
    Normal,
    Expanded,
}

// ------------------------------------------------------------
// Type: RibbonMetrics
// Purpose: Continuous sizes inside the current density mode.
// ------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RibbonMetrics {
    pub density: RibbonDensity,
    pub font_size: f32,
    pub icon_size: f32,
    pub row_height: f32,
    pub pad_h: f32,
    pub pad_v: f32,
    pub icon_text_gap: f32,
    pub button_gap: f32,
    pub group_gap: f32,
    pub caption_size: f32,
    pub show_captions: bool,
    pub show_icons: bool,
    pub rows: usize,
}

// ------------------------------------------------------------
// Type: RibbonCommand
// Purpose: One reusable ribbon action; layout is not per-toolbar.
// ------------------------------------------------------------
#[derive(Clone, Copy, Debug)]
pub struct RibbonCommand {
    pub id: &'static str,
    pub label: &'static str,
    pub short_label: &'static str,
    pub icon: &'static str,
    pub tooltip: &'static str,
    pub priority: u8,
    pub enabled: bool,
    pub active: bool,
}

// ------------------------------------------------------------
// Type: RibbonGroup
// Purpose: Named command set that collapses into one overflow menu.
// ------------------------------------------------------------
#[derive(Clone, Debug)]
pub struct RibbonGroup {
    pub id: &'static str,
    pub name: &'static str,
    pub commands: Vec<RibbonCommand>,
}

#[derive(Clone, Debug)]
pub struct LayerChoice {
    pub name: String,
    pub frozen: bool,
    pub color: Color32,
}

#[derive(Clone, Debug)]
pub struct LayerState {
    pub current: String,
    pub current_color: Color32,
    pub layers: Vec<LayerChoice>,
    pub can_set_from_selected: bool,
    pub has_document: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RibbonAction {
    Command(&'static str),
    SetLayer(String),
    SetSelectedLayerCurrent,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeasuredCommand {
    pub width: f32,
    pub priority: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeasuredGroup {
    pub commands: Vec<MeasuredCommand>,
    pub overflow_width: f32,
    pub caption_width: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackedGroup {
    pub visible: Vec<usize>,
    pub overflow: Vec<usize>,
}

pub fn density_from_height(height: f32) -> RibbonDensity {
    if height < MICRO_MAX {
        RibbonDensity::Micro
    } else if height < COMPACT_MAX {
        RibbonDensity::Compact
    } else if height < NORMAL_MAX {
        RibbonDensity::Normal
    } else {
        RibbonDensity::Expanded
    }
}

pub fn resolve_density(height: f32, previous: Option<RibbonDensity>) -> RibbonDensity {
    let target = density_from_height(height);
    let Some(prev) = previous else {
        return target;
    };
    if prev == target {
        return prev;
    }
    match prev {
        RibbonDensity::Micro => {
            if height > MICRO_LEAVE {
                target
            } else {
                prev
            }
        }
        RibbonDensity::Compact => {
            if height < MICRO_ENTER {
                RibbonDensity::Micro
            } else if height > COMPACT_LEAVE {
                target
            } else {
                prev
            }
        }
        RibbonDensity::Normal => {
            if height < COMPACT_ENTER {
                density_from_height(height.min(COMPACT_ENTER - 0.01))
            } else if height > NORMAL_LEAVE {
                RibbonDensity::Expanded
            } else {
                prev
            }
        }
        RibbonDensity::Expanded => {
            if height < NORMAL_ENTER {
                target
            } else {
                prev
            }
        }
    }
}

pub fn density_for(ui: &Ui, height: f32) -> RibbonDensity {
    let id = ui.id().with("ribbon-density");
    let previous = ui.ctx().data(|data| data.get_temp::<RibbonDensity>(id));
    let density = resolve_density(height, previous);
    ui.ctx().data_mut(|data| data.insert_temp(id, density));
    density
}

pub fn metrics_for(density: RibbonDensity, height: f32) -> RibbonMetrics {
    match density {
        RibbonDensity::Micro => RibbonMetrics {
            density,
            font_size: 11.0,
            icon_size: 0.0,
            row_height: height.clamp(16.0, 26.0),
            pad_h: 5.0,
            pad_v: 2.0,
            icon_text_gap: 0.0,
            button_gap: 2.0,
            group_gap: 5.0,
            caption_size: 0.0,
            show_captions: false,
            show_icons: false,
            rows: 1,
        },
        RibbonDensity::Compact => {
            let t = ((height - MICRO_MAX) / (COMPACT_MAX - MICRO_MAX)).clamp(0.0, 1.0);
            RibbonMetrics {
                density,
                font_size: 11.0,
                icon_size: lerp(12.0, 14.0, t),
                row_height: lerp(22.0, 26.0, t).clamp(16.0, height.max(16.0)),
                pad_h: 5.0,
                pad_v: 2.0,
                icon_text_gap: 4.0,
                button_gap: 2.0,
                group_gap: 5.0,
                caption_size: 0.0,
                show_captions: false,
                show_icons: true,
                rows: 1,
            }
        }
        RibbonDensity::Normal => {
            let t = ((height - COMPACT_MAX) / (NORMAL_MAX - COMPACT_MAX)).clamp(0.0, 1.0);
            let row_height = lerp(26.0, 31.0, t);
            let rows = if height >= 52.0 { 2 } else { 1 };
            RibbonMetrics {
                density,
                font_size: lerp(11.0, 13.0, t),
                icon_size: lerp(14.0, 18.0, t),
                row_height,
                pad_h: 5.0,
                pad_v: 3.0,
                icon_text_gap: 4.0,
                button_gap: 2.0,
                group_gap: 5.0,
                caption_size: 0.0,
                show_captions: false,
                show_icons: true,
                rows,
            }
        }
        RibbonDensity::Expanded => {
            let t = ((height - NORMAL_MAX) / 40.0).clamp(0.0, 1.0);
            let show_captions = height >= 70.0;
            let row_height = lerp(31.0, 34.0, t);
            let caption = if show_captions { 10.0 } else { 0.0 };
            let content = (height - caption - 4.0).max(row_height);
            let rows = if content >= row_height * 2.0 + 2.0 {
                2
            } else {
                1
            };
            RibbonMetrics {
                density,
                font_size: lerp(13.0, 14.0, t),
                icon_size: lerp(18.0, 20.0, t),
                row_height,
                pad_h: 6.0,
                pad_v: 3.0,
                icon_text_gap: 4.0,
                button_gap: 2.0,
                group_gap: 6.0,
                caption_size: caption,
                show_captions,
                show_icons: true,
                rows,
            }
        }
    }
}

pub fn command_content_width(text_width: f32, metrics: &RibbonMetrics) -> f32 {
    let inner = if metrics.show_icons {
        metrics.icon_size + metrics.icon_text_gap + text_width
    } else {
        text_width
    };
    metrics.pad_h * 2.0 + inner
}

pub fn pack_ribbon(
    groups: &[MeasuredGroup],
    available_width: f32,
    button_gap: f32,
    group_gap: f32,
    rows: usize,
) -> Vec<PackedGroup> {
    let mut hidden: Vec<Vec<bool>> = groups
        .iter()
        .map(|group| vec![false; group.commands.len()])
        .collect();
    loop {
        let widths: Vec<f32> = groups
            .iter()
            .enumerate()
            .map(|(index, group)| group_width(group, &hidden[index], button_gap))
            .collect();
        if groups_fit(&widths, available_width, rows.max(1), group_gap) {
            break;
        }
        let Some((group_index, command_index)) = next_hide_victim(groups, &hidden) else {
            break;
        };
        hidden[group_index][command_index] = true;
    }
    hidden
        .iter()
        .map(|flags| PackedGroup {
            visible: flags
                .iter()
                .enumerate()
                .filter(|(_, hidden)| !**hidden)
                .map(|(index, _)| index)
                .collect(),
            overflow: flags
                .iter()
                .enumerate()
                .filter(|(_, hidden)| **hidden)
                .map(|(index, _)| index)
                .collect(),
        })
        .collect()
}

pub fn groups_fit(widths: &[f32], row_width: f32, max_rows: usize, group_gap: f32) -> bool {
    if widths.is_empty() {
        return true;
    }
    let mut rows_used = 1usize;
    let mut x = 0.0f32;
    for (index, &width) in widths.iter().enumerate() {
        if width > row_width + 0.5 {
            return false;
        }
        if index == 0 {
            x = width;
            continue;
        }
        let need = group_gap + width;
        if x > 0.0 && x + need > row_width {
            rows_used += 1;
            x = width;
            if rows_used > max_rows {
                return false;
            }
        } else {
            x += need;
        }
    }
    rows_used <= max_rows
}

pub fn packed_group_width(group: &MeasuredGroup, packed: &PackedGroup, button_gap: f32) -> f32 {
    let hidden: Vec<bool> = (0..group.commands.len())
        .map(|index| packed.overflow.contains(&index))
        .collect();
    group_width(group, &hidden, button_gap)
}

pub fn show(
    ui: &mut Ui,
    groups: &[RibbonGroup],
    layer: Option<&LayerState>,
    metrics: &RibbonMetrics,
) -> Option<RibbonAction> {
    let available = ui.available_size();
    let layer_width = layer
        .map(|state| estimate_layer_width(ui, state, metrics, available.x))
        .unwrap_or(0.0);
    let separator = if layer.is_some() {
        metrics.group_gap
    } else {
        0.0
    };
    let command_width = (available.x - layer_width - separator).max(32.0);
    let measured: Vec<MeasuredGroup> = groups
        .iter()
        .map(|group| measure_group(ui, group, metrics))
        .collect();
    let packed = pack_ribbon(
        &measured,
        command_width,
        metrics.button_gap,
        metrics.group_gap,
        metrics.rows,
    );
    let packed_widths: Vec<f32> = measured
        .iter()
        .zip(packed.iter())
        .map(|(group, plan)| packed_group_width(group, plan, metrics.button_gap))
        .collect();
    let emergency = !groups_fit(
        &packed_widths,
        command_width,
        metrics.rows.max(1),
        metrics.group_gap,
    );

    let mut action = None;
    ui.spacing_mut().item_spacing = Vec2::new(metrics.button_gap, 1.0);
    ui.horizontal(|ui| {
        let command_size = Vec2::new(command_width, available.y);
        ui.allocate_ui(command_size, |ui| {
            if emergency {
                egui::ScrollArea::horizontal()
                    .id_salt("ribbon-overflow-scroll")
                    .auto_shrink([false, true])
                    .scroll_bar_visibility(
                        egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                    )
                    .show(ui, |ui| {
                        action = draw_command_row(ui, groups, &packed, metrics, false);
                    });
            } else {
                action = draw_command_row(ui, groups, &packed, metrics, metrics.rows > 1);
            }
        });
        if let Some(state) = layer {
            ui.allocate_ui(Vec2::new(layer_width, available.y), |ui| {
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    if let Some(layer_action) = layer_control(ui, state, metrics, available.x) {
                        action = Some(layer_action);
                    }
                });
            });
        }
    });
    action
}

fn draw_command_row(
    ui: &mut Ui,
    groups: &[RibbonGroup],
    packed: &[PackedGroup],
    metrics: &RibbonMetrics,
    wrap: bool,
) -> Option<RibbonAction> {
    let mut action = None;
    ui.with_layout(
        Layout::left_to_right(Align::Center).with_main_wrap(wrap),
        |ui| {
            let row_left = ui.cursor().left();
            for (index, group) in groups.iter().enumerate() {
                if index > 0 {
                    group_separator(ui, metrics, row_left);
                }
                if let Some(id) = draw_group(ui, group, &packed[index], metrics) {
                    action = Some(RibbonAction::Command(id));
                }
            }
        },
    );
    action
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

fn measure_text(ui: &Ui, text: &str, size: f32) -> f32 {
    ui.fonts(|fonts| {
        fonts
            .layout_no_wrap(text.to_owned(), FontId::proportional(size), Color32::WHITE)
            .size()
            .x
    })
}

fn command_label(command: &RibbonCommand, metrics: &RibbonMetrics) -> &'static str {
    if metrics.density == RibbonDensity::Micro {
        command.short_label
    } else {
        command.label
    }
}

fn overflow_caption(name: &str) -> String {
    format!("{name} ▾")
}

fn measure_group(ui: &Ui, group: &RibbonGroup, metrics: &RibbonMetrics) -> MeasuredGroup {
    let commands = group
        .commands
        .iter()
        .map(|command| MeasuredCommand {
            width: command_content_width(
                measure_text(ui, command_label(command, metrics), metrics.font_size),
                metrics,
            ),
            priority: command.priority,
        })
        .collect();
    let overflow_width =
        metrics.pad_h * 2.0 + measure_text(ui, &overflow_caption(group.name), metrics.font_size);
    let caption_width = if metrics.show_captions {
        measure_text(ui, group.name, metrics.caption_size)
    } else {
        0.0
    };
    MeasuredGroup {
        commands,
        overflow_width,
        caption_width,
    }
}

fn group_width(group: &MeasuredGroup, hidden: &[bool], button_gap: f32) -> f32 {
    let visible: Vec<f32> = group
        .commands
        .iter()
        .enumerate()
        .filter(|(index, _)| !hidden.get(*index).copied().unwrap_or(false))
        .map(|(_, command)| command.width)
        .collect();
    let commands_width = if visible.is_empty() {
        0.0
    } else {
        visible.iter().sum::<f32>() + button_gap * (visible.len() - 1) as f32
    };
    let any_hidden = hidden.iter().any(|flag| *flag);
    let with_overflow = if any_hidden {
        if commands_width <= 0.0 {
            group.overflow_width
        } else {
            commands_width + button_gap + group.overflow_width
        }
    } else {
        commands_width
    };
    with_overflow.max(group.caption_width)
}

fn next_hide_victim(groups: &[MeasuredGroup], hidden: &[Vec<bool>]) -> Option<(usize, usize)> {
    let mut victim: Option<(u8, usize, usize)> = None;
    for (group_index, group) in groups.iter().enumerate() {
        for (command_index, command) in group.commands.iter().enumerate() {
            if hidden[group_index][command_index] {
                continue;
            }
            let key = (command.priority, group_index, command_index);
            if victim.map(|previous| key > previous).unwrap_or(true) {
                victim = Some(key);
            }
        }
    }
    victim.map(|(_, group_index, command_index)| (group_index, command_index))
}

fn group_separator(ui: &mut Ui, metrics: &RibbonMetrics, row_left: f32) {
    let height = if metrics.show_captions {
        metrics.caption_size + 2.0 + metrics.row_height
    } else {
        metrics.row_height
    };
    if ui.cursor().left() <= row_left + 1.0 {
        return;
    }
    let (rect, _) = ui.allocate_exact_size(Vec2::new(metrics.group_gap, height), Sense::hover());
    let stroke = Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color);
    ui.painter()
        .vline(rect.center().x, rect.y_range().shrink(2.0), stroke);
}

fn draw_group(
    ui: &mut Ui,
    group: &RibbonGroup,
    packed: &PackedGroup,
    metrics: &RibbonMetrics,
) -> Option<&'static str> {
    let mut clicked = None;
    ui.push_id(group.id, |ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(metrics.button_gap, 1.0);
            if metrics.show_captions {
                ui.label(
                    RichText::new(group.name)
                        .size(metrics.caption_size)
                        .weak()
                        .small(),
                );
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(metrics.button_gap, 0.0);
                for &index in &packed.visible {
                    let command = &group.commands[index];
                    if command_button(ui, command, metrics).clicked() {
                        clicked = Some(command.id);
                    }
                }
                if !packed.overflow.is_empty() {
                    if let Some(id) = overflow_menu(ui, group, &packed.overflow, metrics) {
                        clicked = Some(id);
                    }
                }
            });
        });
    });
    clicked
}

fn overflow_menu(
    ui: &mut Ui,
    group: &RibbonGroup,
    overflow: &[usize],
    metrics: &RibbonMetrics,
) -> Option<&'static str> {
    let label = overflow_caption(group.name);
    let response = chip_button(ui, None, &label, true, false, metrics)
        .on_hover_text(format!("More {} commands", group.name.to_lowercase()));
    let mut chosen = None;
    if let Some(inner) = egui::Popup::menu(&response).show(|ui| {
        for &index in overflow {
            let command = &group.commands[index];
            let text = if metrics.show_icons {
                format!("{}  {}", command.icon, command.label)
            } else {
                command.label.to_string()
            };
            let item = ui.add_enabled(
                command.enabled,
                egui::Button::selectable(command.active, text),
            );
            let item = if command.enabled {
                item.on_hover_text(command.tooltip)
            } else {
                item.on_disabled_hover_text(command.tooltip)
            };
            if item.clicked() {
                return Some(command.id);
            }
        }
        None
    }) {
        chosen = inner.inner;
    }
    chosen
}

fn command_button(ui: &mut Ui, command: &RibbonCommand, metrics: &RibbonMetrics) -> egui::Response {
    let label = command_label(command, metrics);
    let icon = metrics.show_icons.then_some(command.icon);
    let response = chip_button(ui, icon, label, command.enabled, command.active, metrics);
    if command.enabled {
        response.on_hover_text(command.tooltip)
    } else {
        response.on_disabled_hover_text(command.tooltip)
    }
}

fn chip_button(
    ui: &mut Ui,
    icon: Option<&str>,
    label: &str,
    enabled: bool,
    active: bool,
    metrics: &RibbonMetrics,
) -> egui::Response {
    let text_width = measure_text(ui, label, metrics.font_size);
    let width = if icon.is_some() && metrics.show_icons {
        command_content_width(text_width, metrics)
    } else {
        metrics.pad_h * 2.0 + text_width
    };
    let size = Vec2::new(width, metrics.row_height);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    paint_chip(ui, rect, enabled, active, &response);
    let (icon_color, text_color) = chip_colors(ui, enabled, active);
    let mut x = rect.left() + metrics.pad_h;
    let center_y = rect.center().y;
    if let Some(icon) = icon.filter(|_| metrics.show_icons) {
        ui.painter().text(
            Pos2::new(x + metrics.icon_size * 0.5, center_y),
            egui::Align2::CENTER_CENTER,
            icon,
            FontId::proportional(metrics.icon_size),
            icon_color,
        );
        x += metrics.icon_size + metrics.icon_text_gap;
    }
    ui.painter().text(
        Pos2::new(x, center_y),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(metrics.font_size),
        text_color,
    );
    response
}

fn paint_chip(ui: &Ui, rect: egui::Rect, enabled: bool, active: bool, response: &egui::Response) {
    let hovered = enabled && response.hovered();
    let pressed = enabled && response.is_pointer_button_down_on();
    let fill = if !enabled {
        Color32::TRANSPARENT
    } else if active {
        ui.visuals().selection.bg_fill
    } else if pressed {
        ui.visuals().widgets.active.bg_fill
    } else if hovered {
        ui.visuals().widgets.hovered.bg_fill.linear_multiply(0.55)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 2.0, fill);
    let stroke = if active {
        ui.visuals().selection.stroke
    } else if hovered && enabled {
        Stroke::new(1.0, ui.visuals().widgets.hovered.bg_stroke.color)
    } else {
        Stroke::NONE
    };
    if stroke.width > 0.0 {
        ui.painter()
            .rect_stroke(rect, 2.0, stroke, StrokeKind::Inside);
    }
}

fn chip_colors(ui: &Ui, enabled: bool, active: bool) -> (Color32, Color32) {
    if !enabled {
        let weak = ui.visuals().weak_text_color();
        (weak, weak)
    } else if active {
        let color = ui.visuals().strong_text_color();
        (color, color)
    } else {
        (
            ui.visuals().widgets.inactive.fg_stroke.color,
            ui.visuals().text_color(),
        )
    }
}

fn estimate_layer_width(
    ui: &Ui,
    state: &LayerState,
    metrics: &RibbonMetrics,
    total_width: f32,
) -> f32 {
    if !state.has_document {
        return command_content_width(measure_text(ui, "No drawing", metrics.font_size), metrics);
    }
    let show_set = show_set_from_selected(metrics.density, total_width);
    let label = layer_chip_label(ui, state, metrics);
    let mut width = command_content_width(measure_text(ui, &label, metrics.font_size), metrics);
    if metrics.density != RibbonDensity::Micro {
        width += 12.0 + 4.0;
    }
    if show_set {
        width += metrics.button_gap
            + command_content_width(
                measure_text(ui, "Set from selected", metrics.font_size),
                metrics,
            );
    }
    width
}

fn show_set_from_selected(density: RibbonDensity, total_width: f32) -> bool {
    match density {
        RibbonDensity::Micro | RibbonDensity::Compact => false,
        RibbonDensity::Normal => total_width >= 860.0,
        RibbonDensity::Expanded => true,
    }
}

fn layer_name_budget(density: RibbonDensity) -> f32 {
    match density {
        RibbonDensity::Micro => 0.0,
        RibbonDensity::Compact => 90.0,
        RibbonDensity::Normal => 120.0,
        RibbonDensity::Expanded => 200.0,
    }
}

fn ellipsize(ui: &Ui, text: &str, font_size: f32, max_width: f32) -> String {
    if max_width <= 0.0 || measure_text(ui, text, font_size) <= max_width {
        return text.to_owned();
    }
    let mut truncated = text.to_owned();
    while truncated.len() > 1 {
        truncated.pop();
        let candidate = format!("{truncated}…");
        if measure_text(ui, &candidate, font_size) <= max_width {
            return candidate;
        }
    }
    "…".into()
}

fn layer_chip_label(ui: &Ui, state: &LayerState, metrics: &RibbonMetrics) -> String {
    match metrics.density {
        RibbonDensity::Micro => "Layer ▾".into(),
        RibbonDensity::Compact | RibbonDensity::Normal => {
            let name = ellipsize(
                ui,
                &state.current,
                metrics.font_size,
                layer_name_budget(metrics.density),
            );
            format!("{name} ▾")
        }
        RibbonDensity::Expanded => {
            let name = ellipsize(
                ui,
                &state.current,
                metrics.font_size,
                layer_name_budget(metrics.density),
            );
            format!("Layer: {name} ▾")
        }
    }
}

fn layer_control(
    ui: &mut Ui,
    state: &LayerState,
    metrics: &RibbonMetrics,
    total_width: f32,
) -> Option<RibbonAction> {
    if !state.has_document {
        ui.label(RichText::new("No drawing").size(metrics.font_size).weak());
        return None;
    }
    let mut action = None;
    if metrics.density != RibbonDensity::Micro {
        paint_swatch(ui, state.current_color, metrics);
    }
    let label = layer_chip_label(ui, state, metrics);
    let response =
        chip_button(ui, None, &label, true, false, metrics).on_hover_text("Current layer");
    if let Some(inner) = egui::Popup::menu(&response).show(|ui| {
        let mut chosen = None;
        for layer in &state.layers {
            ui.horizontal(|ui| {
                paint_swatch(ui, layer.color, metrics);
                let text = if layer.frozen {
                    format!("{} (frozen)", layer.name)
                } else {
                    layer.name.clone()
                };
                if ui
                    .add_enabled(
                        !layer.frozen,
                        egui::Button::selectable(layer.name == state.current, text),
                    )
                    .clicked()
                {
                    chosen = Some(layer.name.clone());
                }
            });
        }
        chosen
    }) {
        if let Some(name) = inner.inner {
            action = Some(RibbonAction::SetLayer(name));
        }
    }
    if show_set_from_selected(metrics.density, total_width) {
        let set = chip_button(
            ui,
            None,
            "Set from selected",
            state.can_set_from_selected,
            false,
            metrics,
        );
        let set = if state.can_set_from_selected {
            set.on_hover_text("Use the selected object's layer")
        } else {
            set.on_disabled_hover_text("Use the selected object's layer")
        };
        if set.clicked() {
            action = Some(RibbonAction::SetSelectedLayerCurrent);
        }
    }
    action
}

fn paint_swatch(ui: &mut Ui, color: Color32, metrics: &RibbonMetrics) {
    let side = metrics.icon_size.clamp(8.0, 14.0).max(10.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
    ui.painter().rect_filled(rect, 1.0, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(width: f32, priority: u8) -> MeasuredCommand {
        MeasuredCommand { width, priority }
    }

    fn group(commands: Vec<MeasuredCommand>, overflow_width: f32) -> MeasuredGroup {
        MeasuredGroup {
            commands,
            overflow_width,
            caption_width: 0.0,
        }
    }

    #[test]
    fn density_breakpoints_match_the_height_table() {
        assert_eq!(density_from_height(28.0), RibbonDensity::Micro);
        assert_eq!(density_from_height(30.0), RibbonDensity::Compact);
        assert_eq!(density_from_height(41.9), RibbonDensity::Compact);
        assert_eq!(density_from_height(42.0), RibbonDensity::Normal);
        assert_eq!(density_from_height(63.9), RibbonDensity::Normal);
        assert_eq!(density_from_height(64.0), RibbonDensity::Expanded);
    }

    #[test]
    fn hysteresis_avoids_flashing_around_micro() {
        assert_eq!(
            resolve_density(30.5, Some(RibbonDensity::Micro)),
            RibbonDensity::Micro
        );
        assert_eq!(
            resolve_density(31.5, Some(RibbonDensity::Micro)),
            RibbonDensity::Compact
        );
        assert_eq!(
            resolve_density(29.5, Some(RibbonDensity::Compact)),
            RibbonDensity::Compact
        );
        assert_eq!(
            resolve_density(28.5, Some(RibbonDensity::Compact)),
            RibbonDensity::Micro
        );
    }

    #[test]
    fn hysteresis_holds_normal_and_expanded() {
        assert_eq!(
            resolve_density(42.5, Some(RibbonDensity::Compact)),
            RibbonDensity::Compact
        );
        assert_eq!(
            resolve_density(43.5, Some(RibbonDensity::Compact)),
            RibbonDensity::Normal
        );
        assert_eq!(
            resolve_density(63.0, Some(RibbonDensity::Expanded)),
            RibbonDensity::Expanded
        );
        assert_eq!(
            resolve_density(61.0, Some(RibbonDensity::Expanded)),
            RibbonDensity::Normal
        );
    }

    #[test]
    fn normal_metrics_lerp_inside_the_breakpoint() {
        let low = metrics_for(RibbonDensity::Normal, 42.0);
        let high = metrics_for(RibbonDensity::Normal, 64.0);
        assert!((low.font_size - 11.0).abs() < 1e-4);
        assert!((high.font_size - 13.0).abs() < 1e-4);
        assert!((low.icon_size - 14.0).abs() < 1e-4);
        assert!((high.icon_size - 18.0).abs() < 1e-4);
        assert!(high.row_height > low.row_height);
        assert_eq!(low.rows, 1);
        assert_eq!(metrics_for(RibbonDensity::Normal, 52.0).rows, 2);
        assert!(!low.show_icons || low.density == RibbonDensity::Normal);
        assert!(low.show_icons);
        assert!(!low.show_captions);
    }

    #[test]
    fn micro_is_text_only_and_single_row() {
        let metrics = metrics_for(RibbonDensity::Micro, 28.0);
        assert!(!metrics.show_icons);
        assert_eq!(metrics.rows, 1);
        assert_eq!(metrics.icon_size, 0.0);
    }

    #[test]
    fn content_width_uses_horizontal_icon_and_label() {
        let metrics = metrics_for(RibbonDensity::Compact, 36.0);
        let width = command_content_width(40.0, &metrics);
        let expected = metrics.pad_h * 2.0 + metrics.icon_size + metrics.icon_text_gap + 40.0;
        assert!((width - expected).abs() < 1e-4);
        let micro = metrics_for(RibbonDensity::Micro, 24.0);
        assert!((command_content_width(40.0, &micro) - (micro.pad_h * 2.0 + 40.0)).abs() < 1e-4);
    }

    #[test]
    fn overflow_hides_lowest_priority_first() {
        let groups = vec![
            group(vec![cmd(40.0, 0), cmd(40.0, 1)], 36.0),
            group(vec![cmd(50.0, 0), cmd(50.0, 1), cmd(50.0, 4)], 40.0),
        ];
        let packed = pack_ribbon(&groups, 232.0, 2.0, 5.0, 1);
        assert_eq!(packed[0].visible, vec![0, 1]);
        assert!(packed[0].overflow.is_empty());
        assert_eq!(packed[1].visible, vec![0, 1]);
        assert_eq!(packed[1].overflow, vec![2]);
    }

    #[test]
    fn overflow_collapses_a_group_to_its_menu() {
        let groups = vec![group(vec![cmd(80.0, 0), cmd(80.0, 1), cmd(80.0, 2)], 48.0)];
        let packed = pack_ribbon(&groups, 50.0, 2.0, 5.0, 1);
        assert!(packed[0].visible.is_empty());
        assert_eq!(packed[0].overflow, vec![0, 1, 2]);
    }

    #[test]
    fn two_rows_keep_more_commands_visible() {
        let groups = vec![
            group(vec![cmd(70.0, 0), cmd(70.0, 1)], 40.0),
            group(vec![cmd(70.0, 0), cmd(70.0, 1)], 40.0),
            group(vec![cmd(70.0, 0), cmd(70.0, 1)], 40.0),
        ];
        let one = pack_ribbon(&groups, 160.0, 2.0, 5.0, 1);
        let two = pack_ribbon(&groups, 160.0, 2.0, 5.0, 2);
        let visible = |packed: &[PackedGroup]| {
            packed
                .iter()
                .map(|group| group.visible.len())
                .sum::<usize>()
        };
        assert!(visible(&two) > visible(&one));
    }

    #[test]
    fn packing_is_stable_for_nearby_widths() {
        let groups = vec![group(
            vec![cmd(40.0, 0), cmd(48.0, 1), cmd(56.0, 2), cmd(64.0, 3)],
            40.0,
        )];
        let a = pack_ribbon(&groups, 200.0, 2.0, 5.0, 1);
        let b = pack_ribbon(&groups, 201.0, 2.0, 5.0, 1);
        assert_eq!(a, b);
        let wider = pack_ribbon(&groups, 400.0, 2.0, 5.0, 1);
        assert!(wider[0].overflow.is_empty());
        assert_eq!(wider[0].visible.len(), 4);
    }

    #[test]
    fn groups_wrap_instead_of_scrolling_when_two_rows_fit() {
        assert!(groups_fit(&[80.0, 80.0, 80.0], 170.0, 2, 5.0));
        assert!(!groups_fit(&[80.0, 80.0, 80.0], 170.0, 1, 5.0));
    }
}
