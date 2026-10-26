//! The repo's own gate: `.fleet/workflow.yml` on the base branch lists the steps `open_pr` runs. This module parses
//! it, reads it from `origin/<base>` (never a local ref, which an agent can rewrite) and picks the steps a change needs.
use crate::git::{self, GitError};
use serde::Deserialize;
use std::path::Path;

pub const WORKFLOW_FILE: &str = ".fleet/workflow.yml";

#[derive(Clone, Debug, PartialEq)]
pub struct Workflow {
    /// Git `:(glob)` pathspecs for files no step needs to check.
    pub ignore: Vec<String>,
    /// In file order, which is run order.
    pub steps: Vec<WorkflowStep>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowStep {
    pub name: String,
    pub run: String,
    /// Empty means the step always runs.
    pub paths: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkflowError {
    #[error("no {WORKFLOW_FILE} on {base}")]
    Missing { base: String },
    #[error("{WORKFLOW_FILE} on {base} is invalid: {error}")]
    Invalid { base: String, error: String },
    /// The last stderr line of the failed fetch.
    #[error("cannot fetch the base branch: {0}")]
    Fetch(String),
    #[error(transparent)]
    Git(#[from] GitError),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWorkflow {
    #[serde(default)]
    ignore: Vec<String>,
    steps: serde_yaml_ng::Mapping,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStep {
    run: String,
    #[serde(default)]
    paths: Vec<String>,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Parses and validates the file's text. The error is a sentence for the person who owns the file.
pub fn parse(text: &str) -> Result<Workflow, String> {
    let raw: RawWorkflow = serde_yaml_ng::from_str(text).map_err(|e| e.to_string())?;
    if raw.steps.is_empty() {
        return Err("`steps` is empty".into());
    }
    let mut steps: Vec<WorkflowStep> = Vec::new();
    for (key, value) in raw.steps {
        let name = key
            .as_str()
            .ok_or("a step name is not a string")?
            .to_string();
        if !valid_name(&name) {
            return Err(format!("step name `{name}` must match [a-z0-9-]+"));
        }
        // serde_yaml_ng refuses a repeated key already; this keeps the rule if that ever changes.
        if steps.iter().any(|s| s.name == name) {
            return Err(format!("step `{name}` is defined twice"));
        }
        let step: RawStep =
            serde_yaml_ng::from_value(value).map_err(|e| format!("step `{name}`: {e}"))?;
        if step.run.trim().is_empty() {
            return Err(format!("step `{name}` has an empty `run`"));
        }
        steps.push(WorkflowStep {
            name,
            run: step.run,
            paths: step.paths,
        });
    }
    Ok(Workflow {
        ignore: raw.ignore,
        steps,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Picked {
    pub step: WorkflowStep,
    /// The changed files that picked it; empty for a step without `paths`.
    pub because: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    pub run: Vec<Picked>,
    /// Changed files no step's `paths` and no `ignore` pattern covers; any makes every step run.
    pub unmatched: Vec<String>,
    /// Every changed file, ignored ones included.
    pub changed: usize,
}

fn glob(pattern: &str) -> String {
    format!(":(glob){pattern}")
}

/// Picks the steps for the change `from..to` (`from` is the merge base). A file nothing covers fails closed: every
/// step runs, since the file may be something the steps depend on.
pub async fn select(wt: &Path, from: &str, to: &str, wf: &Workflow) -> Result<Selection, GitError> {
    let changed = git::changed_files(wt, from, to, &[]).await?.len();

    let mut cover = vec![".".to_string()];
    cover.extend(
        wf.steps
            .iter()
            .flat_map(|s| &s.paths)
            .chain(&wf.ignore)
            .map(|p| format!(":(exclude,glob){p}")),
    );
    let unmatched = git::changed_files(wt, from, to, &cover).await?;

    let mut run = Vec::new();
    for step in &wf.steps {
        let because = if step.paths.is_empty() {
            Vec::new()
        } else {
            let specs: Vec<String> = step.paths.iter().map(|p| glob(p)).collect();
            git::changed_files(wt, from, to, &specs).await?
        };
        if unmatched.is_empty() && !step.paths.is_empty() && because.is_empty() {
            continue;
        }
        run.push(Picked {
            step: step.clone(),
            because,
        });
    }
    Ok(Selection {
        run,
        unmatched,
        changed,
    })
}

fn last_line(stderr: &str) -> String {
    stderr.lines().last().unwrap_or("").trim().to_string()
}

async fn read_at(repo: &Path, base: &str, rev: &str) -> Result<Workflow, WorkflowError> {
    let text = git::show_file(repo, rev, WORKFLOW_FILE)
        .await?
        .ok_or_else(|| WorkflowError::Missing { base: base.into() })?;
    parse(&text).map_err(|error| WorkflowError::Invalid {
        base: base.into(),
        error,
    })
}

/// Fetches `+refs/heads/<base>:refs/remotes/origin/<base>` with `env`, then reads the file at `origin/<base>`.
/// Returns the workflow and the base commit it was read at.
pub async fn fetch(
    repo: &Path,
    base: &str,
    env: &[(String, String)],
) -> Result<(Workflow, String), WorkflowError> {
    git::fetch_branch(repo, base, env)
        .await
        .map_err(|e| match e {
            GitError::Git { stderr, .. } => WorkflowError::Fetch(last_line(&stderr)),
            other => WorkflowError::Git(other),
        })?;
    let rev = format!("refs/remotes/origin/{base}");
    let sha = git::rev_parse_commit(repo, &rev).await?;
    Ok((read_at(repo, base, &sha).await?, sha))
}

/// Same read without fetching (the remote-tracking ref as last fetched), for the repo list.
pub async fn read(repo: &Path, base: &str) -> Result<Workflow, WorkflowError> {
    read_at(repo, base, &format!("refs/remotes/origin/{base}")).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_steps_in_file_order() {
        let wf =
            parse("steps:\n  b:\n    run: x\n  a:\n    run: y\n    paths: [src/**]\n").unwrap();
        assert_eq!(
            wf.steps.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert_eq!(wf.steps[1].paths, ["src/**"]);
    }

    #[test]
    fn refuses_bad_files() {
        for bad in [
            "steps: {}\n",
            "steps:\n  a:\n    run: x\n    when: y\n",
            "steps:\n  A:\n    run: x\n",
            "steps:\n  a:\n    run: '  '\n",
            "nope: 1\nsteps:\n  a:\n    run: x\n",
            "steps:\n  a:\n    run: x\n  a:\n    run: y\n",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
