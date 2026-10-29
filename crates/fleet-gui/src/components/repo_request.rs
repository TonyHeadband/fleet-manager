use crate::theme::{sans, size, tokens, Weight};
use crate::widgets::{button, chip, fui, ButtonVariant};
use eframe::egui::{Align, Layout, RichText, TextEdit, Ui};
use fleet_api::{RepoInfo, RepoRequest};

/// What the owner decided about an agent's repo request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestDecision {
    /// Grant access to this repo of the product.
    Grant(String),
    /// Refuse, with an optional reason (empty when none was typed).
    Refuse(String),
}

/// The box that replaces the reply box while an agent waits for a repo: what it wants and why, the product's repos
/// to grant from (the requested one picked when the product has it), and an optional reason for refusing. A repo
/// the product lacks is added in Products first; nothing here adds one. `target` and `reason` are the caller's,
/// kept per agent; the box never clears them. While `blocked` (a decision is in flight, so a double click cannot
/// send it twice, or the daemon is restarting) Grant and Refuse are disabled, with the reason on hover.
pub fn repo_request_box(
    ui: &mut Ui,
    req: &RepoRequest,
    repos: &[RepoInfo],
    target: &mut String,
    reason: &mut String,
    blocked: Option<&str>,
) -> Option<RequestDecision> {
    let p = tokens(ui.visuals().dark_mode);
    let reason_id = ui.make_persistent_id("repo_request_reason");

    ui.label(
        RichText::new(format!("Wants {}", req.repo))
            .font(sans(ui, size::BODY, Weight::Medium))
            .color(p.text_high),
    );
    if !req.why.is_empty() {
        ui.label(RichText::new(&req.why).color(p.text));
    }
    ui.add_space(8.0);

    let caption = |ui: &mut Ui, text: &str| fui::mono_label(ui, text, p.text_low);
    caption(ui, "Grant access to");
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        for r in repos {
            if chip(ui, &r.name, *target == r.name).clicked() {
                *target = r.name.clone();
            }
        }
    });
    let known = repos.iter().any(|r| r.name == *target);
    if !repos.iter().any(|r| r.name == req.repo) {
        ui.label(
            RichText::new(format!(
                "{} is not a repo of this product: add it in Products first, or refuse",
                req.repo
            ))
            .size(size::SMALL)
            .color(p.text_low),
        );
    }
    ui.add_space(6.0);

    let busy = blocked.is_some();
    let why = blocked.unwrap_or_default();
    let ready = !busy && known;
    let mut decision = None;
    // The inner layout is right-to-left, which would claim the panel's whole height without this row around it.
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let grant = ui
                .add_enabled_ui(ready, |ui| button(ui, "Grant", ButtonVariant::Primary))
                .inner
                .on_disabled_hover_text(why);
            if grant.clicked() && ready {
                decision = Some(RequestDecision::Grant(target.clone()));
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
