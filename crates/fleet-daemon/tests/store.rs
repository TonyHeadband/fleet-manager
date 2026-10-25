use fleet_api::{
    Agent, AgentId, EventKind, GithubAuth, ProjectState, RepoSource, Status, StatusTimes,
    StopCause, Workspace,
};
use fleet_daemon::store::{url_host, PersonaRow, RepoRow, Store};

fn agent(id: &str) -> Agent {
    Agent {
        id: AgentId(id.into()),
        task: "t".into(),
        workdir: format!("/w/{id}"),
        workspaces: vec![Workspace {
            repo: "agent-core".into(),
            branch: format!("fleet/{id}"),
            worktree: format!("/w/{id}/agent-core"),
            base: "abc123".into(),
            from_branch: None,
            has_changes: false,
            owned: false,
        }],
        tmux_session: format!("fleet-{id}"),
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
        kind: fleet_api::AgentKind::Solo,
        project: None,
        role: None,
        step: None,
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

fn row(name: &str, source: RepoSource, location: &str, base: &str) -> RepoRow {
    RepoRow {
        name: name.into(),
        source,
        location: location.into(),
        base: base.into(),
        persona: None,
        roster: vec![],
        test_command: None,
    }
}

fn seed(name: &str, path: &str) -> fleet_daemon::config::Repo {
    fleet_daemon::config::Repo {
        name: name.into(),
        path: path.into(),
    }
}

#[tokio::test]
async fn insert_then_list_and_lookup_by_name_and_location() {
    let s = mem().await;
    let a = row("a", RepoSource::Path, "/r/a", "/r/a");
    let b = row("b", RepoSource::Url, "git@h:o/b.git", "/state/repos/b");
    s.insert_repo(&a).await.unwrap();
    s.insert_repo(&b).await.unwrap();
    assert_eq!(s.repos().await.unwrap(), vec![a.clone(), b.clone()]);
    assert_eq!(s.repo("", "b").await.unwrap(), Some(b.clone()));
    assert_eq!(s.repo("", "nope").await.unwrap(), None);
    assert_eq!(s.repo_by_location("/r/a").await.unwrap(), Some(a));
    assert_eq!(s.repo_by_location("/r/zzz").await.unwrap(), None);
}

#[tokio::test]
async fn inserting_a_taken_name_fails() {
    let s = mem().await;
    s.insert_repo(&row("a", RepoSource::Path, "/r/a", "/r/a"))
        .await
        .unwrap();
    assert!(s
        .insert_repo(&row("a", RepoSource::Path, "/r/other", "/r/other"))
        .await
        .is_err());
    assert_eq!(s.repos().await.unwrap().len(), 1);
}

#[tokio::test]
async fn seeding_twice_adds_nothing_the_second_time() {
    let s = mem().await;
    let seeds = [seed("a", "/r/a"), seed("b", "/r/b")];
    s.seed_repos(&seeds).await.unwrap();
    let first = s.repos().await.unwrap();
    assert_eq!(first.len(), 2);
    assert!(first
        .iter()
        .all(|r| r.source == RepoSource::Path && r.base.to_str() == Some(&r.location)));
    s.seed_repos(&seeds).await.unwrap();
    assert_eq!(s.repos().await.unwrap(), first);
}

#[tokio::test]
async fn seeding_skips_a_location_already_remembered_under_another_name() {
    let s = mem().await;
    s.insert_repo(&row("mine", RepoSource::Path, "/r/a", "/r/a"))
        .await
        .unwrap();
    s.seed_repos(&[seed("a", "/r/a")]).await.unwrap();
    let all = s.repos().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "mine");
}

fn agent_at(id: &str, status: Status, since: i64) -> Agent {
    Agent {
        status,
        status_since: since,
        ..agent(id)
    }
}

#[tokio::test]
async fn inserting_an_agent_creates_its_project_record() {
    let s = mem().await;
    s.insert_agent(&agent_at("a1", Status::Queued, 100), "t")
        .await
        .unwrap();
    let p = s.project(&AgentId("a1".into())).await.unwrap().unwrap();
    assert_eq!(
        (p.state, p.created_at, p.agents_used),
        (ProjectState::Running, Some(100), 1)
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
    let p = s.project(&id).await.unwrap().unwrap();
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
async fn stopping_marks_the_project_and_logs_the_cause() {
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
    let p = s.project(&id).await.unwrap().unwrap();
    assert_eq!(
        (p.state, p.stop_cause, p.stopped_at),
        (ProjectState::Stopped, Some(StopCause::Crashed), Some(150))
    );
    assert_eq!(p.times.working, 50);
    assert_eq!(p.log.last().unwrap().text, "crashed while editing");
}

#[tokio::test]
async fn workspace_changes_reach_the_project_repos() {
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
    assert_eq!(s.project(&id).await.unwrap().unwrap().repos.len(), 2);
}

#[tokio::test]
async fn removing_the_agent_keeps_the_project() {
    let s = mem().await;
    let id = AgentId("a1".into());
    s.insert_agent(&agent("a1"), "t").await.unwrap();
    s.remove_agent(&id).await.unwrap();
    assert!(s.project(&id).await.unwrap().is_some());
    assert_eq!(s.projects().await.unwrap().len(), 1);
}

#[tokio::test]
async fn project_log_and_update_edit_the_record() {
    let s = mem().await;
    let id = AgentId("a1".into());
    s.insert_agent(&agent("a1"), "t").await.unwrap();
    s.project_log(&id, 5, "repo x added").await.unwrap();
    s.update_project(&id, |p| p.agents_used = 2).await.unwrap();
    let p = s.project(&id).await.unwrap().unwrap();
    assert_eq!((p.agents_used, p.log.last().unwrap().at), (2, 5));
    assert!(s
        .project_log(&AgentId("nope".into()), 1, "x")
        .await
        .is_err());
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
    // Rows from before projects existed have an agent and no project.
    let raw =
        sqlx::SqlitePool::connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&db))
            .await
            .unwrap();
    sqlx::query("DELETE FROM projects WHERE id IN ('a2', 'a3')")
        .execute(&raw)
        .await
        .unwrap();
    assert_eq!(s.backfill_projects().await.unwrap(), 2);
    assert_eq!(s.backfill_projects().await.unwrap(), 0);
    let p = s.project(&AgentId("a2".into())).await.unwrap().unwrap();
    assert_eq!((p.created_at, p.state), (None, ProjectState::Stopped));
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
async fn migrates_v3_database_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fleet.db");
    {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();
        for sql in [
            include_str!("../migrations/001_init.sql"),
            include_str!("../migrations/002_repos.sql"),
            include_str!("../migrations/003_projects.sql"),
        ] {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO repos (name, source, location, base, added_at) VALUES ('a', 'path', '/r/a', '/r/a', 5)")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }
    for _ in 0..2 {
        let s = Store::open(&path).await.unwrap();
        let repos = s.repos().await.unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "a");
        assert_eq!(repos[0].persona, None);
        drop(s);
        let opts = sqlx::sqlite::SqliteConnectOptions::new().filename(&path);
        let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();
        let v: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(v, 5);
        pool.close().await;
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
async fn project_count_skips_archived() {
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
    s.archive_project(&AgentId("a3".into()), |p| {
        p.state = ProjectState::Archived;
    })
    .await
    .unwrap();
    assert_eq!(s.persona_project_count("p").await.unwrap(), 2);
    assert_eq!(s.persona_project_count("other").await.unwrap(), 0);
}

#[tokio::test]
async fn adopt_orphans_by_host() {
    let s = mem().await;
    for id in ["a1", "a2", "a3"] {
        let mut a = agent(id);
        a.persona = String::new();
        s.insert_agent(&a, "t").await.unwrap();
    }
    s.archive_project(&AgentId("a3".into()), |p| {
        p.state = ProjectState::Archived;
    })
    .await
    .unwrap();
    // An archived orphan's record: no agent ever ran in it, and it belongs to no persona by design.
    let mut found = s.project(&AgentId("a3".into())).await.unwrap().unwrap();
    found.id = AgentId("orphan".into());
    found.agents_used = 0;
    found.persona = String::new();
    s.insert_project(&found).await.unwrap();
    for (name, source, loc) in [
        ("gh", RepoSource::Url, "https://github.com/a/b"),
        ("ghe", RepoSource::Url, "https://ghe.example.invalid/c/d"),
        ("ssh", RepoSource::Url, "git@github.com:e/f"),
        ("local", RepoSource::Path, "/r/local"),
        ("work", RepoSource::Path, "/r/work"),
    ] {
        s.insert_repo(&row(name, source, loc, loc)).await.unwrap();
    }
    // The caller found /r/work's origin on another host, so only /r/local is passed.
    let (projects, repos) = s
        .adopt_orphans("personal", "github.com", &["/r/local".into()])
        .await
        .unwrap();
    assert_eq!(projects, 3);
    assert_eq!(repos, 2);
    for p in s.projects().await.unwrap() {
        let want = if p.id.0 == "orphan" { "" } else { "personal" };
        assert_eq!(p.persona, want, "{}", p.id.0);
    }
    for a in s.snapshot().await.unwrap().agents {
        assert_eq!(a.persona, "personal");
    }
    let persona_of = |name: &'static str| {
        let s = &s;
        async move {
            s.repos()
                .await
                .unwrap()
                .into_iter()
                .find(|r| r.name == name)
                .unwrap()
                .persona
        }
    };
    assert_eq!(persona_of("gh").await.as_deref(), Some("personal"));
    assert_eq!(persona_of("local").await.as_deref(), Some("personal"));
    assert_eq!(persona_of("ghe").await, None);
    assert_eq!(persona_of("ssh").await, None);
    assert_eq!(persona_of("work").await, None);
    assert_eq!(
        s.adopt_orphans("other", "github.com", &[]).await.unwrap(),
        (0, 0)
    );
    assert_eq!(
        s.adopt_orphans("work", "ghe.example.invalid", &["/r/work".into()])
            .await
            .unwrap(),
        (0, 2)
    );
    assert_eq!(persona_of("work").await.as_deref(), Some("work"));
}

#[tokio::test]
async fn repo_names_are_per_persona() {
    let s = mem().await;
    let mut a = row("x", RepoSource::Path, "/r/a", "/r/a");
    a.persona = Some("a".into());
    let mut b = row("x", RepoSource::Path, "/r/b", "/r/b");
    b.persona = Some("b".into());
    s.insert_repo(&a).await.unwrap();
    s.insert_repo(&b).await.unwrap();
    assert_eq!(s.repo("a", "x").await.unwrap(), Some(a.clone()));
    assert_eq!(s.repo("b", "x").await.unwrap(), Some(b.clone()));
    assert_eq!(s.repo("c", "x").await.unwrap(), None);
    assert_eq!(s.repos_for("a").await.unwrap(), vec![a.clone()]);
    let mut dup = row("y", RepoSource::Path, "/r/a", "/r/a");
    dup.persona = Some("b".into());
    assert!(s.insert_repo(&dup).await.is_err(), "same location twice");
    let mut same = a.clone();
    same.location = "/r/other".into();
    assert!(
        s.insert_repo(&same).await.is_err(),
        "same name in one persona"
    );
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
    project: Option<&str>,
    step: Option<u32>,
) -> Agent {
    Agent {
        kind,
        project: project.map(|p| AgentId(p.into())),
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
async fn fleet_agent_joins_project_record() {
    let (s, o, _) = fleet().await;
    assert_eq!(s.projects().await.unwrap().len(), 1);
    let p = s.project(&o).await.unwrap().unwrap();
    assert_eq!(p.agents_used, 2);
    assert!(p.log.iter().any(|l| l.text == "rust started for step 1"));
    assert_eq!(s.project_agents(&o).await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_restarted_orchestrator_becomes_the_lead() {
    let (s, o, _) = fleet().await;
    let o2 = fleet_agent("o2", fleet_api::AgentKind::Orchestrator, Some("o"), None);
    s.insert_agent(&o2, "t3").await.unwrap();
    let p = s.project(&o).await.unwrap().unwrap();
    assert_eq!((p.lead, p.agents_used), (Some(o2.id), 3));
    assert_eq!(p.log.last().unwrap().text, "orchestrator restarted");
}

#[tokio::test]
async fn project_stops_with_last_agent() {
    let (s, o, sp) = fleet().await;
    s.update_agent(&o, |a| stop(a, StopCause::Stopped, 200))
        .await
        .unwrap();
    assert_eq!(
        s.project(&o).await.unwrap().unwrap().state,
        ProjectState::Running
    );
    s.update_agent(&sp, |a| stop(a, StopCause::Finished, 300))
        .await
        .unwrap();
    let p = s.project(&o).await.unwrap().unwrap();
    assert_eq!(
        (p.state, p.stop_cause, p.stopped_at),
        (ProjectState::Stopped, Some(StopCause::Finished), Some(300))
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
    assert_eq!(s.project(&o).await.unwrap().unwrap().times.working, 0);
    s.update_agent(&o, |a| {
        a.status = Status::Working;
        a.status_since = 160;
    })
    .await
    .unwrap();
    let p = s.project(&o).await.unwrap().unwrap();
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
    let kinds: Vec<_> = s
        .take_inbox(&o)
        .await
        .unwrap()
        .iter()
        .map(|e| (e.kind, e.step))
        .collect();
    assert_eq!(
        kinds,
        [(NeedsYou, 1), (IdleWithoutFinish, 1), (Finished, 1)]
    );
    assert!(s.take_inbox(&o).await.unwrap().is_empty());
}

#[tokio::test]
async fn crash_marks_step_failed() {
    use fleet_api::{Approval, BreakdownVersion, FleetEventKind, Step, StepState};
    let (s, o, sp) = fleet().await;
    s.update_project(&o, |p| {
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
    let p = s.project(&o).await.unwrap().unwrap();
    assert_eq!(p.breakdowns[0].steps[0].state, StepState::Failed);
    assert_eq!(p.inbox.len(), 1);
    assert_eq!(p.inbox[0].kind, FleetEventKind::Failed);
}

#[tokio::test]
async fn repo_fleet_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fleet.db");
    {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();
        for sql in [
            include_str!("../migrations/001_init.sql"),
            include_str!("../migrations/002_repos.sql"),
            include_str!("../migrations/003_projects.sql"),
            include_str!("../migrations/004_personas.sql"),
        ] {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }
        sqlx::raw_sql("PRAGMA user_version = 4")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO repos (persona, name, source, location, base, added_at) VALUES ('p', 'a', 'path', '/r/a', '/r/a', 5)")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }
    let s = Store::open(&path).await.unwrap();
    let r = s.repo("p", "a").await.unwrap().unwrap();
    assert_eq!((r.roster.len(), r.test_command.clone()), (0, None));
    s.set_repo_fleet(
        "p",
        "a",
        &fleet_api::RepoFleet {
            roster: vec!["rust".into()],
            test_command: Some("cargo test".into()),
        },
    )
    .await
    .unwrap();
    let r = s.repo("p", "a").await.unwrap().unwrap();
    assert_eq!(r.roster, ["rust"]);
    assert_eq!(r.test_command.as_deref(), Some("cargo test"));
}
