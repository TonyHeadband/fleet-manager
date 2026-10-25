//! The breakdown's version rules and its Markdown rendering, without a daemon.
use fleet_api::{AgentId, Approval, BreakdownVersion, StepDraft, StepState};
use fleet_daemon::fleet::breakdown::{next_version, render, BreakdownError};

const NOW: i64 = 5_000;

fn roster() -> Vec<String> {
    vec!["rust".into(), "reviewer".into()]
}

fn draft(n: Option<u32>, title: &str, specialist: &str, depends_on: &[u32]) -> StepDraft {
    StepDraft {
        n,
        title: title.into(),
        specialist: specialist.into(),
        brief: format!("do {title}"),
        depends_on: depends_on.to_vec(),
    }
}

fn first(drafts: &[StepDraft]) -> BreakdownVersion {
    next_version(&[], drafts, &roster(), NOW).unwrap()
}

/// `v` approved, with step `n` started by an agent.
fn approved_with_running(mut v: BreakdownVersion, n: u32) -> BreakdownVersion {
    v.approval = Approval::Approved;
    v.decided_at = Some(NOW);
    let s = v.steps.iter_mut().find(|s| s.n == n).unwrap();
    s.state = StepState::Running;
    s.agent = Some(AgentId("spec-1".into()));
    s.branch = Some("fleet/p/1-api".into());
    v
}

#[test]
fn first_version_numbers_steps() {
    let v = first(&[
        draft(None, "api", "rust", &[]),
        draft(None, "ui", "rust", &[1]),
        draft(None, "review", "reviewer", &[1, 2]),
    ]);
    assert_eq!(v.version, 1);
    assert_eq!(v.approval, Approval::Proposed);
    assert_eq!(v.proposed_at, NOW);
    assert_eq!((v.comment.clone(), v.decided_at), (None, None));
    assert_eq!(v.steps.iter().map(|s| s.n).collect::<Vec<_>>(), [1, 2, 3]);
    assert!(v.steps.iter().all(|s| s.state == StepState::Pending
        && s.agent.is_none()
        && s.branch.is_none()
        && s.report.is_none()));
    assert_eq!(v.steps[2].depends_on, [1, 2]);
    assert_eq!(v.steps[2].brief, "do review");
}

#[test]
fn rejects_bad_proposals() {
    let r = roster();
    assert_eq!(next_version(&[], &[], &r, NOW), Err(BreakdownError::Empty));

    let err = next_version(
        &[],
        &[
            draft(None, "a", "rust", &[2]),
            draft(None, "b", "rust", &[1]),
        ],
        &r,
        NOW,
    )
    .unwrap_err();
    assert!(matches!(err, BreakdownError::Cycle { .. }), "{err:?}");
    assert!(err.to_string().contains("step 1"), "{err}");

    let err = next_version(&[], &[draft(None, "a", "rust", &[9])], &r, NOW).unwrap_err();
    assert_eq!(err, BreakdownError::UnknownDependency { step: 1, dep: 9 });
    assert!(err.to_string().contains("step 1") && err.to_string().contains('9'));

    let err = next_version(&[], &[draft(None, "a", "go", &[])], &r, NOW).unwrap_err();
    assert_eq!(
        err,
        BreakdownError::NotInRoster {
            step: 1,
            specialist: "go".into()
        }
    );
    assert!(err.to_string().contains("step 1") && err.to_string().contains("go"));

    let v1 = first(&[draft(None, "a", "rust", &[])]);
    let err = next_version(&[v1], &[draft(Some(7), "a", "rust", &[])], &r, NOW).unwrap_err();
    assert_eq!(err, BreakdownError::UnknownStep { step: 7 });
    assert!(err.to_string().contains("step 7"), "{err}");

    // A step that depends on itself is a cycle too.
    let err = next_version(&[], &[draft(None, "a", "rust", &[1])], &r, NOW).unwrap_err();
    assert!(matches!(err, BreakdownError::Cycle { .. }), "{err:?}");

    // Each kind is its own error, so the orchestrator is told exactly what to fix.
    let v1 = first(&[draft(None, "a", "rust", &[])]);
    let err = next_version(
        &[v1],
        &[
            draft(Some(1), "a", "rust", &[]),
            draft(Some(1), "b", "rust", &[]),
        ],
        &r,
        NOW,
    )
    .unwrap_err();
    assert_eq!(err, BreakdownError::DuplicateStep { step: 1 });
}

#[test]
fn running_steps_must_be_carried_unchanged() {
    let r = roster();
    let v1 = approved_with_running(first(&[draft(None, "api", "rust", &[])]), 1);
    let prev = [v1];

    let err = next_version(&prev, &[draft(None, "ui", "rust", &[])], &r, NOW).unwrap_err();
    assert_eq!(err, BreakdownError::Dropped { step: 1 });
    assert!(err.to_string().contains("step 1"), "{err}");

    let mut changed = draft(Some(1), "api", "rust", &[]);
    changed.brief = "something else".into();
    let err = next_version(&prev, &[changed], &r, NOW).unwrap_err();
    assert_eq!(
        err,
        BreakdownError::Changed {
            step: 1,
            field: "brief"
        }
    );
    assert!(err.to_string().contains("step 1"), "{err}");

    let v2 = next_version(
        &prev,
        &[
            draft(Some(1), "api", "rust", &[]),
            draft(None, "ui", "rust", &[1]),
        ],
        &r,
        NOW + 1,
    )
    .unwrap();
    assert_eq!(v2.version, 2);
    assert_eq!(v2.approval, Approval::Proposed);
    assert_eq!(v2.steps[1].n, 2);
    assert_eq!(v2.steps[1].state, StepState::Pending);
    let carried = &v2.steps[0];
    assert_eq!(carried.state, StepState::Running);
    assert_eq!(carried.agent, Some(AgentId("spec-1".into())));
    assert_eq!(carried.branch.as_deref(), Some("fleet/p/1-api"));
}

#[test]
fn pending_steps_may_change_or_go() {
    let r = roster();
    let v1 = first(&[
        draft(None, "api", "rust", &[]),
        draft(None, "ui", "rust", &[1]),
    ]);
    let mut v1 = approved_with_running(v1, 1);
    v1.steps[0].state = StepState::Done;
    v1.steps[0].report = Some("done".into());
    let mut edited = draft(Some(2), "ui, smaller", "reviewer", &[]);
    edited.brief = "new brief".into();
    let v2 = next_version(
        &[v1.clone()],
        &[draft(Some(1), "api", "rust", &[]), edited],
        &r,
        NOW,
    )
    .unwrap();
    assert_eq!(v2.steps[1].title, "ui, smaller");
    assert_eq!(v2.steps[1].specialist, "reviewer");
    assert_eq!(v2.steps[1].brief, "new brief");
    assert_eq!(v2.steps[0].report.as_deref(), Some("done"));

    let v2 = next_version(&[v1], &[draft(Some(1), "api", "rust", &[])], &r, NOW).unwrap();
    assert_eq!(v2.steps.len(), 1);
}

#[test]
fn numbers_never_reused() {
    let r = roster();
    let mut v1 = first(&[draft(None, "a", "rust", &[]), draft(None, "b", "rust", &[])]);
    v1.approval = Approval::ChangesRequested;
    let v2 = next_version(
        &[v1],
        &[
            draft(Some(1), "a", "rust", &[]),
            draft(None, "c", "rust", &[]),
        ],
        &r,
        NOW,
    )
    .unwrap();
    assert_eq!(v2.steps.iter().map(|s| s.n).collect::<Vec<_>>(), [1, 3]);
}

#[test]
fn render_lists_steps() {
    let mut v = first(&[
        draft(None, "api", "rust", &[]),
        draft(None, "a | b", "reviewer", &[1]),
    ]);
    v.steps[0].agent = Some(AgentId("abcdef1234567890".into()));
    v.steps[0].state = StepState::Running;
    let md = render("fix the login bug\nand more", &v);
    assert!(md.starts_with("# Breakdown: fix the login bug\n"), "{md}");
    assert!(md.contains("Version 1, proposed"), "{md}");
    assert!(
        md.contains("| # | Step | Specialist | Depends on | State | Agent |"),
        "{md}"
    );
    assert!(
        md.contains("| 1 | api | rust | | running | abcdef1234567890 |"),
        "{md}"
    );
    assert!(
        md.contains("| 2 | a \\| b | reviewer | 1 | pending | |"),
        "{md}"
    );
    assert!(md.contains("### 1. api\n\ndo api\n"), "{md}");
    assert!(md.contains("### 2. a | b\n\ndo a | b\n"), "{md}");

    v.approval = Approval::ChangesRequested;
    v.comment = Some("split the api".into());
    let md = render("x", &v);
    assert!(
        md.contains("Version 1, changes requested: split the api"),
        "{md}"
    );
}
