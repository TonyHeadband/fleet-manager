//! The breakdown an orchestrator proposes: what a new version may change, and its `BREAKDOWN.md` rendering.
use fleet_api::{Approval, BreakdownVersion, Step, StepDraft, StepState};
use std::collections::{BTreeMap, HashSet};

/// The rendering's file in the task folder, beside (not inside) the agents' folders.
pub const BREAKDOWN_FILE: &str = "BREAKDOWN.md";

/// The owner's decision on a `propose_breakdown` call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BreakdownAnswer {
    Approved,
    /// The owner's comment, never empty.
    ChangesRequested(String),
}

/// Why a proposal is refused. Each names the step, so the orchestrator knows what to fix.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BreakdownError {
    #[error("the breakdown has no steps")]
    Empty,
    #[error("step {step} is not in the last version; leave `n` out for a new step")]
    UnknownStep { step: u32 },
    #[error("step {step} is listed twice")]
    DuplicateStep { step: u32 },
    #[error("step {step}: {specialist} is not one of this task's specialists")]
    NotASpecialist { step: u32, specialist: String },
    #[error("step {step} depends on step {dep}, which is not in this breakdown")]
    UnknownDependency { step: u32, dep: u32 },
    #[error("step {} depends on itself through {}", .steps[0], path(.steps))]
    Cycle { steps: Vec<u32> },
    #[error("steps {first} and {second} both say they add the release note: pick one")]
    TwoReleaseNotes { first: u32, second: u32 },
    #[error("step {step} has started, so it must be carried over unchanged (with its `n`)")]
    Dropped { step: u32 },
    #[error("step {step} has started, so its {field} cannot change")]
    Changed { step: u32, field: &'static str },
}

fn path(steps: &[u32]) -> String {
    steps
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(" → ")
}

/// The version `drafts` make after `previous` (oldest first), or why it is refused. A carried step keeps its
/// progress from the last approved version, which is the one specialists run from; a new step takes the next
/// number never used in any version, in draft order.
pub fn next_version(
    previous: &[BreakdownVersion],
    drafts: &[StepDraft],
    specialists: &[String],
    now: i64,
) -> Result<BreakdownVersion, BreakdownError> {
    if drafts.is_empty() {
        return Err(BreakdownError::Empty);
    }
    let latest = previous.last();
    let approved = previous
        .iter()
        .rev()
        .find(|v| v.approval == Approval::Approved);
    let mut next = previous
        .iter()
        .flat_map(|v| &v.steps)
        .map(|s| s.n)
        .max()
        .unwrap_or(0)
        + 1;
    let mut seen = HashSet::new();
    let mut steps = Vec::with_capacity(drafts.len());
    for d in drafts {
        let (n, progress) = match d.n {
            Some(n) => {
                let Some(old) = latest.and_then(|v| v.steps.iter().find(|s| s.n == n)) else {
                    return Err(BreakdownError::UnknownStep { step: n });
                };
                let source = approved
                    .and_then(|v| v.steps.iter().find(|s| s.n == n))
                    .unwrap_or(old);
                (n, Some(source))
            }
            None => {
                next += 1;
                (next - 1, None)
            }
        };
        if !seen.insert(n) {
            return Err(BreakdownError::DuplicateStep { step: n });
        }
        steps.push(Step {
            n,
            title: d.title.clone(),
            specialist: d.specialist.clone(),
            brief: d.brief.clone(),
            depends_on: d.depends_on.clone(),
            release_note: d.release_note,
            mode: None,
            effort: None,
            state: progress.map_or(StepState::Pending, |s| s.state),
            agent: progress.and_then(|s| s.agent.clone()),
            branch: progress.and_then(|s| s.branch.clone()),
            report: progress.and_then(|s| s.report.clone()),
        });
    }
    for s in &steps {
        if !specialists.contains(&s.specialist) {
            return Err(BreakdownError::NotASpecialist {
                step: s.n,
                specialist: s.specialist.clone(),
            });
        }
        if let Some(&dep) = s.depends_on.iter().find(|d| !seen.contains(d)) {
            return Err(BreakdownError::UnknownDependency { step: s.n, dep });
        }
    }
    if let Some(cycle) = find_cycle(&steps) {
        return Err(BreakdownError::Cycle { steps: cycle });
    }
    let mut noted = steps.iter().filter(|s| s.release_note);
    if let (Some(first), Some(second)) = (noted.next(), noted.next()) {
        return Err(BreakdownError::TwoReleaseNotes {
            first: first.n,
            second: second.n,
        });
    }
    for old in approved
        .iter()
        .flat_map(|v| &v.steps)
        .filter(|s| s.state != StepState::Pending)
    {
        let Some(new) = steps.iter().find(|s| s.n == old.n) else {
            return Err(BreakdownError::Dropped { step: old.n });
        };
        let field = if new.title != old.title {
            "title"
        } else if new.specialist != old.specialist {
            "specialist"
        } else if new.brief != old.brief {
            "brief"
        } else if new.depends_on != old.depends_on {
            "depends_on"
        } else if new.release_note != old.release_note {
            "release_note"
        } else {
            continue;
        };
        return Err(BreakdownError::Changed { step: old.n, field });
    }
    Ok(BreakdownVersion {
        version: latest.map_or(1, |v| v.version + 1),
        steps,
        approval: Approval::Proposed,
        comment: None,
        proposed_at: now,
        decided_at: None,
    })
}

/// A dependency cycle as the path that closes it (`[1, 2, 1]`), if there is one. Every dependency is known to exist.
fn find_cycle(steps: &[Step]) -> Option<Vec<u32>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Visiting,
        Done,
    }
    fn visit(
        n: u32,
        deps: &BTreeMap<u32, &[u32]>,
        marks: &mut BTreeMap<u32, Mark>,
        stack: &mut Vec<u32>,
    ) -> Option<Vec<u32>> {
        match marks.get(&n) {
            Some(Mark::Done) => return None,
            Some(Mark::Visiting) => {
                let start = stack.iter().position(|&s| s == n).unwrap_or(0);
                let mut cycle = stack[start..].to_vec();
                cycle.push(n);
                return Some(cycle);
            }
            None => {}
        }
        marks.insert(n, Mark::Visiting);
        stack.push(n);
        for &d in deps.get(&n).copied().unwrap_or_default() {
            if let Some(c) = visit(d, deps, marks, stack) {
                return Some(c);
            }
        }
        stack.pop();
        marks.insert(n, Mark::Done);
        None
    }
    let deps: BTreeMap<u32, &[u32]> = steps.iter().map(|s| (s.n, &s.depends_on[..])).collect();
    let mut marks = BTreeMap::new();
    steps
        .iter()
        .find_map(|s| visit(s.n, &deps, &mut marks, &mut Vec::new()))
}

/// One line, safe inside a Markdown table cell.
fn cell(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "\\|")
}

fn row(cells: &[String]) -> String {
    let mut line = String::from("|");
    for c in cells {
        if c.is_empty() {
            line.push_str(" |");
        } else {
            line.push_str(&format!(" {c} |"));
        }
    }
    line.push('\n');
    line
}

/// A step's state as `BREAKDOWN.md` and the fleet tools say it.
pub fn state_word(state: StepState) -> &'static str {
    match state {
        StepState::Pending => "pending",
        StepState::Running => "running",
        StepState::Done => "done",
        StepState::Failed => "failed",
    }
}

/// `BREAKDOWN.md`: a copy of the stored version for reading and diffing; the task record stays the source.
pub fn render(task_goal: &str, v: &BreakdownVersion) -> String {
    let title = task_goal.lines().next().unwrap_or_default().trim();
    let approval = match v.approval {
        Approval::Proposed => "proposed",
        Approval::Approved => "approved",
        Approval::ChangesRequested => "changes requested",
    };
    let mut text = format!("# Breakdown: {title}\n\nVersion {}, {approval}", v.version);
    if let Some(c) = v.comment.as_deref().filter(|c| !c.is_empty()) {
        text.push_str(&format!(": {c}"));
    }
    text.push_str(
        ".\n\n| # | Step | Specialist | Depends on | State | Agent |\n|---|---|---|---|---|---|\n",
    );
    for s in &v.steps {
        let state = state_word(s.state);
        let deps = s
            .depends_on
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        text.push_str(&row(&[
            s.n.to_string(),
            cell(&s.title),
            cell(&s.specialist),
            deps,
            state.into(),
            s.agent.as_ref().map(|a| cell(&a.0)).unwrap_or_default(),
        ]));
    }
    for s in &v.steps {
        text.push_str(&format!(
            "\n### {}. {}\n\n{}\n",
            s.n,
            s.title.split_whitespace().collect::<Vec<_>>().join(" "),
            s.brief.trim_end()
        ));
    }
    text
}
