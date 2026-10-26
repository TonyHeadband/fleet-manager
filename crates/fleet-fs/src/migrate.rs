//! One-time move of the folders the app used under its old name, fleet-board. Both the daemon and the app call it
//! at startup, before reading the config, so whichever starts first does the move.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const CONFIG: (&str, &str) = (".config/fleet-board", ".config/fleet-manager");
const STATE: (&str, &str) = (".local/share/fleet-board", ".local/share/fleet-manager");

/// Moves each old folder under `home` to its new name when the old one exists and the new one does not. Returns
/// one line per move or refusal, for the caller's log. Never overwrites: when both exist the new one wins.
pub fn migrate_legacy_dirs(home: &Path) -> Vec<String> {
    let mut notes = Vec::new();
    let (old, new) = (home.join(CONFIG.0), home.join(CONFIG.1));
    if old.is_dir() && !new.exists() {
        notes.push(rename(&old, &new));
    }

    let (old, new) = (home.join(STATE.0), home.join(STATE.1));
    if !old.is_dir() || new.exists() {
        return notes;
    }
    // A config that names the old folder explicitly would point at nothing after the move.
    if config_names_old_state(&home.join(CONFIG.1).join("config.toml")) {
        notes.push(format!(
            "left {} in place: config.toml sets state_dir to it",
            old.display()
        ));
        return notes;
    }
    // Moving the folder under a running daemon would leave it writing to a path that no longer exists.
    if let Some(pid) = running_daemon(&old) {
        notes.push(format!(
            "left {} in place: fleet-daemon (pid {pid}) still runs from it; stop it and restart",
            old.display()
        ));
        return notes;
    }
    notes.push(rename(&old, &new));
    if new.is_dir() {
        notes.extend(repair_clones(&new.join("repos")));
    }
    notes
}

fn rename(old: &Path, new: &Path) -> String {
    let moved = new
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::rename(old, new));
    match moved {
        Ok(()) => format!("moved {} to {}", old.display(), new.display()),
        Err(e) => format!("could not move {} to {}: {e}", old.display(), new.display()),
    }
}

fn config_names_old_state(config: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(config) else {
        return false;
    };
    text.lines()
        .map(str::trim)
        .any(|l| l.starts_with("state_dir") && l.contains(STATE.0))
}

fn running_daemon(state: &Path) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(state.join("daemon.pid"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let out = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .starts_with("fleet-daemon")
        .then_some(pid)
}

/// Repos added by URL are cloned under the state folder, and their worktrees in task folders record the clone's
/// absolute path; `git worktree repair` run from the moved clone rewrites those links.
fn repair_clones(repos: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(repos) else {
        return Vec::new();
    };
    let clones: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    clones
        .into_iter()
        .filter_map(|clone| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&clone)
                .args(["worktree", "repair"])
                .stdin(Stdio::null())
                .output();
            match out {
                Ok(o) if o.status.success() => None,
                Ok(o) => Some(format!(
                    "git worktree repair in {}: {}",
                    clone.display(),
                    String::from_utf8_lossy(&o.stderr).trim()
                )),
                Err(e) => Some(format!("git worktree repair in {}: {e}", clone.display())),
            }
        })
        .collect()
}
