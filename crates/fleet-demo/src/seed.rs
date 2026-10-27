//! Builds a shot's starting repos from `<shot>/seed/<name>/`, so a shot carries its own files instead of the
//! recorder creating them inline.
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::timeline::RepoSpec;

/// Copies `<shot>/seed/<name>/` (dotfiles too) to `<work>/<name>` as a repo with one commit. With `origin`, a
/// bare `<work>/<name>.origin.git` becomes its `origin` with `main` pushed and `origin/HEAD` set, as after a clone.
pub fn seed_repo(shot: &Path, repo: &RepoSpec, work: &Path) -> Result<PathBuf, String> {
    let seed = shot.join("seed").join(&repo.name);
    if !seed.is_dir() {
        return Err(format!(
            "repo {}: no seed folder {}",
            repo.name,
            seed.display()
        ));
    }
    let dest = work.join(&repo.name);
    copy_tree(&seed, &dest)?;
    git(&dest, &["init", "-q", "-b", "main"])?;
    git(&dest, &["add", "-A"])?;
    let (name, email) = &repo.author;
    let (name, email) = (format!("user.name={name}"), format!("user.email={email}"));
    // Signing is off so a developer's global config cannot make the seed ask for a key.
    git(
        &dest,
        &[
            "-c",
            &name,
            "-c",
            &email,
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            &repo.message,
        ],
    )?;
    if repo.origin {
        let bare = work.join(format!("{}.origin.git", repo.name));
        let bare_arg = bare.to_string_lossy();
        git(work, &["init", "-q", "--bare", "-b", "main", &bare_arg])?;
        git(&dest, &["remote", "add", "origin", &bare_arg])?;
        git(&dest, &["push", "-q", "-u", "origin", "main"])?;
        git(&dest, &["remote", "set-head", "origin", "main"])?;
    }
    Ok(dest)
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    let io =
        |what: &str, p: &Path, e: std::io::Error| format!("cannot {what} {}: {e}", p.display());
    std::fs::create_dir_all(to).map_err(|e| io("create", to, e))?;
    for entry in std::fs::read_dir(from).map_err(|e| io("read", from, e))? {
        let entry = entry.map_err(|e| io("read", from, e))?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if src.is_dir() {
            copy_tree(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| io("copy", &src, e))?;
        }
    }
    Ok(())
}

fn git(dir: &Path, args: &[&str]) -> Result<(), String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}
