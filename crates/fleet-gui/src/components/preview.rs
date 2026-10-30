use super::breakdown::{breakdown_panel_deciding, pending, BreakdownAction};
use super::folder_picker::merge_note;
use super::reply::reply_box;
use super::repo_request::{repo_request_box, RequestDecision};
use crate::theme::{layout, mono, sans, size, tokens, tone_color, Weight};
use crate::widgets::{
    badge, badge_items, button, field, fui, icon_button, tabs, text_dot, BadgeTone, ButtonVariant,
    Icon,
};
use eframe::egui::{
    self, vec2, Align, Frame, Label, Layout, Margin, Panel, RichText, ScrollArea, Sense, TextStyle,
    Ui,
};
use fleet_api::{
    breakdown_decision, eval, repo_request, Agent, AgentKind, DiffReport, Effort, FieldDef,
    FieldKind, FleetView, FormDef, PermissionMode, RepoInfo, RepoRequest, Tone, UiSchema,
    Workspace,
};
use fleet_client::schema::{column_for, field_value, usable_form, FieldValue};
use std::collections::BTreeMap;

// Lite's egui margins are whole pixels.
const M: i8 = crate::theme::panel::MARGIN as i8;
const TITLE_TOP: i8 = crate::theme::panel::TITLE_TOP as i8;
const TITLE_BOTTOM: i8 = crate::theme::panel::TITLE_BOTTOM as i8;
const BAND_GAP: i8 = crate::theme::panel::BAND_GAP as i8;
const BODY_Y: i8 = crate::theme::panel::BODY_Y as i8;

/// A longer line is cut for display so one pathological line can never stall a frame.
const MAX_LINE_CHARS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewTab {
    #[default]
    Terminal,
    Diff,
    /// An orchestrator's breakdown and test gate.
    Breakdown,
}

#[derive(Default)]
pub enum DiffView {
    #[default]
    NotLoaded,
    Loading,
    Loaded(DiffReport),
    Failed(String),
}

/// How the panel sits on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelLayout {
    /// A resizable panel at the board's right edge.
    #[default]
    Side,
    /// The panel covers the board. `forced`: the window is too narrow to keep the board, so the user cannot go back.
    Full { forced: bool },
}

/// The layout for a panel `panel_width` wide on a `board_width` area: it covers the board when it would leave
/// less than a quarter of it.
pub fn panel_layout(board_width: f32, panel_width: f32, chosen_full: bool) -> PanelLayout {
    if board_width - panel_width < board_width * layout::BOARD_KEEP {
        PanelLayout::Full { forced: true }
    } else if chosen_full {
        PanelLayout::Full { forced: false }
    } else {
        PanelLayout::Side
    }
}

pub struct PreviewState {
    /// Set by the app each frame, before the panel is drawn: it decides the fullscreen button.
    pub layout: PanelLayout,
    /// Set by the app each frame: why Open in terminal cannot run now, or `None` when it can.
    pub terminal_blocked: Option<&'static str>,
    pub agent_id: String,
    /// Set by the app each frame, so a title the agent sets shows while the panel is open.
    pub title: String,
    /// The goal in full, under the title when the two differ.
    pub goal: String,
    /// `repo / branch`, drawn in the panel header.
    pub path: String,
    pub tab: PreviewTab,
    pub text: String,
    pub closed: bool,
    pub diff: DiffView,
    /// The reply box takes the cursor on the first frame the panel draws, if it has a box then; the panel clears
    /// this on that frame either way, so a box that appears later cannot steal the cursor from another field.
    pub focus_reply: bool,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewState {
    pub fn new() -> PreviewState {
        PreviewState {
            layout: PanelLayout::Side,
            terminal_blocked: None,
            agent_id: String::new(),
            title: String::new(),
            goal: String::new(),
            path: String::new(),
            tab: PreviewTab::default(),
            text: String::new(),
            closed: false,
            diff: DiffView::default(),
            focus_reply: false,
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
    /// The product's repos: a repo request is granted from these.
    pub remembered: &'a [RepoInfo],
    /// When set, its box replaces the reply box.
    pub request: Option<RequestDraft<'a>>,
}

/// A pending repo request and the owner's draft for it, kept per agent by the app.
pub struct RequestDraft<'a> {
    pub request: &'a RepoRequest,
    pub target: &'a mut String,
    pub reason: &'a mut String,
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

/// What an orchestrated task adds to the panel: the Breakdown tab on its orchestrator, a link to the task on
/// a fleet agent.
pub enum FleetPanel<'a> {
    Task(TaskPanel<'a>),
    /// A fleet agent's task, by its task, and the id of the agent that leads it now.
    Member {
        task: &'a str,
        lead: &'a str,
    },
}

/// The Breakdown tab's inputs.
pub struct TaskPanel<'a> {
    /// `None` until the first fetch answers.
    pub view: Option<&'a Result<FleetView, String>>,
    /// The task's agents, which the step table names.
    pub agents: Vec<&'a Agent>,
    /// The task's persona's nicknames for its specialists.
    pub nicknames: &'a BTreeMap<String, String>,
    /// The orchestrator waits on its breakdown, so the tab comes first.
    pub first: bool,
    /// A decision is in flight, or landed and the view showing it has not: Approve and Request changes wait.
    pub busy: bool,
    pub draft: &'a mut String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewAction {
    None,
    /// The app should fetch the diff and put the result in `PreviewState::diff`.
    LoadDiff,
    Close,
    /// Switch between the side panel and the full board area.
    ToggleFullscreen,
    /// Open the agent's session in a terminal emulator.
    OpenTerminal,
    /// The user sent this text from the reply box; the app runs the input action and clears the box.
    Reply(String),
    /// Grant the pending repo request, with this target.
    Grant(String),
    /// Refuse the pending repo request, with this reason (maybe empty).
    Refuse(String),
    /// Approve the task's proposed breakdown.
    Approve,
    /// Send the proposed breakdown back with this comment.
    RequestChanges(String),
    /// Open this agent's panel instead.
    Open(String),
}

/// The side panel: the agent's terminal text and its diff, under the title, the details block when `details` is
/// given and a Repos row when `repos` is given; at the bottom the repo request box (when `repos` carries a request)
/// or else a reply box when `reply` is given. `fleet` adds the Breakdown tab or the link to the agent's task.
pub fn preview_pane(
    ui: &mut Ui,
    st: &mut PreviewState,
    reply: Option<Reply<'_>>,
    repos: Option<RepoPanel<'_>>,
    details: Option<AgentDetails<'_>>,
    fleet: Option<FleetPanel<'_>>,
) -> PreviewAction {
    let mut action = PreviewAction::None;
    let p = tokens(ui.visuals().dark_mode);
    let item_spacing = ui.spacing().item_spacing;
    // The bands and their rules touch; only the body gets the usual spacing back.
    ui.spacing_mut().item_spacing.y = 0.0;

    let focus_reply = std::mem::take(&mut st.focus_reply);
    let (mut task, member) = match fleet {
        Some(FleetPanel::Task(panel)) => (Some(panel), None),
        Some(FleetPanel::Member { task, lead }) => (None, Some((task, lead))),
        None => (None, None),
    };
    // The tab of an orchestrator left for one of its fleet agents' panels.
    if st.tab == PreviewTab::Breakdown && task.is_none() {
        st.tab = PreviewTab::Terminal;
    }
    // The daemon's forms, read before `details` is used up below.
    let schema = details.as_ref().map(|d| d.schema);
    let decision = usable_form(schema, "breakdown_decision").unwrap_or_else(breakdown_decision);
    let request_form = usable_form(schema, "repo_request").unwrap_or_else(repo_request);
    let show_row = repos.is_some();
    let (workspaces, remembered, request) = match repos {
        Some(RepoPanel {
            workspaces,
            remembered,
            request,
        }) => (workspaces, remembered, request),
        None => (&[][..], &[][..], None),
    };
    // Shown before the body so the body fills only what is left; the panel's own separator is the top border.
    if let Some(RequestDraft {
        request,
        target,
        reason,
        blocked,
    }) = request
    {
        Panel::bottom("reply")
            .frame(Frame::new().fill(p.surface).inner_margin(Margin::same(M)))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = item_spacing;
                // Salted by agent for the same reason as the reply box: cursor and undo history are keyed by widget id.
                ui.push_id(&st.agent_id, |ui| {
                    // The reason field takes the cursor when the panel opens on an agent that is waiting for a repo.
                    if focus_reply {
                        let reason_id = ui.make_persistent_id("repo_request_reason");
                        ui.memory_mut(|m| m.request_focus(reason_id));
                    }
                    match repo_request_box(
                        ui,
                        &request_form,
                        request,
                        remembered,
                        target,
                        reason,
                        blocked,
                    ) {
                        Some(RequestDecision::Grant(t)) => action = PreviewAction::Grant(t),
                        Some(RequestDecision::Refuse(r)) => action = PreviewAction::Refuse(r),
                        None => {}
                    }
                });
            });
    } else if let Some(Reply { label, text }) = reply {
        Panel::bottom("reply")
            .frame(Frame::new().fill(p.surface).inner_margin(Margin::same(M)))
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
        .inner_margin(Margin::symmetric(M, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // The rule under it makes up the column header's height, so the two bottom lines meet.
            ui.set_height(layout::COLUMN_HEADER - 1.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, Icon::Close, "Close").clicked() {
                    action = PreviewAction::Close;
                }
                // Buttons are added right to left, so a new one goes after this block to sit on its left.
                match st.layout {
                    PanelLayout::Side => {
                        if icon_button(ui, Icon::Expand, "Fullscreen").clicked() {
                            action = PreviewAction::ToggleFullscreen;
                        }
                    }
                    PanelLayout::Full { forced: false } => {
                        if icon_button(ui, Icon::Collapse, "Back to the board").clicked() {
                            action = PreviewAction::ToggleFullscreen;
                        }
                    }
                    PanelLayout::Full { forced: true } => {
                        ui.add_enabled_ui(false, |ui| {
                            icon_button(ui, Icon::Collapse, "Back to the board")
                                .on_disabled_hover_text("the window is too narrow for the board");
                        });
                    }
                }
                match st.terminal_blocked {
                    None => {
                        if icon_button(ui, Icon::Terminal, "Open in terminal").clicked() {
                            action = PreviewAction::OpenTerminal;
                        }
                    }
                    Some(why) => {
                        ui.add_enabled_ui(false, |ui| {
                            icon_button(ui, Icon::Terminal, "Open in terminal")
                                .on_disabled_hover_text(why);
                        });
                    }
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
            left: M,
            right: M,
            top: TITLE_TOP,
            bottom: TITLE_BOTTOM,
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
            if st.goal != st.title {
                ui.add(
                    Label::new(
                        RichText::new(&st.goal)
                            .font(sans(ui, size::SMALL, Weight::Regular))
                            .color(p.text_low),
                    )
                    .wrap(),
                );
            }
        });

    if let Some((goal, lead)) = member {
        Frame::new()
            .inner_margin(Margin {
                left: M,
                right: M,
                top: 0,
                bottom: BAND_GAP,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let link = ui.link(RichText::new(format!("↳ {goal}")).size(size::SMALL));
                if link.clicked() {
                    action = PreviewAction::Open(lead.to_string());
                }
            });
    }

    if let Some(d) = details {
        Frame::new()
            .inner_margin(Margin {
                left: M,
                right: M,
                top: 0,
                bottom: BAND_GAP,
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
                left: M,
                right: M,
                top: 0,
                bottom: BAND_GAP,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = item_spacing;
                repos_row(ui, workspaces);
            });
    }

    Frame::new()
        .inner_margin(Margin::symmetric(M, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let mut list = vec![
                    (PreviewTab::Terminal, "Terminal"),
                    (PreviewTab::Diff, "Diff"),
                ];
                match &task {
                    Some(p) if p.first => list.insert(0, (PreviewTab::Breakdown, "Breakdown")),
                    Some(_) => list.push((PreviewTab::Breakdown, "Breakdown")),
                    None => {}
                }
                if let Some(t) = tabs(ui, &list, st.tab) {
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

    Frame::new()
        .fill(p.bg)
        .inner_margin(Margin::symmetric(M, BODY_Y))
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
                PreviewTab::Breakdown => {
                    if let Some(panel) = task.as_mut() {
                        let asked = breakdown_tab(ui, panel, &decision);
                        if action == PreviewAction::None {
                            action = asked;
                        }
                    }
                }
            }
        });
    action
}

/// The Breakdown tab's body: a note until the view arrives, then the breakdown, scrolling on its own.
fn breakdown_tab(ui: &mut Ui, panel: &mut TaskPanel<'_>, decision: &FormDef) -> PreviewAction {
    let p = tokens(ui.visuals().dark_mode);
    let view = match panel.view {
        None => {
            ui.label(
                RichText::new("Loading…")
                    .size(size::SMALL)
                    .color(p.text_low),
            );
            return PreviewAction::None;
        }
        Some(Err(e)) => {
            ui.label(RichText::new(e).color(p.danger));
            return PreviewAction::None;
        }
        Some(Ok(view)) => view,
    };
    let decidable = pending(view);
    ScrollArea::vertical()
        .id_salt("breakdown")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            breakdown_panel_deciding(
                ui,
                view,
                &panel.agents,
                panel.nicknames,
                decidable,
                panel.busy,
                panel.draft,
                decision,
            )
        })
        .inner
        .into()
}

impl From<BreakdownAction> for PreviewAction {
    fn from(a: BreakdownAction) -> PreviewAction {
        match a {
            BreakdownAction::None => PreviewAction::None,
            BreakdownAction::Approve => PreviewAction::Approve,
            BreakdownAction::RequestChanges(c) => PreviewAction::RequestChanges(c),
            BreakdownAction::Open(id) => PreviewAction::Open(id),
        }
    }
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
    let column = column_for(d.schema, d.agent);
    ui.horizontal_wrapped(|ui| {
        if let Some(column) = column {
            let font = sans(ui, size::SMALL, Weight::Medium);
            text_dot(ui, tone_color(column.tone, dark), 7.0, &font);
            fui::mono_label(ui, &column.title, p.text_high);
        }
        for f in of_kind(FieldKind::Duration) {
            field(ui, f, d.agent, d.now);
        }
        for f in of_kind(FieldKind::Badge).filter(|f| f.key != "repos") {
            for (text, tone) in badge_items(f, d.agent) {
                // A stopped task's cause reads "Stopped" under the column's own "Stopped".
                if column.is_some_and(|c| c.title == text) {
                    continue;
                }
                badge(ui, &text, &f.label, tone);
            }
        }
        for f in of_kind(FieldKind::Bytes) {
            field(ui, f, d.agent, d.now);
        }
    });
    let others = shown.iter().filter(|f| {
        matches!(f.kind, FieldKind::Text | FieldKind::Mono)
            && !matches!(f.key.as_str(), "title" | "activity")
    });
    for f in others {
        ui.horizontal(|ui| {
            fui::mono_label(ui, &f.label, p.text_low);
            field(ui, f, d.agent, d.now);
        });
    }
    // Only a fleet agent has a resolved model; a solo agent runs on whatever its persona's Claude is set to.
    if d.agent.kind != AgentKind::Solo {
        let model = d.agent.model.as_deref().unwrap_or("persona default");
        ui.label(
            RichText::new(format!("Model: {model}"))
                .font(mono(size::META))
                .color(p.text_low),
        );
    }
    if let Some(line) = mode_effort_line(d.agent.mode, d.agent.effort) {
        ui.label(RichText::new(line).font(mono(size::META)).color(p.text_low));
    }
}

/// `Mode: auto · Effort: high`; nothing for an agent that predates them.
fn mode_effort_line(mode: Option<PermissionMode>, effort: Option<Effort>) -> Option<String> {
    let parts: Vec<String> = mode
        .map(|m| format!("Mode: {}", m.as_flag()))
        .into_iter()
        .chain(effort.map(|e| format!("Effort: {}", e.as_flag())))
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Terminal text, read-only and stuck to its bottom, filling the rest of `ui`. `id` keeps its scroll apart.
pub(crate) fn screen_view(ui: &mut Ui, id: &str, text: &str) {
    // The daemon sends the whole screen, empty rows under short output included; kept, they would be what the
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

/// The chip tooltip: where the task branch started and, when that was a branch, how to merge it back.
pub fn workspace_tip(w: &Workspace) -> String {
    if w.owned {
        return format!("{} · created in this task", w.branch);
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

/// The Repos row: a badge per workspace. Repos come from the product, so nothing is added here.
fn repos_row(ui: &mut Ui, workspaces: &[Workspace]) {
    let p = tokens(ui.visuals().dark_mode);
    ui.horizontal_wrapped(|ui| {
        fui::mono_label(ui, "Repos", p.text_low);
        for w in workspaces {
            badge(ui, &w.repo, &workspace_tip(w), BadgeTone::Neutral);
        }
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_layout_is_side_while_a_quarter_of_the_board_stays_visible() {
        assert_eq!(panel_layout(1200.0, 420.0, false), PanelLayout::Side);
    }

    #[test]
    fn panel_layout_is_forced_full_when_the_board_would_shrink_under_a_quarter() {
        assert_eq!(
            panel_layout(500.0, 420.0, false),
            PanelLayout::Full { forced: true }
        );
    }

    #[test]
    fn panel_layout_is_chosen_full_on_request() {
        assert_eq!(
            panel_layout(1200.0, 420.0, true),
            PanelLayout::Full { forced: false }
        );
    }
}
