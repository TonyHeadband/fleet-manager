mod common;

use eframe::egui::{self, accesskit::Role, Key, Modifiers, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use fleet_api::{
    default_ui_schema, Agent, AgentId, BlockedReason, DiffReport, FieldDef, FieldKind, GithubAuth,
    Persona, RepoInfo, RepoRequest, Status, StopCause, UiSchema, Workspace, WorkspaceDiff,
};
use fleet_gui::components::{
    filter_summary, new_task_dialog, preview_pane, reply_box, repo_filter, repo_request_box,
    workspace_tip, AgentDetails, DiffView, FolderPicker, PickMode, PreviewAction, PreviewState,
    PreviewTab, RepoPanel, RequestDecision, RequestDraft, Scan, SpawnAction, SpawnState,
};
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

fn repos() -> Vec<String> {
    vec!["core".into(), "portal".into(), "infra".into()]
}

fn remembered() -> Vec<RepoInfo> {
    repos()
        .into_iter()
        .map(|name| RepoInfo {
            path: Some(format!("/r/{name}")),
            // A URL: the fixture path is not a checkout to list.
            url: Some(format!("https://example.invalid/{name}.git")),
            name,
            product: "p".into(),
            roster: vec![],
            workflow: fleet_api::WorkflowState::Missing {
                base: "main".into(),
            },
        })
        .collect()
}

/// A picker that never reads the disk, for tests that only look at the button and the dialog.
fn inert_picker() -> FolderPicker {
    FolderPicker::new(
        PickMode::Repo,
        PathBuf::from("/no/such/gui.toml"),
        vec![],
        PathBuf::from("/no/such/home"),
    )
    .without_scans()
}

/// `dev/lathe` is a git repo with one commit; `dev` is the picker's home, so it opens there.
struct Dev {
    _t: tempfile::TempDir,
    dev: PathBuf,
}

impl Dev {
    fn new() -> Dev {
        let t = tempfile::tempdir().unwrap();
        let dev = t.path().canonicalize().unwrap().join("dev");
        let core = dev.join("lathe");
        std::fs::create_dir_all(&core).unwrap();
        let git = |args: &[&str]| {
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
                .current_dir(&core)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q"]);
        std::fs::write(core.join("f.txt"), "one\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "first commit"]);
        Dev { _t: t, dev }
    }

    fn lathe(&self) -> PathBuf {
        self.dev.join("lathe")
    }

    fn picker(&self) -> FolderPicker {
        FolderPicker::new(
            PickMode::Repo,
            self.dev.join("gui.toml"),
            vec![],
            self.dev.clone(),
        )
        .without_scans()
    }

    /// Clicks Browse…, then picks `lathe`'s only checkout and Selects it.
    fn pick_lathe<T>(
        &self,
        h: &mut Harness<'static, T>,
        picker: impl Fn(&mut T) -> &mut FolderPicker,
    ) {
        let click = |h: &mut Harness<'static, T>, label: &str| {
            h.get_by_role_and_label(Role::Button, label).click();
            h.run();
        };
        click(h, "Browse…");
        picker(h.state_mut()).deliver(Scan::Listed(fleet_fs::list(&self.dev, &[]).unwrap()));
        h.run();
        click(h, "lathe");
        picker(h.state_mut()).deliver(Scan::Checkouts {
            repo: self.lathe(),
            list: Ok(fleet_fs::checkouts(&self.lathe(), &[]).unwrap()),
        });
        h.run();
        click(h, &self.lathe().display().to_string());
        click(h, "Select");
    }
}

fn counted() -> Vec<(String, usize)> {
    repos()
        .into_iter()
        .enumerate()
        .map(|(i, r)| (r, i + 1))
        .collect()
}

// ---- repo filter ----

fn names(selected: &[&str]) -> Vec<String> {
    selected.iter().map(|s| s.to_string()).collect()
}

#[test]
fn filter_summary_names_the_selection() {
    assert_eq!(filter_summary(&[]), "All repos");
    assert_eq!(filter_summary(&names(&["core"])), "core");
    assert_eq!(filter_summary(&names(&["core", "portal"])), "2 repos");
}

#[test]
fn repo_filter_toggles_repos_in_click_order() {
    let mut h = Harness::new_ui_state(
        |ui, sel: &mut Vec<String>| {
            repo_filter(ui, &counted(), sel);
        },
        Vec::<String>::new(),
    );
    h.get_by_role(Role::ComboBox).click();
    h.run();
    for (repo, expected) in [
        ("core", &["core"][..]),
        ("portal", &["core", "portal"]),
        ("core", &["portal"]),
        ("portal", &[]),
    ] {
        h.get_by_role_and_label(Role::CheckBox, repo).click();
        h.run();
        assert_eq!(h.state(), expected);
    }
}

#[test]
fn all_repos_clears_the_selection() {
    struct S {
        sel: Vec<String>,
        changes: usize,
    }
    let mut h = Harness::new_ui_state(
        |ui, s: &mut S| {
            if repo_filter(ui, &counted(), &mut s.sel) {
                s.changes += 1;
            }
        },
        S {
            sel: names(&["core"]),
            changes: 0,
        },
    );
    h.get_by_role(Role::ComboBox).click();
    h.run();
    h.get_by_role_and_label(Role::CheckBox, "All repos").click();
    h.run();
    assert!(h.state().sel.is_empty());
    assert_eq!(h.state().changes, 1);
}

#[test]
fn repo_filter_reports_a_change_once_per_click() {
    struct S {
        sel: Vec<String>,
        changes: usize,
    }
    let mut h = Harness::new_ui_state(
        |ui, s: &mut S| {
            if repo_filter(ui, &counted(), &mut s.sel) {
                s.changes += 1;
            }
        },
        S {
            sel: vec![],
            changes: 0,
        },
    );
    h.run();
    assert_eq!(h.state().changes, 0);
    h.get_by_role(Role::ComboBox).click();
    h.run();
    h.get_by_role_and_label(Role::CheckBox, "infra").click();
    h.run();
    assert_eq!(h.state().changes, 1);
}

// ---- new agent dialog ----

struct Spawn {
    st: SpawnState,
    started: Vec<(Vec<String>, String)>,
}

fn spawn_harness(error: Option<&str>) -> Harness<'static, Spawn> {
    let st = SpawnState {
        open: true,
        error: error.map(String::from),
        ..SpawnState::new(inert_picker())
    };
    spawn_harness_with(st)
}

/// One ready persona, "Ann" (p).
fn ann() -> Vec<Persona> {
    vec![Persona {
        id: "p".into(),
        name: "Ann".into(),
        host: "github.com".into(),
        commit_name: "Ann Example".into(),
        commit_email: "ann@example.invalid".into(),
        github_auth: GithubAuth::Gh,
        claude_dir: "/personas/p/claude".into(),
        gh_dir: "/personas/p/gh".into(),
        ready: true,
        claude: None,
        github: None,
        tasks: 0,
    }]
}

/// The dialog under Ann, with her repos fetched.
fn spawn_harness_with(mut st: SpawnState) -> Harness<'static, Spawn> {
    st.persona = "p".into();
    st.repos = remembered();
    Harness::builder()
        .with_max_steps(60)
        .with_size(Vec2::new(900.0, 500.0))
        .build_ui_state(
            |ui, s: &mut Spawn| {
                if let Some(SpawnAction::Start {
                    persona,
                    repos,
                    prompt,
                    ..
                }) = new_task_dialog(ui.ctx(), &ann(), &[], None, &mut s.st)
                {
                    assert_eq!(persona, "p");
                    s.started
                        .push((repos.into_iter().map(|r| r.repo).collect(), prompt));
                }
            },
            Spawn {
                st,
                started: vec![],
            },
        )
}

#[test]
fn the_dialog_is_about_440_wide_on_the_shared_modal_frame() {
    let mut h = spawn_harness(None);
    h.run();
    fn walk(shape: &egui::Shape, out: &mut Vec<(egui::Rect, egui::Color32, u8)>) {
        match shape {
            egui::Shape::Rect(r) => out.push((r.rect, r.fill, r.corner_radius.nw)),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut rects = Vec::new();
    h.output()
        .shapes
        .iter()
        .for_each(|c| walk(&c.shape, &mut rects));
    let title = h.get_by_label("New task").rect().center();
    let surface = fleet_gui::theme::tokens(egui::Visuals::default().dark_mode).surface;
    let (frame, _, radius) = rects
        .into_iter()
        .filter(|(r, fill, _)| *fill == surface && r.contains(title))
        .min_by(|a, b| a.0.area().total_cmp(&b.0.area()))
        .expect("the dialog's frame");
    assert!(
        (frame.width() - 440.0).abs() <= 2.0,
        "the dialog is {}px wide",
        frame.width()
    );
    assert_eq!(radius, fleet_gui::theme::radius::DIALOG);
}

#[test]
fn spawn_is_disabled_until_the_prompt_has_text() {
    let mut h = spawn_harness(None);
    h.get_by_role_and_label(Role::Button, "Start task").click();
    h.run();
    assert!(h.state().started.is_empty());
    let prompt = h.get_by_role(Role::MultilineTextInput);
    prompt.click();
    prompt.type_text("   ");
    h.run();
    h.get_by_role_and_label(Role::Button, "Start task").click();
    h.run();
    assert!(h.state().started.is_empty(), "a blank prompt is not a task");
}

#[test]
fn spawn_returns_repo_and_prompt_unchanged() {
    let mut h = spawn_harness(None);
    let prompt = h.get_by_role(Role::MultilineTextInput);
    prompt.click();
    prompt.type_text("fix $(x) \"q\" {a}\nsecond line é");
    h.run();
    h.get_by_role_and_label(Role::Button, "Start task").click();
    h.run();
    assert_eq!(
        h.state().started,
        [(vec![], "fix $(x) \"q\" {a}\nsecond line é".to_string())]
    );
}

#[test]
fn starting_with_no_repos_sends_an_empty_list() {
    let mut h = spawn_harness(None);
    let goal = h.get_by_role(Role::MultilineTextInput);
    goal.click();
    goal.type_text("just think");
    h.run();
    h.get_by_role_and_label(Role::Button, "Start task").click();
    h.run();
    assert_eq!(h.state().started, [(vec![], "just think".to_string())]);
}

fn type_entry(h: &mut Harness<'static, Spawn>, text: &str) {
    let field = h.get_by_label("Path or git URL");
    field.click();
    field.type_text(text);
    h.run();
    h.get_by_role_and_label(Role::Button, "Add").click();
    h.run();
}

#[test]
fn picking_a_remembered_repo_and_typing_a_url_sends_both_in_order() {
    let mut h = spawn_harness(None);
    h.get_by_role_and_label(Role::Button, "portal").click();
    h.run();
    type_entry(&mut h, "git@host:team/x.git");
    let goal = h.get_by_role(Role::MultilineTextInput);
    goal.click();
    goal.type_text("do it");
    h.run();
    h.get_by_role_and_label(Role::Button, "Start task").click();
    h.run();
    assert_eq!(
        h.state().started,
        [(
            vec!["portal".to_string(), "git@host:team/x.git".to_string()],
            "do it".to_string()
        )]
    );
}

#[test]
fn start_includes_a_typed_entry_that_was_not_added() {
    let mut h = spawn_harness(None);
    h.get_by_role_and_label(Role::Button, "core").click();
    h.run();
    let field = h.get_by_label("Path or git URL");
    field.click();
    field.type_text(" /tmp/a ");
    let goal = h.get_by_role(Role::MultilineTextInput);
    goal.click();
    goal.type_text("go");
    h.run();
    h.get_by_role_and_label(Role::Button, "Start task").click();
    h.run();
    assert_eq!(
        h.state().started,
        [(
            vec!["core".to_string(), "/tmp/a".to_string()],
            "go".to_string()
        )]
    );
    assert!(h.state().st.picker.typed.is_empty());
}

#[test]
fn plain_enter_in_the_path_field_adds_a_chip_and_does_not_start() {
    let mut h = spawn_harness(None);
    h.state_mut().st.prompt = "go".into();
    let field = h.get_by_label("Path or git URL");
    field.click();
    field.type_text("/tmp/a");
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().st.picker.chosen, ["/tmp/a"]);
    assert!(h.state().started.is_empty());
}

#[test]
fn typing_the_same_entry_twice_adds_it_once() {
    let mut h = spawn_harness(None);
    type_entry(&mut h, "/tmp/a");
    type_entry(&mut h, "  /tmp/a ");
    type_entry(&mut h, "   ");
    assert_eq!(h.state().st.picker.chosen, ["/tmp/a"]);
}

#[test]
fn removing_a_typed_chip() {
    let mut h = spawn_harness(None);
    type_entry(&mut h, "/tmp/a");
    h.get_by_label("/tmp/a");
    h.get_by_role_and_label(Role::Button, "Remove /tmp/a")
        .click();
    h.run();
    assert!(h.state().st.picker.chosen.is_empty());
    assert!(h.query_by_label("/tmp/a").is_none());
}

#[test]
fn spawn_error_is_shown_and_the_prompt_kept() {
    let mut h = spawn_harness(Some("unknown repo \"x\""));
    h.get_by_label("unknown repo \"x\"");
    let prompt = h.get_by_role(Role::MultilineTextInput);
    prompt.click();
    prompt.type_text("do it");
    h.run();
    assert_eq!(
        h.get_by_role(Role::MultilineTextInput)
            .value()
            .unwrap_or_default(),
        "do it"
    );
}

#[test]
fn a_busy_spawn_cannot_be_sent_twice() {
    let mut h = spawn_harness(None);
    h.state_mut().st.busy = true;
    h.state_mut().st.prompt = "go".into();
    h.run();
    h.get_by_role_and_label(Role::Button, "Start task").click();
    h.run();
    assert!(h.state().started.is_empty());
}

#[test]
fn the_prompt_has_the_focus_when_the_dialog_opens() {
    let h = spawn_harness(None);
    assert!(h.get_by_role(Role::MultilineTextInput).is_focused());
}

#[test]
fn ctrl_enter_starts_without_adding_a_newline() {
    let mut h = spawn_harness(None);
    let prompt = h.get_by_role(Role::MultilineTextInput);
    prompt.click();
    prompt.type_text("do it");
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert_eq!(h.state().started, [(vec![], "do it".to_string())]);
    assert_eq!(h.state().st.prompt, "do it");
}

#[test]
fn ctrl_enter_on_a_blank_prompt_starts_nothing() {
    let mut h = spawn_harness(None);
    h.get_by_role(Role::MultilineTextInput).click();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert!(h.state().started.is_empty());
    assert_eq!(h.state().st.prompt, "");
}

#[test]
fn plain_enter_inserts_a_newline() {
    let mut h = spawn_harness(None);
    let prompt = h.get_by_role(Role::MultilineTextInput);
    prompt.click();
    prompt.type_text("a");
    h.key_press(Key::Enter);
    h.run();
    h.get_by_role(Role::MultilineTextInput).type_text("b");
    h.run();
    assert_eq!(h.state().st.prompt, "a\nb");
    assert!(h.state().started.is_empty());
}

#[test]
fn escape_and_cancel_close_and_keep_the_text() {
    let mut h = spawn_harness(None);
    let prompt = h.get_by_role(Role::MultilineTextInput);
    prompt.click();
    prompt.type_text("keep me");
    h.run();
    h.key_press(Key::Escape);
    h.run();
    assert!(!h.state().st.open);
    assert_eq!(h.state().st.prompt, "keep me");

    h.state_mut().st.open = true;
    h.run();
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run();
    assert!(!h.state().st.open);
    assert_eq!(h.state().st.prompt, "keep me");
}

#[test]
fn the_close_button_closes_and_keeps_the_text() {
    let mut h = spawn_harness(None);
    h.state_mut().st.prompt = "keep me".into();
    h.run();
    h.get_by_role_and_label(Role::Button, "Close").click();
    h.run();
    assert!(!h.state().st.open);
    assert_eq!(h.state().st.prompt, "keep me");
}

#[test]
fn closed_dialog_draws_nothing() {
    let mut h = spawn_harness_with(SpawnState::new(inert_picker()));
    h.run();
    assert!(h.query_by_label("New task").is_none());
}

// ---- reply box ----

struct Reply {
    text: String,
    sent: Vec<String>,
}

fn reply_harness(text: &str) -> Harness<'static, Reply> {
    Harness::builder()
        .with_size(Vec2::new(500.0, 300.0))
        .build_ui_state(
            |ui, r: &mut Reply| {
                if let Some(t) = reply_box(ui, "Answer", &mut r.text, false) {
                    r.sent.push(t);
                }
            },
            Reply {
                text: text.into(),
                sent: vec![],
            },
        )
}

#[test]
fn reply_box_sends_on_ctrl_enter_unchanged() {
    let mut h = reply_harness("");
    let field = h.get_by_role(Role::MultilineTextInput);
    field.click();
    field.type_text("$(x) \"q\" {a} é -n");
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert_eq!(h.state().sent, ["$(x) \"q\" {a} é -n"]);
    assert_eq!(
        h.state().text,
        "$(x) \"q\" {a} é -n",
        "the caller clears the text, not the box"
    );
}

#[test]
fn reply_box_enter_inserts_a_newline() {
    let mut h = reply_harness("");
    let field = h.get_by_role(Role::MultilineTextInput);
    field.click();
    field.type_text("a");
    h.key_press(Key::Enter);
    h.run();
    h.get_by_role(Role::MultilineTextInput).type_text("b");
    h.run();
    assert_eq!(h.state().text, "a\nb");
    assert!(h.state().sent.is_empty());
}

#[test]
fn reply_box_send_button_sends_and_is_disabled_when_blank() {
    let mut h = reply_harness("   ");
    h.get_by_role_and_label(Role::Button, "Send").click();
    h.run();
    assert!(h.state().sent.is_empty(), "whitespace is not an answer");
    h.state_mut().text = "1".into();
    h.run();
    h.get_by_role_and_label(Role::Button, "Send").click();
    h.run();
    assert_eq!(h.state().sent, ["1"]);
}

#[test]
fn reply_box_ctrl_enter_on_a_blank_box_sends_nothing() {
    let mut h = reply_harness("");
    h.get_by_role(Role::MultilineTextInput).click();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert!(h.state().sent.is_empty());
    assert_eq!(h.state().text, "");
}

#[test]
fn ctrl_enter_outside_the_reply_box_sends_nothing() {
    let mut h = reply_harness("draft");
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert!(h.state().sent.is_empty(), "the box did not have the cursor");
}

#[test]
fn preview_without_reply_has_no_reply_box() {
    let h = preview_harness(state("$ ready"));
    assert_eq!(h.query_all_by_role(Role::MultilineTextInput).count(), 0);
    assert!(h.query_by_role_and_label(Role::Button, "Send").is_none());
}

// ---- preview ----

struct Preview {
    st: PreviewState,
    actions: Vec<PreviewAction>,
}

fn preview_harness(st: PreviewState) -> Harness<'static, Preview> {
    Harness::builder()
        .with_size(Vec2::new(700.0, 500.0))
        .build_ui_state(
            |ui, p: &mut Preview| {
                let a = preview_pane(ui, &mut p.st, None, None, None, None);
                if a != PreviewAction::None {
                    p.actions.push(a);
                }
            },
            Preview {
                st,
                actions: vec![],
            },
        )
}

fn state(text: &str) -> PreviewState {
    PreviewState {
        agent_id: "a1".into(),
        title: "fix the tests".into(),
        text: text.into(),
        ..PreviewState::new(inert_picker())
    }
}

fn report() -> DiffReport {
    DiffReport {
        workspaces: vec![
            WorkspaceDiff {
                repo: "core".into(),
                diff: "+++ b/new.txt\n+hello\n-gone\n".into(),
                truncated: false,
            },
            WorkspaceDiff {
                repo: "portal".into(),
                diff: String::new(),
                truncated: false,
            },
            WorkspaceDiff {
                repo: "big".into(),
                diff: "+cut here\n".into(),
                truncated: true,
            },
        ],
    }
}

#[test]
fn preview_terminal_tab_is_read_only() {
    let h = preview_harness(state("$ ready\n> hello"));
    h.get_by_label("fix the tests");
    h.get_by_label_contains("$ ready");
    assert_eq!(
        h.query_all_by_role(Role::TextInput).count()
            + h.query_all_by_role(Role::MultilineTextInput).count(),
        0
    );
}

#[test]
fn pane_text_no_font_can_draw_is_shown_as_boxes_not_a_panic() {
    // Terminal output is full of characters the bundled fonts lack (prompts, box drawing, CJK, emoji). The test
    // harness panics on a missing glyph unless the app installs its policy, which `theme::apply` does.
    let st = state("❯ ● ⎿ 日本語 🙂 ╭──╮\nplain line");
    let mut h = Harness::builder()
        .with_size(Vec2::new(700.0, 500.0))
        .build_ui_state(
            |ui, p: &mut Preview| {
                fleet_gui::theme::apply(ui.ctx());
                preview_pane(ui, &mut p.st, None, None, None, None);
            },
            Preview {
                st,
                actions: vec![],
            },
        );
    h.run();
    h.get_by_label_contains("plain line");
}

#[test]
fn preview_says_session_ended_when_closed() {
    let mut st = state("bye");
    st.closed = true;
    preview_harness(st).get_by_label("Session ended");
}

#[test]
fn session_ended_is_inside_the_panel_above_the_text() {
    let mut st = state("bye");
    st.closed = true;
    let h = preview_harness(st);
    let note = h.get_by_label("Session ended").rect();
    // The harness hosts the pane inside an 8 px outer margin, so its visible bottom is at 492.
    assert!(
        note.top() >= 0.0 && note.bottom() <= 492.0,
        "the note is below the pane's visible bottom: {note:?}"
    );
    let text = h.get_by_label_contains("bye").rect();
    assert!(
        note.bottom() <= text.top(),
        "the note sits above the terminal text: {note:?} vs {text:?}"
    );
}

#[test]
fn a_pane_padded_with_blank_rows_still_shows_its_text() {
    // tmux captures the whole pane, so short output arrives followed by its empty rows; sticking to the bottom of
    // those would scroll the real text out of sight.
    let text = format!("● Write(notes.txt)\n  done{}", "\n".repeat(80));
    let h = preview_harness(state(&text));
    let line = h.get_by_label_contains("Write(notes.txt)").rect();
    assert!(
        line.top() >= 0.0 && line.bottom() <= 492.0,
        "the text is scrolled out of the pane: {line:?}"
    );
}

#[test]
fn header_shows_repo_and_branch_path() {
    let mut st = state("x");
    st.path = "core / fleet/x".into();
    let mut h = preview_harness(st);
    h.get_by_label("core / fleet/x");
    h.get_by_role_and_label(Role::Button, "Close").click();
    h.run();
    assert_eq!(h.state().actions, [PreviewAction::Close]);
}

#[test]
fn diff_tab_requests_a_load_and_shows_each_workspace() {
    let mut h = preview_harness(state("x"));
    assert_eq!(
        h.get_by_label("Terminal").accesskit_node_toggled(),
        Some(true)
    );
    h.get_by_label("Diff").click();
    h.run();
    assert_eq!(h.state().actions, [PreviewAction::LoadDiff]);
    assert_eq!(h.state().st.tab, PreviewTab::Diff);
    assert!(matches!(h.state().st.diff, DiffView::Loading));
    h.state_mut().st.diff = DiffView::Loaded(report());
    h.run();
    for l in ["core", "portal", "big", "+hello", "No changes"] {
        h.get_by_label_contains(l);
    }
    h.get_by_label_contains("cut at the size limit");
}

#[test]
fn diff_failure_shows_the_message_and_refresh_asks_again() {
    let mut h = preview_harness(state("x"));
    h.get_by_label("Diff").click();
    h.run();
    h.state_mut().st.diff = DiffView::Failed("no such agent".into());
    h.run();
    h.get_by_label_contains("no such agent");
    h.get_by_label("Refresh").click();
    h.run();
    assert_eq!(
        h.state().actions,
        [PreviewAction::LoadDiff, PreviewAction::LoadDiff]
    );
}

#[test]
fn switching_back_to_the_terminal_keeps_the_loaded_diff() {
    let mut h = preview_harness(state("x"));
    h.get_by_label("Diff").click();
    h.run();
    h.state_mut().st.diff = DiffView::Loaded(report());
    h.run();
    h.get_by_label("Terminal").click();
    h.run();
    h.get_by_label("Diff").click();
    h.run();
    assert!(
        matches!(h.state().st.diff, DiffView::Loaded(_)),
        "reopening the tab must not discard it"
    );
    assert_eq!(
        h.state().actions.len(),
        1,
        "no second load for a diff already loaded"
    );
}

#[test]
fn long_terminal_lines_keep_one_row_each_instead_of_wrapping_over_their_neighbours() {
    // A tmux pane is 80+ columns, wider than the default panel; wrapped rows broke the virtualised list (found in review).
    let line = |i: usize| format!("row{i:02} {}", "x".repeat(190));
    let text: String = (0..30).map(|i| line(i) + "\n").collect();
    let h = preview_harness(state(&text));
    let rows: Vec<_> = h.query_all_by_label_contains("row").collect();
    assert!(rows.len() >= 10, "only {} rows drawn", rows.len());
    let tops: Vec<f32> = rows.iter().map(|r| r.rect().min.y).collect();
    assert!(
        tops.iter().all(|t| *t >= 0.0),
        "rows were drawn above the pane: {tops:?}"
    );
    let heights: Vec<f32> = rows.iter().map(|r| r.rect().height()).collect();
    let (lo, hi) = heights
        .iter()
        .fold((f32::MAX, 0f32), |(l, h), x| (l.min(*x), h.max(*x)));
    assert!(
        hi < 30.0 && hi - lo < 1.0,
        "rows differ in height, so they wrapped: {heights:?}"
    );
    let mut sorted = tops.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!(
        sorted.windows(2).all(|w| w[1] - w[0] > 5.0),
        "rows overlap: {sorted:?}"
    );
}

#[test]
fn a_one_mebibyte_pane_and_diff_render_in_well_under_a_second() {
    let line = "x".repeat(49) + "\n";
    let big = line.repeat(21_000); // about 1 MiB
    assert!(big.len() > 1_000_000);
    let mut h = preview_harness(state(&big));
    let t = Instant::now();
    h.run();
    let pane = t.elapsed();
    h.get_by_label("Diff").click();
    h.run();
    h.state_mut().st.diff = DiffView::Loaded(DiffReport {
        workspaces: vec![WorkspaceDiff {
            repo: "core".into(),
            diff: big.clone(),
            truncated: false,
        }],
    });
    let t = Instant::now();
    h.run();
    let diff = t.elapsed();
    // A frame lays out only the visible rows; generous bound because tests run unoptimised.
    assert!(
        pane.as_millis() < 750 && diff.as_millis() < 750,
        "pane {pane:?}, diff {diff:?}"
    );
    let _ = egui::Id::NULL;
}

trait Toggled {
    fn accesskit_node_toggled(&self) -> Option<bool>;
}

impl Toggled for egui_kittest::Node<'_> {
    fn accesskit_node_toggled(&self) -> Option<bool> {
        use egui_kittest::kittest::NodeT;
        self.accesskit_node()
            .toggled()
            .map(|t| t == egui::accesskit::Toggled::True)
    }
}

// ---- repos row and repo request ----

struct Requested {
    st: PreviewState,
    workspaces: Vec<Workspace>,
    request: Option<RepoRequest>,
    target: String,
    reason: String,
    browse: FolderPicker,
    replies: String,
    busy: bool,
    actions: Vec<PreviewAction>,
}

fn workspace(repo: &str, owned: bool) -> Workspace {
    Workspace {
        repo: repo.into(),
        branch: "fleet/idea".into(),
        worktree: format!("/w/{repo}"),
        base: "abc".into(),
        from_branch: None,
        has_changes: false,
        owned,
        changed: false,
    }
}

fn requested_harness(
    workspaces: Vec<Workspace>,
    request: Option<RepoRequest>,
) -> Harness<'static, Requested> {
    let target = request.as_ref().map(|r| r.repo.clone()).unwrap_or_default();
    let init = Requested {
        st: state("$ ready"),
        workspaces,
        request,
        target,
        reason: String::new(),
        browse: inert_picker(),
        replies: String::new(),
        busy: false,
        actions: vec![],
    };
    Harness::builder()
        .with_size(Vec2::new(700.0, 600.0))
        .build_ui_state(
            |ui, r: &mut Requested| {
                let remembered = remembered();
                let req = r.request.clone();
                let draft = req.as_ref().map(|request| RequestDraft {
                    request,
                    target: &mut r.target,
                    reason: &mut r.reason,
                    browse: &mut r.browse,
                    blocked: r.busy.then_some("busy"),
                });
                let panel = RepoPanel {
                    workspaces: &r.workspaces,
                    remembered: &remembered,
                    request: draft,
                    add_blocked: None,
                    stopped: false,
                };
                let reply = Some(fleet_gui::components::Reply {
                    label: "Answer",
                    text: &mut r.replies,
                });
                let a = preview_pane(ui, &mut r.st, reply, Some(panel), None, None);
                if a != PreviewAction::None {
                    r.actions.push(a);
                }
            },
            init,
        )
}

fn portal_request() -> Option<RepoRequest> {
    Some(RepoRequest {
        repo: "portal".into(),
        why: "needs the login form".into(),
    })
}

#[test]
fn a_pending_request_replaces_the_reply_box() {
    let h = requested_harness(vec![workspace("core", false)], portal_request());
    assert!(
        h.query_by_label_contains("Answer").is_none(),
        "the reply box was drawn"
    );
    assert!(h.query_by_role_and_label(Role::Button, "Send").is_none());
    h.get_by_label("Wants portal");
    h.get_by_label("needs the login form");
    h.get_by_role_and_label(Role::Button, "Grant");
    h.get_by_role_and_label(Role::Button, "Refuse");
}

#[test]
fn without_a_request_the_reply_box_stays() {
    let h = requested_harness(vec![workspace("core", false)], None);
    h.get_by_role_and_label(Role::Button, "Send");
    assert!(h.query_by_role_and_label(Role::Button, "Grant").is_none());
}

#[test]
fn grant_sends_the_edited_target() {
    let mut h = requested_harness(vec![workspace("core", false)], portal_request());
    h.get_by_role_and_label(Role::TextInput, "Grant access to")
        .value()
        .filter(|v| v == "portal")
        .expect("prefilled with the repo");
    h.state_mut().target = "~/src/portal-fork".into();
    h.run();
    h.get_by_role_and_label(Role::Button, "Grant").click();
    h.run();
    assert_eq!(
        h.state().actions,
        [PreviewAction::Grant("~/src/portal-fork".into())]
    );
}

#[test]
fn ctrl_enter_in_the_target_grants_and_elsewhere_does_not() {
    let mut h = requested_harness(vec![], portal_request());
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert!(
        h.state().actions.is_empty(),
        "the box did not have the cursor"
    );
    h.get_by_role_and_label(Role::TextInput, "Grant access to")
        .click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert_eq!(h.state().actions, [PreviewAction::Grant("portal".into())]);
}

#[test]
fn refuse_sends_the_reason() {
    let mut h = requested_harness(vec![], portal_request());
    h.state_mut().reason = "  not this one ".into();
    h.run();
    h.get_by_role_and_label(Role::Button, "Refuse").click();
    h.run();
    assert_eq!(
        h.state().actions,
        [PreviewAction::Refuse("not this one".into())]
    );
}

#[test]
fn refuse_without_a_reason_sends_an_empty_one() {
    let mut h = requested_harness(vec![], portal_request());
    h.get_by_role_and_label(Role::Button, "Refuse").click();
    h.run();
    assert_eq!(h.state().actions, [PreviewAction::Refuse(String::new())]);
}

#[test]
fn a_blank_target_cannot_be_granted() {
    let mut h = requested_harness(vec![], portal_request());
    h.state_mut().target = "  ".into();
    h.run();
    h.get_by_role_and_label(Role::Button, "Grant").click();
    h.run();
    assert!(h.state().actions.is_empty());
}

#[test]
fn the_request_box_unit_returns_decisions() {
    struct S {
        target: String,
        reason: String,
        browse: FolderPicker,
        got: Vec<RequestDecision>,
    }
    let mut h = Harness::builder()
        .with_size(Vec2::new(500.0, 400.0))
        .build_ui_state(
            |ui, s: &mut S| {
                let req = RepoRequest {
                    repo: "portal".into(),
                    why: String::new(),
                };
                if let Some(d) =
                    repo_request_box(ui, &req, &mut s.target, &mut s.reason, &mut s.browse, None)
                {
                    s.got.push(d);
                }
            },
            S {
                target: "portal".into(),
                reason: String::new(),
                browse: inert_picker(),
                got: vec![],
            },
        );
    h.get_by_role_and_label(Role::Button, "Grant").click();
    h.run();
    assert_eq!(h.state().got, [RequestDecision::Grant("portal".into())]);
}

#[test]
fn the_repos_row_lists_workspaces_and_owned_ones_say_so() {
    let h = requested_harness(
        vec![workspace("core", false), workspace("notes", true)],
        None,
    );
    h.get_by_label("Repos");
    h.get_by_label("core");
    h.get_by_label("notes");
    h.get_by_role_and_label(Role::Button, "Add repo");
}

#[test]
fn add_repo_emits_one_action_per_chosen_entry() {
    let mut h = requested_harness(vec![workspace("core", false)], None);
    h.get_by_role_and_label(Role::Button, "Add repo").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "portal").click();
    h.run();
    let field = h.get_by_role_and_label(Role::TextInput, "Path or git URL");
    field.click();
    field.type_text("git@host:team/extra.git");
    h.run();
    h.get_by_role_and_label(Role::Button, "Add repos").click();
    h.run();
    assert_eq!(
        h.state().actions,
        [
            PreviewAction::AddRepo("portal".into()),
            PreviewAction::AddRepo("git@host:team/extra.git".into())
        ]
    );
    assert!(
        h.query_by_role_and_label(Role::Button, "Add repos")
            .is_none(),
        "the picker closes"
    );
}

#[test]
fn add_repos_is_disabled_until_something_is_chosen() {
    let mut h = requested_harness(vec![], None);
    h.get_by_role_and_label(Role::Button, "Add repo").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Add repos").click();
    h.run();
    assert!(h.state().actions.is_empty());
}

#[test]
fn ctrl_enter_in_the_reason_field_does_not_grant() {
    let mut h = requested_harness(vec![], portal_request());
    h.get_by_role_and_label(Role::TextInput, "Reason (optional)")
        .click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert!(h.state().actions.is_empty());
}

#[test]
fn a_busy_request_cannot_be_decided_again() {
    let mut h = requested_harness(vec![], portal_request());
    h.state_mut().busy = true;
    h.run();
    h.get_by_role_and_label(Role::Button, "Grant").click();
    h.get_by_role_and_label(Role::Button, "Refuse").click();
    h.get_by_role_and_label(Role::TextInput, "Grant access to")
        .click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert!(h.state().actions.is_empty());
}

/// A fresh install: the task has no repo and nothing is remembered yet, and Add repo is the only way in.
#[test]
fn a_zero_repo_agent_with_nothing_remembered_still_offers_add_repo() {
    let h = Harness::builder()
        .with_size(Vec2::new(700.0, 600.0))
        .build_ui_state(
            |ui, st: &mut PreviewState| {
                let panel = RepoPanel {
                    workspaces: &[],
                    remembered: &[],
                    request: None,
                    add_blocked: None,
                    stopped: false,
                };
                preview_pane(ui, st, None, Some(panel), None, None);
            },
            state("$ ready"),
        );
    h.get_by_label("Repos");
    h.get_by_role_and_label(Role::Button, "Add repo");
}

/// A stopped task runs nowhere, so a repo added to it would serve nobody; its repos still show.
#[test]
fn a_stopped_agent_offers_no_add_repo() {
    let workspaces = vec![workspace("core", false)];
    let h = Harness::builder()
        .with_size(Vec2::new(700.0, 600.0))
        .build_ui_state(
            move |ui, st: &mut PreviewState| {
                let panel = RepoPanel {
                    workspaces: &workspaces,
                    remembered: &[],
                    request: None,
                    add_blocked: None,
                    stopped: true,
                };
                preview_pane(ui, st, None, Some(panel), None, None);
            },
            state("$ ready"),
        );
    h.get_by_label("Repos");
    h.get_by_label("core");
    assert!(h
        .query_by_role_and_label(Role::Button, "Add repo")
        .is_none());
}

// ---- Browse in the repo fields ----

#[test]
fn new_task_browse_fills_the_path_field() {
    let dev = Dev::new();
    let mut h = spawn_harness_with(SpawnState::new(dev.picker()));
    h.state_mut().st.open = true;
    h.state_mut().st.prompt = "do it".into();
    h.run();
    // `SpawnState::new(dev.picker())` is the dialog's repo picker; the field's own Browse… is the only one on screen.
    dev.pick_lathe(&mut h, |s| &mut s.st.picker.browse);
    let want = dev.lathe().display().to_string();
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Path or git URL")
            .value()
            .as_deref(),
        Some(want.as_str())
    );
    assert!(
        h.state().started.is_empty(),
        "Browse must only fill the field"
    );
    h.get_by_role_and_label(Role::Button, "Start task").click();
    h.run();
    assert_eq!(h.state().started, [(vec![want], "do it".to_string())]);
}

#[test]
fn new_task_browse_leaves_the_cursor_at_the_end_of_a_long_path() {
    let dev = Dev::new();
    let mut h = spawn_harness_with(SpawnState::new(dev.picker()));
    h.state_mut().st.open = true;
    h.run();
    dev.pick_lathe(&mut h, |s| &mut s.st.picker.browse);
    let want = dev.lathe().display().to_string();
    let field = h.get_by_role_and_label(Role::TextInput, "Path or git URL");
    let node = field.accesskit_node();
    let sel = node.text_selection().expect("the field has no selection");
    assert_eq!(sel.end().to_global_usv_index(), want.chars().count());
    assert_eq!(sel.start().to_global_usv_index(), want.chars().count());
}

#[test]
fn escape_closes_only_the_picker_over_the_new_task_dialog() {
    let mut h = spawn_harness(None);
    h.run();
    h.get_by_role_and_label(Role::Button, "Browse…").click();
    h.run();
    assert!(h.state().st.picker.browse.is_open());
    h.get_by_label("Choose a repo");
    h.key_press(Key::Escape);
    h.run();
    assert!(
        !h.state().st.picker.browse.is_open(),
        "the picker stayed open"
    );
    assert!(h.state().st.open, "Escape closed the New task dialog too");
    h.get_by_label("New task");
}

#[test]
fn add_repo_browse_fills_the_field() {
    let dev = Dev::new();
    let mut h = requested_harness(vec![workspace("core", false)], None);
    h.state_mut().st.picker.browse = dev.picker();
    h.get_by_role_and_label(Role::Button, "Add repo").click();
    h.run();
    dev.pick_lathe(&mut h, |r| &mut r.st.picker.browse);
    let want = dev.lathe().display().to_string();
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Path or git URL")
            .value()
            .as_deref(),
        Some(want.as_str())
    );
    assert!(
        h.state().actions.is_empty(),
        "Browse must only fill the field"
    );
    h.get_by_role_and_label(Role::Button, "Add repos").click();
    h.run();
    assert_eq!(h.state().actions, [PreviewAction::AddRepo(want)]);
}

#[test]
fn grant_browse_replaces_the_target() {
    let dev = Dev::new();
    let mut h = requested_harness(vec![workspace("core", false)], portal_request());
    h.state_mut().browse = dev.picker();
    dev.pick_lathe(&mut h, |r| &mut r.browse);
    let want = dev.lathe().display().to_string();
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Grant access to")
            .value()
            .as_deref(),
        Some(want.as_str())
    );
    assert!(h.state().actions.is_empty());
    h.get_by_role_and_label(Role::Button, "Grant").click();
    h.run();
    assert_eq!(h.state().actions, [PreviewAction::Grant(want)]);
}

#[test]
fn ctrl_enter_right_after_browse_grants_the_picked_path() {
    let dev = Dev::new();
    let mut h = requested_harness(vec![workspace("core", false)], portal_request());
    h.state_mut().browse = dev.picker();
    dev.pick_lathe(&mut h, |r| &mut r.browse);
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert_eq!(
        h.state().actions,
        [PreviewAction::Grant(dev.lathe().display().to_string())]
    );
}

#[test]
fn workspace_tip_names_start_and_merge_route() {
    let mut w = workspace("core", false);
    w.branch = "fleet/x".into();
    w.base = "abcdef1234".into();
    w.from_branch = Some("feat".into());
    assert_eq!(
        workspace_tip(&w),
        "fleet/x, started from feat at abcdef1\n\nMerge fleet/x into feat first (a fast-forward if you did not commit there meanwhile), then merge feat as usual."
    );
}

#[test]
fn workspace_tip_for_a_url_repo_has_no_merge_note() {
    let mut w = workspace("core", false);
    w.branch = "fleet/x".into();
    w.base = "abcdef1234".into();
    w.from_branch = Some("origin/main".into());
    assert_eq!(
        workspace_tip(&w),
        "fleet/x, started from origin/main at abcdef1"
    );
}

#[test]
fn workspace_tip_without_from_branch() {
    let mut w = workspace("core", false);
    w.branch = "fleet/x".into();
    w.base = "abcdef1234".into();
    assert_eq!(workspace_tip(&w), "fleet/x, started at abcdef1");
    w.owned = true;
    assert_eq!(workspace_tip(&w), "fleet/x · created in this task");
}

/// While a restart blocks Add repo, repos still queued in the panel stay queued and the window does not spin
/// redrawing for them: the restart's result wakes it. `run()` panics on a repaint asked for every frame.
#[test]
fn a_blocked_queued_add_waits_without_spinning() {
    let mut st = state("$ ready");
    st.queue.extend(["core".to_string(), "portal".to_string()]);
    let mut h = Harness::builder()
        .with_size(Vec2::new(700.0, 600.0))
        .build_ui_state(
            |ui, (st, actions): &mut (PreviewState, Vec<PreviewAction>)| {
                let panel = RepoPanel {
                    workspaces: &[],
                    remembered: &[],
                    request: None,
                    add_blocked: Some("The daemon is restarting"),
                    stopped: false,
                };
                let a = preview_pane(ui, st, None, Some(panel), None, None);
                if a != PreviewAction::None {
                    actions.push(a);
                }
            },
            (st, vec![]),
        );
    h.run();
    assert!(
        h.state()
            .1
            .iter()
            .all(|a| !matches!(a, PreviewAction::AddRepo(_))),
        "{:?}",
        h.state().1
    );
    assert_eq!(h.state().0.queue.len(), 2);
}

// ---- The details block ----

fn details_agent(status: Status) -> Agent {
    Agent {
        id: AgentId("a1".into()),
        goal: "fix the tests".into(),
        workdir: "/w".into(),
        workspaces: vec![workspace("core", false), workspace("portal", false)],
        tmux_session: "fleet-a1".into(),
        status,
        activity: "wants to run Bash".into(),
        status_since: 1000,
        has_changes: true,
        paused: true,
        owner: "antonio".into(),
        blocked_reason: None,
        repo_request: None,
        stop_cause: None,
        disk_bytes: None,
        persona: "p".into(),
        product: "p".into(),
        kind: fleet_api::AgentKind::Solo,
        task: None,
        role: None,
        step: None,
        model: None,
        attach: None,
    }
}

/// The panel for `agent`, its details drawn from `schema`, with tooltips at once.
fn details_harness(schema: UiSchema, agent: Agent) -> Harness<'static, PreviewState> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(700.0, 600.0))
        .build_ui_state(
            move |ui, st: &mut PreviewState| {
                let panel = RepoPanel {
                    workspaces: &agent.workspaces,
                    remembered: &[],
                    request: None,
                    add_blocked: None,
                    stopped: agent.status == Status::Stopped,
                };
                let details = AgentDetails {
                    schema: &schema,
                    agent: &agent,
                    now: 1300,
                };
                preview_pane(ui, st, None, Some(panel), Some(details), None);
            },
            state("$ ready"),
        );
    h.run();
    h
}

#[test]
fn the_details_show_the_status_every_badge_and_the_branch() {
    let mut a = details_agent(Status::NeedsYou);
    a.blocked_reason = Some(BlockedReason::Permission);
    let h = details_harness(default_ui_schema(), a);
    for label in [
        "Needs you",
        "5m",
        "Permission needed",
        "Has changes",
        "Paused",
        "Branch",
        "fleet/idea",
        "core",
        "portal",
    ] {
        h.get_by_label(label);
    }
}

#[test]
fn a_stopped_agents_details_show_its_cause_and_size() {
    let mut a = details_agent(Status::Stopped);
    a.stop_cause = Some(StopCause::Crashed);
    a.disk_bytes = Some(7_600_000_000);
    let h = details_harness(default_ui_schema(), a);
    for label in ["Stopped", "Crashed", "7.6 GB"] {
        h.get_by_label(label);
    }
}

#[test]
fn a_stopped_agents_details_say_stopped_once() {
    let mut a = details_agent(Status::Stopped);
    a.stop_cause = Some(StopCause::Stopped);
    let h = details_harness(default_ui_schema(), a);
    assert_eq!(h.query_all_by_label("Stopped").count(), 1);
}

#[test]
fn the_details_show_a_field_this_client_does_not_know() {
    let mut schema = default_ui_schema();
    schema.card_fields.push(FieldDef {
        key: "owner".into(),
        label: "Owner".into(),
        kind: FieldKind::Mono,
        show_when: fleet_api::Condition::Always,
        value_labels: Default::default(),
        value_tones: Default::default(),
    });
    let h = details_harness(schema, details_agent(Status::Working));
    h.get_by_label("Owner");
    h.get_by_label("antonio");
}

#[test]
fn the_details_sit_between_the_title_and_the_tabs() {
    let h = details_harness(default_ui_schema(), details_agent(Status::Working));
    let title = h.get_by_label("fix the tests").rect();
    let status = h.get_by_label("Working").rect();
    let tabs = h.get_by_role_and_label(Role::Button, "Terminal").rect();
    assert!(title.bottom() <= status.top() && status.bottom() <= tabs.top());
}

// ---- orchestrated tasks: breakdown, gate, stop and restart ----

mod fleet {
    use super::*;
    use fleet_api::{
        Approval, BreakdownVersion, DecideBreakdown, FleetView, GateRun, GateStep, Snapshot, Step,
        StepState,
    };
    use fleet_client::http::FleetClient;
    use fleet_client::stream::StreamMsg;
    use fleet_gui::app::{AppResult, FleetApp, FleetCall};
    use fleet_gui::components::{breakdown_panel, BreakdownAction};

    fn agent(id: &str, goal: &str, status: Status) -> Agent {
        Agent {
            id: AgentId(id.into()),
            goal: goal.into(),
            workdir: "/w".into(),
            workspaces: vec![],
            tmux_session: format!("fleet-{id}"),
            status,
            activity: "running Bash".into(),
            status_since: 1000,
            has_changes: false,
            paused: false,
            owner: "antonio".into(),
            blocked_reason: None,
            repo_request: None,
            stop_cause: None,
            disk_bytes: None,
            persona: "p".into(),
            product: "p".into(),
            kind: fleet_api::AgentKind::Solo,
            task: None,
            role: None,
            step: None,
            model: None,
            attach: None,
        }
    }

    fn orchestrator(status: Status) -> Agent {
        let mut a = agent("o", "Fix login", status);
        a.kind = fleet_api::AgentKind::Orchestrator;
        a.role = Some("bug-fix".into());
        a
    }

    fn specialist(id: &str, goal: &str, status: Status) -> Agent {
        let mut a = agent(id, goal, status);
        a.kind = fleet_api::AgentKind::Specialist;
        a.task = Some(AgentId("o".into()));
        a.role = Some("rust".into());
        a
    }

    fn step(n: u32, title: &str, state: StepState, agent: Option<&str>) -> Step {
        Step {
            n,
            title: title.into(),
            specialist: "rust".into(),
            brief: "do it".into(),
            depends_on: if n > 1 { vec![n - 1] } else { vec![] },
            state,
            agent: agent.map(|a| AgentId(a.into())),
            branch: None,
            report: None,
        }
    }

    fn version(version: u32, approval: Approval, steps: Vec<Step>) -> BreakdownVersion {
        BreakdownVersion {
            version,
            steps,
            approval,
            comment: None,
            proposed_at: 1000,
            decided_at: None,
        }
    }

    fn view(breakdowns: Vec<BreakdownVersion>, gate_runs: Vec<GateRun>) -> FleetView {
        FleetView {
            task: AgentId("o".into()),
            breakdowns,
            gate_runs,
        }
    }

    fn proposed() -> FleetView {
        view(
            vec![
                version(
                    1,
                    Approval::ChangesRequested,
                    vec![step(1, "old plan", StepState::Pending, None)],
                ),
                version(
                    2,
                    Approval::Proposed,
                    vec![
                        step(1, "fix redirect", StepState::Running, Some("f1")),
                        step(2, "add a test", StepState::Pending, None),
                    ],
                ),
            ],
            vec![],
        )
    }

    fn gate_step(name: &str, command: &str, exit_code: Option<i32>, tail: &str) -> GateStep {
        GateStep {
            name: name.into(),
            command: command.into(),
            because: vec![],
            exit_code,
            duration_secs: 42,
            output_tail: tail.into(),
        }
    }

    /// A run of one step, `test`.
    fn gate(exit_code: Option<i32>, tail: &str, pr: Option<&str>) -> GateRun {
        GateRun {
            at: 1200,
            base: "main".into(),
            steps: vec![gate_step("test", "cargo test", exit_code, tail)],
            unmatched: vec![],
            pr_url: pr.map(String::from),
        }
    }

    struct Panel {
        view: FleetView,
        agents: Vec<Agent>,
        pending: bool,
        draft: String,
        actions: Vec<BreakdownAction>,
    }

    fn panel(view: FleetView, pending: bool) -> Harness<'static, Panel> {
        let mut h = Harness::builder()
            .with_size(Vec2::new(700.0, 700.0))
            .build_ui_state(
                |ui, p: &mut Panel| {
                    let agents: Vec<&Agent> = p.agents.iter().collect();
                    let a = breakdown_panel(ui, &p.view, &agents, p.pending, &mut p.draft);
                    if a != BreakdownAction::None {
                        p.actions.push(a);
                    }
                },
                Panel {
                    view,
                    agents: vec![specialist("f1", "fix redirect", Status::Working)],
                    pending,
                    draft: String::new(),
                    actions: vec![],
                },
            );
        h.run();
        h
    }

    fn app(agents: Vec<Agent>) -> Harness<'static, FleetApp> {
        app_with(agents, None)
    }

    /// `log`: the app's log reaches it, so the footer shows its notices.
    fn app_with(
        agents: Vec<Agent>,
        log: Option<fleet_log::LogHandle>,
    ) -> Harness<'static, FleetApp> {
        let mut app = FleetApp::detached(
            FleetClient::new("http://127.0.0.1:1"),
            egui::Context::default(),
        )
        .without_calls();
        if let Some(log) = log {
            app = app.with_log(log);
        }
        app.set_clock(Box::new(|| 1300));
        app.handle_result(AppResult::Schema(Ok(default_ui_schema())));
        app.handle_result(AppResult::Repos(Ok(vec![])));
        app.handle_stream(StreamMsg::Snapshot(Snapshot { seq: 1, agents }));
        let mut h = Harness::builder()
            .with_max_steps(60)
            .with_size(Vec2::new(1400.0, 800.0))
            .build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), app);
        // The app's fonts carry "↳"; the harness's stock ones do not.
        fleet_gui::theme::apply(&h.ctx);
        h.run();
        h
    }

    fn click(h: &mut Harness<'static, FleetApp>, label: &str) {
        h.get_by_role_and_label(Role::Button, label).click();
        h.run();
    }

    /// Opens the side panel the way the user does: a click on the card or fleet row labelled `label`.
    fn open(h: &mut Harness<'static, FleetApp>, label: &str) {
        h.get_by_label(label).click();
        h.run();
    }

    #[test]
    fn breakdown_panel_shows_the_latest_version_and_its_history() {
        let mut h = panel(proposed(), true);
        for label in ["#", "Step", "Specialist", "Depends on", "State", "Agent"] {
            h.get_by_label(label);
        }
        h.get_by_label("v2 · proposed");
        h.get_by_label("add a test");
        assert!(
            h.query_by_label("old plan").is_none(),
            "earlier versions wait behind History"
        );
        h.get_by_label("History").click();
        h.run();
        h.get_by_label("v1 · changes requested");
        h.get_by_label("old plan");
        // The agent cell names the agent and opens it.
        h.get_by_role_and_label(Role::Link, "fix redirect").click();
        h.run();
        assert_eq!(h.state().actions, [BreakdownAction::Open("f1".into())]);
    }

    #[test]
    fn breakdown_approve_and_request_changes() {
        let mut h = panel(proposed(), true);
        h.get_by_role_and_label(Role::Button, "Approve").click();
        h.run();
        assert_eq!(h.state().actions, [BreakdownAction::Approve]);
        let request = |h: &Harness<'static, Panel>| {
            h.get_by_role_and_label(Role::Button, "Request changes")
                .accesskit_node()
                .is_disabled()
        };
        assert!(request(&h), "Request changes needs a comment");
        h.get_by_role_and_label(Role::MultilineTextInput, "Or ask for changes")
            .click();
        h.run();
        h.get_by_role_and_label(Role::MultilineTextInput, "Or ask for changes")
            .type_text("split step 2");
        h.run();
        assert!(!request(&h));
        h.get_by_role_and_label(Role::Button, "Request changes")
            .click();
        h.run();
        assert_eq!(
            h.state().actions.last(),
            Some(&BreakdownAction::RequestChanges("split step 2".into()))
        );

        let h = panel(proposed(), false);
        assert!(h.query_by_role_and_label(Role::Button, "Approve").is_none());
        assert!(h
            .query_by_role_and_label(Role::Button, "Request changes")
            .is_none());
        assert_eq!(h.query_all_by_role(Role::MultilineTextInput).count(), 0);

        // In the app, each decision goes to the daemon for the task.
        let mut lead = orchestrator(Status::NeedsYou);
        lead.blocked_reason = Some(BlockedReason::Breakdown);
        let mut h = app(vec![
            lead,
            specialist("f1", "fix redirect", Status::Working),
        ]);
        open(&mut h, "Fix login");
        h.state_mut()
            .handle_result(AppResult::FleetView("o".into(), Ok(proposed())));
        h.run();
        click(&mut h, "Approve");
        let decide = |approve: bool, comment: Option<&str>| {
            FleetCall::Decide(
                "o".into(),
                DecideBreakdown {
                    approve,
                    comment: comment.map(String::from),
                    // The version on screen, so a newer proposal is never decided unseen.
                    version: Some(2),
                },
            )
        };
        assert!(h.state().fleet_calls().contains(&decide(true, None)));
        assert!(
            h.get_by_role_and_label(Role::Button, "Approve")
                .accesskit_node()
                .is_disabled(),
            "one decision at a time"
        );
        h.state_mut()
            .handle_result(AppResult::BreakdownDecided("o".into(), Ok(())));
        h.run();
        let approve_enabled = |h: &Harness<'static, FleetApp>| {
            !h.get_by_role_and_label(Role::Button, "Approve")
                .accesskit_node()
                .is_disabled()
        };
        // The view on screen still shows v2 as proposed: a decided version cannot be decided again from it.
        assert!(!approve_enabled(&h), "decided, and no new view yet");
        let calls = h.state().fleet_calls().len();
        h.get_by_role_and_label(Role::Button, "Approve").click();
        h.run();
        assert_eq!(h.state().fleet_calls().len(), calls);
        // The agent links stay usable while the decision controls wait.
        h.get_by_role_and_label(Role::Link, "fix redirect");
        assert!(!h
            .get_by_role_and_label(Role::Link, "fix redirect")
            .accesskit_node()
            .is_disabled());
        h.state_mut()
            .handle_result(AppResult::FleetView("o".into(), Ok(proposed())));
        h.run();
        assert!(approve_enabled(&h), "a new view brings the controls back");
        h.get_by_role_and_label(Role::MultilineTextInput, "Or ask for changes")
            .click();
        h.run();
        h.get_by_role_and_label(Role::MultilineTextInput, "Or ask for changes")
            .type_text("split step 2");
        h.run();
        click(&mut h, "Request changes");
        assert!(h
            .state()
            .fleet_calls()
            .contains(&decide(false, Some("split step 2"))));
    }

    #[test]
    fn breakdown_tab_first_when_pending() {
        let mut lead = orchestrator(Status::NeedsYou);
        lead.blocked_reason = Some(BlockedReason::Breakdown);
        let mut h = app(vec![lead]);
        open(&mut h, "Fix login");
        assert!(h
            .state()
            .fleet_calls()
            .contains(&FleetCall::View("o".into())));
        let x = |h: &Harness<'static, FleetApp>, l: &str| {
            h.get_by_role_and_label(Role::Button, l).rect().center().x
        };
        assert!(x(&h, "Breakdown") < x(&h, "Terminal"));
        h.state_mut()
            .handle_result(AppResult::FleetView("o".into(), Ok(proposed())));
        h.run();
        h.get_by_label("v2 · proposed");

        // Not waiting on a breakdown: the tab is last and the terminal shows.
        let mut h = app(vec![orchestrator(Status::Working)]);
        open(&mut h, "Fix login");
        h.state_mut()
            .handle_result(AppResult::FleetView("o".into(), Ok(proposed())));
        h.run();
        assert!(x(&h, "Breakdown") > x(&h, "Diff"));
        assert!(h.query_by_label("v2 · proposed").is_none());
        click(&mut h, "Breakdown");
        h.get_by_label("v2 · proposed");

        // A change of any of the task's agents fetches the view again.
        let before = h.state().fleet_calls().len();
        h.state_mut()
            .handle_stream(StreamMsg::Event(Box::new(fleet_api::Event {
                seq: 2,
                kind: fleet_api::EventKind::StatusChanged {
                    id: AgentId("o".into()),
                    status: Status::Review,
                    activity: "done".into(),
                    since: 1200,
                    has_changes: false,
                    paused: false,
                    blocked_reason: None,
                    repo_request: None,
                    stop_cause: None,
                    disk_bytes: None,
                },
            })));
        h.run();
        assert!(h.state().fleet_calls()[before..].contains(&FleetCall::View("o".into())));
    }

    #[test]
    fn gate_failure_shows_tail() {
        let mut v = proposed();
        v.gate_runs = vec![
            gate(Some(0), "ok", Some("https://example.invalid/pr/1")),
            gate(Some(101), "test login::redirect ... FAILED", None),
        ];
        let mut h = panel(v, false);
        h.get_by_label("test failed (exit 101)");
        assert!(
            h.query_by_label_contains("FAILED").is_none(),
            "the tail is folded"
        );
        assert!(
            h.query_by_role(Role::Link).is_none_or(|l| l
                .accesskit_node()
                .label()
                .is_none_or(|t| !t.contains("pr/1"))),
            "only the last run's PR counts"
        );
        h.get_by_label("Output").click();
        h.run();
        h.get_by_label_contains("test login::redirect ... FAILED");

        let mut v = proposed();
        v.gate_runs = vec![gate(Some(0), "ok", Some("https://example.invalid/pr/2"))];
        let h = panel(v, false);
        h.get_by_label("Tests passed");
        h.get_by_role_and_label(Role::Link, "https://example.invalid/pr/2");

        let mut v = proposed();
        v.gate_runs = vec![gate(None, "", None)];
        panel(v, false).get_by_label("test timed out");
    }

    /// A long log line scrolls inside the Output area instead of widening the panel (#193).
    #[test]
    fn a_long_output_line_keeps_the_panel_width() {
        let mut v = proposed();
        let line = format!(
            "thread 'git::tests' panicked at {}",
            "crates/fleet-daemon/src/git.rs:".repeat(14)
        );
        v.gate_runs = vec![gate(Some(101), &line, None)];
        let mut h = panel(v, false);
        h.get_by_label("Output").click();
        h.run();
        h.get_by_label_contains("panicked at");
        h.fit_contents();
        let width = h.ctx.content_rect().width();
        assert!(width <= 700.5, "the panel's content grew to {width} px");
    }

    #[test]
    fn gate_lists_each_step() {
        let mut v = proposed();
        let mut fmt = gate_step("fmt", "cargo fmt --all --check", Some(0), "fmt says fine");
        fmt.duration_secs = 3;
        let gui = gate_step(
            "gui",
            "cargo build -p fleet-gui\ncargo test -p fleet-gui",
            Some(101),
            "gui test FAILED",
        );
        v.gate_runs = vec![GateRun {
            at: 1300,
            base: "main".into(),
            steps: vec![fmt, gui],
            unmatched: vec!["build.rs".into()],
            pr_url: None,
        }];
        let mut h = panel(v, false);
        h.get_by_label("gui failed (exit 101)");
        h.get_by_label("fmt · cargo fmt --all --check · 3s · ok");
        h.get_by_label("gui · cargo build -p fleet-gui cargo test -p fleet-gui · 42s · exit 101");
        h.get_by_label("Every step ran: no step covers build.rs");
        assert!(
            h.query_by_label_contains("FAILED").is_none(),
            "the tail is folded"
        );
        h.get_by_label("Output").click();
        h.run();
        h.get_by_label_contains("gui test FAILED");
        assert!(
            h.query_by_label_contains("fmt says fine").is_none(),
            "only the failed step's output"
        );
    }

    #[test]
    fn specialist_panel_links_to_task() {
        let mut h = app(vec![
            orchestrator(Status::Working),
            specialist("f1", "fix redirect", Status::Working),
        ]);
        open(&mut h, "rust fix redirect, working");
        assert!(h
            .query_by_role_and_label(Role::Button, "Breakdown")
            .is_none());
        h.get_by_label("↳ Fix login").click();
        h.run();
        assert_eq!(h.state().preview_agent(), Some("o"));
        h.get_by_role_and_label(Role::Button, "Breakdown");
    }

    #[test]
    fn panel_shows_the_model() {
        let mut lead = orchestrator(Status::Working);
        lead.model = Some("opus".into());
        let mut h = app(vec![
            lead,
            specialist("f1", "fix redirect", Status::Working),
        ]);
        open(&mut h, "Fix login");
        h.get_by_label("Model: opus");
        open(&mut h, "rust fix redirect, working");
        h.get_by_label("Model: persona default");
        assert!(h.query_by_label("Model: opus").is_none());
    }

    #[test]
    fn task_stop_confirms_with_count() {
        let (_guard, log) = common::log::test_log();
        let mut h = app_with(
            vec![
                orchestrator(Status::Working),
                specialist("f1", "fix redirect", Status::Working),
                specialist("f2", "add a test", Status::Queued),
                specialist("f3", "done step", Status::Stopped),
            ],
            Some(log),
        );
        click(&mut h, "Stop");
        h.get_by_label("Stop the orchestrator and 2 fleet agents?");
        assert!(h.state().fleet_calls().is_empty());
        click(&mut h, "Cancel");
        assert!(h
            .query_by_label("Stop the orchestrator and 2 fleet agents?")
            .is_none());
        click(&mut h, "Stop");
        click(&mut h, "Stop all");
        assert_eq!(h.state().fleet_calls(), [FleetCall::StopTask("o".into())]);
        h.state_mut()
            .handle_result(AppResult::TaskStopped("o".into(), Ok(3)));
        h.run();
        h.get_by_label_contains("Fix login: 3 agents stopped");
    }

    #[test]
    fn restart_offered_once_the_lead_stopped() {
        let h = app(vec![
            orchestrator(Status::Working),
            specialist("f1", "fix redirect", Status::Working),
        ]);
        assert!(h
            .query_by_role_and_label(Role::Button, "Restart orchestrator")
            .is_none());

        // Everything stopped by Stop task: the task is stopped, and can still be run again.
        let mut lead = orchestrator(Status::Stopped);
        lead.stop_cause = Some(fleet_api::StopCause::Stopped);
        let mut fleet = specialist("f1", "fix redirect", Status::Stopped);
        fleet.stop_cause = Some(fleet_api::StopCause::Stopped);
        let h = app(vec![lead, fleet]);
        assert!(h
            .query_by_role_and_label(Role::Button, "Restart orchestrator")
            .is_some());

        let mut h = app(vec![
            orchestrator(Status::Stopped),
            specialist("f1", "fix redirect", Status::Working),
        ]);
        click(&mut h, "Restart orchestrator");
        assert_eq!(h.state().fleet_calls(), [FleetCall::Restart("o".into())]);
        let mut fresh = orchestrator(Status::Queued);
        fresh.id = AgentId("o2".into());
        fresh.task = Some(AgentId("o".into()));
        h.state_mut()
            .handle_result(AppResult::OrchestratorRestarted("o".into(), Ok(fresh)));
        h.run();
    }

    #[test]
    fn the_board_marks_the_pr_from_the_fleet_view() {
        let mut h = app(vec![
            orchestrator(Status::Review),
            specialist("f1", "fix redirect", Status::Review),
        ]);
        let pr_marks = |h: &Harness<'static, FleetApp>| h.query_all_by_label("PR open").count();
        assert_eq!(pr_marks(&h), 0);
        let mut v = proposed();
        v.gate_runs = vec![gate(Some(0), "ok", Some("https://example.invalid/pr/3"))];
        h.state_mut()
            .handle_result(AppResult::FleetView("o".into(), Ok(v)));
        h.run();
        assert!(
            pr_marks(&h) > 0,
            "the fleet rows show the PR the view knows"
        );
    }
}

#[test]
fn the_repo_filter_reports_its_value_as_written() {
    let h = Harness::new_ui_state(
        |ui, sel: &mut Vec<String>| {
            repo_filter(ui, &counted(), sel);
        },
        Vec::<String>::new(),
    );
    let value = h
        .get_by_role_and_label(Role::ComboBox, "Repo filter")
        .accesskit_node()
        .value();
    assert_eq!(value.as_deref(), Some("Repo All repos"));
}
