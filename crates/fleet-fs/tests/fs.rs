use fleet_fs::{
    checkouts, dirty_count, head, list, main_root, toplevel, Entry, EntryKind, FsError, MainRoot,
};
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .current_dir(dir)
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

fn init(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q"]);
}

fn repo_with_commit(dir: &Path) {
    init(dir);
    std::fs::write(dir.join("f.txt"), "one\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "first"]);
}

fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap()
}

fn tmp() -> (tempfile::TempDir, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let root = canon(t.path());
    (t, root)
}

#[test]
fn list_marks_checkouts_and_skips_hidden() {
    let (_t, root) = tmp();
    std::fs::create_dir(root.join("a")).unwrap();
    std::fs::create_dir(root.join(".hidden")).unwrap();
    std::fs::write(root.join("file.txt"), "x").unwrap();
    repo_with_commit(&root.join("repo"));
    let l = list(&root, &[]).unwrap();
    assert_eq!(l.dir, root);
    assert_eq!(
        l.entries,
        vec![
            Entry {
                name: "a".into(),
                path: root.join("a"),
                kind: EntryKind::Folder
            },
            Entry {
                name: "repo".into(),
                path: root.join("repo"),
                kind: EntryKind::Checkout {
                    branch: Some("main".into())
                }
            },
        ]
    );
}

#[test]
fn list_sorts_case_insensitively() {
    let (_t, root) = tmp();
    for n in ["b", "A", "c"] {
        std::fs::create_dir(root.join(n)).unwrap();
    }
    let names: Vec<_> = list(&root, &[])
        .unwrap()
        .entries
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["A", "b", "c"]);
}

#[test]
fn list_marks_the_excluded_root() {
    let (_t, root) = tmp();
    std::fs::create_dir_all(root.join("tasks/p1")).unwrap();
    std::fs::create_dir(root.join("other")).unwrap();
    let l = list(&root, &[root.join("tasks")]).unwrap();
    let kinds: Vec<_> = l
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e.kind.clone()))
        .collect();
    assert_eq!(
        kinds,
        [("other", EntryKind::Folder), ("tasks", EntryKind::Excluded)]
    );
    let inside = list(&root.join("tasks"), &[root.join("tasks")]).unwrap();
    assert_eq!(inside.entries[0].kind, EntryKind::Excluded);
}

#[test]
fn list_marks_a_task_under_an_old_root() {
    let (_t, root) = tmp();
    std::fs::create_dir_all(root.join("old/p1")).unwrap();
    std::fs::create_dir_all(root.join("old/stray")).unwrap();
    let l = list(
        &root.join("old"),
        &[root.join("tasks"), root.join("old/p1")],
    )
    .unwrap();
    let kinds: Vec<_> = l
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e.kind.clone()))
        .collect();
    assert_eq!(
        kinds,
        [("p1", EntryKind::Excluded), ("stray", EntryKind::Folder)]
    );
}

#[cfg(unix)]
#[test]
fn list_follows_symlinks_to_folders() {
    let (_t, root) = tmp();
    std::fs::create_dir(root.join("real")).unwrap();
    std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
    let names: Vec<_> = list(&root, &[])
        .unwrap()
        .entries
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["link", "real"]);
}

#[test]
fn list_of_a_missing_dir_is_an_io_error() {
    let (_t, root) = tmp();
    assert!(matches!(list(&root.join("nope"), &[]), Err(FsError::Io(_))));
}

#[test]
fn head_reads_branch_commit_and_subject() {
    let (_t, root) = tmp();
    repo_with_commit(&root);
    let h = head(&root).unwrap();
    assert_eq!(h.branch.as_deref(), Some("main"));
    assert_eq!(h.subject, "first");
    assert_eq!(h.commit, Some(git(&root, &["rev-parse", "HEAD"])));
}

#[test]
fn head_of_an_unborn_repo_has_no_commit() {
    let (_t, root) = tmp();
    init(&root);
    let h = head(&root).unwrap();
    assert_eq!(h.commit, None);
    assert_eq!(h.branch.as_deref(), Some("main"));
}

#[test]
fn head_of_a_detached_checkout_has_no_branch() {
    let (_t, root) = tmp();
    repo_with_commit(&root);
    git(&root, &["checkout", "-q", "--detach"]);
    let h = head(&root).unwrap();
    assert_eq!(h.branch, None);
    assert!(h.commit.is_some());
}

#[test]
fn head_of_a_non_repo_is_not_a_repo() {
    let (_t, root) = tmp();
    assert!(matches!(head(&root), Err(FsError::NotARepo(_))));
}

#[test]
fn dirty_count_counts_tracked_and_untracked_not_ignored() {
    let (_t, root) = tmp();
    init(&root);
    std::fs::write(root.join("f.txt"), "one\n").unwrap();
    std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "first"]);
    assert_eq!(dirty_count(&root).unwrap(), 0);
    std::fs::write(root.join("f.txt"), "two\n").unwrap();
    std::fs::write(root.join("new.txt"), "x").unwrap();
    std::fs::write(root.join("skip.log"), "x").unwrap();
    assert_eq!(dirty_count(&root).unwrap(), 2);
}

#[test]
fn toplevel_of_a_subfolder_is_its_checkout() {
    let (_t, root) = tmp();
    repo_with_commit(&root.join("repo"));
    std::fs::create_dir_all(root.join("repo/src/deep")).unwrap();
    assert_eq!(
        toplevel(&root.join("repo/src/deep")).unwrap(),
        Some(root.join("repo"))
    );
    assert_eq!(toplevel(&root).unwrap(), None);
}

#[test]
fn toplevel_of_a_linked_worktree_subfolder_is_the_worktree() {
    let (_t, root) = tmp();
    let repo = root.join("repo");
    repo_with_commit(&repo);
    let wt = root.join("feat");
    git(
        &repo,
        &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()],
    );
    std::fs::create_dir(wt.join("sub")).unwrap();
    assert_eq!(toplevel(&wt.join("sub")).unwrap(), Some(wt));
}

fn repo_with_worktrees(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let repo = root.join("repo");
    repo_with_commit(&repo);
    let feat = root.join("feat");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat",
            feat.to_str().unwrap(),
        ],
    );
    let excl = root.join("tasks");
    std::fs::create_dir(&excl).unwrap();
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "p1",
            excl.join("p1").to_str().unwrap(),
        ],
    );
    (repo, feat, excl)
}

#[test]
fn checkouts_lists_main_then_worktrees_and_skips_excluded() {
    let (_t, root) = tmp();
    let (repo, feat, excl) = repo_with_worktrees(&root);
    for from in [&repo, &feat] {
        let c = checkouts(from, std::slice::from_ref(&excl)).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(
            (c[0].path.clone(), c[0].main, c[0].head.branch.as_deref()),
            (repo.clone(), true, Some("main"))
        );
        assert_eq!(
            (c[1].path.clone(), c[1].main, c[1].head.branch.as_deref()),
            (feat.clone(), false, Some("feat"))
        );
    }
    assert_eq!(checkouts(&repo, &[]).unwrap().len(), 3);
}

#[test]
fn checkouts_skips_a_deleted_worktree() {
    let (_t, root) = tmp();
    let (repo, feat, excl) = repo_with_worktrees(&root);
    std::fs::remove_dir_all(&feat).unwrap();
    let c = checkouts(&repo, std::slice::from_ref(&excl)).unwrap();
    assert_eq!(c.len(), 1);
    assert!(c[0].main);
}

#[test]
fn checkouts_skips_a_worktree_whose_git_file_points_nowhere() {
    let (_t, root) = tmp();
    let (repo, feat, excl) = repo_with_worktrees(&root);
    std::fs::write(feat.join(".git"), "gitdir: /nonexistent/nowhere\n").unwrap();
    let c = checkouts(&repo, std::slice::from_ref(&excl)).unwrap();
    assert_eq!(c.len(), 1);
    assert!(c[0].main);
}

#[test]
fn main_root_of_a_subdir_is_the_toplevel() {
    let (_t, root) = tmp();
    let repo = root.join("proj");
    repo_with_commit(&repo);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    assert!(
        matches!(main_root(&repo.join("src")).unwrap(), Some(MainRoot::Worktree(p)) if p == repo)
    );
    assert!(main_root(&root).unwrap().is_none());
}

#[test]
fn main_root_of_a_linked_worktree_is_the_main_repo() {
    let (_t, root) = tmp();
    let (repo, feat, _) = repo_with_worktrees(&root);
    assert!(matches!(main_root(&feat).unwrap(), Some(MainRoot::Worktree(p)) if p == repo));
}

#[test]
fn main_root_of_a_bare_repo_is_bare() {
    let (_t, root) = tmp();
    let seed = root.join("seed");
    repo_with_commit(&seed);
    let bare = root.join("up.git");
    git(
        &root,
        &[
            "clone",
            "-q",
            "--bare",
            seed.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    assert!(matches!(main_root(&bare).unwrap(), Some(MainRoot::Bare)));
}

#[test]
fn normalise_resolves_dot_dot_lexically() {
    for (raw, want) in [
        ("/a/b/../c", "/a/c"),
        ("/a/./b/", "/a/b"),
        ("/..", "/"),
        ("/a/../..", "/"),
        ("a/../../b", "../b"),
    ] {
        assert_eq!(
            fleet_fs::normalise(Path::new(raw)),
            PathBuf::from(want),
            "{raw}"
        );
    }
}
