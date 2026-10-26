//! The board view: columns of cards, filtered by repo and persona.
use super::{title, TaskFilter, View};
use crate::components::{board, persona_filter, repo_filter, CardState, Command};
use crate::theme::{sans, size, tokens, tone_color, Weight};
use crate::widgets::{badge, chip, format_bytes, text_dot, BadgeTone};
use eframe::egui::{self, vec2, Align, CursorIcon, Frame, Layout, RichText, Sense, Ui, UiBuilder};
use fleet_api::{Agent, StopCause, TaskList, TaskState, Tone, UiSchema};
use fleet_client::config::FleetDisplay;
use std::collections::HashMap;

/// The strip's task counts, from the tasks list as last fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Counts {
    running: usize,
    /// Every cause, crashed included, as the Tasks view's totals count them.
    stopped: usize,
    crashed: usize,
    orphans: usize,
    /// What the tasks and orphans still hold on disk; archived ones hold nothing.
    held: u64,
}

impl Counts {
    fn of(list: &TaskList) -> Counts {
        let mut c = Counts {
            running: 0,
            stopped: 0,
            crashed: 0,
            orphans: list.orphans.len(),
            held: list.orphans.iter().filter_map(|o| o.bytes).sum(),
        };
        for p in &list.tasks {
            match p.state {
                TaskState::Running => c.running += 1,
                TaskState::Stopped => {
                    c.stopped += 1;
                    if p.stop_cause == Some(StopCause::Crashed) {
                        c.crashed += 1;
                    }
                }
                TaskState::Archived => continue,
            }
            c.held += p.footprint.as_ref().map_or(0, |f| f.total);
        }
        c
    }
}

/// A count's row height: a badge's, as on the cards.
const COUNT_HEIGHT: f32 = 17.0;

/// A count that opens the Tasks view filtered to it when clicked. The Ui senses the click before its contents
/// register, so the label and badge inside keep their own hover. It keeps the strip's right-to-left layout (a
/// left-to-right child would start at the far left of the row), so `add` draws its parts last first.
fn count(ui: &mut Ui, filter: TaskFilter, add: impl FnOnce(&mut Ui)) -> Option<TaskFilter> {
    let layout = Layout::right_to_left(Align::Center);
    // In the strip's full-height centred row, a badge's frame would grow to the whole row: give it a short one.
    let response = ui
        .allocate_ui_with_layout(vec2(ui.available_width(), COUNT_HEIGHT), layout, |ui| {
            ui.scope_builder(
                UiBuilder::new()
                    .id_salt(("task_count", filter))
                    .sense(Sense::click()),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    // Labels must not swallow the click.
                    ui.style_mut().interaction.selectable_labels = false;
                    add(ui);
                },
            )
            .response
        })
        .inner
        .on_hover_cursor(CursorIcon::PointingHand);
    response.clicked().then_some(filter)
}

/// "Tasks: ● N running ● N stopped [N crashed] [N orphan] · <disk>". Drawn right to left from the strip's end,
/// so the calls run in reverse reading order. Returns the filter of a clicked count.
fn task_counts(ui: &mut Ui, c: Counts, stopped_tone: Tone) -> Option<TaskFilter> {
    let dark = ui.visuals().dark_mode;
    let p = tokens(dark);
    let font = sans(ui, size::SMALL, Weight::Regular);
    let mut clicked = None;
    ui.label(
        RichText::new(format!("· {}", format_bytes(c.held)))
            .font(font.clone())
            .color(p.text_low),
    );
    // No ⚠ before it: glyphs are painted in this app and no bundled font carries that one; the amber badge is the
    // Tasks view's Orphan look.
    if c.orphans > 0 {
        clicked = clicked.or(count(ui, TaskFilter::Orphans, |ui| {
            badge(
                ui,
                &format!("{} orphan", c.orphans),
                "Show orphans",
                BadgeTone::Tinted(Tone::Caution),
            );
        }));
    }
    if c.crashed > 0 {
        clicked = clicked.or(count(ui, TaskFilter::Crashed, |ui| {
            badge(
                ui,
                &format!("{} crashed", c.crashed),
                "Show crashed tasks",
                BadgeTone::Tinted(Tone::Danger),
            );
        }));
    }
    for (filter, n, word, tone) in [
        (TaskFilter::Stopped, c.stopped, "stopped", stopped_tone),
        (TaskFilter::Running, c.running, "running", Tone::Done),
    ] {
        clicked = clicked.or(count(ui, filter, |ui| {
            ui.label(
                RichText::new(format!("{n} {word}"))
                    .font(font.clone())
                    .color(p.text),
            );
            text_dot(ui, tone_color(tone, dark), 7.0, &font);
        }));
    }
    ui.label(RichText::new("Tasks:").font(font.clone()).color(p.text_low));
    clicked
}

/// Title, the repo filter over `(repo, agent count)`, the persona filter over `(persona id, label, agent count)` and
/// the Show / Compact / Hide switch for fleets, left to right: a later filter goes after them. Exactly one row, so it
/// lines up with the drawer header; anything more goes in the strip under it. Returns a newly picked fleet display.
pub(crate) fn header(
    ui: &mut Ui,
    repos: &[(String, usize)],
    filter: &mut Vec<String>,
    personas: &[(String, String, usize)],
    persona_selected: &mut Vec<String>,
    fleet: FleetDisplay,
) -> Option<FleetDisplay> {
    title(ui, View::Kanban);
    ui.add_space(6.0);
    repo_filter(ui, repos, filter);
    persona_filter(ui, personas, persona_selected);
    ui.add_space(6.0);
    fleet_switch(ui, fleet)
}

/// "Fleet" and one chip per display, the current one selected.
fn fleet_switch(ui: &mut Ui, current: FleetDisplay) -> Option<FleetDisplay> {
    let p = tokens(ui.visuals().dark_mode);
    ui.label(
        RichText::new("Fleet")
            .font(sans(ui, size::SMALL, Weight::Regular))
            .color(p.text_low),
    );
    let mut picked = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for d in FleetDisplay::ALL {
            if chip(ui, d.label(), d == current).clicked() && d != current {
                picked = Some(d);
            }
        }
    });
    picked
}

/// The row under the header: the task counts at its far end once the tasks list has been fetched; view
/// controls take its start. The stopped count's dot takes `stopped_tone`, the Stopped column's. Returns the filter of
/// a clicked count.
pub(crate) fn strip(
    ui: &mut Ui,
    tasks: Option<&TaskList>,
    stopped_tone: Tone,
) -> Option<TaskFilter> {
    let list = tasks?;
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        task_counts(ui, Counts::of(list), stopped_tone)
    })
    .inner
}

/// The board in the space left after the panels. Draw the detail panel first: a side panel takes its width from
/// what the central panel has not yet claimed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn main(
    ui: &mut Ui,
    schema: &UiSchema,
    shown: &[Agent],
    now: i64,
    cards: &mut HashMap<String, CardState>,
    selected: Option<&str>,
    tags: &HashMap<String, String>,
    display: FleetDisplay,
    prs: &HashMap<String, bool>,
) -> Vec<Command> {
    let bg = tokens(ui.visuals().dark_mode).bg;
    // No margin: the column headers sit flush under the header and the dividers reach the footer.
    egui::CentralPanel::default()
        .frame(Frame::new().fill(bg))
        .show(ui, |ui| {
            board(ui, schema, shown, now, cards, selected, tags, display, prs)
        })
        .inner
}
