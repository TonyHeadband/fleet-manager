//! The Tasks view: totals across the top, every task and orphan in a narrow list, and the selected one's
//! passport. Archive starts here and nowhere else.
use super::{title, View};
use crate::components::{
    orphan_passport, release_filter, task_passport, DiffView, PassportAction, ReleaseFilter,
    TaskPassport,
};
use crate::theme::{sans, size, tokens, Weight};
use crate::widgets::{badge, button, format_bytes, list_row, toggle, BadgeTone, ButtonVariant};
use eframe::egui::{
    self, Align, Frame, Label, Layout, Margin, Panel, Rect, RichText, ScrollArea, Ui,
};
use fleet_api::{
    task_outcome, task_progress, Orphan, Release, RepoInfo, StopCause, Task, TaskList, TaskOutcome,
    TaskProgress, TaskState, Tone,
};
use std::collections::HashMap;

/// Width of the task list; the passport takes the rest (the mockup's 250 of 1400).
pub const LIST_WIDTH: f32 = 250.0;

/// Which rows the list shows. The Kanban header's counts open the view on one of these.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum TaskFilter {
    #[default]
    All,
    Running,
    /// Stopped for any cause, crashed included.
    Stopped,
    Crashed,
    Orphans,
    /// Tasks whose PRs all merged, archived ones included.
    Merged,
    /// Tasks archived without a merged PR.
    Dropped,
}

impl TaskFilter {
    fn label(self) -> &'static str {
        match self {
            TaskFilter::All => "all",
            TaskFilter::Running => "running",
            TaskFilter::Stopped => "stopped",
            TaskFilter::Crashed => "crashed",
            TaskFilter::Orphans => "orphans",
            TaskFilter::Merged => "merged",
            TaskFilter::Dropped => "dropped",
        }
    }
}

/// The active product as the view needs it: which tasks are listed, which repos have a URL, which releases a
/// task can move to. With no product selected nothing narrows the list.
#[derive(Clone, Copy, Default)]
pub struct Scope<'a> {
    pub product: Option<&'a str>,
    pub repos: &'a [RepoInfo],
    pub releases: &'a [Release],
    /// The active product was found in the loaded list, so `repos` is its real list and not a gap.
    pub repos_known: bool,
    /// The release filter the list narrows by.
    pub release: ReleaseFilter,
}

impl Scope<'_> {
    fn has(&self, t: &Task) -> bool {
        self.product.is_none_or(|p| t.product == p)
    }

    /// Whether the product gives one of the task's repos a URL, so that its work lands in a PR.
    fn has_url_repo(&self, t: &Task) -> bool {
        t.repos.iter().any(|w| {
            self.repos
                .iter()
                .any(|r| r.name == w.repo && r.url.is_some())
        })
    }
}

/// How an outcome reads on a badge.
pub(crate) fn outcome_look(o: TaskOutcome) -> (&'static str, BadgeTone) {
    match o {
        TaskOutcome::Open => ("Open", BadgeTone::Neutral),
        TaskOutcome::Merged => ("Merged", BadgeTone::Tinted(Tone::Done)),
        TaskOutcome::DoneByHand => ("Done by hand", BadgeTone::Tinted(Tone::Done)),
        TaskOutcome::Dropped => ("Dropped", BadgeTone::Tinted(Tone::Caution)),
    }
}

/// `3/7 steps` for an orchestrated task; for a solo one the board column of its agent, as the schema titles it.
pub fn progress_text(t: &Task, live: &HashMap<String, Live>) -> String {
    match task_progress(t) {
        TaskProgress::Steps { done, planned } => format!("{done}/{planned} steps"),
        TaskProgress::Column(status) => match live.get(&t.id.0) {
            Some(l) => l.status.clone(),
            None => format!("{status:?}"),
        },
    }
}

/// What the view keeps between frames.
pub struct TasksState {
    pub list: Option<TaskList>,
    /// The last fetch failed with this.
    pub error: Option<String>,
    pub show_archived: bool,
    pub filter: TaskFilter,
    /// A task id or an orphan id.
    pub selected: Option<String>,
    /// The diff shown in the passport, for the task id it was fetched for.
    pub diff: Option<(String, DiffView)>,
    /// The passport's Stop was clicked once and waits for Confirm Stop.
    pub confirm_stop: bool,
    /// Where the list and the passport landed last frame, for tests.
    pub rects: (Rect, Rect),
}

impl Default for TasksState {
    fn default() -> TasksState {
        TasksState {
            list: None,
            error: None,
            show_archived: false,
            filter: TaskFilter::All,
            selected: None,
            diff: None,
            confirm_stop: false,
            rects: (Rect::NOTHING, Rect::NOTHING),
        }
    }
}

/// What the app knows about a running task's agent: its board status's title and tone, and whether the schema
/// offers Stop for it.
pub struct Live {
    pub status: String,
    pub tone: Tone,
    pub can_stop: bool,
}

/// What the user asked for in the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TasksAction {
    /// An action on the selected row. `name` is what messages call it.
    Passport {
        id: String,
        name: String,
        orphan: bool,
        action: PassportAction,
    },
    Retry,
}

/// One row of the list.
#[derive(Clone, Copy)]
enum Entry<'a> {
    Task(&'a Task),
    Orphan(&'a Orphan),
}

impl Entry<'_> {
    fn id(&self) -> &str {
        match self {
            Entry::Task(p) => &p.id.0,
            Entry::Orphan(o) => &o.id,
        }
    }

    fn name(&self) -> String {
        match self {
            Entry::Task(p) => p.name.clone(),
            Entry::Orphan(o) => orphan_name(o),
        }
    }

    fn bytes(&self) -> Option<u64> {
        match self {
            Entry::Task(p) => p.footprint.as_ref().map(|f| f.total),
            Entry::Orphan(o) => o.bytes,
        }
    }

    /// Running, stopped with crashed first, orphans, archived.
    fn group(&self) -> u8 {
        match self {
            Entry::Task(p) => match (p.state, p.stop_cause) {
                (TaskState::Running, _) => 0,
                (TaskState::Stopped, Some(StopCause::Crashed)) => 1,
                (TaskState::Stopped, _) => 2,
                (TaskState::Archived, _) => 4,
            },
            Entry::Orphan(_) => 3,
        }
    }
}

/// A session's name, or the folder's last component.
pub fn orphan_name(o: &Orphan) -> String {
    std::path::Path::new(&o.path)
        .file_name()
        .map_or_else(|| o.path.clone(), |n| n.to_string_lossy().into_owned())
}

fn cause_look(cause: Option<StopCause>) -> (&'static str, Tone) {
    match cause {
        Some(StopCause::Exited) => ("Exited", Tone::Active),
        Some(StopCause::Crashed) => ("Crashed", Tone::Danger),
        Some(StopCause::Finished) => ("Finished", Tone::Active),
        Some(StopCause::Stopped) | None => ("Stopped", Tone::Caution),
    }
}

/// The row's badge: a running task in its board status's colour, a stopped one in its cause's.
fn look(e: Entry<'_>, live: &HashMap<String, Live>) -> (String, BadgeTone) {
    match e {
        Entry::Task(p) => match p.state {
            TaskState::Running => match live.get(&p.id.0) {
                Some(l) => (l.status.clone(), BadgeTone::Tinted(l.tone)),
                None => ("Running".into(), BadgeTone::Neutral),
            },
            TaskState::Stopped => {
                let (text, tone) = cause_look(p.stop_cause);
                (text.into(), BadgeTone::Tinted(tone))
            }
            TaskState::Archived => ("Archived".into(), BadgeTone::Neutral),
        },
        // No ⚠: glyphs are painted in this app, never typed, and no installed font carries that one.
        Entry::Orphan(_) => ("Orphan".into(), BadgeTone::Tinted(Tone::Caution)),
    }
}

/// The rows to show, in list order: newest first within each group, orphans by path.
fn entries<'a>(
    list: &'a TaskList,
    filter: TaskFilter,
    archived: bool,
    scope: &Scope<'_>,
) -> Vec<Entry<'a>> {
    let mut rows: Vec<Entry> = list
        .tasks
        .iter()
        .filter(|p| scope.has(p) && scope.release.allows(p.release))
        .filter(|p| match filter {
            TaskFilter::All => p.state != TaskState::Archived || archived,
            TaskFilter::Running => p.state == TaskState::Running,
            TaskFilter::Stopped => p.state == TaskState::Stopped,
            TaskFilter::Crashed => {
                p.state == TaskState::Stopped && p.stop_cause == Some(StopCause::Crashed)
            }
            // An archived task is where a merged or dropped one ends up, so these look past Show archived.
            TaskFilter::Merged => task_outcome(p) == TaskOutcome::Merged,
            TaskFilter::Dropped => task_outcome(p) == TaskOutcome::Dropped,
            TaskFilter::Orphans => false,
        })
        .map(Entry::Task)
        .collect();
    if matches!(filter, TaskFilter::All | TaskFilter::Orphans) {
        rows.extend(list.orphans.iter().map(Entry::Orphan));
    }
    rows.sort_by(|a, b| {
        a.group().cmp(&b.group()).then_with(|| match (a, b) {
            // `None` (start unknown) sorts as oldest.
            (Entry::Task(x), Entry::Task(y)) => y.created_at.cmp(&x.created_at),
            (Entry::Orphan(x), Entry::Orphan(y)) => x.path.cmp(&y.path),
            _ => std::cmp::Ordering::Equal,
        })
    });
    rows
}

/// Title and the Show archived toggle.
pub(crate) fn header(
    ui: &mut Ui,
    show_archived: &mut bool,
    releases: &[Release],
    release: &mut ReleaseFilter,
) {
    title(ui, View::Tasks);
    ui.add_space(6.0);
    if toggle(ui, "Show archived", *show_archived).clicked() {
        *show_archived = !*show_archived;
    }
    ui.add_space(6.0);
    release_filter(ui, releases, release);
}

fn stat(ui: &mut Ui, caption: &str, values: &[String]) {
    let p = tokens(ui.visuals().dark_mode);
    ui.vertical(|ui| {
        ui.label(RichText::new(caption).size(size::SMALL).color(p.text_low));
        ui.horizontal(|ui| {
            for v in values {
                ui.label(
                    RichText::new(v)
                        .font(sans(ui, size::BODY, Weight::Medium))
                        .color(p.text_high),
                );
            }
        });
    });
    ui.add_space(28.0);
}

/// `tasks` are the active product's; orphans belong to none and are always counted.
fn totals(ui: &mut Ui, list: &TaskList, tasks: &[&Task]) {
    let in_state = |s: TaskState| tasks.iter().copied().filter(move |p| p.state == s);
    let running = in_state(TaskState::Running).count();
    let stopped = in_state(TaskState::Stopped).count();
    let crashed = in_state(TaskState::Stopped)
        .filter(|p| p.stop_cause == Some(StopCause::Crashed))
        .count();
    let orphans = list.orphans.len();
    let mut counts = vec![format!("{running} running"), format!("{stopped} stopped")];
    if crashed > 0 {
        counts.push(format!("{crashed} crashed"));
    }
    if orphans > 0 {
        counts.push(format!(
            "{orphans} {}",
            if orphans == 1 { "orphan" } else { "orphans" }
        ));
    }
    let size = |p: &Task| p.footprint.as_ref().map_or(0, |f| f.total);
    let orphan_bytes: u64 = list.orphans.iter().filter_map(|o| o.bytes).sum();
    let held: u64 = tasks
        .iter()
        .filter(|p| p.state != TaskState::Archived)
        .map(|p| size(p))
        .sum::<u64>()
        + orphan_bytes;
    let reclaimable: u64 = in_state(TaskState::Stopped).map(size).sum::<u64>() + orphan_bytes;
    // Records made on upgrade say nothing about when they started, so they are not "since this version".
    let agents: u32 = tasks
        .iter()
        .filter(|p| p.created_at.is_some())
        .map(|p| p.agents_used)
        .sum();
    ui.horizontal(|ui| {
        stat(ui, "Tasks", &counts);
        stat(
            ui,
            "Disk",
            &[
                format!("{} held", format_bytes(held)),
                format!("{} reclaimable", format_bytes(reclaimable)),
            ],
        );
        stat(
            ui,
            "Clones cache",
            &[list
                .clones_bytes
                .map_or_else(|| "measuring…".into(), format_bytes)],
        );
        stat(ui, "Agents started", &[agents.to_string()]);
        stat(ui, "Tokens · cost", &["not tracked yet".into()]);
    });
}

/// The outcome badge and the progress, which a running solo task already shows as its state badge.
pub(crate) fn outcome_line(ui: &mut Ui, t: &Task, state_text: &str, live: &HashMap<String, Live>) {
    let p = tokens(ui.visuals().dark_mode);
    let (text, tone) = outcome_look(task_outcome(t));
    let progress = progress_text(t, live);
    ui.horizontal(|ui| {
        badge(ui, text, "Outcome", tone);
        if progress != state_text {
            ui.add(
                Label::new(RichText::new(progress).size(size::SMALL).color(p.text_low)).truncate(),
            );
        }
    });
}

fn list_view(
    ui: &mut Ui,
    rows: &[Entry<'_>],
    st: &mut TasksState,
    live: &HashMap<String, Live>,
    personas: &HashMap<String, String>,
) {
    let p = tokens(ui.visuals().dark_mode);
    if st.filter != TaskFilter::All {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Showing {}", st.filter.label()))
                    .size(size::SMALL)
                    .color(p.text_low),
            );
            if button(ui, "Show all", ButtonVariant::Ghost).clicked() {
                st.filter = TaskFilter::All;
            }
        });
    }
    if rows.is_empty() {
        ui.label(
            RichText::new("No tasks")
                .size(size::SMALL)
                .color(p.text_low),
        );
    }
    ScrollArea::vertical()
        .id_salt("tasks_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for e in rows {
                let selected = st.selected.as_deref() == Some(e.id());
                let name = e.name();
                let (text, tone) = look(*e, live);
                let archived = matches!(e, Entry::Task(pr) if pr.state == TaskState::Archived);
                let row = ui
                    .scope(|ui| {
                        if archived {
                            ui.multiply_opacity(0.55);
                        }
                        list_row(ui, &name, selected, |ui| {
                            ui.horizontal(|ui| {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let size = e.bytes().map_or_else(|| "—".into(), format_bytes);
                                    ui.label(
                                        RichText::new(size).size(size::SMALL).color(p.text_low),
                                    );
                                    badge(ui, &text, "State", tone);
                                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                        ui.spacing_mut().item_spacing.y = 1.0;
                                        ui.add(
                                            Label::new(RichText::new(&name).color(p.text_high))
                                                .truncate(),
                                        );
                                        // Every task runs under one, whatever the board's persona filter.
                                        if let Entry::Task(pr) = e {
                                            outcome_line(ui, pr, &text, live);
                                            ui.add(
                                                Label::new(
                                                    RichText::new(
                                                        personas
                                                            .get(&pr.persona)
                                                            .map_or(&pr.persona, |l| l),
                                                    )
                                                    .size(size::SMALL)
                                                    .color(p.text_low),
                                                )
                                                .truncate(),
                                            );
                                        }
                                    });
                                });
                            });
                        })
                    })
                    .inner;
                if row.clicked() && !selected {
                    st.selected = Some(e.id().to_string());
                    st.confirm_stop = false;
                }
            }
        });
}

/// Totals, list and passport in the space left after the app's rows. `live` maps running task ids to their
/// agent; `now` is the clock in seconds; `personas` maps persona ids to their labels (`persona_labels`).
pub(crate) fn main(
    ui: &mut Ui,
    st: &mut TasksState,
    live: &HashMap<String, Live>,
    now: i64,
    personas: &HashMap<String, String>,
    scope: Scope<'_>,
) -> Option<TasksAction> {
    let p = tokens(ui.visuals().dark_mode);
    let Some(list) = st.list.take() else {
        let mut retry = false;
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .inner_margin(Margin::symmetric(22, 18)),
            )
            .show(ui, |ui| match &st.error {
                Some(e) => {
                    ui.label(RichText::new(format!("Cannot load the tasks: {e}")).color(p.danger));
                    ui.add_space(10.0);
                    retry = button(ui, "Retry", ButtonVariant::Primary).clicked();
                }
                None => {
                    ui.label(RichText::new("Loading…").color(p.text_low));
                }
            });
        return retry.then_some(TasksAction::Retry);
    };
    let action = with_list(ui, st, &list, live, now, personas, &scope);
    st.list = Some(list);
    action
}

fn with_list(
    ui: &mut Ui,
    st: &mut TasksState,
    list: &TaskList,
    live: &HashMap<String, Live>,
    now: i64,
    personas: &HashMap<String, String>,
    scope: &Scope<'_>,
) -> Option<TasksAction> {
    let p = tokens(ui.visuals().dark_mode);
    let rows = entries(list, st.filter, st.show_archived, scope);
    let in_product: Vec<&Task> = list.tasks.iter().filter(|t| scope.has(t)).collect();
    // A selection the list no longer shows (archived and hidden, filtered out, gone) moves to the first row.
    if !rows.iter().any(|e| Some(e.id()) == st.selected.as_deref()) {
        st.selected = rows.first().map(|e| e.id().to_string());
        st.confirm_stop = false;
    }

    Panel::top("tasks_totals")
        .frame(
            Frame::new()
                .fill(p.surface)
                .inner_margin(Margin::symmetric(22, 10)),
        )
        .show(ui, |ui| {
            totals(ui, list, &in_product);
            if let Some(e) = &st.error {
                ui.label(
                    RichText::new(format!("Could not refresh: {e}"))
                        .size(size::SMALL)
                        .color(p.danger),
                );
            }
        });
    let left = Panel::left("tasks_list")
        .exact_size(LIST_WIDTH)
        .resizable(false)
        .frame(Frame::new().fill(p.surface).inner_margin(Margin::same(8)))
        .show(ui, |ui| list_view(ui, &rows, st, live, personas));

    let selected = rows
        .iter()
        .copied()
        .find(|e| Some(e.id()) == st.selected.as_deref());
    // A stopped task's diff loads with its passport, and again when another stopped task is selected.
    let mut action: Option<PassportAction> = None;
    if let Some(Entry::Task(pr)) = selected {
        if pr.state == TaskState::Stopped && st.diff.as_ref().is_none_or(|(id, _)| *id != pr.id.0) {
            st.diff = Some((pr.id.0.clone(), DiffView::Loading));
            action = Some(PassportAction::LoadDiff);
        }
    }
    let no_diff = DiffView::NotLoaded;
    let central = egui::CentralPanel::default()
        .frame(Frame::new().fill(p.bg))
        .show(ui, |ui| {
            ScrollArea::vertical()
                .id_salt("passport")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    Frame::new()
                        .inner_margin(Margin::symmetric(22, 18))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let Some(e) = selected else {
                                ui.label(
                                    RichText::new("Nothing to show")
                                        .size(size::SMALL)
                                        .color(p.text_low),
                                );
                                return None;
                            };
                            let (text, tone) = look(e, live);
                            ui.push_id(e.id(), |ui| match e {
                                Entry::Task(pr) => {
                                    let diff = match &st.diff {
                                        Some((id, d)) if *id == pr.id.0 => d,
                                        _ => &no_diff,
                                    };
                                    let can_stop = live.get(&pr.id.0).is_some_and(|l| l.can_stop);
                                    task_passport(
                                        ui,
                                        TaskPassport {
                                            task: pr,
                                            badge: (&text, tone),
                                            now,
                                            can_stop,
                                            diff,
                                            outcome: task_outcome(pr),
                                            progress: progress_text(pr, live),
                                            releases: scope.releases,
                                            can_mark_done: pr.state != TaskState::Archived
                                                && pr.done_at.is_none()
                                                && !pr.done_by_hand
                                                && scope.repos_known
                                                && !scope.has_url_repo(pr),
                                        },
                                        &mut st.confirm_stop,
                                    )
                                }
                                Entry::Orphan(o) => {
                                    orphan_passport(ui, o, &e.name(), (&text, tone))
                                }
                            })
                            .inner
                        })
                        .inner
                })
                .inner
        });
    st.rects = (left.response.rect, central.response.rect);
    // The user's click wins over the automatic diff load, which then happens next frame.
    if let Some(clicked) = central.inner {
        if action == Some(PassportAction::LoadDiff) {
            st.diff = None;
        }
        action = Some(clicked);
    }
    let e = selected?;
    action.map(|action| TasksAction::Passport {
        id: e.id().to_string(),
        name: e.name(),
        orphan: matches!(e, Entry::Orphan(_)),
        action,
    })
}
