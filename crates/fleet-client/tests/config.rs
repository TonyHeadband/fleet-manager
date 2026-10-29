use fleet_api::{Effort, PermissionMode};
use fleet_client::config::{
    config_arg, exists, gui_settings_path, load, parse_labels, parse_limit, prepare_personas_dir,
    prepare_tasks_root, set_agent_defaults, set_limits, set_personas_dir, set_pr, set_tasks_root,
    write_initial, CloseChoice, ConfigError, GuiSettings, Limits, PrSettings, SettingsWriter,
    ThemeChoice,
};
use std::path::Path;

struct Env {
    _tmp: tempfile::TempDir,
    home: std::path::PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_path_buf();
    Env { _tmp: tmp, home }
}

fn write(e: &Env, body: &str) -> std::path::PathBuf {
    let p = e.home.join("config.toml");
    std::fs::write(&p, body).unwrap();
    p
}

#[test]
fn reads_listen_and_state_dir_and_ignores_every_other_key() {
    let e = env();
    let p = write(
        &e,
        r#"
listen = "127.0.0.1:9000"
state_dir = "~/st"
branch_prefix = "alice"
claude_command = ["claude", "--verbose"]
some_future_key = { nested = true }
[[repos]]
name = "core"
path = "~/dev/core"
"#,
    );
    let c = load(&p, &e.home).unwrap();
    assert_eq!(c.listen.to_string(), "127.0.0.1:9000");
    assert_eq!(c.state_dir, e.home.join("st"));
    assert_eq!(c.path, p);
    assert_eq!(c.base_url(), "http://127.0.0.1:9000");
}

#[test]
fn defaults_match_the_daemons() {
    let e = env();
    let c = load(
        &write(&e, "[[repos]]\nname = \"core\"\npath = \"~/x\"\n"),
        &e.home,
    )
    .unwrap();
    assert_eq!(c.listen.to_string(), "127.0.0.1:7878");
    assert_eq!(c.state_dir, fleet_fs::dirs::data_dir(&e.home));
    assert_eq!(
        c.pidfile(),
        fleet_fs::dirs::data_dir(&e.home).join("daemon.pid")
    );
    assert_eq!(
        c.log_file(),
        fleet_fs::dirs::data_dir(&e.home).join("daemon.log")
    );
}

#[test]
fn an_ipv6_loopback_listen_address_gets_a_bracketed_url() {
    let e = env();
    let c = load(&write(&e, "listen = \"[::1]:7878\"\n"), &e.home).unwrap();
    assert_eq!(c.base_url(), "http://[::1]:7878");
}

#[test]
fn a_non_loopback_listen_address_is_refused() {
    let e = env();
    for addr in ["0.0.0.0:7878", "192.168.1.20:7878", "[::]:7878"] {
        let p = write(&e, &format!("listen = \"{addr}\"\n"));
        let err = load(&p, &e.home).unwrap_err();
        assert!(
            matches!(err, fleet_client::config::ConfigError::NonLoopback(..)),
            "{addr}: {err}"
        );
        assert!(err.to_string().contains("not loopback"), "{err}");
    }
}

#[test]
fn missing_file_is_defaults() {
    let e = env();
    let p = e.home.join("nope.toml");
    assert!(!exists(&p));
    let c = load(&p, &e.home).unwrap();
    assert_eq!(c.listen.to_string(), "127.0.0.1:7878");
    assert_eq!(c.tasks_root, e.home.join("fleet-tasks"));
    assert_eq!(c.state_dir, fleet_fs::dirs::data_dir(&e.home));
    assert_eq!(
        c.personas_dir,
        fleet_fs::dirs::data_dir(&e.home).join("personas")
    );
}

#[test]
fn the_default_personas_dir_is_built_from_the_data_dir() {
    let e = env();
    let p = load(&e.home.join("nope.toml"), &e.home)
        .unwrap()
        .personas_dir;
    assert!(p.ends_with("personas"), "{p:?}");
    // Not a `~/AppData/Local/...` string joined on: the slashes would be mixed.
    #[cfg(windows)]
    assert!(!p.to_string_lossy().contains('/'), "{p:?}");
}

#[test]
fn reads_personas_dir() {
    let e = env();
    let p = write(&e, "personas_dir = \"~/keys/personas\"\n");
    assert_eq!(
        load(&p, &e.home).unwrap().personas_dir,
        e.home.join("keys/personas")
    );
}

#[test]
fn write_initial_then_load_round_trips_the_root() {
    let e = env();
    let p = e.home.join("cfg/dir/config.toml");
    let root = e.home.join("my tasks");
    write_initial(&p, &root, &e.home.join("personas"), &e.home).unwrap();
    assert!(exists(&p));
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.starts_with("tasks_root = \"~/my tasks\"\n"), "{text}");
    assert_eq!(load(&p, &e.home).unwrap().tasks_root, root);
}

#[test]
fn write_initial_writes_a_root_outside_home_as_is() {
    let e = env();
    let p = e.home.join("config.toml");
    write_initial(
        &p,
        Path::new("/srv/fleet"),
        &e.home.join("personas"),
        &e.home,
    )
    .unwrap();
    assert_eq!(
        load(&p, &e.home).unwrap().tasks_root,
        Path::new("/srv/fleet")
    );
}

#[test]
fn write_initial_writes_personas_dir() {
    let e = env();
    let p = e.home.join("config.toml");
    write_initial(
        &p,
        &e.home.join("fleet-tasks"),
        &e.home.join("keys/personas"),
        &e.home,
    )
    .unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(
        text.contains("personas_dir = \"~/keys/personas\"\n"),
        "{text}"
    );
    // The key must be valid TOML the daemon's strict parser accepts, next to the template.
    assert!(toml::from_str::<toml::Table>(&text)
        .unwrap()
        .contains_key("personas_dir"));
}

#[test]
fn write_initial_never_overwrites_an_existing_config() {
    let e = env();
    let p = write(&e, "branch_prefix = \"mine\"\n");
    let err = write_initial(
        &p,
        &e.home.join("fleet-tasks"),
        &e.home.join("personas"),
        &e.home,
    )
    .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        "branch_prefix = \"mine\"\n"
    );
}

#[test]
fn write_initial_keeps_the_template_comments() {
    let e = env();
    let p = e.home.join("config.toml");
    write_initial(
        &p,
        &e.home.join("fleet-tasks"),
        &e.home.join("personas"),
        &e.home,
    )
    .unwrap();
    assert!(std::fs::read_to_string(&p)
        .unwrap()
        .contains("# claude_command"));
}

#[test]
fn invalid_toml_names_the_path() {
    let e = env();
    let err = load(&write(&e, "listen = = ="), &e.home).unwrap_err();
    assert!(matches!(err, ConfigError::Parse(..)));
    assert!(err.to_string().contains("config.toml"), "{err}");
}

#[cfg(not(windows))]
#[test]
fn default_paths_are_under_dot_config() {
    let home = Path::new("/home/u");
    assert_eq!(
        fleet_client::config::default_path(home),
        Path::new("/home/u/.config/fleet-manager/config.toml")
    );
    assert_eq!(
        gui_settings_path(home),
        Path::new("/home/u/.config/fleet-manager/gui.toml")
    );
}

#[test]
fn gui_settings_round_trip_and_create_the_directory() {
    let e = env();
    let p = gui_settings_path(&e.home);
    assert_eq!(
        GuiSettings::load(&p),
        GuiSettings::default(),
        "missing file is the default"
    );
    let s = GuiSettings {
        close_choice: Some(CloseChoice::StopDaemon),
        ..Default::default()
    };
    s.save(&p).unwrap();
    assert_eq!(GuiSettings::load(&p), s);
    GuiSettings {
        close_choice: None,
        ..Default::default()
    }
    .save(&p)
    .unwrap();
    assert_eq!(GuiSettings::load(&p).close_choice, None);
}

#[test]
fn gui_settings_keeps_active_product() {
    let e = env();
    let p = gui_settings_path(&e.home);
    assert_eq!(GuiSettings::load(&p).active_product, None);
    GuiSettings {
        active_product: Some("portal".into()),
        ..Default::default()
    }
    .save(&p)
    .unwrap();
    assert_eq!(
        GuiSettings::load(&p).active_product.as_deref(),
        Some("portal")
    );
    std::fs::write(&p, "theme = \"dark\"\n").unwrap();
    assert_eq!(GuiSettings::load(&p).active_product, None);
}

#[test]
fn gui_settings_keeps_active_persona() {
    let e = env();
    let p = gui_settings_path(&e.home);
    assert_eq!(GuiSettings::load(&p).active_persona, None);
    GuiSettings {
        active_persona: Some("work".into()),
        ..Default::default()
    }
    .save(&p)
    .unwrap();
    assert_eq!(
        GuiSettings::load(&p).active_persona.as_deref(),
        Some("work")
    );
    // A file written before the key existed still loads.
    std::fs::write(&p, "theme = \"dark\"\n").unwrap();
    assert_eq!(GuiSettings::load(&p).active_persona, None);
}

#[test]
fn gui_settings_keeps_the_skipped_version() {
    let e = env();
    let p = gui_settings_path(&e.home);
    GuiSettings {
        skipped_version: Some("0.2.0".into()),
        ..Default::default()
    }
    .save(&p)
    .unwrap();
    assert_eq!(
        GuiSettings::load(&p).skipped_version.as_deref(),
        Some("0.2.0")
    );
    // A file written before the key existed still loads, keeping its other choices.
    std::fs::write(&p, "theme = \"dark\"\n").unwrap();
    let old = GuiSettings::load(&p);
    assert_eq!(old.skipped_version, None);
    assert_eq!(old.theme, ThemeChoice::Dark);
}

#[test]
fn corrupt_gui_settings_fall_back_to_the_default() {
    let e = env();
    let p = gui_settings_path(&e.home);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    for bad in [
        "not toml at all = = =",
        "close_choice = \"explode\"",
        "close_choice = 7",
        "\u{0}\u{1}",
    ] {
        std::fs::write(&p, bad).unwrap();
        assert_eq!(GuiSettings::load(&p), GuiSettings::default(), "{bad:?}");
    }
}

#[test]
fn theme_choice_names_itself() {
    assert_eq!(ThemeChoice::System.label(), "System");
    assert_eq!(ThemeChoice::Light.label(), "Light");
    assert_eq!(ThemeChoice::Dark.label(), "Dark");
}

#[test]
fn a_settings_file_without_theme_loads_as_system() {
    let e = env();
    let p = gui_settings_path(&e.home);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "close_choice = \"stop_daemon\"\n").unwrap();
    let s = GuiSettings::load(&p);
    assert_eq!(s.theme, ThemeChoice::System);
    assert_eq!(s.close_choice, Some(CloseChoice::StopDaemon));
}

#[test]
fn an_unknown_theme_loads_as_system_and_keeps_the_close_choice() {
    let e = env();
    let p = gui_settings_path(&e.home);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "close_choice = \"keep_running\"\ntheme = \"Dark\"\n").unwrap();
    let s = GuiSettings::load(&p);
    assert_eq!(s.theme, ThemeChoice::System);
    assert_eq!(s.close_choice, Some(CloseChoice::KeepRunning));
}

#[test]
fn update_keeps_fields_it_does_not_touch() {
    let e = env();
    let p = gui_settings_path(&e.home);
    GuiSettings::update(&p, |s| s.theme = ThemeChoice::Dark).unwrap();
    GuiSettings::update(&p, |s| s.close_choice = Some(CloseChoice::KeepRunning)).unwrap();
    let s = GuiSettings::load(&p);
    assert_eq!(s.theme, ThemeChoice::Dark);
    assert_eq!(s.close_choice, Some(CloseChoice::KeepRunning));
}

#[test]
fn sticky_alerts_default_on_and_survive_a_round_trip() {
    let e = env();
    let p = gui_settings_path(&e.home);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "theme = \"dark\"\n").unwrap();
    assert!(
        GuiSettings::load(&p).sticky_alerts,
        "a file written before the key existed reads as on"
    );
    GuiSettings::update(&p, |s| s.sticky_alerts = false).unwrap();
    let s = GuiSettings::load(&p);
    assert!(!s.sticky_alerts);
    assert_eq!(s.theme, ThemeChoice::Dark);
}

#[test]
fn set_tasks_root_keeps_comments_and_keys() {
    let e = env();
    let p = write(
        &e,
        "# my own notes\nlisten = \"127.0.0.1:9000\"\ntasks_root = \"/old/place\" # moved twice\n\n[[repos]]\nname = \"core\"\npath = \"~/dev/core\"\n",
    );
    set_tasks_root(&p, &e.home.join("x"), &e.home).unwrap();
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        "# my own notes\nlisten = \"127.0.0.1:9000\"\ntasks_root = \"~/x\" # moved twice\n\n[[repos]]\nname = \"core\"\npath = \"~/dev/core\"\n",
    );
    let c = load(&p, &e.home).unwrap();
    assert_eq!(c.tasks_root, e.home.join("x"));
    assert_eq!(c.listen.to_string(), "127.0.0.1:9000");
    assert!(
        !e.home.join("config.toml.tmp").exists(),
        "the temp file is renamed over the config"
    );
}

#[test]
fn set_tasks_root_adds_the_key_when_the_file_has_none() {
    let e = env();
    let p = write(
        &e,
        "# notes\nbranch_prefix = \"mine\"\n\n[[repos]]\nname = \"core\"\npath = \"~/dev/core\"\n",
    );
    set_tasks_root(&p, Path::new("/srv/fleet"), &e.home).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(
        text.contains("# notes") && text.contains("branch_prefix = \"mine\""),
        "{text}"
    );
    assert_eq!(
        load(&p, &e.home).unwrap().tasks_root,
        Path::new("/srv/fleet")
    );
}

#[test]
fn set_tasks_root_creates_a_missing_file() {
    let e = env();
    let p = e.home.join("cfg/config.toml");
    set_tasks_root(&p, &e.home.join("x"), &e.home).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.starts_with("tasks_root = \"~/x\"\n"), "{text}");
    assert!(
        text.contains("# claude_command"),
        "the template comes with it: {text}"
    );
}

#[test]
fn set_tasks_root_refuses_a_file_that_is_not_toml() {
    let e = env();
    let p = write(&e, "listen = = =");
    assert!(set_tasks_root(&p, &e.home.join("x"), &e.home).is_err());
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        "listen = = =",
        "a broken file is left for the user to fix"
    );
}

#[test]
fn prepare_tasks_root_rejects_relative_and_empty() {
    let e = env();
    assert_eq!(
        prepare_tasks_root("  ", &e.home),
        Err("Enter a folder for the tasks".to_string())
    );
    assert_eq!(
        prepare_tasks_root("tasks", &e.home),
        Err("tasks is not an absolute path".to_string())
    );
}

#[test]
fn prepare_tasks_root_expands_home_and_creates_the_folder() {
    let e = env();
    let dir = prepare_tasks_root(" ~/a/b ", &e.home).unwrap();
    assert_eq!(dir, e.home.join("a/b"));
    assert!(dir.is_dir());
    let blocker = e.home.join("file");
    std::fs::write(&blocker, "").unwrap();
    let err = prepare_tasks_root(blocker.join("sub").to_str().unwrap(), &e.home).unwrap_err();
    assert!(err.starts_with("Cannot create "), "{err}");
}

#[cfg(unix)]
#[test]
fn set_tasks_root_keeps_a_symlinked_config_a_link() {
    let e = env();
    let dotfiles = e.home.join("dotfiles");
    std::fs::create_dir_all(&dotfiles).unwrap();
    let target = dotfiles.join("config.toml");
    std::fs::write(&target, "# mine\ntasks_root = \"/old\"\n").unwrap();
    let link = e.home.join("config.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    set_tasks_root(&link, &e.home.join("x"), &e.home).unwrap();
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the config is still a link"
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "# mine\ntasks_root = \"~/x\"\n"
    );
    assert!(!dotfiles.join("config.toml.tmp").exists() && !e.home.join("config.toml.tmp").exists());
}

/// The config may hold secrets the user locked down; rewriting it must not widen its mode to the umask default.
#[cfg(unix)]
#[test]
fn set_tasks_root_keeps_the_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let e = env();
    let p = write(&e, "tasks_root = \"/old\"\n");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    set_tasks_root(&p, Path::new("/srv/fleet"), &e.home).unwrap();
    assert_eq!(
        std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

/// A temp file left by an earlier crash may have a wide mode; the new one must not inherit it.
#[cfg(unix)]
#[test]
fn set_tasks_root_does_not_reuse_a_leftover_temp_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let e = env();
    let p = write(&e, "tasks_root = \"/old\"\n");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    let tmp = e.home.join("config.toml.tmp");
    std::fs::write(&tmp, "stale").unwrap();
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o666)).unwrap();
    set_tasks_root(&p, Path::new("/srv/fleet"), &e.home).unwrap();
    assert_eq!(
        std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(!tmp.exists());
}

#[test]
fn set_personas_dir_keeps_comments_and_keys() {
    let e = env();
    let p = write(
        &e,
        "# my own notes\ntasks_root = \"~/p\"\npersonas_dir = \"/old/personas\" # with the logins\n\n[[repos]]\nname = \"core\"\npath = \"~/dev/core\"\n",
    );
    set_personas_dir(&p, &e.home.join("keys/personas"), &e.home).unwrap();
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        "# my own notes\ntasks_root = \"~/p\"\npersonas_dir = \"~/keys/personas\" # with the logins\n\n[[repos]]\nname = \"core\"\npath = \"~/dev/core\"\n",
    );
    let c = load(&p, &e.home).unwrap();
    assert_eq!(c.personas_dir, e.home.join("keys/personas"));
    assert_eq!(c.tasks_root, e.home.join("p"));
    assert!(!e.home.join("config.toml.tmp").exists());
}

#[test]
fn set_personas_dir_adds_the_key_when_the_file_has_none() {
    let e = env();
    let p = write(&e, "# notes\ntasks_root = \"~/p\"\n");
    set_personas_dir(&p, Path::new("/srv/personas"), &e.home).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(
        text.contains("# notes") && text.contains("personas_dir = \"/srv/personas\""),
        "{text}"
    );
    let c = load(&p, &e.home).unwrap();
    assert_eq!(c.personas_dir, Path::new("/srv/personas"));
    assert_eq!(c.tasks_root, e.home.join("p"));
}

#[test]
fn set_personas_dir_creates_a_missing_file() {
    let e = env();
    let p = e.home.join("cfg/config.toml");
    set_personas_dir(&p, &e.home.join("keys"), &e.home).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(
        text.starts_with("tasks_root = \"~/fleet-tasks\"\npersonas_dir = \"~/keys\"\n"),
        "the default tasks folder and the chosen personas folder: {text}"
    );
    assert!(text.contains("# claude_command"), "{text}");
}

#[test]
fn prepare_personas_dir_expands_home_without_creating_the_folder() {
    let e = env();
    assert_eq!(
        prepare_personas_dir("  ", &e.home),
        Err("Enter a folder for the personas".to_string())
    );
    assert_eq!(
        prepare_personas_dir("personas", &e.home),
        Err("personas is not an absolute path".to_string())
    );
    let dir = prepare_personas_dir(" ~/a/b ", &e.home).unwrap();
    assert_eq!(dir, e.home.join("a/b"));
    assert!(!dir.exists(), "the daemon creates it, with its own modes");
}

/// `current` must never show a change older than one asked for, even while the writer is saving the newer one: found
/// on Windows CI, where a save landing between `current`'s read and its look at the queue lost the newer change.
#[test]
fn current_never_goes_back_while_a_write_lands() {
    let e = env();
    let path = gui_settings_path(&e.home);
    GuiSettings::default().save(&path).unwrap();
    let w = SettingsWriter::shared();
    for i in 0..2000 {
        let want = if i % 2 == 0 {
            ThemeChoice::Dark
        } else {
            ThemeChoice::Light
        };
        w.update(&path, move |s| s.theme = want, |_| {});
        assert_eq!(w.current(&path).theme, want, "round {i}");
    }
    w.flush();
}

#[test]
fn background_writes_land_in_order_and_keep_other_fields() {
    let e = env();
    let path = gui_settings_path(&e.home);
    GuiSettings {
        last_pick_dir: Some("/x".into()),
        ..Default::default()
    }
    .save(&path)
    .unwrap();
    let w = SettingsWriter::shared();
    w.update(&path, |s| s.theme = ThemeChoice::Light, |_| {});
    w.update(&path, |s| s.theme = ThemeChoice::Dark, |_| {});
    w.update(
        &path,
        |s| s.close_choice = Some(CloseChoice::KeepRunning),
        |_| {},
    );
    // Seen at once, before anything is known to be on disk.
    assert_eq!(w.current(&path).theme, ThemeChoice::Dark);
    w.flush();
    let saved = GuiSettings::load(&path);
    assert_eq!(
        (saved.theme, saved.close_choice),
        (ThemeChoice::Dark, Some(CloseChoice::KeepRunning))
    );
    assert_eq!(
        saved.last_pick_dir.as_deref(),
        Some(Path::new("/x")),
        "a field nobody changed was lost"
    );
}

#[test]
fn a_failed_background_write_is_reported_and_forgotten() {
    let e = env();
    // A file where the parent folder should be: the write cannot succeed.
    std::fs::write(e.home.join("blocker"), "").unwrap();
    let path = e.home.join("blocker/gui.toml");
    let (tx, rx) = std::sync::mpsc::channel();
    SettingsWriter::shared().update(
        &path,
        |s| s.theme = ThemeChoice::Dark,
        move |r| tx.send(r.is_err()).unwrap(),
    );
    assert!(
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
        "the failure was not reported"
    );
    assert_eq!(
        SettingsWriter::shared().current(&path).theme,
        ThemeChoice::System,
        "a failed change is still applied"
    );
}

#[test]
fn config_arg_takes_both_spellings() {
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        config_arg(args(&["--config", "/a.toml"])),
        Some("/a.toml".into())
    );
    assert_eq!(
        config_arg(args(&["--config=/b c.toml"])),
        Some("/b c.toml".into())
    );
    assert_eq!(config_arg(args(&["--version"])), None);
    assert_eq!(config_arg(args(&["--config"])), None);
}

#[test]
fn gui_settings_keeps_fleet_display_and_reads_an_unknown_one_as_compact() {
    use fleet_client::config::FleetDisplay;
    let e = env();
    let p = gui_settings_path(&e.home);
    assert_eq!(GuiSettings::load(&p).fleet_display, FleetDisplay::Compact);
    GuiSettings {
        fleet_display: FleetDisplay::Hide,
        ..Default::default()
    }
    .save(&p)
    .unwrap();
    assert!(std::fs::read_to_string(&p)
        .unwrap()
        .contains("fleet_display = \"hide\""));
    assert_eq!(GuiSettings::load(&p).fleet_display, FleetDisplay::Hide);
    std::fs::write(&p, "theme = \"dark\"\nfleet_display = \"sideways\"\n").unwrap();
    let s = GuiSettings::load(&p);
    assert_eq!(
        (s.theme, s.fleet_display),
        (ThemeChoice::Dark, FleetDisplay::Compact)
    );
}

#[test]
fn projects_root_is_refused_with_its_new_name() {
    let e = env();
    let p = write(&e, "projects_root = \"~/x\"\n");
    let err = load(&p, &e.home).unwrap_err().to_string();
    assert!(
        err.contains("projects_root was renamed to tasks_root; rename it in"),
        "{err}"
    );
}

#[test]
fn tasks_root_defaults_to_fleet_tasks() {
    let e = env();
    let c = load(&e.home.join("missing.toml"), &e.home).unwrap();
    assert!(c.tasks_root.ends_with("fleet-tasks"));
}

#[test]
fn fonts_default_to_plex_without_the_keys() {
    use fleet_client::config::{MonoFont, UiFont};
    let e = env();
    let p = gui_settings_path(&e.home);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "theme = \"dark\"\n").unwrap();
    let s = GuiSettings::load(&p);
    assert_eq!((s.ui_font, s.mono_font), (UiFont::Plex, MonoFont::Plex));
}

#[test]
fn unknown_font_reads_as_plex_and_keeps_the_rest() {
    use fleet_client::config::{MonoFont, UiFont};
    let e = env();
    let p = gui_settings_path(&e.home);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    for ui in ["\"geist\"", "3"] {
        std::fs::write(
            &p,
            format!("ui_font = {ui}\nmono_font = \"jetbrains\"\ntheme = \"dark\"\n"),
        )
        .unwrap();
        let s = GuiSettings::load(&p);
        assert_eq!(
            (s.ui_font, s.mono_font, s.theme),
            (UiFont::Plex, MonoFont::JetBrains, ThemeChoice::Dark),
            "ui_font = {ui}"
        );
    }
}

#[test]
fn fonts_round_trip() {
    use fleet_client::config::{MonoFont, UiFont};
    let e = env();
    let p = gui_settings_path(&e.home);
    for (ui, mono) in UiFont::ALL.into_iter().zip(MonoFont::ALL) {
        GuiSettings::update(&p, |s| {
            s.ui_font = ui;
            s.mono_font = mono;
        })
        .unwrap();
        let s = GuiSettings::load(&p);
        assert_eq!((s.ui_font, s.mono_font), (ui, mono));
    }
    let on_disk = std::fs::read_to_string(&p).unwrap();
    assert!(on_disk.contains("ui_font = \"atkinson\""), "{on_disk}");
    assert!(on_disk.contains("mono_font = \"atkinson\""), "{on_disk}");
    GuiSettings::update(&p, |s| s.mono_font = MonoFont::JetBrains).unwrap();
    let on_disk = std::fs::read_to_string(&p).unwrap();
    assert!(on_disk.contains("mono_font = \"jetbrains\""), "{on_disk}");
}

#[test]
fn fonts_name_themselves() {
    use fleet_client::config::{MonoFont, UiFont};
    assert_eq!(
        UiFont::ALL.map(UiFont::label),
        [
            "IBM Plex Sans",
            "Space Grotesk",
            "Atkinson Hyperlegible Next"
        ]
    );
    assert_eq!(
        MonoFont::ALL.map(MonoFont::label),
        [
            "IBM Plex Mono",
            "JetBrains Mono",
            "Atkinson Hyperlegible Mono"
        ]
    );
}

#[test]
fn animations_default_on_without_the_key() {
    let e = env();
    let p = gui_settings_path(&e.home);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "theme = \"dark\"\n").unwrap();
    assert!(GuiSettings::load(&p).animations);
    assert!(GuiSettings::default().animations);
    GuiSettings::update(&p, |s| s.animations = false).unwrap();
    assert!(!GuiSettings::load(&p).animations);
}

#[test]
fn limits_default_to_the_daemons() {
    let e = env();
    let c = load(&e.home.join("nope.toml"), &e.home).unwrap();
    assert_eq!(c.limits, Limits::default());
    assert_eq!(
        (
            c.limits.max_breakdown_versions,
            c.limits.max_steps,
            c.limits.max_parallel_specialists
        ),
        (8, 20, 3)
    );
}

#[test]
fn reads_the_limits() {
    let e = env();
    let p = write(&e, "max_steps = 30\nmax_parallel_specialists = 5\n");
    let l = load(&p, &e.home).unwrap().limits;
    assert_eq!(
        (
            l.max_breakdown_versions,
            l.max_steps,
            l.max_parallel_specialists
        ),
        (8, 30, 5)
    );
}

#[test]
fn set_limits_keeps_comments_and_other_keys() {
    let e = env();
    let p = write(
        &e,
        "# mine\ntasks_root = \"~/p\"\nmax_steps = 30 # tried 30\n\n[[repos]]\nname = \"core\"\npath = \"~/dev/core\"\n",
    );
    let limits = Limits {
        max_breakdown_versions: 4,
        max_steps: 50,
        max_parallel_specialists: 2,
        max_pr_attempts: 5,
    };
    set_limits(&p, &limits, &e.home).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.contains("max_steps = 50 # tried 30\n"), "{text}");
    assert!(text.starts_with("# mine\ntasks_root = \"~/p\"\n"), "{text}");
    assert!(text.contains("[[repos]]\nname = \"core\""), "{text}");
    let c = load(&p, &e.home).unwrap();
    assert_eq!(c.limits, limits);
    assert_eq!(c.tasks_root, e.home.join("p"));
}

#[test]
fn set_limits_writes_only_the_keys_that_changed() {
    let e = env();
    let p = write(&e, "tasks_root = \"~/p\"\nmax_steps = 30\n");
    let limits = Limits {
        max_steps: 30,
        max_breakdown_versions: 4,
        ..Limits::default()
    };
    set_limits(&p, &limits, &e.home).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.contains("max_breakdown_versions = 4\n"), "{text}");
    assert!(text.contains("max_steps = 30\n"), "{text}");
    assert!(
        !text.contains("max_parallel_specialists"),
        "an untouched default stays absent: {text}"
    );
    assert_eq!(load(&p, &e.home).unwrap().limits, limits);
}

#[test]
fn set_limits_writes_a_default_over_a_value_the_file_holds() {
    let e = env();
    let p = write(&e, "tasks_root = \"~/p\"\nmax_steps = 30\n");
    set_limits(&p, &Limits::default(), &e.home).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(
        text.contains(&format!("max_steps = {}\n", Limits::default().max_steps)),
        "{text}"
    );
    assert!(!text.contains("max_breakdown_versions"), "{text}");
}

#[test]
fn set_limits_creates_a_missing_file() {
    let e = env();
    let p = fleet_fs::dirs::config_dir(&e.home).join("config.toml");
    let limits = Limits {
        max_breakdown_versions: 1,
        ..Limits::default()
    };
    set_limits(&p, &limits, &e.home).unwrap();
    assert_eq!(load(&p, &e.home).unwrap().limits, limits);
}

#[test]
fn pr_settings_default_read_and_write_back() {
    let e = env();
    let p = write(&e, "tasks_root = \"~/p\"\n");
    assert_eq!(load(&p, &e.home).unwrap().pr, PrSettings::default());
    let pr = PrSettings {
        style: "conventional".into(),
        attribution: "Made with Fleet Manager".into(),
        draft: true,
        labels: vec!["fleet".into(), "bot".into()],
    };
    set_pr(&p, &pr, &e.home).unwrap();
    assert_eq!(load(&p, &e.home).unwrap().pr, pr);
    set_pr(&p, &PrSettings::default(), &e.home).unwrap();
    assert_eq!(load(&p, &e.home).unwrap().pr, PrSettings::default());
}

#[test]
fn set_pr_does_not_pin_defaults() {
    let e = env();
    let p = write(&e, "tasks_root = \"~/p\"\n");
    set_pr(&p, &PrSettings::default(), &e.home).unwrap();
    assert!(!std::fs::read_to_string(&p).unwrap().contains("pr_"));
}

#[test]
fn labels_are_trimmed_and_empty_ones_dropped() {
    assert_eq!(parse_labels(" a, ,b ,"), ["a", "b"]);
    assert!(parse_labels("").is_empty());
}

#[test]
fn set_limits_refuses_a_file_that_is_not_toml() {
    let e = env();
    let p = write(&e, "this is = = not toml");
    assert!(set_limits(&p, &Limits::default(), &e.home).is_err());
}

#[test]
fn a_limit_is_a_positive_whole_number() {
    assert_eq!(parse_limit(" 12 "), Some(12));
    for bad in ["", "0", "-1", "2.5", "many"] {
        assert_eq!(parse_limit(bad), None, "{bad:?}");
    }
}

#[test]
fn agent_defaults_are_auto_and_medium_and_write_back_keeping_comments() {
    let e = env();
    let p = write(&e, "# my notes\ntasks_root = \"~/p\"\n");
    let c = load(&p, &e.home).unwrap();
    assert_eq!(
        (c.default_mode, c.default_effort),
        (PermissionMode::Auto, Effort::Medium)
    );
    set_agent_defaults(&p, PermissionMode::Manual, Effort::High, &e.home).unwrap();
    let c = load(&p, &e.home).unwrap();
    assert_eq!(
        (c.default_mode, c.default_effort),
        (PermissionMode::Manual, Effort::High)
    );
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.contains("default_effort = \"high\""), "{text}");
    assert!(text.contains("# my notes"), "{text}");
}

#[test]
fn set_agent_defaults_does_not_pin_defaults() {
    let e = env();
    let p = write(&e, "tasks_root = \"~/p\"\n");
    set_agent_defaults(&p, PermissionMode::Auto, Effort::Medium, &e.home).unwrap();
    assert!(!std::fs::read_to_string(&p).unwrap().contains("default_"));
}

fn with_listen(e: &Env, listen: &str) -> fleet_client::config::ClientConfig {
    let p = write(
        e,
        &format!(
            "listen = \"{listen}\"\nstate_dir = {:?}\n",
            e.home.join("state").display().to_string()
        ),
    );
    std::fs::create_dir_all(e.home.join("state")).unwrap();
    load(&p, &e.home).unwrap()
}

#[test]
fn port_zero_is_a_valid_listen_and_base_url_follows_the_addr_file() {
    let e = env();
    let cfg = with_listen(&e, "127.0.0.1:0");
    assert_eq!(cfg.base_url(), "http://127.0.0.1:0");
    std::fs::write(e.home.join("state/daemon.addr"), "127.0.0.1:43123\n").unwrap();
    assert_eq!(cfg.base_url(), "http://127.0.0.1:43123");
}

#[test]
fn port_zero_with_an_unreadable_addr_file_keeps_the_configured_address() {
    let e = env();
    let cfg = with_listen(&e, "127.0.0.1:0");
    for bad in ["", "garbage", "0.0.0.0:80", "127.0.0.1:0"] {
        std::fs::write(e.home.join("state/daemon.addr"), bad).unwrap();
        assert_eq!(cfg.base_url(), "http://127.0.0.1:0", "{bad:?}");
    }
}

#[test]
fn a_fixed_port_ignores_the_addr_file() {
    let e = env();
    let cfg = with_listen(&e, "127.0.0.1:9000");
    std::fs::write(e.home.join("state/daemon.addr"), "127.0.0.1:43123\n").unwrap();
    assert_eq!(cfg.base_url(), "http://127.0.0.1:9000");
}
