use std::path::{Path, PathBuf};

use fleet_demo::scenario::{load_dir, StepKind};

fn demo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../demo")
}

fn shots() -> Vec<PathBuf> {
    let mut shots: Vec<PathBuf> = std::fs::read_dir(demo_dir().join("shots"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    shots.sort();
    assert!(!shots.is_empty());
    shots
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_under(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn every_demo_scenario_parses() {
    for shot in shots() {
        let all = load_dir(&shot.join("scenarios")).unwrap();
        assert!(!all.is_empty(), "{} has no scenarios", shot.display());
    }
}

#[test]
fn every_write_source_exists() {
    for shot in shots() {
        for (path, scenario) in &load_dir(&shot.join("scenarios")).unwrap() {
            let steps = scenario
                .main
                .iter()
                .chain(scenario.follow_ups.iter().flat_map(|f| &f.steps));
            for step in steps {
                if let StepKind::Write { from, .. } = &step.kind {
                    // The agent resolves `from` against the shot folder, not the scenarios folder.
                    let source = shot.join(from);
                    assert!(source.is_file(), "{} names {from}", path.display());
                }
            }
        }
    }
}

#[test]
fn no_demo_source_matches_the_gate_ignore() {
    let mut files = Vec::new();
    files_under(&demo_dir(), &mut files);
    assert!(!files.is_empty());
    for f in files {
        assert!(
            f.extension().is_none_or(|e| e != "md"),
            "{} would be ignored by the gate",
            f.display()
        );
    }
}
