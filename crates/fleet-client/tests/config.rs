use fleet_client::config::{default_path, gui_settings_path, load, CloseChoice, ConfigError, GuiSettings};
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
    assert_eq!(c.state_dir, e.home.join(".local/share/fleet-board"));
    assert_eq!(c.pidfile(), e.home.join(".local/share/fleet-board/daemon.pid"));
    assert_eq!(c.log_file(), e.home.join(".local/share/fleet-board/daemon.log"));
}

#[test]
fn an_ipv6_loopback_listen_address_gets_a_bracketed_url() {
    let e = env();
    let c = load(&write(&e, "listen = \"[::1]:7878\"\n"), &e.home).unwrap();
    assert_eq!(c.base_url(), "http://[::1]:7878");
}

#[test]
fn a_missing_file_names_the_path() {
    let e = env();
    let err = load(&e.home.join("nope.toml"), &e.home).unwrap_err();
    assert!(matches!(err, ConfigError::Io(..)));
    assert!(err.to_string().contains("nope.toml"), "{err}");
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
    assert_eq!(default_path(home), Path::new("/home/u/.config/fleet-board/config.toml"));
    assert_eq!(gui_settings_path(home), Path::new("/home/u/.config/fleet-board/gui.toml"));
}

#[test]
fn gui_settings_round_trip_and_create_the_directory() {
    let e = env();
    let p = gui_settings_path(&e.home);
    assert_eq!(GuiSettings::load(&p), GuiSettings::default(), "missing file is the default");
    let s = GuiSettings { close_choice: Some(CloseChoice::StopDaemon) };
    s.save(&p).unwrap();
    assert_eq!(GuiSettings::load(&p), s);
    GuiSettings { close_choice: None }.save(&p).unwrap();
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
