use eframe::egui::{self, accesskit::Role, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use fleet_api::{default_ui_schema, GithubAuth, LoginState, Persona, PersonaList};
use fleet_client::http::FleetClient;
use fleet_client::shortcuts::{self, Places, ShortcutState};
use fleet_gui::app::{AppResult, FleetApp};
use fleet_gui::views::View;
use std::path::PathBuf;

const FILE: &str = if cfg!(windows) {
    "Fleet Manager.lnk"
} else {
    "fleet-manager.desktop"
};

struct Fx {
    _tmp: tempfile::TempDir,
    places: Places,
    exe: PathBuf,
}

/// Temp places and a stand-in exe: nothing here touches the real Start menu or Desktop.
fn fx() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let exe = tmp.path().join("app").join(if cfg!(windows) {
        "fleet-gui.exe"
    } else {
        "fleet-gui"
    });
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"x").unwrap();
    let places = Places {
        menu: tmp.path().join("menu"),
        desktop: tmp.path().join("desktop"),
    };
    Fx {
        _tmp: tmp,
        places,
        exe,
    }
}

fn app(fx: &Fx) -> FleetApp {
    let mut app = FleetApp::detached(
        FleetClient::new("http://127.0.0.1:1"),
        egui::Context::default(),
    )
    .without_calls()
    .with_shortcuts(fx.places.clone(), fx.exe.clone());
    app.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    app.handle_result(AppResult::Repos(Ok(vec![])));
    app
}

fn harness(app: FleetApp) -> Harness<'static, FleetApp> {
    Harness::builder()
        .with_max_steps(60)
        .with_size(Vec2::new(1200.0, 800.0))
        .build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), app)
}

fn persona(ready: bool) -> Persona {
    let ok = || {
        Some(LoginState {
            ok: true,
            account: Some("ann".into()),
            detail: None,
            tool: None,
        })
    };
    Persona {
        id: "d1".into(),
        name: "Work".into(),
        host: "github.com".into(),
        commit_name: "Ann Example".into(),
        commit_email: "ann@example.invalid".into(),
        github_auth: GithubAuth::Gh,
        claude_dir: "/personas/d1/claude".into(),
        gh_dir: "/personas/d1/gh".into(),
        ready,
        claude: ready.then(ok).flatten(),
        github: ready.then(ok).flatten(),
        tasks: 0,
    }
}

fn list(personas: Vec<Persona>) -> PersonaList {
    PersonaList {
        personas,
        claude_command: vec!["claude".into()],
        gh_command: "gh".into(),
        tasks_root: "/tasks".into(),
        claude_path: None,
        gh_path: None,
    }
}

fn settings(fx: &Fx) -> Harness<'static, FleetApp> {
    let mut a = app(fx);
    a.handle_result(AppResult::Personas(Ok(list(vec![persona(true)]))));
    a.set_view(View::Settings);
    let mut h = harness(a);
    h.run();
    h
}

fn click(h: &mut Harness<'static, FleetApp>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).click();
    h.run();
}

#[test]
fn settings_adds_and_removes_the_ticked_places() {
    let fx = fx();
    let mut h = settings(&fx);
    h.get_by_label("No shortcuts yet");
    // The Start menu is ticked at first, the Desktop is not.
    click(&mut h, Role::Button, "Add shortcuts");
    assert!(fx.places.menu.join(FILE).exists());
    assert!(!fx.places.desktop.join(FILE).exists());
    h.get_by_label("Shortcuts in: Start menu");

    click(&mut h, Role::CheckBox, "Desktop");
    click(&mut h, Role::Button, "Add shortcuts");
    h.get_by_label("Shortcuts in: Start menu, Desktop");

    click(&mut h, Role::CheckBox, "Start menu");
    click(&mut h, Role::Button, "Remove shortcuts");
    assert!(fx.places.menu.join(FILE).exists());
    assert!(!fx.places.desktop.join(FILE).exists());
    h.get_by_label("Shortcuts in: Start menu");

    // Nothing ticked: nothing to add or remove.
    click(&mut h, Role::CheckBox, "Desktop");
    assert!(h
        .get_by_role_and_label(Role::Button, "Add shortcuts")
        .accesskit_node()
        .is_disabled());
}

#[test]
fn an_old_location_offers_update() {
    let fx = fx();
    let old = fx.exe.with_file_name(format!(
        "old-{}",
        fx.exe.file_name().unwrap().to_string_lossy()
    ));
    std::fs::write(&old, b"x").unwrap();
    shortcuts::add(
        &fx.places,
        shortcuts::Which {
            menu: true,
            desktop: false,
        },
        &old,
    )
    .unwrap();
    let mut h = settings(&fx);
    h.get_by_label_contains("Shortcuts point to an old location");
    click(&mut h, Role::Button, "Update");
    assert_eq!(
        shortcuts::state(&fx.places, &fx.exe),
        ShortcutState::Current {
            menu: true,
            desktop: false
        }
    );
    assert!(h
        .query_by_label_contains("Shortcuts point to an old location")
        .is_none());
}

/// The wizard on its Log in step with a persona that is ready, so Finish can be clicked.
fn wizard_at_finish(fx: &Fx) -> Harness<'static, FleetApp> {
    let mut a = app(fx);
    a.handle_result(AppResult::Personas(Ok(list(vec![]))));
    let mut h = harness(a);
    h.run();
    for (label, text) in [
        ("Name", "Work"),
        ("Commit name", "Ann Example"),
        ("Commit email", "ann@example.invalid"),
    ] {
        let field = h.get_by_role_and_label(Role::TextInput, label);
        field.click();
        field.type_text(text);
        h.run();
    }
    click(&mut h, Role::Button, "Next");
    h.state_mut()
        .handle_result(AppResult::PersonaSaved(Ok(persona(false))));
    h.run();
    h.state_mut()
        .handle_result(AppResult::PersonaChecked(Ok(persona(true))));
    h.run();
    h
}

/// What the wizard calls the menu: Windows has a Start menu, Linux desktops an app menu.
const MENU_BOX: &str = if cfg!(windows) {
    "Add a Start menu shortcut"
} else {
    "Add an app menu shortcut"
};

#[test]
fn the_wizard_offers_an_unticked_start_menu_shortcut() {
    let fx = fx();
    let mut h = wizard_at_finish(&fx);
    let tick = h.get_by_role_and_label(Role::CheckBox, MENU_BOX);
    assert_ne!(
        tick.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    click(&mut h, Role::Button, "Finish");
    assert!(!fx.places.menu.join(FILE).exists());
}

#[test]
fn finishing_with_the_box_ticked_adds_the_menu_shortcut_only() {
    let fx = fx();
    let mut h = wizard_at_finish(&fx);
    click(&mut h, Role::CheckBox, MENU_BOX);
    click(&mut h, Role::Button, "Finish");
    assert!(fx.places.menu.join(FILE).exists());
    assert!(!fx.places.desktop.join(FILE).exists());
    assert_eq!(
        shortcuts::state(&fx.places, &fx.exe),
        ShortcutState::Current {
            menu: true,
            desktop: false
        }
    );
}
