//! A persona's forge token, kept in `<personas_dir>/<id>/forge/token`. Nothing under `forge/` is handed to an agent.
use crate::personas::check_command;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

const GH_TOKEN_TIMEOUT: Duration = Duration::from_secs(10);

pub fn token_path(personas_dir: &Path, persona: &str) -> PathBuf {
    personas_dir.join(persona).join("forge").join("token")
}

/// The stored token; none when the file is missing, unreadable or blank.
pub fn read_token(path: &Path) -> Option<String> {
    let token = std::fs::read_to_string(path).ok()?;
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_string())
}

/// Replaces the file through a sibling, so a reader never sees half of it. The folder is 0700 and the file 0600 on
/// Unix; the sibling is made private before the token goes into it.
pub fn write_token(path: &Path, token: &str) -> io::Result<()> {
    let dir = path.parent().expect("a token path has a folder");
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir)?;
    let tmp = dir.join("token.tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    {
        use std::io::Write;
        let mut f = opts.open(&tmp)?;
        f.write_all(token.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// What `gh auth token --hostname <host>` prints with `gh_dir` as its config dir and the ambient tokens stripped; none when gh is missing, fails or
/// prints nothing. `path` is the PATH gh is looked for on.
pub async fn token_from_gh(gh: &str, gh_dir: &Path, host: &str, path: &OsStr) -> Option<String> {
    let mut cmd = check_command(
        gh,
        &["auth", "token", "--hostname", host],
        "GH_CONFIG_DIR",
        gh_dir,
        path,
    )?;
    let out = tokio::time::timeout(GH_TOKEN_TIMEOUT, cmd.output())
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!token.is_empty()).then_some(token)
}
