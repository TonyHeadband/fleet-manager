use eframe::egui::{self, accesskit::Role, Vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use fleet_api::{DiffReport, WorkspaceDiff};
use fleet_gui::components::{preview_pane, repo_filter, spawn_form, DiffView, PreviewAction, PreviewState, PreviewTab, SpawnState};
use std::time::Instant;

fn repos() -> Vec<String> {
    vec!["core".into(), "portal".into(), "infra".into()]
}

// ---- repo filter ----

#[test]
fn repo_filter_toggles_repos_in_click_order() {
    let mut h = Harness::new_ui_state(|ui, sel: &mut Vec<String>| {
        repo_filter(ui, &repos(), sel);
    }, Vec::<String>::new());
    h.get_by_label("core").click();
    h.run();
    assert_eq!(h.state(), &["core"]);
    h.get_by_label("portal").click();
    h.run();
    assert_eq!(h.state(), &["core", "portal"]);
    h.get_by_label("core").click();
    h.run();
    assert_eq!(h.state(), &["portal"]);
    h.get_by_label("portal").click();
    h.run();
    assert!(h.state().is_empty());
}

#[test]
fn repo_filter_reports_a_change_once_per_click() {
    struct S {
        sel: Vec<String>,
        changes: usize,
    }
    let mut h = Harness::new_ui_state(|ui, s: &mut S| {
        if repo_filter(ui, &repos(), &mut s.sel) {
            s.changes += 1;
        }
    }, S { sel: vec![], changes: 0 });
    h.run();
    assert_eq!(h.state().changes, 0);
    h.get_by_label("infra").click();
    h.run();
    assert_eq!(h.state().changes, 1);
}

// ---- spawn form ----

struct Spawn {
    st: SpawnState,
    started: Vec<(String, String)>,
}

fn spawn_harness(error: Option<&str>) -> Harness<'static, Spawn> {
    let st = SpawnState { error: error.map(String::from), ..SpawnState::default() };
    Harness::builder().with_size(Vec2::new(900.0, 300.0)).build_ui_state(
        |ui, s: &mut Spawn| {
            if let Some(req) = spawn_form(ui, &repos(), &mut s.st) {
                s.started.push(req);
            }
        },
        Spawn { st, started: vec![] },
    )
}

#[test]
fn spawn_is_disabled_until_the_prompt_has_text() {
    let mut h = spawn_harness(None);
    h.get_by_role_and_label(Role::Button, "Start agent").click();
    h.run();
    assert!(h.state().started.is_empty());
    let prompt = h.get_by_role(Role::MultilineTextInput);
    prompt.click();
    prompt.type_text("   ");
    h.run();
    h.get_by_role_and_label(Role::Button, "Start agent").click();
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
    h.get_by_role_and_label(Role::Button, "Start agent").click();
    h.run();
    assert_eq!(h.state().started, [("core".to_string(), "fix $(x) \"q\" {a}\nsecond line é".to_string())]);
}

#[test]
fn spawn_uses_the_chosen_repo() {
    let mut h = spawn_harness(None);
    h.get_by_role(Role::ComboBox).click();
    h.run();
    h.get_by_label("portal").click();
    h.run();
    let prompt = h.get_by_role(Role::MultilineTextInput);
    prompt.click();
    prompt.type_text("do it");
    h.run();
    h.get_by_role_and_label(Role::Button, "Start agent").click();
    h.run();
    assert_eq!(h.state().started, [("portal".to_string(), "do it".to_string())]);
}

#[test]
fn spawn_error_is_shown_and_the_prompt_kept() {
    let mut h = spawn_harness(Some("unknown repo \"x\""));
    h.get_by_label("unknown repo \"x\"");
    let prompt = h.get_by_role(Role::MultilineTextInput);
    prompt.click();
    prompt.type_text("do it");
    h.run();
    assert_eq!(h.get_by_role(Role::MultilineTextInput).value().unwrap_or_default(), "do it");
}

#[test]
fn a_busy_spawn_cannot_be_sent_twice() {
    let mut h = spawn_harness(None);
    h.state_mut().st.busy = true;
    h.state_mut().st.prompt = "go".into();
    h.run();
    h.get_by_role_and_label(Role::Button, "Start agent").click();
    h.run();
    assert!(h.state().started.is_empty());
}

// ---- preview ----

struct Preview {
    st: PreviewState,
    actions: Vec<PreviewAction>,
}

fn preview_harness(st: PreviewState) -> Harness<'static, Preview> {
    Harness::builder().with_size(Vec2::new(700.0, 500.0)).build_ui_state(
        |ui, p: &mut Preview| {
            let a = preview_pane(ui, &mut p.st);
            if a != PreviewAction::None {
                p.actions.push(a);
            }
        },
        Preview { st, actions: vec![] },
    )
}

fn state(text: &str) -> PreviewState {
    PreviewState { agent_id: "a1".into(), title: "fix the tests".into(), text: text.into(), ..PreviewState::default() }
}

fn report() -> DiffReport {
    DiffReport {
        workspaces: vec![
            WorkspaceDiff { repo: "core".into(), diff: "+++ b/new.txt\n+hello\n-gone\n".into(), truncated: false },
            WorkspaceDiff { repo: "portal".into(), diff: String::new(), truncated: false },
            WorkspaceDiff { repo: "big".into(), diff: "+cut here\n".into(), truncated: true },
        ],
    }
}

#[test]
fn preview_terminal_tab_is_read_only() {
    let h = preview_harness(state("$ ready\n> hello"));
    h.get_by_label("fix the tests");
    h.get_by_label_contains("$ ready");
    assert_eq!(h.query_all_by_role(Role::TextInput).count() + h.query_all_by_role(Role::MultilineTextInput).count(), 0);
}

#[test]
fn pane_text_no_font_can_draw_is_shown_as_boxes_not_a_panic() {
    // Terminal output is full of characters the bundled fonts lack (prompts, box drawing, CJK, emoji). The test
    // harness panics on a missing glyph unless the app installs its policy, which `theme::apply` does.
    let st = state("❯ ● ⎿ 日本語 🙂 ╭──╮\nplain line");
    let mut h = Harness::builder().with_size(Vec2::new(700.0, 500.0)).build_ui_state(
        |ui, p: &mut Preview| {
            fleet_gui::theme::apply(ui.ctx());
            preview_pane(ui, &mut p.st);
        },
        Preview { st, actions: vec![] },
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
fn close_button_returns_close() {
    let mut h = preview_harness(state("x"));
    h.get_by_label("Close").click();
    h.run();
    assert_eq!(h.state().actions, [PreviewAction::Close]);
}

#[test]
fn diff_tab_requests_a_load_and_shows_each_workspace() {
    let mut h = preview_harness(state("x"));
    assert_eq!(h.get_by_label("Terminal").accesskit_node_toggled(), Some(true));
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
    assert_eq!(h.state().actions, [PreviewAction::LoadDiff, PreviewAction::LoadDiff]);
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
    assert!(matches!(h.state().st.diff, DiffView::Loaded(_)), "reopening the tab must not discard it");
    assert_eq!(h.state().actions.len(), 1, "no second load for a diff already loaded");
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
    assert!(tops.iter().all(|t| *t >= 0.0), "rows were drawn above the pane: {tops:?}");
    let heights: Vec<f32> = rows.iter().map(|r| r.rect().height()).collect();
    let (lo, hi) = heights.iter().fold((f32::MAX, 0f32), |(l, h), x| (l.min(*x), h.max(*x)));
    assert!(hi < 30.0 && hi - lo < 1.0, "rows differ in height, so they wrapped: {heights:?}");
    let mut sorted = tops.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!(sorted.windows(2).all(|w| w[1] - w[0] > 5.0), "rows overlap: {sorted:?}");
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
    h.state_mut().st.diff = DiffView::Loaded(DiffReport { workspaces: vec![WorkspaceDiff { repo: "core".into(), diff: big.clone(), truncated: false }] });
    let t = Instant::now();
    h.run();
    let diff = t.elapsed();
    // A frame lays out only the visible rows; generous bound because tests run unoptimised.
    assert!(pane.as_millis() < 750 && diff.as_millis() < 750, "pane {pane:?}, diff {diff:?}");
    let _ = egui::Id::NULL;
}

trait Toggled {
    fn accesskit_node_toggled(&self) -> Option<bool>;
}

impl Toggled for egui_kittest::Node<'_> {
    fn accesskit_node_toggled(&self) -> Option<bool> {
        use egui_kittest::kittest::NodeT;
        self.accesskit_node().toggled().map(|t| t == egui::accesskit::Toggled::True)
    }
}
