//! The POSIX shell that scripts (login scripts, gate commands) run under: `sh` on Unix; on Windows the `bash.exe`
//! of Git for Windows, which Claude Code needs there anyway. Never `System32\bash.exe`: that one is WSL's.
use std::path::{Path, PathBuf};

/// Git for Windows lays out `<root>\cmd\git.exe` (what goes on PATH) and `<root>\bin\bash.exe`.
pub fn git_bash_beside(git: &Path) -> Option<PathBuf> {
    let bash = git.parent()?.parent()?.join("bin").join("bash.exe");
    bash.is_file().then_some(bash)
}

/// `CLAUDE_CODE_GIT_BASH_PATH` (Claude Code's own setting) when it names a file, else the bash beside `git`.
pub fn pick(env_override: Option<PathBuf>, git_on_path: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(p) = env_override.filter(|p| p.is_file()) {
        return Some(p);
    }
    git_bash_beside(&git_on_path?)
}

/// Git for Windows' terminal, `<root>\usr\bin\mintty.exe`, for its `<root>\bin\bash.exe`.
pub fn mintty_beside_bash(bash: &Path) -> Option<PathBuf> {
    let mintty = bash
        .parent()?
        .parent()?
        .join("usr")
        .join("bin")
        .join("mintty.exe");
    mintty.is_file().then_some(mintty)
}

/// The shell to hand a script to with `-c`, or `None` on Windows without Git for Windows.
pub fn posix_shell() -> Option<PathBuf> {
    if cfg!(windows) {
        pick(
            std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH").map(PathBuf::from),
            which::which("git").ok(),
        )
    } else {
        Some(PathBuf::from("sh"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_tree(root: &Path) -> PathBuf {
        std::fs::create_dir_all(root.join("cmd")).unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(root.join("cmd").join("git.exe"), b"").unwrap();
        std::fs::write(root.join("bin").join("bash.exe"), b"").unwrap();
        root.join("cmd").join("git.exe")
    }

    #[test]
    fn git_bash_is_found_beside_the_git_on_path() {
        // A user-scope install, not under Program Files.
        let t = tempfile::tempdir().unwrap();
        let git = git_tree(&t.path().join("AppData/Local/Programs/Git"));
        assert_eq!(
            git_bash_beside(&git),
            Some(t.path().join("AppData/Local/Programs/Git/bin/bash.exe"))
        );
    }

    #[test]
    fn a_git_without_bash_beside_it_gives_none() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("cmd")).unwrap();
        assert_eq!(git_bash_beside(&t.path().join("cmd/git.exe")), None);
    }

    #[test]
    fn the_override_wins_when_it_exists() {
        let t = tempfile::tempdir().unwrap();
        let mine = t.path().join("my-bash.exe");
        std::fs::write(&mine, b"").unwrap();
        let git = git_tree(&t.path().join("Git"));
        assert_eq!(pick(Some(mine.clone()), Some(git)), Some(mine));
    }

    #[test]
    fn mintty_is_found_from_git_bash() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Git");
        std::fs::create_dir_all(root.join("usr/bin")).unwrap();
        std::fs::write(root.join("usr/bin/mintty.exe"), b"").unwrap();
        assert_eq!(
            mintty_beside_bash(&root.join("bin/bash.exe")),
            Some(root.join("usr/bin/mintty.exe"))
        );
        assert_eq!(
            mintty_beside_bash(&t.path().join("elsewhere/bash.exe")),
            None
        );
    }

    #[test]
    fn posix_shell_never_picks_wsl_bash() {
        // Only WSL's bash exists: no override, and the git found is not Git for Windows (no bin\bash.exe beside it).
        let t = tempfile::tempdir().unwrap();
        let sys = t.path().join("Windows/System32");
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(sys.join("bash.exe"), b"").unwrap();
        std::fs::write(sys.join("git.exe"), b"").unwrap();
        assert_eq!(pick(None, Some(sys.join("git.exe"))), None);
        assert_eq!(pick(Some(t.path().join("missing.exe")), None), None);
    }
}
