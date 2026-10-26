use eframe::egui::{accesskit::Role, Key, Modifiers, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use fleet_client::config::GuiSettings;
use fleet_gui::components::{browse, FolderPicker, PickMode, Scan};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// `gui.toml` once every background write asked for so far has landed.
fn on_disk(path: &std::path::Path) -> fleet_client::config::GuiSettings {
    fleet_client::config::SettingsWriter::shared().flush();
    fleet_client::config::GuiSettings::load(path)
}

fn git(dir: &Path, args: &[&str]) {
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
}

/// `root/dev` holds a repo `core` on `main`, its linked worktree `core-feat` on `feat`, a plain folder `notes`
/// and the tasks root `tasks`; `root/home` stands in for the home folder.
struct Tree {
    _t: tempfile::TempDir,
    root: PathBuf,
}

impl Tree {
    fn new() -> Tree {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        let core = root.join("dev/core");
        std::fs::create_dir_all(&core).unwrap();
        git(&core, &["init", "-q"]);
        std::fs::write(core.join("f.txt"), "one\n").unwrap();
        git(&core, &["add", "."]);
        git(&core, &["commit", "-q", "-m", "first commit"]);
        git(
            &core,
            &["worktree", "add", "-q", "-b", "feat", "../core-feat"],
        );
        for d in [
            "dev/notes/inner",
            "dev/tasks/p1",
            "home/docs",
            "elsewhere/stale-only",
        ] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        Tree { _t: t, root }
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn settings(&self) -> PathBuf {
        self.p("gui.toml")
    }

    fn picker(&self, mode: PickMode) -> FolderPicker {
        FolderPicker::new(
            mode,
            self.settings(),
            vec![self.p("dev/tasks")],
            self.p("home"),
        )
        .without_scans()
    }
}

struct S {
    picker: FolderPicker,
    text: String,
    picked: usize,
}

fn harness(picker: FolderPicker, text: &str) -> Harness<'static, S> {
    Harness::builder()
        .with_max_steps(60)
        .with_size(Vec2::new(900.0, 700.0))
        .build_ui_state(
            |ui, s: &mut S| {
                if browse(ui, &mut s.picker, &mut s.text) {
                    s.picked += 1;
                }
            },
            S {
                picker,
                text: text.into(),
                picked: 0,
            },
        )
}

fn open(h: &mut Harness<'static, S>) {
    h.get_by_role_and_label(Role::Button, "Browse…").click();
    h.run();
}

fn path_bar(h: &Harness<'static, S>) -> String {
    h.get_by_label("Folder path").value().unwrap_or_default()
}

fn show(p: &Path) -> String {
    p.display().to_string()
}

fn deliver(h: &mut Harness<'static, S>, scan: Scan) {
    h.state_mut().picker.deliver(scan);
    h.run();
}

fn list(h: &mut Harness<'static, S>, dir: &Path) {
    deliver(h, Scan::Listed(fleet_fs::list(dir, &[]).unwrap()));
}

fn row(h: &Harness<'static, S>, label: &str) -> bool {
    h.query_by_role_and_label(Role::Button, label).is_some()
}

fn click(h: &mut Harness<'static, S>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).click();
    h.run();
}

fn select_disabled(h: &Harness<'static, S>) -> bool {
    h.get_by_role_and_label(Role::Button, "Select")
        .accesskit_node()
        .is_disabled()
}

/// Opens the repo picker at `dev`, lists it and expands `core`'s checkouts.
fn expanded_core(t: &Tree) -> Harness<'static, S> {
    let mut h = harness(t.picker(PickMode::Repo), &show(&t.p("dev")));
    open(&mut h);
    list(&mut h, &t.p("dev"));
    click(&mut h, "core");
    deliver(
        &mut h,
        Scan::Checkouts {
            repo: t.p("dev/core"),
            list: Ok(fleet_fs::checkouts(&t.p("dev/core"), &[]).unwrap()),
        },
    );
    h
}

#[test]
fn browse_opens_at_the_text_when_it_is_a_folder() {
    let t = Tree::new();
    let mut h = harness(
        t.picker(PickMode::Folder),
        &format!("  {}  ", show(&t.p("dev/notes"))),
    );
    open(&mut h);
    assert!(h.state().picker.is_open());
    h.get_by_label("Choose a folder");
    assert_eq!(path_bar(&h), show(&t.p("dev/notes")));
}

#[test]
fn browse_expands_a_leading_tilde_with_home() {
    let t = Tree::new();
    let mut h = harness(t.picker(PickMode::Folder), "~/docs");
    open(&mut h);
    assert_eq!(path_bar(&h), show(&t.p("home/docs")));
}

#[test]
fn browse_opens_at_last_pick_dir_then_home() {
    let t = Tree::new();
    GuiSettings::update(&t.settings(), |s| s.last_pick_dir = Some(t.p("dev"))).unwrap();
    let mut h = harness(t.picker(PickMode::Repo), "/no/such/folder");
    open(&mut h);
    h.get_by_label("Choose a repo");
    assert_eq!(path_bar(&h), show(&t.p("dev")));

    click(&mut h, "Cancel");
    std::fs::remove_dir_all(t.p("dev")).unwrap();
    open(&mut h);
    assert_eq!(path_bar(&h), show(&t.p("home")));
}

#[test]
fn repo_mode_skips_a_last_pick_dir_under_the_tasks_root() {
    let t = Tree::new();
    GuiSettings::update(&t.settings(), |s| {
        s.last_pick_dir = Some(t.p("dev/tasks/p1"))
    })
    .unwrap();
    let mut h = harness(t.picker(PickMode::Repo), "");
    open(&mut h);
    assert_eq!(
        path_bar(&h),
        show(&t.p("home")),
        "nothing there can be picked, so start at home"
    );
}

#[test]
fn repo_mode_footer_names_the_chosen_checkout() {
    let t = Tree::new();
    let mut h = expanded_core(&t);
    let core = show(&t.p("dev/core"));
    assert!(h.query_by_label(&format!("Select: {core}")).is_none());
    // The row's own label is its path; the footer is the second match for it.
    h.get_all_by_role_and_label(Role::Button, &core)
        .next()
        .unwrap()
        .click();
    h.run();
    h.get_by_label(&format!("Select: {core}"));
}

#[test]
fn repo_mode_select_is_disabled_until_a_checkout_is_chosen() {
    let t = Tree::new();
    let mut h = harness(t.picker(PickMode::Repo), "");
    GuiSettings::update(&t.settings(), |s| s.last_pick_dir = Some(t.p("dev"))).unwrap();
    open(&mut h);
    list(&mut h, &t.p("dev"));
    assert!(select_disabled(&h));
    click(&mut h, "Select");
    assert!(h.state().picker.is_open());

    click(&mut h, "core");
    deliver(
        &mut h,
        Scan::Checkouts {
            repo: t.p("dev/core"),
            list: Ok(fleet_fs::checkouts(&t.p("dev/core"), &[]).unwrap()),
        },
    );
    assert!(
        select_disabled(&h),
        "expanding a repo chooses none of its checkouts"
    );
    let feat = show(&t.p("dev/core-feat"));
    let note = h
        .get_by_role_and_label(Role::Button, &feat)
        .accesskit_node()
        .description();
    assert_eq!(
        note.as_deref(),
        Some("Merge fleet/<task> into feat first (a fast-forward if you did not commit there meanwhile), then merge feat as usual.")
    );
    assert_eq!(
        h.get_all_by_label("first commit").count(),
        2,
        "each checkout shows its last commit"
    );
    let commit = fleet_fs::head(&t.p("dev/core")).unwrap().commit.unwrap();
    assert_eq!(
        h.get_all_by_label(&commit[..7]).count(),
        2,
        "and its short hash"
    );

    click(&mut h, &feat);
    assert!(!select_disabled(&h));
    click(&mut h, "Select");
    assert_eq!(h.state().text, feat);
    assert_eq!(h.state().picked, 1);
    assert!(!h.state().picker.is_open());
    assert_eq!(on_disk(&t.settings()).last_pick_dir, Some(t.p("dev")));
}

#[test]
fn folder_mode_select_takes_the_current_folder() {
    let t = Tree::new();
    let mut h = harness(t.picker(PickMode::Folder), &show(&t.p("dev")));
    open(&mut h);
    list(&mut h, &t.p("dev"));
    click(&mut h, "notes");
    assert_eq!(path_bar(&h), show(&t.p("dev/notes")));
    list(&mut h, &t.p("dev/notes"));
    assert!(row(&h, "inner"));
    click(&mut h, "Select");
    assert_eq!(h.state().text, show(&t.p("dev/notes")));
    assert!(!h.state().picker.is_open());
    assert_eq!(on_disk(&t.settings()).last_pick_dir, Some(t.p("dev/notes")));
}

#[test]
fn up_and_home_navigate() {
    let t = Tree::new();
    let mut h = harness(t.picker(PickMode::Folder), &show(&t.p("dev/notes")));
    open(&mut h);
    click(&mut h, "Up");
    assert_eq!(path_bar(&h), show(&t.p("dev")));
    click(&mut h, "Home");
    assert_eq!(path_bar(&h), show(&t.p("home")));
}

#[test]
fn enter_in_the_path_bar_navigates() {
    let t = Tree::new();
    let mut h = harness(t.picker(PickMode::Folder), &show(&t.p("dev")));
    open(&mut h);
    list(&mut h, &t.p("dev"));
    let bar = h.get_by_label("Folder path");
    bar.focus();
    bar.type_text("/notes");
    h.key_press(Key::Enter);
    h.run();
    list(&mut h, &t.p("dev/notes"));
    assert!(row(&h, "inner"));
}

#[test]
fn dot_dot_in_the_path_bar_is_resolved() {
    let t = Tree::new();
    let mut h = harness(t.picker(PickMode::Folder), &show(&t.p("dev")));
    open(&mut h);
    list(&mut h, &t.p("dev"));
    let bar = h.get_by_label("Folder path");
    bar.focus();
    bar.type_text("/notes/../notes/inner/..");
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(path_bar(&h), show(&t.p("dev/notes")));
    list(&mut h, &t.p("dev/notes"));
    click(&mut h, "Up");
    assert_eq!(path_bar(&h), show(&t.p("dev")));
}

#[test]
fn a_first_folder_that_fails_does_not_say_reading() {
    let t = Tree::new();
    let mut h = harness(t.picker(PickMode::Folder), &show(&t.p("dev")));
    open(&mut h);
    deliver(
        &mut h,
        Scan::Failed {
            dir: t.p("dev"),
            error: "Cannot open dev: Permission denied".into(),
        },
    );
    assert!(h.query_by_label("Reading folder…").is_none());
    h.get_by_label("Can't open this folder");
    assert!(select_disabled(&h));
}

#[test]
fn cancel_leaves_the_text_alone() {
    let t = Tree::new();
    let mut h = harness(t.picker(PickMode::Folder), "typed by hand");
    open(&mut h);
    list(&mut h, &t.p("home"));
    click(&mut h, "Cancel");
    assert!(!h.state().picker.is_open());
    assert_eq!(h.state().text, "typed by hand");

    open(&mut h);
    h.key_press(Key::Escape);
    h.run();
    assert!(!h.state().picker.is_open());
    assert_eq!(h.state().text, "typed by hand");
    assert_eq!(h.state().picked, 0);
    assert_eq!(on_disk(&t.settings()).last_pick_dir, None);
}

#[test]
fn excluded_folder_is_not_selectable_in_repo_mode() {
    let t = Tree::new();
    let mut h = harness(t.picker(PickMode::Repo), &show(&t.p("dev")));
    open(&mut h);
    deliver(
        &mut h,
        Scan::Listed(fleet_fs::list(&t.p("dev"), &[t.p("dev/tasks")]).unwrap()),
    );
    click(&mut h, "tasks");
    assert_eq!(
        path_bar(&h),
        show(&t.p("dev")),
        "an excluded folder is not opened"
    );
    assert!(row(&h, "core"));
    assert!(select_disabled(&h));
}

#[test]
fn a_failed_scan_shows_the_error_and_keeps_the_listing() {
    let t = Tree::new();
    let mut h = harness(t.picker(PickMode::Folder), &show(&t.p("dev")));
    open(&mut h);
    list(&mut h, &t.p("dev"));
    let missing = t.p("missing");
    h.get_by_label("Folder path").focus();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_label("Folder path").type_text(&show(&missing));
    h.key_press(Key::Enter);
    h.run();
    deliver(
        &mut h,
        Scan::Failed {
            dir: missing.clone(),
            error: "Cannot open the missing folder".into(),
        },
    );
    assert!(h.state().picker.is_open());
    h.get_by_label("Cannot open the missing folder");
    assert!(
        row(&h, "core") && row(&h, "notes"),
        "the previous listing stays"
    );
    click(&mut h, "Select");
    assert_eq!(
        h.state().text,
        show(&t.p("dev")),
        "Select takes the folder that is listed, not the one that failed"
    );
}

#[test]
fn a_stale_listing_is_ignored() {
    let t = Tree::new();
    let mut h = harness(t.picker(PickMode::Folder), &show(&t.p("dev")));
    open(&mut h);
    click(&mut h, "Home");
    list(&mut h, &t.p("elsewhere"));
    assert!(
        !row(&h, "stale-only"),
        "a listing for a folder we left is dropped"
    );
    list(&mut h, &t.p("home"));
    assert!(row(&h, "docs"));
    list(&mut h, &t.p("elsewhere"));
    assert!(row(&h, "docs") && !row(&h, "stale-only"));
}

#[test]
fn dirty_note_is_singular_for_one() {
    let t = Tree::new();
    let mut h = expanded_core(&t);
    deliver(
        &mut h,
        Scan::Dirty {
            path: t.p("dev/core"),
            count: 1,
        },
    );
    h.get_by_label("1 uncommitted file won't be included");
    deliver(
        &mut h,
        Scan::Dirty {
            path: t.p("dev/core-feat"),
            count: 3,
        },
    );
    h.get_by_label("3 uncommitted files won't be included");
}

#[test]
fn a_detached_checkout_has_no_merge_note() {
    let t = Tree::new();
    git(&t.p("dev/core-feat"), &["checkout", "-q", "--detach"]);
    let h = expanded_core(&t);
    let feat = show(&t.p("dev/core-feat"));
    let row = h.get_by_role_and_label(Role::Button, &feat);
    assert_eq!(row.accesskit_node().description(), None);
    assert!(h.get_all_by_label("detached").count() >= 1);
}

#[test]
fn clicking_the_expanded_repo_again_collapses_it() {
    let t = Tree::new();
    let mut h = expanded_core(&t);
    let feat = show(&t.p("dev/core-feat"));
    click(&mut h, &feat);
    click(&mut h, "core");
    assert!(!row(&h, &feat));
    assert!(select_disabled(&h));
}

/// The real thing: the scan runs on a thread, the frame keeps drawing, and the listing shows up when it lands.
#[test]
fn scans_run_in_the_background() {
    let t = Tree::new();
    let picker = FolderPicker::new(PickMode::Repo, t.settings(), vec![], t.p("home"));
    let mut h = harness(picker, &show(&t.p("dev")));
    open(&mut h);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !row(&h, "core") {
        assert!(Instant::now() < deadline, "the listing never arrived");
        std::thread::sleep(Duration::from_millis(10));
        h.run();
    }
    click(&mut h, "core");
    let feat = show(&t.p("dev/core-feat"));
    while !row(&h, &feat) {
        assert!(Instant::now() < deadline, "the checkouts never arrived");
        std::thread::sleep(Duration::from_millis(10));
        h.run();
    }
}

/// The start folder is found on the scan thread too (an `is_dir` on a hung mount would freeze the window): the path
/// bar fills in when it lands.
#[test]
fn the_start_folder_is_found_in_the_background() {
    let t = Tree::new();
    std::fs::create_dir_all(t.p("home")).unwrap();
    let picker = FolderPicker::new(PickMode::Folder, t.settings(), vec![], t.p("home"));
    let mut h = harness(picker, "");
    open(&mut h);
    let deadline = Instant::now() + Duration::from_secs(10);
    while h
        .get_by_role_and_label(Role::Button, "Select")
        .accesskit_node()
        .is_disabled()
    {
        assert!(Instant::now() < deadline, "the start folder never arrived");
        std::thread::sleep(Duration::from_millis(10));
        h.run();
    }
    assert_eq!(path_bar(&h), show(&t.p("home")));
}

// ---- long paths ----

/// `root/<long>/dev/foo` and its worktree `root/<long>/dev/foo-feat`: the paths differ only at the very end.
fn long_tree() -> (tempfile::TempDir, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let dev = t.path().canonicalize().unwrap().join("a-rather-long-site-folder/another-long-segment-of-the-path/yet-another-long-directory-name/dev");
    let repo = dev.join("foo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    std::fs::write(repo.join("f.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "first commit"]);
    git(
        &repo,
        &["worktree", "add", "-q", "-b", "feat", "../foo-feat"],
    );
    (t, dev)
}

/// The text drawn by plain labels (not the rows' accessible names, which carry the full path).
fn drawn(h: &Harness<'static, S>, containing: &str) -> Vec<String> {
    h.query_all_by_role(Role::Label)
        .filter_map(|n| {
            n.accesskit_node()
                .value()
                .or_else(|| n.accesskit_node().label())
        })
        .filter(|t| t.contains(containing))
        .collect()
}

#[test]
fn long_checkout_paths_keep_the_part_that_tells_them_apart() {
    let (_t, dev) = long_tree();
    let picker = FolderPicker::new(PickMode::Repo, dev.join("gui.toml"), vec![], dev.clone())
        .without_scans();
    let mut h = harness(picker, &show(&dev));
    open(&mut h);
    list(&mut h, &dev);
    click(&mut h, "foo");
    deliver(
        &mut h,
        Scan::Checkouts {
            repo: dev.join("foo"),
            list: Ok(fleet_fs::checkouts(&dev.join("foo"), &[]).unwrap()),
        },
    );
    for last in ["foo", "foo-feat"] {
        let full = show(&dev.join(last));
        let seen = drawn(&h, &format!("/{last}"));
        let seen = seen
            .iter()
            .find(|t| t.ends_with(&format!("/{last}")))
            .unwrap_or_else(|| panic!("no label ends with /{last}: {seen:?}"));
        assert!(
            seen.contains('…') && seen.len() < full.len(),
            "{seen} is not an elided {full}"
        );
        assert!(seen.starts_with('/'), "{seen} lost its root");
    }
}

#[test]
fn the_select_footer_elides_its_path_and_keeps_the_prefix() {
    let (_t, dev) = long_tree();
    let picker = FolderPicker::new(PickMode::Repo, dev.join("gui.toml"), vec![], dev.clone())
        .without_scans();
    let mut h = harness(picker, &show(&dev));
    open(&mut h);
    list(&mut h, &dev);
    click(&mut h, "foo");
    deliver(
        &mut h,
        Scan::Checkouts {
            repo: dev.join("foo"),
            list: Ok(fleet_fs::checkouts(&dev.join("foo"), &[]).unwrap()),
        },
    );
    click(&mut h, &show(&dev.join("foo-feat")));
    let footer = drawn(&h, "Select: ");
    assert!(
        footer
            .iter()
            .any(|t| t.starts_with("Select: /") && t.contains('…') && t.ends_with("/foo-feat")),
        "{footer:?}"
    );
    let tails = drawn(&h, "foo-feat");
    assert!(
        tails.len() >= 2,
        "row and footer should both show the tail: {tails:?}"
    );
    assert!(tails.iter().all(|t| t.chars().count() < 100), "{tails:?}");
}
