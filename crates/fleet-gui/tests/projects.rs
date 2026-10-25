mod common;

use eframe::egui::{self, accesskit::Role, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use fleet_api::{
    default_ui_schema, Agent, AgentId, ArchivePlan, ArchiveReport, Footprint, FootprintItem,
    LogEntry, Orphan, OrphanKind, PlanItem, Project, ProjectList, ProjectState, Snapshot, Status,
    StatusTimes, StopCause, Workspace,
};
use fleet_client::http::FleetClient;
use fleet_client::stream::StreamMsg;
use fleet_gui::app::{AppResult, FleetApp};
use fleet_gui::views::{ProjectFilter, View};
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
    }
}

fn project(id: &str, name: &str, state: ProjectState, cause: Option<StopCause>) -> Project {
    Project {
        id: AgentId(id.into()),
        name: name.into(),
        task: format!("{name} goal\nmore detail"),
        workdir: format!("/p/{name}"),
        repos: vec![ws("core")],
        state,
        stop_cause: cause,
        created_at: Some(NOW - 7200),
        stopped_at: (state != ProjectState::Running).then_some(NOW - 600),
        archived_at: (state == ProjectState::Archived).then_some(NOW - 60),
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
    }
}

fn agent(id: &str, task: &str, status: Status) -> Agent {
    Agent {
        id: AgentId(id.into()),
        task: task.into(),
        workdir: "/p/x".into(),
        workspaces: vec![ws("core")],
        tmux_session: format!("fleet-{id}"),
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
    }
}

/// One running, two stopped (one of them crashed), one archived, and an orphan folder.
fn list() -> ProjectList {
    ProjectList {
        projects: vec![
            project("r1", "alpha", ProjectState::Running, None),
            project(
                "s1",
                "bravo",
                ProjectState::Stopped,
                Some(StopCause::Stopped),
            ),
            project(
                "c1",
                "charlie",
                ProjectState::Stopped,
                Some(StopCause::Crashed),
            ),
            project(
                "a1",
                "delta",
                ProjectState::Archived,
                Some(StopCause::Stopped),
            ),
        ],
        orphans: vec![Orphan {
            id: "project_folder:/p/echo".into(),
            kind: OrphanKind::ProjectFolder,
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
    app.handle_result(AppResult::Projects(Ok(list())));
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

/// The Projects view with `select` selected.
fn projects(select: &str) -> Harness<'static, FleetApp> {
    projects_of(app(), select)
}

fn projects_of(mut a: FleetApp, select: &str) -> Harness<'static, FleetApp> {
    a.open_projects(ProjectFilter::All, Some(select.into()));
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
fn the_drawer_lists_projects_between_kanban_and_settings() {
    let mut h = harness(app());
    h.run();
    let y = |h: &Harness<'static, FleetApp>, label: &str| {
        h.get_by_role_and_label(Role::Button, label)
            .rect()
            .center()
            .y
    };
    let (kanban, projects, settings) = (y(&h, "Kanban"), y(&h, "Projects"), y(&h, "Settings"));
    assert!(
        kanban < projects && projects < settings,
        "{kanban} {projects} {settings}"
    );
    h.get_by_role_and_label(Role::Button, "Projects").click();
    h.run();
    assert_eq!(h.state().view(), View::Projects);
}

#[test]
fn the_list_is_narrower_than_the_passport() {
    let h = projects("r1");
    let (list_rect, passport_rect) = h.state().projects_rects();
    assert!(list_rect.width() > 0.0);
    assert!(
        list_rect.width() < passport_rect.width(),
        "{list_rect:?} {passport_rect:?}"
    );
    assert!(list_rect.right() <= passport_rect.left());
}

#[test]
fn totals_count_crashed_on_their_own() {
    let h = projects("r1");
    h.get_by_label("1 running");
    h.get_by_label("2 stopped");
    h.get_by_label("1 crashed");
    h.get_by_label("1 orphan");
    // The clone cache, measured.
    h.get_by_label("2.5 GB");
}

#[test]
fn the_list_orders_running_then_crashed_then_stopped_then_orphans() {
    let h = projects("r1");
    let y = |label: &str| h.get_by_role_and_label(Role::Button, label).rect().top();
    assert!(y("alpha") < y("charlie"));
    assert!(y("charlie") < y("bravo"));
    assert!(y("bravo") < y("echo"));
}

#[test]
fn archive_is_disabled_until_the_project_is_stopped() {
    let h = projects("r1");
    assert!(
        archive_button_disabled(&h),
        "a running project cannot be archived"
    );
    h.get_by_role_and_label(Role::Button, "Stop");
    drop(h);
    let h = projects("s1");
    assert!(!archive_button_disabled(&h));
    assert!(h.query_by_role_and_label(Role::Button, "Stop").is_none());
}

#[test]
fn the_archive_dialog_warns_about_uncommitted_files_and_focuses_cancel() {
    let mut h = projects("s1");
    h.get_by_role_and_label(Role::Button, "Archive…").click();
    h.run();
    // The plan has not arrived: nothing to confirm yet.
    h.get_by_label("Measuring…");
    assert!(h
        .get_by_role_and_label(Role::Button, "Archive")
        .accesskit_node()
        .is_disabled());
    h.state_mut().handle_result(AppResult::ArchivePlan(
        "s1".into(),
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
fn a_plan_for_another_project_does_not_enable_archive() {
    let mut h = projects("s1");
    h.get_by_role_and_label(Role::Button, "Archive…").click();
    h.run();
    h.state_mut().handle_result(AppResult::ArchivePlan(
        "c1".into(),
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
    let mut h = projects_of(a, "s1");
    h.get_by_role_and_label(Role::Button, "Archive…").click();
    h.run();
    h.state_mut().handle_result(AppResult::ArchivePlan(
        "s1".into(),
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
    let mut h = projects_of(a, "s1");
    h.get_by_role_and_label(Role::Button, "Archive…").click();
    h.run();
    h.state_mut().handle_result(AppResult::ArchivePlan(
        "s1".into(),
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
    let changed = "This project changed since the plan was made. Review it again.";
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
    h.state_mut().handle_result(AppResult::ArchivePlan(
        "s1".into(),
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
fn archived_projects_are_hidden_until_show_archived() {
    let mut h = projects("r1");
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
    let h = projects("r1");
    // Once in the totals and once in the passport.
    assert_eq!(h.get_all_by_label("not tracked yet").count(), 2);
}

#[test]
fn the_passport_shows_the_goal_repos_footprint_and_log() {
    let h = projects("s1");
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
fn a_backfilled_project_shows_its_times_as_unknown() {
    let mut a = app();
    let mut old = project(
        "s1",
        "bravo",
        ProjectState::Stopped,
        Some(StopCause::Exited),
    );
    old.created_at = None;
    old.times = StatusTimes::default();
    a.handle_result(AppResult::Projects(Ok(ProjectList {
        projects: vec![old],
        orphans: vec![],
        clones_bytes: None,
    })));
    let h = projects_of(a, "s1");
    h.get_by_label("started: unknown");
    assert!(h.query_by_label("0s").is_none(), "unknown is not zero");
}

#[test]
fn open_projects_filters_the_list() {
    let mut a = app();
    a.open_projects(ProjectFilter::Crashed, None);
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
fn a_session_orphan_offers_end_session_with_a_warning() {
    let mut a = app();
    let mut l = list();
    l.orphans = vec![Orphan {
        id: "session:fleet-zz".into(),
        kind: OrphanKind::Session,
        path: "fleet-zz".into(),
        worktrees: vec![],
        bytes: None,
    }];
    a.handle_result(AppResult::Projects(Ok(l)));
    let mut h = projects_of(a, "session:fleet-zz");
    assert!(h
        .query_by_role_and_label(Role::Button, "Archive…")
        .is_none());
    h.get_by_role_and_label(Role::Button, "End session…")
        .click();
    h.run();
    h.get_by_label("An agent may be working in this session. End it?");
    assert!(h.get_by_role_and_label(Role::Button, "Cancel").is_focused());
}

#[test]
fn stop_in_the_passport_asks_for_a_second_click() {
    let mut h = projects("r1");
    h.get_by_role_and_label(Role::Button, "Stop").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Confirm Stop");
}

#[test]
fn open_on_board_shows_the_card_in_the_side_panel() {
    let mut h = projects("r1");
    h.get_by_role_and_label(Role::Button, "Open on board")
        .click();
    h.run();
    assert_eq!(h.state().view(), View::Kanban);
    assert_eq!(h.state().preview_agent(), Some("r1"));
}

#[test]
fn a_stopped_projects_diff_shows_in_its_passport() {
    let mut h = projects("s1");
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
    let mut h = projects("r1");
    let (sent, _) = h.state().projects_fetches();
    h.get_by_role_and_label(Role::Button, "Refresh").click();
    h.run();
    assert_eq!(
        h.state().projects_fetches(),
        (sent, 0),
        "a refresh overlapped the fetch in flight"
    );
    // Once that list lands, the queued refresh goes, so its newer list is the last one applied.
    h.state_mut().handle_result(AppResult::Projects(Ok(list())));
    assert_eq!(h.state().projects_fetches(), (sent + 1, 1));
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
        projects: 0,
    }
}

#[test]
fn project_row_shows_persona() {
    let mut a = app();
    let mut l = list();
    // bravo's persona was deleted since; charlie predates personas.
    l.projects[1].persona = "gone".into();
    l.projects[2].persona = String::new();
    a.handle_result(AppResult::Projects(Ok(l)));
    a.handle_result(AppResult::Personas(Ok(fleet_api::PersonaList {
        personas: vec![persona("p", "Personal"), persona("w", "Work")],
        claude_command: vec!["claude".into()],
        gh_command: "gh".into(),
        projects_root: "/projects".into(),
    })));
    let mut h = harness(a);
    h.run();
    // The board's persona filter leaves the Projects view alone.
    h.get_by_role_and_label(Role::ComboBox, "Persona filter")
        .click();
    h.run();
    h.get_by_role_and_label(Role::CheckBox, "Work").click();
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(h.query_by_label("alpha goal").is_none());
    h.get_by_role_and_label(Role::Button, "Projects").click();
    h.run();
    let list = h.state().projects_rects().0;
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
fn project_rows_tell_same_named_personas_apart() {
    let mut a = app();
    let mut l = list();
    l.projects[1].persona = "w".into();
    a.handle_result(AppResult::Projects(Ok(l)));
    a.handle_result(AppResult::Personas(Ok(fleet_api::PersonaList {
        personas: vec![persona("p", "Ann"), persona("w", "Ann")],
        claude_command: vec!["claude".into()],
        gh_command: "gh".into(),
        projects_root: "/projects".into(),
    })));
    let mut h = harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "Projects").click();
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
