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

fn agent(id: &str, task: &str, status: Status) -> Agent {
    Agent {
        id: AgentId(id.into()),
        task: task.into(),
        workdir: "/w".into(),
        workspaces: vec![Workspace {
            repo: "core".into(),
            branch: "fleet/x".into(),
            worktree: "/w/core".into(),
            base: "abc".into(),
            from_branch: None,
            has_changes: false,
            owned: false,
        }],
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
    for view in ["Kanban", "Settings"] {
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
fn new_project_is_in_the_drawer() {
    let mut h = harness(app());
    h.run();
    let r = h.state().layout_rects();
    let drawer = r.drawer_header.union(r.drawer_footer);
    let button = h.get_by_role_and_label(Role::Button, "New project").rect();
    assert!(
        drawer.contains_rect(button),
        "New project at {button:?} is outside the drawer {drawer:?}"
    );
    h.get_by_role_and_label(Role::Button, "New project").click();
    h.run();
    assert!(
        h.state().spawn_state().open,
        "New project did not open the dialog"
    );
}

#[test]
fn new_project_is_there_on_settings_too() {
    let mut h = harness(app());
    h.run();
    nav(&mut h, "Settings");
    h.get_by_role_and_label(Role::Button, "New project").click();
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
