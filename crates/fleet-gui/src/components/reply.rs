use crate::theme::{mono, size, tokens};
use crate::widgets::{button, ButtonVariant};
use eframe::egui::{Align, Key, Layout, Modifiers, RichText, TextEdit, Ui};

const ROWS: usize = 3;

/// The reply box at the bottom of the side panel. Returns the text to send on Ctrl+Enter or a Send click (the
/// caller clears `text`). Enter inserts a newline. `focus` asks for the cursor this frame.
pub fn reply_box(ui: &mut Ui, label: &str, text: &mut String, focus: bool) -> Option<String> {
    let low = tokens(ui.visuals().dark_mode).text_low;
    let id = ui.make_persistent_id("reply_text");
    // Taken before the TextEdit is drawn, or the edit would insert a newline for it. Only while the field has the
    // cursor: the panel sits beside a board, and Ctrl+Enter elsewhere must not send a half-written answer.
    let ctrl_enter = ui.memory(|m| m.has_focus(id)) && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));

    let edit = ui.add(TextEdit::multiline(text).id(id).desired_rows(ROWS).desired_width(f32::INFINITY).hint_text(format!("{label}…")));
    if focus {
        edit.request_focus();
    }
    ui.add_space(8.0);

    let ready = !text.trim().is_empty();
    let mut send = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Ctrl+Enter").font(mono(size::SMALL)).color(low));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            send = ui.add_enabled_ui(ready, |ui| button(ui, "Send", ButtonVariant::Primary)).inner.clicked();
        });
    });
    ((send || ctrl_enter) && ready).then(|| text.clone())
}
