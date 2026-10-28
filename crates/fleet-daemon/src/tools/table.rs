//! The tool table: what is required, how to ask each tool its version, how to install it.

use super::pkg::Manager;
use fleet_api::{Os, ToolFix, ToolId};

pub struct ToolSpec {
    pub id: ToolId,
    pub required: bool,
    pub program: &'static str,
    pub version_args: &'static [&'static str],
    pub min_version: Option<&'static str>,
    pub link: &'static str,
}

const GIT_LINK: &str = "https://git-scm.com/downloads";
const CLAUDE_LINK: &str = "https://docs.claude.com/en/docs/claude-code/setup";
const GH_LINK: &str = "https://cli.github.com/";

const GIT: ToolSpec = ToolSpec {
    id: ToolId::Git,
    required: true,
    program: "git",
    version_args: &["--version"],
    min_version: None,
    link: GIT_LINK,
};
const GIT_BASH: ToolSpec = ToolSpec {
    id: ToolId::GitBash,
    required: true,
    program: "bash",
    version_args: &["--version"],
    min_version: None,
    link: GIT_LINK,
};
const CLAUDE: ToolSpec = ToolSpec {
    id: ToolId::Claude,
    required: true,
    program: "claude",
    version_args: &["--version"],
    min_version: None,
    link: CLAUDE_LINK,
};
const GH: ToolSpec = ToolSpec {
    id: ToolId::Gh,
    required: true,
    program: "gh",
    version_args: &["--version"],
    min_version: Some("2.40"),
    link: GH_LINK,
};

static WINDOWS: [ToolSpec; 4] = [GIT, GIT_BASH, CLAUDE, GH];
static LINUX: [ToolSpec; 3] = [GIT, CLAUDE, GH];

pub fn table(os: Os) -> &'static [ToolSpec] {
    match os {
        Os::Windows => &WINDOWS,
        Os::Linux => &LINUX,
    }
}

/// The exact command to install `id` with `manager`; none when there is no such command.
pub fn install_command(id: ToolId, os: Os, manager: Option<Manager>) -> Option<String> {
    use Manager::*;
    let cmd = match (id, os) {
        // Claude Code: Anthropic's official installers, whatever the package manager.
        // https://code.claude.com/docs/en/setup (docs.claude.com/en/docs/claude-code/setup redirects here)
        (ToolId::Claude, Os::Windows) => {
            r#"powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://claude.ai/install.ps1 | iex""#
        }
        (ToolId::Claude, Os::Linux) => "curl -fsSL https://claude.ai/install.sh | bash",
        // Git for Windows brings Git Bash with it.
        // https://git-scm.com/downloads and https://git-scm.com/install/linux
        (ToolId::Git | ToolId::GitBash, _) => match manager? {
            Winget => "winget install --id Git.Git -e --source winget",
            Apt => "sudo apt-get install -y git",
            Dnf => "sudo dnf install -y git",
            Pacman => "sudo pacman -S --noconfirm git",
            Zypper => "sudo zypper install -y git",
        },
        // https://github.com/cli/cli/blob/trunk/docs/install_linux.md
        (ToolId::Gh, _) => match manager? {
            Winget => "winget install --id GitHub.cli -e --source winget",
            Apt => concat!(
                "sudo mkdir -p -m 755 /etc/apt/keyrings",
                " && sudo curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg",
                " -o /etc/apt/keyrings/githubcli-archive-keyring.gpg",
                " && sudo chmod go+r /etc/apt/keyrings/githubcli-archive-keyring.gpg",
                " && echo \"deb [arch=$(dpkg --print-architecture)",
                " signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg]",
                " https://cli.github.com/packages stable main\"",
                " | sudo tee /etc/apt/sources.list.d/github-cli.list > /dev/null",
                " && sudo apt-get update && sudo apt-get install -y gh",
            ),
            // The repo file is fetched directly: `dnf config-manager` differs between dnf4 and dnf5.
            Dnf => concat!(
                "sudo curl -fsSL https://cli.github.com/packages/rpm/gh-cli.repo",
                " -o /etc/yum.repos.d/gh-cli.repo && sudo dnf install -y gh",
            ),
            Pacman => "sudo pacman -S --noconfirm github-cli",
            Zypper => concat!(
                "sudo zypper addrepo https://cli.github.com/packages/rpm/gh-cli.repo",
                " && sudo zypper --gpg-auto-import-keys refresh",
                " && sudo zypper install -y gh",
            ),
        },
    };
    Some(cmd.to_string())
}

/// How to fix `id` here: the install command when there is one, always the download link.
pub fn fix(id: ToolId, os: Os, manager: Option<Manager>) -> ToolFix {
    let install = install_command(id, os, manager);
    let link = match id {
        ToolId::Git | ToolId::GitBash => GIT_LINK,
        ToolId::Claude => CLAUDE_LINK,
        ToolId::Gh => GH_LINK,
    };
    // Claude's installer needs no package manager, so none is named for it.
    let manager = match id {
        ToolId::Claude => None,
        _ => manager
            .filter(|_| install.is_some())
            .map(|m| m.label().to_string()),
    };
    ToolFix {
        install,
        link: link.to_string(),
        manager,
    }
}

/// The dotted version number in a tool's `--version` output.
pub fn parse_version(id: ToolId, stdout: &str) -> Option<String> {
    let line = stdout.lines().next()?.trim();
    let token = match id {
        // `git version 2.43.0` / `git version 2.43.0.windows.1`
        ToolId::Git | ToolId::GitBash => line
            .strip_prefix("git version ")?
            .split_whitespace()
            .next()?,
        // `2.0.14 (Claude Code)`
        ToolId::Claude => line.split_whitespace().next()?,
        // `gh version 2.40.1 (2023-12-13)`
        ToolId::Gh => line
            .strip_prefix("gh version ")?
            .split_whitespace()
            .next()?,
    };
    // Keep only the leading numeric dotted parts (drops `.windows.1`).
    let numeric: Vec<&str> = token
        .split('.')
        .take_while(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        .collect();
    (!numeric.is_empty()).then(|| numeric.join("."))
}

/// Numeric dotted comparison; missing parts count as 0.
pub fn at_least(found: &str, needed: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    let (f, n) = (parts(found), parts(needed));
    for i in 0..f.len().max(n.len()) {
        let (a, b) = (
            f.get(i).copied().unwrap_or(0),
            n.get(i).copied().unwrap_or(0),
        );
        if a != b {
            return a > b;
        }
    }
    true
}
