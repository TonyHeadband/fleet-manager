use fleet_fs::migrate::migrate_legacy_dirs;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main"])
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn moves_the_old_config_and_state_folders() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    write(&h.join(".config/fleet-board/config.toml"), "projects_root = \"~/p\"\n");
    write(&h.join(".local/share/fleet-board/fleet.db"), "db");

    let notes = migrate_legacy_dirs(h);

    assert_eq!(std::fs::read_to_string(h.join(".config/fleet-manager/config.toml")).unwrap(), "projects_root = \"~/p\"\n");
    assert_eq!(std::fs::read_to_string(h.join(".local/share/fleet-manager/fleet.db")).unwrap(), "db");
    assert!(!h.join(".config/fleet-board").exists());
    assert!(!h.join(".local/share/fleet-board").exists());
    assert_eq!(notes.len(), 2, "{notes:?}");
}

#[test]
fn does_nothing_without_old_folders_and_twice_is_a_no_op() {
    let home = tempfile::tempdir().unwrap();
    assert!(migrate_legacy_dirs(home.path()).is_empty());
    write(&home.path().join(".config/fleet-board/gui.toml"), "");
    migrate_legacy_dirs(home.path());
    assert!(migrate_legacy_dirs(home.path()).is_empty());
}

#[test]
fn never_overwrites_an_existing_new_folder() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    write(&h.join(".config/fleet-board/config.toml"), "old");
    write(&h.join(".config/fleet-manager/config.toml"), "new");

    migrate_legacy_dirs(h);

    assert_eq!(std::fs::read_to_string(h.join(".config/fleet-manager/config.toml")).unwrap(), "new");
    assert_eq!(std::fs::read_to_string(h.join(".config/fleet-board/config.toml")).unwrap(), "old");
}

#[test]
fn leaves_the_state_folder_when_the_config_names_it() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    write(&h.join(".config/fleet-board/config.toml"), "state_dir = \"~/.local/share/fleet-board\"\n");
    write(&h.join(".local/share/fleet-board/fleet.db"), "db");

    let notes = migrate_legacy_dirs(h);

    assert!(h.join(".config/fleet-manager/config.toml").is_file());
    assert!(h.join(".local/share/fleet-board/fleet.db").is_file());
    assert!(!h.join(".local/share/fleet-manager").exists());
    assert!(notes.iter().any(|n| n.contains("state_dir")), "{notes:?}");
}

#[test]
fn a_commented_state_dir_does_not_count() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    write(&h.join(".config/fleet-board/config.toml"), "# state_dir = \"~/.local/share/fleet-board\"\n");
    write(&h.join(".local/share/fleet-board/fleet.db"), "db");

    migrate_legacy_dirs(h);

    assert!(h.join(".local/share/fleet-manager/fleet.db").is_file());
}

#[test]
fn a_pidfile_of_another_process_does_not_block_the_move() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    // Our own pid is alive but is no fleet-daemon: a stale pidfile whose pid was reused.
    write(&h.join(".local/share/fleet-board/daemon.pid"), &format!("{}\n", std::process::id()));
    migrate_legacy_dirs(h);
    assert!(h.join(".local/share/fleet-manager/daemon.pid").is_file());
}

#[test]
fn repairs_worktrees_of_cloned_repos() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    let clone = h.join(".local/share/fleet-board/repos/tools");
    std::fs::create_dir_all(&clone).unwrap();
    git(&clone, &["init", "-q"]);
    git(&clone, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let wt = h.join("fleet-projects/x/tools");
    git(&clone, &["worktree", "add", "-q", "-b", "fleet/x", wt.to_str().unwrap()]);

    migrate_legacy_dirs(h);

    assert_eq!(git(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]), "fleet/x");
    let common = git(&wt, &["rev-parse", "--path-format=absolute", "--git-common-dir"]);
    assert!(common.contains("fleet-manager/repos/tools"), "{common}");
}
