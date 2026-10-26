//! A repo's own Claude Code permission rules, carried over to the agent. Claude starts in the task folder, one
//! level above each repo, and reads `.claude/` settings only from its working directory, so a repo's
//! `.claude/settings.json` never loads. Its deny and ask rules are copied into the agent's `--settings` file,
//! rewritten to cover the same files from the task folder; allow rules, hooks and MCP servers are not (one
//! repo's allow would loosen every repo of the task), and spawn says so.
use serde_json::Value;
use std::path::Path;

/// The settings files Claude would read from inside the repo.
const SETTINGS_FILES: [&str; 2] = [".claude/settings.json", ".claude/settings.local.json"];

/// Rules merged from a task's repos, and one line per thing that was not merged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoRules {
    pub deny: Vec<String>,
    pub ask: Vec<String>,
    pub not_applied: Vec<String>,
}

impl RepoRules {
    /// Reads each repo's settings; `repos` pairs the repo's folder name inside the task folder with its path.
    pub fn collect<'a>(repos: impl IntoIterator<Item = (&'a str, &'a Path)>) -> RepoRules {
        let mut rules = RepoRules::default();
        for (folder, path) in repos {
            for file in SETTINGS_FILES {
                let Ok(text) = std::fs::read_to_string(path.join(file)) else {
                    continue;
                };
                let settings: Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(e) => {
                        rules.not_applied.push(format!(
                            "{folder}: {file} is not valid JSON ({e}), so none of its rules apply"
                        ));
                        continue;
                    }
                };
                rules.add(folder, file, &settings);
            }
            if path.join(".mcp.json").is_file() {
                rules.not_applied.push(format!(
                    "{folder}: .mcp.json is not loaded, so its MCP servers are not available"
                ));
            }
        }
        rules.deny.dedup();
        rules.ask.dedup();
        rules
    }

    fn add(&mut self, folder: &str, file: &str, settings: &Value) {
        let list = |key: &str| -> Vec<String> {
            settings["permissions"][key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|r| rewrite(r, folder))
                .collect()
        };
        self.deny.extend(list("deny"));
        self.ask.extend(list("ask"));
        let mut skipped = Vec::new();
        if settings["permissions"]["allow"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
        {
            skipped.push("allow rules");
        }
        if settings.get("hooks").is_some_and(|h| !h.is_null()) {
            skipped.push("hooks");
        }
        let others = settings
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(k, _)| !matches!(k.as_str(), "permissions" | "hooks" | "$schema"))
            .count();
        if others > 0 {
            skipped.push("other settings");
        }
        if !skipped.is_empty() {
            self.not_applied.push(format!(
                "{folder}: {file} {} do not apply to this agent (its deny and ask rules do)",
                skipped.join(", ")
            ));
        }
    }
}

/// One rule as it must read in a settings file used from the task folder. Only file rules carry paths: `//abs`
/// and `~/` paths keep their meaning; `/p` (anchored at the settings file, i.e. the repo) and `./p` become
/// `<folder>/p`; a bare pattern matches at any depth below the repo, so it becomes `<folder>/**/p`, which may cover
/// a little more than an anchored original did (the safe side for deny and ask). `Write`, `NotebookEdit` and `Glob`
/// rules never match anything, so they are written as the `Edit` and `Read` rules that do. Every other rule (Bash,
/// WebFetch, MCP) does not depend on where it is read and is copied as is.
pub fn rewrite(rule: &str, folder: &str) -> String {
    let Some((tool, spec)) = rule.strip_suffix(')').and_then(|r| r.split_once('(')) else {
        return rule.to_string();
    };
    let tool = match tool {
        "Read" | "Edit" => tool,
        "Write" | "NotebookEdit" => "Edit",
        "Glob" => "Read",
        _ => return rule.to_string(),
    };
    let (neg, path) = match spec.strip_prefix('!') {
        Some(p) => ("!", p),
        None => ("", spec),
    };
    let path = if path.starts_with("//") || path == "~" || path.starts_with("~/") {
        path.to_string()
    } else if let Some(p) = path.strip_prefix("./").or_else(|| path.strip_prefix('/')) {
        format!("{folder}/{p}")
    } else {
        format!("{folder}/**/{}", path.strip_prefix("**/").unwrap_or(path))
    };
    format!("{tool}({neg}{path})")
}
