//! The `env:` block of `.fleet/workflow.yml`: variables every agent of a task and its gate share. A value may start
//! with `{task}/`, which becomes a folder inside the task's own folder.
use super::breakdown::BREAKDOWN_FILE;
use super::ops::{FLEET_DIR, INTERNAL_DIR, ORCHESTRATOR_DIR, SNAPSHOTS_DIR};
use crate::agent_config::{reserved_env, valid_env_name};
use fleet_api::{AgentKind, EnvLine, EnvSource, Task};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// The gate's own folder in the task folder; a repo path may not take it over.
pub const GATE_BUILD_DIR: &str = "gate-build";

const TASK_PREFIX: &str = "{task}/";

/// Names that would change which program runs or where it looks, whatever the repo's intent.
const REPO_REFUSED: [&str; 8] = [
    "PATH",
    "HOME",
    "XDG_CONFIG_HOME",
    "BASH_ENV",
    "ENV",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
];

/// Folders the daemon owns inside a task folder.
const TASK_OWNED: [&str; 6] = [
    ORCHESTRATOR_DIR,
    FLEET_DIR,
    INTERNAL_DIR,
    SNAPSHOTS_DIR,
    BREAKDOWN_FILE,
    GATE_BUILD_DIR,
];

/// What a task of the repo runs with, by name: agents get the repo env over the fleet env, the gate the repo env
/// alone. A fleet value the repo sets again is marked and followed by the repo's line.
pub fn effective(
    fleet: &BTreeMap<String, String>,
    repo: &BTreeMap<String, String>,
) -> Vec<EnvLine> {
    let mut lines = Vec::new();
    let names: std::collections::BTreeSet<&String> = fleet.keys().chain(repo.keys()).collect();
    for name in names {
        if let Some(value) = fleet.get(name) {
            lines.push(EnvLine {
                name: name.clone(),
                value: value.clone(),
                source: EnvSource::Fleet,
                gate: false,
                overridden: repo.contains_key(name),
            });
        }
        if let Some(value) = repo.get(name) {
            lines.push(EnvLine {
                name: name.clone(),
                value: value.clone(),
                source: EnvSource::Repo,
                gate: true,
                overridden: false,
            });
        }
    }
    lines
}

/// The first bad entry, as a sentence.
pub fn check(env: &BTreeMap<String, String>) -> Result<(), String> {
    for (name, value) in env {
        if !valid_env_name(name) {
            return Err(format!(
                "env `{name}`: a name is letters, digits and `_`, not starting with a digit"
            ));
        }
        // Windows reads `Path` as `PATH`.
        let upper = name.to_ascii_uppercase();
        if reserved_env(&upper)
            || REPO_REFUSED.contains(&upper.as_str())
            || upper.starts_with("DYLD_")
        {
            return Err(format!("env `{name}`: this name is not allowed"));
        }
        check_value(name, value)?;
    }
    Ok(())
}

fn check_value(name: &str, value: &str) -> Result<(), String> {
    let misplaced = || {
        format!(
            "env `{name}`: `{{task}}` must start the value as `{{task}}/<path>`, and no other placeholder is known"
        )
    };
    let Some(rest) = value.strip_prefix(TASK_PREFIX) else {
        return if has_placeholder(value) {
            Err(misplaced())
        } else {
            Ok(())
        };
    };
    if has_placeholder(rest) {
        return Err(misplaced());
    }
    if rest.trim().is_empty() {
        return Err(format!("env `{name}`: `{{task}}/` needs a path after it"));
    }
    if rest.contains('\\') {
        return Err(format!("env `{name}`: use `/` in the path, not `\\`"));
    }
    // An empty component would make `Path::join` see an absolute path and leave the task folder.
    let parts: Vec<&str> = rest.split('/').collect();
    if parts
        .iter()
        .any(|c| c.is_empty() || *c == "." || *c == "..")
    {
        return Err(format!(
            "env `{name}`: the path must not have an empty, `.` or `..` component"
        ));
    }
    // Windows drops a trailing `.` or space (`internal.` is `internal`), and it and macOS ignore case.
    if parts[0].ends_with(['.', ' ']) {
        return Err(format!(
            "env `{name}`: `{}` must not end with `.` or a space",
            parts[0]
        ));
    }
    if TASK_OWNED
        .iter()
        .any(|owned| owned.eq_ignore_ascii_case(parts[0]))
    {
        return Err(format!(
            "env `{name}`: `{}` is a folder Fleet Manager keeps in the task folder",
            parts[0]
        ));
    }
    Ok(())
}

/// A `{letters}` group: the shape of a placeholder, whichever one it names.
fn has_placeholder(text: &str) -> bool {
    let mut rest = text;
    while let Some(i) = rest.find('{') {
        let after = &rest[i + 1..];
        if let Some(end) = after.find('}') {
            if end > 0 && after[..end].chars().all(|c| c.is_ascii_alphabetic()) {
                return true;
            }
        }
        rest = after;
    }
    false
}

pub fn expand(env: &BTreeMap<String, String>, task_dir: &Path) -> BTreeMap<String, String> {
    env.iter()
        .map(|(k, v)| {
            let value = match v.strip_prefix(TASK_PREFIX) {
                Some(rest) => task_dir.join(rest).to_string_lossy().into_owned(),
                None => v.clone(),
            };
            (k.clone(), value)
        })
        .collect()
}

/// The folders inside the task folder the env points at, in name order.
pub fn task_paths(env: &BTreeMap<String, String>, task_dir: &Path) -> Vec<PathBuf> {
    env.values()
        .filter_map(|v| v.strip_prefix(TASK_PREFIX))
        .map(|rest| task_dir.join(rest))
        .collect()
}

/// The folders an orchestrated task's builds write into: each `{task}/…` path of its repo env and the gate's own,
/// once each, a path inside another left out (it is weighed and removed with that one). None for a solo task.
pub fn build_dirs(task: &Task) -> Vec<PathBuf> {
    if task.kind != AgentKind::Orchestrator {
        return vec![];
    }
    build_dirs_in(&task.repo_env, Path::new(&task.workdir))
}

fn build_dirs_in(env: &BTreeMap<String, String>, task_dir: &Path) -> Vec<PathBuf> {
    let mut paths = task_paths(env, task_dir);
    paths.push(task_dir.join(GATE_BUILD_DIR));
    // Sorted by component, a folder comes right before everything inside it.
    paths.sort();
    let mut out: Vec<PathBuf> = vec![];
    for p in paths {
        if !out.last().is_some_and(|kept| p.starts_with(kept)) {
            out.push(p);
        }
    }
    out
}

/// True when `path` (one of `build_dirs`) can be weighed or removed whole as the task's own: it is inside `task_dir`
/// and every folder on the way to it is a real one, not a link (`<task>/out/src` with `out` a link to a worktree
/// would be that worktree's `src`); and resolved, it is neither at, in nor around one of `protected`. `path` itself
/// may be a link: removing it leaves what it points to. Blocking.
pub fn owns_build_dir(path: &Path, task_dir: &Path, protected: &[&Path]) -> bool {
    let Ok(rel) = path.strip_prefix(task_dir) else {
        return false;
    };
    if rel.as_os_str().is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return false;
    }
    let Ok(root) = fleet_fs::canonical(task_dir) else {
        return false;
    };
    let resolved = root.join(rel);
    let no_link = path
        .parent()
        .zip(resolved.parent())
        .is_some_and(|(parent, want)| fleet_fs::canonical(parent).is_ok_and(|p| p == want));
    no_link
        && !protected.iter().any(|p| {
            let real = fleet_fs::canonical(p).unwrap_or_else(|_| p.to_path_buf());
            overlaps(path, p) || overlaps(&resolved, &real)
        })
}

/// One path at or under the other, ignoring case: on a case-insensitive disk `<task>/Orchestrator` is
/// `<task>/orchestrator`, and a false match only keeps a folder.
fn overlaps(a: &Path, b: &Path) -> bool {
    let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
    let (a, b) = (lower(a), lower(b));
    a.starts_with(&b) || b.starts_with(&a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_marks_overrides_and_gate() {
        let fleet = BTreeMap::from([("A".into(), "1".into()), ("B".into(), "2".into())]);
        let repo = BTreeMap::from([("B".into(), "3".into()), ("C".into(), "{task}/t".into())]);
        let got: Vec<_> = effective(&fleet, &repo)
            .into_iter()
            .map(|l| (l.name, l.value, l.source, l.gate, l.overridden))
            .collect();
        let line = |n: &str, v: &str, s, g, o| (n.to_string(), v.to_string(), s, g, o);
        assert_eq!(
            got,
            [
                line("A", "1", EnvSource::Fleet, false, false),
                line("B", "2", EnvSource::Fleet, false, true),
                line("B", "3", EnvSource::Repo, true, false),
                line("C", "{task}/t", EnvSource::Repo, true, false),
            ]
        );
        assert!(effective(&BTreeMap::new(), &BTreeMap::new()).is_empty());
    }

    #[test]
    fn expands_only_the_prefix_and_lists_task_paths() {
        let env = BTreeMap::from([
            ("A".into(), "{task}/target".into()),
            ("B".into(), "-Dwarnings".into()),
        ]);
        let dir = Path::new("/t/x");
        // The platform's own separator: `\` on Windows.
        assert_eq!(expand(&env, dir)["A"], dir.join("target").to_string_lossy());
        assert_eq!(expand(&env, dir)["B"], "-Dwarnings");
        assert_eq!(task_paths(&env, dir), [PathBuf::from("/t/x/target")]);
    }

    #[test]
    fn build_dirs_are_listed_once_with_the_gates() {
        let env = BTreeMap::from([
            ("A".into(), "{task}/out".into()),
            ("B".into(), "{task}/out/b".into()),
            ("C".into(), "{task}/target".into()),
            ("D".into(), "{task}/target".into()),
            ("E".into(), "{task}/out-c".into()),
        ]);
        let dir = Path::new("/t/x");
        assert_eq!(
            build_dirs_in(&env, dir),
            ["/t/x/gate-build", "/t/x/out", "/t/x/out-c", "/t/x/target"].map(PathBuf::from)
        );
    }
}
