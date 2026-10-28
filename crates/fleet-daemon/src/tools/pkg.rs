//! Package-manager detection.

use fleet_api::Os;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manager {
    Winget,
    Apt,
    Dnf,
    Pacman,
    Zypper,
}

impl Manager {
    /// The executable looked up on PATH.
    pub fn program(self) -> &'static str {
        match self {
            Manager::Winget => "winget",
            Manager::Apt => "apt-get",
            Manager::Dnf => "dnf",
            Manager::Pacman => "pacman",
            Manager::Zypper => "zypper",
        }
    }

    /// The name shown to people and stored in `ToolFix.manager`.
    pub fn label(self) -> &'static str {
        match self {
            Manager::Apt => "apt",
            other => other.program(),
        }
    }
}

/// Windows: winget if present. Linux: the first of apt-get, dnf, pacman, zypper on PATH.
pub fn detect_manager(os: Os, on_path: impl Fn(&str) -> bool) -> Option<Manager> {
    let candidates: &[Manager] = match os {
        Os::Windows => &[Manager::Winget],
        Os::Linux => &[Manager::Apt, Manager::Dnf, Manager::Pacman, Manager::Zypper],
    };
    candidates.iter().copied().find(|m| on_path(m.program()))
}
