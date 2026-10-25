use eframe::egui::{self, accesskit::Role, Vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use fleet_api::{
    default_ui_schema, Agent, AgentId, BlockedReason, ColumnDef, Status, Tone, UiSchema, Workspace,
};
use fleet_gui::components::{board, CardState, Command};
use fleet_gui::theme;
use std::collections::HashMap;
use std::time::Duration;

const NOW: i64 = 1300;

fn ws(repo: &str) -> Workspace {
    Workspace {
        repo: repo.into(),
        branch: "fleet/fix".into(),
        worktree: format!("/w/{repo}"),
        base: "abc".into(),
        from_branch: None,
        has_changes: false,
        owned: false,
    }
}

fn agent(id: &str, task: &str, status: Status) -> Agent {
    Agent {
        id: AgentId(id.into()),
        task: task.into(),
        workdir: "/w".into(),
        workspaces: vec![ws("core")],
        tmux_session: format!("fleet-{id}"),
        status,
        activity: "running Bash".into(),
        status_since: 1000,
        has_changes: false,
        paused: false,
        owner: "antonio".into(),
        blocked_reason: None,
        repo_request: None,
        stop_cause: None,
        disk_bytes: None,
    }
}

#[derive(Default)]
struct S {
    states: HashMap<String, CardState>,
    commands: Vec<Command>,
}

fn harness(schema: UiSchema, agents: Vec<Agent>) -> Harness<'static, S> {
    Harness::builder()
        .with_size(Vec2::new(1400.0, 800.0))
        .build_ui_state(
            move |ui, s: &mut S| {
                let cmds = board(ui, &schema, &agents, NOW, &mut s.states, None);
                s.commands.extend(cmds);
            },
            S::default(),
        )
}

fn x(h: &Harness<'_, S>, label: &str) -> f32 {
    h.get_by_label(label).rect().center().x
}

fn y(h: &Harness<'_, S>, label: &str) -> f32 {
    h.get_by_label(label).rect().center().y
}

fn needs_you() -> Agent {
    let mut a = agent("n1", "ask me", Status::NeedsYou);
    a.activity = "wants to run Bash".into();
    a.blocked_reason = Some(BlockedReason::Permission);
    a
}

#[test]
fn board_renders_columns_from_schema_only() {
    let custom = UiSchema {
        columns: vec![
            ColumnDef {
                id: "left".into(),
                title: "Alpha lane".into(),
                statuses: vec![Status::Working],
                empty_hint: "nothing left".into(),
                tone: Tone::Active,
            },
            ColumnDef {
                id: "right".into(),
                title: "Beta lane".into(),
                statuses: vec![Status::Review],
                empty_hint: "nothing right".into(),
                tone: Tone::Done,
            },
        ],
        ..default_ui_schema()
    };
    let h = harness(custom, vec![agent("w", "the working one", Status::Working)]);
    h.get_by_label("Alpha lane");
    h.get_by_label("Beta lane");
    h.get_by_label("the working one");
    h.get_by_label("nothing right");
    assert!(
        h.query_by_label("Needs you").is_none() && h.query_by_label("Queued").is_none(),
        "a column the schema lacks was drawn"
    );
}

#[test]
fn each_agent_lands_in_the_column_the_schema_maps_its_status_to() {
    let agents = vec![
        agent("q", "t-q", Status::Queued),
        agent("d", "t-d", Status::Stopped),
        agent("r", "t-r", Status::Review),
        needs_you(),
    ];
    let h = harness(default_ui_schema(), agents);
    let (queued, needs, review) = (x(&h, "Queued"), x(&h, "Needs you"), x(&h, "Review"));
    let near = |card: f32, col: f32| (card - col).abs() < 200.0;
    assert!(
        near(x(&h, "t-q"), queued),
        "Queued card is not under Queued"
    );
    assert!(
        near(x(&h, "ask me"), needs),
        "Needs you card is not under Needs you"
    );
    assert!(
        near(x(&h, "t-d"), review) && near(x(&h, "t-r"), review),
        "Dead and Review cards belong in Review"
    );
    h.get_by_label("Agent conversations appear here");
}

#[test]
fn longest_waiting_card_first() {
    let mut newer = agent("new", "newer", Status::Working);
    newer.status_since = 900;
    let mut older = agent("old", "older", Status::Working);
    older.status_since = 100;
    let h = harness(default_ui_schema(), vec![newer, older]);
    assert!(
        y(&h, "older") < y(&h, "newer"),
        "the longest-waiting card must come first"
    );
}

#[test]
fn card_shows_project_task_activity_and_time_waiting_for_needs_you() {
    let h = harness(default_ui_schema(), vec![needs_you()]);
    for label in [
        "core",
        "ask me",
        "wants to run Bash",
        "Permission needed",
        "5m",
    ] {
        h.get_by_label(label);
    }
}

#[test]
fn card_shows_only_fields_whose_condition_holds() {
    let h = harness(
        default_ui_schema(),
        vec![agent("w", "working", Status::Working)],
    );
    assert!(h.query_by_label("Permission needed").is_none());
    assert!(h.query_by_label("Paused").is_none());
    assert!(h.query_by_label("Has changes").is_none());
}

#[test]
fn card_shows_a_changes_badge_only_when_the_agent_has_changes() {
    let mut a = agent("r", "done", Status::Review);
    a.has_changes = true;
    harness(default_ui_schema(), vec![a]).get_by_label("Has changes");
}

#[test]
fn one_badge_per_workspace_repo() {
    let mut a = agent("w", "two repos", Status::Working);
    a.workspaces = vec![ws("core"), ws("portal")];
    let h = harness(default_ui_schema(), vec![a]);
    h.get_by_label("core");
    h.get_by_label("portal");
}

/// Every filled rectangle painted this frame, with its fill.
fn painted_rects(h: &Harness<'_, S>) -> Vec<(egui::Rect, egui::Color32)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(egui::Rect, egui::Color32)>) {
        match shape {
            egui::Shape::Rect(r) => out.push((r.rect, r.fill)),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    h.output()
        .shapes
        .iter()
        .for_each(|s| walk(&s.shape, &mut out));
    out
}

#[test]
fn neighbouring_repos_and_badges_do_not_touch() {
    let mut a = needs_you();
    a.workspaces = vec![ws("core"), ws("portal")];
    a.has_changes = true;
    let mut h = harness(default_ui_schema(), vec![a]);
    h.run();
    let gap = h.get_by_label("portal").rect().left() - h.get_by_label("core").rect().right();
    assert!(gap >= 4.0, "repos are {gap}px apart");

    // A badge's text is inset by its frame, so the gap is measured between the painted frames.
    let dark = egui::Visuals::default().dark_mode;
    let frame = |fill: egui::Color32| {
        painted_rects(&h)
            .into_iter()
            .find(|(_, f)| *f == fill)
            .map(|(r, _)| r)
            .expect("badge frame painted")
    };
    let blocked = frame(theme::tone_tint(Tone::Attention, dark));
    let changes = frame(theme::tokens(dark).raised);
    assert!(
        (blocked.center().y - changes.center().y).abs() < 2.0,
        "the badges share a row"
    );
    let gap = changes.left() - blocked.right();
    assert!(gap >= 4.0, "badges are {gap}px apart");
}

#[test]
fn needs_you_card_has_no_answer_field() {
    let h = harness(default_ui_schema(), vec![needs_you()]);
    assert_eq!(
        h.query_all_by_role(Role::TextInput).count()
            + h.query_all_by_role(Role::MultilineTextInput).count(),
        0
    );
    assert!(h.query_by_role_and_label(Role::Button, "Send").is_none());
    h.get_by_label("Permission needed");
    h.get_by_label("Kill");
}

#[test]
fn kill_needs_a_second_click() {
    let mut h = harness(
        default_ui_schema(),
        vec![agent("w", "working", Status::Working)],
    );
    h.get_by_label("Kill").click();
    h.run();
    assert!(
        h.state().commands.is_empty(),
        "the first click only asks for confirmation"
    );
    h.get_by_label("Confirm Kill").click();
    h.run();
    let [Command::Run { action, .. }] = h.state().commands.as_slice() else {
        panic!("expected one command")
    };
    assert_eq!(action.id, "kill");
    assert!(
        h.query_by_label("Confirm Kill").is_none(),
        "confirmation resets after use"
    );
}

#[test]
fn plain_actions_run_on_one_click_with_no_input() {
    let mut h = harness(
        default_ui_schema(),
        vec![agent("w", "working", Status::Working)],
    );
    h.get_by_label("Pause").click();
    h.run();
    let [Command::Run { action, input, .. }] = h.state().commands.as_slice() else {
        panic!("expected one command")
    };
    assert_eq!((action.id.as_str(), input.is_none()), ("pause", true));
}

#[test]
fn clicking_the_card_body_returns_open() {
    let mut h = harness(
        default_ui_schema(),
        vec![agent("w", "open me", Status::Working)],
    );
    h.get_by_label("open me").click();
    h.run();
    assert_eq!(h.state().commands, [Command::Open("w".into())]);
}

#[test]
fn meta_row_puts_repos_left_and_time_right_on_one_line() {
    let h = harness(
        default_ui_schema(),
        vec![agent("w", "the task", Status::Working)],
    );
    assert!(
        (y(&h, "core") - y(&h, "5m")).abs() < 2.0,
        "repos and time share a line"
    );
    assert!(x(&h, "core") < x(&h, "5m"), "repos left, time right");
    assert!(
        y(&h, "core") < y(&h, "the task"),
        "the meta row sits above the task"
    );
}

#[test]
fn long_unbroken_task_and_branch_stay_inside_the_card() {
    let col_w = 1400.0 / 5.0;
    let board_with = |task: &str, branch: &str| {
        let mut a = agent("w", task, Status::Working);
        a.workspaces[0].branch = branch.into();
        harness(default_ui_schema(), vec![a])
    };
    let short = board_with("short", "fleet/fix");
    let long_task = "x".repeat(300);
    let long_branch = format!("fleet/{}", "y".repeat(300));
    let long = board_with(&long_task, &long_branch);

    let task = long.get_by_label(&long_task).rect();
    let body_row = short.get_by_label("short").rect().height();
    assert!(
        task.width() <= col_w,
        "task is {}px wide in a {col_w}px column",
        task.width()
    );
    assert!(
        task.height() <= 2.5 * body_row,
        "task is {}px tall, one row is {body_row}px",
        task.height()
    );

    let meta_row = long.get_by_label("core").rect().height();
    let branch = long.get_by_label(&long_branch).rect();
    assert!(
        branch.width() <= col_w && branch.height() <= 1.5 * meta_row,
        "branch is {}x{}",
        branch.width(),
        branch.height()
    );

    let gap = |h: &Harness<'_, S>| x(h, "Review") - x(h, "Needs you");
    assert_eq!(
        gap(&long),
        gap(&short),
        "long text must not resize the columns"
    );
}

/// The painted text layouts whose source is `text`.
fn painted_galleys(h: &Harness<'_, S>, text: &str) -> Vec<std::sync::Arc<egui::Galley>> {
    fn walk(shape: &egui::Shape, text: &str, out: &mut Vec<std::sync::Arc<egui::Galley>>) {
        match shape {
            egui::Shape::Text(t) if t.galley.job.text == text => out.push(t.galley.clone()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, text, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    h.output()
        .shapes
        .iter()
        .for_each(|s| walk(&s.shape, text, &mut out));
    out
}

#[test]
fn a_wrapped_task_breaks_between_words() {
    let task = "Add pagination to the shots list endpoint and keep the existing sort order stable across pages";
    let mut h = harness(default_ui_schema(), vec![agent("w", task, Status::Working)]);
    h.run();
    let [galley] = &painted_galleys(&h, task)[..] else {
        panic!("expected the task painted once")
    };
    let rows: Vec<String> = galley
        .rows
        .iter()
        .map(|r| r.glyphs.iter().map(|g| g.chr).collect())
        .collect();
    assert!(
        rows.len() >= 2,
        "the task should wrap in a fifth of the window: {rows:?}"
    );
    for pair in rows.windows(2) {
        let (end, start) = (
            pair[0].chars().last().unwrap(),
            pair[1].chars().next().unwrap(),
        );
        assert!(
            end.is_whitespace() || start.is_whitespace(),
            "a row break splits a word: {rows:?}"
        );
    }
    // Rows that break between words are short of the width; justified, their spaces would stretch to fill it.
    assert!(!galley.job.justify, "the task is laid out justified");
}

/// Centres of the painted filled circles of this radius.
fn circles(h: &Harness<'_, S>, radius: f32) -> Vec<egui::Pos2> {
    fn walk(shape: &egui::Shape, radius: f32, out: &mut Vec<egui::Pos2>) {
        match shape {
            egui::Shape::Circle(c) if (c.radius - radius).abs() < 0.01 => out.push(c.center),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, radius, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    h.output()
        .shapes
        .iter()
        .for_each(|s| walk(&s.shape, radius, &mut out));
    out
}

#[test]
fn the_working_dot_is_centred_on_its_activity_line() {
    let mut h = harness(
        default_ui_schema(),
        vec![agent("w", "busy", Status::Working)],
    );
    h.run();
    let [dot] = circles(&h, 2.5)[..] else {
        panic!("expected one 5 px activity dot")
    };
    let text = h.get_by_label("running Bash").rect();
    assert!(
        (dot.y - text.center().y).abs() <= 1.0,
        "dot at y={}, its text spans {:?}",
        dot.y,
        text.y_range()
    );
    assert!(dot.x < text.left(), "the dot comes before the text");
}

#[test]
fn working_card_shows_a_dot_and_draws_no_continuous_repaint() {
    let mut h = harness(
        default_ui_schema(),
        vec![agent("w", "busy", Status::Working)],
    );
    // `run` panics if anything keeps asking for repaints, as a pulsing dot would.
    h.run();
    h.get_by_label("running Bash");
}

/// The delay until egui wants the next frame; `Duration::MAX` when nothing asked for one.
fn repaint_delay(h: &Harness<'_, S>) -> Duration {
    h.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay
}

#[test]
fn working_dot_asks_for_frames_only_while_focused() {
    let mut h = harness(
        default_ui_schema(),
        vec![agent("w", "busy", Status::Working)],
    );
    // `run` ignores delayed repaints, so the delay itself is what tells a pulse from an idle window.
    h.step();
    assert_eq!(
        repaint_delay(&h),
        Duration::MAX,
        "an unfocused board must stay idle"
    );

    h.input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .focused = Some(true);
    h.step();
    assert!(
        repaint_delay(&h) <= theme::ANIMATION_FRAME,
        "the pulse schedules no frame: {:?}",
        repaint_delay(&h)
    );

    h.input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .focused = Some(false);
    h.step();
    assert_eq!(repaint_delay(&h), Duration::MAX);
}

#[test]
fn a_paused_agents_dot_stays_still_even_while_focused() {
    let mut a = agent("w", "on hold", Status::Working);
    a.paused = true;
    let mut h = harness(default_ui_schema(), vec![a]);
    h.input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .focused = Some(true);
    h.step();
    assert_eq!(
        repaint_delay(&h),
        Duration::MAX,
        "a paused agent's dot must not pulse"
    );
    assert_eq!(circles(&h, 2.5).len(), 1, "the dot is still drawn, at rest");
}

/// Hovers `label` with no tooltip delay and returns whether a second node carries the same text,
/// which only the tooltip can be.
fn tooltip_shows(label: &str, a: Agent) -> bool {
    let mut h = harness(default_ui_schema(), vec![a]);
    h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    h.run();
    h.get_by_label(label).hover();
    h.run();
    h.query_all_by_label(label).count() > 1
}

#[test]
fn hovering_a_cut_task_shows_its_full_text() {
    let task = "x".repeat(300);
    assert!(
        tooltip_shows(&task, agent("w", &task, Status::Working)),
        "no tooltip with the full task"
    );
}

#[test]
fn hovering_a_cut_branch_shows_its_full_text() {
    let branch = format!("fleet/{}", "y".repeat(300));
    let mut a = agent("w", "t", Status::Working);
    a.workspaces[0].branch = branch.clone();
    assert!(tooltip_shows(&branch, a), "no tooltip with the full branch");
}

#[test]
fn a_repo_access_card_shows_the_repo_access_badge() {
    let mut a = agent("r1", "wants a repo", Status::NeedsYou);
    a.blocked_reason = Some(BlockedReason::RepoAccess);
    let h = harness(default_ui_schema(), vec![a]);
    h.get_by_label("Repo access");
}
