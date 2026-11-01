//! What a supervisor conversation runs in: its persona's folder, its settings and its CLAUDE.md.
use crate::agents::AgentService;
use crate::config::Config;
use crate::hooks_cfg::RoleSettings;
use crate::store::StoreError;
use std::path::{Path, PathBuf};

const CLAUDE_MD: &str = include_str!("../../builtin/supervisor/CLAUDE.md");

/// The persona's supervisor folder, every conversation's cwd: one folder, so `--resume` finds each session again.
pub fn folder(cfg: &Config, persona: &str) -> PathBuf {
    cfg.state_dir.join("supervisor").join(persona)
}

/// Reads every product checkout of the persona and the internal repos; edits nowhere a task or a repo lives.
pub(crate) async fn role_settings(
    svc: &AgentService,
    persona: &str,
) -> Result<RoleSettings, StoreError> {
    let mut read_dirs = Vec::new();
    for product in svc.store.products().await? {
        for repo in svc.store.repos(&product.id).await? {
            let dir = svc.repos.clone_dir(persona, &product.id, &repo.name);
            read_dirs.push(dir.to_string_lossy().into_owned());
        }
    }
    let internal = svc.internal().root().to_path_buf();
    read_dirs.push(internal.to_string_lossy().into_owned());
    let cfg = svc.config();
    Ok(RoleSettings::supervisor(
        read_dirs,
        &[
            cfg.state_dir.join("repos"),
            cfg.tasks_root.clone(),
            internal,
        ],
    ))
}

/// Written on every start, so a conversation always runs on this build's instructions.
pub(crate) fn write_claude_md(dir: &Path) -> std::io::Result<()> {
    std::fs::write(dir.join("CLAUDE.md"), CLAUDE_MD)
}
