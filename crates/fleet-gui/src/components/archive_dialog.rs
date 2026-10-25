//! The confirmations: Archive, which deletes folders, End session, which kills a live tmux session nobody owns,
//! Management's Delete persona, and the plain ones (Stop a project, Reset a definition). Each puts the focus on Cancel.
use super::modal_frame;
use crate::theme::{mono, size, tokens};
use crate::widgets::{button, format_bytes, ButtonVariant};
use eframe::egui::{self, Response, RichText, TextStyle, Ui};
use fleet_api::ArchivePlan;

/// The plan the dialog shows. A dialog always starts `Measuring`: a plan is only ever shown for the open it was
/// fetched for, so what the user confirms is what is on disk now.
pub enum PlanView {
    Measuring,
    Ready(ArchivePlan),
    Failed(String),
}

/// An open Archive dialog. `id` is a project id or an orphan id; `name` is what the footer will call it.
pub struct ArchiveDialog {
    pub id: String,
    pub name: String,
    pub orphan: bool,
    pub plan: PlanView,
    /// The archive itself is in flight.
    pub archiving: bool,
    /// Why the plan on screen is not the first one: the daemon refused the confirmed plan as stale.
    pub notice: Option<String>,
    /// Cancel still has to take the focus (on the dialog's first frame).
    pub focus_cancel: bool,
}

impl ArchiveDialog {
    pub fn new(id: String, name: String, orphan: bool) -> ArchiveDialog {
        ArchiveDialog {
            id,
            name,
            orphan,
            plan: PlanView::Measuring,
            archiving: false,
            notice: None,
            focus_cancel: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    Go,
    Cancel,
}

/// Cancel, the safe default. It takes the focus on the dialog's first frame, away from the button that opened it,
/// and whenever nothing has it: the plan arriving above the buttons changes their ids, which drops the focus.
fn cancel_button(ui: &mut Ui, first: &mut bool) -> Response {
    let cancel = button(ui, "Cancel", ButtonVariant::Default);
    if std::mem::take(first) || ui.ctx().memory(|m| m.focused().is_none()) {
        cancel.request_focus();
    }
    cancel
}

fn title(ui: &mut Ui, text: &str) {
    let p = tokens(ui.visuals().dark_mode);
    ui.label(
        RichText::new(text)
            .text_style(TextStyle::Heading)
            .color(p.text_high),
    );
    ui.add_space(6.0);
}

/// The plan and Cancel / Archive. Archive is enabled only with a plan on screen and nothing in flight. Escape and a
/// click outside cancel, except while the archive runs (its result still arrives and is announced).
pub fn archive_dialog(ctx: &egui::Context, d: &mut ArchiveDialog) -> Option<Confirm> {
    let p = tokens(ctx.global_style().visuals.dark_mode);
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::unique("archive_dialog"))
        .frame(modal_frame(ctx))
        .show(ctx, |ui| {
            ui.set_max_width(560.0);
            title(ui, &format!("Archive {}?", d.name));
            if let Some(notice) = &d.notice {
                ui.add(egui::Label::new(RichText::new(notice).color(p.text_high)).wrap());
                ui.add_space(6.0);
            }
            match &d.plan {
                PlanView::Measuring => {
                    ui.label(RichText::new("Measuring…").color(p.text_low));
                }
                PlanView::Failed(e) => {
                    ui.add(egui::Label::new(RichText::new(e).color(p.danger)).wrap());
                }
                PlanView::Ready(plan) => plan_rows(ui, plan),
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if cancel_button(ui, &mut d.focus_cancel).clicked() {
                    answer = Some(Confirm::Cancel);
                }
                let ready = matches!(d.plan, PlanView::Ready(_)) && !d.archiving;
                if ui
                    .add_enabled_ui(ready, |ui| button(ui, "Archive", ButtonVariant::Danger))
                    .inner
                    .clicked()
                {
                    answer = Some(Confirm::Go);
                }
                if d.archiving {
                    ui.label(
                        RichText::new("Archiving…")
                            .size(size::SMALL)
                            .color(p.text_low),
                    );
                }
            });
        });
    if answer.is_none() && modal.should_close() && !d.archiving {
        answer = Some(Confirm::Cancel);
    }
    answer
}

/// One row per item: what is removed with its size and any work it loses, and what stays.
fn plan_rows(ui: &mut Ui, plan: &ArchivePlan) {
    let p = tokens(ui.visuals().dark_mode);
    ui.label(RichText::new("What archiving does:").color(p.text));
    ui.add_space(4.0);
    egui::Grid::new("archive_plan")
        .num_columns(3)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            for item in &plan.items {
                let colour = if item.stays { p.text_low } else { p.text };
                ui.label(
                    RichText::new(&item.path)
                        .font(mono(size::CODE))
                        .color(colour),
                );
                ui.label(RichText::new(format_bytes(item.bytes)).color(colour));
                if item.stays {
                    ui.label(RichText::new("stays").color(p.text_low))
                        .on_hover_text("Kept on disk: archiving does not remove it");
                } else if item.uncommitted_files > 0 {
                    let n = item.uncommitted_files;
                    let files = if n == 1 { "file" } else { "files" };
                    ui.label(
                        RichText::new(format!("{n} uncommitted {files} will be lost"))
                            .color(p.danger),
                    );
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
        });
    ui.add_space(6.0);
    ui.label(RichText::new(format!("Frees {}", format_bytes(plan.total))).color(p.text_high));
    ui.label(
        RichText::new("Branches are kept: only uncommitted work is lost.")
            .size(size::SMALL)
            .color(p.text_low),
    );
}

/// An open End session confirmation for the orphan session `id`, named `session`.
pub struct EndSessionDialog {
    pub id: String,
    pub session: String,
    pub focus_cancel: bool,
}

/// Asks before ending a live session with no project record.
pub fn end_session_dialog(ctx: &egui::Context, d: &mut EndSessionDialog) -> Option<Confirm> {
    let session = d.session.clone();
    let p = tokens(ctx.global_style().visuals.dark_mode);
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::unique("end_session_dialog"))
        .frame(modal_frame(ctx))
        .show(ctx, |ui| {
            ui.set_max_width(520.0);
            title(ui, &format!("End session {session}?"));
            ui.label(
                RichText::new("An agent may be working in this session. End it?").color(p.text),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if cancel_button(ui, &mut d.focus_cancel).clicked() {
                    answer = Some(Confirm::Cancel);
                }
                if button(ui, "End session", ButtonVariant::Danger).clicked() {
                    answer = Some(Confirm::Go);
                }
            });
        });
    if answer.is_none() && modal.should_close() {
        answer = Some(Confirm::Cancel);
    }
    answer
}

/// An open Delete persona confirmation.
pub struct DeletePersonaDialog {
    pub id: String,
    pub name: String,
    pub focus_cancel: bool,
}

/// Asks before deleting a persona: its folder, with both logins, goes; clones holding archived branches stay.
pub fn delete_persona_dialog(ctx: &egui::Context, d: &mut DeletePersonaDialog) -> Option<Confirm> {
    let p = tokens(ctx.global_style().visuals.dark_mode);
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::unique("delete_persona_dialog"))
        .frame(modal_frame(ctx))
        .show(ctx, |ui| {
            ui.set_max_width(520.0);
            title(ui, &format!("Delete {}?", d.name));
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Its folder is removed, with its Claude and GitHub logins, and its remembered repos are \
                         forgotten. Branches its archived projects kept stay where they are.",
                    )
                        .color(p.text),
                )
                .wrap(),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if cancel_button(ui, &mut d.focus_cancel).clicked() {
                    answer = Some(Confirm::Cancel);
                }
                if button(ui, "Delete", ButtonVariant::Danger).clicked() {
                    answer = Some(Confirm::Go);
                }
            });
        });
    if answer.is_none() && modal.should_close() {
        answer = Some(Confirm::Cancel);
    }
    answer
}

/// A plain yes/no confirmation: `title` as the question, `body` under it, Cancel and a danger button named `go`.
pub struct ConfirmDialog {
    pub title: String,
    pub body: String,
    pub go: &'static str,
    pub focus_cancel: bool,
}

impl ConfirmDialog {
    pub fn new(title: String, body: &str, go: &'static str) -> ConfirmDialog {
        ConfirmDialog {
            title,
            body: body.into(),
            go,
            focus_cancel: true,
        }
    }
}

/// Asks `d`'s question in the app, with the focus on Cancel. Escape and a click outside cancel.
pub fn confirm_dialog(ctx: &egui::Context, d: &mut ConfirmDialog) -> Option<Confirm> {
    let p = tokens(ctx.global_style().visuals.dark_mode);
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::unique("confirm_dialog"))
        .frame(modal_frame(ctx))
        .show(ctx, |ui| {
            ui.set_max_width(520.0);
            title(ui, &d.title);
            ui.add(egui::Label::new(RichText::new(&d.body).color(p.text)).wrap());
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if cancel_button(ui, &mut d.focus_cancel).clicked() {
                    answer = Some(Confirm::Cancel);
                }
                if button(ui, d.go, ButtonVariant::Danger).clicked() {
                    answer = Some(Confirm::Go);
                }
            });
        });
    if answer.is_none() && modal.should_close() {
        answer = Some(Confirm::Cancel);
    }
    answer
}
