mod common;

use eframe::egui::{
    self,
    accesskit::{Role, Toggled},
    Vec2,
};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use fleet_api::{default_ui_schema, Agent, AgentId, Snapshot, Status, Workspace};
use fleet_client::config::ThemeChoice;
use fleet_client::http::FleetClient;
use fleet_client::stream::StreamMsg;
use fleet_gui::app::{AppResult, FleetApp};
use fleet_gui::theme::{self, layout};
use fleet_gui::views::View;

/// `gui.toml` once every background write asked for so far has landed.
fn on_disk(path: &std::path::Path) -> fleet_client::config::GuiSettings {
    fleet_client::config::SettingsWriter::shared().flush();
    fleet_client::config::GuiSettings::load(path)
}

fn agent(id: &str, goal: &str, status: Status) -> Agent {
    Agent {
        id: AgentId(id.into()),
        goal: goal.into(),
        title: None,
        workdir: "/w".into(),
        workspaces: vec![Workspace {
            repo: "core".into(),
            branch: "fleet/x".into(),
            worktree: "/w/core".into(),
            base: "abc".into(),
            from_branch: None,
            has_changes: false,
            owned: false,
            changed: false,
        }],
        session: format!("fleet-{id}"),
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
        mode: None,
        effort: None,
    }
}

fn app() -> FleetApp {
    let mut app = FleetApp::detached(
        FleetClient::new("http://127.0.0.1:1"),
        egui::Context::default(),
    );
    app.set_clock(Box::new(|| 1300));
    app.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    app.handle_result(AppResult::Repos(Ok(vec![])));
    app
}

fn with_agents(mut app: FleetApp, agents: Vec<Agent>) -> FleetApp {
    app.handle_stream(StreamMsg::Snapshot(Snapshot { seq: 1, agents }));
    app
}

fn harness(app: FleetApp) -> Harness<'static, FleetApp> {
    Harness::builder()
        .with_max_steps(60)
        .with_size(Vec2::new(1400.0, 800.0))
        .build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), app)
}

fn nav(h: &mut Harness<'static, FleetApp>, entry: &str) {
    h.get_by_role_and_label(Role::Button, entry).click();
    h.run();
}

#[test]
fn rows_line_up() {
    let mut h = harness(with_agents(
        app(),
        vec![agent("a", "work", Status::Working)],
    ));
    // The rows' lines are panel separators, drawn from the visuals the app installs.
    theme::apply(&h.ctx);
    h.run();
    for view in ["Kanban", "Tasks", "Management", "Settings"] {
        nav(&mut h, view);
        let r = h.state().layout_rects();
        assert_eq!(
            r.drawer_header.bottom(),
            r.header.bottom(),
            "{view}: header rows end at different heights: {r:?}"
        );
        assert_eq!(r.drawer_header.height(), layout::HEADER, "{view}: {r:?}");
        assert_eq!(r.header.height(), layout::HEADER, "{view}: {r:?}");
        assert_eq!(
            r.drawer_footer.top(),
            r.footer.top(),
            "{view}: footer rows start at different heights: {r:?}"
        );
        assert_eq!(r.drawer_footer.height(), layout::FOOTER, "{view}: {r:?}");
        assert_eq!(r.footer.height(), layout::FOOTER, "{view}: {r:?}");
        assert_eq!(
            r.header.left() - r.drawer_header.left(),
            layout::DRAWER,
            "{view}: {r:?}"
        );
        // The one pixel column between the rows is the drawer's own border line.
        assert_eq!(
            r.header.left() - r.drawer_header.right(),
            1.0,
            "{view}: {r:?}"
        );
        assert_eq!(
            r.footer.left() - r.drawer_footer.right(),
            1.0,
            "{view}: {r:?}"
        );
        // One unbroken line under the headers and one over the footers, from the window's left edge to its right.
        let border = theme::tokens(h.ctx.global_style().visuals.dark_mode).border;
        for (name, y) in [
            ("header", r.header.bottom() - 0.5),
            ("footer", r.footer.top() + 0.5),
        ] {
            let mut x = r.drawer_header.left() + 0.5;
            while x < r.header.right() {
                assert_eq!(
                    colour_at(&h, egui::pos2(x, y)),
                    Some(border),
                    "{view}: the {name} line breaks at x={x}"
                );
                x += 1.0;
            }
        }
    }
}

/// The colour a viewer sees at `p`: the last filled rect or line painted over it.
fn colour_at(h: &Harness<'_, FleetApp>, p: egui::Pos2) -> Option<egui::Color32> {
    fn walk(shape: &egui::Shape, p: egui::Pos2, top: &mut Option<egui::Color32>) {
        match shape {
            egui::Shape::Rect(r) if r.fill.a() > 0 && r.rect.contains(p) => *top = Some(r.fill),
            egui::Shape::LineSegment {
                points: [a, b],
                stroke,
            } => {
                if stroke.width > 0.0
                    && egui::Rect::from_two_pos(*a, *b)
                        .expand(stroke.width / 2.0)
                        .contains(p)
                {
                    *top = Some(stroke.color);
                }
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, p, top)),
            _ => {}
        }
    }
    let mut top = None;
    for c in &h.output().shapes {
        if c.clip_rect.contains(p) {
            walk(&c.shape, p, &mut top);
        }
    }
    top
}

#[test]
fn nav_switches_views() {
    let mut h = harness(with_agents(
        app(),
        vec![agent("a", "work", Status::Working)],
    ));
    h.run();
    assert_eq!(h.state().view(), View::Kanban, "the app opens on Kanban");
    h.get_by_label("Queued");
    nav(&mut h, "Settings");
    assert_eq!(h.state().view(), View::Settings);
    h.get_by_role_and_label(Role::Label, "Settings");
    for tab in ["General", "Daemon", "Log"] {
        h.get_by_role_and_label(Role::Button, tab);
    }
    assert!(
        h.query_by_label("Queued").is_none(),
        "the board's columns are still drawn on Settings"
    );
    assert!(h.query_by_label("work").is_none());
    nav(&mut h, "Kanban");
    h.get_by_role_and_label(Role::Label, "Kanban");
    h.get_by_label("Queued");
    h.get_by_label("work");
}

#[test]
fn settings_remembers_its_tab_while_the_app_runs() {
    let mut h = harness(app());
    h.run();
    nav(&mut h, "Settings");
    assert!(
        h.get_by_role_and_label(Role::Button, "General")
            .accesskit_node()
            .toggled()
            == Some(Toggled::True),
        "Settings opens on General"
    );
    h.get_by_role_and_label(Role::Button, "Log").click();
    h.run();
    nav(&mut h, "Kanban");
    nav(&mut h, "Settings");
    assert!(
        h.get_by_role_and_label(Role::Button, "Log")
            .accesskit_node()
            .toggled()
            == Some(Toggled::True)
    );
}

#[test]
fn selection_survives_settings() {
    let mut h = harness(with_agents(
        app(),
        vec![
            agent("a", "fix the tests", Status::Working),
            agent("b", "other", Status::Review),
        ],
    ));
    h.run();
    h.get_by_label("fix the tests").click();
    h.run();
    assert_eq!(h.state().preview_agent(), Some("a"));
    h.get_by_role_and_label(Role::Button, "Terminal");
    nav(&mut h, "Settings");
    assert!(
        h.query_by_role_and_label(Role::Button, "Terminal")
            .is_none(),
        "the preview is drawn on Settings"
    );
    assert_eq!(
        h.state().preview_agent(),
        Some("a"),
        "Settings dropped the selection"
    );
    nav(&mut h, "Kanban");
    assert_eq!(h.state().preview_agent(), Some("a"));
    h.get_by_role_and_label(Role::Button, "Terminal");
}

#[test]
fn drawer_footer_counts() {
    let mut a = with_agents(
        app(),
        vec![
            agent("a", "one", Status::Working),
            agent("b", "two", Status::NeedsYou),
            agent("c", "three", Status::Review),
        ],
    );
    a.handle_stream(StreamMsg::Connected);
    let mut h = harness(a);
    h.run();
    h.get_by_label("live");
    h.get_by_label("3 agents · 1 needs you");

    h.state_mut().handle_stream(StreamMsg::Snapshot(Snapshot {
        seq: 2,
        agents: vec![
            agent("a", "one", Status::NeedsYou),
            agent("b", "two", Status::NeedsYou),
        ],
    }));
    h.run();
    h.get_by_label("2 agents · 2 need you");

    h.state_mut().handle_stream(StreamMsg::Snapshot(Snapshot {
        seq: 3,
        agents: vec![agent("a", "one", Status::Working)],
    }));
    h.run();
    h.get_by_label("1 agent");

    h.state_mut().handle_stream(StreamMsg::Disconnected);
    h.run();
    h.get_by_label("reconnecting…");
    assert!(h.query_by_label("live").is_none());
}

#[test]
fn new_task_is_in_the_drawer() {
    let mut h = harness(app());
    h.run();
    let r = h.state().layout_rects();
    let drawer = r.drawer_header.union(r.drawer_footer);
    let button = h.get_by_role_and_label(Role::Button, "New task").rect();
    assert!(
        drawer.contains_rect(button),
        "New task at {button:?} is outside the drawer {drawer:?}"
    );
    h.get_by_role_and_label(Role::Button, "New task").click();
    h.run();
    assert!(
        h.state().spawn_state().open,
        "New task did not open the dialog"
    );
}

#[test]
fn new_task_is_there_on_settings_too() {
    let mut h = harness(app());
    h.run();
    nav(&mut h, "Settings");
    h.get_by_role_and_label(Role::Button, "New task").click();
    h.run();
    assert!(h.state().spawn_state().open);
}

#[test]
fn theme_choice_saves_and_applies() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("gui.toml");
    let ctx = egui::Context::default();
    let mut a = FleetApp::detached(FleetClient::new("http://127.0.0.1:1"), ctx.clone())
        .with_settings(path.clone());
    a.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    let mut h = harness(a);
    h.run();
    nav(&mut h, "Settings");
    h.get_by_role_and_label(Role::Label, "Theme");
    h.get_by_label("Dark").click();
    h.run();
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("theme = \"dark\""));
    assert_eq!(on_disk(&path).theme, ThemeChoice::Dark);
    assert_eq!(h.state().theme_choice(), ThemeChoice::Dark);
    assert_eq!(
        ctx.options(|o| o.theme_preference),
        egui::ThemePreference::Dark
    );
    h.get_by_label("System").click();
    h.run();
    assert_eq!(on_disk(&path).theme, ThemeChoice::System);
    assert_eq!(
        ctx.options(|o| o.theme_preference),
        egui::ThemePreference::System
    );
}

/// Runs `ctx` (the app's own context, not the harness's) for a frame with the window focused, so pending fonts
/// install and `animate` sees focus.
fn focused_frame(ctx: &egui::Context) {
    let mut input = egui::RawInput::default();
    input
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .focused = Some(true);
    ctx.run_ui(input, |_| {}).textures_delta.clear();
}

fn first_font(ctx: &egui::Context, family: egui::FontFamily) -> String {
    ctx.fonts(|f| f.definitions().families[&family][0].clone())
}

#[test]
fn font_choices_save_and_apply_independently() {
    use fleet_client::config::{MonoFont, UiFont};
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("gui.toml");
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut a = FleetApp::detached(FleetClient::new("http://127.0.0.1:1"), ctx.clone())
        .with_settings(path.clone());
    a.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    let mut h = harness(a);
    h.run();
    nav(&mut h, "Settings");
    h.get_by_role_and_label(Role::Label, "Font");
    h.get_by_role_and_label(Role::Label, "Mono font");

    h.get_by_label("Space Grotesk").click();
    h.run();
    focused_frame(&ctx);
    assert_eq!(on_disk(&path).ui_font, UiFont::SpaceGrotesk);
    assert_eq!(h.state().fonts().ui, UiFont::SpaceGrotesk);
    assert_eq!(
        first_font(&ctx, egui::FontFamily::Proportional),
        "space-grotesk"
    );
    assert_eq!(first_font(&ctx, egui::FontFamily::Monospace), "plex-mono");

    h.get_by_label("JetBrains Mono").click();
    h.run();
    focused_frame(&ctx);
    let saved = on_disk(&path);
    assert_eq!(
        (saved.ui_font, saved.mono_font),
        (UiFont::SpaceGrotesk, MonoFont::JetBrains)
    );
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("mono_font = \"jetbrains\""));
    assert_eq!(
        first_font(&ctx, egui::FontFamily::Monospace),
        "jetbrains-mono"
    );
    assert_eq!(
        first_font(&ctx, egui::FontFamily::Proportional),
        "space-grotesk"
    );

    h.get_by_label("IBM Plex Sans").click();
    h.run();
    focused_frame(&ctx);
    assert_eq!(on_disk(&path).ui_font, UiFont::Plex);
    assert_eq!(
        first_font(&ctx, egui::FontFamily::Proportional),
        "plex-sans"
    );
    assert_eq!(
        first_font(&ctx, egui::FontFamily::Monospace),
        "jetbrains-mono"
    );
}

#[test]
fn font_chips_wrap_in_a_narrow_window() {
    let mut a = FleetApp::detached(
        FleetClient::new("http://127.0.0.1:1"),
        egui::Context::default(),
    );
    a.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    let mut h = Harness::builder()
        .with_max_steps(60)
        .with_size(Vec2::new(560.0, 900.0))
        .build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), a);
    h.run();
    nav(&mut h, "Settings");
    for label in ["Atkinson Hyperlegible Next", "Atkinson Hyperlegible Mono"] {
        let r = h.get_by_label(label).rect();
        assert!(
            r.right() <= 560.0,
            "{label} chip ends at {} in a 560 px window",
            r.right()
        );
    }
}

#[test]
fn saved_fonts_apply_at_startup() {
    use fleet_client::config::{GuiSettings, MonoFont, UiFont};
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("gui.toml");
    GuiSettings {
        ui_font: UiFont::Atkinson,
        mono_font: MonoFont::Atkinson,
        ..Default::default()
    }
    .save(&path)
    .unwrap();
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let a =
        FleetApp::detached(FleetClient::new("http://127.0.0.1:1"), ctx.clone()).with_settings(path);
    focused_frame(&ctx);
    assert_eq!(a.fonts().ui, UiFont::Atkinson);
    assert_eq!(
        first_font(&ctx, egui::FontFamily::Proportional),
        "atkinson-next"
    );
    assert_eq!(
        first_font(&ctx, egui::FontFamily::Monospace),
        "atkinson-mono"
    );
}

#[test]
fn animations_switch_saves_and_applies() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("gui.toml");
    let ctx = egui::Context::default();
    let mut a = FleetApp::detached(FleetClient::new("http://127.0.0.1:1"), ctx.clone())
        .with_settings(path.clone());
    a.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    let mut h = harness(a);
    h.run();
    nav(&mut h, "Settings");
    focused_frame(&ctx);
    assert!(theme::animate(&ctx), "on by default while focused");
    h.get_by_label("Animations").click();
    h.run();
    focused_frame(&ctx);
    assert!(!on_disk(&path).animations);
    assert!(!h.state().animations());
    assert!(!theme::animate(&ctx), "off even while focused");
    h.get_by_label("Animations").click();
    h.run();
    focused_frame(&ctx);
    assert!(on_disk(&path).animations);
    assert!(theme::animate(&ctx));
}

#[test]
fn general_lists_the_close_choices() {
    let mut h = harness(app());
    h.run();
    nav(&mut h, "Settings");
    h.get_by_role_and_label(Role::Label, "Closing the window");
    for choice in ["Ask", "Keep daemon running", "Stop daemon"] {
        h.get_by_label(choice);
    }
}

#[test]
fn the_theme_button_is_gone() {
    let mut h = harness(app());
    h.run();
    assert!(h.query_by_label_contains("Theme:").is_none());
}

#[test]
fn an_action_error_shows_in_the_content_footer() {
    let (_guard, log) = common::log::test_log();
    let mut h = harness(with_agents(
        app().with_log(log),
        vec![agent("a", "work", Status::Working)],
    ));
    h.run();
    h.state_mut()
        .handle_result(AppResult::Action(Err("agent is not running".into())));
    h.run();
    let error = h.get_by_label("agent is not running").rect();
    let footer = h.state().layout_rects().footer;
    assert!(
        footer.contains_rect(error),
        "the error at {error:?} is not in the footer {footer:?}"
    );
}

fn with_counts(mut app: FleetApp) -> FleetApp {
    app.handle_result(AppResult::Tasks(Ok(fleet_api::TaskList {
        tasks: vec![],
        orphans: vec![],
        clones_bytes: None,
    })));
    app
}

#[test]
fn the_task_counts_sit_in_a_strip_between_the_header_and_the_columns() {
    let mut h = harness(with_counts(with_agents(
        app(),
        vec![agent("a", "work", Status::Working)],
    )));
    theme::apply(&h.ctx);
    h.run();
    let r = h.state().layout_rects();
    assert_eq!(r.header.height(), layout::HEADER, "{r:?}");
    assert_eq!(r.strip.top(), r.header.bottom(), "{r:?}");
    assert_eq!(r.strip.height(), layout::STRIP, "{r:?}");
    assert_eq!(
        (r.strip.left(), r.strip.right()),
        (r.header.left(), r.header.right()),
        "the strip spans the content area: {r:?}"
    );
    for (role, label) in [(Role::Label, "Kanban"), (Role::ComboBox, "Repo filter")] {
        let at = h.get_by_role_and_label(role, label).rect();
        assert!(r.header.contains_rect(at), "{label} {at:?} left the header");
    }
    for label in ["Tasks:", "0 running", "0 stopped"] {
        let at = h.get_by_label(label).rect();
        assert!(
            r.strip.contains_rect(at),
            "{label} {at:?} is not in the strip {r:?}"
        );
    }
    let column = h.get_by_label("Queued").rect();
    assert!(
        column.top() >= r.strip.bottom(),
        "the columns start above the strip's end: {column:?}, {r:?}"
    );
}

#[test]
fn only_the_board_has_the_strip() {
    let mut h = harness(with_counts(app()));
    h.run();
    for view in ["Tasks", "Settings"] {
        nav(&mut h, view);
        assert_eq!(
            h.state().layout_rects().strip,
            egui::Rect::NOTHING,
            "{view}"
        );
        assert!(h.query_by_label("Tasks:").is_none(), "{view}");
    }
}

#[test]
fn the_panel_header_and_the_column_headers_share_one_bottom_line() {
    let mut h = harness(with_counts(with_agents(
        app(),
        vec![agent("a", "work", Status::Working)],
    )));
    theme::apply(&h.ctx);
    h.run();
    h.get_by_label("work").click();
    h.run();
    let r = h.state().layout_rects();
    let bottom = r.strip.bottom() + layout::COLUMN_HEADER;
    let close = h.get_by_role_and_label(Role::Button, "Close").rect();
    let queued = h.get_by_label("Queued").rect();
    for (name, at) in [("Close", close), ("Queued", queued)] {
        assert!(
            at.top() >= r.strip.bottom() && at.bottom() <= bottom,
            "{name} {at:?} is not in the row between y={} and y={bottom}",
            r.strip.bottom()
        );
    }
    assert!(close.left() > queued.right(), "the panel is to the right");
    // One unbroken line under the column headers and the panel's top row, from the board's left to the panel's right.
    let border = theme::tokens(h.ctx.global_style().visuals.dark_mode).border;
    let y = bottom - 0.5;
    let mut x = r.strip.left() + 0.5;
    while x < r.strip.right() {
        assert_eq!(
            colour_at(&h, egui::pos2(x, y)),
            Some(border),
            "the line under the headers breaks at x={x}"
        );
        x += 1.0;
    }
}

/// Every shape the frame painted, in paint order, with the clip rect it was painted under.
fn painted(h: &Harness<'_, FleetApp>) -> Vec<(egui::Rect, egui::Shape)> {
    fn walk(clip: egui::Rect, shape: &egui::Shape, out: &mut Vec<(egui::Rect, egui::Shape)>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(clip, s, out)),
            other => out.push((clip, other.clone())),
        }
    }
    let mut out = Vec::new();
    for c in &h.output().shapes {
        walk(c.clip_rect, &c.shape, &mut out);
    }
    out
}

/// The watermark's C blade: the one 50-vertex mesh, in a faint accent.
fn blade(shapes: &[(egui::Rect, egui::Shape)]) -> Option<(usize, egui::Rect, egui::Color32)> {
    shapes
        .iter()
        .enumerate()
        .find_map(|(i, (clip, s))| match s {
            egui::Shape::Mesh(m) if m.vertices.len() == 50 => Some((i, *clip, m.vertices[0].color)),
            _ => None,
        })
}

/// The sweep wedge's outline points: the only 14-point filled path.
fn wedge(shapes: &[(egui::Rect, egui::Shape)]) -> Vec<egui::Pos2> {
    shapes
        .iter()
        .find_map(|(_, s)| match s {
            egui::Shape::Path(p) if p.points.len() == 14 && p.fill.a() > 0 => {
                Some(p.points.clone())
            }
            _ => None,
        })
        .expect("the sweep wedge is painted")
}

fn watermark_harness(height: f32) -> Harness<'static, FleetApp> {
    let mut h = Harness::builder()
        .with_max_steps(60)
        .with_size(Vec2::new(1400.0, height))
        .build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), app());
    theme::apply(&h.ctx);
    h.run();
    h
}

#[test]
fn the_drawer_shows_the_watermark_behind_the_nav() {
    let h = watermark_harness(800.0);
    let r = h.state().layout_rects();
    let shapes = painted(&h);
    let (at, clip, colour) = blade(&shapes).expect("the blade is painted");
    let fill = theme::tokens(true).fill;
    let want = fill.gamma_multiply(colour.a() as f32 / 255.0);
    for (got, want) in [
        (colour.r(), want.r()),
        (colour.g(), want.g()),
        (colour.b(), want.b()),
    ] {
        assert!(
            got.abs_diff(want) <= 1,
            "the watermark is the accent: {colour:?} vs {want:?}"
        );
    }
    assert!(colour.a() > 0 && colour.a() <= 26, "faint: {colour:?}");
    // The icon is wider than the drawer, so what shows is what the clip leaves.
    assert!(clip.left() >= r.drawer_header.left(), "{clip:?} {r:?}");
    assert!(clip.right() <= r.drawer_header.right(), "{clip:?} {r:?}");
    assert!(clip.bottom() <= r.drawer_footer.top(), "{clip:?} {r:?}");
    let nav_text = shapes
        .iter()
        .position(|(_, s)| matches!(s, egui::Shape::Text(t) if t.galley.text() == "Kanban"))
        .expect("the nav is painted");
    assert!(
        at < nav_text,
        "the watermark ({at}) paints before the nav ({nav_text})"
    );
}

#[test]
fn a_short_window_clips_the_watermark() {
    let h = watermark_harness(360.0);
    let r = h.state().layout_rects();
    let (_, clip, _) = blade(&painted(&h)).expect("the blade is painted");
    assert!(clip.bottom() <= r.drawer_footer.top(), "{clip:?} {r:?}");
    assert!(clip.right() <= r.drawer_header.right(), "{clip:?} {r:?}");
    assert!(clip.left() >= r.drawer_header.left(), "{clip:?} {r:?}");
}

#[test]
fn the_radar_turns_only_while_animating() {
    let mut h = watermark_harness(800.0);
    h.input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .focused = Some(true);
    let at = |h: &mut Harness<'static, FleetApp>, t: f64| {
        h.input_mut().time = Some(t);
        // `step`, not `run`: a turning radar schedules a repaint, which would make `run` loop.
        h.step();
        let delay = h.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        (wedge(&painted(h)), delay)
    };
    let (a, delay) = at(&mut h, 0.0);
    assert!(
        delay <= theme::ANIMATION_FRAME,
        "asks for the next frame: {delay:?}"
    );
    let (b, _) = at(&mut h, 10.0);
    assert_ne!(a, b, "the wedge turns");

    theme::set_animations(&h.ctx, false);
    let (a, _) = at(&mut h, 20.0);
    let (b, delay) = at(&mut h, 30.0);
    assert_eq!(a, b, "still with animations off");
    assert!(
        delay > theme::ANIMATION_FRAME,
        "no repaint asked for: {delay:?}"
    );
}
