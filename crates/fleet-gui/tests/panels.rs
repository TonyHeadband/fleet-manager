use eframe::egui::{self, accesskit::Role, Key, Modifiers, Vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use fleet_api::{DiffReport, WorkspaceDiff};
use fleet_gui::components::{filter_summary, new_agent_dialog, preview_pane, reply_box, repo_filter, DiffView, PreviewAction, PreviewState, PreviewTab, SpawnState};
use std::time::Instant;

fn repos() -> Vec<String> {
    vec!["core".into(), "portal".into(), "infra".into()]
}

fn counted() -> Vec<(String, usize)> {
    repos().into_iter().enumerate().map(|(i, r)| (r, i + 1)).collect()
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
    let mut h = Harness::new_ui_state(|ui, sel: &mut Vec<String>| {
        repo_filter(ui, &counted(), sel);
    }, Vec::<String>::new());
    h.get_by_role(Role::ComboBox).click();
    h.run();
    for (repo, expected) in [("core", &["core"][..]), ("portal", &["core", "portal"]), ("core", &["portal"]), ("portal", &[])] {
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
    let mut h = Harness::new_ui_state(|ui, s: &mut S| {
        if repo_filter(ui, &counted(), &mut s.sel) {
            s.changes += 1;
        }
    }, S { sel: names(&["core"]), changes: 0 });
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
    let mut h = Harness::new_ui_state(|ui, s: &mut S| {
        if repo_filter(ui, &counted(), &mut s.sel) {
            s.changes += 1;
        }
    }, S { sel: vec![], changes: 0 });
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
    started: Vec<(String, String)>,
}

fn spawn_harness(error: Option<&str>) -> Harness<'static, Spawn> {
    let st = SpawnState { open: true, error: error.map(String::from), ..SpawnState::default() };
    spawn_harness_with(st)
}

fn spawn_harness_with(st: SpawnState) -> Harness<'static, Spawn> {
    Harness::builder().with_max_steps(60).with_size(Vec2::new(900.0, 500.0)).build_ui_state(
        |ui, s: &mut Spawn| {
            if let Some(req) = new_agent_dialog(ui.ctx(), &repos(), &mut s.st) {
                s.started.push(req);
            }
        },
        Spawn { st, started: vec![] },
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
    h.output().shapes.iter().for_each(|c| walk(&c.shape, &mut rects));
    let title = h.get_by_label("New agent").rect().center();
    let surface = fleet_gui::theme::tokens(egui::Visuals::default().dark_mode).surface;
    let (frame, _, radius) = rects
        .into_iter()
        .filter(|(r, fill, _)| *fill == surface && r.contains(title))
        .min_by(|a, b| a.0.area().total_cmp(&b.0.area()))
        .expect("the dialog's frame");
    assert!((frame.width() - 440.0).abs() <= 2.0, "the dialog is {}px wide", frame.width());
    assert_eq!(radius, fleet_gui::theme::radius::DIALOG);
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
    assert_eq!(h.state().started, [("core".to_string(), "do it".to_string())]);
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
    let mut h = spawn_harness_with(SpawnState::default());
    h.run();
    assert!(h.query_by_label("New agent").is_none());
}

// ---- reply box ----

struct Reply {
    text: String,
    sent: Vec<String>,
}

fn reply_harness(text: &str) -> Harness<'static, Reply> {
    Harness::builder().with_size(Vec2::new(500.0, 300.0)).build_ui_state(
        |ui, r: &mut Reply| {
            if let Some(t) = reply_box(ui, "Answer", &mut r.text, false) {
                r.sent.push(t);
            }
        },
        Reply { text: text.into(), sent: vec![] },
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
    assert_eq!(h.state().text, "$(x) \"q\" {a} é -n", "the caller clears the text, not the box");
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
    Harness::builder().with_size(Vec2::new(700.0, 500.0)).build_ui_state(
        |ui, p: &mut Preview| {
            let a = preview_pane(ui, &mut p.st, None);
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
            preview_pane(ui, &mut p.st, None);
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
fn session_ended_is_inside_the_panel_above_the_text() {
    let mut st = state("bye");
    st.closed = true;
    let h = preview_harness(st);
    let note = h.get_by_label("Session ended").rect();
    // The harness hosts the pane inside an 8 px outer margin, so its visible bottom is at 492.
    assert!(note.top() >= 0.0 && note.bottom() <= 492.0, "the note is below the pane's visible bottom: {note:?}");
    let text = h.get_by_label_contains("bye").rect();
    assert!(note.bottom() <= text.top(), "the note sits above the terminal text: {note:?} vs {text:?}");
}

#[test]
fn a_pane_padded_with_blank_rows_still_shows_its_text() {
    // tmux captures the whole pane, so short output arrives followed by its empty rows; sticking to the bottom of
    // those would scroll the real text out of sight.
    let text = format!("● Write(notes.txt)\n  done{}", "\n".repeat(80));
    let h = preview_harness(state(&text));
    let line = h.get_by_label_contains("Write(notes.txt)").rect();
    assert!(line.top() >= 0.0 && line.bottom() <= 492.0, "the text is scrolled out of the pane: {line:?}");
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
