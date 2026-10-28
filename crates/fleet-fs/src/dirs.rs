//! Where Fleet keeps its files, per OS, all under the user's home folder so a fake home tests every layout:
//! Linux and macOS `~/.config/fleet-manager` and `~/.local/share/fleet-manager`; Windows
//! `~\AppData\Roaming\fleet-manager` and `~\AppData\Local\fleet-manager`.
use std::path::{Path, PathBuf};

/// The user's home folder: `%USERPROFILE%` on Windows, `$HOME` elsewhere. `/` only when the system has none.
pub fn home_dir() -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// Config folder (config.toml, gui.toml, agent/, fleet/).
pub fn config_dir(home: &Path) -> PathBuf {
    layout(home, cfg!(windows)).0
}

/// Data folder (state: database, daemon.pid, daemon.log; personas).
pub fn data_dir(home: &Path) -> PathBuf {
    layout(home, cfg!(windows)).1
}

/// Both folders for either OS, so the tests cover the Windows layout on Linux too.
fn layout(home: &Path, windows: bool) -> (PathBuf, PathBuf) {
    if windows {
        let app = home.join("AppData");
        (
            app.join("Roaming").join("fleet-manager"),
            app.join("Local").join("fleet-manager"),
        )
    } else {
        (
            home.join(".config/fleet-manager"),
            home.join(".local/share/fleet-manager"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_layout() {
        let h = Path::new("/home/u");
        assert_eq!(
            layout(h, false),
            (
                h.join(".config/fleet-manager"),
                h.join(".local/share/fleet-manager")
            )
        );
    }

    #[test]
    fn windows_layout() {
        let h = Path::new("C:/Users/u");
        assert_eq!(
            layout(h, true),
            (
                h.join("AppData").join("Roaming").join("fleet-manager"),
                h.join("AppData").join("Local").join("fleet-manager")
            )
        );
    }

    #[test]
    fn the_home_folder_is_never_the_root_fallback_on_a_normal_machine() {
        // On Windows `HOME` is unset; the old lookup fell back to "/".
        assert_ne!(home_dir(), PathBuf::from("/"));
        assert!(home_dir().is_absolute());
    }
}
