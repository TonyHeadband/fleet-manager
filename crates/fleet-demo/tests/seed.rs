use std::path::Path;
use std::process::Command;

use fleet_demo::seed::seed_repo;
use fleet_demo::timeline::RepoSpec;

fn spec(origin: bool) -> RepoSpec {
    RepoSpec {
        name: "bakery".into(),
        message: "Start the bakery".into(),
        author: ("Baker".into(), "baker@pie.example".into()),
        origin,
        roster: vec![],
    }
}

fn shot_with_seed(dir: &Path) {
    let seed = dir.join("seed/bakery");
    std::fs::create_dir_all(seed.join(".fleet")).unwrap();
    std::fs::write(seed.join("README.txt"), "Recipes for the bakery.\n").unwrap();
    std::fs::write(seed.join(".fleet/workflow.yml"), "gate: {}\n").unwrap();
}

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn seed_commits_the_folder_with_its_author() {
    let shot = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    shot_with_seed(shot.path());

    let repo = seed_repo(shot.path(), &spec(false), work.path()).unwrap();

    assert_eq!(repo, work.path().join("bakery"));
    assert_eq!(
        git(&repo, &["log", "--format=%an%x09%ae%x09%s"]),
        "Baker\tbaker@pie.example\tStart the bakery"
    );
    let tracked = git(&repo, &["ls-files"]);
    assert_eq!(tracked, ".fleet/workflow.yml\nREADME.txt");
}

#[test]
fn seed_with_origin_pushes_main_to_a_bare_repo() {
    let shot = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    shot_with_seed(shot.path());

    let repo = seed_repo(shot.path(), &spec(true), work.path()).unwrap();

    assert!(work.path().join("bakery.origin.git").is_dir());
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let remote = git(&repo, &["ls-remote", "origin", "main"]);
    assert_eq!(remote, format!("{head}\trefs/heads/main"));
    assert_eq!(
        git(&repo, &["rev-parse", "--abbrev-ref", "origin/HEAD"]),
        "origin/main"
    );
}

#[test]
fn a_missing_seed_folder_is_named() {
    let shot = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();

    let e = seed_repo(shot.path(), &spec(false), work.path()).unwrap_err();

    assert!(e.contains("seed/bakery"), "{e}");
}
