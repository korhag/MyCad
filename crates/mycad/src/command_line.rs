//! Docked command line. Typed names use the same aliases as the ribbon.

use eframe::egui::{self, Key, Ui};

use crate::app::MyCadApp;

const MAX_HISTORY: usize = 80;

// ------------------------------------------------------------
// Type: CommandLine
// Purpose: The text the user is typing and a short record of
//          recent commands. The drawing itself is not stored here.
// ------------------------------------------------------------
#[derive(Debug, Clone, Default)]
pub struct CommandLine {
    buffer: String,
    history: Vec<String>,
}

impl CommandLine {
    fn push(&mut self, line: String) {
        if line.trim().is_empty() {
            return;
        }
        self.history.push(line);
        if self.history.len() > MAX_HISTORY {
            let extra = self.history.len() - MAX_HISTORY;
            self.history.drain(0..extra);
        }
    }
}

pub fn show(ui: &mut Ui, app: &mut MyCadApp) {
    let prompt = app.command_prompt().to_string();
    let status = app.status_line().to_string();
    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .max_height((ui.available_height() - 28.0).max(24.0))
        .show(ui, |ui| {
            for line in &app.command_line.history {
                ui.label(line);
            }
            ui.label(prompt);
            if status != app.command_prompt() {
                ui.weak(status);
            }
        });

    let mut buffer = std::mem::take(&mut app.command_line.buffer);
    let response = ui.add(
        egui::TextEdit::singleline(&mut buffer)
            .desired_width(f32::INFINITY)
            .hint_text("Type a command"),
    );
    let enter = ui.input(|input| input.key_pressed(Key::Enter));
    let space = ui.input(|input| input.key_pressed(Key::Space));
    let escape = ui.input(|input| input.key_pressed(Key::Escape));
    if response.has_focus() && escape {
        if buffer.is_empty() {
            app.command_line.buffer = buffer;
            app.cancel_active_command();
        } else {
            buffer.clear();
            app.command_line.buffer = buffer;
        }
        return;
    }
    // Enter moves focus off the field. Space stays focused and still runs the alias.
    if (response.lost_focus() && enter) || (response.has_focus() && (enter || space)) {
        let text = buffer.trim().to_string();
        buffer.clear();
        app.command_line.buffer = buffer;
        let echo = if text.is_empty() {
            "Command: (repeat)".to_string()
        } else {
            format!("Command: {text}")
        };
        app.command_line.push(echo);
        app.run_typed_command(&text);
        app.command_line.push(app.status_line().to_string());
        return;
    }
    app.command_line.buffer = buffer;
}
