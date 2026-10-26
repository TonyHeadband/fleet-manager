use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    pub task: String,
    pub main: Vec<Step>,
    pub follow_ups: Vec<FollowUp>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FollowUp {
    /// The follow-up prompt text that starts these steps.
    pub on: String,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// Delay since the previous step.
    pub after: Duration,
    pub kind: StepKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StepKind {
    Say(String),
    Tool {
        tool: String,
        input: String,
        output: Vec<String>,
    },
    Permission {
        tool: String,
        input: String,
    },
    Question {
        text: String,
    },
    Write {
        path: String,
        from: String,
        commit: Option<String>,
    },
    Stop,
}

#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    #[error("{file}: line {line}: {message}")]
    Toml {
        file: String,
        line: usize,
        message: String,
    },
    #[error("{file}: {message}")]
    Invalid { file: String, message: String },
    #[error("{file}: {source}")]
    Io {
        file: String,
        source: std::io::Error,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScenario {
    task: String,
    #[serde(default)]
    step: Vec<RawStep>,
    #[serde(default)]
    follow_up: Vec<RawFollowUp>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFollowUp {
    on: String,
    #[serde(default)]
    step: Vec<RawStep>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStep {
    after: Option<String>,
    say: Option<String>,
    tool: Option<String>,
    input: Option<String>,
    output: Option<Vec<String>>,
    permission: Option<RawToolCall>,
    question: Option<String>,
    write: Option<RawWrite>,
    commit: Option<String>,
    stop: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawToolCall {
    tool: String,
    input: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWrite {
    path: String,
    from: String,
}

pub fn parse(text: &str, origin: &Path) -> Result<Scenario, ScenarioError> {
    let file = origin.display().to_string();
    let raw: RawScenario = toml::from_str(text).map_err(|e| {
        let line = e
            .span()
            .map(|s| text[..s.start.min(text.len())].matches('\n').count() + 1)
            .unwrap_or(1);
        ScenarioError::Toml {
            file: file.clone(),
            line,
            message: e.message().to_string(),
        }
    })?;
    let invalid = |message: String| ScenarioError::Invalid {
        file: file.clone(),
        message,
    };
    let convert = |steps: Vec<RawStep>, scope: &str| -> Result<Vec<Step>, ScenarioError> {
        steps
            .into_iter()
            .enumerate()
            .map(|(i, s)| {
                convert_step(s).map_err(|m| invalid(format!("{scope} step {}: {m}", i + 1)))
            })
            .collect()
    };
    let main = convert(raw.step, "step")?;
    let follow_ups = raw
        .follow_up
        .into_iter()
        .enumerate()
        .map(|(i, f)| {
            Ok(FollowUp {
                on: f.on,
                steps: convert(f.step, &format!("follow_up {}", i + 1))?,
            })
        })
        .collect::<Result<_, ScenarioError>>()?;
    Ok(Scenario {
        task: raw.task,
        main,
        follow_ups,
    })
}

fn convert_step(s: RawStep) -> Result<Step, String> {
    let after = match &s.after {
        Some(d) => parse_duration(d)?,
        None => Duration::ZERO,
    };
    if s.stop == Some(false) {
        return Err("`stop = false` is not a step; remove the key".into());
    }
    let tool_given = s.tool.is_some() || s.input.is_some() || s.output.is_some();
    let kinds = [
        s.say.is_some(),
        tool_given,
        s.permission.is_some(),
        s.question.is_some(),
        s.write.is_some(),
        s.stop.is_some(),
    ];
    match kinds.iter().filter(|k| **k).count() {
        1 => {}
        0 => {
            return Err("needs one of say, tool, permission, question, write, stop".into());
        }
        _ => {
            return Err(
                "exactly one of say, tool, permission, question, write, stop per step".into(),
            );
        }
    }
    if s.commit.is_some() && s.write.is_none() {
        return Err("`commit` only applies to a `write` step".into());
    }
    let kind = if let Some(text) = s.say {
        StepKind::Say(text)
    } else if tool_given {
        StepKind::Tool {
            tool: s.tool.ok_or("tool step needs `tool`")?,
            input: s.input.ok_or("tool step needs `input`")?,
            output: s.output.unwrap_or_default(),
        }
    } else if let Some(p) = s.permission {
        StepKind::Permission {
            tool: p.tool,
            input: p.input,
        }
    } else if let Some(text) = s.question {
        StepKind::Question { text }
    } else if let Some(w) = s.write {
        StepKind::Write {
            path: w.path,
            from: w.from,
            commit: s.commit,
        }
    } else {
        StepKind::Stop
    };
    Ok(Step { after, kind })
}

fn parse_duration(text: &str) -> Result<Duration, String> {
    let bad = || format!("bad duration {text:?}, expected `<n>s` or `<n>ms`");
    if let Some(n) = text.strip_suffix("ms") {
        return n
            .parse::<u64>()
            .map(Duration::from_millis)
            .map_err(|_| bad());
    }
    let n = text.strip_suffix('s').ok_or_else(bad)?;
    let secs: f64 = n.parse().map_err(|_| bad())?;
    if !secs.is_finite() || secs < 0.0 {
        return Err(bad());
    }
    Ok(Duration::from_secs_f64(secs))
}

pub fn load_dir(dir: &Path) -> Result<Vec<(PathBuf, Scenario)>, ScenarioError> {
    let io_err = |source| ScenarioError::Io {
        file: dir.display().to_string(),
        source,
    };
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(io_err)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()
        .map_err(io_err)?;
    // Sorted so a failure or a duplicate-task conflict is reproducible across machines.
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let is_scenario = path.extension().is_some_and(|e| e == "toml")
            && path.file_name().is_some_and(|n| n != "timeline.toml");
        if !is_scenario {
            continue;
        }
        let text = fs::read_to_string(&path).map_err(|source| ScenarioError::Io {
            file: path.display().to_string(),
            source,
        })?;
        let scenario = parse(&text, &path)?;
        out.push((path, scenario));
    }
    Ok(out)
}

pub fn find<'a>(all: &'a [(PathBuf, Scenario)], prompt: &str) -> Option<&'a Scenario> {
    let prompt = prompt.trim();
    all.iter().map(|(_, s)| s).find(|s| s.task == prompt)
}
