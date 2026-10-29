use fleet_api::{Agent, AgentId, BlockedReason, Event, EventKind, Snapshot, Status, Workspace};
use fleet_client::board::apply;

fn ws(repo: &str, has_changes: bool) -> Workspace {
    Workspace {
        repo: repo.into(),
        branch: "fleet/x".into(),
        worktree: format!("/w/{repo}"),
        base: "abc".into(),
        from_branch: None,
        has_changes,
        owned: false,
        changed: false,
    }
}

fn agent(id: &str, status: Status) -> Agent {
    Agent {
        id: AgentId(id.into()),
        goal: "t".into(),
        title: None,
        workdir: "/w".into(),
        workspaces: vec![ws("core", false)],
        session: format!("fleet-{id}"),
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
        model: None,
        attach: None,
        mode: None,
        effort: None,
    }
}

fn base() -> Snapshot {
    Snapshot {
        seq: 5,
        agents: vec![agent("a1", Status::Working), agent("a2", Status::Queued)],
    }
}

fn ev(seq: u64, kind: EventKind) -> Event {
    Event { seq, kind }
}

#[test]
fn applies_status_changed_and_removed() {
    let mut s = base();
    apply(
        &mut s,
        &ev(
            6,
            EventKind::StatusChanged {
                id: AgentId("a1".into()),
                status: Status::NeedsYou,
                activity: "wants to run Bash".into(),
                since: 2000,
                has_changes: true,
                paused: false,
                blocked_reason: Some(BlockedReason::Permission),
                repo_request: None,
                stop_cause: None,
                disk_bytes: None,
            },
        ),
    );
    assert_eq!(s.seq, 6);
    let a1 = &s.agents[0];
    assert_eq!(
        (
            a1.status,
            a1.activity.as_str(),
            a1.status_since,
            a1.has_changes
        ),
        (Status::NeedsYou, "wants to run Bash", 2000, true)
    );
    assert_eq!(a1.blocked_reason, Some(BlockedReason::Permission));
    assert_eq!(s.agents[1], agent("a2", Status::Queued));
    apply(
        &mut s,
        &ev(
            7,
            EventKind::AgentRemoved {
                id: AgentId("a2".into()),
            },
        ),
    );
    assert_eq!(
        s.agents.iter().map(|a| a.id.0.as_str()).collect::<Vec<_>>(),
        ["a1"]
    );
}

#[test]
fn appends_created_agents_once() {
    let mut s = base();
    let created = ev(
        6,
        EventKind::AgentCreated {
            agent: agent("a3", Status::Queued),
        },
    );
    apply(&mut s, &created);
    assert_eq!(s.agents.len(), 3);
    let again = ev(
        7,
        EventKind::AgentCreated {
            agent: agent("a3", Status::Queued),
        },
    );
    apply(&mut s, &again);
    assert_eq!(
        s.agents.len(),
        3,
        "a replayed creation must not duplicate the card"
    );
}

#[test]
fn ignores_events_at_or_below_the_current_seq() {
    let mut s = base();
    let before = s.clone();
    apply(
        &mut s,
        &ev(
            5,
            EventKind::AgentRemoved {
                id: AgentId("a1".into()),
            },
        ),
    );
    apply(
        &mut s,
        &ev(
            3,
            EventKind::AgentRemoved {
                id: AgentId("a1".into()),
            },
        ),
    );
    assert_eq!(s, before);
}

#[test]
fn unknown_agent_advances_seq_but_changes_nothing() {
    let mut s = base();
    apply(
        &mut s,
        &ev(
            6,
            EventKind::StatusChanged {
                id: AgentId("ghost".into()),
                status: Status::Review,
                activity: String::new(),
                since: 1,
                has_changes: false,
                paused: false,
                blocked_reason: None,
                repo_request: None,
                stop_cause: None,
                disk_bytes: None,
            },
        ),
    );
    assert_eq!(s.seq, 6);
    assert_eq!(s.agents, base().agents);
}

#[test]
fn applies_workspaces_changed_and_derives_has_changes() {
    let mut s = base();
    s.agents[0].workspaces = vec![ws("core", false), ws("portal", false)];
    apply(
        &mut s,
        &ev(
            6,
            EventKind::WorkspacesChanged {
                id: AgentId("a1".into()),
                workspaces: vec![ws("core", false), ws("portal", true)],
            },
        ),
    );
    assert_eq!(
        s.agents[0]
            .workspaces
            .iter()
            .map(|w| w.has_changes)
            .collect::<Vec<_>>(),
        [false, true]
    );
    assert!(s.agents[0].has_changes);
    apply(
        &mut s,
        &ev(
            7,
            EventKind::WorkspacesChanged {
                id: AgentId("a1".into()),
                workspaces: vec![ws("core", false), ws("portal", false)],
            },
        ),
    );
    assert!(!s.agents[0].has_changes);
}

#[test]
fn status_changed_sets_then_clears_the_repo_request() {
    use fleet_api::RepoRequest;
    let mut s = base();
    let status = |seq, request: Option<RepoRequest>| {
        ev(
            seq,
            EventKind::StatusChanged {
                id: AgentId("a1".into()),
                status: Status::NeedsYou,
                activity: "wants repo tools".into(),
                since: 2000,
                has_changes: false,
                paused: false,
                blocked_reason: request.as_ref().map(|_| BlockedReason::RepoAccess),
                repo_request: request,
                stop_cause: None,
                disk_bytes: None,
            },
        )
    };
    let asked = RepoRequest {
        repo: "tools".into(),
        why: "w".into(),
    };
    apply(&mut s, &status(6, Some(asked.clone())));
    assert_eq!(s.agents[0].repo_request, Some(asked));
    apply(&mut s, &status(7, None));
    assert_eq!(s.agents[0].repo_request, None);
}

#[test]
fn applies_title_changed() {
    let mut s = base();
    apply(
        &mut s,
        &ev(
            6,
            EventKind::TitleChanged {
                id: AgentId("a1".into()),
                title: "Flaky tests".into(),
            },
        ),
    );
    assert_eq!(s.agents[0].title.as_deref(), Some("Flaky tests"));
}
