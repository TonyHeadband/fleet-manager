use fleet_api::DefKind;
use fleet_daemon::fleet::defs::{Defs, FleetError};
use fleet_daemon::fleet::skills;
use std::path::Path;

const SKILLS: [&str; 6] = [
    "brainstorming",
    "writing-plans",
    "test-driven-development",
    "systematic-debugging",
    "verification-before-completion",
    "receiving-code-review",
];

fn defs() -> (tempfile::TempDir, Defs) {
    let tmp = tempfile::tempdir().unwrap();
    let d = Defs::new(tmp.path().join("fleet"));
    (tmp, d)
}

fn names(d: &Defs, kind: DefKind) -> Vec<String> {
    let mut v: Vec<String> = d
        .list()
        .into_iter()
        .filter(|i| i.kind == kind)
        .map(|i| i.name)
        .collect();
    v.sort();
    v
}

fn rust_with(frontmatter: &str) -> String {
    format!("---\n{frontmatter}---\nYou write Rust.\n")
}

#[test]
fn builtins_load_and_validate() {
    let (_t, d) = defs();
    let list = d.list();
    let broken: Vec<_> = list.iter().filter(|i| i.error.is_some()).collect();
    assert!(broken.is_empty(), "{broken:?}");
    assert!(list.iter().all(|i| i.builtin && !i.overridden));
    assert!(
        list.iter().all(|i| !i.summary.trim().is_empty()),
        "{list:?}"
    );
    assert_eq!(names(&d, DefKind::Orchestrator), ["direct", "structured"]);
    assert_eq!(
        names(&d, DefKind::Specialist),
        ["reviewer", "rust", "test-writer", "typescript-react"]
    );
    let mut want: Vec<&str> = SKILLS.to_vec();
    want.sort();
    assert_eq!(names(&d, DefKind::Skill), want);

    assert_eq!(
        d.orchestrator("direct").unwrap().skills,
        Vec::<String>::new()
    );
    assert_eq!(
        d.orchestrator("structured").unwrap().skills,
        ["brainstorming", "writing-plans"]
    );
    for code in ["rust", "typescript-react", "test-writer"] {
        assert_eq!(
            d.specialist(code).unwrap().skills,
            [
                "test-driven-development",
                "systematic-debugging",
                "verification-before-completion"
            ],
            "{code}"
        );
    }
    assert_eq!(
        d.specialist("reviewer").unwrap().skills,
        ["receiving-code-review", "verification-before-completion"]
    );
    // The daemon adds the fixed denies and the MCP allowlist; the files carry none of their own.
    let o = d.orchestrator("direct").unwrap();
    assert!(o.tools.allow.is_empty() && o.tools.deny.is_empty());
    assert!(o.prompt.contains("propose_breakdown") && o.prompt.contains("open_pr"));
    assert!(d.specialist("rust").unwrap().prompt.contains("finish_step"));
}

#[test]
fn user_file_overrides_builtin() {
    let (_t, d) = defs();
    let builtin = d.specialist("rust").unwrap().summary;
    let text = rust_with("name: rust\nsummary: Mine now\nskills: [systematic-debugging]\n");
    let info = d.write(DefKind::Specialist, "rust", &text).unwrap();
    assert!(
        info.builtin && info.overridden && info.error.is_none(),
        "{info:?}"
    );
    assert_eq!(d.specialist("rust").unwrap().summary, "Mine now");
    assert_eq!(d.specialist("rust").unwrap().prompt, "You write Rust.\n");
    let f = d.read(DefKind::Specialist, "rust").unwrap();
    assert_eq!((f.text.as_str(), f.builtin), (text.as_str(), false));
    let listed = d
        .list()
        .into_iter()
        .find(|i| i.kind == DefKind::Specialist && i.name == "rust")
        .unwrap();
    assert!(listed.overridden);

    d.reset(DefKind::Specialist, "rust").unwrap();
    assert_eq!(d.specialist("rust").unwrap().summary, builtin);
    assert!(d.read(DefKind::Specialist, "rust").unwrap().builtin);
}

#[test]
fn unknown_key_is_refused() {
    let (_t, d) = defs();
    let text = rust_with("name: rust\nsummary: s\nlimit:\n  paths: [src]\n");
    let err = d.write(DefKind::Specialist, "rust", &text).unwrap_err();
    assert!(matches!(err, FleetError::Invalid { .. }), "{err:?}");
    assert_eq!(err.to_string(), "specialists/rust.md: unknown key `limit`");
    // Nested keys are checked too: a typo under `tools` must not pass silently either.
    let text = rust_with("name: rust\nsummary: s\ntools:\n  alow: [Read]\n");
    let err = d.write(DefKind::Specialist, "rust", &text).unwrap_err();
    assert!(err.to_string().contains("`tools.alow`"), "{err}");
    // Nothing was written: the built-in still applies.
    assert!(!d
        .read(DefKind::Specialist, "rust")
        .unwrap()
        .text
        .contains("limit:"));
}

#[test]
fn a_broken_user_file_is_listed_with_its_error() {
    let (t, d) = defs();
    let dir = t.path().join("fleet/specialists");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("rust.md"),
        rust_with("name: rust\nsummary: s\nlimit: {}\n"),
    )
    .unwrap();
    let info = d
        .list()
        .into_iter()
        .find(|i| i.kind == DefKind::Specialist && i.name == "rust")
        .unwrap();
    assert!(info.overridden);
    assert!(info.error.as_deref().unwrap().contains("limit"), "{info:?}");
    assert!(matches!(
        d.specialist("rust"),
        Err(FleetError::Invalid { .. })
    ));
}

#[test]
fn a_yaml_error_names_the_line_in_the_file() {
    let (_t, d) = defs();
    let err = d
        .write(
            DefKind::Specialist,
            "rust",
            &rust_with("name: rust\nsummary: a: b\n"),
        )
        .unwrap_err();
    assert!(err.to_string().contains("at line 3 column"), "{err}");
}

#[test]
fn missing_skill_is_refused() {
    let (_t, d) = defs();
    let text = rust_with("name: rust\nsummary: s\nskills: [nope]\n");
    let err = d.write(DefKind::Specialist, "rust", &text).unwrap_err();
    assert_eq!(
        err.to_string(),
        "specialists/rust.md: skill `nope` not found"
    );
}

#[test]
fn name_must_match_file() {
    let (_t, d) = defs();
    let err = d
        .write(
            DefKind::Specialist,
            "rust",
            &rust_with("name: other\nsummary: s\n"),
        )
        .unwrap_err();
    assert!(matches!(err, FleetError::Invalid { .. }), "{err:?}");
    assert!(
        err.to_string().starts_with("specialists/rust.md: "),
        "{err}"
    );
}

#[test]
fn text_without_frontmatter_is_refused() {
    let (_t, d) = defs();
    let err = d
        .write(DefKind::Orchestrator, "mine", "just a prompt\n")
        .unwrap_err();
    assert!(matches!(err, FleetError::Invalid { .. }), "{err:?}");
}

#[test]
fn a_name_that_is_not_a_plain_file_name_is_refused() {
    let (_t, d) = defs();
    for name in ["../x", "a/b", "", ".hidden"] {
        let text = rust_with(&format!("name: {name:?}\nsummary: s\n"));
        assert!(
            matches!(
                d.write(DefKind::Specialist, name, &text),
                Err(FleetError::Invalid { .. })
            ),
            "{name:?}"
        );
    }
}

#[test]
fn user_only_definitions_and_skills_round_trip() {
    let (t, d) = defs();
    let skill = "---\nname: egui\ndescription: Use for egui work\nlicense: MIT\n---\nBody\n";
    let info = d.write(DefKind::Skill, "egui", skill).unwrap();
    assert!(!info.builtin && !info.overridden);
    assert_eq!(info.summary, "Use for egui work");
    assert!(t.path().join("fleet/skills/egui/SKILL.md").is_file());

    let text = "---\nname: egui-gui\nsummary: egui screens\nskills: [egui]\nlimits:\n  paths: [\"crates/fleet-gui/**\"]\n---\nYou build egui.\n";
    d.write(DefKind::Specialist, "egui-gui", text).unwrap();
    let s = d.specialist("egui-gui").unwrap();
    assert_eq!(s.limits.paths, ["crates/fleet-gui/**"]);
    assert_eq!(s.skills, ["egui"]);

    d.reset(DefKind::Specialist, "egui-gui").unwrap();
    assert!(matches!(
        d.specialist("egui-gui"),
        Err(FleetError::NotFound { .. })
    ));
}

#[test]
fn a_skill_must_name_its_folder() {
    let (_t, d) = defs();
    let err = d
        .write(
            DefKind::Skill,
            "egui",
            "---\nname: other\ndescription: d\n---\n",
        )
        .unwrap_err();
    assert!(
        err.to_string().starts_with("skills/egui/SKILL.md: "),
        "{err}"
    );
}

#[test]
fn reset_without_a_user_file_is_refused() {
    let (_t, d) = defs();
    assert!(matches!(
        d.reset(DefKind::Specialist, "rust"),
        Err(FleetError::Builtin(_))
    ));
    assert!(matches!(
        d.reset(DefKind::Specialist, "nope"),
        Err(FleetError::NotFound { .. })
    ));
}

#[test]
fn install_copies_only_listed_skills() {
    let (t, d) = defs();
    let agent = t.path().join("agent");
    let skills_dir = agent.join(".claude/skills");
    std::fs::create_dir_all(skills_dir.join("test-driven-development")).unwrap();
    std::fs::write(skills_dir.join("test-driven-development/stale.md"), "old").unwrap();

    skills::install(&d, &["test-driven-development".to_string()], &agent).unwrap();
    let tdd = skills_dir.join("test-driven-development");
    assert!(tdd.join("SKILL.md").is_file());
    assert!(tdd.join("writing-good-tests.md").is_file());
    assert!(!tdd.join("stale.md").exists(), "replaces what was there");
    let present: Vec<_> = std::fs::read_dir(&skills_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(present, ["test-driven-development"]);

    assert!(skills::install(&d, &["nope".to_string()], &agent).is_err());
}

#[test]
fn install_keeps_scripts_runnable_and_layers_a_user_skill() {
    let (t, d) = defs();
    let agent = t.path().join("agent");
    let mine = "---\nname: systematic-debugging\ndescription: mine\n---\nMine\n";
    d.write(DefKind::Skill, "systematic-debugging", mine)
        .unwrap();
    skills::install(&d, &["systematic-debugging".to_string()], &agent).unwrap();
    let dir = agent.join(".claude/skills/systematic-debugging");
    assert_eq!(std::fs::read_to_string(dir.join("SKILL.md")).unwrap(), mine);
    // The built-in's reference files stay available to a user's SKILL.md that still links them.
    assert!(dir.join("root-cause-tracing.md").is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join("find-polluter.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert!(mode & 0o100 != 0, "{mode:o}");
    }
}

fn builtin_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            builtin_files(&p, out);
        } else {
            out.push(p);
        }
    }
}

#[test]
fn no_builtin_mentions_subagents() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("builtin");
    let mut files = Vec::new();
    builtin_files(&root, &mut files);
    assert!(files.len() > 10, "{files:?}");
    for f in files {
        let name = f.file_name().unwrap().to_string_lossy();
        if name == "NOTICE.md" || name == "LICENSE" {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap().to_lowercase();
        for word in ["subagent", "agent tool", "todowrite", "dispatch"] {
            assert!(!text.contains(word), "{} mentions {word:?}", f.display());
        }
    }
}
