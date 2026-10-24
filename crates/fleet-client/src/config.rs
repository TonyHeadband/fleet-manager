//! What the client needs from `~/.config/fleet-manager/config.toml` (the daemon's own file, so every key the
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
    pub projects_root: PathBuf,
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

#[derive(Deserialize, Default)]
struct Raw {
    listen: Option<SocketAddr>,
    state_dir: Option<String>,
    projects_root: Option<String>,
}

pub fn default_path(home: &Path) -> PathBuf {
    home.join(".config/fleet-manager/config.toml")
}

pub fn gui_settings_path(home: &Path) -> PathBuf {
    home.join(".config/fleet-manager/gui.toml")
}

pub fn exists(path: &Path) -> bool {
    path.is_file()
}

/// `~` and `~/…` become paths under `home`; anything else is taken as written.
pub fn expand_home(p: &str, home: &Path) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if p == "~" => home.to_path_buf(),
        None => PathBuf::from(p),
    }
}

/// A missing file is the defaults, the same as in the daemon, so the app opens before any setup.
pub fn load(path: &Path, home: &Path) -> Result<ClientConfig, ConfigError> {
    let raw: Raw = match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|e| ConfigError::Parse(path.to_path_buf(), e))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Raw::default(),
        Err(e) => return Err(ConfigError::Io(path.to_path_buf(), e)),
    };
    let state_dir = match raw.state_dir.as_deref() {
        Some(s) => expand_home(s, home),
        None => home.join(".local/share/fleet-manager"),
    };
    let projects_root = match raw.projects_root.as_deref() {
        Some(s) => expand_home(s, home),
        None => home.join("fleet-projects"),
    };
    Ok(ClientConfig {
        listen: raw.listen.unwrap_or_else(|| "127.0.0.1:7878".parse().expect("static address")),
        state_dir,
        projects_root,
        path: path.to_path_buf(),
    })
}

/// A path under `home` written with `~`, the way the config file and the user write it.
pub fn shorten_home(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// First-run file: the chosen root on top, then the commented template so every other key stays discoverable.
pub fn write_initial(path: &Path, projects_root: &Path, home: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let shown = shorten_home(projects_root, home);
    // Debug formatting gives a valid TOML basic string for any path with quotes or backslashes.
    let body = format!("projects_root = {shown:?}\n\n{}", include_str!("../../../packaging/config.example.toml"));
    // create_new so a hand-edited config is never overwritten, and two first runs cannot race.
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    f.write_all(body.as_bytes())
}

/// The projects folder the user typed, ready to be written: trimmed, `~` expanded, absolute and existing. Any
/// failure is a message for the user.
pub fn prepare_projects_root(typed: &str, home: &Path) -> Result<PathBuf, String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Err("Enter a folder for the projects".into());
    }
    let dir = expand_home(typed, home);
    if !dir.is_absolute() {
        return Err(format!("{} is not an absolute path", dir.display()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Points the daemon's config at `root`, changing only `projects_root`: the file is the user's, with their
/// comments and every key the client does not know. A missing file gets the first-run file.
pub fn set_projects_root(config: &Path, root: &Path, home: &Path) -> std::io::Result<()> {
    let text = match std::fs::read_to_string(config) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return write_initial(config, root, home),
        Err(e) => return Err(e),
    };
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("invalid config {}: {e}", config.display())))?;
    let shown = shorten_home(root, home);
    match doc.get_mut("projects_root").and_then(toml_edit::Item::as_value_mut) {
        // Swapping the value alone keeps the comment after it on the same line.
        Some(value) => {
            let decor = value.decor().clone();
            *value = shown.into();
            *value.decor_mut() = decor;
        }
        None => {
            doc.insert("projects_root", toml_edit::value(shown));
        }
    }
    std::fs::write(config, doc.to_string())
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuiSettings {
    #[serde(default)]
    pub close_choice: Option<CloseChoice>,
    #[serde(default)]
    pub theme: ThemeChoice,
    /// The folder the folder picker last selected from; it opens there next time.
    #[serde(default)]
    pub last_pick_dir: Option<PathBuf>,
    /// Errors and warnings stay in the footer, cycling with the other messages, until clicked away.
    #[serde(default = "yes")]
    pub sticky_alerts: bool,
}

fn yes() -> bool {
    true
}

// Not derived: `bool` defaults to false, and a settings file without the key must read as sticky.
impl Default for GuiSettings {
    fn default() -> GuiSettings {
        GuiSettings { close_choice: None, theme: ThemeChoice::default(), last_pick_dir: None, sticky_alerts: true }
    }
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
