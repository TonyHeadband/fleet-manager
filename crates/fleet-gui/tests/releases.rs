use eframe::egui::{self, accesskit::Role, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use fleet_api::{
    default_ui_schema, parse_date, AgentId, Footprint, LogEntry, NewRelease, PrState, Release,
    ReleaseUpdate, StatusTimes, StopCause, Task, TaskList, TaskPr, TaskState, Workspace,
};
use fleet_client::http::FleetClient;
use fleet_gui::app::{AppResult, FleetApp, ReleaseCall, TaskCall};
use fleet_gui::views::View;

fn noon(date: &str) -> i64 {
    parse_date(date).unwrap() * 86_400 + 43_200
}

fn task(id: &str, state: TaskState, release: Option<i64>) -> Task {
    Task {
        id: AgentId(id.into()),
        name: format!("task-{id}"),
        goal: "goal".into(),
        title: None,
        workdir: format!("/p/{id}"),
        repos: vec![Workspace {
            repo: "core".into(),
            branch: format!("fleet/{id}"),
            worktree: format!("/p/{id}/core"),
            base: "abcdef1234".into(),
            from_branch: Some("main".into()),
            has_changes: false,
            owned: false,
            changed: false,
        }],
        state,
        stop_cause: (state != TaskState::Running).then_some(StopCause::Stopped),
        created_at: Some(1),
        stopped_at: None,
        archived_at: (state == TaskState::Archived).then_some(2),
        agents_used: 1,
        times: StatusTimes::default(),
        log: vec![LogEntry {
            at: 1,
            text: "started".into(),
        }],
        footprint: None::<Footprint>,
        last_screen: None,
        archive: None,
        persona: "p".into(),
        product: "p".into(),
        release,
        breakdowns: vec![],
        gate_runs: vec![],
        prs: vec![],
        done_at: None,
        done_by_hand: false,
        pr_rules: None,
        repo_env: Default::default(),
        inbox: vec![],
        kind: fleet_api::AgentKind::Solo,
        lead: None,
        role: None,
        specialists: vec![],
    }
}

fn merged(id: &str, release: Option<i64>) -> Task {
    let mut t = task(id, TaskState::Archived, release);
    t.repos[0].changed = true;
    t.prs = vec![TaskPr {
        url: "https://git.example/core/pull/1".into(),
        repo: "core".into(),
        state: PrState::Merged,
        merged_at: None,
    }];
    t
}

fn release(id: i64, version: &str, start: &str, target: &str) -> Release {
    Release {
        id,
        product: "p".into(),
        version: version.into(),
        start: start.into(),
        target: target.into(),
        released_at: None,
    }
}

fn products(releases: Vec<Release>) -> AppResult {
    AppResult::Products(Ok(fleet_api::ProductList {
        products: vec![fleet_api::Product {
            id: "p".into(),
            name: "p".into(),
            default_persona: None,
            internal_url: None,
            repos: vec![],
            releases,
            tasks: 0,
            overrides: vec![],
            hidden_specialists: vec![],
        }],
    }))
}

fn releases_view(
    today: &str,
    releases: Vec<Release>,
    tasks: Vec<Task>,
) -> Harness<'static, FleetApp> {
    let mut a = FleetApp::detached(
        FleetClient::new("http://127.0.0.1:1"),
        egui::Context::default(),
    )
    .without_calls()
    .with_product("p");
    let now = noon(today);
    a.set_clock(Box::new(move || now));
    a.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    a.handle_result(AppResult::Repos(Ok(vec![])));
    a.handle_result(products(releases));
    a.handle_result(AppResult::Tasks(Ok(TaskList {
        tasks,
        orphans: vec![],
        clones_bytes: None,
    })));
    a.set_view(View::Releases);
    let mut h = Harness::builder()
        .with_max_steps(60)
        .with_size(Vec2::new(1400.0, 800.0))
        .build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), a);
    h.run();
    h
}

fn type_into(h: &mut Harness<'static, FleetApp>, field: &str, text: &str) {
    h.get_by_role_and_label(Role::TextInput, field).click();
    h.run();
    h.get_by_role_and_label(Role::TextInput, field)
        .type_text(text);
    h.run();
}

#[test]
fn bars_match_release_progress() {
    let h = releases_view(
        "2026-10-11",
        vec![release(1, "1.0", "2026-10-01", "2026-11-01")],
        vec![
            merged("a", Some(1)),
            task("b", TaskState::Archived, Some(1)),
            task("c", TaskState::Stopped, Some(1)),
        ],
    );
    h.get_by_label("1/2 tasks");
    h.get_by_label("11/32 days");
    h.get_by_label("1.0");
    // Its tasks are listed.
    h.get_by_label("task-a");
    h.get_by_label("task-c");
}

#[test]
fn overdue_label_past_target() {
    let h = releases_view(
        "2026-11-06",
        vec![release(1, "1.0", "2026-10-01", "2026-11-01")],
        vec![],
    );
    h.get_by_label("5 days overdue");
}

#[test]
fn bad_dates_show_daemon_error() {
    let mut h = releases_view("2026-10-11", vec![], vec![]);
    h.get_by_role_and_label(Role::Button, "New release").click();
    h.run();
    type_into(&mut h, "Version", "3.0");
    type_into(&mut h, "Target", "soon");
    h.run();
    h.get_by_role_and_label(Role::Button, "Create").click();
    h.run();
    let calls = h.state().release_calls().to_vec();
    assert!(matches!(
        calls.as_slice(),
        [ReleaseCall::Create(p, NewRelease { version, target, .. })]
            if p == "p" && version == "3.0" && target == "soon"
    ));
    h.state_mut().handle_result(AppResult::ReleaseSaved(Err(
        "target is not a date (YYYY-MM-DD)".into(),
    )));
    h.run();
    h.get_by_label("target is not a date (YYYY-MM-DD)");
    // The form stays open for a retry.
    h.get_by_role_and_label(Role::TextInput, "Target");
}

#[test]
fn mark_released_moves_it_down() {
    let mut h = releases_view(
        "2026-10-26",
        vec![
            release(1, "1.0", "2026-10-15", "2026-11-01"),
            release(2, "2.0", "2026-09-01", "2026-10-01"),
        ],
        vec![],
    );
    let y = |h: &Harness<'static, FleetApp>, v: &str| h.get_by_label(v).rect().top();
    assert!(y(&h, "1.0") < y(&h, "2.0"), "newest start first");
    h.query_all_by_role_and_label(Role::Button, "Mark released")
        .next()
        .unwrap()
        .click();
    h.run();
    assert_eq!(
        h.state().release_calls(),
        [ReleaseCall::Update(
            1,
            ReleaseUpdate {
                released: Some(true),
                ..Default::default()
            }
        )]
    );
    let mut done = release(1, "1.0", "2026-10-15", "2026-11-01");
    done.released_at = Some(noon("2026-10-26"));
    h.state_mut().handle_result(products(vec![
        done,
        release(2, "2.0", "2026-09-01", "2026-10-01"),
    ]));
    h.run();
    assert!(
        y(&h, "2.0") < y(&h, "1.0"),
        "released goes after unreleased"
    );
    h.get_by_label("Released 2026-10-26");
    h.get_by_role_and_label(Role::Button, "Unrelease");
}

#[test]
fn delete_is_confirmed_in_the_page() {
    let mut h = releases_view(
        "2026-10-26",
        vec![release(1, "1.0", "2026-10-15", "2026-11-01")],
        vec![],
    );
    h.get_by_role_and_label(Role::Button, "Delete").click();
    h.run();
    h.get_by_label("Delete 1.0? Its tasks go to the backlog");
    assert!(h.state().release_calls().is_empty());
    h.get_by_role_and_label(Role::Button, "Confirm").click();
    h.run();
    assert_eq!(h.state().release_calls(), [ReleaseCall::Delete(1)]);
}

#[test]
fn moving_a_task_and_finished_tasks_are_locked() {
    let mut h = releases_view(
        "2026-10-26",
        vec![
            release(1, "1.0", "2026-10-15", "2026-11-01"),
            release(2, "2.0", "2026-09-01", "2026-10-01"),
        ],
        vec![task("c", TaskState::Stopped, Some(1)), merged("m", Some(2))],
    );
    let combos: Vec<_> = h
        .query_all_by_role_and_label(Role::ComboBox, "Move to…")
        .collect();
    assert_eq!(combos.len(), 2);
    assert!(!combos[0].accesskit_node().is_disabled());
    assert!(combos[1].accesskit_node().is_disabled());
    combos[0].click();
    h.run();
    h.get_by_role_and_label(Role::Button, "2.0").click();
    h.run();
    assert_eq!(
        h.state().task_calls(),
        [TaskCall::Release("c".into(), Some(2))]
    );
}

#[test]
fn no_product_says_pick_one() {
    let mut a = FleetApp::detached(
        FleetClient::new("http://127.0.0.1:1"),
        egui::Context::default(),
    )
    .without_calls();
    a.set_view(View::Releases);
    let mut h = Harness::builder()
        .with_size(Vec2::new(1400.0, 800.0))
        .build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), a);
    h.run();
    // The drawer's product combo says it too.
    assert!(h.query_all_by_label("Pick a product").count() >= 1);
}

#[test]
fn edit_prefills_the_release_not_today() {
    let mut h = releases_view(
        "2026-10-26",
        vec![release(1, "1.0", "2026-10-15", "2026-11-01")],
        vec![],
    );
    h.get_by_role_and_label(Role::Button, "Edit").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Save").click();
    h.run();
    assert_eq!(
        h.state().release_calls(),
        [ReleaseCall::Update(
            1,
            ReleaseUpdate {
                version: Some("1.0".into()),
                start: Some("2026-10-15".into()),
                target: Some("2026-11-01".into()),
                released: None,
            }
        )]
    );
}

#[test]
fn the_form_follows_the_daemons_description() {
    let mut h = releases_view("2026-10-26", vec![], vec![]);
    let mut schema = default_ui_schema();
    let form = schema
        .forms
        .iter_mut()
        .find(|f| f.id == "release_create")
        .unwrap();
    form.title = "New version".into();
    let fleet_api::FormItem::Group { fields } = &mut form.items[0] else {
        panic!("the release form starts with a group");
    };
    fields[0].label = "Number".into();
    h.state_mut().handle_result(AppResult::Schema(Ok(schema)));
    h.get_by_role_and_label(Role::Button, "New release").click();
    h.run();
    h.get_by_label("New version");
    h.get_by_role_and_label(Role::TextInput, "Number");
}

fn refused(field: &str) -> AppResult {
    AppResult::ReleaseSaved(Err(fleet_client::http::FleetError {
        message: "\"soon\" is not a date; use YYYY-MM-DD".into(),
        tool: None,
        field: Some(field.into()),
    }))
}

#[test]
fn a_refusal_lands_under_its_field() {
    let mut h = releases_view("2026-10-11", vec![], vec![]);
    h.get_by_role_and_label(Role::Button, "New release").click();
    h.run();
    h.state_mut().handle_result(refused("target"));
    h.run();
    let msg = h
        .get_by_label("\"soon\" is not a date; use YYYY-MM-DD")
        .rect();
    let target = h.get_by_role_and_label(Role::TextInput, "Target").rect();
    assert!(msg.top() >= target.bottom(), "under the Target input");
    // Form-level text starts at the form's left edge; Target's own column starts after the Start input.
    let start = h.get_by_role_and_label(Role::TextInput, "Start").rect();
    assert!(
        msg.left() >= start.right(),
        "{msg:?} not in Target's column"
    );
}

#[test]
fn a_refusal_for_another_field_stays_at_form_level() {
    let mut h = releases_view("2026-10-11", vec![], vec![]);
    h.get_by_role_and_label(Role::Button, "New release").click();
    h.run();
    h.state_mut().handle_result(refused("owner"));
    h.run();
    let msg = h
        .get_by_label("\"soon\" is not a date; use YYYY-MM-DD")
        .rect();
    let target = h.get_by_role_and_label(Role::TextInput, "Target").rect();
    assert!(msg.top() >= target.bottom());
    assert!(msg.left() < target.left(), "at the form's left edge");
}
