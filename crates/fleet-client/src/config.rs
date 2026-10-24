//! What the client needs from `~/.config/fleet-board/config.toml` (the daemon's own file, so every key the
//! client does not use is ignored) and its own small settings file, `gui.toml`.
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum ConfigError {
    Io(PathBuf, std::io::Error),
    Parse(PathBuf, toml::de::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(p, e) => write!(f, "cannot read config {}: {e}", p.display()),
            ConfigError::Parse(p, e) => write!(f, "invalid config {}: {e}", p.display()),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    pub listen: SocketAddr,
    pub state_dir: PathBuf,
    /// The file this was read from; handed to a daemon the client starts.
    pub path: PathBuf,
}

impl ClientConfig {
    pub fn base_url(&self) -> String {
        format!("http://{}", self.listen)
    }

    pub fn pidfile(&self) -> PathBuf {
        self.state_dir.join("daemon.pid")
    }

    pub fn log_file(&self) -> PathBuf {
        self.state_dir.join("daemon.log")
    }
}

#[derive(Deserialize)]
struct Raw {
    listen: Option<SocketAddr>,
    state_dir: Option<String>,
}

pub fn default_path(home: &Path) -> PathBuf {
    home.join(".config/fleet-board/config.toml")
}

pub fn gui_settings_path(home: &Path) -> PathBuf {
    home.join(".config/fleet-board/gui.toml")
}

pub fn load(path: &Path, home: &Path) -> Result<ClientConfig, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Io(path.to_path_buf(), e))?;
    let raw: Raw = toml::from_str(&text).map_err(|e| ConfigError::Parse(path.to_path_buf(), e))?;
    let state_dir = match raw.state_dir.as_deref() {
        Some(s) => match s.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None if s == "~" => home.to_path_buf(),
            None => PathBuf::from(s),
        },
        None => home.join(".local/share/fleet-board"),
    };
    Ok(ClientConfig {
        listen: raw.listen.unwrap_or_else(|| "127.0.0.1:7878".parse().expect("static address")),
        state_dir,
        path: path.to_path_buf(),
    })
}

/// What closing the window does to the daemon, when the user has said so once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseChoice {
    KeepRunning,
    StopDaemon,
}

/// Which look the window has; `System` follows the desktop's light/dark setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    Light,
    Dark,
    /// Also what an unknown value reads as, so a mistyped theme cannot cost the saved close choice. Last because
    /// serde only takes `other` on the last variant.
    #[default]
    #[serde(other)]
    System,
}

impl ThemeChoice {
    pub fn next(self) -> ThemeChoice {
        match self {
            ThemeChoice::System => ThemeChoice::Light,
            ThemeChoice::Light => ThemeChoice::Dark,
            ThemeChoice::Dark => ThemeChoice::System,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "System",
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GuiSettings {
    #[serde(default)]
    pub close_choice: Option<CloseChoice>,
    #[serde(default)]
    pub theme: ThemeChoice,
}

impl GuiSettings {
    /// A missing or unreadable file is just the defaults: settings must never stop the app from opening.
    pub fn load(path: &Path) -> GuiSettings {
        std::fs::read_to_string(path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, toml::to_string(self).map_err(std::io::Error::other)?)
    }

    /// Re-reads the file, applies `change` and saves, so a writer holding an older copy of the settings cannot
    /// overwrite a field another part of the window saved since.
    pub fn update(path: &Path, change: impl FnOnce(&mut GuiSettings)) -> std::io::Result<GuiSettings> {
        let mut settings = GuiSettings::load(path);
        change(&mut settings);
        settings.save(path)?;
        Ok(settings)
    }
}
