//! Puts a definition's skills into an agent's own folder. Agents run under a persona's sandboxed Claude home, so the
//! skills listed in their definition are the only ones they have.
use super::defs::{Builtin, Defs, FleetError};
use std::path::{Path, PathBuf};

/// Copies each named skill to `<agent_dir>/.claude/skills/<name>/`, replacing what is there. Every name is checked
/// before anything is copied, so a bad name leaves the folder as it was.
pub fn install(defs: &Defs, names: &[String], agent_dir: &Path) -> Result<(), FleetError> {
    let sources = names
        .iter()
        .map(|n| defs.skill_dir(n))
        .collect::<Result<Vec<_>, _>>()?;
    for src in sources {
        let dest = agent_dir.join(".claude/skills").join(&src.name);
        match std::fs::remove_dir_all(&dest) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(&dest, e)),
        }
        std::fs::create_dir_all(&dest).map_err(|e| io(&dest, e))?;
        if src.builtin {
            let prefix = format!("skills/{}/", src.name);
            for path in Builtin::iter().filter(|p| p.starts_with(&prefix)) {
                let file = Builtin::get(&path).expect("listed by iter");
                let to = dest.join(&path[prefix.len()..]);
                write(&to, &file.data)?;
                // The embedded tree keeps no file modes; the shipped scripts are all shell.
                if path.ends_with(".sh") {
                    make_executable(&to)?;
                }
            }
        }
        if let Some(user) = &src.user {
            copy_tree(user, &dest)?;
        }
    }
    Ok(())
}

fn io(path: &Path, source: std::io::Error) -> FleetError {
    FleetError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn write(to: &Path, data: &[u8]) -> Result<(), FleetError> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    }
    std::fs::write(to, data).map_err(|e| io(to, e))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), FleetError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|e| io(path, e))
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<(), FleetError> {
    Ok(())
}

/// `fs::copy` keeps the user's modes, so their own scripts stay runnable. Symlinks are skipped: a link out of the
/// skill folder would hand the agent a file the user never put in it.
fn copy_tree(from: &Path, to: &Path) -> Result<(), FleetError> {
    let mut stack: Vec<(PathBuf, PathBuf)> = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((src, dst)) = stack.pop() {
        std::fs::create_dir_all(&dst).map_err(|e| io(&dst, e))?;
        for entry in std::fs::read_dir(&src).map_err(|e| io(&src, e))? {
            let entry = entry.map_err(|e| io(&src, e))?;
            let kind = entry.file_type().map_err(|e| io(&entry.path(), e))?;
            let target = dst.join(entry.file_name());
            if kind.is_dir() {
                stack.push((entry.path(), target));
            } else if kind.is_file() {
                std::fs::copy(entry.path(), &target).map_err(|e| io(&target, e))?;
            }
        }
    }
    Ok(())
}
