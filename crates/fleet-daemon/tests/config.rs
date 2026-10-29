use fleet_daemon::config::{load_with_home, ConfigError, LimitValues, PrSettings};
use std::path::PathBuf;

struct Env {
    _dir: tempfile::TempDir,
    home: PathBuf,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().to_path_buf();
    std::fs::create_dir_all(home.join("dev/core")).unwrap();
    std::fs::create_dir_all(home.join("dev/portal")).unwrap();
    Env { _dir: dir, home }
}

fn write(env: &Env, body: &str) -> PathBuf {
    let p = env.home.join("config.toml");
    std::fs::write(&p, body).unwrap();
    p
}

#[test]
fn loads_defaults_and_counts_old_repos() {
    let e = env();
    let p = write(
        &e,
        r#"
[[repos]]
name = "core"
path = "~/dev/core"
[[repos]]
name = "portal"
path = "~/dev/portal"
"#,
    );
    let c = load_with_home(&p, &e.home).unwrap();
    assert_eq!(c.listen.to_string(), "127.0.0.1:7878");
    assert_eq!(c.branch_prefix, "fleet");
    assert_eq!(c.state_dir, fleet_fs::dirs::data_dir(&e.home));
    // Repos belong to products now: the old entries still load, and are only counted for a warning.
    assert_eq!(c.legacy_repos, 2);
}

#[test]
fn explicit_values_override_defaults() {
    let e = env();
    let p = write(
        &e,
        r#"
listen = "[::1]:9000"
state_dir = "~/state"
branch_prefix = "alice"
[[repos]]
name = "core"
path = "~/dev/core"
"#,
    );
    let c = load_with_home(&p, &e.home).unwrap();
    assert_eq!(c.listen.to_string(), "[::1]:9000");
    assert_eq!(c.state_dir, e.home.join("state"));
    assert_eq!(c.branch_prefix, "alice");
}

#[test]
fn claude_command_defaults_to_plain_claude_and_can_carry_flags() {
    let e = env();
    let repo = "[[repos]]\nname=\"core\"\npath=\"~/dev/core\"\n";
    let p = write(&e, repo);
    assert_eq!(
        load_with_home(&p, &e.home).unwrap().claude_command,
        ["claude"]
    );
    let p = write(
        &e,
        &format!("claude_command = [\"claude\", \"--permission-mode\", \"default\"]\n{repo}"),
    );
    assert_eq!(
        load_with_home(&p, &e.home).unwrap().claude_command,
        ["claude", "--permission-mode", "default"]
    );
}

#[test]
fn empty_claude_command_is_rejected() {
    let e = env();
    let p = write(
        &e,
        "claude_command = []\n[[repos]]\nname=\"core\"\npath=\"~/dev/core\"\n",
    );
    assert!(matches!(
        load_with_home(&p, &e.home).unwrap_err(),
        ConfigError::EmptyClaudeCommand
    ));
}

#[test]
fn rejects_non_loopback_listen() {
    let e = env();
    for addr in ["0.0.0.0:7878", "192.168.1.5:7878", "[::]:7878"] {
        let p = write(
            &e,
            &format!("listen = \"{addr}\"\n[[repos]]\nname=\"core\"\npath=\"~/dev/core\"\n"),
        );
        let err = load_with_home(&p, &e.home).unwrap_err();
        assert!(matches!(err, ConfigError::NonLoopback(_)), "{addr}: {err}");
    }
}

#[test]
fn rejects_branch_prefix_that_is_not_a_safe_ref_component() {
    let e = env();
    for bad in ["", "a b", "-x", "a..b", "a/", "x;y"] {
        let p = write(
            &e,
            &format!("branch_prefix = \"{bad}\"\n[[repos]]\nname=\"core\"\npath=\"~/dev/core\"\n"),
        );
        assert!(
            matches!(
                load_with_home(&p, &e.home).unwrap_err(),
                ConfigError::BadBranchPrefix(_)
            ),
            "{bad:?}"
        );
    }
}

#[test]
fn missing_file_is_all_defaults() {
    let e = env();
    let c = load_with_home(&e.home.join("nope.toml"), &e.home).unwrap();
    assert_eq!(c.tasks_root, e.home.join("fleet-tasks"));
    assert_eq!(c.legacy_repos, 0);
    assert_eq!(c.listen.to_string(), "127.0.0.1:7878");
}

#[test]
fn tasks_root_expands_tilde() {
    let e = env();
    let p = write(&e, "tasks_root = \"~/work/fleet\"\n");
    assert_eq!(
        load_with_home(&p, &e.home).unwrap().tasks_root,
        e.home.join("work/fleet")
    );
}

#[test]
fn personas_dir_defaults_and_expands() {
    let e = env();
    let missing = load_with_home(&e.home.join("nope.toml"), &e.home).unwrap();
    assert_eq!(
        missing.personas_dir,
        fleet_fs::dirs::data_dir(&e.home).join("personas")
    );
    assert_eq!(missing.gh_command, "gh");
    let p = write(
        &e,
        "personas_dir = \"~/work/personas\"\ngh_command = \"~/opt/gh\"\n",
    );
    let c = load_with_home(&p, &e.home).unwrap();
    assert_eq!(c.personas_dir, e.home.join("work/personas"));
    assert_eq!(c.gh_command, e.home.join("opt/gh").to_string_lossy());
}

#[test]
fn gh_command_empty_is_rejected() {
    let e = env();
    let p = write(&e, "gh_command = \" \"\n");
    assert!(matches!(
        load_with_home(&p, &e.home),
        Err(ConfigError::EmptyGhCommand)
    ));
}

#[test]
fn agent_config_dir_sits_next_to_config() {
    let e = env();
    let p = write(&e, "");
    assert_eq!(
        load_with_home(&p, &e.home).unwrap().agent_config_dir,
        e.home.join("agent")
    );
    // Also without a config file: the directory follows the path asked for.
    let missing = e.home.join("sub/config.toml");
    assert_eq!(
        load_with_home(&missing, &e.home).unwrap().agent_config_dir,
        e.home.join("sub/agent")
    );
}

#[test]
fn fleet_dir_sits_next_to_config() {
    let e = env();
    let p = write(&e, "");
    assert_eq!(
        load_with_home(&p, &e.home).unwrap().fleet_dir,
        e.home.join("fleet")
    );
    let missing = e.home.join("sub/config.toml");
    assert_eq!(
        load_with_home(&missing, &e.home).unwrap().fleet_dir,
        e.home.join("sub/fleet")
    );
}

#[test]
fn test_timeout_defaults_and_refuses_zero() {
    let e = env();
    let p = write(&e, "");
    assert_eq!(load_with_home(&p, &e.home).unwrap().test_timeout_secs, 1800);
    let p = write(&e, "test_timeout_secs = 60\n");
    assert_eq!(load_with_home(&p, &e.home).unwrap().test_timeout_secs, 60);
    let p = write(&e, "test_timeout_secs = 0\n");
    assert!(matches!(
        load_with_home(&p, &e.home),
        Err(ConfigError::ZeroTestTimeout)
    ));
}

#[test]
fn old_repos_of_any_shape_do_not_stop_the_daemon() {
    let e = env();
    let p = write(
        &e,
        "[[repos]]\nname=\"a/b\"\npath=\"~/dev/missing\"\n[[repos]]\nname=\"a/b\"\n",
    );
    let c = load_with_home(&p, &e.home).unwrap();
    assert_eq!(c.legacy_repos, 2);
    assert_eq!(c.tasks_root, e.home.join("fleet-tasks"));
}

#[test]
fn an_unreadable_config_still_names_the_path() {
    let e = env();
    // A directory in place of the file is an Io error other than NotFound, which must not read as "defaults".
    let err = load_with_home(&e.home, &e.home).unwrap_err();
    assert!(matches!(err, ConfigError::Io(..)), "{err}");
}

#[cfg(not(windows))]
#[test]
fn default_path_is_under_dot_config() {
    use fleet_daemon::config::default_path;
    use std::path::Path;
    assert_eq!(
        default_path(Path::new("/home/u")),
        PathBuf::from("/home/u/.config/fleet-manager/config.toml")
    );
}

#[test]
fn telemetry_endpoint_is_read() {
    let e = env();
    let p = write(&e, "[telemetry]\notlp_endpoint = \"http://c:4317\"\n");
    let c = load_with_home(&p, &e.home).unwrap();
    assert_eq!(c.telemetry.otlp_endpoint.as_deref(), Some("http://c:4317"));

    let p = write(&e, "");
    assert_eq!(
        load_with_home(&p, &e.home).unwrap().telemetry.otlp_endpoint,
        None
    );
}

#[test]
fn projects_root_is_refused_with_its_new_name() {
    let e = env();
    let p = write(&e, "projects_root = \"~/x\"\n");
    let err = load_with_home(&p, &e.home).unwrap_err().to_string();
    assert!(
        err.contains("projects_root was renamed to tasks_root; rename it in"),
        "{err}"
    );
    assert!(err.contains(&p.display().to_string()), "{err}");
}

#[test]
fn tasks_root_defaults_to_fleet_tasks() {
    let e = env();
    let c = load_with_home(&e.home.join("missing.toml"), &e.home).unwrap();
    assert!(c.tasks_root.ends_with("fleet-tasks"));
}

#[test]
fn limits_default_to_8_20_3() {
    let e = env();
    let p = e.home.join("missing.toml");
    assert_eq!(
        load_with_home(&p, &e.home).unwrap().limits,
        LimitValues {
            max_breakdown_versions: 8,
            max_steps: 20,
            max_parallel_specialists: 3,
            max_pr_attempts: 3,
        }
    );
}

#[test]
fn limits_are_read() {
    let e = env();
    let p = write(&e, "max_steps = 30\n");
    assert_eq!(
        load_with_home(&p, &e.home).unwrap().limits,
        LimitValues {
            max_breakdown_versions: 8,
            max_steps: 30,
            max_parallel_specialists: 3,
            max_pr_attempts: 3,
        }
    );
}

#[test]
fn zero_limit_is_rejected() {
    let e = env();
    let p = write(&e, "max_parallel_specialists = 0\n");
    let err = load_with_home(&p, &e.home).unwrap_err();
    assert!(
        matches!(err, ConfigError::ZeroLimit("max_parallel_specialists")),
        "{err}"
    );
    assert!(
        err.to_string().contains("max_parallel_specialists"),
        "{err}"
    );
}

#[test]
fn pr_settings_default_to_plain_and_three_attempts() {
    let e = env();
    let c = load_with_home(&e.home.join("missing.toml"), &e.home).unwrap();
    assert_eq!(
        c.pr,
        PrSettings {
            style: "plain".into(),
            attribution: String::new(),
            draft: false,
            labels: vec![],
        }
    );
    assert_eq!(c.limits.max_pr_attempts, 3);
}

#[test]
fn pr_settings_and_max_pr_attempts_are_read() {
    let e = env();
    let p = write(
        &e,
        "pr_style = \"conventional\"\npr_attribution = \"Made with Fleet Manager\"\npr_draft = true\npr_labels = [\"fleet\", \"bot\"]\nmax_pr_attempts = 5\n",
    );
    let c = load_with_home(&p, &e.home).unwrap();
    assert_eq!(
        c.pr,
        PrSettings {
            style: "conventional".into(),
            attribution: "Made with Fleet Manager".into(),
            draft: true,
            labels: vec!["fleet".into(), "bot".into()],
        }
    );
    assert_eq!(c.limits.max_pr_attempts, 5);
}

#[test]
fn zero_max_pr_attempts_is_rejected() {
    let e = env();
    let p = write(&e, "max_pr_attempts = 0\n");
    let err = load_with_home(&p, &e.home).unwrap_err();
    assert!(
        matches!(err, ConfigError::ZeroLimit("max_pr_attempts")),
        "{err}"
    );
}
