use fleet_api::{DefKind, Effort, PermissionMode, Versioning};
use fleet_daemon::fleet::defs::{rel_path, Defs, FleetError};
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
        [
            "docs",
            "reviewer",
            "rust",
            "test-writer",
            "typescript-react"
        ]
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
    assert_eq!(
        d.specialist("docs").unwrap().skills,
        ["verification-before-completion"]
    );
    // The daemon adds the fixed denies and the MCP allowlist; the files carry none of their own.
    let o = d.orchestrator("direct").unwrap();
    assert!(o.tools.allow.is_empty() && o.tools.deny.is_empty());
    assert!(o.prompt.contains("propose_breakdown") && o.prompt.contains("open_pr"));
    assert!(d.specialist("rust").unwrap().prompt.contains("finish_step"));

    for name in ["direct", "structured"] {
        assert_eq!(d.orchestrator(name).unwrap().model.as_deref(), Some("opus"));
    }
    assert_eq!(
        d.specialist("reviewer").unwrap().model.as_deref(),
        Some("opus")
    );
    for name in ["rust", "typescript-react", "test-writer", "docs"] {
        assert_eq!(
            d.specialist(name).unwrap().model.as_deref(),
            Some("sonnet"),
            "{name}"
        );
    }
    let info = |name: &str| {
        d.list()
            .into_iter()
            .find(|i| i.kind == DefKind::Specialist && i.name == name)
            .unwrap()
    };
    assert_eq!(info("rust").model.as_deref(), Some("sonnet"));
}

#[test]
fn model_must_be_one_token() {
    let (t, d) = defs();
    for bad in [
        "sonnet --x",
        "-x",
        "",
        "opus;",
        "opus\\u0001",
        "opus\\u0000",
    ] {
        let text = rust_with(&format!("name: rust\nsummary: s\nmodel: \"{bad}\"\n"));
        let err = d.write(DefKind::Specialist, "rust", &text).unwrap_err();
        assert!(
            matches!(err, FleetError::Invalid { .. }) && err.to_string().contains("model"),
            "{bad:?}: {err}"
        );
    }
    // A broken user file is listed with its error rather than hiding the definition.
    let path = t.path().join("fleet/specialists/mine.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "---\nname: mine\nsummary: s\nmodel: \"sonnet --x\"\n---\nBody\n",
    )
    .unwrap();
    let listed = d.list().into_iter().find(|i| i.name == "mine").unwrap();
    assert!(listed.error.unwrap().contains("model"));

    let text = rust_with("name: rust\nsummary: s\n");
    d.write(DefKind::Specialist, "rust", &text).unwrap();
    assert_eq!(d.specialist("rust").unwrap().model, None);
    let text = rust_with("name: rust\nsummary: s\nmodel: claude-opus-4-1\n");
    let info = d.write(DefKind::Specialist, "rust", &text).unwrap();
    assert_eq!(info.model.as_deref(), Some("claude-opus-4-1"));
}

#[test]
fn builtins_carry_their_mode_and_effort() {
    let (_t, d) = defs();
    let list = d.list();
    let table = [
        (DefKind::Orchestrator, "structured", Effort::High),
        (DefKind::Orchestrator, "direct", Effort::High),
        (DefKind::Specialist, "reviewer", Effort::High),
        (DefKind::Specialist, "rust", Effort::Medium),
        (DefKind::Specialist, "typescript-react", Effort::Medium),
        (DefKind::Specialist, "test-writer", Effort::Medium),
        (DefKind::Specialist, "docs", Effort::Low),
    ];
    for (kind, name, effort) in table {
        let info = list
            .iter()
            .find(|i| i.kind == kind && i.name == name)
            .unwrap();
        assert_eq!(info.mode, Some(PermissionMode::Auto), "{name}");
        assert_eq!(info.effort, Some(effort), "{name}");
    }
}

#[test]
fn a_user_file_sets_mode_and_effort() {
    let (_t, d) = defs();
    let text = rust_with("name: rust\nsummary: s\nmode: dontAsk\neffort: xhigh\n");
    let info = d.write(DefKind::Specialist, "rust", &text).unwrap();
    assert_eq!(info.mode, Some(PermissionMode::DontAsk));
    assert_eq!(info.effort, Some(Effort::Xhigh));
    let def = d.specialist("rust").unwrap();
    assert_eq!(
        (def.mode, def.effort),
        (Some(PermissionMode::DontAsk), Some(Effort::Xhigh))
    );

    let text = "---\nname: direct\nsummary: s\nmode: manual\neffort: max\n---\nBody\n";
    d.write(DefKind::Orchestrator, "direct", text).unwrap();
    let def = d.orchestrator("direct").unwrap();
    assert_eq!(
        (def.mode, def.effort),
        (Some(PermissionMode::Manual), Some(Effort::Max))
    );

    // Without the keys the definition takes the global defaults later: nothing is filled in here.
    let text = rust_with("name: rust\nsummary: s\n");
    let info = d.write(DefKind::Specialist, "rust", &text).unwrap();
    assert_eq!((info.mode, info.effort), (None, None));
}

#[test]
fn a_definition_with_plan_mode_is_invalid_not_fatal() {
    let (t, d) = defs();
    let dir = t.path().join("fleet/specialists");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("x.md"),
        "---\nname: x\nsummary: s\nmode: plan\n---\nBody\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("y.md"),
        "---\nname: y\nsummary: s\neffort: huge\n---\nBody\n",
    )
    .unwrap();
    let list = d.list();
    let x = list.iter().find(|i| i.name == "x").unwrap();
    assert!(x.error.as_deref().unwrap().contains("not allowed"), "{x:?}");
    let y = list.iter().find(|i| i.name == "y").unwrap();
    assert!(y.error.as_deref().unwrap().contains("not allowed"), "{y:?}");
    let ok = list.iter().find(|i| i.name == "rust").unwrap();
    assert!(ok.error.is_none() && ok.mode == Some(PermissionMode::Auto));
    let err = d
        .write(
            DefKind::Specialist,
            "z",
            "---\nname: z\nsummary: s\nmode: bypassPermissions\n---\nB\n",
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("mode bypassPermissions is not allowed"),
        "{err}"
    );
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

#[test]
fn builtins_carry_the_limit_rules() {
    let (_tmp, d) = defs();
    for name in ["direct", "structured"] {
        let prompt = d.orchestrator(name).unwrap().prompt;
        assert!(prompt.contains("limit reached"), "{name}");
        assert!(prompt.contains("wait_for_fleet"), "{name}");
    }
}

#[test]
fn builtin_orchestrators_mention_pr_rules() {
    let (_tmp, d) = defs();
    for name in ["direct", "structured"] {
        let prompt = d.orchestrator(name).unwrap().prompt;
        for word in ["Pull requests", "release_note", "open_pr"] {
            assert!(prompt.contains(word), "{name} lacks {word}");
        }
    }
}

#[test]
fn builtin_texts_write_designs_to_internal_resources() {
    let (_t, d) = defs();
    // Collapse line wraps so phrases can be matched whole.
    let text = |kind, name| {
        let t = d.read(kind, name).unwrap().text;
        t.split_whitespace().collect::<Vec<_>>().join(" ")
    };
    let structured = text(DefKind::Orchestrator, "structured");
    for want in [
        "tasks/<task-name>/design.md",
        "tasks/<task-name>/plan.md",
        "git -C <path> add -A && git -C <path> commit -m",
        "Internal resources",
        "`<task-name>` is the task folder named in `CLAUDE.md`",
        "relative to the internal resources folder",
    ] {
        assert!(structured.contains(want), "structured lacks {want:?}");
    }
    assert!(!structured.contains("Commit it on the task branch"));
    for name in ["brainstorming", "writing-plans"] {
        let t = text(DefKind::Skill, name);
        assert!(
            t.contains("internal resources folder")
                && t.contains("tasks/<task-name>/")
                && t.contains("`<task-name>` is the task folder named in `CLAUDE.md`")
                && t.contains("git -C <path> add -A && git -C <path> commit -m"),
            "{name} lacks the internal resources default"
        );
    }
    let plans = text(DefKind::Skill, "writing-plans");
    assert!(plans.contains("commit before you propose"));
    assert!(!plans.contains("each specialist's branch is cut from"));
}

fn pr_style_with(frontmatter: &str) -> String {
    format!("---\n{frontmatter}---\nHow to write the pull request.\n")
}

#[test]
fn builtin_pr_styles_load() {
    let (_t, d) = defs();
    assert_eq!(
        names(&d, DefKind::PrStyle),
        [
            "conventional",
            "conventional-semver",
            "conventional-semver-pre1.0",
            "minimal",
            "plain"
        ]
    );
    for name in [
        "conventional-semver-pre1.0",
        "conventional-semver",
        "conventional",
        "minimal",
        "plain",
    ] {
        let s = d.pr_style(name).unwrap();
        assert_eq!(s.name, name);
        assert!(
            !s.summary.trim().is_empty() && !s.prose.trim().is_empty(),
            "{name}"
        );
    }
    let s = d.pr_style("conventional-semver-pre1.0").unwrap();
    assert_eq!(s.versioning, Versioning::SemverPre1);
    assert_eq!(s.sections, ["Summary", "Test plan"]);
    let title = regex::Regex::new(s.title.as_deref().unwrap()).unwrap();
    assert!(title.is_match("feat(daemon): x"));
    assert!(title.is_match("fix!: y"));
    assert!(!title.is_match("Feat: x"));
    assert_eq!(
        d.pr_style("conventional-semver").unwrap().versioning,
        Versioning::Semver
    );
    assert_eq!(
        d.pr_style("conventional").unwrap().versioning,
        Versioning::None
    );
    let minimal = d.pr_style("minimal").unwrap();
    assert_eq!(minimal.title, None);
    assert_eq!(minimal.sections, ["Summary"]);
    assert_eq!(minimal.versioning, Versioning::None);
    let plain = d.pr_style("plain").unwrap();
    assert_eq!(plain.title, None);
    assert!(plain.sections.is_empty());
    assert_eq!(plain.versioning, Versioning::None);
    assert_eq!(rel_path(DefKind::PrStyle, "plain"), "pr-styles/plain.md");
}

#[test]
fn conventional_styles_check_commit_subjects_like_the_title() {
    let (_t, d) = defs();
    for name in [
        "conventional",
        "conventional-semver",
        "conventional-semver-pre1.0",
    ] {
        let s = d.pr_style(name).unwrap();
        assert!(s.commits.is_some(), "{name}");
        assert_eq!(s.commits, s.title, "{name}");
        assert!(s.prose.contains("## Commits"), "{name}");
    }
    for name in ["minimal", "plain"] {
        let s = d.pr_style(name).unwrap();
        assert_eq!(s.commits, None, "{name}");
        assert!(!s.prose.contains("## Commits"), "{name}");
    }
}

#[test]
fn pr_style_with_a_bad_commits_regex_does_not_load() {
    let (_t, d) = defs();
    let err = d
        .write(
            DefKind::PrStyle,
            "mine",
            &pr_style_with("name: mine\nsummary: s\ncommits: '('\n"),
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("pr-styles/mine.md") && err.to_string().contains("commits"),
        "{err}"
    );
    assert!(d.pr_style("mine").is_err());
}

#[test]
fn pr_style_with_a_bad_regex_does_not_load() {
    let (t, d) = defs();
    let path = t.path().join("fleet/pr-styles/mine.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, pr_style_with("name: mine\nsummary: s\ntitle: '('\n")).unwrap();
    let info = d
        .list()
        .into_iter()
        .find(|i| i.kind == DefKind::PrStyle && i.name == "mine")
        .unwrap();
    let err = info.error.unwrap();
    assert!(
        err.contains("pr-styles/mine.md") && err.contains("title"),
        "{err}"
    );
    let err = d
        .write(
            DefKind::PrStyle,
            "other",
            &pr_style_with("name: other\nsummary: s\ntitle: '('\n"),
        )
        .unwrap_err();
    assert!(err.to_string().contains("title"), "{err}");
}

#[test]
fn pr_style_with_unknown_versioning_does_not_load() {
    let (_t, d) = defs();
    let err = d
        .write(
            DefKind::PrStyle,
            "mine",
            &pr_style_with("name: mine\nsummary: s\nversioning: calver\n"),
        )
        .unwrap_err();
    assert!(
        matches!(err, FleetError::Invalid { .. }) && err.to_string().contains("pr-styles/mine.md"),
        "{err}"
    );
    assert!(d.pr_style("mine").is_err());
}

#[test]
fn user_pr_style_overrides_builtin() {
    let (_t, d) = defs();
    let text = pr_style_with("name: plain\nsummary: Mine.\nsections: [Why]\n");
    let info = d.write(DefKind::PrStyle, "plain", &text).unwrap();
    assert!(info.builtin && info.overridden);
    let s = d.pr_style("plain").unwrap();
    assert_eq!(s.summary, "Mine.");
    assert_eq!(s.sections, ["Why"]);
    assert!(!d.read(DefKind::PrStyle, "plain").unwrap().builtin);
    d.reset(DefKind::PrStyle, "plain").unwrap();
    assert!(d.pr_style("plain").unwrap().sections.is_empty());
}

#[test]
fn orchestrators_land_steps_without_merge_commits() {
    let (_t, d) = defs();
    for name in ["direct", "structured"] {
        let t = d.read(DefKind::Orchestrator, name).unwrap().text;
        let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(!t.contains("git merge <step branch>"), "{name}");
        for want in [
            "git merge --ff-only <step branch>",
            "git cherry-pick $(git merge-base HEAD <step branch>)..<step branch>",
            "git merge-base --is-ancestor HEAD <step branch>",
            "never a merge commit",
            "do not rebase the step branch",
            "git rebase origin/",
            "GIT_SEQUENCE_EDITOR",
            "once `open_pr` has returned a PR URL",
        ] {
            assert!(
                t.to_lowercase().contains(&want.to_lowercase()),
                "{name} lacks {want:?}"
            );
        }
        // The only `git merge` left is the fast-forward one.
        assert_eq!(
            t.matches("git merge ").count(),
            t.matches("git merge --ff-only").count(),
            "{name}"
        );
    }
}

fn persona(tmp: &Path, id: &str) -> Defs {
    Defs::for_persona(tmp.join(id).join("fleet"), tmp.join("fleet"))
}

fn files_under(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    builtin_files(dir, &mut out);
    out
}

fn specialist_text(name: &str, summary: &str) -> String {
    format!("---\nname: {name}\nsummary: {summary}\n---\nBody.\n")
}

#[test]
fn a_persona_file_wins_over_the_builtin() {
    let t = tempfile::tempdir().unwrap();
    let (a, b) = (persona(t.path(), "a"), persona(t.path(), "b"));
    let shared = Defs::shared(t.path().join("fleet"));
    let builtin = b.specialist("rust").unwrap().summary;
    a.write(
        DefKind::Specialist,
        "rust",
        &specialist_text("rust", "A's rust"),
    )
    .unwrap();
    assert_eq!(a.specialist("rust").unwrap().summary, "A's rust");
    assert_eq!(b.specialist("rust").unwrap().summary, builtin);
    assert_eq!(shared.specialist("rust").unwrap().summary, builtin);
    assert!(t.path().join("a/fleet/specialists/rust.md").is_file());
    assert!(!t.path().join("b").exists());
}

#[test]
fn builtins_are_never_copied() {
    let t = tempfile::tempdir().unwrap();
    let d = persona(t.path(), "a");
    d.list();
    d.read(DefKind::Specialist, "rust").unwrap();
    d.specialist("rust").unwrap();
    d.orchestrator("direct").unwrap();
    d.skill_dir("brainstorming").unwrap();
    assert!(files_under(t.path()).is_empty());
}

#[test]
fn pr_styles_read_the_fleet_root_for_every_persona() {
    let t = tempfile::tempdir().unwrap();
    let (a, b) = (persona(t.path(), "a"), persona(t.path(), "b"));
    let text = pr_style_with("name: house\nsummary: House rules\n");
    a.write(DefKind::PrStyle, "house", &text).unwrap();
    assert!(t.path().join("fleet/pr-styles/house.md").is_file());
    assert_eq!(b.pr_style("house").unwrap().summary, "House rules");
    assert_eq!(
        Defs::shared(t.path().join("fleet"))
            .pr_style("house")
            .unwrap()
            .summary,
        "House rules"
    );
}

#[test]
fn reset_deletes_only_the_personas_copy() {
    let t = tempfile::tempdir().unwrap();
    let (a, b) = (persona(t.path(), "a"), persona(t.path(), "b"));
    for d in [&a, &b] {
        d.write(
            DefKind::Specialist,
            "rust",
            &specialist_text("rust", "Mine"),
        )
        .unwrap();
    }
    a.reset(DefKind::Specialist, "rust").unwrap();
    assert!(a.read(DefKind::Specialist, "rust").unwrap().builtin);
    assert_eq!(b.specialist("rust").unwrap().summary, "Mine");
}

#[test]
fn skills_resolve_through_the_persona() {
    let t = tempfile::tempdir().unwrap();
    let (a, b) = (persona(t.path(), "a"), persona(t.path(), "b"));
    a.write(
        DefKind::Skill,
        "x",
        "---\nname: x\ndescription: An x\n---\n",
    )
    .unwrap();
    let text = "---\nname: styler\nsummary: Styles\nskills: [x]\n---\nBody.\n";
    a.write(DefKind::Specialist, "styler", text).unwrap();
    assert!(a.skill_dir("x").unwrap().user.is_some());
    let err = b.write(DefKind::Specialist, "styler", text).unwrap_err();
    assert!(err.to_string().contains("skill `x` not found"), "{err}");
}

#[test]
fn a_name_is_a_specialist_or_an_orchestrator_never_both() {
    let t = tempfile::tempdir().unwrap();
    let d = persona(t.path(), "a");
    d.write(
        DefKind::Specialist,
        "poet",
        &specialist_text("poet", "Verse"),
    )
    .unwrap();
    let err = d
        .write(
            DefKind::Orchestrator,
            "poet",
            &specialist_text("poet", "Verse"),
        )
        .unwrap_err();
    assert!(err.to_string().contains("already the specialist"), "{err}");
    assert!(!t.path().join("a/fleet/orchestrators/poet.md").exists());
    let err = d
        .write(
            DefKind::Specialist,
            "direct",
            &specialist_text("direct", "x"),
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("already the orchestrator"),
        "{err}"
    );
}

#[test]
fn shared_refuses_to_save_a_persona_kind() {
    let t = tempfile::tempdir().unwrap();
    let d = Defs::shared(t.path().join("fleet"));
    let err = d
        .write(
            DefKind::Specialist,
            "poet",
            &specialist_text("poet", "Verse"),
        )
        .unwrap_err();
    assert!(matches!(err, FleetError::Builtin(m) if m.contains("needs a persona")));
    assert!(files_under(t.path()).is_empty());
}

#[test]
fn move_role_moves_the_file_or_changes_nothing() {
    let t = tempfile::tempdir().unwrap();
    let d = persona(t.path(), "a");
    let root = t.path().join("a/fleet");
    let text = specialist_text("poet", "Verse");
    d.write(DefKind::Specialist, "poet", &text).unwrap();

    let info = d
        .move_role(DefKind::Specialist, DefKind::Orchestrator, "poet")
        .unwrap();
    assert_eq!(
        (info.kind, info.name.as_str()),
        (DefKind::Orchestrator, "poet")
    );
    assert!(!root.join("specialists/poet.md").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("orchestrators/poet.md")).unwrap(),
        text
    );

    // The target already has the name as the persona's own file.
    d.write(
        DefKind::Specialist,
        "bard",
        &specialist_text("bard", "Song"),
    )
    .unwrap();
    std::fs::write(root.join("orchestrators/bard.md"), "other").unwrap();
    assert!(d
        .move_role(DefKind::Specialist, DefKind::Orchestrator, "bard")
        .is_err());
    assert!(root.join("specialists/bard.md").is_file());
    assert_eq!(
        std::fs::read_to_string(root.join("orchestrators/bard.md")).unwrap(),
        "other"
    );

    // The target has it as a built-in: a specialist override named like a built-in orchestrator cannot exist, so
    // write the file by hand.
    std::fs::write(
        root.join("specialists/direct.md"),
        specialist_text("direct", "x"),
    )
    .unwrap();
    assert!(d
        .move_role(DefKind::Specialist, DefKind::Orchestrator, "direct")
        .is_err());
    assert!(root.join("specialists/direct.md").is_file());
    assert!(!root.join("orchestrators/direct.md").exists());
}

#[test]
fn move_role_refuses_a_builtin_even_when_overridden() {
    let t = tempfile::tempdir().unwrap();
    let d = persona(t.path(), "a");
    let root = t.path().join("a/fleet");
    let swap = |d: &Defs, from, to, name| d.move_role(from, to, name);

    // Built-in only: nothing is created.
    assert!(swap(&d, DefKind::Specialist, DefKind::Orchestrator, "rust").is_err());
    assert!(files_under(t.path()).is_empty());

    // Override of a built-in: both the override and the absence of a target stay.
    let text = specialist_text("rust", "Mine");
    d.write(DefKind::Specialist, "rust", &text).unwrap();
    let err = swap(&d, DefKind::Specialist, DefKind::Orchestrator, "rust").unwrap_err();
    assert!(err.to_string().contains("built in"), "{err}");
    assert_eq!(
        std::fs::read_to_string(root.join("specialists/rust.md")).unwrap(),
        text
    );
    assert!(!root.join("orchestrators/rust.md").exists());
}

#[test]
fn move_role_refuses_the_same_kind_and_non_roles() {
    let t = tempfile::tempdir().unwrap();
    let d = persona(t.path(), "a");
    let root = t.path().join("a/fleet");
    let text = specialist_text("poet", "Verse");
    d.write(DefKind::Specialist, "poet", &text).unwrap();
    for (from, to) in [
        (DefKind::Specialist, DefKind::Specialist),
        (DefKind::Specialist, DefKind::Skill),
        (DefKind::Skill, DefKind::Specialist),
        (DefKind::PrStyle, DefKind::Orchestrator),
        (DefKind::Specialist, DefKind::PrStyle),
    ] {
        assert!(d.move_role(from, to, "poet").is_err(), "{from:?}->{to:?}");
    }
    assert_eq!(
        std::fs::read_to_string(root.join("specialists/poet.md")).unwrap(),
        text
    );
    assert_eq!(files_under(t.path()).len(), 1);
}
