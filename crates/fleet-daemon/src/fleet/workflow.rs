//! The repo's own gate: `.fleet/workflow.yml` on the base branch lists the steps `open_pr` runs. This module parses
//! it, reads it from `origin/<base>` (never a local ref, which an agent can rewrite) and picks the steps a change needs.
use crate::git::{self, GitError};
use crate::repos::{Resolved, Start};
use fleet_api::WorkflowState;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const WORKFLOW_FILE: &str = ".fleet/workflow.yml";

#[derive(Clone, Debug, PartialEq)]
pub struct Workflow {
    /// Git `:(glob)` pathspecs for files no step needs to check.
    pub ignore: Vec<String>,
    /// In file order, which is run order.
    pub steps: Vec<WorkflowStep>,
    pub pr: PrFacts,
}

/// What the repo says about its own pull requests; each key is optional.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrFacts {
    /// Pins the PR style by definition name; whether it exists is checked at spawn.
    pub style: Option<String>,
    /// Repo-relative file the release-notes line goes into.
    pub release_notes: Option<String>,
    /// Git `:(glob)` pathspecs for files whose change is breaking.
    pub breaking: Vec<String>,
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
    #[error("cannot fetch the base branch: timed out")]
    FetchTimedOut,
    #[error(transparent)]
    Git(#[from] GitError),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWorkflow {
    #[serde(default)]
    ignore: Vec<String>,
    steps: serde_yaml_ng::Mapping,
    #[serde(default)]
    pr: Option<RawPr>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPr {
    style: Option<String>,
    release_notes: Option<String>,
    #[serde(default)]
    breaking: Vec<String>,
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

/// Patterns are relative to the repo root: a blank, absolute or `..` pattern could never match what the author meant.
fn check_patterns(owner: &str, patterns: &[String]) -> Result<(), String> {
    for p in patterns {
        if p.trim().is_empty() {
            return Err(format!("{owner} has a blank pattern"));
        }
        if p.starts_with('/') {
            return Err(format!("{owner}: pattern `{p}` must not start with `/`"));
        }
        // Git pathspecs split on `/` and read `\` as a glob escape, so `a\b` would never match `a/b`
        // and `..\x` would slip past the `..` check below.
        if p.contains('\\') {
            return Err(format!(
                "{owner}: pattern `{p}` has a `\\`; use `/` in patterns"
            ));
        }
        if p.split('/').any(|c| c == "..") {
            return Err(format!("{owner}: pattern `{p}` must not contain `..`"));
        }
    }
    Ok(())
}

/// Parses and validates the file's text. The error is a sentence for the person who owns the file.
pub fn parse(text: &str) -> Result<Workflow, String> {
    let raw: RawWorkflow = serde_yaml_ng::from_str(text).map_err(|e| e.to_string())?;
    if raw.steps.is_empty() {
        return Err("`steps` is empty".into());
    }
    check_patterns("ignore", &raw.ignore)?;
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
        check_patterns(&format!("step `{name}`"), &step.paths)?;
        if step.run.trim().is_empty() {
            return Err(format!("step `{name}` has an empty `run`"));
        }
        steps.push(WorkflowStep {
            name,
            run: step.run,
            paths: step.paths,
        });
    }
    let pr = match raw.pr {
        Some(pr) => check_pr(pr)?,
        None => PrFacts::default(),
    };
    Ok(Workflow {
        ignore: raw.ignore,
        steps,
        pr,
    })
}

fn check_pr(pr: RawPr) -> Result<PrFacts, String> {
    if let Some(style) = &pr.style {
        if !valid_name(style) {
            return Err(format!("pr: style `{style}` must match [a-z0-9-]+"));
        }
    }
    if let Some(path) = &pr.release_notes {
        if path.trim().is_empty() {
            return Err("pr: `release_notes` is blank".into());
        }
        if path.starts_with('/') {
            return Err(format!(
                "pr: release_notes `{path}` must not start with `/`"
            ));
        }
        if path.split(['/', '\\']).any(|c| c == "..") {
            return Err(format!("pr: release_notes `{path}` must not contain `..`"));
        }
    }
    check_patterns("pr `breaking`", &pr.breaking)?;
    Ok(PrFacts {
        style: pr.style,
        release_notes: pr.release_notes,
        breaking: pr.breaking,
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
    // With nothing to cover files, every step is pathless and runs anyway: nothing fell back.
    let unmatched = if cover.len() == 1 {
        Vec::new()
    } else {
        git::changed_files(wt, from, to, &cover).await?
    };

    let mut run = Vec::new();
    for step in &wf.steps {
        let because = if step.paths.is_empty() {
            Vec::new()
        } else {
            // Ignored files neither pick a step nor, being left out here, count towards it.
            let specs: Vec<String> = step
                .paths
                .iter()
                .map(|p| glob(p))
                .chain(wf.ignore.iter().map(|p| format!(":(exclude,glob){p}")))
                .collect();
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

/// The last `fatal:`/`error:` line, which names the cause; git prints hints after it. Else the last non-empty line.
fn last_line(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    lines
        .iter()
        .rev()
        .find(|l| l.starts_with("fatal:") || l.starts_with("error:"))
        .or(lines.last())
        .copied()
        .unwrap_or("")
        .to_string()
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
/// Returns the workflow and the base commit it was read at. The fetch gets `git::REMOTE_TIMEOUT`: callers hold the
/// base clone's lock across it, so a stalled remote must not block every task on the repo.
pub async fn fetch(
    repo: &Path,
    base: &str,
    env: &[(String, String)],
) -> Result<(Workflow, String), WorkflowError> {
    fetch_within(repo, base, env, git::REMOTE_TIMEOUT).await
}

/// `fetch` with its own bound on the network part; on timeout git is killed and nothing is read.
pub async fn fetch_within(
    repo: &Path,
    base: &str,
    env: &[(String, String)],
    timeout: Duration,
) -> Result<(Workflow, String), WorkflowError> {
    match tokio::time::timeout(timeout, git::fetch_branch(repo, base, env)).await {
        Err(_) => return Err(WorkflowError::FetchTimedOut),
        Ok(Err(GitError::Git { stderr, .. })) => {
            return Err(WorkflowError::Fetch(last_line(&stderr)))
        }
        Ok(Err(other)) => return Err(WorkflowError::Git(other)),
        Ok(Ok(())) => {}
    }
    let rev = format!("refs/remotes/origin/{base}");
    let sha = git::rev_parse_commit(repo, &rev).await?;
    Ok((read_at(repo, base, &sha).await?, sha))
}

/// Same read without fetching (the remote-tracking ref as last fetched), for the repo list.
pub async fn read(repo: &Path, base: &str) -> Result<Workflow, WorkflowError> {
    read_at(repo, base, &format!("refs/remotes/origin/{base}")).await
}

fn without_origin(b: &str) -> String {
    b.strip_prefix("origin/").unwrap_or(b).to_string()
}

async fn origin_branch(repo: &Path) -> Option<String> {
    git::origin_head(repo)
        .await
        .ok()
        .flatten()
        .map(|(_, b)| without_origin(&b))
}

/// The base a task started at `start` off `repo` gets, before its worktree exists: the branch `Repos::attach` will
/// record as `from_branch`, then `pr_base`'s fallbacks (`origin/HEAD`, else `main`). Keep the three in step.
async fn base_for(repo: &Path, start: &Start) -> String {
    let from = match start {
        Start::Checkout(p) => git::current_branch(p).await.ok().flatten(),
        Start::OriginHead => match origin_branch(repo).await {
            Some(b) => Some(b),
            None => git::current_branch(repo).await.ok().flatten(),
        },
    };
    if let Some(b) = from {
        return without_origin(&b);
    }
    origin_branch(repo).await.unwrap_or_else(|| "main".into())
}

/// The base the task being created on `resolved` will open its PR against.
pub(crate) async fn creation_base(resolved: &Resolved) -> String {
    base_for(&resolved.base, &resolved.start).await
}

/// What the repo list shows for a repo read at `at` (see `Repos::listing_base`): the file at `origin/<base>` as last
/// fetched, without fetching. None (nothing on disk yet) reads as unreadable.
pub(crate) async fn state(at: Option<(PathBuf, Start)>) -> WorkflowState {
    let Some((repo, start)) = at else {
        return WorkflowState::Unreadable {
            base: "main".into(),
            error: "not cloned yet; the first task started from its URL clones it".into(),
        };
    };
    let base = base_for(&repo, &start).await;
    match read(&repo, &base).await {
        Ok(wf) => WorkflowState::Found {
            steps: wf.steps.into_iter().map(|s| s.name).collect(),
            base,
        },
        Err(WorkflowError::Missing { .. }) => WorkflowState::Missing { base },
        Err(WorkflowError::Invalid { error, .. }) => WorkflowState::Invalid { base, error },
        // No origin, or `origin/<base>` never fetched: there is nothing to read the file from.
        Err(e) => WorkflowState::Unreadable {
            base,
            error: e.to_string(),
        },
    }
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
    fn empty_paths_is_like_missing_paths() {
        let wf = parse("steps:\n  a:\n    run: x\n    paths: []\n").unwrap();
        assert!(wf.steps[0].paths.is_empty());
    }

    #[test]
    fn last_line_prefers_the_fatal_line() {
        let err = "fatal: '/x' does not appear to be a git repository\nfatal: Could not read from remote repository.\n\nhint: check access\n";
        assert_eq!(
            last_line(err),
            "fatal: Could not read from remote repository."
        );
        assert_eq!(last_line("plain\n\n"), "plain");
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
            "steps:\n  a:\n    run: x\n    paths: ['']\n",
            "steps:\n  a:\n    run: x\n    paths: ['/src/**']\n",
            "steps:\n  a:\n    run: x\n    paths: ['src/../x']\n",
            "ignore: ['  ']\nsteps:\n  a:\n    run: x\n",
            "ignore: ['/x']\nsteps:\n  a:\n    run: x\n",
            "ignore: ['../x']\nsteps:\n  a:\n    run: x\n",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    const STEPS: &str = "steps:\n  a:\n    run: x\n";

    #[test]
    fn pr_section_is_optional() {
        assert_eq!(parse(STEPS).unwrap().pr, PrFacts::default());
    }

    #[test]
    fn pr_section_reads_its_three_keys() {
        let wf = parse(&format!(
            "{STEPS}pr:\n  style: conventional\n  release_notes: docs/RELEASES.md\n  breaking: ['crates/api/**', 'proto/*.proto']\n"
        ))
        .unwrap();
        assert_eq!(wf.pr.style.as_deref(), Some("conventional"));
        assert_eq!(wf.pr.release_notes.as_deref(), Some("docs/RELEASES.md"));
        assert_eq!(wf.pr.breaking, ["crates/api/**", "proto/*.proto"]);
    }

    #[test]
    fn unknown_key_in_pr_is_invalid() {
        let err = parse(&format!("{STEPS}pr:\n  styl: x\n")).unwrap_err();
        assert!(err.contains("styl"), "{err}");
    }

    #[test]
    fn breaking_patterns_are_checked_like_paths() {
        for (bad, want) in [
            ("['/etc/**']", "must not start with `/`"),
            ("['src/../x']", "must not contain `..`"),
            ("['..\\x']", "use `/`"),
            ("['  ']", "blank pattern"),
        ] {
            let err = parse(&format!("{STEPS}pr:\n  breaking: {bad}\n")).unwrap_err();
            assert!(err.contains(want), "{bad}: {err}");
            assert!(err.contains("breaking"), "{bad}: {err}");
        }
    }

    #[test]
    fn release_notes_must_be_a_relative_path() {
        for bad in [
            "/CHANGELOG.md",
            "../CHANGELOG.md",
            "docs/../../x",
            "'  '",
            "..\\x",
        ] {
            let err = parse(&format!("{STEPS}pr:\n  release_notes: {bad}\n")).unwrap_err();
            assert!(err.contains("release_notes"), "{bad}: {err}");
        }
    }

    #[test]
    fn pr_style_must_be_a_definition_name() {
        for bad in ["Conventional", "a/b", "''"] {
            let err = parse(&format!("{STEPS}pr:\n  style: {bad}\n")).unwrap_err();
            assert!(err.contains("style"), "{bad}: {err}");
        }
    }

    #[test]
    fn refuses_backslashes_in_patterns() {
        for bad in [
            "steps:\n  a:\n    run: x\n    paths: ['crates\\fleet-daemon\\**']\n",
            "ignore: ['..\\x']\nsteps:\n  a:\n    run: x\n",
        ] {
            let err = parse(bad).unwrap_err();
            assert!(err.contains("use `/`"), "{bad}: {err}");
        }
    }
}
