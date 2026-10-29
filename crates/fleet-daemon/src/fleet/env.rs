//! The `env:` block of `.fleet/workflow.yml`: variables every agent of a task and its gate share. A value may start
//! with `{task}/`, which becomes a folder inside the task's own folder.
use super::breakdown::BREAKDOWN_FILE;
use super::ops::{FLEET_DIR, INTERNAL_DIR, ORCHESTRATOR_DIR, SNAPSHOTS_DIR};
use crate::agent_config::{reserved_env, valid_env_name};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

/// The first bad entry, as a sentence.
pub fn check(env: &BTreeMap<String, String>) -> Result<(), String> {
    for (name, value) in env {
        if !valid_env_name(name) {
            return Err(format!(
                "env `{name}`: a name is letters, digits and `_`, not starting with a digit"
            ));
        }
        if reserved_env(name) || REPO_REFUSED.contains(&name.as_str()) || name.starts_with("DYLD_")
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
    if TASK_OWNED.contains(&parts[0]) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_only_the_prefix_and_lists_task_paths() {
        let env = BTreeMap::from([
            ("A".into(), "{task}/target".into()),
            ("B".into(), "-Dwarnings".into()),
        ]);
        let dir = Path::new("/t/x");
        assert_eq!(expand(&env, dir)["A"], "/t/x/target");
        assert_eq!(expand(&env, dir)["B"], "-Dwarnings");
        assert_eq!(task_paths(&env, dir), [PathBuf::from("/t/x/target")]);
    }
}
