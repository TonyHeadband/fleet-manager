use fleet_api::{Agent, AgentId, EventKind, RepoSource, Status, Workspace};
use fleet_daemon::store::{RepoRow, Store};

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
    assert!(matches!(e3.kind, EventKind::StatusChanged { status: Status::Working, .. }));
    assert!(matches!(e4.kind, EventKind::AgentRemoved { .. }));
}

#[tokio::test]
async fn unchanged_update_emits_no_event() {
    let s = mem().await;
    s.insert_agent(&agent("a1"), "t1").await.unwrap();
    let before = s.snapshot().await.unwrap().seq;
    assert!(s.update_agent(&AgentId("a1".into()), |_| {}).await.unwrap().is_empty());
    assert_eq!(s.snapshot().await.unwrap().seq, before);
}

#[tokio::test]
async fn blocked_reason_change_is_a_visible_event() {
    let s = mem().await;
    s.insert_agent(&agent("a1"), "t").await.unwrap();
    let ev = s
        .update_agent(&AgentId("a1".into()), |a| a.blocked_reason = Some(fleet_api::BlockedReason::Permission))
        .await
        .unwrap()
        .remove(0);
    assert!(matches!(
        ev.kind,
        EventKind::StatusChanged { blocked_reason: Some(fleet_api::BlockedReason::Permission), .. }
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
        a.workspaces.push(Workspace { repo: "portal".into(), branch: "b".into(), worktree: "/w".into(), base: "x".into(), from_branch: None, has_changes: false, owned: false });
    })
    .await
    .unwrap();
    let evs = s.update_agent(&id, |a| { a.workspaces[1].has_changes = true; second = Some(()); }).await.unwrap();
    assert!(second.is_some());
    assert!(matches!(evs.as_slice(), [e] if matches!(e.kind, EventKind::WorkspacesChanged { .. })), "{evs:?}");
}

#[tokio::test]
async fn update_of_unknown_agent_is_an_error() {
    let s = mem().await;
    assert!(s.update_agent(&AgentId("nope".into()), |_| {}).await.is_err());
}

#[tokio::test]
async fn events_since_returns_gap_free_tail() {
    let s = mem().await;
    s.insert_agent(&agent("a1"), "t1").await.unwrap();
    s.insert_agent(&agent("a2"), "t2").await.unwrap();
    s.update_agent(&AgentId("a1".into()), |a| a.paused = true).await.unwrap();
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
    assert_eq!(snap.agents.iter().map(|a| a.id.0.as_str()).collect::<Vec<_>>(), ["a1", "a2"]);
}

#[tokio::test]
async fn tokens_are_stored_and_removed_with_the_agent() {
    let s = mem().await;
    s.insert_agent(&agent("a1"), "secret").await.unwrap();
    assert_eq!(s.agent_token(&AgentId("a1".into())).await.unwrap().as_deref(), Some("secret"));
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
    s.update_agent(&AgentId("a1".into()), |a| a.status = Status::Working).await.unwrap();
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
    assert!(s.insert_agent(&agent("a1"), "t").await.is_err(), "duplicate id");
    assert_eq!(rx.try_recv().unwrap().seq, 1);
    assert!(rx.try_recv().is_err());
}

fn row(name: &str, source: RepoSource, location: &str, base: &str) -> RepoRow {
    RepoRow { name: name.into(), source, location: location.into(), base: base.into() }
}

fn seed(name: &str, path: &str) -> fleet_daemon::config::Repo {
    fleet_daemon::config::Repo { name: name.into(), path: path.into() }
}

#[tokio::test]
async fn insert_then_list_and_lookup_by_name_and_location() {
    let s = mem().await;
    let a = row("a", RepoSource::Path, "/r/a", "/r/a");
    let b = row("b", RepoSource::Url, "git@h:o/b.git", "/state/repos/b");
    s.insert_repo(&a).await.unwrap();
    s.insert_repo(&b).await.unwrap();
    assert_eq!(s.repos().await.unwrap(), vec![a.clone(), b.clone()]);
    assert_eq!(s.repo("b").await.unwrap(), Some(b.clone()));
    assert_eq!(s.repo("nope").await.unwrap(), None);
    assert_eq!(s.repo_by_location("/r/a").await.unwrap(), Some(a));
    assert_eq!(s.repo_by_location("/r/zzz").await.unwrap(), None);
}

#[tokio::test]
async fn inserting_a_taken_name_fails() {
    let s = mem().await;
    s.insert_repo(&row("a", RepoSource::Path, "/r/a", "/r/a")).await.unwrap();
    assert!(s.insert_repo(&row("a", RepoSource::Path, "/r/other", "/r/other")).await.is_err());
    assert_eq!(s.repos().await.unwrap().len(), 1);
}

#[tokio::test]
async fn seeding_twice_adds_nothing_the_second_time() {
    let s = mem().await;
    let seeds = [seed("a", "/r/a"), seed("b", "/r/b")];
    s.seed_repos(&seeds).await.unwrap();
    let first = s.repos().await.unwrap();
    assert_eq!(first.len(), 2);
    assert!(first.iter().all(|r| r.source == RepoSource::Path && r.base.to_str() == Some(&r.location)));
    s.seed_repos(&seeds).await.unwrap();
    assert_eq!(s.repos().await.unwrap(), first);
}

#[tokio::test]
async fn seeding_skips_a_location_already_remembered_under_another_name() {
    let s = mem().await;
    s.insert_repo(&row("mine", RepoSource::Path, "/r/a", "/r/a")).await.unwrap();
    s.seed_repos(&[seed("a", "/r/a")]).await.unwrap();
    let all = s.repos().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "mine");
}
