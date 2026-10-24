use eframe::egui::{self, accesskit::Role, Vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use fleet_api::{default_ui_schema, Agent, AgentId, BlockedReason, ColumnDef, Status, Tone, UiSchema, Workspace};
use fleet_gui::components::{board, Command, CardState};
use std::collections::HashMap;

const NOW: i64 = 1300;

fn ws(repo: &str) -> Workspace {
    Workspace { repo: repo.into(), branch: "fleet/fix".into(), worktree: format!("/w/{repo}"), base: "abc".into(), has_changes: false }
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
    }
}

#[derive(Default)]
struct S {
    states: HashMap<String, CardState>,
    commands: Vec<Command>,
}

fn harness(schema: UiSchema, agents: Vec<Agent>) -> Harness<'static, S> {
    Harness::builder().with_size(Vec2::new(1400.0, 800.0)).build_ui_state(
        move |ui, s: &mut S| {
            let cmds = board(ui, &schema, &agents, NOW, &mut s.states);
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
            ColumnDef { id: "left".into(), title: "Alpha lane".into(), statuses: vec![Status::Working], empty_hint: "nothing left".into(), tone: Tone::Active },
            ColumnDef { id: "right".into(), title: "Beta lane".into(), statuses: vec![Status::Review], empty_hint: "nothing right".into(), tone: Tone::Done },
        ],
        ..default_ui_schema()
    };
    let h = harness(custom, vec![agent("w", "the working one", Status::Working)]);
    h.get_by_label("Alpha lane");
    h.get_by_label("Beta lane");
    h.get_by_label("the working one");
    h.get_by_label("nothing right");
    assert!(h.query_by_label("Needs you").is_none() && h.query_by_label("Queued").is_none(), "a column the schema lacks was drawn");
}

#[test]
fn each_agent_lands_in_the_column_the_schema_maps_its_status_to() {
    let agents = vec![
        agent("q", "t-q", Status::Queued),
        agent("d", "t-d", Status::Dead),
        agent("r", "t-r", Status::Review),
        needs_you(),
    ];
    let h = harness(default_ui_schema(), agents);
    let (queued, needs, review) = (x(&h, "Queued"), x(&h, "Needs you"), x(&h, "Review"));
    let near = |card: f32, col: f32| (card - col).abs() < 200.0;
    assert!(near(x(&h, "t-q"), queued), "Queued card is not under Queued");
    assert!(near(x(&h, "ask me"), needs), "Needs you card is not under Needs you");
    assert!(near(x(&h, "t-d"), review) && near(x(&h, "t-r"), review), "Dead and Review cards belong in Review");
    h.get_by_label("Agent conversations appear here");
}

#[test]
fn longest_waiting_card_first() {
    let mut newer = agent("new", "newer", Status::Working);
    newer.status_since = 900;
    let mut older = agent("old", "older", Status::Working);
    older.status_since = 100;
    let h = harness(default_ui_schema(), vec![newer, older]);
    assert!(y(&h, "older") < y(&h, "newer"), "the longest-waiting card must come first");
}

#[test]
fn card_shows_project_task_activity_and_time_waiting_for_needs_you() {
    let h = harness(default_ui_schema(), vec![needs_you()]);
    for label in ["core", "ask me", "wants to run Bash", "permission", "5m"] {
        h.get_by_label(label);
    }
}

#[test]
fn card_shows_only_fields_whose_condition_holds() {
    let h = harness(default_ui_schema(), vec![agent("w", "working", Status::Working)]);
    assert!(h.query_by_label("permission").is_none());
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

#[test]
fn card_answer_returns_the_text_unchanged() {
    let mut h = harness(default_ui_schema(), vec![needs_you()]);
    let input = h.get_by_role(Role::TextInput);
    input.click();
    input.type_text("$(x) \"q\" {a} é -n");
    h.run();
    h.get_by_role_and_label(Role::Button, "Answer").click();
    h.run();
    let [Command::Run { action, agent, input }] = h.state().commands.as_slice() else { panic!("{:?}", h.state().commands.len()) };
    assert_eq!((action.id.as_str(), agent.id.0.as_str()), ("answer", "n1"));
    assert_eq!(input.as_deref(), Some("$(x) \"q\" {a} é -n"));
    assert_eq!(h.get_by_role(Role::TextInput).value().unwrap_or_default(), "", "the box clears after sending");
}

#[test]
fn empty_answer_is_disabled() {
    let mut h = harness(default_ui_schema(), vec![needs_you()]);
    h.get_by_role_and_label(Role::Button, "Answer").click();
    h.run();
    assert!(h.state().commands.is_empty());
    let input = h.get_by_role(Role::TextInput);
    input.click();
    input.type_text("   ");
    h.run();
    h.get_by_role_and_label(Role::Button, "Answer").click();
    h.run();
    assert!(h.state().commands.is_empty(), "whitespace is not an answer");
}

#[test]
fn kill_needs_a_second_click() {
    let mut h = harness(default_ui_schema(), vec![agent("w", "working", Status::Working)]);
    h.get_by_label("Kill").click();
    h.run();
    assert!(h.state().commands.is_empty(), "the first click only asks for confirmation");
    h.get_by_label("Confirm Kill").click();
    h.run();
    let [Command::Run { action, .. }] = h.state().commands.as_slice() else { panic!("expected one command") };
    assert_eq!(action.id, "kill");
    assert!(h.query_by_label("Confirm Kill").is_none(), "confirmation resets after use");
}

#[test]
fn plain_actions_run_on_one_click_with_no_input() {
    let mut h = harness(default_ui_schema(), vec![agent("w", "working", Status::Working)]);
    h.get_by_label("Pause").click();
    h.run();
    let [Command::Run { action, input, .. }] = h.state().commands.as_slice() else { panic!("expected one command") };
    assert_eq!((action.id.as_str(), input.is_none()), ("pause", true));
}

#[test]
fn clicking_the_card_body_returns_open() {
    let mut h = harness(default_ui_schema(), vec![agent("w", "open me", Status::Working)]);
    h.get_by_label("open me").click();
    h.run();
    assert_eq!(h.state().commands, [Command::Open("w".into())]);
}

#[test]
fn two_cards_keep_separate_answer_boxes() {
    let mut a = needs_you();
    let mut b = needs_you();
    a.id = AgentId("a".into());
    b.id = AgentId("b".into());
    b.task = "second".into();
    let mut h = harness(default_ui_schema(), vec![a, b]);
    let inputs: Vec<_> = h.get_all_by_role(Role::TextInput).collect();
    assert_eq!(inputs.len(), 2);
    inputs[0].click();
    inputs[0].type_text("only the first");
    h.run();
    let values: Vec<String> = h.get_all_by_role(Role::TextInput).map(|n| n.value().unwrap_or_default()).collect();
    assert_eq!(values.iter().filter(|v| !v.is_empty()).count(), 1, "{values:?}");
    let _ = egui::Id::NULL;
}
