use eframe::egui::{self, accesskit::Role, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use fleet_api::{
    default_ui_schema, AgentConfig, AgentConfigUpdate, AgentSettings, GithubAuth, LoginState,
    Persona, PersonaList, PersonaUpdate, RepoInfo, RepoSource,
};
use fleet_client::http::FleetClient;
use fleet_gui::app::{AgentConfigCall, AppResult, FleetApp, PersonaCall, NO_TERMINAL};
use fleet_gui::views::management::ManagementTab;
use fleet_gui::views::View;

fn login(ok: bool) -> Option<LoginState> {
    Some(LoginState {
        ok,
        account: ok.then(|| "ann".to_string()),
        detail: (!ok).then(|| "token expired".to_string()),
    })
}

fn persona(id: &str, name: &str, ready: bool) -> Persona {
    Persona {
        id: id.into(),
        name: name.into(),
        host: "github.com".into(),
        commit_name: "Ann Example".into(),
        commit_email: "ann@example.invalid".into(),
        github_auth: GithubAuth::Gh,
        claude_dir: format!("/personas/{id}/claude"),
        gh_dir: format!("/personas/{id}/gh"),
        ready,
        claude: ready.then(|| login(true)).flatten(),
        github: ready.then(|| login(true)).flatten(),
        projects: 0,
    }
}

fn list(personas: Vec<Persona>) -> PersonaList {
    PersonaList {
        personas,
        claude_command: vec!["claude".into()],
        gh_command: "gh".into(),
        projects_root: "/projects".into(),
    }
}

fn harness(personas: Vec<Persona>) -> Harness<'static, FleetApp> {
    let mut app = FleetApp::detached(
        FleetClient::new("http://127.0.0.1:1"),
        egui::Context::default(),
    )
    .without_calls();
    app.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    app.handle_result(AppResult::Repos(Ok(vec![])));
    app.handle_result(AppResult::Personas(Ok(list(personas))));
    let mut h = Harness::builder()
        .with_max_steps(60)
        .with_size(Vec2::new(1400.0, 800.0))
        .build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), app);
    h.run();
    h
}

fn management(personas: Vec<Persona>) -> Harness<'static, FleetApp> {
    let mut h = harness(personas);
    h.get_by_role_and_label(Role::Button, "Management").click();
    h.run();
    h
}

fn click(h: &mut Harness<'static, FleetApp>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).click();
    h.run();
}

#[test]
fn nav_lists_management_above_settings() {
    let h = harness(vec![persona("a", "Work", true)]);
    let ys: Vec<f32> = ["Kanban", "Projects", "Management", "Settings"]
        .iter()
        .map(|n| {
            h.get_by_role_and_label(Role::Button, n)
                .accesskit_node()
                .raw_bounds()
                .expect("bounds")
                .y0 as f32
        })
        .collect();
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "nav order: {ys:?}");
}

#[test]
fn management_opens_on_personas_and_remembers_its_tab() {
    let mut h = management(vec![persona("a", "Work", true)]);
    assert_eq!(h.state().view(), View::Management);
    assert_eq!(h.state().management_tab(), ManagementTab::Personas);
    for tab in ["Personas", "Instructions", "Agent settings"] {
        h.get_by_role_and_label(Role::Button, tab);
    }
    click(&mut h, "Instructions");
    click(&mut h, "Kanban");
    click(&mut h, "Management");
    assert_eq!(h.state().management_tab(), ManagementTab::Instructions);
}

#[test]
fn personas_tab_lists_states() {
    let mut broken = persona("b", "Broken", true);
    broken.claude = login(false);
    let h = management(vec![
        persona("a", "Ready", true),
        broken,
        persona("c", "Draft", false),
    ]);
    h.get_by_role_and_label(Role::Button, "Ready ✓");
    h.get_by_role_and_label(Role::Button, "Broken ⚠");
    h.get_by_role_and_label(Role::Button, "Draft draft");
    h.get_by_role_and_label(Role::Button, "+ New persona");
}

#[test]
fn draft_shows_resume_and_delete_and_resume_opens_the_login_step() {
    let mut h = management(vec![
        persona("a", "Ready", true),
        persona("c", "Draft", false),
    ]);
    click(&mut h, "Draft draft");
    h.get_by_role_and_label(Role::Button, "Delete");
    click(&mut h, "Resume");
    h.get_by_label("Log in");
    assert!(h.query_by_label_contains("Step 3 of 3").is_none());
}

#[test]
fn selecting_a_persona_checks_it_and_loads_its_repos() {
    let mut h = management(vec![persona("a", "Work", true), persona("b", "Home", true)]);
    assert!(h
        .state()
        .persona_calls()
        .contains(&PersonaCall::Check("a".into())));
    assert!(h
        .state()
        .persona_calls()
        .contains(&PersonaCall::Repos("a".into())));
    click(&mut h, "Home ✓");
    assert!(h
        .state()
        .persona_calls()
        .contains(&PersonaCall::Repos("b".into())));
    h.state_mut().handle_result(AppResult::PersonaRepos(
        "b".into(),
        Ok(vec![RepoInfo {
            name: "fleet".into(),
            source: RepoSource::Path,
            location: "/p/fleet".into(),
            persona: Some("b".into()),
            roster: vec![],
            // Found, so the row's workflow line does not repeat the repo name this test looks for.
            workflow: fleet_api::WorkflowState::Found {
                base: "main".into(),
                steps: vec!["fmt".into()],
            },
        }]),
    ));
    h.run();
    h.get_by_label_contains("fleet");
}

#[test]
fn delete_disabled_with_projects() {
    let mut busy = persona("a", "Work", true);
    busy.projects = 2;
    let h = management(vec![busy]);
    assert!(h
        .get_by_role_and_label(Role::Button, "Delete")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Used by 2 projects");
}

#[test]
fn delete_asks_in_the_app_then_deletes() {
    let mut h = management(vec![persona("a", "Work", true), persona("b", "Home", true)]);
    click(&mut h, "Delete");
    h.get_by_label("Delete Work?");
    assert!(!h
        .state()
        .persona_calls()
        .iter()
        .any(|c| matches!(c, PersonaCall::Delete(_))));
    // Cancel changes nothing.
    click(&mut h, "Cancel");
    assert!(h.query_by_label("Delete Work?").is_none());
    click(&mut h, "Delete");
    // The dialog's own button is the second Delete.
    h.get_all_by_role_and_label(Role::Button, "Delete")
        .last()
        .unwrap()
        .click();
    h.run();
    assert!(h
        .state()
        .persona_calls()
        .contains(&PersonaCall::Delete("a".into())));
    h.state_mut()
        .handle_result(AppResult::PersonaDeleted("a".into(), Ok(())));
    h.run();
    assert!(h.query_by_role_and_label(Role::Button, "Work ✓").is_none());
    h.get_by_role_and_label(Role::Button, "Home ✓");
}

#[test]
fn relogin_switch_to_pat_sends_update() {
    let mut h = management(vec![persona("a", "Work", true)]);
    click(&mut h, "Personal access token");
    click(&mut h, "Re-login to GitHub");
    assert!(h.state().persona_calls().contains(&PersonaCall::Update(
        "a".into(),
        PersonaUpdate {
            name: None,
            commit_name: None,
            commit_email: None,
            github_auth: Some(GithubAuth::Pat),
        }
    )));
    let mut switched = persona("a", "Work", true);
    switched.github_auth = GithubAuth::Pat;
    h.state_mut().handle_result(AppResult::PersonaUpdated(
        Ok(switched),
        Some(fleet_gui::components::Login::Github),
    ));
    h.run();
    h.get_by_role_and_label(Role::Button, "Save token");
}

#[test]
fn rename_sends_only_the_name() {
    let mut h = management(vec![persona("a", "Work", true)]);
    let field = h.get_by_role_and_label(Role::TextInput, "Name");
    field.click();
    field.type_text(" 2");
    h.run();
    click(&mut h, "Save name");
    assert!(h.state().persona_calls().contains(&PersonaCall::Update(
        "a".into(),
        PersonaUpdate {
            name: Some("Work 2".into()),
            commit_name: None,
            commit_email: None,
            github_auth: None,
        }
    )));
}

#[test]
fn new_persona_opens_setup_without_step_numbers() {
    let mut h = management(vec![persona("a", "Work", true)]);
    click(&mut h, "+ New persona");
    h.get_by_label("New persona");
    assert!(h.query_by_label_contains("Step ").is_none());
    h.get_by_role_and_label(Role::Button, "Cancel");
}

#[test]
fn pat_relogin_uses_the_masked_field_and_records_no_token() {
    let mut pat = persona("a", "Work", true);
    pat.github_auth = GithubAuth::Pat;
    let mut h = management(vec![pat]);
    click(&mut h, "Re-login to GitHub");
    // No auth switch: no update, the token field opens.
    assert!(!h
        .state()
        .persona_calls()
        .iter()
        .any(|c| matches!(c, PersonaCall::Update(..))));
    let field = h.get_by_role_and_label(Role::PasswordInput, "Personal access token");
    field.click();
    field.type_text("ghp_secret");
    h.run();
    let shown = h
        .get_by_role_and_label(Role::PasswordInput, "Personal access token")
        .value()
        .unwrap_or_default();
    assert!(!shown.contains("ghp_secret"), "{shown}");
    click(&mut h, "Save token");
    assert!(h
        .state()
        .persona_calls()
        .contains(&PersonaCall::Pat("a".into())));
    // The token left the field with the call, and never reached the call record.
    let after = h
        .get_by_role_and_label(Role::PasswordInput, "Personal access token")
        .value()
        .unwrap_or_default();
    assert!(after.is_empty(), "{after}");
}

#[test]
fn the_detail_pane_sits_inside_the_window() {
    let h = management(vec![persona("a", "Work", true)]);
    for label in ["Save identity", "Delete", "+ New persona"] {
        let b = h
            .get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .raw_bounds()
            .expect("bounds");
        assert!(
            b.x0 >= 0.0 && b.x1 <= 1400.0 && b.y0 >= 0.0 && b.y1 <= 800.0,
            "{label}: {b:?}"
        );
    }
    let save = h
        .get_by_role_and_label(Role::Button, "Save identity")
        .accesskit_node()
        .raw_bounds()
        .unwrap();
    assert!(
        save.x0 > 200.0 + 260.0,
        "the detail is right of the list: {save:?}"
    );
}

#[test]
fn a_login_result_for_another_persona_is_dropped() {
    let mut h = management(vec![persona("a", "Work", true), persona("b", "Home", true)]);
    click(&mut h, "Re-login to Claude");
    click(&mut h, "Home ✓");
    // Work's terminal answers after the switch to Home.
    h.state_mut()
        .handle_result(AppResult::LoginOpened("a".into(), Err(NO_TERMINAL.into())));
    h.run();
    assert!(h.query_by_label_contains("No terminal found").is_none());
}

#[test]
fn a_token_result_for_another_persona_is_dropped() {
    let mut pat_a = persona("a", "Work", true);
    pat_a.github_auth = GithubAuth::Pat;
    let mut h = management(vec![pat_a, persona("b", "Home", true)]);
    click(&mut h, "Re-login to GitHub");
    let field = h.get_by_role_and_label(Role::PasswordInput, "Personal access token");
    field.click();
    field.type_text("x");
    h.run();
    click(&mut h, "Save token");
    click(&mut h, "Home ✓");
    h.state_mut()
        .handle_result(AppResult::PatSaved("a".into(), Err("bad token".into())));
    h.run();
    assert!(h.query_by_label_contains("bad token").is_none());
}

#[test]
fn leaving_management_forgets_the_delete_dialog() {
    let mut h = management(vec![persona("a", "Work", true)]);
    click(&mut h, "Delete");
    h.get_by_label("Delete Work?");
    h.state_mut().set_view(View::Kanban);
    h.run();
    h.state_mut().set_view(View::Management);
    h.run();
    assert!(h.query_by_label("Delete Work?").is_none());
}

#[test]
fn the_two_relogins_are_named() {
    let h = management(vec![persona("a", "Work", true)]);
    h.get_by_role_and_label(Role::Button, "Re-login to Claude");
    h.get_by_role_and_label(Role::Button, "Re-login to GitHub");
}

const INSTRUCTIONS: &str = "Instructions for every agent";

fn config(instructions: &str) -> AgentConfig {
    AgentConfig {
        instructions: instructions.into(),
        settings: AgentSettings::default(),
        settings_error: None,
    }
}

/// Management on `tab`, with the daemon's config injected.
fn config_tab(tab: &str, cfg: AgentConfig) -> Harness<'static, FleetApp> {
    let mut h = management(vec![persona("a", "Work", true)]);
    h.state_mut().handle_result(AppResult::AgentConfig(Ok(cfg)));
    click(&mut h, tab);
    h
}

fn type_into(h: &mut Harness<'static, FleetApp>, label: &str, text: &str) {
    let role = if label == INSTRUCTIONS {
        Role::MultilineTextInput
    } else {
        Role::TextInput
    };
    let field = h.get_by_role_and_label(role, label);
    field.click();
    field.type_text(text);
    h.run();
}

fn save_enabled(h: &Harness<'static, FleetApp>, label: &str) -> bool {
    !h.get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .is_disabled()
}

#[test]
fn opening_management_loads_the_config_once() {
    let h = management(vec![persona("a", "Work", true)]);
    assert_eq!(h.state().agent_config_calls(), [AgentConfigCall::Load]);
}

#[test]
fn instructions_save_sends_text() {
    let mut h = config_tab("Instructions", config("Be brief."));
    assert!(!save_enabled(&h, "Save"));
    assert!(!save_enabled(&h, "Revert"));
    h.get_by_label_contains("Applies to agents started after saving.");
    type_into(&mut h, INSTRUCTIONS, " Be kind.");
    assert!(save_enabled(&h, "Save"));
    click(&mut h, "Save");
    // Only the text goes: the settings file is not this tab's to rewrite.
    assert_eq!(
        h.state().agent_config_calls().last(),
        Some(&AgentConfigCall::Save(AgentConfigUpdate {
            instructions: Some("Be brief. Be kind.".into()),
            settings: None,
        }))
    );
    h.state_mut()
        .handle_result(AppResult::AgentConfigSaved(Ok(config(
            "Be brief. Be kind.",
        ))));
    h.run();
    assert!(!save_enabled(&h, "Save"));
}

#[test]
fn revert_restores_saved() {
    let mut h = config_tab("Instructions", config("Be brief."));
    type_into(&mut h, INSTRUCTIONS, " Extra");
    click(&mut h, "Revert");
    assert_eq!(
        h.get_by_role_and_label(Role::MultilineTextInput, INSTRUCTIONS)
            .value()
            .as_deref(),
        Some("Be brief.")
    );
    assert!(!save_enabled(&h, "Save"));

    click(&mut h, "Agent settings");
    click(&mut h, "Add allow rule");
    type_into(&mut h, "allow rule 1", "Bash(ls)");
    click(&mut h, "Revert");
    assert!(h
        .query_by_role_and_label(Role::TextInput, "allow rule 1")
        .is_none());
}

#[test]
fn settings_rows_add_and_remove() {
    let mut cfg = config("");
    cfg.settings.deny = vec!["Read(./.env)".into()];
    let mut h = config_tab("Agent settings", cfg);
    h.get_by_role_and_label(Role::TextInput, "deny rule 1");
    click(&mut h, "Add allow rule");
    click(&mut h, "Add variable");
    type_into(&mut h, "allow rule 1", "Bash(ls)");
    type_into(&mut h, "Variable name 1", "EDITOR");
    type_into(&mut h, "Variable value 1", "vim");
    click(&mut h, "Save");
    let mut settings = AgentSettings {
        allow: vec!["Bash(ls)".into()],
        deny: vec!["Read(./.env)".into()],
        ..Default::default()
    };
    settings.env.insert("EDITOR".into(), "vim".into());
    let want = AgentConfigUpdate {
        instructions: None,
        settings: Some(settings),
    };
    assert_eq!(
        h.state().agent_config_calls().last(),
        Some(&AgentConfigCall::Save(want))
    );
    h.state_mut().handle_result(AppResult::AgentConfigSaved(Ok({
        let mut c = config("");
        c.settings.allow = vec!["Bash(ls)".into()];
        c.settings.deny = vec!["Read(./.env)".into()];
        c.settings.env.insert("EDITOR".into(), "vim".into());
        c
    })));
    h.run();
    click(&mut h, "Remove deny rule 1");
    assert!(h
        .query_by_role_and_label(Role::TextInput, "deny rule 1")
        .is_none());
    click(&mut h, "Remove variable 1");
    assert!(h
        .query_by_role_and_label(Role::TextInput, "Variable name 1")
        .is_none());
    assert!(save_enabled(&h, "Save"));
}

#[test]
fn duplicate_variable_is_a_local_error() {
    let mut h = config_tab("Agent settings", config(""));
    for _ in 0..2 {
        click(&mut h, "Add variable");
    }
    type_into(&mut h, "Variable name 1", "A");
    type_into(&mut h, "Variable name 2", "A");
    let calls = h.state().agent_config_calls().len();
    click(&mut h, "Save");
    assert_eq!(h.state().agent_config_calls().len(), calls);
    h.get_by_label_contains("A is set twice");
}

#[test]
fn server_rejection_shows_message_and_keeps_draft() {
    let mut h = config_tab("Agent settings", config(""));
    click(&mut h, "Add variable");
    type_into(&mut h, "Variable name 1", "GH_TOKEN");
    click(&mut h, "Save");
    h.state_mut().handle_result(AppResult::AgentConfigSaved(Err(
        "GH_TOKEN is reserved".into()
    )));
    h.run();
    h.get_by_label_contains("GH_TOKEN is reserved");
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Variable name 1")
            .value()
            .as_deref(),
        Some("GH_TOKEN")
    );
    assert!(save_enabled(&h, "Save"));
}

#[test]
fn leaving_the_tab_keeps_the_draft() {
    let mut h = config_tab("Instructions", config("a"));
    type_into(&mut h, INSTRUCTIONS, "b");
    click(&mut h, "Personas");
    click(&mut h, "Instructions");
    assert_eq!(
        h.get_by_role_and_label(Role::MultilineTextInput, INSTRUCTIONS)
            .value()
            .as_deref(),
        Some("ab")
    );
}

#[test]
fn a_broken_file_shows_its_message_and_stays_editable() {
    let mut h = management(vec![persona("a", "Work", true)]);
    h.state_mut().handle_result(AppResult::AgentConfig(Err(
        "settings.json is not valid JSON".into(),
    )));
    click(&mut h, "Agent settings");
    h.get_by_label_contains("settings.json is not valid JSON");
    click(&mut h, "Add allow rule");
    type_into(&mut h, "allow rule 1", "Bash(ls)");
    assert!(save_enabled(&h, "Save"));
    // Re-entering the tab asks again.
    click(&mut h, "Personas");
    click(&mut h, "Agent settings");
    assert_eq!(
        h.state().agent_config_calls(),
        [AgentConfigCall::Load, AgentConfigCall::Load]
    );
}

#[test]
fn a_settings_save_after_a_settings_error_never_sends_instructions() {
    let mut cfg = config("keep me");
    cfg.settings_error = Some("cannot read settings.json: bad".into());
    let mut h = config_tab("Agent settings", cfg);
    h.get_by_label_contains("cannot read settings.json: bad");
    // Saving valid settings is how the file is repaired, so Save is on while Revert (nothing to undo) is not.
    assert!(save_enabled(&h, "Save"));
    assert!(!save_enabled(&h, "Revert"));
    click(&mut h, "Add allow rule");
    type_into(&mut h, "allow rule 1", "Read");
    click(&mut h, "Save");
    let Some(AgentConfigCall::Save(sent)) = h.state().agent_config_calls().last() else {
        panic!("no save");
    };
    assert_eq!(sent.instructions, None);
    assert_eq!(sent.settings.as_ref().unwrap().allow, ["Read"]);
    h.state_mut().handle_result(AppResult::AgentConfigSaved(Ok({
        let mut c = config("keep me");
        c.settings.allow = vec!["Read".into()];
        c
    })));
    h.run();
    assert!(h
        .query_by_label_contains("cannot read settings.json")
        .is_none());
}

#[test]
fn the_instructions_tab_works_while_settings_are_broken() {
    let mut cfg = config("keep me");
    cfg.settings_error = Some("settings.json is broken".into());
    let mut h = config_tab("Instructions", cfg);
    assert!(h
        .query_by_label_contains("settings.json is broken")
        .is_none());
    type_into(&mut h, INSTRUCTIONS, "!");
    click(&mut h, "Save");
    assert_eq!(
        h.state().agent_config_calls().last(),
        Some(&AgentConfigCall::Save(AgentConfigUpdate {
            instructions: Some("keep me!".into()),
            settings: None,
        }))
    );
}

#[test]
fn a_saved_instructions_answer_keeps_a_settings_draft() {
    let mut h = config_tab("Agent settings", config(""));
    click(&mut h, "Add allow rule");
    click(&mut h, "Instructions");
    type_into(&mut h, INSTRUCTIONS, "x");
    click(&mut h, "Save");
    h.state_mut()
        .handle_result(AppResult::AgentConfigSaved(Ok(config("x"))));
    click(&mut h, "Agent settings");
    h.get_by_role_and_label(Role::TextInput, "allow rule 1");
}

fn def(kind: fleet_api::DefKind, name: &str, error: Option<&str>) -> fleet_api::DefInfo {
    fleet_api::DefInfo {
        kind,
        name: name.into(),
        summary: String::new(),
        builtin: true,
        overridden: false,
        error: error.map(String::from),
    }
}

fn fleet_repo(roster: &[&str]) -> RepoInfo {
    RepoInfo {
        name: "fleet".into(),
        source: RepoSource::Path,
        location: "/p/fleet".into(),
        persona: Some("a".into()),
        roster: roster.iter().map(|r| r.to_string()).collect(),
        workflow: fleet_api::WorkflowState::Missing {
            base: "main".into(),
        },
    }
}

#[test]
fn repo_row_saves_roster() {
    use fleet_api::DefKind::{Orchestrator, Specialist};
    let mut h = management(vec![persona("a", "Work", true)]);
    h.state_mut().handle_result(AppResult::Definitions(Ok(vec![
        def(Orchestrator, "direct", None),
        def(Specialist, "reviewer", None),
        def(Specialist, "rust", None),
        def(Specialist, "stale", Some("specialists/stale.md: bad")),
    ])));
    h.state_mut().handle_result(AppResult::PersonaRepos(
        "a".into(),
        Ok(vec![fleet_repo(&["rust", "stale"])]),
    ));
    h.run();
    // Only specialists are offered, and the one that no longer loads is named so it is not lost silently.
    assert!(h.query_by_role_and_label(Role::Button, "direct").is_none());
    h.get_by_label_contains("stale");
    assert!(h
        .query_by_role_and_label(Role::TextInput, "Test command")
        .is_none());
    click(&mut h, "reviewer");
    click(&mut h, "Save");
    let sent = fleet_api::RepoFleet {
        roster: vec!["rust".into(), "reviewer".into()],
    };
    assert_eq!(
        h.state().persona_calls().last(),
        Some(&PersonaCall::RepoFleet("a".into(), "fleet".into(), sent))
    );
    h.state_mut().handle_result(AppResult::RepoFleetSaved(
        "a".into(),
        Ok(fleet_repo(&["rust", "reviewer"])),
    ));
    h.run();
    assert!(h
        .get_by_role_and_label(Role::Button, "Save")
        .accesskit_node()
        .is_disabled());
    assert!(h.query_by_label_contains("stale").is_none());
}

#[test]
fn a_refused_repo_save_shows_its_reason() {
    let mut h = management(vec![persona("a", "Work", true)]);
    h.state_mut()
        .handle_result(AppResult::Definitions(Ok(vec![def(
            fleet_api::DefKind::Specialist,
            "rust",
            None,
        )])));
    h.state_mut().handle_result(AppResult::PersonaRepos(
        "a".into(),
        Ok(vec![fleet_repo(&[])]),
    ));
    h.run();
    click(&mut h, "rust");
    click(&mut h, "Save");
    h.state_mut().handle_result(AppResult::RepoFleetSaved(
        "a".into(),
        Err("no specialist named `x`".into()),
    ));
    h.run();
    h.get_by_label_contains("no specialist named `x`");
}

#[test]
fn definitions_tabs() {
    use fleet_api::DefFile;
    use fleet_api::DefKind::{Orchestrator, Specialist};
    use fleet_gui::app::DefCall;
    let mut h = management(vec![persona("a", "Work", true)]);
    let mut overridden = def(Specialist, "rust", None);
    overridden.overridden = true;
    let mut mine = def(Specialist, "go", None);
    mine.builtin = false;
    h.state_mut().handle_result(AppResult::Definitions(Ok(vec![
        def(Orchestrator, "bug-fix", None),
        overridden,
        mine,
        def(
            Specialist,
            "broken",
            Some("specialists/broken.md: no skills"),
        ),
    ])));
    h.run();
    click(&mut h, "Agent settings");
    for tab in ["Orchestrators", "Specialists", "Skills"] {
        h.get_by_role_and_label(Role::Button, tab);
    }

    // A built-in is read-only until Override.
    click(&mut h, "Orchestrators");
    assert_eq!(h.state().management_tab(), ManagementTab::Orchestrators);
    assert!(h.query_by_label_contains("rust").is_none());
    click(&mut h, "bug-fix built-in");
    assert_eq!(
        h.state().definition_calls().last(),
        Some(&DefCall::Read(Orchestrator, "bug-fix".into()))
    );
    let text = "---\nname: bug-fix\n---\nFix it.";
    h.state_mut().handle_result(AppResult::Definition(
        Orchestrator,
        "bug-fix".into(),
        Ok(DefFile {
            kind: Orchestrator,
            name: "bug-fix".into(),
            text: text.into(),
            builtin: true,
        }),
    ));
    h.run();
    assert!(h.query_by_role_and_label(Role::Button, "Save").is_none());
    let editor = h.get_by_role(Role::MultilineTextInput);
    editor.click();
    editor.type_text("typed");
    h.run();
    assert_eq!(
        h.get_by_role(Role::MultilineTextInput)
            .accesskit_node()
            .value()
            .as_deref(),
        Some(text),
        "a built-in cannot be edited"
    );
    click(&mut h, "Override");
    let editor = h.get_by_role(Role::MultilineTextInput);
    editor.click();
    h.run();
    h.get_by_role(Role::MultilineTextInput).type_text(" Now.");
    h.run();
    click(&mut h, "Save");
    let Some(DefCall::Save(Orchestrator, name, sent)) =
        h.state().definition_calls().last().cloned()
    else {
        panic!("no save: {:?}", h.state().definition_calls());
    };
    assert_eq!(name, "bug-fix");
    assert!(sent.contains("Now."), "{sent:?}");

    // An invalid file: the daemon's message under the editor, and the draft kept.
    h.state_mut().handle_result(AppResult::DefinitionSaved(
        Orchestrator,
        "bug-fix".into(),
        Err("orchestrators/bug-fix.md: missing `skills`".into()),
    ));
    h.run();
    h.get_by_label_contains("missing `skills`");
    assert!(h
        .get_by_role(Role::MultilineTextInput)
        .accesskit_node()
        .value()
        .is_some_and(|v| v.contains("Now.")));

    // The list says what each one is; Reset asks first.
    click(&mut h, "Specialists");
    h.get_by_role_and_label(Role::Button, "go custom");
    h.get_by_role_and_label(Role::Button, "broken built-in ⚠");
    h.get_by_label_contains("no skills");
    click(&mut h, "rust overrides built-in");
    h.state_mut().handle_result(AppResult::Definition(
        Specialist,
        "rust".into(),
        Ok(DefFile {
            kind: Specialist,
            name: "rust".into(),
            text: "---\nname: rust\n---\nMine.".into(),
            builtin: false,
        }),
    ));
    h.run();
    h.get_by_role_and_label(Role::Button, "Revert");
    click(&mut h, "Reset to built-in");
    h.get_by_label("Reset rust to built-in?");
    assert!(!h
        .state()
        .definition_calls()
        .iter()
        .any(|c| matches!(c, DefCall::Reset(..))));
    click(&mut h, "Cancel");
    click(&mut h, "Reset to built-in");
    click(&mut h, "Reset");
    assert_eq!(
        h.state().definition_calls().last(),
        Some(&DefCall::Reset(Specialist, "rust".into()))
    );
    // A refused reset says why under the editor, like a refused save, and keeps the file.
    h.state_mut().handle_result(AppResult::DefinitionReset(
        Specialist,
        "rust".into(),
        Err("specialists/rust.md: permission denied".into()),
    ));
    h.run();
    h.get_by_label_contains("permission denied");
    h.get_by_role_and_label(Role::Button, "Reset to built-in");

    click(&mut h, "Reset to built-in");
    click(&mut h, "Reset");
    h.state_mut().handle_result(AppResult::DefinitionReset(
        Specialist,
        "rust".into(),
        Ok(()),
    ));
    h.run();
    // The draft is gone and the built-in is read again.
    assert_eq!(
        h.state().definition_calls().last(),
        Some(&DefCall::Read(Specialist, "rust".into()))
    );
    assert_eq!(h.query_all_by_role(Role::MultilineTextInput).count(), 0);
    let builtin = "---\nname: rust\n---\nBuilt in.";
    h.state_mut().handle_result(AppResult::Definition(
        Specialist,
        "rust".into(),
        Ok(DefFile {
            kind: Specialist,
            name: "rust".into(),
            text: builtin.into(),
            builtin: true,
        }),
    ));
    h.run();
    h.get_by_role_and_label(Role::Button, "Override");
    assert!(h.query_by_role_and_label(Role::Button, "Save").is_none());
    assert_eq!(
        h.get_by_role(Role::MultilineTextInput)
            .accesskit_node()
            .value()
            .as_deref(),
        Some(builtin)
    );
}

fn row_with(workflow: fleet_api::WorkflowState) -> RepoInfo {
    RepoInfo {
        workflow,
        ..fleet_repo(&[])
    }
}

#[test]
fn personas_row_shows_workflow_state() {
    use fleet_api::WorkflowState::{Found, Invalid, Missing, Unreadable};
    let cases = [
        (
            Found {
                base: "main".into(),
                steps: vec!["fmt".into(), "gui".into()],
            },
            "Checks on main: fmt, gui",
        ),
        (
            Missing {
                base: "main".into(),
            },
            "fleet has no .fleet/workflow.yml on main.",
        ),
        (
            Invalid {
                base: "main".into(),
                error: "bad".into(),
            },
            "fleet: .fleet/workflow.yml on main is invalid: bad",
        ),
        (
            Unreadable {
                base: "main".into(),
                error: "no origin".into(),
            },
            "fleet: cannot read .fleet/workflow.yml on main: no origin",
        ),
    ];
    for (w, text) in cases {
        let mut h = management(vec![persona("a", "Work", true)]);
        h.state_mut()
            .handle_result(AppResult::PersonaRepos("a".into(), Ok(vec![row_with(w)])));
        h.run();
        h.get_by_label_contains(text);
        assert!(h
            .query_by_role_and_label(Role::Button, "Create workflow")
            .is_none());
    }
}

#[test]
fn workflow_tab_explains_the_file() {
    let mut h = management(vec![persona("a", "Work", true)]);
    click(&mut h, "Workflow");
    assert_eq!(h.state().management_tab(), ManagementTab::Workflow);
    h.get_by_label_contains(
        "Each repo lists its checks in .fleet/workflow.yml on its base branch.",
    );
    h.get_by_label_contains("steps:");
    h.get_by_label_contains(
        "An editor for this file is coming. For now, add the file to the repo and push it.",
    );
}
