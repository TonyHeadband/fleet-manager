use fleet_client::config::{default_path, exists, gui_settings_path, load, write_initial, CloseChoice, ConfigError, GuiSettings, ThemeChoice};
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
claude_command = ["claude", "--permission-mode", "default"]
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
    let c = load(&write(&e, "[[repos]]\nname = \"core\"\npath = \"~/x\"\n"), &e.home).unwrap();
    assert_eq!(c.listen.to_string(), "127.0.0.1:7878");
    assert_eq!(c.state_dir, e.home.join(".local/share/fleet-manager"));
    assert_eq!(c.pidfile(), e.home.join(".local/share/fleet-manager/daemon.pid"));
    assert_eq!(c.log_file(), e.home.join(".local/share/fleet-manager/daemon.log"));
}

#[test]
fn an_ipv6_loopback_listen_address_gets_a_bracketed_url() {
    let e = env();
    let c = load(&write(&e, "listen = \"[::1]:7878\"\n"), &e.home).unwrap();
    assert_eq!(c.base_url(), "http://[::1]:7878");
}

#[test]
fn missing_file_is_defaults() {
    let e = env();
    let p = e.home.join("nope.toml");
    assert!(!exists(&p));
    let c = load(&p, &e.home).unwrap();
    assert_eq!(c.listen.to_string(), "127.0.0.1:7878");
    assert_eq!(c.projects_root, e.home.join("fleet-projects"));
    assert_eq!(c.state_dir, e.home.join(".local/share/fleet-manager"));
}

#[test]
fn write_initial_then_load_round_trips_the_root() {
    let e = env();
    let p = e.home.join("cfg/dir/config.toml");
    let root = e.home.join("my projects");
    write_initial(&p, &root, &e.home).unwrap();
    assert!(exists(&p));
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.starts_with("projects_root = \"~/my projects\"\n"), "{text}");
    assert_eq!(load(&p, &e.home).unwrap().projects_root, root);
}

#[test]
fn write_initial_writes_a_root_outside_home_as_is() {
    let e = env();
    let p = e.home.join("config.toml");
    write_initial(&p, Path::new("/srv/fleet"), &e.home).unwrap();
    assert_eq!(load(&p, &e.home).unwrap().projects_root, Path::new("/srv/fleet"));
}

#[test]
fn write_initial_never_overwrites_an_existing_config() {
    let e = env();
    let p = write(&e, "branch_prefix = \"mine\"\n");
    let err = write_initial(&p, &e.home.join("fleet-projects"), &e.home).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "branch_prefix = \"mine\"\n");
}

#[test]
fn write_initial_keeps_the_template_comments() {
    let e = env();
    let p = e.home.join("config.toml");
    write_initial(&p, &e.home.join("fleet-projects"), &e.home).unwrap();
    assert!(std::fs::read_to_string(&p).unwrap().contains("# claude_command"));
}

#[test]
fn invalid_toml_names_the_path() {
    let e = env();
    let err = load(&write(&e, "listen = = ="), &e.home).unwrap_err();
    assert!(matches!(err, ConfigError::Parse(..)));
    assert!(err.to_string().contains("config.toml"), "{err}");
}

#[test]
fn default_paths_are_under_dot_config() {
    let home = Path::new("/home/u");
    assert_eq!(default_path(home), Path::new("/home/u/.config/fleet-manager/config.toml"));
    assert_eq!(gui_settings_path(home), Path::new("/home/u/.config/fleet-manager/gui.toml"));
}

#[test]
fn gui_settings_round_trip_and_create_the_directory() {
    let e = env();
    let p = gui_settings_path(&e.home);
    assert_eq!(GuiSettings::load(&p), GuiSettings::default(), "missing file is the default");
    let s = GuiSettings { close_choice: Some(CloseChoice::StopDaemon), ..Default::default() };
    s.save(&p).unwrap();
    assert_eq!(GuiSettings::load(&p), s);
    GuiSettings { close_choice: None, ..Default::default() }.save(&p).unwrap();
    assert_eq!(GuiSettings::load(&p).close_choice, None);
}

#[test]
fn corrupt_gui_settings_fall_back_to_the_default() {
    let e = env();
    let p = gui_settings_path(&e.home);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    for bad in ["not toml at all = = =", "close_choice = \"explode\"", "close_choice = 7", "\u{0}\u{1}"] {
        std::fs::write(&p, bad).unwrap();
        assert_eq!(GuiSettings::load(&p), GuiSettings::default(), "{bad:?}");
    }
}

#[test]
fn theme_choice_cycles_and_names_itself() {
    assert_eq!(ThemeChoice::System.next(), ThemeChoice::Light);
    assert_eq!(ThemeChoice::Light.next(), ThemeChoice::Dark);
    assert_eq!(ThemeChoice::Dark.next(), ThemeChoice::System);
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
    assert!(GuiSettings::load(&p).sticky_alerts, "a file written before the key existed reads as on");
    GuiSettings::update(&p, |s| s.sticky_alerts = false).unwrap();
    let s = GuiSettings::load(&p);
    assert!(!s.sticky_alerts);
    assert_eq!(s.theme, ThemeChoice::Dark);
}
