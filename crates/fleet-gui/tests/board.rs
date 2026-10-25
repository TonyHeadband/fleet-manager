use eframe::egui::{self, accesskit::Role, Vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use fleet_api::{
    default_ui_schema, Agent, AgentId, BlockedReason, ColumnDef, Status, StopCause, Tone, UiSchema,
    Workspace,
};
use fleet_gui::components::{board, card, CardState, Command, CARD_HEIGHT};
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
        persona: "p".into(),
    }
}

#[derive(Default)]
struct S {
    states: HashMap<String, CardState>,
    commands: Vec<Command>,
}

fn harness(schema: UiSchema, agents: Vec<Agent>) -> Harness<'static, S> {
    harness_wide(1400.0, schema, agents)
}

fn harness_wide(width: f32, schema: UiSchema, agents: Vec<Agent>) -> Harness<'static, S> {
    Harness::builder()
        .with_size(Vec2::new(width, 800.0))
        .build_ui_state(
            move |ui, s: &mut S| {
                let cmds = board(
                    ui,
                    &schema,
                    &agents,
                    NOW,
                    &mut s.states,
                    None,
                    &HashMap::new(),
                );
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
                dim: false,
            },
            ColumnDef {
                id: "right".into(),
                title: "Beta lane".into(),
                statuses: vec![Status::Review],
                empty_hint: "nothing right".into(),
                tone: Tone::Done,
                dim: false,
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
    let stopped = x(&h, "Stopped");
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
        near(x(&h, "t-d"), stopped) && x(&h, "t-d") < queued,
        "a Stopped card belongs in the first column"
    );
    assert!(
        near(x(&h, "t-r"), review),
        "Review card is not under Review"
    );
    assert!(
        h.query_by_label("Talking").is_none(),
        "the Talking column is gone"
    );
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
fn needs_you_card_has_no_answer_field() {
    let h = harness(default_ui_schema(), vec![needs_you()]);
    assert_eq!(
        h.query_all_by_role(Role::TextInput).count()
            + h.query_all_by_role(Role::MultilineTextInput).count(),
        0
    );
    assert!(h.query_by_role_and_label(Role::Button, "Send").is_none());
    h.get_by_label("Permission needed");
    h.get_by_label("Stop");
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
fn a_long_unbroken_task_stays_inside_the_card() {
    let col_w = 1400.0 / 5.0;
    let board_with =
        |task: &str| harness(default_ui_schema(), vec![agent("w", task, Status::Working)]);
    let short = board_with("short");
    let long_task = "x".repeat(300);
    let long = board_with(&long_task);

    let task = long.get_by_label(&long_task).rect();
    let body_row = short.get_by_label("short").rect().height();
    assert!(
        task.width() <= col_w,
        "task is {}px wide in a {col_w}px column",
        task.width()
    );
    assert!(
        task.height() <= 1.5 * body_row,
        "task is {}px tall, one row is {body_row}px",
        task.height()
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
fn a_repo_access_card_shows_the_repo_access_badge() {
    let mut a = agent("r1", "wants a repo", Status::NeedsYou);
    a.blocked_reason = Some(BlockedReason::RepoAccess);
    let h = harness(default_ui_schema(), vec![a]);
    h.get_by_label("Repo access");
}

/// The painted card frames: `surface` rectangles with the card radius.
fn card_frames(h: &Harness<'_, S>) -> Vec<egui::Rect> {
    fn walk(shape: &egui::Shape, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Rect(r)
                if r.corner_radius == egui::CornerRadius::same(theme::radius::CARD) =>
            {
                out.push(r.rect)
            }
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

/// The card frame that holds the node labelled `label`.
fn card_of(h: &Harness<'_, S>, label: &str) -> egui::Rect {
    let at = h.get_by_label(label).rect().center();
    card_frames(h)
        .into_iter()
        .find(|r| r.contains(at))
        .unwrap_or_else(|| panic!("no card frame around {label:?}"))
}

#[test]
fn every_card_has_the_same_height() {
    let long_task = "word ".repeat(60);
    let long_task = long_task.trim_end();
    let mut long = agent("l", long_task, Status::NeedsYou);
    long.blocked_reason = Some(BlockedReason::Permission);
    long.has_changes = true;
    long.paused = true;
    long.activity = "a very long activity line that keeps going ".repeat(5);
    long.workspaces = vec![ws("core"), ws("portal"), ws("infra")];
    let mut short = agent("s", "three word task", Status::Queued);
    short.activity = String::new();
    let mut h = harness(default_ui_schema(), vec![long, short]);
    h.run();
    let (long, short) = (card_of(&h, long_task), card_of(&h, "three word task"));
    assert_eq!(long.height(), short.height());
    assert_eq!(short.height(), CARD_HEIGHT);
}

#[test]
fn cards_in_every_column_have_the_same_height() {
    let mut stopped = agent("x", "stopped one", Status::Stopped);
    stopped.stop_cause = Some(StopCause::Crashed);
    stopped.disk_bytes = Some(7_600_000_000);
    let agents = vec![
        stopped,
        agent("w", "working one", Status::Working),
        needs_you(),
        agent("r", "review one", Status::Review),
    ];
    let mut h = harness(default_ui_schema(), agents);
    h.run();
    for label in ["stopped one", "working one", "ask me", "review one"] {
        assert_eq!(card_of(&h, label).height(), CARD_HEIGHT, "{label}");
    }
}

fn crashed() -> Agent {
    let mut a = agent("x", "fix login", Status::Stopped);
    a.stop_cause = Some(StopCause::Crashed);
    a.activity = "crashed while running Bash".into();
    a.disk_bytes = Some(7_600_000_000);
    a
}

#[test]
fn a_stopped_card_shows_its_cause_badge_and_size() {
    let h = harness(default_ui_schema(), vec![crashed()]);
    h.get_by_label("Crashed");
    h.get_by_label("7.6 GB");
    h.get_by_label("crashed while running Bash");
    assert!(
        h.query_by_label("Stop").is_none(),
        "a stopped card has no Stop"
    );
    assert!(
        h.query_by_label("Ended").is_none(),
        "the old Ended badge is gone"
    );
}

#[test]
fn the_cause_badge_takes_the_tone_the_schema_gives_its_value() {
    let dark = egui::Visuals::default().dark_mode;
    for (cause, tone) in [
        (StopCause::Stopped, Tone::Caution),
        (StopCause::Exited, Tone::Active),
        (StopCause::Crashed, Tone::Danger),
    ] {
        let mut a = crashed();
        a.stop_cause = Some(cause);
        let mut h = harness(default_ui_schema(), vec![a]);
        h.run();
        assert!(
            painted_rects(&h)
                .iter()
                .any(|(_, f)| *f == theme::tone_tint(tone, dark)),
            "{cause:?} badge is not tinted {tone:?}"
        );
    }
}

#[test]
fn the_stopped_column_is_darker() {
    let mut h = harness(default_ui_schema(), vec![]);
    h.run();
    let dark = egui::Visuals::default().dark_mode;
    let hint = h.get_by_label("Nothing stopped").rect().center();
    let sunken = theme::tokens(dark).sunken;
    assert!(
        painted_rects(&h)
            .iter()
            .any(|(r, f)| *f == sunken && r.contains(hint) && r.height() > 400.0),
        "the Stopped column has no darker body"
    );
    let queued = h.get_by_label("Nothing waiting to start").rect().center();
    assert!(
        !painted_rects(&h)
            .iter()
            .any(|(r, f)| *f == sunken && r.contains(queued)),
        "only the Stopped column is darker"
    );
}

#[test]
fn hovering_a_cut_activity_shows_its_full_text() {
    let mut a = agent("w", "t", Status::Working);
    a.activity = "running a command with a very long description that cannot fit ".repeat(3);
    let activity = a.activity.clone();
    assert!(
        tooltip_shows(&activity, a),
        "no tooltip with the full activity"
    );
}

#[test]
fn a_stopped_card_offers_details_and_a_running_one_does_not() {
    let mut h = harness(
        default_ui_schema(),
        vec![crashed(), agent("w", "busy", Status::Working)],
    );
    h.run();
    assert_eq!(h.query_all_by_label("Details").count(), 1);
    h.get_by_label("Details").click();
    h.run();
    assert_eq!(h.state().commands, [Command::Details("x".into())]);
}

#[test]
fn the_card_is_four_rows_title_state_activity_actions() {
    let mut a = needs_you();
    a.workspaces = vec![ws("core")];
    let h = harness(default_ui_schema(), vec![a]);
    let (title, time) = (h.get_by_label("ask me").rect(), h.get_by_label("5m").rect());
    assert!(
        (title.center().y - time.center().y).abs() < 2.0,
        "title and time share the first row"
    );
    assert!(title.center().x < time.center().x, "title left, time right");
    let (badge, repo) = (y(&h, "Permission needed"), y(&h, "core"));
    assert!((badge - repo).abs() < 2.0, "badge and repo share a row");
    assert!(title.center().y < badge, "the state row is under the title");
    assert!(
        badge < y(&h, "wants to run Bash"),
        "activity is under the state row"
    );
    assert!(
        y(&h, "wants to run Bash") < y(&h, "Stop"),
        "the actions are the last row"
    );
}

#[test]
fn the_card_is_about_three_quarters_of_its_old_height() {
    // Five slots (meta, two title lines, badges, activity, actions) made the old card this tall.
    let old = 139.75;
    assert!(
        (0.70 * old..=0.80 * old).contains(&CARD_HEIGHT),
        "CARD_HEIGHT is {CARD_HEIGHT}, old was {old}"
    );
}

#[test]
fn the_title_is_one_line_cut_with_an_ellipsis() {
    let task = "Add pagination to the shots list endpoint and keep the existing sort order stable across pages";
    let mut h = harness(default_ui_schema(), vec![agent("w", task, Status::Working)]);
    h.run();
    let [galley] = &painted_galleys(&h, task)[..] else {
        panic!("expected the task painted once")
    };
    assert_eq!(galley.rows.len(), 1, "the title takes one line");
    assert!(galley.elided, "a long title is cut");
}

#[test]
fn a_card_shows_one_state_badge_stop_cause_first_then_blocked_then_paused() {
    let mut stopped = crashed();
    stopped.paused = true;
    let h = harness(default_ui_schema(), vec![stopped]);
    h.get_by_label("Crashed");
    assert!(h.query_by_label("Paused").is_none(), "only the cause shows");

    let mut blocked = needs_you();
    blocked.paused = true;
    let h = harness(default_ui_schema(), vec![blocked]);
    h.get_by_label("Permission needed");
    assert!(
        h.query_by_label("Paused").is_none(),
        "only the reason shows"
    );

    let mut paused = agent("w", "on hold", Status::Working);
    paused.paused = true;
    harness(default_ui_schema(), vec![paused]).get_by_label("Paused");
}

#[test]
fn has_changes_is_a_dot_not_a_badge() {
    let mut a = agent("r", "done", Status::Review);
    a.has_changes = true;
    let mut h = harness(default_ui_schema(), vec![a]);
    h.run();
    let dot = h.get_by_label("Has changes").rect();
    assert!(
        dot.width() < 10.0,
        "the changes mark is {}px wide: a badge, not a dot",
        dot.width()
    );
    assert_eq!(circles(&h, 3.0).len(), 1, "one 6 px dot is painted");

    let h = harness(
        default_ui_schema(),
        vec![agent("w", "clean", Status::Working)],
    );
    assert!(h.query_by_label("Has changes").is_none());
}

#[test]
fn the_card_names_the_first_repo_and_counts_the_others() {
    let mut a = agent("w", "two repos", Status::Working);
    a.workspaces = vec![ws("core"), ws("portal"), ws("infra")];
    let mut h = harness(default_ui_schema(), vec![a]);
    h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    h.run();
    h.get_by_label("core");
    assert!(
        h.query_by_label("portal").is_none(),
        "only the first repo is named"
    );
    h.get_by_label("+2").hover();
    h.run();
    assert!(
        h.query_by_label_contains("portal").is_some()
            && h.query_by_label_contains("infra").is_some(),
        "the tooltip lists every repo"
    );
}

#[test]
fn a_long_repo_is_cut_and_the_badge_and_size_keep_their_room() {
    let mut a = crashed();
    let repo = "a-repository-with-a-very-long-name-indeed".to_string();
    a.workspaces = vec![ws(&repo)];
    let mut h = harness(default_ui_schema(), vec![a]);
    h.run();
    let card = card_of(&h, "fix login");
    let badge = h.get_by_label("Crashed").rect();
    let size = h.get_by_label("7.6 GB").rect();
    let repo = h.get_by_label(&repo).rect();
    assert!(
        card.contains_rect(badge),
        "badge {badge:?} outside {card:?}"
    );
    assert!(card.contains_rect(size), "size {size:?} outside {card:?}");
    assert!(repo.right() <= size.left(), "the repo runs under the size");
    assert!(
        badge.right() <= repo.left(),
        "the repo runs under the badge"
    );
}

/// The node `label` as a button, and its rect.
fn button_rect(h: &Harness<'_, S>, label: &str) -> egui::Rect {
    h.get_by_role_and_label(Role::Button, label).rect()
}

#[test]
fn actions_are_small_icon_buttons_at_the_right_of_the_last_row() {
    let h = harness(
        default_ui_schema(),
        vec![agent("w", "working", Status::Working)],
    );
    let card = card_of(&h, "working");
    let (pause, stop) = (button_rect(&h, "Pause"), button_rect(&h, "Stop"));
    for (name, r) in [("Pause", pause), ("Stop", stop)] {
        assert!(r.width() <= 22.0 && r.height() <= 22.0, "{name} is {r:?}");
        assert!(
            r.left() >= card.center().x,
            "{name} is not in the right half"
        );
        assert!(card.contains_rect(r), "{name} {r:?} outside {card:?}");
    }
    assert!(pause.right() < stop.left(), "Pause before Stop");
    assert!(
        card.right() - stop.right() < 14.0,
        "Stop is not right-aligned: {stop:?} in {card:?}"
    );
    assert!(
        h.query_by_label("Resume").is_none(),
        "a running agent offers Pause, not Resume"
    );
}

#[test]
fn icon_buttons_show_their_action_as_a_tooltip() {
    let mut h = harness(
        default_ui_schema(),
        vec![agent("w", "working", Status::Working)],
    );
    h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    h.run();
    h.get_by_role_and_label(Role::Button, "Pause").hover();
    h.run();
    assert_eq!(h.query_all_by_label("Pause").count(), 2, "no tooltip");
}

#[test]
fn a_paused_agent_offers_resume() {
    let mut a = agent("w", "on hold", Status::Working);
    a.paused = true;
    let mut h = harness(default_ui_schema(), vec![a]);
    h.get_by_role_and_label(Role::Button, "Resume").click();
    h.run();
    let [Command::Run { action, .. }] = h.state().commands.as_slice() else {
        panic!("expected one command")
    };
    assert_eq!(action.id, "resume");
}

#[test]
fn stop_needs_a_second_click_on_the_red_icon() {
    let mut h = harness(
        default_ui_schema(),
        vec![agent("w", "working", Status::Working)],
    );
    h.run();
    let dark = egui::Visuals::default().dark_mode;
    let red = theme::tone_tint(Tone::Danger, dark);
    assert!(!painted_rects(&h).iter().any(|(_, f)| *f == red));
    h.get_by_role_and_label(Role::Button, "Stop").click();
    h.run();
    assert!(
        h.state().commands.is_empty(),
        "the first click only asks for confirmation"
    );
    let armed = button_rect(&h, "Click again to stop");
    assert!(
        painted_rects(&h)
            .iter()
            .any(|(r, f)| *f == red && r.contains(armed.center())),
        "the armed Stop is not red"
    );
    h.get_by_role_and_label(Role::Button, "Click again to stop")
        .click();
    h.run();
    let [Command::Run { action, .. }] = h.state().commands.as_slice() else {
        panic!("expected one command")
    };
    assert_eq!(action.id, "stop");
    assert!(
        h.query_by_label("Click again to stop").is_none(),
        "confirmation resets after use"
    );
}

#[test]
fn clicking_elsewhere_disarms_stop() {
    let mut h = harness(
        default_ui_schema(),
        vec![agent("w", "working", Status::Working)],
    );
    h.get_by_role_and_label(Role::Button, "Stop").click();
    h.run();
    h.get_by_label("Click again to stop");
    h.get_by_label("Nothing waiting to start").click();
    h.run();
    assert!(
        h.query_by_label("Click again to stop").is_none(),
        "a click elsewhere leaves Stop armed"
    );
    assert!(h.state().commands.is_empty());
}

#[test]
fn an_armed_stop_disarms_after_a_few_seconds() {
    let mut h = harness(
        default_ui_schema(),
        vec![agent("w", "working", Status::Working)],
    );
    h.input_mut().time = Some(100.0);
    h.run();
    h.get_by_role_and_label(Role::Button, "Stop").click();
    h.run();
    h.get_by_label("Click again to stop");
    h.input_mut().time = Some(110.0);
    h.run();
    assert!(
        h.query_by_label("Click again to stop").is_none(),
        "Stop stays armed for good"
    );
}

#[test]
fn an_action_without_an_icon_is_a_text_button() {
    let mut schema = default_ui_schema();
    let mut extra = schema.actions[1].clone();
    extra.id = "nudge".into();
    extra.label = "Nudge".into();
    schema.actions.push(extra);
    let mut h = harness(schema, vec![agent("w", "working", Status::Working)]);
    h.get_by_role_and_label(Role::Button, "Nudge").click();
    h.run();
    let [Command::Run { action, .. }] = h.state().commands.as_slice() else {
        panic!("expected one command")
    };
    assert_eq!(action.id, "nudge");
}

#[test]
fn the_icon_row_fits_a_narrow_window() {
    let mut paused = agent("p", "paused one", Status::Working);
    paused.paused = true;
    let mut h = harness_wide(
        700.0,
        default_ui_schema(),
        vec![
            agent("w", "working one", Status::Working),
            paused,
            crashed(),
        ],
    );
    h.run();
    for (card, buttons) in [
        ("working one", &["Pause", "Stop"][..]),
        ("paused one", &["Resume", "Stop"]),
        ("fix login", &["Details"]),
    ] {
        let frame = card_of(&h, card);
        for b in buttons {
            let r = h
                .query_all_by_role_and_label(Role::Button, b)
                .map(|n| n.rect())
                .find(|r| frame.contains(r.center()))
                .unwrap_or_else(|| panic!("{card}: no {b} on the card"));
            assert!(frame.contains_rect(r), "{card}: {b} {r:?} cut by {frame:?}");
        }
    }
}

#[test]
fn an_armed_stop_and_its_sibling_fit_a_narrow_window() {
    let mut h = harness_wide(
        700.0,
        default_ui_schema(),
        vec![agent("w", "working one", Status::Working)],
    );
    h.get_by_role_and_label(Role::Button, "Stop").click();
    h.run();
    let frame = card_of(&h, "working one");
    for b in ["Pause", "Click again to stop"] {
        let r = button_rect(&h, b);
        assert!(frame.contains_rect(r), "{b} {r:?} cut by {frame:?}");
    }
}

fn card_harness(a: Agent, persona: Option<&'static str>) -> Harness<'static, CardState> {
    Harness::builder()
        .with_size(Vec2::new(240.0, 200.0))
        .build_ui_state(
            move |ui, st: &mut CardState| {
                card(ui, &default_ui_schema(), &a, NOW, st, false, persona);
            },
            CardState::default(),
        )
}

#[test]
fn the_persona_sits_in_the_left_half_of_the_last_row() {
    let name = "antonio.frontino-with-a-long-persona";
    let h = card_harness(agent("w", "working", Status::Working), Some(name));
    let persona = h.get_by_label(name).rect();
    let stop = h.get_by_role_and_label(Role::Button, "Stop").rect();
    assert!(
        (persona.center().y - stop.center().y).abs() < 2.0,
        "persona and actions share the last row"
    );
    assert!(
        persona.right() <= 120.0,
        "the persona runs past the left half: {persona:?}"
    );
}

/// Where each painted text whose source is `text` may show: its clip rect.
fn text_clips(h: &Harness<'_, S>, text: &str) -> Vec<egui::Rect> {
    fn has(shape: &egui::Shape, text: &str) -> bool {
        match shape {
            egui::Shape::Text(t) => t.galley.job.text == text,
            egui::Shape::Vec(v) => v.iter().any(|s| has(s, text)),
            _ => false,
        }
    }
    h.output()
        .shapes
        .iter()
        .filter(|c| has(&c.shape, text))
        .map(|c| c.clip_rect)
        .collect()
}

#[test]
fn in_a_narrow_card_the_repo_count_never_covers_the_size() {
    // Found in the real window: with the side panel open, "+1" ran over the size ("+16.0 KB").
    let mut a = crashed();
    a.workspaces = vec![ws("core"), ws("portal-frontend")];
    let mut h = harness_wide(760.0, default_ui_schema(), vec![a]);
    h.run();
    let size = h.get_by_label("7.6 GB").rect();
    if let Some(repo) = h.query_by_label("core") {
        assert!(
            repo.rect().right() <= size.left(),
            "the repo runs under the size"
        );
    }
    for clip in text_clips(&h, "+1") {
        assert!(
            clip.right() <= size.left(),
            "+1 may paint up to x={}, the size starts at x={}",
            clip.right(),
            size.left()
        );
    }
}

/// The painted extent of each text whose source is `text`, with the clip rect it is painted in.
fn text_rects(h: &Harness<'_, S>, text: &str) -> Vec<(egui::Rect, egui::Rect)> {
    fn walk(
        shape: &egui::Shape,
        text: &str,
        clip: egui::Rect,
        out: &mut Vec<(egui::Rect, egui::Rect)>,
    ) {
        match shape {
            egui::Shape::Text(t) if t.galley.job.text == text => {
                out.push((egui::Rect::from_min_size(t.pos, t.galley.size()), clip))
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, text, clip, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    h.output()
        .shapes
        .iter()
        .for_each(|c| walk(&c.shape, text, c.clip_rect, &mut out));
    out
}

#[test]
fn a_repo_with_no_room_at_all_is_not_drawn_half_cut() {
    // Found in a 900 px window: a sliver of "…" was left between the badge and the size.
    let mut a = crashed();
    a.stop_cause = Some(StopCause::Stopped);
    a.disk_bytes = Some(5982);
    a.workspaces = vec![ws("core"), ws("portal-frontend")];
    for width in (600..=800).step_by(4) {
        let mut h = harness_wide(width as f32, default_ui_schema(), vec![a.clone()]);
        h.run();
        for (r, clip) in text_rects(&h, "core") {
            assert!(
                r.right() <= clip.right(),
                "{width} px: the repo paints to x={} but shows only up to x={}",
                r.right(),
                clip.right()
            );
        }
    }
}
