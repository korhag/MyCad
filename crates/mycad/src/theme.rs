//! Basic visual system: dense retro engineering-workstation chrome.

pub fn apply(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
    ctx.set_visuals(egui::Visuals {
        dark_mode: true,
        override_text_color: Some(egui::Color32::from_rgb(214, 220, 214)),
        window_fill: egui::Color32::from_rgb(28, 32, 30),
        panel_fill: egui::Color32::from_rgb(24, 28, 26),
        faint_bg_color: egui::Color32::from_rgb(36, 42, 38),
        extreme_bg_color: egui::Color32::from_rgb(16, 18, 17),
        widgets: egui::style::Widgets {
            noninteractive: widget(egui::Color32::from_rgb(42, 48, 44)),
            inactive: widget(egui::Color32::from_rgb(48, 56, 50)),
            hovered: widget(egui::Color32::from_rgb(62, 78, 64)),
            active: widget(egui::Color32::from_rgb(80, 110, 78)),
            open: widget(egui::Color32::from_rgb(54, 70, 56)),
        },
        selection: egui::style::Selection {
            bg_fill: egui::Color32::from_rgb(70, 110, 78),
            stroke: egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(160, 200, 140)),
        },
        hyperlink_color: egui::Color32::from_rgb(140, 190, 150),
        ..egui::Visuals::dark()
    });
    ctx.style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.window_margin = egui::Margin::same(10);
        style.visuals.window_stroke =
            egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(70, 90, 72));
    });
}

fn widget(fill: egui::Color32) -> egui::style::WidgetVisuals {
    egui::style::WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(70, 86, 72)),
        fg_stroke: egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(220, 228, 214)),
        corner_radius: egui::CornerRadius::ZERO,
        expansion: 0.0,
    }
}
