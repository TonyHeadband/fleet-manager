use super::folder_picker::{merge_note, FolderPicker};
use super::reply::reply_box;
use super::repo_picker::{repo_picker, PickerState};
use super::repo_request::{repo_request_box, RequestDecision};
use crate::theme::{layout, mono, sans, size, tokens, tone_color, Weight};
use crate::widgets::{
    badge, badge_items, button, field, icon_button, tabs, text_dot, BadgeTone, ButtonVariant, Icon,
};
use eframe::egui::{
    self, vec2, Align, Frame, Label, Layout, Margin, Panel, RichText, ScrollArea, Sense, TextStyle,
    Ui,
};
use fleet_api::{
    eval, Agent, DiffReport, FieldDef, FieldKind, RepoInfo, RepoRequest, Tone, UiSchema, Workspace,
};
use fleet_client::schema::{column_for, field_value, FieldValue};

/// A longer line is cut for display so one pathological line can never stall a frame.
const MAX_LINE_CHARS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewTab {
    #[default]
    Terminal,
    Diff,
}

#[derive(Default)]
pub enum DiffView {
    #[default]
    NotLoaded,
    Loading,
    Loaded(DiffReport),
    Failed(String),
}

pub struct PreviewState {
    pub agent_id: String,
    pub title: String,
    /// `repo / branch`, drawn in the panel header.
    pub path: String,
    pub tab: PreviewTab,
    pub text: String,
    pub closed: bool,
    pub diff: DiffView,
    /// The reply box takes the cursor on the first frame the panel draws, if it has a box then; the panel clears
    /// this on that frame either way, so a box that appears later cannot steal the cursor from another field.
    pub focus_reply: bool,
    /// The "Add repo" picker is open under the Repos row.
    pub adding: bool,
    pub picker: PickerState,
    /// Entries chosen in the picker that `AddRepo` has not been returned for yet; one leaves per frame, because
    /// a frame returns one action.
    pub queue: std::collections::VecDeque<String>,
}

impl PreviewState {
    /// An empty panel; `browse` is the Add repo picker's Browse… dialog.
    pub fn new(browse: FolderPicker) -> PreviewState {
        PreviewState {
            agent_id: String::new(),
            title: String::new(),
            path: String::new(),
            tab: PreviewTab::default(),
            text: String::new(),
            closed: false,
            diff: DiffView::default(),
            focus_reply: false,
            adding: false,
            picker: PickerState::new(browse),
            queue: Default::default(),
        }
    }
}

/// The reply box's inputs: the agent's input action label (the field's hint) and the text kept for this agent.
pub struct Reply<'a> {
    pub label: &'a str,
    pub text: &'a mut String,
}

/// The repos row's inputs, and the pending repo request if the agent has one.
pub struct RepoPanel<'a> {
    pub workspaces: &'a [Workspace],
    /// Offered as chips in the Add repo picker.
    pub remembered: &'a [RepoInfo],
    /// When set, its box replaces the reply box.
    pub request: Option<RequestDraft<'a>>,
    /// Why Add repo cannot run now, if it cannot.
    pub add_blocked: Option<&'a str>,
    /// The agent is stopped: nobody runs the project, so Add repo is not offered.
    pub stopped: bool,
}

/// A pending repo request and the owner's draft for it, kept per agent by the app.
pub struct RequestDraft<'a> {
    pub request: &'a RepoRequest,
    pub target: &'a mut String,
    pub reason: &'a mut String,
    /// This agent's Browse… dialog for the target.
    pub browse: &'a mut FolderPicker,
    /// Why Grant and Refuse cannot run now (a decision for this agent is in flight, or the daemon is
    /// restarting), if they cannot.
    pub blocked: Option<&'a str>,
}

/// What the panel's details block reads: the agent, and the schema that names and shows its fields.
pub struct AgentDetails<'a> {
    pub schema: &'a UiSchema,
    pub agent: &'a Agent,
    /// For the time in status.
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewAction {
    None,
    /// The app should fetch the diff and put the result in `PreviewState::diff`.
    LoadDiff,
    Close,
    /// The user sent this text from the reply box; the app runs the input action and clears the box.
    Reply(String),
    /// Add this remembered name, path or URL to the project.
    AddRepo(String),
    /// Grant the pending repo request, with this target.
    Grant(String),
    /// Refuse the pending repo request, with this reason (maybe empty).
    Refuse(String),
}

/// The side panel: the agent's terminal text and its diff, under the title, the details block when `details` is
/// given and a Repos row when `repos` is given; at the bottom the repo request box (when `repos` carries a request)
/// or else a reply box when `reply` is given.
pub fn preview_pane(
    ui: &mut Ui,
    st: &mut PreviewState,
    reply: Option<Reply<'_>>,
    repos: Option<RepoPanel<'_>>,
    details: Option<AgentDetails<'_>>,
) -> PreviewAction {
    let mut action = PreviewAction::None;
    let p = tokens(ui.visuals().dark_mode);
    let item_spacing = ui.spacing().item_spacing;
    // The bands and their rules touch; only the body gets the usual spacing back.
    ui.spacing_mut().item_spacing.y = 0.0;

    let focus_reply = std::mem::take(&mut st.focus_reply);
    // Always shown for an agent, even with no repo and nothing remembered: Add repo is then the only way in.
    let show_row = repos.is_some();
    let (workspaces, remembered, request, add_blocked, stopped) = match repos {
        Some(RepoPanel {
            workspaces,
            remembered,
            request,
            add_blocked,
            stopped,
        }) => (workspaces, remembered, request, add_blocked, stopped),
        None => (&[][..], &[][..], None, None, false),
    };
    // Shown before the body so the body fills only what is left; the panel's own separator is the top border.
    if let Some(RequestDraft {
        request,
        target,
        reason,
        browse,
        blocked,
    }) = request
    {
        Panel::bottom("reply")
            .frame(Frame::new().fill(p.surface).inner_margin(Margin::same(12)))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = item_spacing;
                // Salted by agent for the same reason as the reply box: cursor and undo history are keyed by widget id.
                ui.push_id(&st.agent_id, |ui| {
                    // The first field takes the cursor when the panel opens on an agent that is waiting for a repo.
                    if focus_reply {
                        let target_id = ui.make_persistent_id("repo_request_target");
                        ui.memory_mut(|m| m.request_focus(target_id));
                    }
                    match repo_request_box(ui, request, target, reason, browse, blocked) {
                        Some(RequestDecision::Grant(t)) => action = PreviewAction::Grant(t),
                        Some(RequestDecision::Refuse(r)) => action = PreviewAction::Refuse(r),
                        None => {}
                    }
                });
            });
    } else if let Some(Reply { label, text }) = reply {
        Panel::bottom("reply")
            .frame(Frame::new().fill(p.surface).inner_margin(Margin::same(12)))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = item_spacing;
                // Salted by agent: egui keys the cursor and undo history by widget id, and one agent's draft must not
                // be reachable (Ctrl+Z) from another's box.
                ui.push_id(&st.agent_id, |ui| {
                    if let Some(sent) = reply_box(ui, label, text, focus_reply) {
                        action = PreviewAction::Reply(sent);
                    }
                });
            });
    }

    Frame::new()
        .inner_margin(Margin::symmetric(12, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // The rule under it makes up the column header's height, so the two bottom lines meet.
            ui.set_height(layout::COLUMN_HEADER - 1.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, Icon::Close, "Close").clicked() {
                    action = PreviewAction::Close;
                }
                // Truncated text shows the whole path in egui's own elision tooltip.
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(
                        Label::new(
                            RichText::new(&st.path)
                                .font(mono(size::SMALL))
                                .color(p.text_high),
                        )
                        .truncate(),
                    );
                });
            });
        });
    rule(ui);

    Frame::new()
        .inner_margin(Margin {
            left: 12,
            right: 12,
            top: 10,
            bottom: 6,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                Label::new(
                    RichText::new(&st.title)
                        .font(sans(ui, size::BODY, Weight::Medium))
                        .color(p.text_high),
                )
                .wrap(),
            );
        });

    if let Some(d) = details {
        Frame::new()
            .inner_margin(Margin {
                left: 12,
                right: 12,
                top: 0,
                bottom: 8,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = item_spacing;
                details_block(ui, &d);
            });
    }

    if show_row {
        Frame::new()
            .inner_margin(Margin {
                left: 12,
                right: 12,
                top: 0,
                bottom: 8,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = item_spacing;
                repos_row(ui, st, workspaces, remembered, add_blocked, !stopped);
            });
    }

    Frame::new()
        .inner_margin(Margin::symmetric(12, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if let Some(t) = tabs(
                    ui,
                    &[
                        (PreviewTab::Terminal, "Terminal"),
                        (PreviewTab::Diff, "Diff"),
                    ],
                    st.tab,
                ) {
                    st.tab = t;
                    if t == PreviewTab::Diff && matches!(st.diff, DiffView::NotLoaded) {
                        st.diff = DiffView::Loading;
                        if action == PreviewAction::None {
                            action = PreviewAction::LoadDiff;
                        }
                    }
                }
                if st.tab == PreviewTab::Diff {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if button(ui, "Refresh", ButtonVariant::Ghost).clicked() {
                            st.diff = DiffView::Loading;
                            if action == PreviewAction::None {
                                action = PreviewAction::LoadDiff;
                            }
                        }
                    });
                }
            });
        });
    rule(ui);

    // One queued entry per frame, and only when nothing else was asked this frame. While Add repo is blocked the
    // entries wait, with no repaint asked for: what unblocks it (the restart's result) wakes the window itself.
    if add_blocked.is_none() {
        if action == PreviewAction::None {
            if let Some(entry) = st.queue.pop_front() {
                action = PreviewAction::AddRepo(entry);
            }
        }
        if !st.queue.is_empty() {
            ui.ctx().request_repaint();
        }
    }

    Frame::new()
        .fill(p.bg)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = item_spacing;
            ui.set_min_size(ui.available_size());
            let note = |ui: &mut Ui, text: &str| {
                ui.label(RichText::new(text).size(size::SMALL).color(p.text_low));
            };
            match st.tab {
                PreviewTab::Terminal => {
                    // Before the rows: they fill the rest of the pane, so anything after them is pushed out of sight.
                    if st.closed {
                        note(ui, "Session ended");
                    }
                    screen_view(ui, "terminal", &st.text);
                }
                PreviewTab::Diff => diff_view(ui, "diff", &st.diff),
            }
        });
    action
}

/// What the fixed-size card cannot show: the status and time in it, every badge and the size, then a line per
/// other field the schema shows (the branch, fields this client does not know). The title above, the terminal
/// below and the Repos row show the task, the activity and the repos.
fn details_block(ui: &mut Ui, d: &AgentDetails<'_>) {
    let dark = ui.visuals().dark_mode;
    let p = tokens(dark);
    let shown: Vec<&FieldDef> = d
        .schema
        .card_fields
        .iter()
        .filter(|f| eval(&f.show_when, d.agent))
        .filter(|f| match field_value(d.agent, &f.key) {
            FieldValue::Absent => false,
            FieldValue::List(items) => !items.is_empty(),
            _ => true,
        })
        .collect();
    let of_kind = |kind: FieldKind| shown.iter().filter(move |f| f.kind == kind);
    ui.horizontal_wrapped(|ui| {
        if let Some(column) = column_for(d.schema, d.agent) {
            let font = sans(ui, size::SMALL, Weight::Medium);
            text_dot(ui, tone_color(column.tone, dark), 7.0, &font);
            ui.label(RichText::new(&column.title).font(font).color(p.text_high));
        }
        for f in of_kind(FieldKind::Duration) {
            field(ui, f, d.agent, d.now);
        }
        for f in of_kind(FieldKind::Badge).filter(|f| f.key != "repos") {
            for (text, tone) in badge_items(f, d.agent) {
                badge(ui, &text, &f.label, tone);
            }
        }
        for f in of_kind(FieldKind::Bytes) {
            field(ui, f, d.agent, d.now);
        }
    });
    let others = shown.iter().filter(|f| {
        matches!(f.kind, FieldKind::Text | FieldKind::Mono)
            && !matches!(f.key.as_str(), "task" | "activity")
    });
    for f in others {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(&f.label)
                    .font(sans(ui, size::SMALL, Weight::Medium))
                    .color(p.text_low),
            );
            field(ui, f, d.agent, d.now);
        });
    }
}

/// Terminal text, read-only and stuck to its bottom, filling the rest of `ui`. `id` keeps its scroll apart.
pub(crate) fn screen_view(ui: &mut Ui, id: &str, text: &str) {
    // tmux sends the whole pane, empty rows under short output included; kept, they would be what the
    // bottom-stuck view shows instead of the text.
    let rows: Vec<Row> = text.trim_end().lines().map(Row::Plain).collect();
    rows_view(ui, id, &rows, true);
}

/// Each repo's diff, as the side panel's Diff tab shows it, filling the rest of `ui`.
pub(crate) fn diff_view(ui: &mut Ui, id: &str, diff: &DiffView) {
    let p = tokens(ui.visuals().dark_mode);
    match diff {
        DiffView::NotLoaded | DiffView::Loading => {
            ui.label(
                RichText::new("Loading…")
                    .size(size::SMALL)
                    .color(p.text_low),
            );
        }
        DiffView::Failed(message) => {
            ui.label(RichText::new(message).color(p.danger));
        }
        DiffView::Loaded(report) => {
            let mut rows: Vec<Row> = Vec::new();
            for w in &report.workspaces {
                rows.push(Row::Heading(&w.repo));
                if w.diff.is_empty() {
                    rows.push(Row::Note("No changes"));
                }
                rows.extend(w.diff.lines().map(Row::Diff));
                if w.truncated {
                    rows.push(Row::Note("The diff was cut at the size limit."));
                }
            }
            rows_view(ui, id, &rows, false);
        }
    }
}

/// The chip tooltip: where the project branch started and, when that was a branch, how to merge it back.
pub fn workspace_tip(w: &Workspace) -> String {
    if w.owned {
        return format!("{} · created in this project", w.branch);
    }
    let start = &w.base[..w.base.len().min(7)];
    match &w.from_branch {
        // Nobody merges into a remote-tracking ref, so a URL repo's origin/<branch> start has no route to explain.
        Some(from) if from.starts_with("origin/") => {
            format!("{}, started from {from} at {start}", w.branch)
        }
        Some(from) => format!(
            "{}, started from {from} at {start}\n\n{}",
            w.branch,
            merge_note(&w.branch, from)
        ),
        None => format!("{}, started at {start}", w.branch),
    }
}

/// The Repos row: a badge per workspace, then Add repo when `can_add`, which opens the picker. While `blocked`,
/// Add repo and Add repos are disabled with the reason on hover.
fn repos_row(
    ui: &mut Ui,
    st: &mut PreviewState,
    workspaces: &[Workspace],
    remembered: &[RepoInfo],
    blocked: Option<&str>,
    can_add: bool,
) {
    let p = tokens(ui.visuals().dark_mode);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("Repos")
                .font(sans(ui, size::SMALL, Weight::Medium))
                .color(p.text_low),
        );
        for w in workspaces {
            badge(ui, &w.repo, &workspace_tip(w), BadgeTone::Neutral);
        }
        if can_add
            && ui
                .add_enabled_ui(blocked.is_none(), |ui| {
                    button(ui, "Add repo", ButtonVariant::Ghost)
                })
                .inner
                .on_disabled_hover_text(blocked.unwrap_or_default())
                .clicked()
        {
            st.adding = !st.adding;
        }
    });
    // A picker left open when the agent stopped goes with the button.
    if can_add && st.adding {
        ui.add_space(6.0);
        // Salted by agent so one agent's half-typed entry is not another's.
        ui.push_id(&st.agent_id, |ui| {
            repo_picker(ui, "add_repo", remembered, &mut st.picker)
        });
        ui.add_space(6.0);
        let ready = blocked.is_none()
            && (!st.picker.chosen.is_empty() || !st.picker.typed.trim().is_empty());
        ui.horizontal(|ui| {
            // Text left in the field counts, as in the New project dialog, so it is never silently dropped.
            let add = ui
                .add_enabled_ui(ready, |ui| button(ui, "Add repos", ButtonVariant::Primary))
                .inner;
            // Only a block has a reason to show; with nothing chosen the empty field says enough.
            let add = match blocked {
                Some(why) => add.on_disabled_hover_text(why),
                None => add,
            };
            if add.clicked() {
                st.picker.commit_typed();
                st.queue.extend(std::mem::take(&mut st.picker.chosen));
                st.adding = false;
            }
        });
    }
}

/// The 1 px line under a band, edge to edge.
fn rule(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 0.0, tokens(ui.visuals().dark_mode).border);
}

enum Row<'a> {
    Plain(&'a str),
    Diff(&'a str),
    Heading(&'a str),
    Note(&'a str),
}

fn clip(line: &str) -> &str {
    match line.char_indices().nth(MAX_LINE_CHARS) {
        Some((i, _)) => &line[..i],
        None => line,
    }
}

/// Draws only the rows in view, so a megabyte of text costs one screen of layout per frame.
fn rows_view(ui: &mut Ui, id: &str, rows: &[Row<'_>], stick: bool) {
    let dark = ui.visuals().dark_mode;
    let p = tokens(dark);
    let height = ui.text_style_height(&TextStyle::Monospace);
    // Code size, not body size, so a heading still fits its slot in the row grid.
    let heading_font = sans(ui, size::CODE, Weight::Medium);
    // One row is one line, however long: a wrapped row would be taller than its slot and break the row grid.
    let line = |ui: &mut Ui, rich: RichText| {
        ui.add(egui::Label::new(rich).wrap_mode(egui::TextWrapMode::Extend));
    };
    ScrollArea::both()
        .id_salt(id)
        .auto_shrink([false, false])
        .stick_to_bottom(stick)
        .show_rows(ui, height, rows.len(), |ui, range| {
            for row in &rows[range] {
                match row {
                    Row::Plain(l) => line(ui, RichText::new(clip(l)).monospace().color(p.text)),
                    Row::Diff(l) => {
                        let colour = if l.starts_with("+++") || l.starts_with("---") {
                            p.text_low
                        } else if l.starts_with('+') {
                            tone_color(Tone::Done, dark)
                        } else if l.starts_with('-') {
                            p.danger
                        } else if l.starts_with("@@") {
                            tone_color(Tone::Active, dark)
                        } else {
                            p.text
                        };
                        line(ui, RichText::new(clip(l)).monospace().color(colour));
                    }
                    Row::Heading(r) => line(
                        ui,
                        RichText::new(*r)
                            .font(heading_font.clone())
                            .color(p.text_high),
                    ),
                    Row::Note(n) => line(ui, RichText::new(*n).size(size::SMALL).color(p.text_low)),
                }
            }
        });
}
