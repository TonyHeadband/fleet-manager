use fleet_daemon::repo_rules::{rewrite, RepoRules};

#[test]
fn file_rules_are_rewritten_to_cover_the_same_files_from_the_project_folder() {
    for (rule, want) in [
        // Anchored at the repo: the settings file's own folder, or the working directory Claude would have had.
        ("Read(/secrets/**)", "Read(api/secrets/**)"),
        ("Read(./.env)", "Read(api/.env)"),
        // Bare patterns match at any depth below the repo.
        ("Read(.env)", "Read(api/**/.env)"),
        ("Read(**/.env)", "Read(api/**/.env)"),
        ("Edit(src/**)", "Edit(api/**/src/**)"),
        ("Edit(!src/keep.rs)", "Edit(!api/**/src/keep.rs)"),
        // Absolute and home paths mean the same files from anywhere.
        ("Read(//etc/shadow)", "Read(//etc/shadow)"),
        ("Edit(~/.ssh/**)", "Edit(~/.ssh/**)"),
        // Rules that never match are written as the ones that do.
        ("Write(/dist/**)", "Edit(api/dist/**)"),
        ("NotebookEdit(*.ipynb)", "Edit(api/**/*.ipynb)"),
        ("Glob(/private/**)", "Read(api/private/**)"),
    ] {
        assert_eq!(rewrite(rule, "api"), want, "{rule}");
    }
}

#[test]
fn rules_without_paths_are_copied_as_they_are() {
    for rule in [
        "Bash(git push *)",
        "Bash(rm:*)",
        "WebFetch(domain:example.com)",
        "mcp__github__create_issue",
        "Edit",
    ] {
        assert_eq!(rewrite(rule, "api"), rule, "{rule}");
    }
}

#[test]
fn collect_takes_deny_and_ask_and_names_what_it_leaves_out() {
    let tmp = tempfile::tempdir().unwrap();
    let api = tmp.path().join("api");
    std::fs::create_dir_all(api.join(".claude")).unwrap();
    std::fs::write(
        api.join(".claude/settings.json"),
        r#"{"permissions": {"allow": ["Bash(npm:*)"], "deny": ["Read(./.env)", "Bash(git push *)"], "ask": ["Edit(/migrations/**)"]}, "hooks": {"Stop": []}}"#,
    )
    .unwrap();
    std::fs::write(api.join(".claude/settings.local.json"), "{not json").unwrap();
    std::fs::write(api.join(".mcp.json"), "{}").unwrap();
    let web = tmp.path().join("web");
    std::fs::create_dir_all(&web).unwrap();
    let rules = RepoRules::collect([("api", api.as_path()), ("web", web.as_path())]);
    assert_eq!(rules.deny, ["Read(api/.env)", "Bash(git push *)"]);
    assert_eq!(rules.ask, ["Edit(api/migrations/**)"]);
    assert_eq!(rules.not_applied.len(), 3, "{:?}", rules.not_applied);
    assert!(
        rules.not_applied[0]
            .starts_with("api: .claude/settings.json allow rules, hooks do not apply"),
        "{:?}",
        rules.not_applied
    );
    assert!(
        rules.not_applied[1].starts_with("api: .claude/settings.local.json is not valid JSON"),
        "{:?}",
        rules.not_applied
    );
    assert!(
        rules.not_applied[2].starts_with("api: .mcp.json is not loaded"),
        "{:?}",
        rules.not_applied
    );
}
