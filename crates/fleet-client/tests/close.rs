use fleet_api::{Agent, AgentId, Status, Workspace};
use fleet_client::close::{close_action, dialog_text, CloseAction, Summary};
use fleet_client::config::CloseChoice;

fn agent(status: Status) -> Agent {
    Agent {
        id: AgentId(format!("{status:?}")),
        goal: "t".into(),
        workdir: "/w".into(),
        workspaces: vec![Workspace {
            repo: "core".into(),
            branch: "b".into(),
            worktree: "/w/core".into(),
            base: "x".into(),
            from_branch: None,
            has_changes: false,
            owned: false,
            changed: false,
        }],
        session: "s".into(),
        status,
        activity: String::new(),
        status_since: 1,
        has_changes: false,
        paused: false,
        owner: "o".into(),
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

fn agents(statuses: &[Status]) -> Vec<Agent> {
    statuses.iter().map(|s| agent(*s)).collect()
}

/// Settings offers Ask as an explicit choice: it must ask, even with nothing at risk and a daemon we started.
#[test]
fn ask_always_asks_even_with_nothing_at_risk_and_a_daemon_we_started() {
    for list in [vec![], agents(&[Status::Stopped, Status::Stopped])] {
        assert_eq!(
            close_action(&list, true, None),
            CloseAction::Ask(Summary {
                running: 0,
                working: 0,
                needs_you: 0
            })
        );
    }
}

#[test]
fn nothing_at_risk_but_we_did_not_start_it_asks() {
    let got = close_action(&agents(&[Status::Stopped]), false, None);
    assert_eq!(
        got,
        CloseAction::Ask(Summary {
            running: 0,
            working: 0,
            needs_you: 0
        })
    );
}

#[test]
fn agents_at_risk_ask_unless_a_choice_is_remembered() {
    let list = agents(&[
        Status::Working,
        Status::NeedsYou,
        Status::NeedsYou,
        Status::Review,
    ]);
    let want = CloseAction::Ask(Summary {
        running: 4,
        working: 1,
        needs_you: 2,
    });
    assert_eq!(close_action(&list, true, None), want);
    assert_eq!(close_action(&list, false, None), want);
}

#[test]
fn remembered_keep_running_closes_only() {
    for started in [true, false] {
        assert_eq!(
            close_action(
                &agents(&[Status::Working]),
                started,
                Some(CloseChoice::KeepRunning)
            ),
            CloseAction::CloseOnly
        );
        assert_eq!(
            close_action(&[], started, Some(CloseChoice::KeepRunning)),
            CloseAction::CloseOnly
        );
    }
}

#[test]
fn remembered_stop_daemon_stops_a_daemon_we_started_even_with_agents_at_risk() {
    // The user chose this; the dialog is for people who have not.
    assert_eq!(
        close_action(
            &agents(&[Status::NeedsYou]),
            true,
            Some(CloseChoice::StopDaemon)
        ),
        CloseAction::StopDaemonAndClose
    );
}

#[test]
fn a_remembered_stop_never_stops_a_daemon_we_did_not_start_without_asking() {
    // "A daemon the client did not start is never stopped without the dialog": started by hand, or by another window.
    let got = close_action(
        &agents(&[Status::Stopped]),
        false,
        Some(CloseChoice::StopDaemon),
    );
    assert_eq!(
        got,
        CloseAction::Ask(Summary {
            running: 0,
            working: 0,
            needs_you: 0
        })
    );
    assert!(matches!(
        close_action(
            &agents(&[Status::Working]),
            false,
            Some(CloseChoice::StopDaemon)
        ),
        CloseAction::Ask(_)
    ));
}

#[test]
fn running_counts_every_agent_not_stopped() {
    let list = agents(&[
        Status::Queued,
        Status::Working,
        Status::NeedsYou,
        Status::Talking,
        Status::Review,
        Status::Stopped,
    ]);
    let CloseAction::Ask(s) = close_action(&list, true, None) else {
        panic!("should ask")
    };
    // Talking and Review agents have live sessions too, which the stop ends.
    assert_eq!((s.running, s.working, s.needs_you), (5, 1, 1));
}

#[test]
fn dialog_text_states_the_numbers_and_the_consequence() {
    let t = dialog_text(&Summary {
        running: 3,
        working: 1,
        needs_you: 2,
    });
    assert!(
        t.starts_with(
            "3 agents are running. They will stop now and resume automatically next time the daemon starts."
        ),
        "{t}"
    );
    for needle in ["1 working", "2 waiting for you"] {
        assert!(t.contains(needle), "missing {needle:?} in: {t}");
    }
    assert!(!t.contains("tmux") && !t.contains("unverified"), "{t}");
}

#[test]
fn dialog_text_leads_with_the_count_of_two() {
    let t = dialog_text(&Summary {
        running: 2,
        working: 2,
        needs_you: 0,
    });
    assert!(
        t.starts_with(
            "2 agents are running. They will stop now and resume automatically next time the daemon starts."
        ),
        "{t}"
    );
}

#[test]
fn dialog_text_uses_singular_for_one_agent_and_omits_zero_parts() {
    let t = dialog_text(&Summary {
        running: 1,
        working: 1,
        needs_you: 0,
    });
    assert!(
        t.starts_with(
            "1 agent is running. It will stop now and resume automatically next time the daemon starts."
        ),
        "{t}"
    );
    assert!(
        t.contains("1 working") && !t.contains("waiting for you"),
        "{t}"
    );
    let none = dialog_text(&Summary {
        running: 0,
        working: 0,
        needs_you: 0,
    });
    assert!(none.contains("No agents are running"), "{none}");
    assert!(
        !none.contains("resume automatically"),
        "nothing is running to resume: {none}"
    );
}
