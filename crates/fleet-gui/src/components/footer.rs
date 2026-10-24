//! The content footer: one message line (cycling through the queue), and a history popup.
use crate::messages::{Message, Messages};
use crate::theme::{mono, sans, size, tokens, tone_color, Weight};
use crate::widgets::{button, text_dot, ButtonVariant};
use eframe::egui::{self, Align, CursorIcon, Label, Layout, Popup, PopupCloseBehavior, RichText, Sense, Ui};
use fleet_api::Tone;
use fleet_log::Notice;
use std::time::Duration;

/// What the footer asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FooterAction {
    None,
    /// Show Settings → Log.
    OpenLog,
}

const DOT: f32 = 7.0;
const POPUP_WIDTH: f32 = 380.0;

fn notice_color(notice: Notice, dark: bool) -> egui::Color32 {
    match notice {
        Notice::Error => tokens(dark).danger,
        Notice::Warning => tone_color(Tone::Attention, dark),
        Notice::Success => tone_color(Tone::Done, dark),
        Notice::Info => tokens(dark).text_low,
    }
}

/// `HH:MM`, UTC like the log it comes from; there is no local zone available without a date dependency.
fn clock(unix: i64) -> String {
    let secs = unix.rem_euclid(86_400);
    format!("{:02}:{:02}", secs / 3_600, secs / 60 % 60)
}

/// The text on one line: a newline would make the label two rows tall in a footer that fits one.
fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

/// Draws the footer's contents into the panel `ui`. `now` is `ctx.input(|i| i.time)`.
pub fn footer(ui: &mut Ui, m: &mut Messages, now: f64) -> FooterAction {
    let dark = ui.visuals().dark_mode;
    let p = tokens(dark);
    m.tick(now);
    if let Some(wait) = m.next_change(now) {
        // Only the next turn: the window sleeps in between. The margin keeps the repaint from landing a hair early.
        ui.ctx().request_repaint_after(Duration::from_secs_f64(wait + 0.05));
    }
    let font = sans(ui, size::SMALL, Weight::Regular);
    let mut action = FooterAction::None;
    let mut clicked = false;
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if !m.history().is_empty() {
            let toggle = button(ui, "History", ButtonVariant::Ghost);
            let popup = Popup::from_toggle_button_response(&toggle).close_behavior(PopupCloseBehavior::CloseOnClickOutside).width(POPUP_WIDTH);
            if let Some(opened) = popup.show(|ui| history(ui, m.history())) {
                if opened.inner {
                    action = FooterAction::OpenLog;
                }
            }
        }
        if let Some((i, n)) = m.position() {
            ui.label(RichText::new(format!("{i}/{n}")).font(mono(size::META)).color(p.text_low));
        }
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            let Some((message, dimmed)) = m.current() else { return };
            text_dot(ui, notice_color(message.notice, dark), DOT, &font);
            let ink = if dimmed { p.text_low } else { p.text_high };
            let label = Label::new(RichText::new(one_line(&message.text)).font(font.clone()).color(ink)).truncate().sense(Sense::click());
            let mut response = ui.add(label).on_hover_text(&message.text);
            // A dimmed message is only a leftover: clicking it does nothing, so it does not look clickable.
            if !dimmed {
                response = response.on_hover_cursor(CursorIcon::PointingHand);
            }
            clicked = response.clicked();
        });
    });
    if clicked {
        m.click(now);
    }
    action
}

/// The popup's rows, newest first; true when `Open log →` was clicked.
fn history(ui: &mut Ui, messages: &[Message]) -> bool {
    let dark = ui.visuals().dark_mode;
    let p = tokens(dark);
    let font = sans(ui, size::SMALL, Weight::Regular);
    ui.set_width(POPUP_WIDTH);
    for message in messages {
        ui.horizontal(|ui| {
            ui.label(RichText::new(clock(message.time)).font(mono(size::META)).color(p.text_low)).on_hover_text("UTC");
            text_dot(ui, notice_color(message.notice, dark), DOT, &font);
            ui.add(Label::new(RichText::new(one_line(&message.text)).font(font.clone()).color(p.text)).truncate()).on_hover_text(&message.text);
        });
    }
    ui.separator();
    let open = button(ui, "Open log →", ButtonVariant::Ghost).clicked();
    if open {
        ui.close();
    }
    open
}
