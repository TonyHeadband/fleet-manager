use eframe::egui::{self, accesskit::Role, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use fleet_api::{
    default_ui_schema, Agent, AgentId, BlockedReason, ColumnDef, Status, StopCause, Tone, UiSchema,
    Workspace,
};
use fleet_client::config::FleetDisplay;
use fleet_gui::components::{board, card, CardState, Command, CARD_HEIGHT, FLEET_ROW_HEIGHT};
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
        changed: false,
    }
}

fn agent(id: &str, goal: &str, status: Status) -> Agent {
    Agent {
        id: AgentId(id.into()),
        goal: goal.into(),
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
        product: "p".into(),
        kind: fleet_api::AgentKind::Solo,
        task: None,
        role: None,
        step: None,
        attach: None,
    }
}

#[derive(Default)]
struct S {
    agents: Vec<Agent>,
    display: FleetDisplay,
    states: HashMap<String, CardState>,
    commands: Vec<Command>,
    selected: Option<String>,
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
                    &s.agents,
                    NOW,
                    &mut s.states,
                    s.selected.as_deref(),
                    &HashMap::new(),
                    s.display,
                    &HashMap::new(),
                );
                s.commands.extend(cmds);
            },
            S {
                agents,
                ..S::default()
            },
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
fn card_shows_task_goal_activity_and_time_waiting_for_needs_you() {
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
        |goal: &str| harness(default_ui_schema(), vec![agent("w", goal, Status::Working)]);
    let short = board_with("short");
    let long_task = "x".repeat(300);
    let long = board_with(&long_task);

    let goal = long.get_by_label(&long_task).rect();
    let body_row = short.get_by_label("short").rect().height();
    assert!(
        goal.width() <= col_w,
        "task is {}px wide in a {col_w}px column",
        goal.width()
    );
    assert!(
        goal.height() <= 1.5 * body_row,
        "task is {}px tall, one row is {body_row}px",
        goal.height()
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

/// Rects painted as the card edge: 4 px wide, filled.
fn edges(h: &Harness<'_, S>) -> Vec<(egui::Rect, egui::Color32)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(egui::Rect, egui::Color32)>) {
        match shape {
            egui::Shape::Rect(r) if (r.rect.width() - 4.0).abs() < 0.01 => {
                out.push((r.rect, r.fill))
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

#[test]
fn cards_carry_no_status_dot() {
    let mut h = harness(
        default_ui_schema(),
        vec![agent("w", "busy", Status::Working)],
    );
    h.run();
    h.get_by_label("running Bash");
    assert!(circles(&h, 2.5).is_empty(), "the 5 px activity dot is gone");
}

#[test]
fn card_edge_takes_the_column_colour() {
    let schema = default_ui_schema();
    for status in [Status::Stopped, Status::Queued, Status::Working] {
        let mut h = harness(schema.clone(), vec![agent("a", "the card", status)]);
        h.run();
        let tone = fleet_client::schema::column_for_status(&schema, status)
            .unwrap()
            .tone;
        let dark = h.ctx.global_style().visuals.dark_mode;
        let want = theme::tone_color(tone, dark);
        assert!(
            edges(&h).iter().any(|(_, c)| *c == want),
            "{status:?}: no 4 px edge in {want:?}, saw {:?}",
            edges(&h)
        );
    }
}

/// The delay until egui wants the next frame; `Duration::MAX` when nothing asked for one.
fn repaint_delay(h: &Harness<'_, S>) -> Duration {
    h.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay
}

#[test]
fn a_fleet_row_carries_the_status_edge_and_no_dot() {
    let mut h = harness(
        default_ui_schema(),
        vec![
            orchestrator("o", "Fix login", Status::Working),
            specialist("f1", "drop cookie", "rust", "o", Status::NeedsYou),
        ],
    );
    h.state_mut().display = FleetDisplay::Compact;
    h.run();
    let row = h.get_by_label("rust drop cookie, needs you").rect();
    let dark = h.ctx.global_style().visuals.dark_mode;
    let want = theme::tone_color(fleet_api::Tone::Attention, dark);
    assert!(edges(&h).iter().any(|(_, c)| *c == want), "{:?}", edges(&h));
    assert!(
        circles(&h, 3.5).iter().all(|c| !row.contains(*c)),
        "the row's 7 px dot is gone"
    );
}

#[test]
fn working_visor_asks_for_frames_only_while_focused() {
    let mut h = harness(
        default_ui_schema(),
        vec![agent("w", "busy", Status::Working)],
    );
    // `run` ignores delayed repaints, so the delay itself is what tells a sweep from an idle window.
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
        repaint_delay(&h) <= theme::SWEEP_FRAME,
        "the visor schedules no frame: {:?}",
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

fn focus(h: &mut Harness<'_, S>, on: bool) {
    h.input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .focused = Some(on);
}

#[test]
fn needs_you_pulses_only_while_focused() {
    let mut h = harness(default_ui_schema(), vec![needs_you()]);
    h.step();
    assert_eq!(repaint_delay(&h), Duration::MAX, "unfocused stays idle");
    focus(&mut h, true);
    h.step();
    assert!(
        repaint_delay(&h) <= theme::SWEEP_FRAME,
        "{:?}",
        repaint_delay(&h)
    );
    theme::set_animations(&h.ctx, false);
    h.step();
    assert_eq!(repaint_delay(&h), Duration::MAX, "animations off");
    theme::set_animations(&h.ctx, true);
    focus(&mut h, false);
    h.step();
    assert_eq!(repaint_delay(&h), Duration::MAX);
}

/// Every painted shape in paint order, `Shape::Vec`s flattened.
fn flat(h: &Harness<'_, S>) -> Vec<egui::Shape> {
    fn walk(shape: &egui::Shape, out: &mut Vec<egui::Shape>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            other => out.push(other.clone()),
        }
    }
    let mut out = Vec::new();
    h.output()
        .shapes
        .iter()
        .for_each(|s| walk(&s.shape, &mut out));
    out
}

/// The fade meshes lying inside `rect` that have a vertex of `color` at the right edge.
fn fades(h: &Harness<'_, S>, rect: egui::Rect, color: egui::Color32) -> Vec<usize> {
    flat(h)
        .iter()
        .enumerate()
        .filter_map(|(i, s)| match s {
            egui::Shape::Mesh(m)
                if !m.vertices.is_empty()
                    && m.vertices.iter().all(|v| rect.expand(0.5).contains(v.pos))
                    && m.vertices
                        .iter()
                        .any(|v| v.color == color && (v.pos.x - rect.right()).abs() < 0.01) =>
            {
                Some(i)
            }
            _ => None,
        })
        .collect()
}

/// Bracket polylines of `color` whose corner point lies within 3 px (plus a pixel) of a corner of `rect`.
fn brackets(h: &Harness<'_, S>, rect: egui::Rect, color: egui::Color32) -> usize {
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.left_bottom(),
        rect.right_bottom(),
    ];
    flat(h)
        .iter()
        .filter(|s| match s {
            egui::Shape::Path(p)
                if p.points.len() == 3
                    && p.stroke.color == egui::epaint::ColorMode::Solid(color) =>
            {
                corners.iter().any(|c| {
                    (p.points[1].x - c.x).abs() <= 4.0 && (p.points[1].y - c.y).abs() <= 4.0
                })
            }
            _ => false,
        })
        .count()
}

fn selected_harness(a: Agent) -> Harness<'static, S> {
    let id = a.id.0.clone();
    let mut h = harness(default_ui_schema(), vec![a]);
    h.state_mut().selected = Some(id);
    h
}

#[test]
fn a_selected_card_fades_and_carries_brackets() {
    for dark in [true, false] {
        let fill = theme::tokens(dark).fill;
        let mut h = selected_harness(agent("w", "busy", Status::Working));
        h.ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        h.run();
        let rect = card_of(&h, "busy");
        assert!(
            !fades(&h, rect, fill).is_empty(),
            "dark={dark}: no fade mesh"
        );
        assert_eq!(brackets(&h, rect, fill), 4, "dark={dark}");

        let mut plain = harness(
            default_ui_schema(),
            vec![agent("w", "busy", Status::Working)],
        );
        plain.ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        plain.run();
        let rect = card_of(&plain, "busy");
        assert!(fades(&plain, rect, fill).is_empty());
        assert_eq!(brackets(&plain, rect, fill), 0);
    }
}

#[test]
fn a_selected_needs_you_card_keeps_its_pulse_edge() {
    let mut h = selected_harness(needs_you());
    h.run();
    let dark = h.ctx.global_style().visuals.dark_mode;
    let rect = card_of(&h, "ask me");
    let fade = *fades(&h, rect, theme::tokens(dark).fill)
        .first()
        .expect("fade mesh");
    let want = theme::tone_color(fleet_api::Tone::Attention, dark);
    let edge = flat(&h)
        .iter()
        .position(|s| matches!(s, egui::Shape::Rect(r) if (r.rect.width() - 4.0).abs() < 0.01 && r.fill == want))
        .expect("the 4 px edge");
    assert!(edge > fade, "edge {edge} painted before the fade {fade}");
}

#[test]
fn the_persona_tag_is_mono_uppercase() {
    let h = card_harness(agent("w", "working", Status::Working), Some("planner"));
    h.get_by_label("planner");
    let upper = h
        .output()
        .shapes
        .iter()
        .find_map(|s| {
            fn find(s: &egui::Shape) -> Option<std::sync::Arc<egui::Galley>> {
                match s {
                    egui::Shape::Text(t) if t.galley.job.text == "PLANNER" => {
                        Some(t.galley.clone())
                    }
                    egui::Shape::Vec(v) => v.iter().find_map(find),
                    _ => None,
                }
            }
            find(&s.shape)
        })
        .expect("a galley reading PLANNER");
    assert_eq!(
        upper.job.sections[0].format.font_id.family,
        theme::mono(11.0).family
    );
}

#[test]
fn a_selected_fleet_row_is_outlined_in_the_accent() {
    let mut h = harness(
        default_ui_schema(),
        vec![
            orchestrator("o", "Fix login", Status::Working),
            specialist("f1", "drop cookie", "rust", "o", Status::Working),
        ],
    );
    h.state_mut().display = FleetDisplay::Compact;
    h.state_mut().selected = Some("f1".into());
    h.run();
    let dark = h.ctx.global_style().visuals.dark_mode;
    let fill = theme::tokens(dark).fill;
    let dashes = flat(&h)
        .iter()
        .filter(|s| matches!(s, egui::Shape::LineSegment { stroke, .. } if stroke.color == fill))
        .count();
    assert!(dashes > 4, "{dashes}");
}

#[test]
fn paused_card_does_not_sweep() {
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
        "a paused agent must not sweep"
    );
    assert!(!edges(&h).is_empty(), "the edge is still drawn, at rest");
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
    let goal = "x".repeat(300);
    assert!(
        tooltip_shows(&goal, agent("w", &goal, Status::Working)),
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
    let goal = "Add pagination to the shots list endpoint and keep the existing sort order stable across pages";
    let mut h = harness(default_ui_schema(), vec![agent("w", goal, Status::Working)]);
    h.run();
    let [galley] = &painted_galleys(&h, goal)[..] else {
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

#[test]
fn a_cut_persona_shows_one_tooltip_in_its_original_case() {
    let name = "antonio.frontino-with-a-long-persona";
    let mut h = card_harness(agent("w", "working", Status::Working), Some(name));
    h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    h.get_by_label(name).hover();
    h.run();
    // The tag itself plus one tooltip; egui's own elided-text tooltip would be a third.
    let nodes = h.query_all_by_label_contains("").filter(|n| {
        n.accesskit_node()
            .value()
            .is_some_and(|v| v.to_lowercase().starts_with("antonio.frontino"))
            || n.accesskit_node()
                .label()
                .is_some_and(|v| v.to_lowercase().starts_with("antonio.frontino"))
    });
    let texts: Vec<_> = nodes
        .map(|n| {
            n.accesskit_node()
                .value()
                .or_else(|| n.accesskit_node().label())
                .unwrap()
        })
        .collect();
    assert_eq!(texts.len(), 2, "{texts:?}");
    assert!(texts.iter().all(|t| t == name), "{texts:?}");
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

fn orchestrator(id: &str, goal: &str, status: Status) -> Agent {
    let mut a = agent(id, goal, status);
    a.kind = fleet_api::AgentKind::Orchestrator;
    a.role = Some("bug-fix".into());
    a
}

fn specialist(id: &str, goal: &str, role: &str, task: &str, status: Status) -> Agent {
    let mut a = agent(id, goal, status);
    a.kind = fleet_api::AgentKind::Specialist;
    a.task = Some(AgentId(task.into()));
    a.role = Some(role.into());
    if status == Status::NeedsYou {
        a.blocked_reason = Some(BlockedReason::Permission);
    }
    a
}

/// The node labelled `label` sits under the column titled `column`.
fn under(h: &Harness<'_, S>, label: &str, column: &str) -> bool {
    (x(h, label) - x(h, column)).abs() < 200.0
}

/// The count in the header of the column titled `column`.
fn column_count(h: &Harness<'_, S>, column: &str) -> String {
    let title = h.get_by_label(column).rect();
    h.query_all_by_label_contains("")
        .filter(|n| {
            let r = n.rect();
            (r.center().y - title.center().y).abs() < 2.0
                && r.left() > title.right()
                && r.left() - title.right() < 30.0
        })
        // A label's text is its accesskit value.
        .find_map(|n| n.accesskit_node().value())
        .unwrap_or_else(|| panic!("no count beside {column:?}"))
}

#[test]
fn group_sits_in_rolled_up_column() {
    let agents = vec![
        orchestrator("o", "Fix login", Status::Working),
        specialist("f1", "fix redirect", "rust", "o", Status::Working),
        specialist("f2", "drop cookie", "rust", "o", Status::NeedsYou),
        agent("s", "solo work", Status::Working),
    ];
    let h = harness(default_ui_schema(), agents);
    assert!(
        under(&h, "Fix login", "Needs you"),
        "the task sits where its most stuck agent is"
    );
    assert!(
        under(&h, "fix redirect", "Needs you"),
        "a working fleet agent stays with its task"
    );
    assert!(under(&h, "solo work", "Working"));
    h.get_by_label("Fix login, 2 fleet agents");
    assert_eq!(column_count(&h, "Working"), "1", "columns count tasks");
    assert_eq!(column_count(&h, "Needs you"), "1");
}

#[test]
fn stuck_row_first_and_group_forced_open() {
    let mut first = specialist("f1", "fix redirect", "rust", "o", Status::Working);
    first.status_since = 100;
    let mut second = specialist("f2", "drop cookie", "test-writer", "o", Status::Working);
    second.status_since = 200;
    let mut h = harness(
        default_ui_schema(),
        vec![
            orchestrator("o", "Fix login", Status::Working),
            first,
            second,
        ],
    );
    h.run();
    assert!(
        y(&h, "rust fix redirect, working") < y(&h, "test-writer drop cookie, working"),
        "longest in its status first"
    );
    h.get_by_label("▾ 2 fleet").click();
    h.run();
    h.get_by_label("▸ 2 fleet");
    assert!(
        h.query_by_label("drop cookie").is_none(),
        "a collapsed group draws no rows"
    );
    h.get_by_label("2 Working");

    h.state_mut().agents[2].status = Status::NeedsYou;
    h.state_mut().agents[2].blocked_reason = Some(BlockedReason::Permission);
    h.run();
    h.get_by_label("▾ 2 fleet");
    let stuck = "test-writer drop cookie, needs you";
    assert!(
        y(&h, stuck) < y(&h, "rust fix redirect, working"),
        "the stuck row comes first"
    );
    h.get_by_label("drop cookie · Permission needed");
    assert!(under(&h, "Fix login", "Needs you"));
    let (row, task) = (h.get_by_label(stuck).rect(), card_of(&h, "Fix login"));
    assert_eq!(row.height(), FLEET_ROW_HEIGHT);
    assert!(
        row.top() > task.bottom() && row.left() > task.left() && row.right() <= task.right(),
        "the fleet sits under its task card, indented: {row:?} vs {task:?}"
    );
    h.get_by_label(stuck).click();
    h.run();
    assert_eq!(h.state().commands, [Command::Open("f2".into())]);
}

#[test]
fn a_stopped_orchestrator_with_live_fleet_asks_for_a_restart() {
    let mut lead = orchestrator("o", "Fix login", Status::Stopped);
    lead.stop_cause = Some(StopCause::Crashed);
    let h = harness(
        default_ui_schema(),
        vec![
            lead,
            specialist("f1", "fix redirect", "rust", "o", Status::Working),
        ],
    );
    h.get_by_label("Orchestrator stopped · Restart?");
    assert!(under(&h, "Fix login", "Needs you"));
    assert!(h.query_by_label("Crashed").is_none(), "one badge");
}

#[test]
fn an_ended_orchestrator_without_fleet_asks_for_a_restart() {
    let mut lead = orchestrator("o", "Fix login", Status::Stopped);
    lead.stop_cause = Some(StopCause::Exited);
    let mut done = specialist("f1", "fix redirect", "rust", "o", Status::Stopped);
    done.stop_cause = Some(StopCause::Finished);
    let mut h = harness(default_ui_schema(), vec![lead, done]);
    h.run();
    h.get_by_label("Orchestrator stopped · Restart?");
    assert!(under(&h, "Fix login", "Needs you"));
    h.get_by_role_and_label(Role::Button, "Restart orchestrator")
        .click();
    h.run();
    assert_eq!(
        h.state().commands,
        [Command::RestartOrchestrator("o".into())]
    );
}

#[test]
fn a_stopped_task_stays_stopped_and_offers_a_restart() {
    let mut lead = orchestrator("o", "Fix login", Status::Stopped);
    lead.stop_cause = Some(StopCause::Stopped);
    let mut fleet = specialist("f1", "fix redirect", "rust", "o", Status::Stopped);
    fleet.stop_cause = Some(StopCause::Stopped);
    let h = harness(default_ui_schema(), vec![lead, fleet]);
    // The card's cause badge says Stopped too; the column title is the topmost.
    let column = h
        .query_all_by_label("Stopped")
        .map(|n| n.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap();
    assert!((x(&h, "Fix login") - column.center().x).abs() < 200.0);
    assert!(h
        .query_by_label("Orchestrator stopped · Restart?")
        .is_none());
    h.get_by_role_and_label(Role::Button, "Restart orchestrator");
}

#[test]
fn a_task_card_takes_its_groups_tone() {
    let mut h = harness(
        default_ui_schema(),
        vec![
            orchestrator("o", "Fix login", Status::Working),
            specialist("f1", "drop cookie", "rust", "o", Status::NeedsYou),
        ],
    );
    h.state_mut().display = FleetDisplay::Hide;
    h.run();
    assert!(under(&h, "Fix login", "Needs you"));
    assert!(
        circles(&h, 2.5).is_empty(),
        "a task that needs you shows no working dot"
    );
}

#[test]
fn display_switch_modes() {
    let agents = vec![
        orchestrator("o", "Fix login", Status::Working),
        specialist("f1", "fix redirect", "rust", "o", Status::Working),
        specialist("f2", "review fix", "reviewer", "o", Status::Review),
    ];
    let mut h = harness(default_ui_schema(), agents);
    h.run();
    h.get_by_label("rust fix redirect, working");
    h.get_by_label("reviewer review fix, review");
    assert!(h.query_by_label("↳ Fix login").is_none());

    // The app's fonts carry "↳"; the harness's stock ones do not.
    theme::apply(&h.ctx);
    h.state_mut().display = FleetDisplay::Show;
    h.run();
    assert_eq!(
        h.query_all_by_label("↳ Fix login").count(),
        2,
        "every fleet agent is a full card naming its task"
    );
    assert!(h.query_by_label("rust fix redirect, working").is_none());
    h.get_by_label("fix redirect");
    assert_eq!(
        h.query_all_by_role_and_label(Role::Button, "Stop").count(),
        3,
        "full cards carry their actions"
    );

    h.state_mut().display = FleetDisplay::Hide;
    h.run();
    assert!(h.query_by_label("fix redirect").is_none());
    assert!(h.query_by_label("review fix").is_none());
    assert!(
        h.query_by_label("▾ 2 fleet").is_none(),
        "nothing to unfold when the fleet is hidden"
    );
    h.get_by_label("1 Working");
    h.get_by_label("1 Review");
}

#[test]
fn solo_cards_unchanged() {
    let mut h = harness(
        default_ui_schema(),
        vec![
            agent("s", "solo work", Status::Working),
            orchestrator("o", "Fix login", Status::Working),
            specialist("f1", "fix redirect", "rust", "o", Status::Working),
        ],
    );
    h.run();
    assert_eq!(card_of(&h, "solo work").height(), CARD_HEIGHT);
    assert!(
        card_of(&h, "Fix login").height() > CARD_HEIGHT,
        "the task card adds its tally row"
    );
    assert_eq!(
        h.query_all_by_label_contains("fleet").count(),
        2,
        "only the task has a toggle and a group label"
    );
}

#[test]
fn display_choice_is_saved() {
    use fleet_api::Snapshot;
    use fleet_client::http::FleetClient;
    use fleet_client::stream::StreamMsg;
    use fleet_gui::app::{AppResult, FleetApp};

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("gui.toml");
    let mut app = FleetApp::detached(
        FleetClient::new("http://127.0.0.1:1"),
        egui::Context::default(),
    )
    .with_settings(path.clone())
    .without_calls();
    app.set_clock(Box::new(|| NOW));
    app.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    app.handle_result(AppResult::Repos(Ok(vec![])));
    app.handle_stream(StreamMsg::Snapshot(Snapshot {
        seq: 1,
        agents: vec![
            orchestrator("o", "Fix login", Status::Working),
            specialist("f1", "fix redirect", "rust", "o", Status::Working),
        ],
    }));
    let mut h = Harness::builder()
        .with_max_steps(60)
        .with_size(Vec2::new(1400.0, 800.0))
        .build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), app);
    theme::apply(&h.ctx);
    h.run();
    h.get_by_label("rust fix redirect, working");
    h.get_by_role_and_label(Role::Button, "Show").click();
    h.run();
    h.get_by_label("↳ Fix login");
    fleet_client::config::SettingsWriter::shared().flush();
    assert_eq!(
        fleet_client::config::GuiSettings::load(&path).fleet_display,
        FleetDisplay::Show
    );
    let reopened = FleetApp::detached(
        FleetClient::new("http://127.0.0.1:1"),
        egui::Context::default(),
    )
    .with_settings(path);
    assert_eq!(reopened.fleet_display(), FleetDisplay::Show);
}

fn click_at(h: &mut Harness<'_, S>, pos: egui::Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
        h.run();
    }
}

fn opened(h: &Harness<'_, S>) -> bool {
    h.state().commands == [Command::Open("w".into())]
}

#[test]
fn clicking_beside_the_icons_opens_the_panel() {
    let mut h = harness(
        default_ui_schema(),
        vec![agent("w", "open me", Status::Working)],
    );
    h.run();
    let pause = h.get_by_label("Pause").rect();
    let card = card_of(&h, "open me");
    // Both the gap just left of the icons and the persona half of the same row.
    for x in [pause.left() - 14.0, card.left() + 60.0] {
        click_at(&mut h, egui::pos2(x, pause.center().y));
        assert!(opened(&h), "x={x}: {:?}", h.state().commands);
        h.state_mut().commands.clear();
    }
}

#[test]
fn clicking_the_lower_rows_opens_the_panel() {
    let mut h = harness(
        default_ui_schema(),
        vec![agent("w", "open me", Status::Working)],
    );
    h.run();
    let card = card_of(&h, "open me");
    let activity = h.get_by_label("running Bash").rect();
    for pos in [
        egui::pos2(card.right() - 20.0, activity.center().y),
        egui::pos2(card.center().x, card.bottom() - 2.0),
        egui::pos2(card.right() - 3.0, card.center().y),
    ] {
        click_at(&mut h, pos);
        assert!(opened(&h), "{pos:?}: {:?}", h.state().commands);
        h.state_mut().commands.clear();
    }
}

#[test]
fn clicking_a_compact_row_anywhere_opens_the_panel() {
    let mut h = harness(
        default_ui_schema(),
        vec![
            orchestrator("o", "Fix login", Status::Working),
            specialist("w", "drop cookie", "rust", "o", Status::Working),
        ],
    );
    h.state_mut().display = FleetDisplay::Compact;
    h.run();
    let row = h.get_by_label("rust drop cookie, working").rect();
    for pos in [
        egui::pos2(row.right() - 2.0, row.center().y),
        egui::pos2(row.left() + 1.0, row.center().y),
        egui::pos2(row.center().x, row.bottom() - 1.0),
    ] {
        click_at(&mut h, pos);
        assert!(opened(&h), "{pos:?}: {:?}", h.state().commands);
        h.state_mut().commands.clear();
    }
}

#[test]
fn clicking_stop_does_not_open_the_panel() {
    let mut h = harness(
        default_ui_schema(),
        vec![agent("w", "open me", Status::Working)],
    );
    h.run();
    let at = h.get_by_label("Stop").rect().center();
    click_at(&mut h, at);
    assert!(
        !h.state().commands.contains(&Command::Open("w".into())),
        "{:?}",
        h.state().commands
    );
}

fn mono_galleys(h: &Harness<'_, S>, text: &str) -> usize {
    painted_galleys(h, text)
        .iter()
        .filter(|g| {
            g.job
                .sections
                .iter()
                .all(|s| s.format.font_id.family == egui::FontFamily::Monospace)
        })
        .count()
}

#[test]
fn column_titles_are_mono_uppercase_and_keep_their_labels() {
    let h = harness(
        default_ui_schema(),
        vec![needs_you(), {
            let mut b = needs_you();
            b.id = AgentId("n2".into());
            b
        }],
    );
    assert!(mono_galleys(&h, "NEEDS YOU") >= 1);
    assert!(mono_galleys(&h, "QUEUED") >= 1);
    h.get_by_label("Needs you");
    h.get_by_label("Queued");
    assert!(mono_galleys(&h, "02") >= 1, "the count reads 02");
}

#[test]
fn column_headers_carry_ticks() {
    let h = harness(default_ui_schema(), vec![needs_you()]);
    let t = theme::tokens(egui::Visuals::default().dark_mode);
    let title = h.get_by_label("Needs you").rect();
    // The header is COLUMN_HEADER tall and its title is centred in it.
    let bottom = title.center().y + theme::layout::COLUMN_HEADER / 2.0;
    let segs: Vec<[egui::Pos2; 2]> = flat(&h)
        .iter()
        .filter_map(|s| match s {
            egui::Shape::LineSegment { points, stroke }
                if stroke.color == t.dashed && points[0].x == points[1].x =>
            {
                Some(*points)
            }
            _ => None,
        })
        .filter(|p| (p[1].y - (bottom - 1.0)).abs() < 1.0)
        .collect();
    assert!(segs.len() >= 20, "ticks along the headers: {}", segs.len());
    let mut xs: Vec<f32> = segs.iter().map(|p| p[0].x).collect();
    xs.sort_by(f32::total_cmp);
    assert!(xs.windows(2).any(|w| (w[1] - w[0] - 10.0).abs() < 0.01));
    // The bottom pixel row is the border's.
    assert!(segs.iter().all(|p| p[1].y <= bottom - 1.0 + 0.01));
}
