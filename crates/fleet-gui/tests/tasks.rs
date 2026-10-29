mod common;

use eframe::egui::{self, accesskit::Role, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use fleet_api::{
    default_ui_schema, Agent, AgentId, ArchivePlan, ArchiveReport, Footprint, FootprintItem,
    LogEntry, Orphan, OrphanKind, PlanItem, Snapshot, Status, StatusTimes, StopCause, Task,
    TaskList, TaskState, Workspace,
};
use fleet_client::http::FleetClient;
use fleet_client::stream::StreamMsg;
use fleet_gui::app::{AppResult, FleetApp};
use fleet_gui::views::{TaskFilter, View};
use fleet_log::Notice;

const NOW: i64 = 100_000;

fn ws(repo: &str) -> Workspace {
    Workspace {
        repo: repo.into(),
        branch: format!("fleet/{repo}-x"),
        worktree: format!("/p/x/{repo}"),
        base: "abcdef1234".into(),
        from_branch: Some("main".into()),
        has_changes: false,
        owned: false,
        changed: false,
    }
}

fn task(id: &str, name: &str, state: TaskState, cause: Option<StopCause>) -> Task {
    Task {
        id: AgentId(id.into()),
        name: name.into(),
        goal: format!("{name} goal\nmore detail"),
        title: None,
        workdir: format!("/p/{name}"),
        repos: vec![ws("core")],
        state,
        stop_cause: cause,
        created_at: Some(NOW - 7200),
        stopped_at: (state != TaskState::Running).then_some(NOW - 600),
        archived_at: (state == TaskState::Archived).then_some(NOW - 60),
        agents_used: 1,
        times: StatusTimes {
            queued: 5,
            working: 3600,
            needs_you: 120,
            review: 0,
        },
        log: vec![LogEntry {
            at: NOW - 7200,
            text: "started with core".into(),
        }],
        footprint: Some(Footprint {
            measured_at: NOW - 300,
            items: vec![FootprintItem {
                label: "core".into(),
                path: format!("/p/{name}/core"),
                bytes: 8_000_000_000,
                build_bytes: Some(7_600_000_000),
            }],
            total: 8_000_000_000,
        }),
        last_screen: None,
        archive: None,
        persona: "p".into(),
        product: "p".into(),
        release: None,
        breakdowns: vec![],
        gate_runs: vec![],
        prs: vec![],
        done_at: None,
        done_by_hand: false,
        pr_rules: None,
        inbox: vec![],
        kind: fleet_api::AgentKind::Solo,
        lead: None,
        role: None,
        specialists: vec![],
    }
}

fn agent(id: &str, goal: &str, status: Status) -> Agent {
    Agent {
        id: AgentId(id.into()),
        goal: goal.into(),
        title: None,
        workdir: "/p/x".into(),
        workspaces: vec![ws("core")],
        session: format!("fleet-{id}"),
        status,
        activity: "running Bash".into(),
        status_since: NOW - 10,
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
        model: None,
        attach: None,
        mode: None,
        effort: None,
    }
}

/// One running, two stopped (one of them crashed), one archived, and an orphan folder.
fn list() -> TaskList {
    TaskList {
        tasks: vec![
            task("r1", "alpha", TaskState::Running, None),
            task("s1", "bravo", TaskState::Stopped, Some(StopCause::Stopped)),
            task(
                "c1",
                "charlie",
                TaskState::Stopped,
                Some(StopCause::Crashed),
            ),
            task("a1", "delta", TaskState::Archived, Some(StopCause::Stopped)),
        ],
        orphans: vec![Orphan {
            id: "task_folder:/p/echo".into(),
            kind: OrphanKind::TaskFolder,
            path: "/p/echo".into(),
            worktrees: vec!["/p/echo/core".into()],
            bytes: Some(1_000_000),
        }],
        clones_bytes: Some(2_500_000_000),
    }
}

fn app() -> FleetApp {
    let mut app = FleetApp::detached(
        FleetClient::new("http://127.0.0.1:1"),
        egui::Context::default(),
    )
    .without_calls();
    app.set_clock(Box::new(|| NOW));
    app.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    app.handle_result(AppResult::Repos(Ok(vec![])));
    app.handle_stream(StreamMsg::Snapshot(Snapshot {
        seq: 1,
        agents: vec![agent("r1", "alpha goal", Status::Working)],
    }));
    app.handle_result(AppResult::Tasks(Ok(list())));
    app
}

fn logged_app() -> (tracing::subscriber::DefaultGuard, FleetApp) {
    let (guard, handle) = common::log::test_log();
    (guard, app().with_log(handle))
}

fn harness(app: FleetApp) -> Harness<'static, FleetApp> {
    Harness::builder()
        .with_max_steps(60)
        .with_size(Vec2::new(1400.0, 800.0))
        .build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), app)
}

/// The Tasks view with `select` selected.
fn tasks(select: &str) -> Harness<'static, FleetApp> {
    tasks_of(app(), select)
}

fn tasks_of(mut a: FleetApp, select: &str) -> Harness<'static, FleetApp> {
    a.open_tasks(TaskFilter::All, Some(select.into()));
    let mut h = harness(a);
    h.run();
    h
}

fn has_notice(app: &FleetApp, notice: Notice, text: &str) -> bool {
    app.log_entries()
        .iter()
        .any(|e| e.notice() == Some(notice) && e.message == text)
}

fn archive_button_disabled(h: &Harness<'static, FleetApp>) -> bool {
    h.get_by_role_and_label(Role::Button, "Archive…")
        .accesskit_node()
        .is_disabled()
}

#[test]
fn the_drawer_lists_tasks_between_kanban_and_settings() {
    let mut h = harness(app());
    h.run();
    let y = |h: &Harness<'static, FleetApp>, label: &str| {
        h.get_by_role_and_label(Role::Button, label)
            .rect()
            .center()
            .y
    };
    let (kanban, tasks, settings) = (y(&h, "Kanban"), y(&h, "Tasks"), y(&h, "Settings"));
    assert!(
        kanban < tasks && tasks < settings,
        "{kanban} {tasks} {settings}"
    );
    h.get_by_role_and_label(Role::Button, "Tasks").click();
    h.run();
    assert_eq!(h.state().view(), View::Tasks);
}

#[test]
fn the_list_is_narrower_than_the_passport() {
    let h = tasks("r1");
    let (list_rect, passport_rect) = h.state().tasks_rects();
    assert!(list_rect.width() > 0.0);
    assert!(
        list_rect.width() < passport_rect.width(),
        "{list_rect:?} {passport_rect:?}"
    );
    assert!(list_rect.right() <= passport_rect.left());
}

#[test]
fn totals_count_crashed_on_their_own() {
    let h = tasks("r1");
    h.get_by_label("1 running");
    h.get_by_label("2 stopped");
    h.get_by_label("1 crashed");
    h.get_by_label("1 orphan");
    // The clone cache, measured.
    h.get_by_label("2.5 GB");
}

#[test]
fn the_list_orders_running_then_crashed_then_stopped_then_orphans() {
    let h = tasks("r1");
    let y = |label: &str| h.get_by_role_and_label(Role::Button, label).rect().top();
    assert!(y("alpha") < y("charlie"));
    assert!(y("charlie") < y("bravo"));
    assert!(y("bravo") < y("echo"));
}

#[test]
fn archive_is_disabled_until_the_task_is_stopped() {
    let h = tasks("r1");
    assert!(
        archive_button_disabled(&h),
        "a running task cannot be archived"
    );
    h.get_by_role_and_label(Role::Button, "Stop");
    drop(h);
    let h = tasks("s1");
    assert!(!archive_button_disabled(&h));
    assert!(h.query_by_role_and_label(Role::Button, "Stop").is_none());
}

#[test]
fn the_archive_dialog_warns_about_uncommitted_files_and_focuses_cancel() {
    let mut h = tasks("s1");
    h.get_by_role_and_label(Role::Button, "Archive…").click();
    h.run();
    // The plan has not arrived: nothing to confirm yet.
    h.get_by_label("Measuring…");
    assert!(h
        .get_by_role_and_label(Role::Button, "Archive")
        .accesskit_node()
        .is_disabled());
    let seq = h.state().plan_fetches();
    h.state_mut().handle_result(AppResult::ArchivePlan(
        "s1".into(),
        seq,
        Ok(ArchivePlan {
            items: vec![
                PlanItem {
                    path: "/p/bravo/core".into(),
                    bytes: 8_000_000_000,
                    uncommitted_files: 3,
                    stays: false,
                },
                PlanItem {
                    path: "/p/bravo".into(),
                    bytes: 4_000,
                    uncommitted_files: 0,
                    stays: true,
                },
            ],
            total: 8_000_000_000,
        }),
    ));
    h.run();
    h.get_by_label("3 uncommitted files will be lost");
    // The folder's own contents stay: shown as staying, and not part of what is freed.
    h.get_by_label("stays");
    h.get_by_label("Frees 8.0 GB");
    assert!(!h
        .get_by_role_and_label(Role::Button, "Archive")
        .accesskit_node()
        .is_disabled());
    assert!(
        h.get_by_role_and_label(Role::Button, "Cancel").is_focused(),
        "the safe choice must be the default"
    );
}

#[test]
fn a_plan_for_another_task_does_not_enable_archive() {
    let mut h = tasks("s1");
    h.get_by_role_and_label(Role::Button, "Archive…").click();
    h.run();
    let seq = h.state().plan_fetches();
    h.state_mut().handle_result(AppResult::ArchivePlan(
        "c1".into(),
        seq,
        Ok(ArchivePlan {
            items: vec![],
            total: 0,
        }),
    ));
    h.run();
    h.get_by_label("Measuring…");
    assert!(h
        .get_by_role_and_label(Role::Button, "Archive")
        .accesskit_node()
        .is_disabled());
}

#[test]
fn a_finished_archive_is_announced_with_what_it_freed_or_kept() {
    let (_log, a) = logged_app();
    let mut h = tasks_of(a, "s1");
    h.get_by_role_and_label(Role::Button, "Archive…").click();
    h.run();
    let seq = h.state().plan_fetches();
    h.state_mut().handle_result(AppResult::ArchivePlan(
        "s1".into(),
        seq,
        Ok(ArchivePlan {
            items: vec![],
            total: 0,
        }),
    ));
    h.run();
    h.get_by_role_and_label(Role::Button, "Archive").click();
    h.run();
    h.state_mut().handle_result(AppResult::Archived(
        "s1".into(),
        Ok(ArchiveReport {
            removed: vec!["/p/bravo/core".into()],
            freed: 8_000_000_000,
            kept: None,
        }),
    ));
    h.state_mut().handle_result(AppResult::Archived(
        "c1".into(),
        Ok(ArchiveReport {
            removed: vec![],
            freed: 0,
            kept: Some(
                "kept /p/charlie: it contains notes.txt; kept /p/charlie/x: it is a symbolic link"
                    .into(),
            ),
        }),
    ));
    h.run();
    assert!(has_notice(
        h.state(),
        Notice::Success,
        "bravo archived, 8.0 GB freed"
    ));
    assert!(has_notice(
        h.state(),
        Notice::Success,
        "charlie archived, kept /p/charlie"
    ));
    assert!(
        h.query_by_label("Measuring…").is_none(),
        "the dialog closes"
    );
}

fn one_worktree_plan(uncommitted: u32) -> ArchivePlan {
    ArchivePlan {
        items: vec![PlanItem {
            path: "/p/bravo/core".into(),
            bytes: 1_000,
            uncommitted_files: uncommitted,
            stays: false,
        }],
        total: 1_000,
    }
}

#[test]
fn a_plan_that_changed_since_it_was_shown_is_measured_again() {
    let (_log, a) = logged_app();
    let mut h = tasks_of(a, "s1");
    h.get_by_role_and_label(Role::Button, "Archive…").click();
    h.run();
    let seq = h.state().plan_fetches();
    h.state_mut().handle_result(AppResult::ArchivePlan(
        "s1".into(),
        seq,
        Ok(one_worktree_plan(0)),
    ));
    h.run();
    h.get_by_role_and_label(Role::Button, "Archive").click();
    h.run();
    h.state_mut().handle_result(AppResult::Archived(
        "s1".into(),
        Err(fleet_api::PLAN_CHANGED.into()),
    ));
    h.run();
    let changed = "This task changed since the plan was made. Review it again.";
    h.get_by_label(changed);
    // The stale plan is gone and nothing can be confirmed until the fresh one is on screen.
    h.get_by_label("Measuring…");
    assert!(h
        .get_by_role_and_label(Role::Button, "Archive")
        .accesskit_node()
        .is_disabled());
    assert!(
        !h.state()
            .log_entries()
            .iter()
            .any(|e| e.notice() == Some(Notice::Error)),
        "a changed plan is not a failure"
    );
    let seq = h.state().plan_fetches();
    h.state_mut().handle_result(AppResult::ArchivePlan(
        "s1".into(),
        seq,
        Ok(one_worktree_plan(2)),
    ));
    h.run();
    h.get_by_label("2 uncommitted files will be lost");
    h.get_by_label(changed);
    assert!(!h
        .get_by_role_and_label(Role::Button, "Archive")
        .accesskit_node()
        .is_disabled());
}

#[test]
fn archived_tasks_are_hidden_until_show_archived() {
    let mut h = tasks("r1");
    assert!(h.query_by_role_and_label(Role::Button, "delta").is_none());
    h.get_by_label("Show archived").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "delta").click();
    h.run();
    // Archived: read-only, no way back to a board it left.
    assert!(h
        .query_by_role_and_label(Role::Button, "Open on board")
        .is_none());
    assert!(h
        .query_by_role_and_label(Role::Button, "Archive…")
        .is_none());
}

#[test]
fn tokens_and_cost_say_not_tracked_yet() {
    let h = tasks("r1");
    // Once in the totals and once in the passport.
    assert_eq!(h.get_all_by_label("not tracked yet").count(), 2);
}

#[test]
fn the_passport_shows_the_goal_repos_footprint_and_log() {
    let h = tasks("s1");
    h.get_by_label("bravo goal");
    h.get_by_label("fleet/core-x");
    h.get_by_label_contains("abcdef1");
    h.get_by_label("of which target/");
    h.get_by_label("7.6 GB");
    h.get_by_label("measured 5m ago");
    h.get_by_label("started with core");
    h.get_by_label("not saved");
}

#[test]
fn a_backfilled_task_shows_its_times_as_unknown() {
    let mut a = app();
    let mut old = task("s1", "bravo", TaskState::Stopped, Some(StopCause::Exited));
    old.created_at = None;
    old.times = StatusTimes::default();
    a.handle_result(AppResult::Tasks(Ok(TaskList {
        tasks: vec![old],
        orphans: vec![],
        clones_bytes: None,
    })));
    let h = tasks_of(a, "s1");
    h.get_by_label("started: unknown");
    assert!(h.query_by_label("0s").is_none(), "unknown is not zero");
}

#[test]
fn open_tasks_filters_the_list() {
    let mut a = app();
    a.open_tasks(TaskFilter::Crashed, None);
    let mut h = harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "charlie");
    assert!(h.query_by_role_and_label(Role::Button, "bravo").is_none());
    assert!(h.query_by_role_and_label(Role::Button, "alpha").is_none());
    h.get_by_role_and_label(Role::Button, "Show all").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "bravo");
}

#[test]
fn stop_in_the_passport_asks_for_a_second_click() {
    let mut h = tasks("r1");
    h.get_by_role_and_label(Role::Button, "Stop").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Confirm Stop");
}

#[test]
fn open_on_board_shows_the_card_in_the_side_panel() {
    let mut h = tasks("r1");
    h.get_by_role_and_label(Role::Button, "Open on board")
        .click();
    h.run();
    assert_eq!(h.state().view(), View::Kanban);
    assert_eq!(h.state().preview_agent(), Some("r1"));
}

#[test]
fn a_stopped_tasks_diff_shows_in_its_passport() {
    let mut h = tasks("s1");
    h.state_mut().handle_result(AppResult::Diff(
        "s1".into(),
        Ok(fleet_api::DiffReport {
            workspaces: vec![fleet_api::WorkspaceDiff {
                repo: "core".into(),
                diff: "+added line".into(),
                truncated: false,
            }],
        }),
    ));
    h.run();
    h.get_by_label("+added line");
}

#[test]
fn refresh_waits_for_a_fetch_in_flight() {
    // The view's first frame polls; that fetch is still out when Refresh is clicked.
    let mut h = tasks("r1");
    let (sent, _) = h.state().tasks_fetches();
    h.get_by_role_and_label(Role::Button, "Refresh").click();
    h.run();
    assert_eq!(
        h.state().tasks_fetches(),
        (sent, 0),
        "a refresh overlapped the fetch in flight"
    );
    // Once that list lands, the queued refresh goes, so its newer list is the last one applied.
    h.state_mut().handle_result(AppResult::Tasks(Ok(list())));
    assert_eq!(h.state().tasks_fetches(), (sent + 1, 1));
}

fn persona(id: &str, name: &str) -> fleet_api::Persona {
    fleet_api::Persona {
        id: id.into(),
        name: name.into(),
        host: "github.com".into(),
        commit_name: "Ann Example".into(),
        commit_email: "ann@example.invalid".into(),
        github_auth: fleet_api::GithubAuth::Gh,
        claude_dir: format!("/personas/{id}/claude"),
        gh_dir: format!("/personas/{id}/gh"),
        ready: true,
        claude: None,
        github: None,
        tasks: 0,
        nicknames: Default::default(),
    }
}

#[test]
fn task_row_shows_persona() {
    let mut a = app();
    let mut l = list();
    // bravo's persona was deleted since; charlie predates personas.
    l.tasks[1].persona = "gone".into();
    l.tasks[2].persona = String::new();
    a.handle_result(AppResult::Tasks(Ok(l)));
    a.handle_result(AppResult::Personas(Ok(fleet_api::PersonaList {
        personas: vec![persona("p", "Personal"), persona("w", "Work")],
        claude_command: vec!["claude".into()],
        gh_command: "gh".into(),
        tasks_root: "/tasks".into(),
        claude_path: None,
        gh_path: None,
    })));
    let mut h = harness(a);
    h.run();
    // The board's persona filter leaves the Tasks view alone.
    h.get_by_role_and_label(Role::ComboBox, "Persona filter")
        .click();
    h.run();
    h.get_by_role_and_label(Role::CheckBox, "Work").click();
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(h.query_by_label("alpha goal").is_none());
    h.get_by_role_and_label(Role::Button, "Tasks").click();
    h.run();
    let list = h.state().tasks_rects().0;
    for (row, persona) in [
        ("alpha", "Personal"),
        ("bravo", "gone"),
        ("charlie", "no persona"),
    ] {
        let row = h.get_by_role_and_label(Role::Button, row).rect();
        let tag = h.get_by_label(persona).rect();
        assert!(
            list.contains_rect(tag) && row.contains_rect(tag),
            "{persona} at {tag:?} is not in its row {row:?}"
        );
    }
}

#[test]
fn task_rows_tell_same_named_personas_apart() {
    let mut a = app();
    let mut l = list();
    l.tasks[1].persona = "w".into();
    a.handle_result(AppResult::Tasks(Ok(l)));
    a.handle_result(AppResult::Personas(Ok(fleet_api::PersonaList {
        personas: vec![persona("p", "Ann"), persona("w", "Ann")],
        claude_command: vec!["claude".into()],
        gh_command: "gh".into(),
        tasks_root: "/tasks".into(),
        claude_path: None,
        gh_path: None,
    })));
    let mut h = harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "Tasks").click();
    h.run();
    for (row, persona) in [("alpha", "Ann (p)"), ("bravo", "Ann (w)")] {
        let row = h.get_by_role_and_label(Role::Button, row).rect();
        // charlie is Ann (p) too: one of the matches is the row's.
        assert!(
            h.query_all_by_label(persona)
                .any(|n| row.contains_rect(n.rect())),
            "{persona} is not in its row"
        );
    }
}

#[test]
fn cancel_is_disabled_while_the_archive_runs() {
    let mut h = tasks("s1");
    h.get_by_role_and_label(Role::Button, "Archive…").click();
    h.run();
    let seq = h.state().plan_fetches();
    h.state_mut().handle_result(AppResult::ArchivePlan(
        "s1".into(),
        seq,
        Ok(one_worktree_plan(0)),
    ));
    h.run();
    assert!(!h
        .get_by_role_and_label(Role::Button, "Cancel")
        .accesskit_node()
        .is_disabled());
    h.get_by_role_and_label(Role::Button, "Archive").click();
    h.run();
    let cancel = h.get_by_role_and_label(Role::Button, "Cancel");
    assert!(cancel.accesskit_node().is_disabled());
    cancel.click();
    h.run();
    h.get_by_label("Archiving…");
}

#[test]
fn every_open_of_the_archive_dialog_asks_for_a_fresh_plan() {
    let mut h = tasks("s1");
    assert_eq!(h.state().plan_fetches(), 0);
    for opened in 1..=3 {
        h.get_by_role_and_label(Role::Button, "Archive…").click();
        h.run();
        assert_eq!(h.state().plan_fetches(), opened);
        // Its answer never comes: the next open must not wait for it.
        h.get_by_role_and_label(Role::Button, "Cancel").click();
        h.run();
    }
}

#[test]
fn a_late_plan_from_an_earlier_open_is_not_shown_as_the_fresh_one() {
    let mut h = tasks("s1");
    h.get_by_role_and_label(Role::Button, "Archive…").click();
    h.run();
    let first = h.state().plan_fetches();
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Archive…").click();
    h.run();
    let second = h.state().plan_fetches();
    // The first open's request was slow and answers only now, with what the task held back then.
    h.state_mut().handle_result(AppResult::ArchivePlan(
        "s1".into(),
        first,
        Ok(one_worktree_plan(5)),
    ));
    h.run();
    h.get_by_label("Measuring…");
    assert!(h
        .get_by_role_and_label(Role::Button, "Archive")
        .accesskit_node()
        .is_disabled());
    h.state_mut().handle_result(AppResult::ArchivePlan(
        "s1".into(),
        second,
        Ok(one_worktree_plan(2)),
    ));
    h.run();
    h.get_by_label("2 uncommitted files will be lost");
}

#[test]
fn a_late_plan_from_before_a_changed_plan_is_not_shown_as_the_new_one() {
    let mut h = tasks("s1");
    h.get_by_role_and_label(Role::Button, "Archive…").click();
    h.run();
    let seq = h.state().plan_fetches();
    h.state_mut().handle_result(AppResult::ArchivePlan(
        "s1".into(),
        seq,
        Ok(one_worktree_plan(0)),
    ));
    h.run();
    h.get_by_role_and_label(Role::Button, "Archive").click();
    h.run();
    h.state_mut().handle_result(AppResult::Archived(
        "s1".into(),
        Err(fleet_api::PLAN_CHANGED.into()),
    ));
    h.run();
    // A duplicate of the first answer arriving after the re-measure was asked for is stale too.
    h.state_mut().handle_result(AppResult::ArchivePlan(
        "s1".into(),
        seq,
        Ok(one_worktree_plan(0)),
    ));
    h.run();
    h.get_by_label("Measuring…");
    let fresh = h.state().plan_fetches();
    assert_ne!(fresh, seq);
    h.state_mut().handle_result(AppResult::ArchivePlan(
        "s1".into(),
        fresh,
        Ok(one_worktree_plan(4)),
    ));
    h.run();
    h.get_by_label("4 uncommitted files will be lost");
}

// ---- outcomes, products and releases ----

fn repo_info(name: &str, url: Option<&str>) -> fleet_api::RepoInfo {
    fleet_api::RepoInfo {
        product: "p".into(),
        name: name.into(),
        path: Some(format!("/r/{name}")),
        url: url.map(String::from),
        workflow: fleet_api::WorkflowState::Missing {
            base: "main".into(),
        },
    }
}

fn release(id: i64, version: &str) -> fleet_api::Release {
    fleet_api::Release {
        id,
        product: "p".into(),
        version: version.into(),
        start: "2026-10-01".into(),
        target: "2026-11-01".into(),
        released_at: None,
    }
}

fn product(
    id: &str,
    repos: Vec<fleet_api::RepoInfo>,
    releases: Vec<fleet_api::Release>,
) -> fleet_api::Product {
    fleet_api::Product {
        id: id.into(),
        name: id.into(),
        default_persona: None,
        internal_url: None,
        repos,
        releases,
        tasks: 0,
        overrides: vec![],
        hidden_specialists: vec![],
    }
}

fn with_products(mut a: FleetApp, list: Vec<fleet_api::Product>) -> FleetApp {
    a.handle_result(AppResult::Products(Ok(fleet_api::ProductList {
        products: list,
    })));
    a
}

fn pr(repo: &str, state: fleet_api::PrState) -> fleet_api::TaskPr {
    fleet_api::TaskPr {
        url: format!("https://git.example/{repo}/pull/1"),
        repo: repo.into(),
        state,
        merged_at: None,
    }
}

fn app_with(tasks: Vec<Task>, active: &str) -> FleetApp {
    let mut a = with_products(
        app().with_product(active),
        vec![
            product(
                "p",
                vec![repo_info("core", None)],
                vec![release(1, "1.0"), release(2, "2.0")],
            ),
            product("q", vec![], vec![]),
        ],
    );
    a.handle_result(AppResult::Tasks(Ok(TaskList {
        tasks,
        orphans: list().orphans,
        clones_bytes: None,
    })));
    a
}

#[test]
fn lists_only_active_products_tasks() {
    let mut other = task("o1", "zulu", TaskState::Stopped, Some(StopCause::Stopped));
    other.product = "q".into();
    let mut h = tasks_of(
        app_with(
            vec![task("r1", "alpha", TaskState::Running, None), other],
            "p",
        ),
        "r1",
    );
    h.get_by_role_and_label(Role::Button, "alpha");
    assert!(h.query_by_role_and_label(Role::Button, "zulu").is_none());
    // The totals count the product's tasks only.
    h.get_by_label("1 running");
    h.get_by_label("0 stopped");
    h.run();
}

#[test]
fn orphans_show_for_any_product() {
    let h = tasks_of(
        app_with(vec![task("r1", "alpha", TaskState::Running, None)], "q"),
        "r1",
    );
    assert!(h.query_by_role_and_label(Role::Button, "alpha").is_none());
    h.get_by_role_and_label(Role::Button, "echo");
}

#[test]
fn merged_filter() {
    let mut merged = task("m1", "mike", TaskState::Archived, Some(StopCause::Stopped));
    merged.repos[0].changed = true;
    merged.prs = vec![pr("core", fleet_api::PrState::Merged)];
    let open = task("s1", "bravo", TaskState::Stopped, Some(StopCause::Stopped));
    let mut a = app_with(vec![merged, open], "p");
    a.open_tasks(TaskFilter::Merged, None);
    let mut h = harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "mike");
    assert!(h.query_by_role_and_label(Role::Button, "bravo").is_none());
    assert!(h.query_by_role_and_label(Role::Button, "echo").is_none());
    assert!(!h.query_all_by_label("Merged").next().is_none());
}

#[test]
fn dropped_after_archive_shows_dropped() {
    let archived = task("a1", "delta", TaskState::Archived, Some(StopCause::Stopped));
    let mut a = app_with(vec![archived], "p");
    a.open_tasks(TaskFilter::Dropped, None);
    let mut h = harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "delta");
    assert!(h.query_all_by_label("Dropped").next().is_some());
}

#[test]
fn an_orchestrated_row_shows_its_steps() {
    let step = |n: u32, state: fleet_api::StepState| fleet_api::Step {
        n,
        title: format!("step {n}"),
        specialist: "rust".into(),
        brief: "do it".into(),
        depends_on: vec![],
        release_note: false,
        state,
        agent: None,
        branch: None,
        report: None,
        mode: None,
        effort: None,
    };
    let mut t = task("o1", "oscar", TaskState::Running, None);
    t.kind = fleet_api::AgentKind::Orchestrator;
    t.breakdowns = vec![fleet_api::BreakdownVersion {
        version: 1,
        steps: vec![
            step(1, fleet_api::StepState::Done),
            step(2, fleet_api::StepState::Done),
            step(3, fleet_api::StepState::Pending),
        ],
        approval: fleet_api::Approval::Approved,
        comment: None,
        proposed_at: 1,
        decided_at: Some(2),
    }];
    let h = tasks_of(app_with(vec![t], "p"), "o1");
    assert!(h.query_all_by_label("2/3 steps").next().is_some());
}

#[test]
fn mark_done_only_for_path_only_tasks() {
    let mut a = with_products(
        app(),
        vec![product(
            "p",
            vec![
                repo_info("core", None),
                repo_info("web", Some("https://git.example/web")),
            ],
            vec![],
        )],
    );
    let mut with_url = task(
        "u1",
        "uniform",
        TaskState::Stopped,
        Some(StopCause::Stopped),
    );
    with_url.repos = vec![ws("web")];
    let mut done = task("d1", "delta2", TaskState::Stopped, Some(StopCause::Stopped));
    done.done_by_hand = true;
    let archived = task("a1", "delta", TaskState::Archived, Some(StopCause::Stopped));
    a.handle_result(AppResult::Tasks(Ok(TaskList {
        tasks: vec![
            task("s1", "bravo", TaskState::Stopped, Some(StopCause::Stopped)),
            with_url,
            done,
            archived,
        ],
        orphans: vec![],
        clones_bytes: None,
    })));
    let mut h = tasks_of(a.with_product("p"), "s1");
    h.get_by_role_and_label(Role::Button, "Mark done");
    for (id, filter) in [
        ("u1", TaskFilter::All),
        ("d1", TaskFilter::All),
        ("a1", TaskFilter::Dropped),
    ] {
        h.state_mut().open_tasks(filter, Some(id.into()));
        h.run();
        assert!(
            h.query_by_role_and_label(Role::Button, "Mark done")
                .is_none(),
            "{id}"
        );
    }
    h.state_mut().open_tasks(TaskFilter::All, Some("s1".into()));
    h.run();
    h.get_by_role_and_label(Role::Button, "Mark done").click();
    h.run();
    assert_eq!(
        h.state().task_calls(),
        [fleet_gui::app::TaskCall::Done("s1".into())]
    );
}

#[test]
fn moving_release_calls_client() {
    let mut h = tasks_of(
        app_with(
            vec![task(
                "s1",
                "bravo",
                TaskState::Stopped,
                Some(StopCause::Stopped),
            )],
            "p",
        ),
        "s1",
    );
    h.get_by_role_and_label(Role::ComboBox, "Release").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "2.0").click();
    h.run();
    assert_eq!(
        h.state().task_calls(),
        [fleet_gui::app::TaskCall::Release("s1".into(), Some(2))]
    );
}

#[test]
fn a_finished_task_cannot_change_release() {
    let mut t = task("s1", "bravo", TaskState::Stopped, Some(StopCause::Stopped));
    t.done_by_hand = true;
    let h = tasks_of(app_with(vec![t], "p"), "s1");
    assert!(h
        .get_by_role_and_label(Role::ComboBox, "Release")
        .accesskit_node()
        .is_disabled());
}

#[test]
fn an_open_pr_disables_archive_with_its_reason() {
    let mut t = task("s1", "bravo", TaskState::Stopped, Some(StopCause::Stopped));
    t.prs = vec![pr("core", fleet_api::PrState::Open)];
    let mut h = tasks_of(app_with(vec![t], "p"), "s1");
    assert!(archive_button_disabled(&h));
    h.get_by_label("https://git.example/core/pull/1");
    assert!(
        h.query_all_by_label("Open").count() >= 2,
        "badge and PR state"
    );
    h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    h.get_by_role_and_label(Role::Button, "Archive…").hover();
    h.run();
    h.get_by_label(
        "https://git.example/core/pull/1 is still open; merge or close it before archiving",
    );
}

#[test]
fn mark_done_is_hidden_before_products_load() {
    // The saved product is trusted before the list arrives: its repos are unknown, not URL-less.
    let a = app().with_product("p");
    let h = tasks_of(a, "s1");
    assert!(h
        .query_by_role_and_label(Role::Button, "Mark done")
        .is_none());
}

#[test]
fn a_released_release_is_not_offered() {
    let mut old = release(3, "0.9");
    old.released_at = Some(NOW - 1000);
    let mut a = with_products(
        app().with_product("p"),
        vec![product("p", vec![], vec![release(1, "1.0"), old])],
    );
    a.handle_result(AppResult::Tasks(Ok(list())));
    let mut h = tasks_of(a, "s1");
    h.get_by_role_and_label(Role::ComboBox, "Release").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "1.0");
    assert!(h
        .query_by_role_and_label(Role::Button, "0.9 (released)")
        .is_none());
    assert!(h.query_by_role_and_label(Role::Button, "0.9").is_none());
}

#[test]
fn the_passport_sections_are_mono_labels() {
    let h = tasks("s1");
    h.get_by_label("Goal");
    h.get_by_label("Repos");
    let mut texts = Vec::new();
    fn walk(s: &egui::Shape, out: &mut Vec<(String, egui::FontFamily)>) {
        match s {
            egui::Shape::Text(t) => out.push((
                t.galley.job.text.clone(),
                t.galley.job.sections[0].format.font_id.family.clone(),
            )),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    h.output()
        .shapes
        .iter()
        .for_each(|s| walk(&s.shape, &mut texts));
    for want in ["GOAL", "REPOS"] {
        assert!(
            texts
                .iter()
                .any(|(t, f)| t == want && *f == egui::FontFamily::Monospace),
            "{want} not painted in mono"
        );
    }
}

#[test]
fn the_stat_captions_are_mono_uppercase_and_keep_their_labels() {
    let h = tasks("s1");
    h.get_by_label("Agents started");
    assert!(common::paints_mono(&h, "AGENTS STARTED"));
}
