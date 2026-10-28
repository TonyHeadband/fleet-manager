//! The POSIX shell that scripts (login scripts, gate commands) run under: `sh` on Unix; on Windows the `bash.exe`
//! of Git for Windows, which Claude Code needs there anyway. Never `System32\bash.exe`: that one is WSL's.
use std::path::{Path, PathBuf};

/// The MSYS2 environment folders Git for Windows ships its own `git.exe` in, `<root>\<env>\bin\git.exe`.
const GIT_ENVS: [&str; 5] = ["mingw64", "mingw32", "clangarm64", "clang64", "ucrt64"];

/// Git for Windows' `<root>\bin\bash.exe`, for any `git.exe` of it: `<root>\cmd` (what its installer puts on PATH),
/// `<root>\bin`, or `<root>\mingw64\bin` (what a shell started from Git Bash finds first).
pub fn git_bash_beside(git: &Path) -> Option<PathBuf> {
    let dir = git.parent()?;
    let up = dir.parent()?;
    let in_env = dir
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("bin"))
        && up
            .file_name()
            .is_some_and(|n| GIT_ENVS.iter().any(|e| n.eq_ignore_ascii_case(e)));
    let root = if in_env { up.parent()? } else { up };
    let bash = root.join("bin").join("bash.exe");
    bash.is_file().then_some(bash)
}

/// `CLAUDE_CODE_GIT_BASH_PATH` (Claude Code's own setting) when it names a file, else the bash beside `git`, else
/// the bash of the first Git for Windows install root that has one.
pub fn pick(
    env_override: Option<PathBuf>,
    git_on_path: Option<PathBuf>,
    install_roots: &[PathBuf],
) -> Option<PathBuf> {
    if let Some(p) = env_override.filter(|p| p.is_file()) {
        return Some(p);
    }
    git_on_path
        .and_then(|git| git_bash_beside(&git))
        .or_else(|| {
            install_roots
                .iter()
                .map(|root| root.join("bin").join("bash.exe"))
                .find(|bash| bash.is_file())
        })
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

/// Where the Git for Windows installer puts it: machine-wide under Program Files, or per user.
pub fn install_roots() -> Vec<PathBuf> {
    let machine = ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(|var| std::env::var_os(var).map(|d| PathBuf::from(d).join("Git")));
    let user =
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Programs").join("Git"));
    machine.chain(user).collect()
}

/// The shell to hand a script to with `-c`, or `None` on Windows without Git for Windows.
pub fn posix_shell() -> Option<PathBuf> {
    if cfg!(windows) {
        pick(
            std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH").map(PathBuf::from),
            which::which("git").ok(),
            &install_roots(),
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

    /// Every place Git for Windows puts a `git.exe`, relative to its root.
    const GIT_LAYOUTS: [&str; 5] = [
        "cmd/git.exe",
        "bin/git.exe",
        "mingw64/bin/git.exe",
        "mingw32/bin/git.exe",
        "clangarm64/bin/git.exe",
    ];

    #[test]
    fn git_bash_is_found_from_every_git_for_windows_layout() {
        for layout in GIT_LAYOUTS {
            let t = tempfile::tempdir().unwrap();
            let root = t.path().join("Git");
            git_tree(&root);
            let git = root.join(layout);
            std::fs::create_dir_all(git.parent().unwrap()).unwrap();
            std::fs::write(&git, b"").unwrap();
            assert_eq!(
                git_bash_beside(&git),
                Some(root.join("bin").join("bash.exe")),
                "{layout}"
            );
        }
    }

    #[test]
    fn a_git_on_path_without_bash_falls_back_to_the_install_folders() {
        let t = tempfile::tempdir().unwrap();
        // A git that is not Git for Windows' comes first on PATH; Git for Windows is installed per user.
        let other = t.path().join("tools/git.exe");
        std::fs::create_dir_all(other.parent().unwrap()).unwrap();
        std::fs::write(&other, b"").unwrap();
        let user = t.path().join("AppData/Local/Programs/Git");
        git_tree(&user);
        let missing = t.path().join("Program Files/Git");
        let roots = [missing, user.clone()];
        assert_eq!(
            pick(None, Some(other), &roots),
            Some(user.join("bin").join("bash.exe"))
        );
        assert_eq!(
            pick(None, None, &roots),
            Some(user.join("bin").join("bash.exe")),
            "a GUI started without git on PATH"
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
        assert_eq!(pick(Some(mine.clone()), Some(git), &[]), Some(mine));
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
        assert_eq!(pick(None, Some(sys.join("git.exe")), &[]), None);
        assert_eq!(pick(Some(t.path().join("missing.exe")), None, &[]), None);
        assert_eq!(pick(None, None, &[t.path().join("Windows")]), None);
    }
}
