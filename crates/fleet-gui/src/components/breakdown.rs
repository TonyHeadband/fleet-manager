//! The side panel's Breakdown tab: the orchestrator's plan, Antonio's decision on it, and the test gate.
use super::specialists::specialist_label;
use crate::theme::{mono, sans, size, tokens, tone_color, Weight};
use crate::widgets::{button, fui, text_dot, ButtonVariant};
use eframe::egui::{self, CollapsingHeader, Grid, Label, RichText, ScrollArea, TextEdit, Ui};
use fleet_api::{Agent, Approval, BreakdownVersion, FleetView, GateRun, GateStep, StepState, Tone};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreakdownAction {
    None,
    Approve,
    /// Request changes, with this comment.
    RequestChanges(String),
    /// Open this agent's side panel.
    Open(String),
}

/// The newest version waits on Antonio's decision.
pub fn pending(view: &FleetView) -> bool {
    view.breakdowns
        .last()
        .is_some_and(|v| v.approval == Approval::Proposed)
}

/// The PR the last gate run opened. An older run's PR is not shown: a later run is what the task stands on.
pub fn pr_url(view: &FleetView) -> Option<&str> {
    view.gate_runs.last()?.pr_url.as_deref()
}

/// The latest version's table and `v{n} · {approval}` line, earlier versions behind a folded History; when
/// `pending`, Approve and a Request changes box (enabled once it has text). Below, the last gate run with its
/// output tail folded, and its PR. `agents` name the step agents; a click on one opens it.
pub fn breakdown_panel(
    ui: &mut Ui,
    view: &FleetView,
    agents: &[&Agent],
    nicknames: &BTreeMap<String, String>,
    pending: bool,
    draft: &mut String,
) -> BreakdownAction {
    breakdown_panel_deciding(ui, view, agents, nicknames, pending, false, draft)
}

/// `breakdown_panel` while a decision may be in flight: with `busy`, the decision controls stay on screen, greyed
/// out (so the panel does not jump), and the rest (agent links, History, the gate output) stays usable.
pub fn breakdown_panel_deciding(
    ui: &mut Ui,
    view: &FleetView,
    agents: &[&Agent],
    nicknames: &BTreeMap<String, String>,
    pending: bool,
    busy: bool,
    draft: &mut String,
) -> BreakdownAction {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = BreakdownAction::None;
    let Some((latest, earlier)) = view.breakdowns.split_last() else {
        note(ui, "No breakdown proposed yet.");
        gate(ui, view);
        return action;
    };
    version_line(ui, latest);
    ui.add_space(4.0);
    steps(ui, latest, agents, nicknames, &mut action);
    if !earlier.is_empty() {
        ui.add_space(6.0);
        CollapsingHeader::new("History")
            .id_salt("breakdown_history")
            .default_open(false)
            .show(ui, |ui| {
                for v in earlier.iter().rev() {
                    version_line(ui, v);
                    steps(ui, v, agents, nicknames, &mut action);
                    ui.add_space(6.0);
                }
            });
    }
    if pending {
        ui.add_enabled_ui(!busy, |ui| {
            ui.add_space(10.0);
            if button(ui, "Approve", ButtonVariant::Primary).clicked() {
                action = BreakdownAction::Approve;
            }
            ui.add_space(8.0);
            let label = fui::mono_label(ui, "Or ask for changes", p.text_low);
            ui.add(
                TextEdit::multiline(draft)
                    .hint_text("What should change?")
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(label.id);
            ui.add_space(4.0);
            let comment = draft.trim().to_string();
            let request = ui
                .add_enabled_ui(!comment.is_empty(), |ui| {
                    button(ui, "Request changes", ButtonVariant::Default)
                })
                .inner;
            if request.clicked() && !comment.is_empty() {
                action = BreakdownAction::RequestChanges(comment);
            }
        });
    }
    ui.add_space(12.0);
    let rule = ui.available_rect_before_wrap();
    ui.painter()
        .hline(rule.x_range(), rule.top(), egui::Stroke::new(1.0, p.border));
    ui.add_space(8.0);
    gate(ui, view);
    action
}

fn approval_text(a: Approval) -> &'static str {
    match a {
        Approval::Proposed => "proposed",
        Approval::Approved => "approved",
        Approval::ChangesRequested => "changes requested",
    }
}

fn version_line(ui: &mut Ui, v: &BreakdownVersion) {
    let p = tokens(ui.visuals().dark_mode);
    ui.label(
        RichText::new(format!("v{} · {}", v.version, approval_text(v.approval)))
            .font(sans(ui, size::SMALL, Weight::Medium))
            .color(p.text_high),
    );
    if let Some(c) = v.comment.as_deref().filter(|c| !c.is_empty()) {
        ui.add(
            Label::new(
                RichText::new(format!("Changes asked: {c}"))
                    .size(size::SMALL)
                    .color(p.text),
            )
            .wrap(),
        );
    }
}

fn state_text(s: StepState) -> (&'static str, Tone) {
    match s {
        StepState::Pending => ("pending", Tone::Neutral),
        StepState::Running => ("running", Tone::Active),
        StepState::Done => ("done", Tone::Done),
        StepState::Failed => ("failed", Tone::Danger),
    }
}

/// `# · Step · Specialist · Depends on · State · Agent`; the agent cell is a link to the agent.
fn steps(
    ui: &mut Ui,
    v: &BreakdownVersion,
    agents: &[&Agent],
    nicknames: &BTreeMap<String, String>,
    action: &mut BreakdownAction,
) {
    let dark = ui.visuals().dark_mode;
    let p = tokens(dark);
    let cell = |text: &str| RichText::new(text).size(size::SMALL).color(p.text);
    Grid::new(("breakdown_steps", v.version))
        .num_columns(6)
        .spacing([10.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            for h in ["#", "Step", "Specialist", "Depends on", "State", "Agent"] {
                fui::mono_label(ui, h, p.text_low);
            }
            ui.end_row();
            for s in &v.steps {
                ui.label(cell(&s.n.to_string()));
                ui.add(Label::new(cell(&s.title)).wrap());
                ui.label(cell(&specialist_label(nicknames, &s.specialist)));
                let deps = if s.depends_on.is_empty() {
                    "—".to_string()
                } else {
                    s.depends_on
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                ui.label(cell(&deps));
                let (word, tone) = state_text(s.state);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let font = sans(ui, size::SMALL, Weight::Regular);
                    text_dot(ui, tone_color(tone, dark), 6.0, &font);
                    ui.label(cell(word));
                });
                match &s.agent {
                    Some(id) => {
                        // An agent archived since is no longer on the board: its id is all there is to show.
                        let name = agents
                            .iter()
                            .find(|a| a.id == *id)
                            .map_or_else(|| id.0.clone(), |a| fleet_client::schema::title(a));
                        if ui.link(RichText::new(name).size(size::SMALL)).clicked() {
                            *action = BreakdownAction::Open(id.0.clone());
                        }
                    }
                    None => {
                        ui.label(cell("—"));
                    }
                }
                ui.end_row();
            }
        });
}

fn gate_text(run: &GateRun) -> (String, Tone) {
    if run.passed() {
        return ("Tests passed".into(), Tone::Done);
    }
    let text = match run.steps.iter().find(|s| s.exit_code != Some(0)) {
        Some(s) => match s.exit_code {
            Some(n) => format!("{} failed (exit {n})", s.name),
            None => format!("{} timed out", s.name),
        },
        None => "No step ran".into(),
    };
    (text, Tone::Danger)
}

fn step_line(s: &GateStep) -> String {
    let result = match s.exit_code {
        Some(0) => "ok".to_string(),
        Some(n) => format!("exit {n}"),
        None => "timed out".into(),
    };
    // A multi-line `run:` would break the one line per step.
    let command = s.command.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{} · {command} · {}s · {result}", s.name, s.duration_secs)
}

/// The last gate run: its result, one line per step it ran, the failed step's output tail folded, and its PR.
fn gate(ui: &mut Ui, view: &FleetView) {
    let dark = ui.visuals().dark_mode;
    let p = tokens(dark);
    let Some(run) = view.gate_runs.last() else {
        note(ui, "The tests run when the orchestrator opens the PR.");
        return;
    };
    let (text, tone) = gate_text(run);
    ui.horizontal(|ui| {
        let font = sans(ui, size::SMALL, Weight::Medium);
        text_dot(ui, tone_color(tone, dark), 7.0, &font);
        ui.label(RichText::new(text).font(font).color(p.text_high));
    });
    for s in &run.steps {
        ui.label(
            RichText::new(step_line(s))
                .font(mono(size::META))
                .color(p.text_low),
        );
    }
    if !run.unmatched.is_empty() {
        note(
            ui,
            &format!(
                "Every step ran: no step covers {}",
                run.unmatched.join(", ")
            ),
        );
    }
    let failed = run.steps.iter().find(|s| s.exit_code != Some(0));
    if let Some(s) = failed.filter(|s| !s.output_tail.is_empty()) {
        CollapsingHeader::new("Output")
            .id_salt(("gate_output", run.at))
            .default_open(false)
            .show(ui, |ui| {
                // Both ways: a log line keeps to one line and scrolls sideways; a vertical-only area would let it
                // widen the whole panel.
                ScrollArea::both()
                    .id_salt("gate_tail")
                    .auto_shrink([false, true])
                    .max_height(240.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.add(
                            Label::new(
                                RichText::new(&s.output_tail)
                                    .font(mono(size::CODE))
                                    .color(p.text),
                            )
                            .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    });
            });
    }
    if let Some(url) = pr_url(view) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            fui::mono_label(ui, "PR", p.text_low);
            ui.hyperlink_to(RichText::new(url).size(size::SMALL), url);
        });
    }
}

fn note(ui: &mut Ui, text: &str) {
    let p = tokens(ui.visuals().dark_mode);
    ui.add(Label::new(RichText::new(text).size(size::SMALL).color(p.text_low)).wrap());
}
