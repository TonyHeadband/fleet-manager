//! The Releases view: the active product's releases with their work and time bars, their tasks, and the actions
//! that change them. Dates are typed as `YYYY-MM-DD`; the daemon validates them and its reason is shown here.
use super::tasks::{outcome_line, Live};
use super::{title, View};
use crate::components::form::{self, FormValues};
use crate::theme::{radius, releases as space, sans, size, tokens, Weight};
use crate::widgets::{button, list_row, progress_bar, ButtonVariant};
use eframe::egui::{
    self, accesskit::Role, Frame, Label, Margin, RichText, ScrollArea, Ui, WidgetInfo,
};
use fleet_api::{
    release_progress, task_outcome, today, FormDef, NewRelease, Release, ReleaseProgress,
    ReleaseUpdate, Task, TaskOutcome, Tone,
};
use fleet_client::http::FleetError;
use std::collections::HashMap;

/// The new-release or edit form on screen.
#[derive(Debug, Default)]
pub struct ReleaseForm {
    /// The release being edited; `None` for a new one.
    pub editing: Option<i64>,
    pub values: FormValues,
    /// The save is in flight.
    pub busy: bool,
    /// The daemon's reason for refusing the last save.
    pub error: Option<FleetError>,
}

/// The create and edit forms as the daemon describes them.
pub struct ReleaseForms<'a> {
    pub create: &'a FormDef,
    pub edit: &'a FormDef,
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

/// The local calendar day of a Unix time, so "today" and a release day turn over at the user's midnight, not UTC's.
pub fn local_day(at: i64) -> i64 {
    today(at, utc_offset(at))
}

/// Seconds east of UTC on the local clock at `at`, read for that instant so DST follows the date. 0 (UTC) when the
/// system cannot say.
pub fn utc_offset(at: i64) -> i64 {
    let Ok(t) = jiff::Timestamp::from_second(at) else {
        return 0;
    };
    i64::from(jiff::tz::TimeZone::system().to_offset(t).seconds())
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn main(
    ui: &mut Ui,
    st: &mut ReleasesState,
    product: Option<&str>,
    releases: &[Release],
    tasks: &[Task],
    live: &HashMap<String, Live>,
    now: i64,
    forms: ReleaseForms,
) -> Option<ReleasesAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    egui::CentralPanel::default()
        .frame(Frame::new().fill(p.bg).inner_margin(Margin::symmetric(
            crate::theme::layout::PAGE_X as i8,
            crate::theme::layout::PAGE_Y as i8,
        )))
        .show(ui, |ui| {
            let Some(product) = product else {
                ui.label(RichText::new("Pick a product").color(p.text_low));
                return;
            };
            let day = local_day(now);
            if let Some(a) = toolbar_and_form(ui, st, day, &forms) {
                action = Some(a);
            }
            ui.add_space(space::TOOLBAR_GAP);
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
                            .push_id(r.id, |ui| {
                                card(ui, st, r, tasks, &mine, &others, live, day, forms.edit)
                            })
                            .inner;
                        if card.is_some() {
                            action = card;
                        }
                        ui.add_space(crate::theme::layout::CARD_GAP);
                    }
                });
        });
    action
}

/// The release's two bars with their captions: tasks merged out of those that count, and days used (or overdue).
pub(crate) fn release_bars(ui: &mut Ui, r: &Release, prog: &ReleaseProgress) {
    let p = tokens(ui.visuals().dark_mode);
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
        ui.add_space(space::BAR_GAP);
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
}

fn toolbar_and_form(
    ui: &mut Ui,
    st: &mut ReleasesState,
    day: i64,
    forms: &ReleaseForms,
) -> Option<ReleasesAction> {
    let p = tokens(ui.visuals().dark_mode);
    let Some(state) = st.form.as_mut() else {
        if button(ui, "New release", ButtonVariant::Primary).clicked() {
            st.form = Some(ReleaseForm {
                values: form::initial_values(forms.create, day),
                ..ReleaseForm::default()
            });
        }
        return None;
    };
    let def = if state.editing.is_some() {
        forms.edit
    } else {
        forms.create
    };
    let mut action = None;
    let mut close = false;
    Frame::new()
        .fill(p.surface)
        .corner_radius(radius::CARD)
        .inner_margin(Margin::same(space::FORM_MARGIN as i8))
        .show(ui, |ui| {
            ui.label(
                RichText::new(&def.title)
                    .font(sans(ui, size::TITLE, Weight::Medium))
                    .color(p.text_high),
            );
            ui.add_enabled_ui(!state.busy, |ui| {
                form::fields(
                    ui,
                    def,
                    &mut state.values,
                    state.error.as_ref(),
                    &form::Sources::default(),
                )
            });
            if let Some(e) = state
                .error
                .as_ref()
                .filter(|_| form::field_error(def, state.error.as_ref()).is_none())
            {
                ui.label(RichText::new(&e.message).size(size::SMALL).color(p.danger));
            }
            ui.horizontal(|ui| {
                let ready = !state.busy && form::ready(def, &state.values);
                let go = ui
                    .add_enabled_ui(ready, |ui| {
                        button(ui, &def.submit.label, ButtonVariant::Primary)
                    })
                    .inner;
                if go.clicked() {
                    action = submit(state, def);
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

/// The form's values as the typed request the app sends; the description's keys are the request's fields.
fn submit(state: &mut ReleaseForm, def: &FormDef) -> Option<ReleasesAction> {
    let body = form::body(def, &state.values);
    let parsed = match state.editing {
        Some(id) => serde_json::from_value::<ReleaseUpdate>(body).map(|u| {
            ReleasesAction::Update(
                id,
                ReleaseUpdate {
                    released: None,
                    ..u
                },
            )
        }),
        None => serde_json::from_value::<NewRelease>(body).map(ReleasesAction::Create),
    };
    parsed
        .map_err(|e| state.error = Some(e.to_string().into()))
        .ok()
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
    edit: &FormDef,
) -> Option<ReleasesAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    // Only this release's tasks count, whatever product the list holds.
    let prog = release_progress(r, all_tasks, day);
    Frame::new()
        .fill(p.surface)
        .corner_radius(radius::CARD)
        .inner_margin(Margin::same(crate::theme::layout::CARD_PADDING as i8))
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
            ui.add_space(space::TITLE_GAP);
            release_bars(ui, r, &prog);
            ui.add_space(space::ROW_GAP);
            ui.horizontal(|ui| {
                match r.released_at {
                    None => {
                        if button(ui, "Mark released", ButtonVariant::Default).clicked() {
                            action = Some(ReleasesAction::SetReleased(r.id, true));
                        }
                    }
                    Some(at) => {
                        ui.label(
                            RichText::new(format!("Released {}", format_day(local_day(at))))
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
                        values: form::values_from(
                            edit,
                            &serde_json::to_value(r).unwrap_or_default(),
                        ),
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
            ui.add_space(space::ROW_GAP);
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
                            .width(space::MOVE_TO)
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
    use super::{format_day, local_day, utc_offset};

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

    #[test]
    fn local_day_uses_the_system_offset() {
        let at = fleet_api::parse_date("2026-10-26").unwrap() * 86_400 + 43_200;
        let off = utc_offset(at);
        // Real zones run from UTC-12 to UTC+14, in quarter hours.
        assert!((-12 * 3_600..=14 * 3_600).contains(&off), "{off}");
        assert_eq!(off % 900, 0, "{off}");
        assert_eq!(local_day(at), fleet_api::today(at, off));
    }

    #[test]
    fn utc_offset_is_zero_for_a_time_out_of_range() {
        // Far outside what a calendar can show: the contract is "0 (UTC) when the system cannot say".
        assert_eq!(utc_offset(i64::MAX), 0);
    }
}
