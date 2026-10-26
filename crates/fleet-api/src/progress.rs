//! How far tasks and releases have got, and the calendar arithmetic for release dates. Days are counted since
//! 1970-01-01, UTC.
use crate::{
    AgentKind, Approval, PrState, Release, Status, StepState, Task, TaskOutcome, TaskState,
};

/// Where the task stands for its release's count. `prs` holds one entry per PR found, so a PR closed without merging
/// and a later one on the same branch are both there: the closed one neither holds the task back nor finishes it.
pub fn task_outcome(t: &Task) -> TaskOutcome {
    if t.done_by_hand {
        return TaskOutcome::DoneByHand;
    }
    let open = t.prs.iter().any(|p| p.state == PrState::Open);
    let merged = t.prs.iter().any(|p| p.state == PrState::Merged);
    if merged && !open {
        TaskOutcome::Merged
    } else if t.state == TaskState::Archived {
        TaskOutcome::Dropped
    } else {
        TaskOutcome::Open
    }
}

/// How far one task has got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskProgress {
    /// An orchestrated task: steps of the approved breakdown done, out of all its steps.
    Steps { done: u32, planned: u32 },
    /// A solo task: the board column its agent is in.
    Column(Status),
}

/// For a solo task the column comes from the record's state (`Working` while running, else `Stopped`): the live
/// agent's status is not on the record, so a caller that has the agent replaces it.
pub fn task_progress(t: &Task) -> TaskProgress {
    if t.kind == AgentKind::Solo {
        return TaskProgress::Column(match t.state {
            TaskState::Running => Status::Working,
            TaskState::Stopped | TaskState::Archived => Status::Stopped,
        });
    }
    let Some(plan) = t
        .breakdowns
        .iter()
        .rev()
        .find(|b| b.approval == Approval::Approved)
    else {
        return TaskProgress::Steps {
            done: 0,
            planned: 0,
        };
    };
    TaskProgress::Steps {
        done: plan
            .steps
            .iter()
            .filter(|s| s.state == StepState::Done)
            .count() as u32,
        planned: plan.steps.len() as u32,
    }
}

/// A release's two bars: tasks merged out of those that count, and days used out of the span.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReleaseProgress {
    pub merged: u32,
    pub counted: u32,
    pub days_used: u32,
    pub days_total: u32,
    pub overdue_days: u32,
}

/// `tasks` may hold any product's tasks; only those in `r` count. A date that does not parse (never stored: the
/// routes check them) reads as `today`.
pub fn release_progress(r: &Release, tasks: &[Task], today: i64) -> ReleaseProgress {
    let mut p = ReleaseProgress::default();
    for t in tasks
        .iter()
        .filter(|t| t.release == Some(r.id) && t.product == r.product)
    {
        match task_outcome(t) {
            TaskOutcome::Dropped => continue,
            TaskOutcome::Merged | TaskOutcome::DoneByHand => p.merged += 1,
            TaskOutcome::Open => {}
        }
        p.counted += 1;
    }
    let start = parse_date(&r.start).unwrap_or(today);
    let target = parse_date(&r.target).unwrap_or(today);
    let total = (target - start + 1).max(1);
    p.days_total = total as u32;
    p.days_used = (today - start + 1).clamp(0, total) as u32;
    if r.released_at.is_none() {
        p.overdue_days = (today - target).max(0) as u32;
    }
    p
}

/// `"YYYY-MM-DD"` as days since 1970-01-01; None when it is not a real calendar date.
pub fn parse_date(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = &s[r];
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())
            .flatten()
    };
    let (y, m, d) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    if !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// The UTC day number of a Unix time in seconds.
pub fn today(now_secs: i64) -> i64 {
    now_secs.div_euclid(86_400)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Howard Hinnant's days-from-civil.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentId, StatusTimes, TaskPr};

    fn task(id: &str, prs: &[PrState]) -> Task {
        Task {
            id: AgentId(id.into()),
            product: "app".into(),
            release: Some(1),
            name: id.into(),
            goal: "g".into(),
            workdir: "/w".into(),
            repos: vec![],
            state: TaskState::Stopped,
            stop_cause: None,
            created_at: None,
            stopped_at: None,
            archived_at: None,
            agents_used: 1,
            times: StatusTimes::default(),
            log: vec![],
            footprint: None,
            last_screen: None,
            archive: None,
            persona: "p".into(),
            kind: Default::default(),
            role: None,
            roster: vec![],
            lead: None,
            breakdowns: vec![],
            inbox: vec![],
            gate_runs: vec![],
            prs: prs
                .iter()
                .enumerate()
                .map(|(i, s)| TaskPr {
                    url: format!("https://github.com/o/r/pull/{i}"),
                    repo: "r".into(),
                    state: *s,
                    merged_at: (*s == PrState::Merged).then_some(100),
                })
                .collect(),
            done_at: None,
            done_by_hand: false,
        }
    }

    fn release(start: &str, target: &str) -> Release {
        Release {
            id: 1,
            product: "app".into(),
            version: "1.0".into(),
            start: start.into(),
            target: target.into(),
            released_at: None,
        }
    }

    #[test]
    fn outcome_open_with_no_prs() {
        assert_eq!(task_outcome(&task("a", &[])), TaskOutcome::Open);
    }

    #[test]
    fn outcome_merged_when_only_pr_merged() {
        assert_eq!(
            task_outcome(&task("a", &[PrState::Merged])),
            TaskOutcome::Merged
        );
    }

    #[test]
    fn outcome_open_when_only_pr_closed() {
        assert_eq!(
            task_outcome(&task("a", &[PrState::Closed])),
            TaskOutcome::Open
        );
    }

    #[test]
    fn outcome_merged_after_closed_then_merged() {
        assert_eq!(
            task_outcome(&task("a", &[PrState::Closed, PrState::Merged])),
            TaskOutcome::Merged
        );
        // An open PR on another repo holds it back.
        assert_eq!(
            task_outcome(&task("a", &[PrState::Merged, PrState::Open])),
            TaskOutcome::Open
        );
    }

    #[test]
    fn archived_unmerged_is_dropped() {
        let mut t = task("a", &[PrState::Closed]);
        t.state = TaskState::Archived;
        assert_eq!(task_outcome(&t), TaskOutcome::Dropped);
        t.done_by_hand = true;
        assert_eq!(task_outcome(&t), TaskOutcome::DoneByHand);
    }

    #[test]
    fn archived_merged_stays_merged() {
        let mut t = task("a", &[PrState::Merged]);
        t.state = TaskState::Archived;
        assert_eq!(task_outcome(&t), TaskOutcome::Merged);
    }

    #[test]
    fn progress_skips_dropped() {
        let r = release("2026-11-01", "2026-11-30");
        let mut dropped = task("d", &[]);
        dropped.state = TaskState::Archived;
        let mut by_hand = task("h", &[]);
        by_hand.done_by_hand = true;
        let mut elsewhere = task("e", &[PrState::Merged]);
        elsewhere.release = Some(2);
        let tasks = [
            task("m", &[PrState::Merged]),
            task("o", &[PrState::Open]),
            dropped,
            by_hand,
            elsewhere,
        ];
        let p = release_progress(&r, &tasks, parse_date("2026-11-10").unwrap());
        assert_eq!((p.merged, p.counted), (2, 3));
        assert_eq!((p.days_used, p.days_total, p.overdue_days), (10, 30, 0));
    }

    #[test]
    fn one_day_release_never_divides_by_zero() {
        let r = release("2026-11-01", "2026-11-01");
        let day = parse_date("2026-11-01").unwrap();
        let p = release_progress(&r, &[], day);
        assert_eq!((p.merged, p.counted), (0, 0));
        assert_eq!((p.days_used, p.days_total, p.overdue_days), (1, 1, 0));
        let before = release_progress(&r, &[], day - 5);
        assert_eq!((before.days_used, before.days_total), (0, 1));
    }

    #[test]
    fn past_target_reports_overdue_not_over_100() {
        let mut r = release("2026-11-01", "2026-11-30");
        let late = parse_date("2026-12-05").unwrap();
        let p = release_progress(&r, &[], late);
        assert_eq!((p.days_used, p.days_total, p.overdue_days), (30, 30, 5));
        r.released_at = Some(1);
        assert_eq!(release_progress(&r, &[], late).overdue_days, 0);
    }

    #[test]
    fn orchestrated_progress_counts_done_steps_of_the_approved_breakdown() {
        use crate::{BreakdownVersion, Step};
        let step = |n: u32, state: StepState| Step {
            n,
            title: "t".into(),
            specialist: "s".into(),
            brief: "b".into(),
            depends_on: vec![],
            state,
            agent: None,
            branch: None,
            report: None,
        };
        let version = |v: u32, approval: Approval, steps: Vec<Step>| BreakdownVersion {
            version: v,
            steps,
            approval,
            comment: None,
            proposed_at: 0,
            decided_at: None,
        };
        let mut t = task("a", &[]);
        t.kind = AgentKind::Orchestrator;
        assert_eq!(
            task_progress(&t),
            TaskProgress::Steps {
                done: 0,
                planned: 0
            }
        );
        t.breakdowns = vec![
            version(
                1,
                Approval::Approved,
                vec![step(1, StepState::Done), step(2, StepState::Running)],
            ),
            version(
                2,
                Approval::Proposed,
                vec![
                    step(1, StepState::Done),
                    step(2, StepState::Done),
                    step(3, StepState::Pending),
                ],
            ),
        ];
        assert_eq!(
            task_progress(&t),
            TaskProgress::Steps {
                done: 1,
                planned: 2
            }
        );
        let solo = task("s", &[]);
        assert_eq!(task_progress(&solo), TaskProgress::Column(Status::Stopped));
    }

    #[test]
    fn parse_date_rejects_feb_30() {
        assert_eq!(parse_date("2026-02-30"), None);
        assert_eq!(parse_date("2027-02-29"), None);
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("2026-1-01"), None);
        assert_eq!(parse_date("2026-+1-01"), None);
    }

    #[test]
    fn parse_date_accepts_2028_02_29() {
        assert!(parse_date("2028-02-29").is_some());
        assert_eq!(
            parse_date("2028-03-01"),
            parse_date("2028-02-29").map(|d| d + 1)
        );
    }

    #[test]
    fn parse_date_round_trips_2026_10_26() {
        assert_eq!(parse_date("2026-10-26"), Some(20752));
        assert_eq!(today(20752 * 86_400 + 86_399), 20752);
        assert_eq!(parse_date("1970-01-01"), Some(0));
    }
}
