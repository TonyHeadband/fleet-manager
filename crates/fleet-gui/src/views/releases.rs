//! The Releases view: the active product's releases with their work and time bars, their tasks, and the actions
//! that change them. Dates are typed as `YYYY-MM-DD`; the daemon validates them and its reason is shown here.
use super::tasks::{outcome_line, Live};
use super::{title, View};
use crate::theme::{radius, sans, size, tokens, Weight};
use crate::widgets::{button, list_row, progress_bar, ButtonVariant};
use eframe::egui::{
    self, accesskit::Role, Frame, Label, Margin, RichText, ScrollArea, TextEdit, Ui, WidgetInfo,
};
use fleet_api::{
    release_progress, task_outcome, today, NewRelease, Release, ReleaseUpdate, Task, TaskOutcome,
    Tone,
};
use std::collections::HashMap;

/// The new-release or edit form on screen.
#[derive(Debug, Default)]
pub struct ReleaseForm {
    /// The release being edited; `None` for a new one.
    pub editing: Option<i64>,
    pub version: String,
    pub start: String,
    pub target: String,
    /// The save is in flight.
    pub busy: bool,
    /// The daemon's reason for refusing the last save.
    pub error: Option<String>,
}

#[derive(Debug, Default)]
pub struct ReleasesState {
    pub form: Option<ReleaseForm>,
    /// The release whose Delete waits for Confirm.
    pub confirm_delete: Option<i64>,
}

/// What the user asked for in the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleasesAction {
    Create(NewRelease),
    /// An edit from the form.
    Update(i64, ReleaseUpdate),
    /// Mark released (`true`) or Unrelease (`false`).
    SetReleased(i64, bool),
    Delete(i64),
    /// A task to another release, or to the backlog.
    Move(String, Option<i64>),
}

pub(crate) fn header(ui: &mut Ui) {
    title(ui, View::Releases);
}

/// `YYYY-MM-DD` of a day number counted from 1970-01-01 (civil-from-days).
pub fn format_day(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Unreleased first, then released; newest start first within each, then by id.
fn ordered(releases: &[Release]) -> Vec<&Release> {
    let mut rows: Vec<&Release> = releases.iter().collect();
    rows.sort_by(|a, b| {
        (
            a.released_at.is_some(),
            std::cmp::Reverse(&a.start),
            std::cmp::Reverse(a.id),
        )
            .cmp(&(
                b.released_at.is_some(),
                std::cmp::Reverse(&b.start),
                std::cmp::Reverse(b.id),
            ))
    });
    rows
}

fn plural(n: u32, unit: &str) -> String {
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

pub(crate) fn main(
    ui: &mut Ui,
    st: &mut ReleasesState,
    product: Option<&str>,
    releases: &[Release],
    tasks: &[Task],
    live: &HashMap<String, Live>,
    now: i64,
) -> Option<ReleasesAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    egui::CentralPanel::default()
        .frame(
            Frame::new()
                .fill(p.bg)
                .inner_margin(Margin::symmetric(22, 18)),
        )
        .show(ui, |ui| {
            let Some(product) = product else {
                ui.label(RichText::new("Pick a product").color(p.text_low));
                return;
            };
            let day = today(now);
            if let Some(a) = toolbar_and_form(ui, st, day) {
                action = Some(a);
            }
            ui.add_space(8.0);
            if releases.is_empty() {
                ui.label(
                    RichText::new("No releases")
                        .size(size::SMALL)
                        .color(p.text_low),
                );
            }
            ScrollArea::vertical()
                .id_salt("releases")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for r in ordered(releases) {
                        let mine: Vec<&Task> = tasks
                            .iter()
                            .filter(|t| t.release == Some(r.id) && t.product == product)
                            .collect();
                        let others: Vec<&Release> = releases
                            .iter()
                            .filter(|o| o.id != r.id && o.released_at.is_none())
                            .collect();
                        let card = ui
                            .push_id(r.id, |ui| card(ui, st, r, tasks, &mine, &others, live, day))
                            .inner;
                        if card.is_some() {
                            action = card;
                        }
                        ui.add_space(10.0);
                    }
                });
        });
    action
}

fn toolbar_and_form(ui: &mut Ui, st: &mut ReleasesState, day: i64) -> Option<ReleasesAction> {
    let p = tokens(ui.visuals().dark_mode);
    let Some(form) = st.form.as_mut() else {
        if button(ui, "New release", ButtonVariant::Primary).clicked() {
            st.form = Some(ReleaseForm {
                start: format_day(day),
                ..ReleaseForm::default()
            });
        }
        return None;
    };
    let mut action = None;
    let mut close = false;
    Frame::new()
        .fill(p.surface)
        .corner_radius(radius::CARD)
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            let heading = if form.editing.is_some() {
                "Edit release"
            } else {
                "New release"
            };
            ui.label(
                RichText::new(heading)
                    .font(sans(ui, size::TITLE, Weight::Medium))
                    .color(p.text_high),
            );
            ui.add_enabled_ui(!form.busy, |ui| {
                ui.horizontal(|ui| {
                    for (label, text, width) in [
                        ("Version", &mut form.version, 110.0),
                        ("Start", &mut form.start, 90.0),
                        ("Target", &mut form.target, 90.0),
                    ] {
                        ui.label(RichText::new(label).size(size::SMALL).color(p.text_low));
                        let edit =
                            ui.add(TextEdit::singleline(text).desired_width(width).hint_text(
                                if label == "Version" {
                                    "1.0"
                                } else {
                                    "YYYY-MM-DD"
                                },
                            ));
                        edit.widget_info(|| WidgetInfo::labeled(Role::TextInput, true, label));
                    }
                });
            });
            if let Some(e) = &form.error {
                ui.label(RichText::new(e).size(size::SMALL).color(p.danger));
            }
            ui.horizontal(|ui| {
                let ready = !form.busy && !form.version.trim().is_empty();
                let label = if form.editing.is_some() {
                    "Save"
                } else {
                    "Create"
                };
                let go = ui
                    .add_enabled_ui(ready, |ui| button(ui, label, ButtonVariant::Primary))
                    .inner;
                if go.clicked() {
                    let (version, start, target) = (
                        form.version.trim().to_string(),
                        form.start.trim().to_string(),
                        form.target.trim().to_string(),
                    );
                    action = Some(match form.editing {
                        Some(id) => ReleasesAction::Update(
                            id,
                            ReleaseUpdate {
                                version: Some(version),
                                start: Some(start),
                                target: Some(target),
                                released: None,
                            },
                        ),
                        None => ReleasesAction::Create(NewRelease {
                            version,
                            start,
                            target,
                        }),
                    });
                }
                if button(ui, "Cancel", ButtonVariant::Ghost).clicked() {
                    close = true;
                }
            });
        });
    if close {
        st.form = None;
    }
    action
}

#[allow(clippy::too_many_arguments)]
fn card(
    ui: &mut Ui,
    st: &mut ReleasesState,
    r: &Release,
    all_tasks: &[Task],
    mine: &[&Task],
    others: &[&Release],
    live: &HashMap<String, Live>,
    day: i64,
) -> Option<ReleasesAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    // Only this release's tasks count, whatever product the list holds.
    let prog = release_progress(r, all_tasks, day);
    Frame::new()
        .fill(p.surface)
        .corner_radius(radius::CARD)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(&r.version)
                        .font(sans(ui, size::HEADING, Weight::Medium))
                        .color(p.text_high),
                );
                ui.label(
                    RichText::new(format!("{} to {}", r.start, r.target))
                        .size(size::SMALL)
                        .color(p.text_low),
                );
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let fraction = if prog.counted == 0 {
                    0.0
                } else {
                    prog.merged as f32 / prog.counted as f32
                };
                let done = prog.counted > 0 && prog.merged == prog.counted;
                progress_bar(ui, fraction, if done { Tone::Done } else { Tone::Active });
                ui.label(
                    RichText::new(format!("{}/{} tasks", prog.merged, prog.counted))
                        .size(size::SMALL)
                        .color(p.text),
                );
                ui.add_space(16.0);
                if prog.overdue_days > 0 && r.released_at.is_none() {
                    progress_bar(ui, 1.0, Tone::Attention);
                    ui.label(
                        RichText::new(format!("{} overdue", plural(prog.overdue_days, "day")))
                            .size(size::SMALL)
                            .color(crate::theme::tone_color(
                                Tone::Attention,
                                ui.visuals().dark_mode,
                            )),
                    );
                } else {
                    let f = prog.days_used as f32 / prog.days_total.max(1) as f32;
                    progress_bar(ui, f, Tone::Active);
                    ui.label(
                        RichText::new(format!("{}/{} days", prog.days_used, prog.days_total))
                            .size(size::SMALL)
                            .color(p.text),
                    );
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                match r.released_at {
                    None => {
                        if button(ui, "Mark released", ButtonVariant::Default).clicked() {
                            action = Some(ReleasesAction::SetReleased(r.id, true));
                        }
                    }
                    Some(at) => {
                        ui.label(
                            RichText::new(format!("Released {}", format_day(today(at))))
                                .size(size::SMALL)
                                .color(p.text_low),
                        );
                        if button(ui, "Unrelease", ButtonVariant::Ghost).clicked() {
                            action = Some(ReleasesAction::SetReleased(r.id, false));
                        }
                    }
                }
                if button(ui, "Edit", ButtonVariant::Ghost).clicked() {
                    st.form = Some(ReleaseForm {
                        editing: Some(r.id),
                        version: r.version.clone(),
                        start: r.start.clone(),
                        target: r.target.clone(),
                        ..ReleaseForm::default()
                    });
                }
                if st.confirm_delete == Some(r.id) {
                    ui.label(
                        RichText::new(format!("Delete {}? Its tasks go to the backlog", r.version))
                            .size(size::SMALL)
                            .color(p.danger),
                    );
                    if button(ui, "Confirm", ButtonVariant::Danger).clicked() {
                        st.confirm_delete = None;
                        action = Some(ReleasesAction::Delete(r.id));
                    }
                    if button(ui, "Cancel", ButtonVariant::Ghost).clicked() {
                        st.confirm_delete = None;
                    }
                } else if button(ui, "Delete", ButtonVariant::QuietDanger).clicked() {
                    st.confirm_delete = Some(r.id);
                }
            });
            if mine.is_empty() {
                return;
            }
            ui.add_space(6.0);
            for t in mine {
                if let Some(a) = task_row(ui, t, others, live) {
                    action = Some(a);
                }
            }
        });
    action
}

fn task_row(
    ui: &mut Ui,
    t: &Task,
    others: &[&Release],
    live: &HashMap<String, Live>,
) -> Option<ReleasesAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    ui.push_id(&t.id.0, |ui| {
        ui.horizontal(|ui| {
            ui.add(Label::new(RichText::new(&t.name).color(p.text_high)).truncate());
            outcome_line(ui, t, "", live);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let locked = matches!(
                    task_outcome(t),
                    TaskOutcome::Merged | TaskOutcome::DoneByHand
                );
                let combo = ui
                    .add_enabled_ui(!locked, |ui| {
                        let combo = egui::ComboBox::from_id_salt("move_to")
                            .selected_text("Move to…")
                            .width(90.0)
                            .show_ui(ui, |ui| {
                                for (to, text) in others
                                    .iter()
                                    .map(|o| (Some(o.id), o.version.as_str()))
                                    .chain([(None, "Backlog")])
                                {
                                    if list_row(ui, text, false, |ui| {
                                        ui.label(RichText::new(text).color(p.text_high));
                                    })
                                    .clicked()
                                    {
                                        action = Some(ReleasesAction::Move(t.id.0.clone(), to));
                                    }
                                }
                            });
                        combo.response.widget_info(|| {
                            WidgetInfo::labeled(Role::ComboBox, ui.is_enabled(), "Move to…")
                        });
                        combo.response
                    })
                    .inner;
                if locked {
                    combo.on_disabled_hover_text("A finished task stays in its release");
                }
            });
        });
    });
    action
}

#[cfg(test)]
mod tests {
    use super::format_day;

    #[test]
    fn formats_days() {
        assert_eq!(format_day(0), "1970-01-01");
        assert_eq!(
            format_day(fleet_api::parse_date("2028-02-29").unwrap()),
            "2028-02-29"
        );
        assert_eq!(
            format_day(fleet_api::parse_date("2026-10-26").unwrap()),
            "2026-10-26"
        );
    }
}
