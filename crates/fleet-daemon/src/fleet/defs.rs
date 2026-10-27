//! Orchestrator types (`orchestrators/<name>.md`), specialists (`specialists/<name>.md`) and skills
//! (`skills/<name>/SKILL.md`): Markdown with YAML frontmatter. The built-ins are embedded in the binary; a user file
//! of the same name under `Config.fleet_dir` takes their place until it is reset.
use crate::config::is_safe_dir_name;
use fleet_api::{DefFile, DefInfo, DefKind};
use rust_embed::RustEmbed;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/builtin"]
pub(crate) struct Builtin;

/// Claude Code permission rules the definition adds to the ones the daemon always sets.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToolRules {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

/// Stored and shown, not enforced this round.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrchestratorDef {
    pub name: String,
    pub summary: String,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub tools: ToolRules,
    /// Passed to `claude --model`; `None` leaves the choice to Claude's own default.
    #[serde(default)]
    pub model: Option<String>,
    /// The Markdown body after the frontmatter.
    #[serde(skip)]
    pub prompt: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecialistDef {
    pub name: String,
    pub summary: String,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub tools: ToolRules,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub limits: Limits,
    #[serde(skip)]
    pub prompt: String,
}

/// The two `SKILL.md` keys the daemon reads. Other keys pass: the format is shared with other tools.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct SkillDef {
    pub name: String,
    pub description: String,
}

/// Where a skill's files come from. A user `SKILL.md` is laid over the built-in folder of the same name, so the
/// built-in's reference files stay there for a `SKILL.md` that still links them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillSource {
    pub name: String,
    pub builtin: bool,
    pub user: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum FleetError {
    #[error("{file}: {reason}")]
    Invalid { file: String, reason: String },
    #[error("no {} named `{name}`", kind_word(*kind))]
    NotFound { kind: DefKind, name: String },
    #[error("{0}")]
    Builtin(String),
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

fn kind_word(kind: DefKind) -> &'static str {
    match kind {
        DefKind::Orchestrator => "orchestrator type",
        DefKind::Specialist => "specialist",
        DefKind::Skill => "skill",
    }
}

const KINDS: [DefKind; 3] = [DefKind::Orchestrator, DefKind::Specialist, DefKind::Skill];

/// Path of a definition relative to the built-in root or the user's `fleet/` folder; also how errors name it.
pub fn rel_path(kind: DefKind, name: &str) -> String {
    match kind {
        DefKind::Orchestrator => format!("orchestrators/{name}.md"),
        DefKind::Specialist => format!("specialists/{name}.md"),
        DefKind::Skill => format!("skills/{name}/SKILL.md"),
    }
}

fn invalid(file: &str, reason: impl Into<String>) -> FleetError {
    FleetError::Invalid {
        file: file.to_string(),
        reason: reason.into(),
    }
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> FleetError + '_ {
    move |source| FleetError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Splits `---\n<yaml>\n---\n<body>`; `None` when the text does not open with a frontmatter block.
fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some((&rest[..at], &rest[at + line.len()..]));
        }
        at += line.len();
    }
    None
}

/// A model is passed to `claude --model` as one argument, so it must be one word that cannot read as a flag.
pub fn valid_model(s: &str) -> bool {
    !s.is_empty() && !s.starts_with('-') && !s.contains(char::is_whitespace)
}

fn check_model(file: &str, model: &Option<String>) -> Result<(), FleetError> {
    match model {
        Some(m) if !valid_model(m) => Err(invalid(file, "model must be one word, e.g. sonnet")),
        _ => Ok(()),
    }
}

/// Top-level keys a kind accepts, each with the keys allowed under it (empty: not a table).
type KeySpec = &'static [(&'static str, &'static [&'static str])];

const ORCHESTRATOR_KEYS: KeySpec = &[
    ("name", &[]),
    ("summary", &[]),
    ("skills", &[]),
    ("tools", &["allow", "deny"]),
    ("model", &[]),
];
const SPECIALIST_KEYS: KeySpec = &[
    ("name", &[]),
    ("summary", &[]),
    ("skills", &[]),
    ("tools", &["allow", "deny"]),
    ("model", &[]),
    ("limits", &["paths"]),
];

/// Names the first unknown key as the user wrote it, which serde's own message does not do for nested tables.
fn check_keys(file: &str, value: &serde_yaml_ng::Value, spec: KeySpec) -> Result<(), FleetError> {
    let Some(map) = value.as_mapping() else {
        return Ok(());
    };
    for (k, v) in map {
        let Some(key) = k.as_str() else {
            return Err(invalid(file, "frontmatter keys must be text"));
        };
        let Some((_, nested)) = spec.iter().find(|(name, _)| *name == key) else {
            return Err(invalid(file, format!("unknown key `{key}`")));
        };
        if let (false, Some(inner)) = (nested.is_empty(), v.as_mapping()) {
            for k in inner.keys() {
                let sub = k.as_str().unwrap_or_default();
                if !nested.contains(&sub) {
                    return Err(invalid(file, format!("unknown key `{key}.{sub}`")));
                }
            }
        }
    }
    Ok(())
}

fn parse_front<T: DeserializeOwned>(
    file: &str,
    text: &str,
    keys: Option<KeySpec>,
) -> Result<(T, String), FleetError> {
    let (yaml, body) = split_frontmatter(text)
        .ok_or_else(|| invalid(file, "must start with a `---` frontmatter block"))?;
    let mut value: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(yaml).map_err(|e| invalid(file, yaml_error(&e)))?;
    if value.is_null() {
        value = serde_yaml_ng::Value::Mapping(Default::default());
    }
    if let Some(spec) = keys {
        check_keys(file, &value, spec)?;
    }
    let parsed = serde_yaml_ng::from_value(value).map_err(|e| invalid(file, e.to_string()))?;
    Ok((parsed, body.trim_start_matches(['\r', '\n']).to_string()))
}

/// The parser counts lines from the start of the frontmatter; the person editing counts from the `---` above it.
fn yaml_error(e: &serde_yaml_ng::Error) -> String {
    let text = e.to_string();
    match e.location() {
        Some(l) => text.replace(
            &format!("at line {} column", l.line()),
            &format!("at line {} column", l.line() + 1),
        ),
        None => text,
    }
}

fn check_name(file: &str, found: &str, want: &str) -> Result<(), FleetError> {
    if found != want {
        return Err(invalid(
            file,
            format!("name `{found}` does not match the file name `{want}`"),
        ));
    }
    Ok(())
}

fn check_text(file: &str, key: &str, value: &str) -> Result<(), FleetError> {
    if value.trim().is_empty() {
        return Err(invalid(file, format!("`{key}` is empty")));
    }
    Ok(())
}

pub struct Defs {
    #[allow(dead_code)]
    // Names the embedded tree; its functions are associated, so the value itself is unused.
    builtin: Builtin,
    user: PathBuf,
}

impl Defs {
    /// `user` is `Config.fleet_dir`; it need not exist until something is saved.
    pub fn new(user: PathBuf) -> Defs {
        Defs {
            builtin: Builtin,
            user,
        }
    }

    pub fn orchestrator(&self, name: &str) -> Result<OrchestratorDef, FleetError> {
        let (text, _) = self.text(DefKind::Orchestrator, name)?;
        self.parse_orchestrator(name, &text)
    }

    pub fn specialist(&self, name: &str) -> Result<SpecialistDef, FleetError> {
        let (text, _) = self.text(DefKind::Specialist, name)?;
        self.parse_specialist(name, &text)
    }

    /// A skill that exists and whose `SKILL.md` is valid.
    pub fn skill_dir(&self, name: &str) -> Result<SkillSource, FleetError> {
        let (text, _) = self.text(DefKind::Skill, name)?;
        parse_skill(name, &text)?;
        let user = self.user.join("skills").join(name);
        Ok(SkillSource {
            name: name.to_string(),
            builtin: Builtin::get(&rel_path(DefKind::Skill, name)).is_some(),
            user: user.join("SKILL.md").is_file().then_some(user),
        })
    }

    /// Every definition, built-in or user; one that does not load is listed with its error.
    pub fn list(&self) -> Vec<DefInfo> {
        let mut out = Vec::new();
        for kind in KINDS {
            let builtin = builtin_names(kind);
            let mut names = builtin.clone();
            names.extend(self.user_names(kind));
            names.sort();
            names.dedup();
            for name in names {
                let is_builtin = builtin.contains(&name);
                let user = self.user.join(rel_path(kind, &name)).is_file();
                let (summary, model, error) = match self.summary(kind, &name) {
                    Ok((s, m)) => (s, m, None),
                    Err(e) => (String::new(), None, Some(e.to_string())),
                };
                out.push(DefInfo {
                    kind,
                    name,
                    summary,
                    builtin: is_builtin,
                    overridden: is_builtin && user,
                    model,
                    error,
                });
            }
        }
        out
    }

    pub fn read(&self, kind: DefKind, name: &str) -> Result<DefFile, FleetError> {
        let (text, user) = self.text(kind, name)?;
        Ok(DefFile {
            kind,
            name: name.to_string(),
            text,
            builtin: !user,
        })
    }

    /// Validates `text` as the file it would become, then saves it as the user's copy. For a skill, only `SKILL.md`.
    pub fn write(&self, kind: DefKind, name: &str, text: &str) -> Result<DefInfo, FleetError> {
        self.check_name_safe(kind, name)?;
        match kind {
            DefKind::Orchestrator => {
                self.parse_orchestrator(name, text)?;
            }
            DefKind::Specialist => {
                self.parse_specialist(name, text)?;
            }
            DefKind::Skill => {
                parse_skill(name, text)?;
            }
        }
        let path = self.user.join(rel_path(kind, name));
        let dir = path.parent().expect("a definition path has a parent");
        std::fs::create_dir_all(dir).map_err(io(dir))?;
        // Through a temporary file, so a crash never leaves half a definition that would then fail to load.
        let tmp = path.with_extension("md.tmp");
        std::fs::write(&tmp, text).map_err(io(&tmp))?;
        std::fs::rename(&tmp, &path).map_err(io(&path))?;
        let builtin = Builtin::get(&rel_path(kind, name)).is_some();
        let (summary, model) = self.summary(kind, name)?;
        Ok(DefInfo {
            kind,
            name: name.to_string(),
            summary,
            builtin,
            overridden: builtin,
            model,
            error: None,
        })
    }

    /// Deletes the user's copy, so the built-in (if any) applies again.
    pub fn reset(&self, kind: DefKind, name: &str) -> Result<(), FleetError> {
        self.check_name_safe(kind, name)?;
        let rel = rel_path(kind, name);
        let path = self.user.join(&rel);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(if Builtin::get(&rel).is_some() {
                    FleetError::Builtin(format!("{rel} is built in and has no user copy to reset"))
                } else {
                    FleetError::NotFound {
                        kind,
                        name: name.to_string(),
                    }
                });
            }
            Err(e) => return Err(io(&path)(e)),
        }
        if kind == DefKind::Skill {
            // Only `SKILL.md` made it a user skill; files the user put beside it stay unless the folder is empty.
            let _ = std::fs::remove_dir(path.parent().expect("skill folder"));
        }
        Ok(())
    }

    fn check_name_safe(&self, kind: DefKind, name: &str) -> Result<(), FleetError> {
        if !is_safe_dir_name(name) {
            return Err(invalid(
                &rel_path(kind, name),
                "the name must be letters, digits, '.', '_' or '-' and not start with '.' or '-'",
            ));
        }
        Ok(())
    }

    /// The user's text if there is one, else the built-in's; the flag says it is the user's.
    fn text(&self, kind: DefKind, name: &str) -> Result<(String, bool), FleetError> {
        self.check_name_safe(kind, name)?;
        let rel = rel_path(kind, name);
        let path = self.user.join(&rel);
        match std::fs::read_to_string(&path) {
            Ok(t) => return Ok((t, true)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(&path)(e)),
        }
        let f = Builtin::get(&rel).ok_or_else(|| FleetError::NotFound {
            kind,
            name: name.to_string(),
        })?;
        let text = String::from_utf8(f.data.into_owned())
            .map_err(|_| invalid(&rel, "the built-in is not UTF-8"))?;
        Ok((text, false))
    }

    /// The one-line summary and, for an orchestrator or specialist, its model.
    fn summary(&self, kind: DefKind, name: &str) -> Result<(String, Option<String>), FleetError> {
        Ok(match kind {
            DefKind::Orchestrator => {
                let d = self.orchestrator(name)?;
                (d.summary, d.model)
            }
            DefKind::Specialist => {
                let d = self.specialist(name)?;
                (d.summary, d.model)
            }
            DefKind::Skill => {
                let (text, _) = self.text(kind, name)?;
                (parse_skill(name, &text)?.description, None)
            }
        })
    }

    fn parse_orchestrator(&self, name: &str, text: &str) -> Result<OrchestratorDef, FleetError> {
        let file = rel_path(DefKind::Orchestrator, name);
        let (mut d, prompt): (OrchestratorDef, String) =
            parse_front(&file, text, Some(ORCHESTRATOR_KEYS))?;
        check_name(&file, &d.name, name)?;
        check_text(&file, "summary", &d.summary)?;
        self.check_skills(&file, &d.skills)?;
        check_model(&file, &d.model)?;
        d.prompt = prompt;
        Ok(d)
    }

    fn parse_specialist(&self, name: &str, text: &str) -> Result<SpecialistDef, FleetError> {
        let file = rel_path(DefKind::Specialist, name);
        let (mut d, prompt): (SpecialistDef, String) =
            parse_front(&file, text, Some(SPECIALIST_KEYS))?;
        check_name(&file, &d.name, name)?;
        check_text(&file, "summary", &d.summary)?;
        self.check_skills(&file, &d.skills)?;
        check_model(&file, &d.model)?;
        d.prompt = prompt;
        Ok(d)
    }

    fn check_skills(&self, file: &str, skills: &[String]) -> Result<(), FleetError> {
        for s in skills {
            let found = is_safe_dir_name(s)
                && (Builtin::get(&rel_path(DefKind::Skill, s)).is_some()
                    || self.user.join(rel_path(DefKind::Skill, s)).is_file());
            if !found {
                return Err(invalid(file, format!("skill `{s}` not found")));
            }
        }
        Ok(())
    }

    fn user_names(&self, kind: DefKind) -> Vec<String> {
        let dir = self.user.join(match kind {
            DefKind::Orchestrator => "orchestrators",
            DefKind::Specialist => "specialists",
            DefKind::Skill => "skills",
        });
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|e| {
                let file = e.file_name().into_string().ok()?;
                let name = match kind {
                    DefKind::Skill => file,
                    _ => file.strip_suffix(".md")?.to_string(),
                };
                let ok = is_safe_dir_name(&name) && self.user.join(rel_path(kind, &name)).is_file();
                ok.then_some(name)
            })
            .collect()
    }
}

fn parse_skill(name: &str, text: &str) -> Result<SkillDef, FleetError> {
    let file = rel_path(DefKind::Skill, name);
    let (d, _): (SkillDef, String) = parse_front(&file, text, None)?;
    check_name(&file, &d.name, name)?;
    check_text(&file, "description", &d.description)?;
    Ok(d)
}

fn builtin_names(kind: DefKind) -> Vec<String> {
    Builtin::iter()
        .filter_map(|p| {
            let name = match kind {
                DefKind::Orchestrator => p.strip_prefix("orchestrators/")?.strip_suffix(".md")?,
                DefKind::Specialist => p.strip_prefix("specialists/")?.strip_suffix(".md")?,
                DefKind::Skill => p.strip_prefix("skills/")?.strip_suffix("/SKILL.md")?,
            };
            (!name.contains('/')).then(|| name.to_string())
        })
        .collect()
}
