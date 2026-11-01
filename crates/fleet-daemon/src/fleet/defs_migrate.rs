//! Definitions across personas: the names a product may refer to, and the one-time move of fleet-wide files into
//! the personas that now own them.
use super::defs::{persona_defs_root, Defs};
use fleet_api::{DefInfo, DefKind};
use std::collections::BTreeSet;
use std::io;
use std::path::Path;

/// One row per `kind` name known to any persona or built in. The built-in's row comes first, then the first persona
/// (in `persona_ids` order) that has the name on its own.
pub fn known_defs(
    personas_dir: &Path,
    persona_ids: &[String],
    fleet_root: &Path,
    kind: DefKind,
) -> Vec<DefInfo> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let roots = std::iter::once(Defs::shared(fleet_root.to_path_buf())).chain(
        persona_ids.iter().map(|id| {
            Defs::for_persona(
                persona_defs_root(personas_dir, id),
                fleet_root.to_path_buf(),
            )
        }),
    );
    for defs in roots {
        for d in defs.list().into_iter().filter(|d| d.kind == kind) {
            if seen.insert(d.name.clone()) {
                out.push(d);
            }
        }
    }
    out
}

/// Specialist and orchestrator names known to any persona or built in: what product-level checks accept.
pub fn known_names(
    personas_dir: &Path,
    persona_ids: &[String],
    fleet_root: &Path,
    kind: DefKind,
) -> BTreeSet<String> {
    known_defs(personas_dir, persona_ids, fleet_root, kind)
        .into_iter()
        .map(|d| d.name)
        .collect()
}

/// The folders under the fleet root that belong to a persona now; PR styles stay fleet-wide.
const MOVED: [&str; 3] = ["orchestrators", "specialists", "skills"];

/// Moves leftover fleet-wide orchestrators, specialists and skills into each persona once; none when `persona_ids`
/// is empty. A persona's own file of the same name wins. Nothing leaves the fleet folder until every persona has its
/// copy, so a failure part-way is retried whole at the next start. Returns how many leftovers were moved.
pub fn migrate_fleet_defs(
    fleet_root: &Path,
    personas_dir: &Path,
    persona_ids: &[String],
) -> io::Result<usize> {
    if persona_ids.is_empty() {
        return Ok(0);
    }
    let mut moved = Vec::new();
    for folder in MOVED {
        let dir = fleet_root.join(folder);
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        for entry in entries {
            let entry = entry?;
            for id in persona_ids {
                let to = persona_defs_root(personas_dir, id)
                    .join(folder)
                    .join(entry.file_name());
                if !to.exists() {
                    copy_tree(&entry.path(), &to)?;
                }
            }
            moved.push(entry.path());
        }
    }
    for path in &moved {
        if path.is_dir() {
            std::fs::remove_dir_all(path)?;
        } else {
            std::fs::remove_file(path)?;
        }
    }
    for folder in MOVED {
        // Only when empty: anything unexpected in it is left alone.
        let _ = std::fs::remove_dir(fleet_root.join(folder));
    }
    Ok(moved.len())
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(from, to).map(drop)
    }
}
