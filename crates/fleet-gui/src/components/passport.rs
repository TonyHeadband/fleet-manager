//! The right side of the Projects view: everything known about one project, or about one orphan.
use super::preview::{diff_view, screen_view};
use super::DiffView;
use crate::theme::{mono, sans, size, tokens, Weight};
use crate::widgets::{badge, button, format_bytes, BadgeTone, ButtonVariant};
use eframe::egui::{self, vec2, Align, Frame, Label, Layout, Margin, RichText, Ui};
use fleet_api::{Orphan, OrphanKind, Task, TaskState};
use fleet_client::schema::format_duration;

/// What the user asked for in a passport; the app carries it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassportAction {
    OpenOnBoard,
    Stop,
    Archive,
    EndSession,
    /// Rescan and re-measure.
    Refresh,
    /// Fetch the diff again.
    LoadDiff,
}

/// The project's passport as the view hands it over: its badge, the time, whether Stop can run, and its diff.
pub struct TaskPassport<'a> {
    pub task: &'a Task,
    pub badge: (&'a str, BadgeTone),
    pub now: i64,
    /// The schema offers Stop for the project's agent.
    pub can_stop: bool,
    pub diff: &'a DiffView,
}

/// Height of the Diff and Last screen boxes: they scroll inside the passport, which scrolls too.
const BOX_HEIGHT: f32 = 280.0;

fn ago(now: i64, at: i64) -> String {
    format!("{} ago", format_duration(now - at))
}

fn section(ui: &mut Ui, text: &str) {
    let p = tokens(ui.visuals().dark_mode);
    ui.add_space(16.0);
    ui.label(
        RichText::new(text)
            .font(sans(ui, size::TITLE, Weight::Medium))
            .color(p.text_high),
    );
    ui.add_space(6.0);
}

fn low(ui: &mut Ui, text: impl Into<String>) -> egui::Response {
    let p = tokens(ui.visuals().dark_mode);
    ui.label(RichText::new(text.into()).color(p.text_low))
}

fn code(ui: &mut Ui, text: &str) -> egui::Response {
    let p = tokens(ui.visuals().dark_mode);
    ui.add(Label::new(RichText::new(text).font(mono(size::CODE)).color(p.text)).truncate())
}

/// Name and badge, with the actions at the right end, where they are in sight however long the passport is.
/// `actions` draws right to left: its first button is the rightmost.
fn title_row(
    ui: &mut Ui,
    name: &str,
    look: (&str, BadgeTone),
    tip: &str,
    actions: impl FnOnce(&mut Ui),
) {
    let p = tokens(ui.visuals().dark_mode);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            actions(ui);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.label(
                    RichText::new(name)
                        .font(sans(ui, size::HEADING, Weight::Medium))
                        .color(p.text_high),
                );
                badge(ui, look.0, tip, look.1);
            });
        });
    });
}

/// A boxed, fixed-height area on `bg` for a diff or a screen.
fn text_box(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let p = tokens(ui.visuals().dark_mode);
    Frame::new()
        .fill(p.bg)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.allocate_ui(vec2(ui.available_width(), BOX_HEIGHT), |ui| {
                ui.set_min_size(vec2(ui.available_width(), BOX_HEIGHT));
                ui.set_max_height(BOX_HEIGHT);
                add(ui);
            });
        });
}

/// `confirm_stop` is the view's: Stop asks for a second click, as on a card.
pub fn task_passport(
    ui: &mut Ui,
    pp: TaskPassport<'_>,
    confirm_stop: &mut bool,
) -> Option<PassportAction> {
    let pr = pp.task;
    let now = pp.now;
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    let archived = pr.state == TaskState::Archived;
    let stopped = pr.state == TaskState::Stopped;

    title_row(ui, &pr.name, pp.badge, "State", |ui| {
        // An archived passport is read-only, and its project has left the board.
        if archived {
            return;
        }
        let archive = ui
            .add_enabled_ui(stopped, |ui| {
                button(ui, "Archive…", ButtonVariant::QuietDanger)
            })
            .inner
            .on_disabled_hover_text("stop it first");
        if archive.clicked() {
            action = Some(PassportAction::Archive);
        }
        if pr.state == TaskState::Running && pp.can_stop {
            if *confirm_stop {
                if button(ui, "Confirm Stop", ButtonVariant::Danger).clicked() {
                    *confirm_stop = false;
                    action = Some(PassportAction::Stop);
                }
            } else if button(ui, "Stop", ButtonVariant::Default).clicked() {
                *confirm_stop = true;
            }
        }
        if button(ui, "Open on board", ButtonVariant::Ghost).clicked() {
            action = Some(PassportAction::OpenOnBoard);
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        match pr.created_at {
            Some(at) => low(ui, format!("started {}", ago(now, at))),
            // A record made on upgrade: the start was before the daemon kept one.
            None => low(ui, "started: unknown"),
        };
        if let Some(at) = pr.archived_at {
            low(ui, format!("· archived {}", ago(now, at)));
        }
    });
    code(ui, &pr.workdir);

    section(ui, "Goal");
    let goal = pr.goal.lines().next().unwrap_or_default();
    ui.add(Label::new(RichText::new(goal).color(p.text)).wrap());

    section(ui, "Repos");
    if pr.repos.is_empty() {
        low(ui, "No repo");
    } else {
        egui::Grid::new("passport_repos")
            .num_columns(3)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for w in &pr.repos {
                    ui.label(RichText::new(&w.repo).color(p.text_high));
                    code(ui, &w.branch);
                    let start = &w.base[..w.base.len().min(7)];
                    let from = if w.owned {
                        "created in this project".to_string()
                    } else {
                        match &w.from_branch {
                            Some(b) => format!("from {b} at {start}"),
                            None => format!("from {start}"),
                        }
                    };
                    low(ui, from);
                    ui.end_row();
                }
            });
    }

    section(ui, "Footprint");
    match &pr.footprint {
        Some(f) => {
            egui::Grid::new("passport_footprint")
                .num_columns(3)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    for item in &f.items {
                        ui.label(RichText::new(&item.label).color(p.text));
                        code(ui, &item.path);
                        ui.label(RichText::new(format_bytes(item.bytes)).color(p.text));
                        ui.end_row();
                        if let Some(build) = item.build_bytes {
                            low(ui, "of which target/");
                            ui.label("");
                            low(ui, format_bytes(build));
                            ui.end_row();
                        }
                    }
                    ui.label(RichText::new("Total").color(p.text_high));
                    ui.label("");
                    ui.label(RichText::new(format_bytes(f.total)).color(p.text_high));
                    ui.end_row();
                });
            ui.horizontal(|ui| {
                low(ui, format!("measured {}", ago(now, f.measured_at)));
                if !archived && button(ui, "Refresh", ButtonVariant::Ghost).clicked() {
                    action = Some(PassportAction::Refresh);
                }
            });
        }
        None => {
            ui.horizontal(|ui| {
                low(ui, "not measured yet");
                if !archived && button(ui, "Refresh", ButtonVariant::Ghost).clicked() {
                    action = Some(PassportAction::Refresh);
                }
            });
        }
    }

    section(ui, "Activity");
    egui::Grid::new("passport_activity")
        .num_columns(2)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            low(ui, "Agents used");
            ui.label(pr.agents_used.to_string());
            ui.end_row();
            low(ui, "Time running");
            match pr.created_at {
                Some(start) => {
                    let end = pr.stopped_at.or(pr.archived_at).unwrap_or(now);
                    ui.label(format_duration(end - start))
                }
                None => ui.label("unknown"),
            };
            ui.end_row();
            low(ui, "Time per status");
            // Counted from the version that started keeping them; a record made on upgrade has none.
            if pr.created_at.is_none() && pr.times == Default::default() {
                ui.label("unknown");
            } else {
                let t = &pr.times;
                ui.label(format!(
                    "Working {} · Needs you {} · Review {} · Queued {}",
                    format_duration(t.working),
                    format_duration(t.needs_you),
                    format_duration(t.review),
                    format_duration(t.queued),
                ));
            }
            ui.end_row();
            low(ui, "Tokens · cost");
            low(ui, "not tracked yet");
            ui.end_row();
        });

    section(ui, "Log");
    if pr.log.is_empty() {
        low(ui, "Nothing logged");
    }
    egui::Grid::new("passport_log")
        .num_columns(2)
        .spacing([16.0, 2.0])
        .show(ui, |ui| {
            for e in &pr.log {
                low(ui, ago(now, e.at));
                ui.add(Label::new(RichText::new(&e.text).color(p.text)).wrap());
                ui.end_row();
            }
        });

    // The worktrees are on disk until Archive; an archived project has nothing left to diff.
    if stopped {
        section(ui, "Diff");
        ui.horizontal(|ui| {
            if button(ui, "Refresh", ButtonVariant::Ghost)
                .on_hover_text("Reload the diff")
                .clicked()
            {
                action = Some(PassportAction::LoadDiff);
            }
        });
        ui.push_id("passport_diff", |ui| {
            text_box(ui, |ui| diff_view(ui, "passport_diff", pp.diff))
        });

        section(ui, "Last screen");
        match &pr.last_screen {
            Some(text) => {
                ui.push_id("passport_screen", |ui| {
                    text_box(ui, |ui| screen_view(ui, "passport_screen", text))
                });
            }
            // Only Stop saves one: an exit or a crash leaves no session to read.
            None => {
                low(ui, "not saved");
            }
        }
    }

    ui.add_space(16.0);
    action
}

fn kind_text(kind: OrphanKind) -> &'static str {
    match kind {
        OrphanKind::TaskFolder => "A folder under the projects root with no project record",
        OrphanKind::AgentFolder => "A daemon agent folder with no project record",
        OrphanKind::Worktree => "A worktree under the projects root with no project record",
        OrphanKind::Session => "A live tmux session that no agent owns",
    }
}

/// The orphan's passport: what it is, where, its worktrees and size, and Archive… or, for a session, End session….
pub fn orphan_passport(
    ui: &mut Ui,
    o: &Orphan,
    name: &str,
    look: (&str, BadgeTone),
) -> Option<PassportAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    title_row(ui, name, look, "Found with no project record", |ui| {
        let (label, wanted) = if o.kind == OrphanKind::Session {
            ("End session…", PassportAction::EndSession)
        } else {
            ("Archive…", PassportAction::Archive)
        };
        if button(ui, label, ButtonVariant::QuietDanger).clicked() {
            action = Some(wanted);
        }
    });
    ui.add_space(4.0);
    low(ui, kind_text(o.kind));
    code(ui, &o.path);
    if !o.worktrees.is_empty() {
        section(ui, "Worktrees");
        for w in &o.worktrees {
            code(ui, w);
        }
    }
    if o.kind != OrphanKind::Session {
        section(ui, "Footprint");
        match o.bytes {
            Some(b) => ui.label(RichText::new(format_bytes(b)).color(p.text)),
            None => low(ui, "measuring…"),
        };
    }
    action
}
