//! The director's script: when each task starts and what is typed or cued afterwards, all in one file so the
//! shot is tuned without code changes.
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::scenario::{find, parse_duration, Scenario, StepKind};

/// Two board changes closer than this read as one jump.
const MIN_CHANGE_GAP: Duration = Duration::from_millis(1500);
/// A panel cue is not a column change, so it may sit closer to one; two panel cues still need time to be read.
const MIN_PANEL_GAP: Duration = Duration::from_millis(750);
const MIN_PANEL_STEP: Duration = Duration::from_millis(1500);
/// Between the daemon spawning a task and the agent's first hook: what `fleet-demo-agent` and tmux take. Added
/// to every time the scenario decides.
pub const AGENT_STARTUP: Duration = Duration::from_millis(1000);
/// Float parsing of `0.3s`-style delays can land a nanosecond short.
const EPS: Duration = Duration::from_millis(1);
/// The time to read why a card needs you, before the answer arrives.
const MIN_WAIT: Duration = Duration::from_secs(2);
const LENGTH_RANGE: (Duration, Duration) = (Duration::from_secs(20), Duration::from_secs(25));

#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    /// The product the director creates; the GUI opens on its board.
    pub product: String,
    /// At least one; the first is where tasks are spawned.
    pub repos: Vec<RepoSpec>,
    pub persona: PersonaSpec,
    pub length: Duration,
    /// The moment `board.png` is cut from.
    pub still_at: Duration,
    pub tasks: Vec<Task>,
    pub cues: Vec<Cue>,
}

/// A repo the shot starts with. Its files are `<shot>/seed/<name>/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSpec {
    pub name: String,
    /// The subject of the one commit that holds the seed.
    pub message: String,
    /// (name, email) of that commit.
    pub author: (String, String),
    /// Gives the repo a bare `origin` with `main` pushed, for a shot that shows a remote.
    pub origin: bool,
    /// Specialists the repo offers orchestrated tasks; empty leaves the roster alone.
    pub roster: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaSpec {
    pub name: String,
    pub commit_name: String,
    pub commit_email: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub key: String,
    /// Sent as the spawn prompt; the demo agent finds its scenario by this exact text.
    pub task: String,
    pub at: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    pub at: Duration,
    pub action: Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelTab {
    Terminal,
    Diff,
}

impl PanelTab {
    pub fn as_str(self) -> &'static str {
        match self {
            PanelTab::Terminal => "terminal",
            PanelTab::Diff => "diff",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Answer { task: String, text: String },
    FollowUp { task: String, text: String },
    Panel { task: String, tab: PanelTab },
    PanelClose,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTimeline {
    product: String,
    #[serde(default)]
    repo: Vec<RawRepo>,
    persona: PersonaSpec,
    length: String,
    still_at: String,
    #[serde(default)]
    task: Vec<RawTask>,
    #[serde(default)]
    cue: Vec<RawCue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRepo {
    name: String,
    message: String,
    author: Option<RawAuthor>,
    #[serde(default)]
    origin: bool,
    #[serde(default)]
    roster: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAuthor {
    name: String,
    email: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTask {
    key: String,
    task: String,
    at: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCue {
    at: String,
    answer: Option<RawText>,
    follow_up: Option<RawText>,
    panel: Option<RawPanel>,
    panel_close: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawText {
    task: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPanel {
    task: String,
    tab: PanelTab,
}

/// Reads `<dir>/shot.toml`.
pub fn load_shot(dir: &Path) -> Result<Timeline, String> {
    let path = dir.join("shot.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    parse_timeline(&text)
}

pub fn parse_timeline(text: &str) -> Result<Timeline, String> {
    let raw: RawTimeline = toml::from_str(text).map_err(|e| e.to_string())?;
    if raw.repo.is_empty() {
        return Err("a shot needs at least one [[repo]]".into());
    }
    let repos = raw
        .repo
        .into_iter()
        .map(|r| RepoSpec {
            name: r.name,
            message: r.message,
            author: r.author.map_or_else(
                || ("Baker".into(), "baker@pie.example".into()),
                |a| (a.name, a.email),
            ),
            origin: r.origin,
            roster: r.roster,
        })
        .collect();
    let tasks = raw
        .task
        .into_iter()
        .map(|p| {
            Ok(Task {
                at: parse_duration(&p.at).map_err(|e| format!("task {}: {e}", p.key))?,
                key: p.key,
                task: p.task,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut cues = Vec::new();
    for (i, c) in raw.cue.into_iter().enumerate() {
        let n = i + 1;
        let at = parse_duration(&c.at).map_err(|e| format!("cue {n}: {e}"))?;
        let given = c.answer.is_some() as u8
            + c.follow_up.is_some() as u8
            + c.panel.is_some() as u8
            + (c.panel_close == Some(true)) as u8;
        if given != 1 || c.panel_close == Some(false) {
            return Err(format!(
                "cue {n}: exactly one of answer, follow_up, panel, panel_close"
            ));
        }
        let action = if let Some(a) = c.answer {
            Action::Answer {
                task: a.task,
                text: a.text,
            }
        } else if let Some(f) = c.follow_up {
            Action::FollowUp {
                task: f.task,
                text: f.text,
            }
        } else if let Some(p) = c.panel {
            Action::Panel {
                task: p.task,
                tab: p.tab,
            }
        } else {
            Action::PanelClose
        };
        if let Action::Answer { task, .. }
        | Action::FollowUp { task, .. }
        | Action::Panel { task, .. } = &action
        {
            if !tasks.iter().any(|p| &p.key == task) {
                return Err(format!("cue {n}: no task {task:?}"));
            }
        }
        cues.push(Cue { at, action });
    }
    let mut seen = std::collections::HashSet::new();
    if let Some(p) = tasks.iter().find(|p| !seen.insert(p.key.as_str())) {
        return Err(format!("duplicate task key {:?}", p.key));
    }
    // Stable, so cues at the same instant keep the order they were written in.
    cues.sort_by_key(|c| c.at);
    let length = parse_duration(&raw.length).map_err(|e| format!("length: {e}"))?;
    let still_at = parse_duration(&raw.still_at).map_err(|e| format!("still_at: {e}"))?;
    if still_at >= length {
        return Err("still_at must be before length".into());
    }
    Ok(Timeline {
        product: raw.product,
        repos,
        persona: raw.persona,
        length,
        still_at,
        tasks,
        cues,
    })
}

fn secs(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Queued,
    Working,
    NeedsYou,
    Review,
}

/// What one task does on the board, as the scenario and the cues together would drive it.
#[derive(Default)]
struct Run {
    /// Column changes: (time, what).
    changes: Vec<(Duration, String)>,
    states: Vec<(Duration, State)>,
    /// The Diff tab should show the recipe as it ends up, so it is timed against the card's latest write.
    last_write: Option<Duration>,
}

impl Run {
    fn state_at(&self, t: Duration) -> Option<State> {
        self.states
            .iter()
            .take_while(|(at, _)| *at <= t)
            .last()
            .map(|(_, s)| *s)
    }
}

fn is_wait(kind: &StepKind) -> bool {
    matches!(
        kind,
        StepKind::Permission { .. } | StepKind::Question { .. }
    )
}

/// Walks the main arc, then each follow-up the cues send. The agent starts `AGENT_STARTUP` after the spawn; after
/// an answer or a follow-up it reacts at once, so those arcs run from the cue's own time.
fn simulate(t: &Timeline, p: &Task, scenario: &Scenario, bad: &mut Vec<String>) -> Run {
    let mut run = Run::default();
    run.changes.push((p.at, format!("{} appears", p.key)));
    run.states.push((p.at, State::Queued));
    let mut clock = p.at + AGENT_STARTUP;
    run.states.push((clock, State::Working));

    let mut answers = t.cues.iter().filter_map(|c| match &c.action {
        Action::Answer { task, .. } if *task == p.key => Some(c.at),
        _ => None,
    });
    for step in &scenario.main {
        clock += step.after;
        match &step.kind {
            k if is_wait(k) => {
                run.changes.push((clock, format!("{} needs you", p.key)));
                run.states.push((clock, State::NeedsYou));
                let Some(at) = answers.next() else {
                    bad.push(format!(
                        "task {}: the wait at {} s is never answered",
                        p.key,
                        secs(clock)
                    ));
                    return run;
                };
                if at < clock + MIN_WAIT {
                    bad.push(format!(
                        "task {}: answered at {} s, only {} s after its wait begins ({} s); needs 2 s",
                        p.key,
                        secs(at),
                        secs(at.saturating_sub(clock)),
                        secs(clock)
                    ));
                }
                clock = at.max(clock);
                run.changes.push((clock, format!("{} answered", p.key)));
                run.states.push((clock, State::Working));
            }
            StepKind::Write { .. } => {
                run.last_write = run.last_write.max(Some(clock));
            }
            StepKind::Stop => {
                run.changes
                    .push((clock, format!("{} reaches Review", p.key)));
                run.states.push((clock, State::Review));
            }
            _ => {}
        }
    }
    if answers.next().is_some() {
        bad.push(format!("task {}: more answers than waits", p.key));
    }

    for c in &t.cues {
        let Action::FollowUp { task, text } = &c.action else {
            continue;
        };
        if *task != p.key {
            continue;
        }
        run.changes.push((c.at, format!("{} follows up", p.key)));
        run.states.push((c.at, State::Working));
        let Some(arc) = scenario
            .follow_ups
            .iter()
            .find(|f| f.on.trim() == text.trim())
        else {
            bad.push(format!("task {}: no follow-up arc for {text:?}", p.key));
            continue;
        };
        let mut clock = c.at;
        for step in &arc.steps {
            clock += step.after;
            match &step.kind {
                StepKind::Write { .. } => {
                    run.last_write = run.last_write.max(Some(clock));
                }
                StepKind::Stop => {
                    run.changes
                        .push((clock, format!("{} reaches Review", p.key)));
                    run.states.push((clock, State::Review));
                }
                _ => {}
            }
        }
    }
    run.states.sort_by_key(|(at, _)| *at);
    run
}

/// The pacing rules as a list of violations; empty means the shot is as calm as the spec asks.
pub fn check_pacing(t: &Timeline, scenarios: &[(PathBuf, Scenario)]) -> Vec<String> {
    let mut bad = Vec::new();

    if t.length < LENGTH_RANGE.0 || t.length > LENGTH_RANGE.1 {
        bad.push(format!("length {} s is outside 20-25 s", secs(t.length)));
    }

    let mut runs: Vec<(&Task, Run)> = Vec::new();
    for p in &t.tasks {
        match find(scenarios, &p.task) {
            Some(scenario) => runs.push((p, simulate(t, p, scenario, &mut bad))),
            None => bad.push(format!("task {}: no scenario for task {:?}", p.key, p.task)),
        }
    }

    let mut changes: Vec<&(Duration, String)> = runs.iter().flat_map(|(_, r)| &r.changes).collect();
    changes.sort_by_key(|(at, _)| *at);
    for pair in changes.windows(2) {
        if pair[1].0 < pair[0].0 + MIN_CHANGE_GAP - EPS {
            bad.push(format!(
                "board changes at {} s ({}) and {} s ({}) are less than 1.5 s apart",
                secs(pair[0].0),
                pair[0].1,
                secs(pair[1].0),
                pair[1].1
            ));
        }
    }

    let panel: Vec<&Cue> = t
        .cues
        .iter()
        .filter(|c| matches!(c.action, Action::Panel { .. } | Action::PanelClose))
        .collect();
    match panel.as_slice() {
        [a, b, c] => match (&a.action, &b.action, &c.action) {
            (
                Action::Panel {
                    task: p1,
                    tab: PanelTab::Terminal,
                },
                Action::Panel {
                    task: p2,
                    tab: PanelTab::Diff,
                },
                Action::PanelClose,
            ) if p1 == p2 => {
                if let Some((_, run)) = runs.iter().find(|(p, _)| p.key == *p1) {
                    if run.state_at(a.at) != Some(State::Working) {
                        bad.push(format!(
                            "the Terminal tab opens on {p1} at {} s, when it is not Working",
                            secs(a.at)
                        ));
                    }
                    match run.last_write {
                        Some(w) if b.at >= w => {}
                        other => bad.push(format!(
                            "the Diff tab opens on {p1} at {} s, before its last recipe write ({})",
                            secs(b.at),
                            other.map_or("never".into(), |w| format!("{} s", secs(w)))
                        )),
                    }
                }
                for pair in [(a, b), (b, c)] {
                    if pair.1.at < pair.0.at + MIN_PANEL_STEP - EPS {
                        bad.push(format!(
                            "panel cues at {} s and {} s are less than 1.5 s apart",
                            secs(pair.0.at),
                            secs(pair.1.at)
                        ));
                    }
                }
            }
            _ => bad.push(PANEL_SEQUENCE.into()),
        },
        _ => bad.push(PANEL_SEQUENCE.into()),
    }
    for cue in &panel {
        if let Some((at, what)) = changes
            .iter()
            .find(|(at, _)| at.abs_diff(cue.at) + EPS < MIN_PANEL_GAP)
            .map(|c| (c.0, &c.1))
        {
            bad.push(format!(
                "panel cue at {} s is less than 0.75 s from the board change at {} s ({what})",
                secs(cue.at),
                secs(at)
            ));
        }
    }

    let count = |state: State| {
        runs.iter()
            .filter(|(_, r)| r.state_at(t.still_at) == Some(state))
            .count()
    };
    let (w, n, r) = (
        count(State::Working),
        count(State::NeedsYou),
        count(State::Review),
    );
    if (w, n, r) != (1, 1, 1) {
        bad.push(format!(
            "still_at {} s shows {w} Working, {n} Needs you, {r} Review; it wants one of each",
            secs(t.still_at)
        ));
    }
    if let Some((at, what)) = changes
        .iter()
        .find(|(at, _)| at.abs_diff(t.still_at) + EPS < MIN_PANEL_GAP)
        .map(|c| (c.0, &c.1))
    {
        bad.push(format!(
            "still_at {} s is less than 0.75 s from the board change at {} s ({what})",
            secs(t.still_at),
            secs(at)
        ));
    }
    bad
}

const PANEL_SEQUENCE: &str =
    "the side panel needs exactly one sequence: terminal, diff, close, on one task";
