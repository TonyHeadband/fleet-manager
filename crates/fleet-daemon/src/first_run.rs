//! Pre-answers Claude Code's first-run prompts (onboarding, the fullscreen-renderer upsell) in a persona's config
//! dir, so a fresh session never stops on a menu that fires no hook and leaves its card Working.

use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::warn;

/// Makes every temp name unique within the process, so two seeds of one dir never share a file.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

const FULLSCREEN_UPSELL_SEEN: i64 = 3;

/// Adds what is missing to `<claude_dir>/.claude.json`. Claude Code rewrites that file itself, so an existing value
/// stays (the upsell count only goes up), the write is atomic, and a file that does not parse is left alone.
pub fn seed_first_run(claude_dir: &Path) -> std::io::Result<()> {
    let path = claude_dir.join(".claude.json");
    let mut doc = match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(v @ Value::Object(_)) => v,
            _ => {
                warn!(path = %path.display(), "not a JSON object, first-run prompts not pre-answered");
                return Ok(());
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e),
    };
    let obj = doc.as_object_mut().expect("checked above");
    let before = obj.clone();
    for key in ["hasCompletedOnboarding", "hasSeenAutoDefaultNotice"] {
        obj.entry(key).or_insert(Value::Bool(true));
    }
    let seen = obj
        .get("fullscreenUpsellSeenCount")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if seen < FULLSCREEN_UPSELL_SEEN {
        obj.insert(
            "fullscreenUpsellSeenCount".into(),
            json!(FULLSCREEN_UPSELL_SEEN),
        );
    }
    if *obj == before {
        return Ok(());
    }

    std::fs::create_dir_all(claude_dir)?;
    // Same directory, so the rename never crosses a filesystem. Created 0600 like Claude Code's own file unless the
    // existing one says otherwise.
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mode = std::fs::metadata(&path)
        .map(|m| m.permissions().mode() & 0o7777)
        .unwrap_or(0o600);
    // Accepted: a live Claude under the same persona can interleave with this read-modify-write; only the first seed
    // changes the file, and Claude re-adds its own state.
    let tmp = claude_dir.join(format!(
        ".claude.json.fleet-{}-{}.tmp",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&tmp)?;
        f.write_all(serde_json::to_string_pretty(&doc)?.as_bytes())?;
        f.sync_all()?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
        std::fs::rename(&tmp, &path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}
