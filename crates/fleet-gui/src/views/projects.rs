//! The Projects view: totals across the top, every project and orphan in a narrow list, and the selected one's
//! passport. Archive starts here and nowhere else.
use super::{title, View};
use crate::components::{
    orphan_passport, project_passport, DiffView, PassportAction, ProjectPassport,
};
use crate::theme::{sans, size, tokens, Weight};
use crate::widgets::{badge, button, format_bytes, list_row, toggle, BadgeTone, ButtonVariant};
use eframe::egui::{
    self, Align, Frame, Label, Layout, Margin, Panel, Rect, RichText, ScrollArea, Ui,
};
use fleet_api::{Orphan, Project, ProjectList, ProjectState, StopCause, Tone};
use std::collections::HashMap;

/// Width of the project list; the passport takes the rest (the mockup's 250 of 1400).
pub const LIST_WIDTH: f32 = 250.0;

/// Which rows the list shows. The Kanban header's counts open the view on one of these.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ProjectFilter {
    #[default]
    All,
    Running,
    /// Stopped for any cause, crashed included.
    Stopped,
    Crashed,
    Orphans,
}

impl ProjectFilter {
    fn label(self) -> &'static str {
        match self {
            ProjectFilter::All => "all",
            ProjectFilter::Running => "running",
            ProjectFilter::Stopped => "stopped",
            ProjectFilter::Crashed => "crashed",
            ProjectFilter::Orphans => "orphans",
        }
    }
}

/// What the view keeps between frames.
pub struct ProjectsState {
    pub list: Option<ProjectList>,
    /// The last fetch failed with this.
    pub error: Option<String>,
    pub show_archived: bool,
    pub filter: ProjectFilter,
    /// A project id or an orphan id.
    pub selected: Option<String>,
    /// The diff shown in the passport, for the project id it was fetched for.
    pub diff: Option<(String, DiffView)>,
    /// The passport's Stop was clicked once and waits for Confirm Stop.
    pub confirm_stop: bool,
    /// Where the list and the passport landed last frame, for tests.
    pub rects: (Rect, Rect),
}

impl Default for ProjectsState {
    fn default() -> ProjectsState {
        ProjectsState {
            list: None,
            error: None,
            show_archived: false,
            filter: ProjectFilter::All,
            selected: None,
            diff: None,
            confirm_stop: false,
            rects: (Rect::NOTHING, Rect::NOTHING),
        }
    }
}

/// What the app knows about a running project's agent: its board status's title and tone, and whether the schema
/// offers Stop for it.
pub struct Live {
    pub status: String,
    pub tone: Tone,
    pub can_stop: bool,
}

/// What the user asked for in the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectsAction {
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
    Project(&'a Project),
    Orphan(&'a Orphan),
}

impl Entry<'_> {
    fn id(&self) -> &str {
        match self {
            Entry::Project(p) => &p.id.0,
            Entry::Orphan(o) => &o.id,
        }
    }

    fn name(&self) -> String {
        match self {
            Entry::Project(p) => p.name.clone(),
            Entry::Orphan(o) => orphan_name(o),
        }
    }

    fn bytes(&self) -> Option<u64> {
        match self {
            Entry::Project(p) => p.footprint.as_ref().map(|f| f.total),
            Entry::Orphan(o) => o.bytes,
        }
    }

    /// Running, stopped with crashed first, orphans, archived.
    fn group(&self) -> u8 {
        match self {
            Entry::Project(p) => match (p.state, p.stop_cause) {
                (ProjectState::Running, _) => 0,
                (ProjectState::Stopped, Some(StopCause::Crashed)) => 1,
                (ProjectState::Stopped, _) => 2,
                (ProjectState::Archived, _) => 4,
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

/// The row's badge: a running project in its board status's colour, a stopped one in its cause's.
fn look(e: Entry<'_>, live: &HashMap<String, Live>) -> (String, BadgeTone) {
    match e {
        Entry::Project(p) => match p.state {
            ProjectState::Running => match live.get(&p.id.0) {
                Some(l) => (l.status.clone(), BadgeTone::Tinted(l.tone)),
                None => ("Running".into(), BadgeTone::Neutral),
            },
            ProjectState::Stopped => {
                let (text, tone) = cause_look(p.stop_cause);
                (text.into(), BadgeTone::Tinted(tone))
            }
            ProjectState::Archived => ("Archived".into(), BadgeTone::Neutral),
        },
        // No ⚠: glyphs are painted in this app, never typed, and no installed font carries that one.
        Entry::Orphan(_) => ("Orphan".into(), BadgeTone::Tinted(Tone::Caution)),
    }
}

/// The rows to show, in list order: newest first within each group, orphans by path.
fn entries<'a>(list: &'a ProjectList, filter: ProjectFilter, archived: bool) -> Vec<Entry<'a>> {
    let mut rows: Vec<Entry> = list
        .projects
        .iter()
        .filter(|p| match filter {
            ProjectFilter::All => p.state != ProjectState::Archived || archived,
            ProjectFilter::Running => p.state == ProjectState::Running,
            ProjectFilter::Stopped => p.state == ProjectState::Stopped,
            ProjectFilter::Crashed => {
                p.state == ProjectState::Stopped && p.stop_cause == Some(StopCause::Crashed)
            }
            ProjectFilter::Orphans => false,
        })
        .map(Entry::Project)
        .collect();
    if matches!(filter, ProjectFilter::All | ProjectFilter::Orphans) {
        rows.extend(list.orphans.iter().map(Entry::Orphan));
    }
    rows.sort_by(|a, b| {
        a.group().cmp(&b.group()).then_with(|| match (a, b) {
            // `None` (start unknown) sorts as oldest.
            (Entry::Project(x), Entry::Project(y)) => y.created_at.cmp(&x.created_at),
            (Entry::Orphan(x), Entry::Orphan(y)) => x.path.cmp(&y.path),
            _ => std::cmp::Ordering::Equal,
        })
    });
    rows
}

/// Title and the Show archived toggle.
pub(crate) fn header(ui: &mut Ui, show_archived: &mut bool) {
    title(ui, View::Projects);
    ui.add_space(6.0);
    if toggle(ui, "Show archived", *show_archived).clicked() {
        *show_archived = !*show_archived;
    }
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

fn totals(ui: &mut Ui, list: &ProjectList) {
    let in_state = |s: ProjectState| list.projects.iter().filter(move |p| p.state == s);
    let running = in_state(ProjectState::Running).count();
    let stopped = in_state(ProjectState::Stopped).count();
    let crashed = in_state(ProjectState::Stopped)
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
    let size = |p: &Project| p.footprint.as_ref().map_or(0, |f| f.total);
    let orphan_bytes: u64 = list.orphans.iter().filter_map(|o| o.bytes).sum();
    let held: u64 = list
        .projects
        .iter()
        .filter(|p| p.state != ProjectState::Archived)
        .map(size)
        .sum::<u64>()
        + orphan_bytes;
    let reclaimable: u64 = in_state(ProjectState::Stopped).map(size).sum::<u64>() + orphan_bytes;
    // Records made on upgrade say nothing about when they started, so they are not "since this version".
    let agents: u32 = list
        .projects
        .iter()
        .filter(|p| p.created_at.is_some())
        .map(|p| p.agents_used)
        .sum();
    ui.horizontal(|ui| {
        stat(ui, "Projects", &counts);
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

fn list_view(
    ui: &mut Ui,
    rows: &[Entry<'_>],
    st: &mut ProjectsState,
    live: &HashMap<String, Live>,
    personas: &HashMap<String, String>,
) {
    let p = tokens(ui.visuals().dark_mode);
    if st.filter != ProjectFilter::All {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Showing {}", st.filter.label()))
                    .size(size::SMALL)
                    .color(p.text_low),
            );
            if button(ui, "Show all", ButtonVariant::Ghost).clicked() {
                st.filter = ProjectFilter::All;
            }
        });
    }
    if rows.is_empty() {
        ui.label(
            RichText::new("No projects")
                .size(size::SMALL)
                .color(p.text_low),
        );
    }
    ScrollArea::vertical()
        .id_salt("projects_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for e in rows {
                let selected = st.selected.as_deref() == Some(e.id());
                let name = e.name();
                let (text, tone) = look(*e, live);
                let archived =
                    matches!(e, Entry::Project(pr) if pr.state == ProjectState::Archived);
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
                                        // Every project runs under one, whatever the board's persona filter.
                                        if let Entry::Project(pr) = e {
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

/// Totals, list and passport in the space left after the app's rows. `live` maps running project ids to their
/// agent; `now` is the clock in seconds; `personas` maps persona ids to their labels (`persona_labels`).
pub(crate) fn main(
    ui: &mut Ui,
    st: &mut ProjectsState,
    live: &HashMap<String, Live>,
    now: i64,
    personas: &HashMap<String, String>,
) -> Option<ProjectsAction> {
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
                    ui.label(
                        RichText::new(format!("Cannot load the projects: {e}")).color(p.danger),
                    );
                    ui.add_space(10.0);
                    retry = button(ui, "Retry", ButtonVariant::Primary).clicked();
                }
                None => {
                    ui.label(RichText::new("Loading…").color(p.text_low));
                }
            });
        return retry.then_some(ProjectsAction::Retry);
    };
    let action = with_list(ui, st, &list, live, now, personas);
    st.list = Some(list);
    action
}

fn with_list(
    ui: &mut Ui,
    st: &mut ProjectsState,
    list: &ProjectList,
    live: &HashMap<String, Live>,
    now: i64,
    personas: &HashMap<String, String>,
) -> Option<ProjectsAction> {
    let p = tokens(ui.visuals().dark_mode);
    let rows = entries(list, st.filter, st.show_archived);
    // A selection the list no longer shows (archived and hidden, filtered out, gone) moves to the first row.
    if !rows.iter().any(|e| Some(e.id()) == st.selected.as_deref()) {
        st.selected = rows.first().map(|e| e.id().to_string());
        st.confirm_stop = false;
    }

    Panel::top("projects_totals")
        .frame(
            Frame::new()
                .fill(p.surface)
                .inner_margin(Margin::symmetric(22, 10)),
        )
        .show(ui, |ui| {
            totals(ui, list);
            if let Some(e) = &st.error {
                ui.label(
                    RichText::new(format!("Could not refresh: {e}"))
                        .size(size::SMALL)
                        .color(p.danger),
                );
            }
        });
    let left = Panel::left("projects_list")
        .exact_size(LIST_WIDTH)
        .resizable(false)
        .frame(Frame::new().fill(p.surface).inner_margin(Margin::same(8)))
        .show(ui, |ui| list_view(ui, &rows, st, live, personas));

    let selected = rows
        .iter()
        .copied()
        .find(|e| Some(e.id()) == st.selected.as_deref());
    // A stopped project's diff loads with its passport, and again when another stopped project is selected.
    let mut action: Option<PassportAction> = None;
    if let Some(Entry::Project(pr)) = selected {
        if pr.state == ProjectState::Stopped
            && st.diff.as_ref().is_none_or(|(id, _)| *id != pr.id.0)
        {
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
                                Entry::Project(pr) => {
                                    let diff = match &st.diff {
                                        Some((id, d)) if *id == pr.id.0 => d,
                                        _ => &no_diff,
                                    };
                                    let can_stop = live.get(&pr.id.0).is_some_and(|l| l.can_stop);
                                    project_passport(
                                        ui,
                                        ProjectPassport {
                                            project: pr,
                                            badge: (&text, tone),
                                            now,
                                            can_stop,
                                            diff,
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
    action.map(|action| ProjectsAction::Passport {
        id: e.id().to_string(),
        name: e.name(),
        orphan: matches!(e, Entry::Orphan(_)),
        action,
    })
}
