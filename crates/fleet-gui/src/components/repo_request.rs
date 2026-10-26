use super::{browse, FolderPicker};
use crate::theme::{mono, sans, size, tokens, Weight};
use crate::widgets::{button, path_edit, ButtonVariant};
use eframe::egui::{Align, Key, Layout, Modifiers, RichText, TextEdit, Ui};
use fleet_api::RepoRequest;

/// What the owner decided about an agent's repo request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestDecision {
    /// Grant access to this target (the requested name, or whatever the owner edited it to).
    Grant(String),
    /// Refuse, with an optional reason (empty when none was typed).
    Refuse(String),
}

/// The box that replaces the reply box while an agent waits for a repo: what it wants and why, the target to
/// grant (editable, so "portal" can become a path or URL), and an optional reason for refusing. Ctrl+Enter
/// grants, and only from the target field. `target` and `reason` are the caller's, kept per agent; the box never
/// clears them. `browse` is this agent's own picker; picking replaces `target`. While `blocked` (a decision is in
/// flight, so a double click or a key repeat cannot send it twice, or the daemon is restarting) Grant and Refuse
/// are disabled, with the reason on hover.
pub fn repo_request_box(
    ui: &mut Ui,
    req: &RepoRequest,
    target: &mut String,
    reason: &mut String,
    browse_picker: &mut FolderPicker,
    blocked: Option<&str>,
) -> Option<RequestDecision> {
    let p = tokens(ui.visuals().dark_mode);
    let target_id = ui.make_persistent_id("repo_request_target");
    let reason_id = ui.make_persistent_id("repo_request_reason");
    // Taken before the fields are drawn. Only while the target has the cursor: the panel sits beside a board and
    // the New task dialog, and Ctrl+Enter there (or in the reason field) must not grant access.
    let ctrl_enter = ui.memory(|m| m.has_focus(target_id))
        && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));

    ui.label(
        RichText::new(format!("Wants {}", req.repo))
            .font(sans(ui, size::BODY, Weight::Medium))
            .color(p.text_high),
    );
    if !req.why.is_empty() {
        ui.label(RichText::new(&req.why).color(p.text));
    }
    ui.add_space(8.0);

    let caption = |ui: &mut Ui, text: &str| {
        ui.label(
            RichText::new(text)
                .font(sans(ui, size::SMALL, Weight::Medium))
                .color(p.text_low),
        )
    };
    let label = caption(ui, "Grant access to");
    ui.add_space(2.0);
    // The inner layout is right-to-left, which would claim the panel's whole height without this row around it.
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // Drawn first, from the right, so the field takes exactly the rest of the row.
            let picked = browse(ui, browse_picker, target);
            let width = ui.available_width();
            path_edit(ui, target_id, target, width, None, "", picked).labelled_by(label.id);
        });
    });
    ui.add_space(6.0);

    let busy = blocked.is_some();
    let why = blocked.unwrap_or_default();
    let ready = !busy && !target.trim().is_empty();
    let mut decision = None;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Ctrl+Enter")
                .font(mono(size::SMALL))
                .color(p.text_low),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let grant = ui
                .add_enabled_ui(ready, |ui| button(ui, "Grant", ButtonVariant::Primary))
                .inner
                .on_disabled_hover_text(why);
            if (grant.clicked() || ctrl_enter) && ready {
                decision = Some(RequestDecision::Grant(target.trim().to_string()));
            }
        });
    });
    ui.add_space(8.0);

    let label = caption(ui, "Reason (optional)");
    ui.add_space(2.0);
    ui.add(
        TextEdit::singleline(reason)
            .id(reason_id)
            .desired_width(f32::INFINITY),
    )
    .labelled_by(label.id);
    ui.add_space(6.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if ui
            .add_enabled_ui(!busy, |ui| button(ui, "Refuse", ButtonVariant::QuietDanger))
            .inner
            .on_disabled_hover_text(why)
            .clicked()
        {
            decision = Some(RequestDecision::Refuse(reason.trim().to_string()));
        }
    });
    decision
}
