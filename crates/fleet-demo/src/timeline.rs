//! The director's script: when each project starts and what is typed or cued afterwards, all in one file so the
//! shot is tuned without code changes.
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

use crate::scenario::{find, parse_duration, Scenario, StepKind};

/// Two cues that move cards closer than this read as one jump.
const MIN_CUE_GAP: Duration = Duration::from_millis(1500);
/// The time to read why a card needs you, before the answer arrives.
const MIN_WAIT: Duration = Duration::from_secs(2);
const LENGTH_RANGE: (Duration, Duration) = (Duration::from_secs(20), Duration::from_secs(25));

#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    pub persona: PersonaSpec,
    /// Name of the path repo the recorder creates; the director attaches `--repo` to every project.
    pub repo: String,
    pub length: Duration,
    /// The moment `board.png` is cut from.
    pub still_at: Duration,
    pub projects: Vec<Project>,
    pub cues: Vec<Cue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaSpec {
    pub name: String,
    pub commit_name: String,
    pub commit_email: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
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
    Answer { project: String, text: String },
    FollowUp { project: String, text: String },
    Panel { project: String, tab: PanelTab },
    PanelClose,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTimeline {
    persona: PersonaSpec,
    repo: String,
    length: String,
    still_at: String,
    #[serde(default)]
    project: Vec<RawProject>,
    #[serde(default)]
    cue: Vec<RawCue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProject {
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
    project: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPanel {
    project: String,
    tab: PanelTab,
}

pub fn parse_timeline(text: &str) -> Result<Timeline, String> {
    let raw: RawTimeline = toml::from_str(text).map_err(|e| e.to_string())?;
    let projects = raw
        .project
        .into_iter()
        .map(|p| {
            Ok(Project {
                at: parse_duration(&p.at).map_err(|e| format!("project {}: {e}", p.key))?,
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
                project: a.project,
                text: a.text,
            }
        } else if let Some(f) = c.follow_up {
            Action::FollowUp {
                project: f.project,
                text: f.text,
            }
        } else if let Some(p) = c.panel {
            Action::Panel {
                project: p.project,
                tab: p.tab,
            }
        } else {
            Action::PanelClose
        };
        if let Action::Answer { project, .. }
        | Action::FollowUp { project, .. }
        | Action::Panel { project, .. } = &action
        {
            if !projects.iter().any(|p| &p.key == project) {
                return Err(format!("cue {n}: no project {project:?}"));
            }
        }
        cues.push(Cue { at, action });
    }
    // Stable, so cues at the same instant keep the order they were written in.
    cues.sort_by_key(|c| c.at);
    Ok(Timeline {
        persona: raw.persona,
        repo: raw.repo,
        length: parse_duration(&raw.length).map_err(|e| format!("length: {e}"))?,
        still_at: parse_duration(&raw.still_at).map_err(|e| format!("still_at: {e}"))?,
        projects,
        cues,
    })
}

fn secs(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64())
}

/// The pacing rules as a list of violations; empty means the shot is as calm as the spec asks.
pub fn check_pacing(t: &Timeline, scenarios: &[(PathBuf, Scenario)]) -> Vec<String> {
    let mut bad = Vec::new();

    if t.length < LENGTH_RANGE.0 || t.length > LENGTH_RANGE.1 {
        bad.push(format!("length {} s is outside 20-25 s", secs(t.length)));
    }

    let moving: Vec<Duration> = t
        .cues
        .iter()
        .filter(|c| matches!(c.action, Action::Answer { .. } | Action::FollowUp { .. }))
        .map(|c| c.at)
        .collect();
    for pair in moving.windows(2) {
        if pair[1] - pair[0] < MIN_CUE_GAP {
            bad.push(format!(
                "cues at {} s and {} s move cards less than 1.5 s apart",
                secs(pair[0]),
                secs(pair[1])
            ));
        }
    }

    let panel: Vec<&Cue> = t
        .cues
        .iter()
        .filter(|c| matches!(c.action, Action::Panel { .. } | Action::PanelClose))
        .collect();
    let sequence_ok = match panel.as_slice() {
        [a, b, c] => matches!(
            (&a.action, &b.action, &c.action),
            (
                Action::Panel { project: p1, tab: PanelTab::Terminal },
                Action::Panel { project: p2, tab: PanelTab::Diff },
                Action::PanelClose,
            ) if p1 == p2
        ),
        _ => false,
    };
    if !sequence_ok {
        bad.push(
            "the side panel needs exactly one sequence: terminal, diff, close, on one project"
                .into(),
        );
    }

    for p in &t.projects {
        let Some(scenario) = find(scenarios, &p.task) else {
            bad.push(format!(
                "project {}: no scenario for task {:?}",
                p.key, p.task
            ));
            continue;
        };
        let answers: Vec<(Duration, &str)> = t
            .cues
            .iter()
            .filter_map(|c| match &c.action {
                Action::Answer { project, .. } if *project == p.key => {
                    Some((c.at, project.as_str()))
                }
                _ => None,
            })
            .collect();
        let mut answers = answers.into_iter();
        let mut clock = p.at;
        for step in &scenario.main {
            clock += step.after;
            if !matches!(
                step.kind,
                StepKind::Permission { .. } | StepKind::Question { .. }
            ) {
                continue;
            }
            match answers.next() {
                None => bad.push(format!(
                    "project {}: the wait at {} s is never answered",
                    p.key,
                    secs(clock)
                )),
                Some((at, _)) => {
                    if at < clock + MIN_WAIT {
                        bad.push(format!(
                            "project {}: answered at {} s, only {} s after its wait begins ({} s); needs 2 s",
                            p.key,
                            secs(at),
                            secs(at.saturating_sub(clock)),
                            secs(clock)
                        ));
                    }
                    clock = at.max(clock);
                }
            }
        }
        if answers.next().is_some() {
            bad.push(format!("project {}: more answers than waits", p.key));
        }
        for c in &t.cues {
            if let Action::FollowUp { project, text } = &c.action {
                if *project == p.key
                    && !scenario
                        .follow_ups
                        .iter()
                        .any(|f| f.on.trim() == text.trim())
                {
                    bad.push(format!("project {}: no follow-up arc for {text:?}", p.key));
                }
            }
        }
    }
    bad
}
