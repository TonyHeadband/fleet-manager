//! The native bridge: what the web UI asks of the machine it runs on, when it runs in the Tauri app. Each command
//! calls `fleet-client`, which holds the decisions the egui app made, so the two apps decide alike.
use fleet_client::config::{self, ClientConfig};
use fleet_client::daemon_control::{connect_or_start, find_daemon_binary, Started};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

#[tauri::command]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Whether the config file exists: without one, this is a first run.
#[tauri::command]
pub fn has_config(paths: tauri::State<'_, Paths>) -> bool {
    config::exists(&paths.config)
}

/// The daemon this window started, if it did: only that one is stopped on close without asking.
#[derive(Default, Clone)]
pub struct Owner(Arc<Mutex<Option<u32>>>);

impl Owner {
    pub fn pid(&self) -> Option<u32> {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StartReply {
    pub we_started: bool,
    /// The daemon's address: on port 0, the one it wrote to `daemon.addr` once up.
    pub url: String,
}

/// Connects to the daemon or starts it, as egui's window does: a config that can't be read stops here with its
/// reason, where the other commands fall back to the defaults.
pub fn start_with(
    paths: &Paths,
    owner: &Owner,
    find: impl FnOnce() -> Option<PathBuf>,
) -> Result<StartReply, String> {
    let cfg = config::load(&paths.config, &paths.home).map_err(|e| e.to_string())?;
    let started =
        connect_or_start(&cfg, find, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    let pid = match started {
        Started::AlreadyRunning => None,
        Started::StartedByUs(pid) => Some(pid),
    };
    // A Retry that connects to a daemon already up keeps the owner a previous attempt recorded.
    if pid.is_some() {
        *owner.0.lock().unwrap_or_else(|e| e.into_inner()) = pid;
    }
    Ok(StartReply {
        we_started: owner.pid().is_some(),
        url: cfg.base_url(),
    })
}

/// Blocks for up to the start's wait, so it runs off the async runtime's workers.
#[tauri::command]
pub async fn start_daemon(
    paths: tauri::State<'_, Paths>,
    owner: tauri::State<'_, Owner>,
) -> Result<StartReply, String> {
    let (paths, owner) = (paths.inner().clone(), owner.inner().clone());
    tauri::async_runtime::spawn_blocking(move || start_with(&paths, &owner, find_daemon_binary))
        .await
        .map_err(|e| e.to_string())?
}

/// Where the daemon keeps its state, shown on the failure screen.
#[tauri::command]
pub fn state_dir(paths: tauri::State<'_, Paths>) -> String {
    paths.load().state_dir.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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
    fn a_running_daemon_on_port_0_is_reached_at_the_address_it_wrote() {
        let dir = tempfile::tempdir().unwrap();
        // Answers `is_up` as a daemon would.
        let daemon = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = daemon.local_addr().unwrap();
        std::thread::spawn(move || {
            for mut s in daemon.incoming().flatten() {
                let _ = std::io::Read::read(&mut s, &mut [0; 512]);
                let _ = std::io::Write::write_all(
                    &mut s,
                    b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n",
                );
            }
        });
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(state.join("daemon.addr"), addr.to_string()).unwrap();
        let toml = format!(
            "listen = \"127.0.0.1:0\"\nstate_dir = \"{}\"\n",
            state.display()
        );
        let reply = start_with(
            &paths_in(dir.path(), Some(&toml)),
            &Owner::default(),
            || None,
        )
        .unwrap();
        assert_eq!(
            reply,
            StartReply {
                we_started: false,
                url: format!("http://{addr}")
            }
        );
    }

    #[test]
    fn a_broken_config_stops_the_start_with_its_reason() {
        let dir = tempfile::tempdir().unwrap();
        let err = start_with(
            &paths_in(dir.path(), Some("listen = [")),
            &Owner::default(),
            || None,
        )
        .unwrap_err();
        assert!(err.contains("config.toml"), "{err}");
    }

    #[test]
    fn no_daemon_and_no_binary_says_so() {
        let dir = tempfile::tempdir().unwrap();
        // Port 1 never answers, so the start looks for a binary and finds none.
        let toml = format!(
            "listen = \"127.0.0.1:1\"\nstate_dir = \"{}\"\n",
            dir.path().join("state").display()
        );
        let owner = Owner::default();
        let err = start_with(&paths_in(dir.path(), Some(&toml)), &owner, || None).unwrap_err();
        assert_eq!(
            err,
            "fleet-daemon was not found next to fleet-gui or on PATH"
        );
        assert_eq!(
            owner.pid(),
            None,
            "nothing was started, so this window owns no daemon"
        );
    }

    #[test]
    fn a_broken_config_falls_back_to_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path(), Some("listen = ["));
        let defaults = config::defaults(&paths.config, &paths.home);
        assert_eq!(paths.load().state_dir, defaults.state_dir);
    }

    #[test]
    fn the_state_dir_is_the_configs() {
        let dir = tempfile::tempdir().unwrap();
        let toml = format!("state_dir = \"{}\"\n", dir.path().join("st").display());
        assert_eq!(
            paths_in(dir.path(), Some(&toml)).load().state_dir,
            dir.path().join("st")
        );
    }
}
