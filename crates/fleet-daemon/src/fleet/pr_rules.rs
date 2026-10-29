//! The PR rule check: a title, body and diff facts against a style's rules. Pure; `open_pr` runs it before the
//! workflow steps.
use std::fmt;

use fleet_api::{PrRules, Versioning};
use regex::Regex;

/// What the check needs to know about a PR.
pub struct Change<'a> {
    pub title: &'a str,
    pub body: &'a str,
    /// Paths in the diff that match the repo's `breaking` globs.
    pub breaking_touched: Vec<String>,
    pub release_notes_touched: bool,
    /// The number of the step the breakdown marked `release_note: true`, if any.
    pub note_planned: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Miss {
    /// The title pattern it did not match.
    Title(String),
    /// The missing section's name.
    Section(String),
    /// The breaking paths touched and the line to add.
    VersionNote(String),
    Attribution,
    /// The step that planned the release note and the release-notes file the diff left untouched.
    ReleaseNote(u32, String),
}

impl fmt::Display for Miss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Miss::Title(pattern) => write!(f, "title does not match `{pattern}`"),
            Miss::Section(name) => write!(f, "no `## {name}` section"),
            Miss::VersionNote(what) => write!(f, "breaking path {what}"),
            Miss::Attribution => write!(f, "the body does not end with the attribution line"),
            Miss::ReleaseNote(step, file) => {
                write!(
                    f,
                    "step {step} planned a release note but {file} is unchanged"
                )
            }
        }
    }
}

impl Miss {
    /// Whether the orchestrator fixes it by rewriting the title or body; a missing release note needs a fix step.
    pub fn self_fixable(&self) -> bool {
        !matches!(self, Miss::ReleaseNote(..))
    }
}

/// The word a breaking change takes under a scheme.
pub fn bump_word(v: Versioning) -> Option<&'static str> {
    match v {
        Versioning::Semver => Some("major"),
        Versioning::SemverPre1 => Some("minor"),
        Versioning::None => None,
    }
}

/// Every rule the change misses, in the order title, sections, version note, attribution, release note.
pub fn check(rules: &PrRules, c: &Change) -> Vec<Miss> {
    let mut misses = Vec::new();

    // The pattern was validated when the style loaded; one that still fails to compile is treated as no rule.
    if let Some(pattern) = &rules.title {
        if let Ok(re) = Regex::new(pattern) {
            let title = c.title.trim();
            if title.contains('\n') || !re.is_match(title) {
                misses.push(Miss::Title(pattern.clone()));
            }
        }
    }

    for section in &rules.sections {
        if !has_section(c.body, section) {
            misses.push(Miss::Section(section.clone()));
        }
    }

    if let Some(word) = bump_word(rules.versioning) {
        if !c.breaking_touched.is_empty() && !has_version_note(c.body) {
            misses.push(Miss::VersionNote(format!(
                "{} changed: add `Version: {word} — <why>`",
                c.breaking_touched.join(", ")
            )));
        }
    }

    if !ends_with_attribution(c.body, &rules.attribution) {
        misses.push(Miss::Attribution);
    }

    if let Some(file) = &rules.release_notes {
        if let Some(step) = c.note_planned.filter(|_| !c.release_notes_touched) {
            misses.push(Miss::ReleaseNote(step, file.clone()));
        }
    }

    misses
}

/// A markdown heading of any level with this name, case-insensitive.
fn has_section(body: &str, name: &str) -> bool {
    let name = name.trim();
    body.lines().any(|line| {
        let line = line.trim();
        let text = line.trim_start_matches('#');
        let level = line.len() - text.len();
        (1..=6).contains(&level)
            && text.starts_with(char::is_whitespace)
            && text.trim().eq_ignore_ascii_case(name)
    })
}

/// `Version: <major|minor|patch> — <why>`; `-` stands in for the em dash. The word is not judged.
fn has_version_note(body: &str) -> bool {
    let re =
        Regex::new(r"(?m)^\s*Version:\s*(major|minor|patch)\s+[—-]\s+\S").expect("static pattern");
    re.is_match(body)
}

/// The body's last non-blank lines are the attribution's lines; an empty attribution is no rule.
fn ends_with_attribution(body: &str, attribution: &str) -> bool {
    let lines = |s: &str| -> Vec<String> {
        s.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    };
    let wanted = lines(attribution);
    let have = lines(body);
    have.len() >= wanted.len() && have[have.len() - wanted.len()..] == wanted[..]
}

#[cfg(test)]
mod tests {
    use super::*;

    const ATTR: &str = "🤖 Generated with [Claude Code](https://claude.com/claude-code)";

    fn repo_rules() -> PrRules {
        PrRules {
            style: "conventional-semver-pre1.0".into(),
            title: Some(r"^(feat|fix|docs|chore|refactor|test)(\([a-z0-9-]+\))?!?: .+".into()),
            sections: vec!["Summary".into(), "Test plan".into()],
            versioning: Versioning::SemverPre1,
            release_notes: Some("packaging/changes/unreleased.md".into()),
            breaking: vec!["crates/fleet-daemon/migrations/**".into()],
            attribution: ATTR.into(),
            ..PrRules::default()
        }
    }

    fn body() -> String {
        format!("## Summary\n\nDoes a thing.\n\n## Test plan\n\n- cargo test\n\n{ATTR}\n")
    }

    fn change<'a>(title: &'a str, body: &'a str) -> Change<'a> {
        Change {
            title,
            body,
            breaking_touched: vec![],
            release_notes_touched: false,
            note_planned: None,
        }
    }

    #[test]
    fn everything_passes() {
        let b = body();
        assert_eq!(
            check(&repo_rules(), &change("feat(daemon): check a PR", &b)),
            vec![]
        );
    }

    #[test]
    fn title_miss() {
        let b = body();
        let m = check(&repo_rules(), &change("Add stuff", &b));
        assert!(matches!(m.as_slice(), [Miss::Title(_)]));
        assert!(m[0].to_string().starts_with("title does not match"));
    }

    #[test]
    fn title_is_trimmed_and_one_line() {
        let b = body();
        let r = repo_rules();
        assert!(check(&r, &change("  feat: x \n", &b)).is_empty());
        assert!(matches!(
            check(&r, &change("feat: x\nsecond line", &b)).as_slice(),
            [Miss::Title(_)]
        ));
    }

    #[test]
    fn section_miss() {
        let b = format!("## Summary\n\nx\n\n{ATTR}");
        let m = check(&repo_rules(), &change("feat: x", &b));
        assert_eq!(m, vec![Miss::Section("Test plan".into())]);
        assert_eq!(m[0].to_string(), "no `## Test plan` section");
    }

    #[test]
    fn sections_any_level_any_case() {
        let b = format!("# summary\n\nx\n\n### TEST PLAN\n\ny\n\n{ATTR}");
        assert!(check(&repo_rules(), &change("feat: x", &b)).is_empty());
        let b = format!("#Summary\nSummary\n## Test plan extra\n\n{ATTR}");
        assert_eq!(check(&repo_rules(), &change("feat: x", &b)).len(), 2);
    }

    #[test]
    fn version_note_miss() {
        let b = body();
        let mut c = change("feat: x", &b);
        c.breaking_touched = vec!["crates/fleet-daemon/migrations/011_pr.sql".into()];
        let m = check(&repo_rules(), &c);
        assert!(matches!(m.as_slice(), [Miss::VersionNote(_)]));
        assert_eq!(
            m[0].to_string(),
            "breaking path crates/fleet-daemon/migrations/011_pr.sql changed: add `Version: minor — <why>`"
        );
    }

    #[test]
    fn version_note_only_when_a_breaking_path_changed() {
        let r = repo_rules();
        let plain = body();
        assert!(check(&r, &change("feat: x", &plain)).is_empty());
        let with = |line: &str| format!("## Summary\n\nx\n\n## Test plan\n\ny\n\n{line}\n\n{ATTR}");
        for (line, ok) in [
            ("Version: minor — migration 011", true),
            ("Version: patch - x", true),
            ("Version: big", false),
            ("Version: minor", false),
        ] {
            let b = with(line);
            let mut c = change("feat: x", &b);
            c.breaking_touched = vec!["a".into()];
            assert_eq!(check(&r, &c).is_empty(), ok, "{line}");
        }
    }

    #[test]
    fn versioning_none_never_asks_for_a_note() {
        let mut r = repo_rules();
        r.versioning = Versioning::None;
        let b = body();
        let mut c = change("feat: x", &b);
        c.breaking_touched = vec!["a".into()];
        assert!(check(&r, &c).is_empty());
    }

    #[test]
    fn attribution_is_the_last_non_blank_line() {
        let r = repo_rules();
        let b = format!("## Summary\n\nx\n\n## Test plan\n\ny\n\n{ATTR}\n\n\n");
        assert!(check(&r, &change("feat: x", &b)).is_empty());
        let b = format!("{ATTR}\n\n## Summary\n\nx\n\n## Test plan\n\ny\n");
        assert_eq!(check(&r, &change("feat: x", &b)), vec![Miss::Attribution]);
    }

    #[test]
    fn release_note_miss() {
        let b = body();
        let r = repo_rules();
        let mut c = change("feat: x", &b);
        c.note_planned = Some(3);
        let m = check(&r, &c);
        assert_eq!(
            m,
            vec![Miss::ReleaseNote(
                3,
                "packaging/changes/unreleased.md".into()
            )]
        );
        assert_eq!(
            m[0].to_string(),
            "step 3 planned a release note but packaging/changes/unreleased.md is unchanged"
        );
        assert!(!m[0].self_fixable());
        assert!(Miss::Attribution.self_fixable());
        c.release_notes_touched = true;
        assert!(check(&r, &c).is_empty());
        c.release_notes_touched = false;
        c.note_planned = None;
        assert!(check(&r, &c).is_empty());
    }

    #[test]
    fn bump_words() {
        assert_eq!(bump_word(Versioning::Semver), Some("major"));
        assert_eq!(bump_word(Versioning::SemverPre1), Some("minor"));
        assert_eq!(bump_word(Versioning::None), None);
    }

    #[test]
    fn empty_rules_pass_anything() {
        let mut c = change("whatever", "");
        c.breaking_touched = vec!["a".into()];
        c.note_planned = Some(1);
        assert!(check(&PrRules::default(), &c).is_empty());
    }
}
