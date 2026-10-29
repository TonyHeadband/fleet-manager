use fleet_api::{
    Agent, AgentId, EventKind, GithubAuth, Status, StatusTimes, StopCause, TaskState, Workspace,
};
use fleet_daemon::store::{
    leftovers, url_host, PersonaRow, ProductRow, ReleaseRow, RepoRow, Store, StoreError,
};
use std::path::{Path, PathBuf};

fn agent(id: &str) -> Agent {
    Agent {
        id: AgentId(id.into()),
        goal: "t".into(),
        workdir: format!("/w/{id}"),
        workspaces: vec![Workspace {
            repo: "agent-core".into(),
            branch: format!("fleet/{id}"),
            worktree: format!("/w/{id}/agent-core"),
            base: "abc123".into(),
            from_branch: None,
            has_changes: false,
            owned: false,
            changed: false,
        }],
        session: format!("fleet-{id}"),
        status: Status::Queued,
        activity: "".into(),
        status_since: 100,
        has_changes: false,
        paused: false,
        owner: "antonio".into(),
        blocked_reason: None,
        repo_request: None,
        stop_cause: None,
        disk_bytes: None,
        persona: "p".into(),
        product: "prod".into(),
        kind: fleet_api::AgentKind::Solo,
        task: None,
        role: None,
        step: None,
        model: None,
        attach: None,
    }
}

async fn mem() -> Store {
    Store::open(":memory:".as_ref()).await.unwrap()
}

#[tokio::test]
async fn seq_is_monotonic_across_writes() {
    let s = mem().await;
    let e1 = s.insert_agent(&agent("a1"), "t1").await.unwrap();
    let e2 = s.insert_agent(&agent("a2"), "t2").await.unwrap();
    let e3 = s
        .update_agent(&AgentId("a1".into()), |a| a.status = Status::Working)
        .await
        .unwrap()
        .remove(0);
    let e4 = s.remove_agent(&AgentId("a2".into())).await.unwrap();
    assert_eq!([e1.seq, e2.seq, e3.seq, e4.seq], [1, 2, 3, 4]);
    assert!(matches!(e1.kind, EventKind::AgentCreated { .. }));
    assert!(matches!(
        e3.kind,
        EventKind::StatusChanged {
            status: Status::Working,
            ..
        }
    ));
    assert!(matches!(e4.kind, EventKind::AgentRemoved { .. }));
}

#[tokio::test]
async fn unchanged_update_emits_no_event() {
    let s = mem().await;
    s.insert_agent(&agent("a1"), "t1").await.unwrap();
    let before = s.snapshot().await.unwrap().seq;
    assert!(s
        .update_agent(&AgentId("a1".into()), |_| {})
        .await
        .unwrap()
        .is_empty());
    assert_eq!(s.snapshot().await.unwrap().seq, before);
}

#[tokio::test]
async fn blocked_reason_change_is_a_visible_event() {
    let s = mem().await;
    s.insert_agent(&agent("a1"), "t").await.unwrap();
    let ev = s
        .update_agent(&AgentId("a1".into()), |a| {
            a.blocked_reason = Some(fleet_api::BlockedReason::Permission)
        })
        .await
        .unwrap()
        .remove(0);
    assert!(matches!(
        ev.kind,
        EventKind::StatusChanged {
            blocked_reason: Some(fleet_api::BlockedReason::Permission),
            ..
        }
    ));
}

#[tokio::test]
async fn a_workspace_flip_emits_workspaces_changed_and_any_change_status_changed() {
    let s = mem().await;
    s.insert_agent(&agent("a1"), "t").await.unwrap();
    let id = AgentId("a1".into());
    let evs = s
        .update_agent(&id, |a| {
            a.workspaces[0].has_changes = true;
            a.has_changes = true;
        })
        .await
        .unwrap();
    let kinds: Vec<&str> = evs
        .iter()
        .map(|e| match &e.kind {
            EventKind::StatusChanged { .. } => "status",
            EventKind::WorkspacesChanged { .. } => "workspaces",
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(kinds, ["status", "workspaces"]);
    assert_eq!(evs[0].seq + 1, evs[1].seq);

    // A second repo flipping while the agent already has changes touches only the workspaces.
    let mut second = None;
    s.update_agent(&id, |a| {
        a.workspaces.push(Workspace {
            repo: "portal".into(),
            branch: "b".into(),
            worktree: "/w".into(),
            base: "x".into(),
            from_branch: None,
            has_changes: false,
            owned: false,
            changed: false,
        });
    })
    .await
    .unwrap();
    let evs = s
        .update_agent(&id, |a| {
            a.workspaces[1].has_changes = true;
            second = Some(());
        })
        .await
        .unwrap();
    assert!(second.is_some());
    assert!(
        matches!(evs.as_slice(), [e] if matches!(e.kind, EventKind::WorkspacesChanged { .. })),
        "{evs:?}"
    );
}

#[tokio::test]
async fn update_of_unknown_agent_is_an_error() {
    let s = mem().await;
    assert!(s
        .update_agent(&AgentId("nope".into()), |_| {})
        .await
        .is_err());
}

#[tokio::test]
async fn events_since_returns_gap_free_tail() {
    let s = mem().await;
    s.insert_agent(&agent("a1"), "t1").await.unwrap();
    s.insert_agent(&agent("a2"), "t2").await.unwrap();
    s.update_agent(&AgentId("a1".into()), |a| a.paused = true)
        .await
        .unwrap();
    s.remove_agent(&AgentId("a2".into())).await.unwrap();
    let tail = s.events_since(2).await.unwrap().unwrap();
    assert_eq!(tail.iter().map(|e| e.seq).collect::<Vec<_>>(), [3, 4]);
    assert!(s.events_since(4).await.unwrap().unwrap().is_empty());
    assert_eq!(s.events_since(0).await.unwrap().unwrap().len(), 4);
}

#[tokio::test]
async fn events_since_old_seq_returns_none_after_trim() {
    let s = Store::open_with(":memory:".as_ref(), 3).await.unwrap();
    for i in 1..=6 {
        s.insert_agent(&agent(&format!("a{i}")), "t").await.unwrap();
    }
    assert!(s.events_since(1).await.unwrap().is_none());
    assert!(s.events_since(2).await.unwrap().is_none());
    let tail = s.events_since(3).await.unwrap().unwrap();
    assert_eq!(tail.iter().map(|e| e.seq).collect::<Vec<_>>(), [4, 5, 6]);
}

#[tokio::test]
async fn events_since_a_future_seq_returns_none() {
    let s = mem().await;
    s.insert_agent(&agent("a1"), "t").await.unwrap();
    assert!(s.events_since(99).await.unwrap().is_none());
}

#[tokio::test]
async fn snapshot_seq_matches_last_event_and_lists_agents_in_order() {
    let s = mem().await;
    assert_eq!(s.snapshot().await.unwrap().seq, 0);
    s.insert_agent(&agent("a1"), "t1").await.unwrap();
    let last = s.insert_agent(&agent("a2"), "t2").await.unwrap();
    let snap = s.snapshot().await.unwrap();
    assert_eq!(snap.seq, last.seq);
    assert_eq!(
        snap.agents
            .iter()
            .map(|a| a.id.0.as_str())
            .collect::<Vec<_>>(),
        ["a1", "a2"]
    );
}

#[tokio::test]
async fn tokens_are_stored_and_removed_with_the_agent() {
    let s = mem().await;
    s.insert_agent(&agent("a1"), "secret").await.unwrap();
    assert_eq!(
        s.agent_token(&AgentId("a1".into()))
            .await
            .unwrap()
            .as_deref(),
        Some("secret")
    );
    assert_eq!(s.agent_token(&AgentId("zz".into())).await.unwrap(), None);
    s.remove_agent(&AgentId("a1".into())).await.unwrap();
    assert_eq!(s.agent_token(&AgentId("a1".into())).await.unwrap(), None);
}

#[tokio::test]
async fn state_survives_reopening_the_database_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fleet.db");
    {
        let s = Store::open(&path).await.unwrap();
        s.insert_agent(&agent("a1"), "t1").await.unwrap();
    }
    let s = Store::open(&path).await.unwrap();
    let snap = s.snapshot().await.unwrap();
    assert_eq!((snap.seq, snap.agents.len()), (1, 1));
    let next = s.insert_agent(&agent("a2"), "t2").await.unwrap();
    assert_eq!(next.seq, 2);
}

#[tokio::test]
async fn subscribers_receive_committed_events_in_order_and_nothing_for_noops() {
    let s = mem().await;
    let mut rx = s.subscribe();
    s.insert_agent(&agent("a1"), "t").await.unwrap();
    s.update_agent(&AgentId("a1".into()), |_| {}).await.unwrap();
    s.update_agent(&AgentId("a1".into()), |a| a.status = Status::Working)
        .await
        .unwrap();
    s.remove_agent(&AgentId("a1".into())).await.unwrap();
    let got: Vec<u64> = (0..3).map(|_| rx.try_recv().unwrap().seq).collect();
    assert_eq!(got, [1, 2, 3]);
    assert!(rx.try_recv().is_err(), "no event for the no-op update");
}

#[tokio::test]
async fn a_failed_write_broadcasts_nothing() {
    let s = mem().await;
    let mut rx = s.subscribe();
    s.insert_agent(&agent("a1"), "t").await.unwrap();
    assert!(
        s.insert_agent(&agent("a1"), "t").await.is_err(),
        "duplicate id"
    );
    assert_eq!(rx.try_recv().unwrap().seq, 1);
    assert!(rx.try_recv().is_err());
}

fn agent_at(id: &str, status: Status, since: i64) -> Agent {
    Agent {
        status,
        status_since: since,
        ..agent(id)
    }
}

#[tokio::test]
async fn inserting_an_agent_creates_its_task_record() {
    let s = mem().await;
    s.insert_agent(&agent_at("a1", Status::Queued, 100), "t")
        .await
        .unwrap();
    let p = s.task(&AgentId("a1".into())).await.unwrap().unwrap();
    assert_eq!(
        (p.state, p.created_at, p.agents_used),
        (TaskState::Running, Some(100), 1)
    );
    assert_eq!(p.name, "a1");
    assert_eq!(p.log[0].text, "started with agent-core");
}

#[tokio::test]
async fn status_time_accumulates_per_status() {
    let s = mem().await;
    let id = AgentId("a1".into());
    s.insert_agent(&agent_at("a1", Status::Queued, 100), "t")
        .await
        .unwrap();
    for (status, at) in [
        (Status::Working, 130),
        (Status::NeedsYou, 200),
        (Status::Working, 260),
    ] {
        s.update_agent(&id, |a| {
            a.status = status;
            a.status_since = at;
        })
        .await
        .unwrap();
    }
    let p = s.task(&id).await.unwrap().unwrap();
    assert_eq!(
        p.times,
        StatusTimes {
            queued: 30,
            working: 70,
            needs_you: 60,
            review: 0
        }
    );
}

#[tokio::test]
async fn stopping_marks_the_task_and_logs_the_cause() {
    let s = mem().await;
    let id = AgentId("a1".into());
    s.insert_agent(&agent_at("a1", Status::Working, 100), "t")
        .await
        .unwrap();
    s.update_agent(&id, |a| {
        a.status = Status::Stopped;
        a.stop_cause = Some(StopCause::Crashed);
        a.activity = "crashed while editing".into();
        a.status_since = 150;
    })
    .await
    .unwrap();
    let p = s.task(&id).await.unwrap().unwrap();
    assert_eq!(
        (p.state, p.stop_cause, p.stopped_at),
        (TaskState::Stopped, Some(StopCause::Crashed), Some(150))
    );
    assert_eq!(p.times.working, 50);
    assert_eq!(p.log.last().unwrap().text, "crashed while editing");
}

#[tokio::test]
async fn workspace_changes_reach_the_task_repos() {
    let s = mem().await;
    let id = AgentId("a1".into());
    s.insert_agent(&agent("a1"), "t").await.unwrap();
    s.update_agent(&id, |a| {
        let mut w = a.workspaces[0].clone();
        w.repo = "other".into();
        a.workspaces.push(w);
    })
    .await
    .unwrap();
    assert_eq!(s.task(&id).await.unwrap().unwrap().repos.len(), 2);
}

#[tokio::test]
async fn removing_the_agent_keeps_the_task() {
    let s = mem().await;
    let id = AgentId("a1".into());
    s.insert_agent(&agent("a1"), "t").await.unwrap();
    s.remove_agent(&id).await.unwrap();
    assert!(s.task(&id).await.unwrap().is_some());
    assert_eq!(s.tasks().await.unwrap().len(), 1);
}

#[tokio::test]
async fn task_log_and_update_edit_the_record() {
    let s = mem().await;
    let id = AgentId("a1".into());
    s.insert_agent(&agent("a1"), "t").await.unwrap();
    s.task_log(&id, 5, "repo x added").await.unwrap();
    s.update_task(&id, |p| p.agents_used = 2).await.unwrap();
    let p = s.task(&id).await.unwrap().unwrap();
    assert_eq!((p.agents_used, p.log.last().unwrap().at), (2, 5));
    assert!(s.task_log(&AgentId("nope".into()), 1, "x").await.is_err());
}

#[tokio::test]
async fn backfill_creates_one_record_per_agent_without_one() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("fleet.db");
    let s = Store::open(&db).await.unwrap();
    s.insert_agent(&agent("a1"), "t").await.unwrap();
    s.insert_agent(&agent_at("a2", Status::Stopped, 120), "t")
        .await
        .unwrap();
    s.insert_agent(&agent("a3"), "t").await.unwrap();
    // Rows from before tasks existed have an agent and no task.
    let raw =
        sqlx::SqlitePool::connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&db))
            .await
            .unwrap();
    sqlx::query("DELETE FROM tasks WHERE id IN ('a2', 'a3')")
        .execute(&raw)
        .await
        .unwrap();
    assert_eq!(s.backfill_tasks().await.unwrap(), 2);
    assert_eq!(s.backfill_tasks().await.unwrap(), 0);
    let p = s.task(&AgentId("a2".into())).await.unwrap().unwrap();
    assert_eq!((p.created_at, p.state), (None, TaskState::Stopped));
    assert_eq!(p.log[0].text, "record created on upgrade");
}

fn persona_row(id: &str) -> PersonaRow {
    PersonaRow {
        id: id.into(),
        name: id.into(),
        host: "github.com".into(),
        commit_name: "Antonio".into(),
        commit_email: "a@example.com".into(),
        github_auth: GithubAuth::Gh,
        ready: false,
        created_at: 1,
    }
}

#[tokio::test]
async fn persona_rows_round_trip() {
    let s = mem().await;
    let mut p = persona_row("work");
    s.insert_persona(&p).await.unwrap();
    assert_eq!(s.persona("work").await.unwrap(), Some(p.clone()));
    assert_eq!(s.persona("nope").await.unwrap(), None);
    assert!(s.insert_persona(&p).await.is_err(), "duplicate id");
    p.name = "Work".into();
    p.github_auth = GithubAuth::Pat;
    s.update_persona(&p).await.unwrap();
    assert_eq!(s.persona("work").await.unwrap(), Some(p.clone()));
    s.set_persona_ready("work").await.unwrap();
    assert!(s.persona("work").await.unwrap().unwrap().ready);
    s.insert_persona(&persona_row("home")).await.unwrap();
    assert_eq!(s.personas().await.unwrap().len(), 2);
    s.delete_persona("work").await.unwrap();
    assert_eq!(s.persona("work").await.unwrap(), None);
    assert_eq!(s.personas().await.unwrap().len(), 1);
}

#[tokio::test]
async fn task_count_skips_archived() {
    let s = mem().await;
    for id in ["a1", "a2", "a3"] {
        s.insert_agent(&agent(id), "t").await.unwrap();
    }
    s.update_agent(&AgentId("a2".into()), |a| {
        a.status = Status::Stopped;
        a.stop_cause = Some(StopCause::Exited);
    })
    .await
    .unwrap();
    s.archive_task(&AgentId("a3".into()), |p| {
        p.state = TaskState::Archived;
    })
    .await
    .unwrap();
    assert_eq!(s.persona_task_count("p").await.unwrap(), 2);
    assert_eq!(s.persona_task_count("other").await.unwrap(), 0);
}

#[tokio::test]
async fn a_failed_archive_is_rolled_back_before_the_next_write() {
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::{ConnectOptions, Executor};
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("fleet.db");
    let s = Store::open(&db).await.unwrap();
    let id = AgentId("a1".into());
    s.insert_agent(&agent("a1"), "t").await.unwrap();
    let mut conn = SqliteConnectOptions::new()
        .filename(&db)
        .connect()
        .await
        .unwrap();
    conn.execute(
        "CREATE TRIGGER no_archive BEFORE DELETE ON agents BEGIN SELECT RAISE(ABORT, 'disk full'); END",
    )
    .await
    .unwrap();

    // The race needs the connection's worker thread to lag, so keep every core busy.
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let spinners: Vec<_> = (0..std::thread::available_parallelism().map_or(4, |n| n.get()))
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    std::hint::spin_loop();
                }
            })
        })
        .collect();
    let before = s.task(&id).await.unwrap().unwrap().log.len();
    for n in 0..200 {
        s.archive_task(&id, |p| p.state = TaskState::Archived)
            .await
            .unwrap_err();
        // The next writer must not meet the failed transaction's lock.
        s.task_log(&id, n, &format!("failed {n}")).await.unwrap();
    }

    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    spinners.into_iter().for_each(|t| t.join().unwrap());
    let p = s.task(&id).await.unwrap().unwrap();
    assert_eq!(p.log.len(), before + 200);
    assert_ne!(p.state, TaskState::Archived);
}

#[test]
fn url_host_takes_only_http_urls() {
    assert_eq!(
        url_host("https://GitHub.com/a/b").as_deref(),
        Some("github.com")
    );
    assert_eq!(
        url_host("http://www.example.com:8080/a").as_deref(),
        Some("example.com")
    );
    assert_eq!(url_host("git@github.com:a/b").as_deref(), None);
    assert_eq!(url_host("ssh://git@github.com/a/b").as_deref(), None);
    assert_eq!(url_host("/some/path"), None);
}

fn fleet_agent(
    id: &str,
    kind: fleet_api::AgentKind,
    task: Option<&str>,
    step: Option<u32>,
) -> Agent {
    Agent {
        kind,
        task: task.map(|p| AgentId(p.into())),
        role: (kind == fleet_api::AgentKind::Specialist).then(|| "rust".to_string()),
        step,
        ..agent(id)
    }
}

async fn fleet() -> (Store, AgentId, AgentId) {
    let s = mem().await;
    let o = fleet_agent("o", fleet_api::AgentKind::Orchestrator, None, None);
    let sp = fleet_agent("s", fleet_api::AgentKind::Specialist, Some("o"), Some(1));
    s.insert_agent(&o, "t1").await.unwrap();
    s.insert_agent(&sp, "t2").await.unwrap();
    (s, o.id, sp.id)
}

fn stop(a: &mut Agent, cause: StopCause, at: i64) {
    a.status = Status::Stopped;
    a.stop_cause = Some(cause);
    a.status_since = at;
}

#[tokio::test]
async fn fleet_agent_joins_task_record() {
    let (s, o, _) = fleet().await;
    assert_eq!(s.tasks().await.unwrap().len(), 1);
    let p = s.task(&o).await.unwrap().unwrap();
    assert_eq!(p.agents_used, 2);
    assert!(p.log.iter().any(|l| l.text == "rust started for step 1"));
    assert_eq!(s.task_agents(&o).await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_restarted_orchestrator_becomes_the_lead() {
    let (s, o, _) = fleet().await;
    let o2 = fleet_agent("o2", fleet_api::AgentKind::Orchestrator, Some("o"), None);
    s.insert_agent(&o2, "t3").await.unwrap();
    let p = s.task(&o).await.unwrap().unwrap();
    assert_eq!((p.lead, p.agents_used), (Some(o2.id), 3));
    assert_eq!(p.log.last().unwrap().text, "orchestrator restarted");
}

#[tokio::test]
async fn task_stops_with_last_agent() {
    let (s, o, sp) = fleet().await;
    s.update_agent(&o, |a| stop(a, StopCause::Stopped, 200))
        .await
        .unwrap();
    assert_eq!(s.task(&o).await.unwrap().unwrap().state, TaskState::Running);
    s.update_agent(&sp, |a| stop(a, StopCause::Finished, 300))
        .await
        .unwrap();
    let p = s.task(&o).await.unwrap().unwrap();
    assert_eq!(
        (p.state, p.stop_cause, p.stopped_at),
        (TaskState::Stopped, Some(StopCause::Finished), Some(300))
    );
}

#[tokio::test]
async fn only_lead_counts_time() {
    let (s, o, sp) = fleet().await;
    s.update_agent(&sp, |a| {
        a.status = Status::Working;
        a.status_since = 130;
    })
    .await
    .unwrap();
    s.update_agent(&sp, |a| {
        a.status = Status::Review;
        a.status_since = 200;
    })
    .await
    .unwrap();
    assert_eq!(s.task(&o).await.unwrap().unwrap().times.working, 0);
    s.update_agent(&o, |a| {
        a.status = Status::Working;
        a.status_since = 160;
    })
    .await
    .unwrap();
    let p = s.task(&o).await.unwrap().unwrap();
    assert_eq!(p.times.queued, 60);
}

#[tokio::test]
async fn specialist_transitions_fill_inbox() {
    use fleet_api::FleetEventKind::*;
    let (s, o, sp) = fleet().await;
    for (status, at) in [
        (Status::NeedsYou, 110),
        (Status::Review, 120),
        (Status::Stopped, 130),
    ] {
        s.update_agent(&sp, |a| {
            a.status = status;
            a.status_since = at;
            a.activity = "asks".into();
            if status == Status::Stopped {
                a.stop_cause = Some(StopCause::Finished);
            }
        })
        .await
        .unwrap();
    }
    let queued = s.peek_inbox(&o).await.unwrap();
    // A peek leaves them queued.
    assert_eq!(s.peek_inbox(&o).await.unwrap(), queued);
    let kinds: Vec<_> = queued.iter().map(|e| (e.kind, e.step)).collect();
    assert_eq!(
        kinds,
        [(NeedsYou, 1), (IdleWithoutFinish, 1), (Finished, 1)]
    );
    // The ack drops only what was peeked.
    s.ack_inbox(&o, 2).await.unwrap();
    let rest = s.peek_inbox(&o).await.unwrap();
    assert_eq!(rest, queued[2..]);
    s.ack_inbox(&o, 5).await.unwrap();
    assert!(s.peek_inbox(&o).await.unwrap().is_empty());
}

#[tokio::test]
async fn crash_marks_step_failed() {
    use fleet_api::{Approval, BreakdownVersion, FleetEventKind, Step, StepState};
    let (s, o, sp) = fleet().await;
    s.update_task(&o, |p| {
        p.breakdowns.push(BreakdownVersion {
            version: 1,
            steps: vec![Step {
                n: 1,
                title: "t".into(),
                specialist: "rust".into(),
                brief: "b".into(),
                depends_on: vec![],
                state: StepState::Running,
                agent: Some(AgentId("s".into())),
                branch: None,
                report: None,
            }],
            approval: Approval::Approved,
            comment: None,
            proposed_at: 1,
            decided_at: Some(2),
        })
    })
    .await
    .unwrap();
    s.update_agent(&sp, |a| {
        stop(a, StopCause::Crashed, 150);
        a.activity = "boom".into();
    })
    .await
    .unwrap();
    let p = s.task(&o).await.unwrap().unwrap();
    assert_eq!(p.breakdowns[0].steps[0].state, StepState::Failed);
    assert_eq!(p.inbox.len(), 1);
    assert_eq!(p.inbox[0].kind, FleetEventKind::Failed);
}

#[tokio::test]
async fn crash_marks_step_failed_in_every_version() {
    use fleet_api::{Approval, BreakdownVersion, Step, StepState};
    let (s, o, sp) = fleet().await;
    let running = |n: u32| Step {
        n,
        title: "t".into(),
        specialist: "rust".into(),
        brief: "b".into(),
        depends_on: vec![],
        state: StepState::Running,
        agent: Some(AgentId("s".into())),
        branch: None,
        report: None,
    };
    let version = |version: u32, approval: Approval, steps: Vec<Step>| BreakdownVersion {
        version,
        steps,
        approval,
        comment: None,
        proposed_at: 1,
        decided_at: None,
    };
    s.update_task(&o, |p| {
        p.breakdowns
            .push(version(1, Approval::Approved, vec![running(1)]));
        // v2 carries step 1 and adds step 2, which is Pending and must stay so.
        let mut pending = running(2);
        pending.state = StepState::Pending;
        pending.agent = None;
        p.breakdowns
            .push(version(2, Approval::Proposed, vec![running(1), pending]));
    })
    .await
    .unwrap();
    s.update_agent(&sp, |a| stop(a, StopCause::Crashed, 150))
        .await
        .unwrap();
    let p = s.task(&o).await.unwrap().unwrap();
    assert_eq!(p.breakdowns[0].steps[0].state, StepState::Failed);
    assert_eq!(p.breakdowns[1].steps[0].state, StepState::Failed);
    assert_eq!(p.breakdowns[1].steps[1].state, StepState::Pending);
}

fn product_row(id: &str, name: &str) -> ProductRow {
    ProductRow {
        id: id.into(),
        name: name.into(),
        default_persona: None,
        internal_url: None,
        created_at: 1,
    }
}

fn release_row(product: &str, version: &str) -> ReleaseRow {
    ReleaseRow {
        id: 0,
        product: product.into(),
        version: version.into(),
        start: "2026-11-01".into(),
        target: "2026-12-01".into(),
        released_at: None,
    }
}

fn repo_row(product: &str, name: &str, path: Option<&str>, url: Option<&str>) -> RepoRow {
    RepoRow {
        product: product.into(),
        name: name.into(),
        path: path.map(PathBuf::from),
        url: url.map(str::to_string),
    }
}

async fn user_version(path: &Path) -> i64 {
    let opts = sqlx::sqlite::SqliteConnectOptions::new().filename(path);
    let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();
    let v = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    v
}

/// The schema as SQLite keeps it, one entry per table and index, sorted.
async fn schema(path: &Path) -> Vec<String> {
    let opts = sqlx::sqlite::SqliteConnectOptions::new().filename(path);
    let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();
    let mut rows: Vec<String> = sqlx::query_scalar(
        "SELECT type || ' ' || name || ': ' || COALESCE(sql, '') FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    pool.close().await;
    rows.sort();
    rows
}

/// A database as version 6 left it, holding a persona, a repo, a task and an agent.
async fn v6_database(path: &Path) {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    // One connection: another would prepare 006's ALTER against the schema before 005's.
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    for sql in [
        include_str!("../migrations/001_init.sql"),
        include_str!("../migrations/002_repos.sql"),
        include_str!("../migrations/003_projects.sql"),
        include_str!("../migrations/004_personas.sql"),
        include_str!("../migrations/005_fleet.sql"),
        include_str!("../migrations/006_workflow.sql"),
    ] {
        sqlx::raw_sql(sql).execute(&pool).await.unwrap();
    }
    sqlx::raw_sql("PRAGMA user_version = 6")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO personas (id, name, host, commit_name, commit_email, github_auth, ready, created_at) VALUES ('p', 'p', 'github.com', 'A', 'a@example.com', 'gh', 1, 1)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO repos (persona, name, source, location, base, added_at) VALUES ('p', 'a', 'path', '/r/a', '/r/a', 5)")
        .execute(&pool)
        .await
        .unwrap();
    let a = serde_json::to_string(&agent("a1")).unwrap();
    sqlx::query("INSERT INTO agents (id, token, data) VALUES ('a1', 't', ?)")
        .bind(&a)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO projects (id, data) VALUES ('a1', '{}')") // the table before 007
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

#[tokio::test]
async fn fresh_start_keeps_personas_and_drops_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fleet.db");
    v6_database(&path).await;
    for _ in 0..2 {
        let s = Store::open(&path).await.unwrap();
        assert_eq!(s.persona("p").await.unwrap().unwrap().host, "github.com");
        assert!(s.tasks().await.unwrap().is_empty());
        assert!(s.products().await.unwrap().is_empty());
        assert!(s.snapshot().await.unwrap().agents.is_empty());
        drop(s);
        assert_eq!(user_version(&path).await, 12);
    }
}

#[tokio::test]
async fn a_fresh_database_matches_an_upgraded_one() {
    let dir = tempfile::tempdir().unwrap();
    let upgraded = dir.path().join("upgraded.db");
    v6_database(&upgraded).await;
    drop(Store::open(&upgraded).await.unwrap());
    let fresh = dir.path().join("fresh.db");
    drop(Store::open(&fresh).await.unwrap());
    // Opened again: the first migrations run on every open and must not bring an old table back.
    drop(Store::open(&fresh).await.unwrap());
    assert_eq!(schema(&fresh).await, schema(&upgraded).await);
    assert_eq!(user_version(&fresh).await, 12);
}

#[tokio::test]
async fn only_the_reset_reports_a_fresh_start() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fleet.db");
    v6_database(&path).await;
    assert!(Store::open(&path).await.unwrap().was_reset());
    assert!(!Store::open(&path).await.unwrap().was_reset());
}

#[test]
fn leftovers_lists_what_the_old_rows_left() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let tasks = dir.path().join("tasks");
    std::fs::create_dir_all(state.join("repos/p/core")).unwrap();
    std::fs::create_dir_all(tasks.join("fix-it")).unwrap();
    std::fs::write(tasks.join("notes.txt"), "x").unwrap();
    let mut found = leftovers(&state, &tasks);
    found.sort();
    assert_eq!(
        found,
        vec![
            state.join("repos/p"),
            tasks.join("fix-it"),
            tasks.join("notes.txt")
        ]
    );
    assert!(leftovers(&dir.path().join("none"), &dir.path().join("none")).is_empty());
}

#[tokio::test]
async fn product_names_are_unique() {
    let s = mem().await;
    s.insert_product(&product_row("app", "App")).await.unwrap();
    let err = s
        .insert_product(&product_row("app-2", "App"))
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Conflict(_)), "{err:?}");
    assert_eq!(s.products().await.unwrap().len(), 1);
}

#[tokio::test]
async fn products_round_trip() {
    let s = mem().await;
    let mut p = product_row("app", "App");
    s.insert_product(&p).await.unwrap();
    s.insert_product(&product_row("web", "Web")).await.unwrap();
    p.name = "The app".into();
    p.default_persona = Some("p".into());
    s.update_product(&p).await.unwrap();
    assert_eq!(s.product("app").await.unwrap(), Some(p));
    assert_eq!(s.product("nope").await.unwrap(), None);
    let mut w = s.product("web").await.unwrap().unwrap();
    w.name = "The app".into();
    assert!(matches!(
        s.update_product(&w).await.unwrap_err(),
        StoreError::Conflict(_)
    ));
}

#[tokio::test]
async fn repo_needs_a_path_or_url() {
    let s = mem().await;
    s.insert_product(&product_row("app", "App")).await.unwrap();
    assert!(s
        .upsert_repo(&repo_row("app", "core", None, None))
        .await
        .is_err());
    assert!(s.repos("app").await.unwrap().is_empty());
}

#[tokio::test]
async fn repos_are_per_product() {
    let s = mem().await;
    let a = repo_row("a", "core", Some("/r/a"), None);
    let b = repo_row("b", "core", None, Some("https://github.com/o/core.git"));
    s.upsert_repo(&a).await.unwrap();
    s.upsert_repo(&b).await.unwrap();
    assert_eq!(s.repos("a").await.unwrap(), vec![a.clone()]);
    assert_eq!(s.repo("b", "core").await.unwrap(), Some(b.clone()));
    assert_eq!(s.repo("c", "core").await.unwrap(), None);
    // An upsert adds the URL to the path repo, and keeps it one row.
    let both = repo_row(
        "a",
        "core",
        Some("/r/a"),
        Some("https://github.com/o/core.git"),
    );
    s.upsert_repo(&both).await.unwrap();
    let got = s.repo("a", "core").await.unwrap().unwrap();
    assert_eq!(got.url, both.url);
    s.delete_repo("a", "core").await.unwrap();
    assert!(s.repos("a").await.unwrap().is_empty());
    assert_eq!(s.repos("b").await.unwrap().len(), 1);
}

#[tokio::test]
async fn deleting_a_used_product_is_refused() {
    let s = mem().await;
    s.insert_product(&product_row("prod", "Prod"))
        .await
        .unwrap();
    s.upsert_repo(&repo_row("prod", "core", Some("/r/a"), None))
        .await
        .unwrap();
    s.insert_agent(&agent("a1"), "t").await.unwrap();
    assert_eq!(s.delete_product_if_unused("prod").await.unwrap(), 1);
    assert!(s.product("prod").await.unwrap().is_some());
    assert_eq!(s.repos("prod").await.unwrap().len(), 1);

    s.insert_product(&product_row("idle", "Idle"))
        .await
        .unwrap();
    s.upsert_repo(&repo_row("idle", "core", Some("/r/b"), None))
        .await
        .unwrap();
    s.insert_release(&release_row("idle", "v1")).await.unwrap();
    assert_eq!(s.delete_product_if_unused("idle").await.unwrap(), 0);
    assert!(s.product("idle").await.unwrap().is_none());
    assert!(s.repos("idle").await.unwrap().is_empty());
    assert!(s.releases("idle").await.unwrap().is_empty());
}

#[tokio::test]
async fn releases_round_trip() {
    let s = mem().await;
    let id = s.insert_release(&release_row("prod", "v1")).await.unwrap();
    let mut r = s.release(id).await.unwrap().unwrap();
    assert_eq!((r.id, r.version.as_str()), (id, "v1"));
    r.target = "2027-01-01".into();
    r.released_at = Some(5);
    s.update_release(&r).await.unwrap();
    assert_eq!(s.releases("prod").await.unwrap(), vec![r]);
    assert!(s.insert_release(&release_row("prod", "v1")).await.is_err());
}

#[tokio::test]
async fn deleting_a_release_moves_its_tasks_to_the_backlog() {
    let s = mem().await;
    let v1 = s.insert_release(&release_row("prod", "v1")).await.unwrap();
    let v2 = s.insert_release(&release_row("prod", "v2")).await.unwrap();
    for (id, release) in [("a1", v1), ("a2", v2)] {
        s.insert_agent(&agent(id), "t").await.unwrap();
        s.update_task(&AgentId(id.into()), |t| t.release = Some(release))
            .await
            .unwrap();
    }
    s.delete_release(v1).await.unwrap();
    assert!(s.release(v1).await.unwrap().is_none());
    let release_of = |id: &str| {
        let (s, id) = (&s, AgentId(id.into()));
        async move { s.task(&id).await.unwrap().unwrap().release }
    };
    assert_eq!(release_of("a1").await, None);
    assert_eq!(release_of("a2").await, Some(v2));
}

#[tokio::test]
async fn a_task_record_takes_its_agents_product() {
    let s = mem().await;
    s.insert_agent(&agent("a1"), "t").await.unwrap();
    let t = s.task(&AgentId("a1".into())).await.unwrap().unwrap();
    assert_eq!((t.product.as_str(), t.release), ("prod", None));
}

#[tokio::test]
async fn a_repo_roster_is_dropped_and_nothing_is_hidden_after_the_upgrade() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("fleet.db");
    drop(Store::open(&db).await.unwrap());
    // A version 10 database: repos carry a roster, products hide nothing and name no internal repo.
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", db.display()))
        .await
        .unwrap();
    for sql in [
        "ALTER TABLE products DROP COLUMN internal_url",
        "ALTER TABLE products DROP COLUMN hidden_specialists",
        "ALTER TABLE repos ADD COLUMN roster TEXT NOT NULL DEFAULT '[]'",
        "INSERT INTO products (id, name, default_persona, created_at) VALUES ('a', 'A', NULL, 1)",
        "INSERT INTO repos (product, name, path, url, roster, added_at) VALUES ('a', 'core', '/r/a', NULL, '[\"rust\"]', 1)",
        "PRAGMA user_version = 10",
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
    }
    pool.close().await;
    let s = Store::open(&db).await.unwrap();
    assert_eq!(
        s.repo("a", "core").await.unwrap(),
        Some(repo_row("a", "core", Some("/r/a"), None))
    );
    assert!(s.hidden_specialists("a").await.unwrap().is_empty());
    s.set_hidden_specialists("a", &["docs".into(), "rust".into(), "docs".into()])
        .await
        .unwrap();
    assert_eq!(s.hidden_specialists("a").await.unwrap(), ["docs", "rust"]);
    let err = s.set_hidden_specialists("gone", &[]).await.unwrap_err();
    assert!(matches!(err, StoreError::ProductGone(_)), "{err:?}");
}

#[tokio::test]
async fn stored_agents_from_before_the_rename_still_load() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("fleet.db");
    {
        let store = Store::open(&db).await.unwrap();
        store.insert_agent(&agent("a1"), "tok").await.unwrap();
    }
    // Rewrite the rows the way an older daemon stored them, and rewind the schema version.
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", db.display()))
        .await
        .unwrap();
    for old in [
        "UPDATE agents SET data = replace(data, '\"session\":', '\"tmux_session\":')",
        "UPDATE events SET data = replace(data, '\"session\":', '\"tmux_session\":')",
        // Steps 10 to 12 run again after the rewind: undo them.
        "ALTER TABLE agents DROP COLUMN claude_session",
        "ALTER TABLE products DROP COLUMN hidden_specialists",
        "ALTER TABLE repos ADD COLUMN roster TEXT NOT NULL DEFAULT '[]'",
        "ALTER TABLE products DROP COLUMN internal_url",
    ] {
        sqlx::query(old).execute(&pool).await.unwrap();
    }
    sqlx::query("PRAGMA user_version = 8")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let store = Store::open(&db).await.unwrap();
    let a = store
        .snapshot()
        .await
        .unwrap()
        .agents
        .into_iter()
        .find(|a| a.id.0 == "a1")
        .unwrap();
    assert_eq!(a.session, "fleet-a1");
}

#[tokio::test]
async fn an_agent_from_before_session_ids_has_none_and_a_set_one_is_read_back() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("fleet.db");
    {
        let store = Store::open(&db).await.unwrap();
        store.insert_agent(&agent("a1"), "tok").await.unwrap();
    }
    // A version 9 database: no claude_session column, and step 11 not run yet.
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", db.display()))
        .await
        .unwrap();
    for sql in [
        "ALTER TABLE agents DROP COLUMN claude_session",
        "ALTER TABLE products DROP COLUMN hidden_specialists",
        "ALTER TABLE repos ADD COLUMN roster TEXT NOT NULL DEFAULT '[]'",
        "ALTER TABLE products DROP COLUMN internal_url",
        "PRAGMA user_version = 9",
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
    }
    pool.close().await;
    let store = Store::open(&db).await.unwrap();
    let id = AgentId("a1".into());
    assert_eq!(store.claude_session(&id).await.unwrap(), None);
    store.set_claude_session(&id, "u-1").await.unwrap();
    assert_eq!(
        store.claude_session(&id).await.unwrap().as_deref(),
        Some("u-1")
    );
    assert_eq!(
        store
            .claude_session(&AgentId("nobody".into()))
            .await
            .unwrap(),
        None
    );
}
