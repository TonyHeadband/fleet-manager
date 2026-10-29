use eframe::egui::{self, accesskit::Role, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use fleet_api::{default_ui_schema, GithubAuth, LoginState, NewPersona, Persona, PersonaList};
use fleet_client::http::FleetClient;
use fleet_gui::app::{AppResult, FleetApp, PersonaCall, NO_TERMINAL};
use fleet_gui::components::Login;

fn login(ok: bool, account: &str) -> Option<LoginState> {
    Some(LoginState {
        ok,
        account: ok.then(|| account.to_string()),
        detail: (!ok).then(|| "not logged in".to_string()),
        tool: None,
    })
}

fn persona(id: &str, auth: GithubAuth) -> Persona {
    Persona {
        id: id.into(),
        name: "Work".into(),
        host: "github.com".into(),
        commit_name: "Ann Example".into(),
        commit_email: "ann@example.invalid".into(),
        github_auth: auth,
        claude_dir: format!("/personas/{id}/claude"),
        gh_dir: format!("/personas/{id}/gh"),
        ready: false,
        claude: None,
        github: None,
        tasks: 0,
        nicknames: Default::default(),
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

fn app() -> FleetApp {
    let mut app = FleetApp::detached(
        FleetClient::new("http://127.0.0.1:1"),
        egui::Context::default(),
    )
    .without_calls();
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

/// The wizard on its first persona step, with nothing created yet.
fn wizard() -> Harness<'static, FleetApp> {
    let mut a = app();
    a.handle_result(AppResult::Personas(Ok(list(vec![]))));
    let mut h = harness(a);
    h.run();
    h
}

fn type_into(h: &mut Harness<'static, FleetApp>, label: &str, text: &str) {
    let field = h.get_by_role_and_label(Role::TextInput, label);
    field.click();
    field.type_text(text);
    h.run();
}

fn fill_identity(h: &mut Harness<'static, FleetApp>) {
    type_into(h, "Name", "Work");
    type_into(h, "Commit name", "Ann Example");
    type_into(h, "Commit email", "ann@example.invalid");
}

/// The wizard on the Log in step for a persona the daemon just created.
fn at_login(auth: GithubAuth) -> Harness<'static, FleetApp> {
    let mut h = wizard();
    fill_identity(&mut h);
    if auth == GithubAuth::Pat {
        h.get_by_role_and_label(Role::Button, "Personal access token")
            .click();
        h.run();
    }
    h.get_by_role_and_label(Role::Button, "Next").click();
    h.run();
    h.state_mut()
        .handle_result(AppResult::PersonaSaved(Ok(persona("d1", auth))));
    h.run();
    h
}

fn finish_disabled(h: &Harness<'static, FleetApp>) -> bool {
    h.get_by_role_and_label(Role::Button, "Finish")
        .accesskit_node()
        .is_disabled()
}

#[test]
fn no_ready_persona_shows_wizard_and_blocks_new_task() {
    let h = wizard();
    h.get_by_label("Step 3 of 4 · Your first persona");
    assert!(h
        .get_by_role_and_label(Role::Button, "New task")
        .accesskit_node()
        .is_disabled());
    // The wizard has no way out but Finish.
    assert!(h.query_by_role_and_label(Role::Button, "Cancel").is_none());
    assert!(h.query_by_role_and_label(Role::Button, "Close").is_none());
}

#[test]
fn a_ready_persona_shows_the_board() {
    let mut a = app();
    let mut p = persona("p1", GithubAuth::Gh);
    p.ready = true;
    a.handle_result(AppResult::Personas(Ok(list(vec![p]))));
    let mut h = harness(a);
    h.run();
    assert!(h.query_by_label_contains("Step 3 of 4").is_none());
    assert!(!h
        .get_by_role_and_label(Role::Button, "New task")
        .accesskit_node()
        .is_disabled());
    assert_eq!(h.state().active_persona(), Some("p1"));
}

#[test]
fn a_persona_list_not_loaded_blocks_nothing() {
    let mut h = harness(app());
    h.run();
    assert!(h.query_by_label_contains("Step 3 of 4").is_none());
    assert!(!h
        .get_by_role_and_label(Role::Button, "New task")
        .accesskit_node()
        .is_disabled());
}

#[test]
fn next_creates_the_persona_with_github_com_by_default() {
    let mut h = wizard();
    h.get_by_label("GitHub host");
    fill_identity(&mut h);
    h.get_by_role_and_label(Role::Button, "Next").click();
    h.run();
    assert!(h
        .state()
        .persona_calls()
        .contains(&PersonaCall::Create(NewPersona {
            name: "Work".into(),
            host: "github.com".into(),
            commit_name: "Ann Example".into(),
            commit_email: "ann@example.invalid".into(),
            github_auth: GithubAuth::Gh,
        })));
}

#[test]
fn create_error_shows_under_form() {
    let mut h = wizard();
    fill_identity(&mut h);
    h.get_by_role_and_label(Role::Button, "Next").click();
    h.run();
    h.state_mut().handle_result(AppResult::PersonaSaved(Err(
        "the github host must be a host name, like github.com".into(),
    )));
    h.run();
    h.get_by_label("the github host must be a host name, like github.com");
    h.get_by_label("Step 3 of 4 · Your first persona");
}

#[test]
fn the_login_step_checks_the_persona() {
    let h = at_login(GithubAuth::Gh);
    h.get_by_label("Step 4 of 4 · Log in");
    h.get_by_label("Trust the folder (second option), then type /login");
    assert_eq!(
        h.query_all_by_role_and_label(Role::Button, "Log in…")
            .count(),
        2
    );
    assert!(h
        .state()
        .persona_calls()
        .contains(&PersonaCall::Check("d1".into())));
}

#[test]
fn finish_disabled_until_both_logins_ok() {
    let mut h = at_login(GithubAuth::Gh);
    assert!(finish_disabled(&h));
    let mut p = persona("d1", GithubAuth::Gh);
    p.claude = login(false, "");
    p.github = login(true, "ann");
    h.state_mut()
        .handle_result(AppResult::PersonaChecked(Ok(p.clone())));
    h.run();
    h.get_by_label("ann");
    h.get_by_label("not logged in");
    assert!(finish_disabled(&h));
    assert_eq!(h.state().active_persona(), None);

    p.claude = login(true, "ann@example.invalid");
    p.ready = true;
    h.state_mut()
        .handle_result(AppResult::PersonaChecked(Ok(p)));
    h.run();
    assert!(!finish_disabled(&h));
    assert_eq!(h.state().active_persona(), Some("d1"));
    h.get_by_role_and_label(Role::Button, "Finish").click();
    h.run();
    assert!(h.query_by_label_contains("Step 4 of 4").is_none());
    assert!(h.query_by_label_contains("Step 3 of 4").is_none());
    assert!(!h
        .get_by_role_and_label(Role::Button, "New task")
        .accesskit_node()
        .is_disabled());
}

#[test]
fn pat_field_names_the_permissions_it_needs() {
    let h = at_login(GithubAuth::Pat);
    assert!(h
        .query_by_label_contains("Contents and Pull requests: Read and write")
        .is_some());
    assert!(h.query_by_label_contains("repo and read:org").is_some());
}

#[test]
fn pat_field_is_masked_and_cleared_after_submit() {
    let mut h = at_login(GithubAuth::Pat);
    let field = h.get_by_role_and_label(Role::PasswordInput, "Personal access token");
    field.click();
    field.type_text("ghp_secret");
    h.run();
    let shown = h
        .get_by_role_and_label(Role::PasswordInput, "Personal access token")
        .value()
        .unwrap_or_default();
    assert!(!shown.contains("ghp_secret"), "{shown}");
    assert_eq!(shown.chars().count(), "ghp_secret".len());
    h.get_by_role_and_label(Role::Button, "Save token").click();
    h.run();
    assert_eq!(
        h.get_by_role_and_label(Role::PasswordInput, "Personal access token")
            .value()
            .unwrap_or_default(),
        ""
    );
    assert!(h
        .state()
        .persona_calls()
        .contains(&PersonaCall::Pat("d1".into())));
    // A PAT persona has no gh login button: only Claude's.
    assert_eq!(
        h.query_all_by_role_and_label(Role::Button, "Log in…")
            .count(),
        1
    );
}

#[test]
fn cancel_on_login_deletes_draft() {
    let mut h = at_login(GithubAuth::Gh);
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run();
    assert!(h
        .state()
        .persona_calls()
        .contains(&PersonaCall::Delete("d1".into())));
    h.get_by_label("Step 3 of 4 · Your first persona");
    // What was typed stays, so the user can fix it and go again.
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Name").value(),
        Some("Work".into())
    );
}

#[test]
fn no_terminal_shows_the_script_to_copy() {
    let mut h = at_login(GithubAuth::Gh);
    h.query_all_by_role_and_label(Role::Button, "Log in…")
        .next()
        .unwrap()
        .click();
    h.run();
    assert!(h
        .state()
        .persona_calls()
        .contains(&PersonaCall::Login(Login::Claude)));
    h.state_mut()
        .handle_result(AppResult::LoginOpened("d1".into(), Err(NO_TERMINAL.into())));
    h.run();
    h.get_by_label_contains("CLAUDE_CONFIG_DIR=/personas/d1/claude");
    h.get_by_role_and_label(Role::Button, "Copy");
}

#[test]
fn a_draft_left_behind_resumes_at_login() {
    let mut a = app();
    a.handle_result(AppResult::Personas(Ok(list(vec![persona(
        "d9",
        GithubAuth::Gh,
    )]))));
    let mut h = harness(a);
    h.run();
    h.get_by_label("Step 4 of 4 · Log in");
    assert!(h
        .state()
        .persona_calls()
        .contains(&PersonaCall::Check("d9".into())));
}

/// The script row reuses the trust hint's layout: Copy keeps its size and stays inside the dialog.
#[test]
fn the_script_and_copy_fit_the_dialog() {
    let mut h = at_login(GithubAuth::Gh);
    h.query_all_by_role_and_label(Role::Button, "Log in…")
        .nth(1)
        .unwrap()
        .click();
    h.run();
    h.state_mut()
        .handle_result(AppResult::LoginOpened("d1".into(), Err(NO_TERMINAL.into())));
    h.run();
    let script = h.get_by_label_contains("GH_CONFIG_DIR=/personas/d1/gh");
    let copy = h.get_by_role_and_label(Role::Button, "Copy").rect();
    let finish = h.get_by_role_and_label(Role::Button, "Finish").rect();
    assert!(copy.width() > copy.height(), "Copy is {copy:?}");
    assert!(
        copy.right() <= finish.right() + 1.0 && script.rect().left() < copy.left(),
        "Copy {copy:?} leaves the dialog (Finish at {finish:?})"
    );
}

#[test]
fn the_first_ready_persona_is_saved_as_active() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("gui.toml");
    let mut a = app().with_settings(path.clone());
    a.handle_result(AppResult::Personas(Ok(list(vec![persona(
        "d1",
        GithubAuth::Gh,
    )]))));
    let mut p = persona("d1", GithubAuth::Gh);
    p.ready = true;
    a.handle_result(AppResult::PersonaChecked(Ok(p)));
    assert_eq!(a.active_persona(), Some("d1"));
    fleet_client::config::SettingsWriter::shared().flush();
    let saved = fleet_client::config::GuiSettings::load(&path);
    assert_eq!(saved.active_persona.as_deref(), Some("d1"));

    // An active persona is kept when another becomes ready.
    let mut other = persona("d2", GithubAuth::Gh);
    other.ready = true;
    a.handle_result(AppResult::PersonaChecked(Ok(other)));
    assert_eq!(a.active_persona(), Some("d1"));
}

fn ready(id: &str) -> Persona {
    let mut p = persona(id, GithubAuth::Gh);
    p.claude = login(true, "ann@example.invalid");
    p.github = login(true, "ann");
    p.ready = true;
    p
}

#[test]
fn cancel_is_hidden_once_the_persona_is_ready() {
    let mut h = at_login(GithubAuth::Gh);
    h.get_by_role_and_label(Role::Button, "Cancel");
    h.state_mut()
        .handle_result(AppResult::PersonaChecked(Ok(ready("d1"))));
    h.run();
    assert!(h.query_by_role_and_label(Role::Button, "Cancel").is_none());
}

#[test]
fn a_deleted_persona_is_no_longer_active() {
    let mut h = at_login(GithubAuth::Gh);
    h.state_mut()
        .handle_result(AppResult::PersonaChecked(Ok(ready("d1"))));
    h.run();
    assert_eq!(h.state().active_persona(), Some("d1"));
    // Logged out again before Finish: Cancel is back and deletes it.
    let mut out = ready("d1");
    out.ready = false;
    out.claude = login(false, "");
    h.state_mut()
        .handle_result(AppResult::PersonaChecked(Ok(out)));
    h.run();
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run();
    h.state_mut()
        .handle_result(AppResult::PersonaDeleted("d1".into(), Ok(())));
    assert_eq!(h.state().active_persona(), None);

    fill_identity(&mut h);
    h.get_by_role_and_label(Role::Button, "Next").click();
    h.run();
    h.state_mut()
        .handle_result(AppResult::PersonaSaved(Ok(persona("d2", GithubAuth::Gh))));
    h.state_mut()
        .handle_result(AppResult::PersonaChecked(Ok(ready("d2"))));
    assert_eq!(h.state().active_persona(), Some("d2"));
}

#[test]
fn a_check_landing_after_cancel_does_not_make_the_draft_active() {
    let mut h = at_login(GithubAuth::Gh);
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run();
    h.state_mut()
        .handle_result(AppResult::PersonaChecked(Ok(ready("d1"))));
    assert_eq!(h.state().active_persona(), None);
}

#[test]
fn a_saved_active_persona_no_longer_listed_is_replaced() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("gui.toml");
    let mut s = fleet_client::config::GuiSettings::load(&path);
    s.active_persona = Some("gone".into());
    s.save(&path).unwrap();
    let mut a = app().with_settings(path);
    assert_eq!(a.active_persona(), Some("gone"));
    a.handle_result(AppResult::Personas(Ok(list(vec![ready("p1")]))));
    assert_eq!(a.active_persona(), Some("p1"));
}

#[test]
fn a_check_error_for_another_persona_is_not_shown() {
    let mut h = at_login(GithubAuth::Gh);
    // The check `at_login` started was for d1; Cancel drops it, and its failure must not reach the new draft.
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Next").click();
    h.run();
    h.state_mut()
        .handle_result(AppResult::PersonaSaved(Ok(persona("d2", GithubAuth::Gh))));
    h.state_mut()
        .handle_result(AppResult::PersonaChecked(Err("daemon gone".into())));
    h.run();
    assert!(h.query_by_label_contains("daemon gone").is_none());
}

#[test]
fn undo_after_save_token_does_not_bring_the_token_back() {
    let mut h = at_login(GithubAuth::Pat);
    let field = h.get_by_role_and_label(Role::PasswordInput, "Personal access token");
    field.click();
    field.type_text("ghp_secret");
    // Long enough for the edit to settle into an undo point.
    for _ in 0..90 {
        h.step();
    }
    h.get_by_role_and_label(Role::Button, "Save token").click();
    h.run();
    let state = egui::text_edit::TextEditState::load(&h.ctx, fleet_gui::components::pat_field_id())
        .expect("the field has state");
    let empty = (egui::text::CCursorRange::default(), String::new());
    assert!(!state.undoer().has_undo(&empty));
    let field = h.get_by_role_and_label(Role::PasswordInput, "Personal access token");
    field.click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run();
    assert_eq!(
        h.get_by_role_and_label(Role::PasswordInput, "Personal access token")
            .value()
            .unwrap_or_default(),
        ""
    );
}

#[test]
fn an_upgrade_without_the_folders_step_counts_three_steps() {
    let mut a = app().with_wizard_steps(fleet_gui::components::WizardSteps {
        folders: false,
        requirements: true,
    });
    a.handle_result(AppResult::Personas(Ok(list(vec![]))));
    let mut h = harness(a);
    h.run();
    h.get_by_label("Step 2 of 3 · Your first persona");
    fill_identity(&mut h);
    h.get_by_role_and_label(Role::Button, "Next").click();
    h.run();
    h.state_mut()
        .handle_result(AppResult::PersonaSaved(Ok(persona("d1", GithubAuth::Gh))));
    h.run();
    h.get_by_label("Step 3 of 3 · Log in");
}

fn tool_login(e: fleet_api::ToolError) -> Option<LoginState> {
    Some(LoginState {
        ok: false,
        account: None,
        detail: Some(e.to_string()),
        tool: Some(e),
    })
}

/// A missing CLI is shown as its tool error, with its fix, and no Log in: a terminal could not run it.
#[test]
fn a_missing_cli_shows_its_fix_instead_of_log_in() {
    use fleet_api::{ToolError, ToolFix, ToolId};
    let mut h = at_login(GithubAuth::Gh);
    let log_ins = |h: &Harness<'static, FleetApp>| {
        h.query_all_by_role_and_label(Role::Button, "Log in…")
            .count()
    };
    assert_eq!(log_ins(&h), 2);
    let mut p = persona("d1", GithubAuth::Gh);
    p.claude = tool_login(ToolError::NotInstalled {
        tool: ToolId::Claude,
        fix: ToolFix {
            install: Some("install-claude".into()),
            link: "https://example.invalid/claude".into(),
            manager: None,
        },
    });
    // Only logged out: Log in is the fix.
    p.github = tool_login(ToolError::NotLoggedIn {
        tool: ToolId::Gh,
        persona: Some("Work".into()),
    });
    h.state_mut()
        .handle_result(AppResult::PersonaChecked(Ok(p)));
    h.run();
    h.get_by_label("Claude Code is not installed");
    h.get_by_label("install-claude");
    h.get_by_label("https://example.invalid/claude");
    // Installing is the setup wizard's; here the command is only offered to copy.
    assert!(h.query_by_role_and_label(Role::Button, "Install").is_none());
    assert_eq!(log_ins(&h), 1, "GitHub keeps its Log in");
}
