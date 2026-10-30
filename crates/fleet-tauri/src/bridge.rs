//! The native bridge: what the web UI asks of the machine it runs on, when it runs in the Tauri app. Each command
//! calls `fleet-client`, which holds the decisions the egui app made, so the two apps decide alike.
use fleet_client::config::{self, ClientConfig};
use std::path::{Path, PathBuf};

/// Where this window's config lives and whose home it expands `~` against.
#[derive(Clone, Debug)]
pub struct Paths {
    pub config: PathBuf,
    pub home: PathBuf,
}

impl Paths {
    /// `--config PATH` when given, else the default config of the user running the app.
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Paths {
        let home = fleet_fs::dirs::home_dir();
        let config = config::config_arg(args).unwrap_or_else(|| config::default_path(&home));
        Paths { config, home }
    }

    /// The config as it is now; a broken one gives the defaults, as egui's window does until Retry.
    pub fn load(&self) -> ClientConfig {
        config::load(&self.config, &self.home)
            .unwrap_or_else(|_| config::defaults(&self.config, &self.home))
    }
}

/// The daemon's address the web UI talks to: the configured one, or the one it wrote to `daemon.addr` on port 0.
pub fn daemon_url_of(paths: &Paths) -> String {
    paths.load().base_url()
}

#[tauri::command]
pub fn daemon_url(paths: tauri::State<'_, Paths>) -> String {
    daemon_url_of(&paths)
}

#[tauri::command]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Whether the config file exists: without one, this is a first run.
#[tauri::command]
pub fn has_config(paths: tauri::State<'_, Paths>) -> bool {
    config::exists(&paths.config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths_in(dir: &Path, toml: Option<&str>) -> Paths {
        let config = dir.join("config.toml");
        if let Some(t) = toml {
            std::fs::write(&config, t).unwrap();
        }
        Paths {
            config,
            home: dir.to_path_buf(),
        }
    }

    #[test]
    fn daemon_url_follows_the_config_and_daemon_addr() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            daemon_url_of(&paths_in(dir.path(), None)),
            "http://127.0.0.1:7878",
            "no config: the default"
        );
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let toml = format!(
            "listen = \"127.0.0.1:0\"\nstate_dir = \"{}\"\n",
            state.display()
        );
        let paths = paths_in(dir.path(), Some(&toml));
        std::fs::write(state.join("daemon.addr"), "127.0.0.1:45123").unwrap();
        assert_eq!(
            daemon_url_of(&paths),
            "http://127.0.0.1:45123",
            "port 0: the address the daemon wrote"
        );
    }

    #[test]
    fn a_broken_config_falls_back_to_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            daemon_url_of(&paths_in(dir.path(), Some("listen = ["))),
            "http://127.0.0.1:7878"
        );
    }
}
