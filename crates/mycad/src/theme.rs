//! Basic visual system: dense retro engineering-workstation chrome.

/// Monospace face egui already bundles. It covers arrows and the dropdown
/// triangle that Ubuntu Light does not, so those characters draw as glyphs
/// instead of empty squares. It is a fallback, not the face used for body text.
const UI_SYMBOL_FALLBACK_FONT: &str = "Hack";

pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
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

fn font_definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    if let Some(proportional) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        proportional.push(UI_SYMBOL_FALLBACK_FONT.to_owned());
    }
    fonts
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::font_definitions;

    #[test]
    fn proportional_fonts_cover_every_non_ascii_ui_character() {
        let fonts = font_definitions();
        let family = fonts
            .families
            .get(&egui::FontFamily::Proportional)
            .expect("proportional font family");
        let faces: Vec<ttf_parser::Face<'_>> = family
            .iter()
            .map(|name| {
                let data = fonts
                    .font_data
                    .get(name)
                    .unwrap_or_else(|| panic!("font {name} is listed but has no bytes"));
                ttf_parser::Face::parse(data.font.as_ref(), 0)
                    .unwrap_or_else(|err| panic!("could not parse font {name}: {err}"))
            })
            .collect();

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        visit_sources(&root, &faces, &mut missing);
        if missing.is_empty() {
            return;
        }
        let detail = missing
            .iter()
            .map(|(ch, path, line)| {
                format!(
                    "  {ch} U+{code:04X} {path}:{line}",
                    code = *ch as u32
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        panic!("UI text uses characters no bundled proportional font can draw:\n{detail}");
    }

    fn visit_sources(dir: &Path, faces: &[ttf_parser::Face<'_>], missing: &mut Vec<(char, String, usize)>) {
        let entries = fs::read_dir(dir).unwrap_or_else(|err| panic!("could not read {}: {err}", dir.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|err| panic!("could not read an entry in {}: {err}", dir.display()));
            let path = entry.path();
            if path.is_dir() {
                visit_sources(&path, faces, missing);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("could not read {}: {err}", path.display()));
            for (index, line) in source.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                for ch in line.chars() {
                    if !ch.is_ascii() && !faces.iter().any(|face| face.glyph_index(ch).is_some()) {
                        missing.push((ch, path.display().to_string(), index + 1));
                    }
                }
            }
        }
    }
}
