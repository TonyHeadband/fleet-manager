use fleet_api::{AgentConfig, AgentConfigUpdate, AgentSettings};
use fleet_daemon::agent_config::{load, load_lenient, save, validate};

fn full(c: &AgentConfig) -> AgentConfigUpdate {
    AgentConfigUpdate {
        instructions: Some(c.instructions.clone()),
        settings: Some(c.settings.clone()),
    }
}

fn config(f: impl FnOnce(&mut AgentConfig)) -> AgentConfig {
    let mut c = AgentConfig::default();
    f(&mut c);
    c
}

#[test]
fn missing_files_give_defaults() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        load(&dir.path().join("nope")).unwrap(),
        AgentConfig::default()
    );
}

#[test]
fn save_then_load_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().join("agent");
    let c = AgentConfig {
        instructions: "Be terse.\n".into(),
        settings: AgentSettings {
            allow: vec!["Bash(git status)".into(), "Read".into()],
            deny: vec!["WebFetch".into()],
            env: [("EDITOR".to_string(), "vim".to_string())].into(),
        },
        settings_error: None,
    };
    save(&dir, &full(&c)).unwrap();
    assert_eq!(load(&dir).unwrap(), c);
    assert_eq!(
        std::fs::read_to_string(dir.join("instructions.md")).unwrap(),
        "Be terse.\n"
    );
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names.len(), 2, "no temp file left behind: {names:?}");
}

#[test]
fn reserved_env_is_rejected() {
    for key in [
        "GH_TOKEN",
        "CLAUDE_CONFIG_DIR",
        "GIT_ASKPASS",
        "MCP_TOOL_TIMEOUT",
        "FLEET_TOKEN",
        "SSH_AUTH_SOCK",
        "GITHUB_TOKEN",
    ] {
        let c = config(|c| {
            c.settings.env.insert(key.into(), "x".into());
        });
        let err = validate(&c).unwrap_err().to_string();
        assert!(err.contains(key), "{key}: {err}");
    }
    for key in ["1X", "A B", "", "A=B", "A-B"] {
        let c = config(|c| {
            c.settings.env.insert(key.into(), "x".into());
        });
        assert!(validate(&c).is_err(), "{key:?}");
    }
    let ok = config(|c| {
        c.settings.env.insert("EDITOR".into(), "vim".into());
    });
    validate(&ok).unwrap();
}

#[test]
fn deny_on_fleet_tools_is_rejected() {
    for rule in ["mcp__fleet__request_repo", "mcp__fleet"] {
        let c = config(|c| c.settings.deny.push(rule.into()));
        let err = validate(&c).unwrap_err().to_string();
        assert!(err.contains(rule), "{rule}: {err}");
    }
    validate(&config(|c| c.settings.deny.push("mcp__other".into()))).unwrap();
}

#[test]
fn bad_rule_is_rejected() {
    for rule in ["Bash(git:*", " Read", "", "Bash(x)y", "1Bash"] {
        for into_deny in [false, true] {
            let c = config(|c| {
                if into_deny {
                    c.settings.deny.push(rule.into())
                } else {
                    c.settings.allow.push(rule.into())
                }
            });
            let err = validate(&c).unwrap_err().to_string();
            assert!(err.contains(&format!("{rule:?}")), "{rule:?}: {err}");
        }
    }
    validate(&config(|c| c.settings.allow.push("Bash(git:*)".into()))).unwrap();
}

#[test]
fn save_refuses_an_invalid_config_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().join("agent");
    let c = config(|c| {
        c.instructions = "x".into();
        c.settings.env.insert("GH_HOST".into(), "x".into());
    });
    assert!(save(&dir, &full(&c)).is_err());
    assert!(!dir.exists());
}

#[test]
fn a_partial_settings_file_fills_the_rest_with_defaults() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("settings.json"), r#"{"allow":["Read"]}"#).unwrap();
    let c = load(dir.path()).unwrap();
    assert_eq!(c.settings.allow, ["Read"]);
    assert!(c.settings.env.is_empty());
}

#[test]
fn a_broken_settings_file_is_an_error_naming_the_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("settings.json"), "{").unwrap();
    let err = load(dir.path()).unwrap_err().to_string();
    assert!(err.contains("settings.json"), "{err}");
}

#[test]
fn deny_with_arguments_on_fleet_tools_is_rejected() {
    for rule in [
        "mcp__fleet__request_repo(*)",
        "mcp__fleet(*)",
        "mcp__fleet__list_repos(x)",
    ] {
        let c = config(|c| c.settings.deny.push(rule.into()));
        let err = validate(&c).unwrap_err().to_string();
        assert!(err.contains(rule), "{rule}: {err}");
    }
    validate(&config(|c| c.settings.deny.push("mcp__other(*)".into()))).unwrap();
}

#[test]
fn an_unknown_settings_key_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("settings.json"), r#"{"denny":["Read"]}"#).unwrap();
    let err = load(dir.path()).unwrap_err().to_string();
    assert!(
        err.contains("settings.json") && err.contains("denny"),
        "{err}"
    );
}

#[test]
fn ambient_foundry_vars_are_reserved() {
    for key in ["CLAUDE_CODE_USE_FOUNDRY", "ANTHROPIC_FOUNDRY_API_KEY"] {
        let c = config(|c| {
            c.settings.env.insert(key.into(), "x".into());
        });
        assert!(validate(&c).unwrap_err().to_string().contains(key), "{key}");
    }
}

#[test]
fn concurrent_saves_leave_a_valid_config() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().to_path_buf();
    let threads: Vec<_> = (0..8)
        .map(|i| {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let c = config(|c| c.instructions = format!("v{i}"));
                for _ in 0..20 {
                    save(&dir, &full(&c)).unwrap();
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    load(&dir).unwrap();
}

#[test]
fn lenient_load_keeps_the_instructions_when_settings_are_broken() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("instructions.md"), "keep me").unwrap();
    std::fs::write(dir.path().join("settings.json"), "{").unwrap();
    let c = load_lenient(dir.path()).unwrap();
    assert_eq!(c.instructions, "keep me");
    assert_eq!(c.settings, AgentSettings::default());
    assert!(c.settings_error.unwrap().contains("settings.json"));
    // A file that parses but breaks a rule is reported too.
    std::fs::write(
        dir.path().join("settings.json"),
        r#"{"env":{"GH_TOKEN":"x"}}"#,
    )
    .unwrap();
    let c = load_lenient(dir.path()).unwrap();
    assert!(c.settings_error.unwrap().contains("GH_TOKEN"));
}

#[test]
fn a_save_writes_only_the_part_given() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("instructions.md"), "keep me").unwrap();
    std::fs::write(dir.path().join("settings.json"), "{").unwrap();
    let settings = AgentSettings {
        allow: vec!["Read".into()],
        ..Default::default()
    };
    save(
        dir.path(),
        &AgentConfigUpdate {
            instructions: None,
            settings: Some(settings.clone()),
        },
    )
    .unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("instructions.md")).unwrap(),
        b"keep me"
    );
    assert_eq!(load(dir.path()).unwrap().settings, settings);

    let before = std::fs::read(dir.path().join("settings.json")).unwrap();
    save(
        dir.path(),
        &AgentConfigUpdate {
            instructions: Some("new".into()),
            settings: None,
        },
    )
    .unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("settings.json")).unwrap(),
        before
    );
}

#[test]
fn an_empty_update_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    assert!(save(dir.path(), &AgentConfigUpdate::default()).is_err());
}
