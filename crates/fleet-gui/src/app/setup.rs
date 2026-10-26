//! The app's side of the persona setup: when it shows, the calls it asks for and what their answers change.
use super::{
    products::{repos_in, PICK_PRODUCT},
    AgentConfigCall, AppResult, FleetApp, PersonaCall, PersonasLoad, CHECK_EVERY, NO_TERMINAL,
};
use crate::components::{
    delete_persona_dialog, modal_frame, persona_labels, persona_setup_ui, Confirm,
    DeletePersonaDialog, Login, PersonaSetup, SetupAction, SetupMode, SetupStep,
};
use crate::views::management::{self, ConfigLoad, ConfigPart, ManagementAction};
use eframe::egui;
use fleet_api::{AgentConfigUpdate, GithubAuth, Persona, PersonaUpdate};
use fleet_client::config::SettingsWriter;
use fleet_client::http::FleetError;
use fleet_client::login;
use std::collections::HashMap;

/// What a terminal that was (not) opened changes: nothing, the script to run by hand, or the error.
fn opened(
    result: Result<(), String>,
    which: Login,
    script: String,
    (claude_script, gh_script): (&mut Option<String>, &mut Option<String>),
    error: &mut Option<String>,
) {
    match result {
        Ok(()) => {}
        Err(e) if e == NO_TERMINAL => match which {
            Login::Claude => *claude_script = Some(script),
            Login::Github => *gh_script = Some(script),
        },
        Err(e) => *error = Some(format!("Could not open a terminal: {e}")),
    }
}

impl FleetApp {
    /// The persona new tasks default to.
    pub fn active_persona(&self) -> Option<&str> {
        self.active_persona.as_deref()
    }

    #[doc(hidden)]
    pub fn agent_config_calls(&self) -> &[AgentConfigCall] {
        &self.agent_config_calls
    }

    pub fn persona_calls(&self) -> &[PersonaCall] {
        &self.persona_calls
    }

    /// Why New task cannot run for want of a persona. An app that never asked for the list (tests, embedding)
    /// blocks nothing.
    pub(super) fn persona_blocker(&self) -> Option<&'static str> {
        match (&self.personas, self.personas_load) {
            (Some(list), _) => {
                (!list.personas.iter().any(|p| p.ready)).then_some("Set up a persona first")
            }
            (None, PersonasLoad::Idle) => None,
            (None, PersonasLoad::Loading) => Some("Loading personas…"),
            (None, PersonasLoad::Failed) => Some("Could not load the personas; retrying"),
        }
    }

    pub(super) fn fetch_personas(&mut self) {
        if self.personas_load == PersonasLoad::Loading {
            return;
        }
        self.personas_sent += 1;
        if !self.calls {
            return;
        }
        self.personas_load = PersonasLoad::Loading;
        self.background(|c| AppResult::Personas(c.personas().map_err(|e| e.0)));
    }

    /// A failed first load is asked again on the tasks poll: until it lands, nothing can be gated on it.
    pub(super) fn retry_personas(&mut self) {
        if self.personas.is_none() && self.personas_load == PersonasLoad::Failed {
            self.fetch_personas();
        }
    }

    /// The listed personas' names by id; drafts included, as a task may name one being set up again.
    pub(super) fn persona_names(&self) -> HashMap<String, String> {
        self.personas
            .iter()
            .flat_map(|l| &l.personas)
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect()
    }

    /// The board's persona filter rows `(id, label, agent count)`: the ready personas, then any persona an agent
    /// runs under that is not one of them. The labels are the cards' tags too.
    pub(super) fn persona_rows(&self) -> Vec<(String, String, usize)> {
        let mut counts: Vec<(String, usize)> = self
            .personas
            .iter()
            .flat_map(|l| &l.personas)
            .filter(|p| p.ready)
            .map(|p| (p.id.clone(), 0))
            .collect();
        for agent in &self.snapshot.agents {
            match counts.iter_mut().find(|(id, _)| *id == agent.persona) {
                Some((_, n)) => *n += 1,
                None => counts.push((agent.persona.clone(), 1)),
            }
        }
        let names = self.persona_names();
        let mut labels = persona_labels(&names, counts.iter().map(|(id, _)| id.as_str()));
        counts
            .into_iter()
            .map(|(id, n)| {
                let label = labels.remove(&id).unwrap_or_else(|| id.clone());
                (id, label, n)
            })
            .collect()
    }

    /// Opens New task under the product's default persona if it is ready, else the active one (else the first ready
    /// one). The caller's `refresh_repos` then asks
    /// for that persona's repos.
    pub(super) fn open_spawn_dialog(&mut self) {
        let ready = |id: &str| {
            self.personas
                .iter()
                .flat_map(|l| &l.personas)
                .any(|p| p.ready && p.id == id)
        };
        let default = self
            .active_product()
            .and_then(|id| self.listed_products().iter().find(|p| p.id == id))
            .and_then(|p| p.default_persona.as_deref());
        let persona = match default
            .filter(|id| ready(id))
            .or(self.active_persona.as_deref())
        {
            Some(id) if ready(id) => id.to_string(),
            _ => self
                .personas
                .iter()
                .flat_map(|l| &l.personas)
                .find(|p| p.ready)
                .map(|p| p.id.clone())
                .unwrap_or_default(),
        };
        self.spawn.set_persona(persona);
        self.spawn.reset_choices();
        self.spawn.open = true;
    }

    /// The dialog's persona's remembered repos, while the dialog is open.
    pub(super) fn fetch_spawn_repos(&mut self) {
        if !self.spawn.open || self.spawn.persona.is_empty() {
            return;
        }
        let id = self.spawn.persona.clone();
        self.persona_calls.push(PersonaCall::Repos(id.clone()));
        let product = self.active_product().map(str::to_string);
        self.background(move |c| {
            AppResult::PersonaRepos(id.clone(), repos_in(c, product.as_deref()).map_err(|e| e.0))
        });
    }

    /// The active persona is one the list still has. Before the list loads, the saved one is taken on trust.
    fn active_listed(&self) -> bool {
        let Some(id) = &self.active_persona else {
            return false;
        };
        self.personas
            .as_ref()
            .is_none_or(|l| l.personas.iter().any(|p| &p.id == id))
    }

    /// The first persona to become ready is the active one, if none (still listed) is yet.
    fn adopt_active(&mut self, p: &Persona) {
        let listed = self
            .personas
            .as_ref()
            .is_some_and(|l| l.personas.iter().any(|l| l.id == p.id));
        if p.ready && listed && !self.active_listed() {
            self.set_active(Some(p.id.clone()));
        }
    }

    /// Sets and saves the active persona, like a Settings choice.
    pub(super) fn set_active(&mut self, id: Option<String>) {
        if self.active_persona == id {
            return;
        }
        self.active_persona = id.clone();
        let Some(path) = &self.settings_path else {
            return;
        };
        let (tx, ctx) = (self.results_tx.clone(), self.ctx.clone());
        SettingsWriter::shared().update(
            path,
            move |s| s.active_persona = id.clone(),
            move |saved| {
                let _ = tx.send(AppResult::SettingsSaved(
                    saved
                        .map(drop)
                        .map_err(|e| format!("Could not save the active persona: {e}")),
                ));
                ctx.request_repaint();
            },
        );
    }

    /// Replaces the listed persona with this newer copy; a persona not listed (deleted meanwhile) stays out.
    fn update_listed(&mut self, p: &Persona) {
        if let Some(listed) = self
            .personas
            .as_mut()
            .and_then(|l| l.personas.iter_mut().find(|l| l.id == p.id))
        {
            *listed = p.clone();
        }
    }

    pub(super) fn handle_persona_result(&mut self, r: AppResult) {
        match r {
            AppResult::Personas(Ok(list)) => {
                self.personas_load = PersonasLoad::Idle;
                self.personas_refetched
                    .extend(self.personas_refetching.drain(..));
                self.personas_error = None;
                let first_ready = list.personas.iter().find(|p| p.ready).cloned();
                if first_ready.is_none() && self.setup.is_none() {
                    // A draft left behind (a crash between the steps) is picked up rather than duplicated.
                    let mut setup = match list.personas.first() {
                        Some(draft) => PersonaSetup::resume(SetupMode::Wizard, draft.clone()),
                        None => PersonaSetup::new(SetupMode::Wizard),
                    };
                    setup.folders_step = self.folders_step;
                    self.setup = Some(setup);
                    self.check_due = f64::NEG_INFINITY;
                }
                self.personas = Some(list);
                if !self.active_listed() {
                    self.set_active(None);
                }
                if let Some(first) = first_ready {
                    self.adopt_active(&first);
                }
            }
            AppResult::Personas(Err(e)) => {
                self.personas_load = PersonasLoad::Failed;
                // Retried on the next event or snapshot, not per frame: the refetch runs only from those.
                self.personas_refetching.clear();
                // Retried on every poll: say it once, and again only if the reason changes.
                if self.personas_error.as_ref() != Some(&e) {
                    tracing::error!(notify = "error", "Could not load the personas: {e}");
                    self.personas_error = Some(e);
                }
            }
            AppResult::PersonaSaved(result) => {
                let Some(st) = self.setup.as_mut() else {
                    return;
                };
                st.busy = false;
                match result {
                    Ok(p) => {
                        st.error = None;
                        st.step = SetupStep::Login;
                        st.persona = Some(p.clone());
                        if let Some(list) = self.personas.as_mut() {
                            list.personas.push(p);
                        }
                        self.check_due = f64::NEG_INFINITY;
                    }
                    Err(e) => st.error = Some(e),
                }
            }
            AppResult::PersonaChecked(result) => {
                let checked = self.checking.take();
                match result {
                    Ok(p) => {
                        self.update_listed(&p);
                        self.adopt_active(&p);
                        if self.management.selected.as_ref() == Some(&p.id) {
                            self.management.check_error = None;
                        }
                        if let Some(st) = self
                            .setup
                            .as_mut()
                            .filter(|st| st.persona.as_ref().is_some_and(|s| s.id == p.id))
                        {
                            st.check_error = None;
                            st.persona = Some(p);
                        }
                    }
                    // An error carries no id: it is the check in flight, reported only if still the setup's.
                    Err(e) => {
                        if checked.is_some() && checked == self.management.selected {
                            self.management.check_error =
                                Some(format!("Could not check the logins: {e}"));
                        }
                        if let Some(st) = self
                            .setup
                            .as_mut()
                            .filter(|st| st.persona.as_ref().map(|p| &p.id) == checked.as_ref())
                        {
                            st.check_error = Some(format!("Could not check the logins: {e}"));
                        }
                    }
                }
            }
            AppResult::PersonaDeleted(id, Ok(())) => {
                if let Some(list) = self.personas.as_mut() {
                    list.personas.retain(|p| p.id != id);
                }
                if self.active_persona.as_ref() == Some(&id) {
                    self.set_active(None);
                }
            }
            AppResult::PersonaDeleted(_, Err(e)) => {
                tracing::error!(notify = "error", "Could not delete the persona: {e}");
                // It was taken off the list when Cancel asked for the delete; the daemon's list says what is left.
                self.personas_load = PersonasLoad::Idle;
                self.fetch_personas();
            }
            AppResult::PersonaUpdated(result, relogin) => match result {
                Ok(p) => {
                    self.update_listed(&p);
                    if self.management.selected.as_ref() == Some(&p.id) {
                        self.management.error = None;
                        self.management.github_auth = p.github_auth;
                        if relogin == Some(Login::Github) {
                            self.begin_github_login(p);
                        }
                    }
                }
                Err(e) => self.management.error = Some(format!("Could not save: {e}")),
            },
            AppResult::PersonaRepos(id, result) => {
                // One fetch can answer both: the dialog's persona may be the one Management shows.
                if self.spawn.persona == id {
                    match &result {
                        Ok(repos) => {
                            self.spawn.repos = repos.clone();
                            self.spawn.repos_error = None;
                        }
                        Err(e) => {
                            self.spawn.repos_error = Some(format!("Could not load the repos: {e}"))
                        }
                    }
                }
                if self.management.selected.as_ref() == Some(&id) {
                    self.management.repos = Some(result);
                }
            }
            AppResult::RepoFleetSaved(id, result) => {
                if let Ok(info) = &result {
                    // The dialog's copy and the board's list carry the roster too.
                    self.refresh_repos();
                    if self.management.selected.as_ref() == Some(&id) {
                        self.management.repo_saved(info);
                    }
                }
                // An answer for a persona no longer selected has no row to land on.
                if self.management.selected.as_ref() == Some(&id) {
                    let repo = self.management.fleet_saving.take();
                    if let (Err(e), Some(repo)) = (result, repo) {
                        self.management.fleet_error = Some((repo, e));
                    }
                }
            }
            // Both carry the persona they were started for: one for a persona no longer selected or set up is dropped.
            AppResult::LoginOpened(id, result) => {
                let for_setup = |st: &PersonaSetup| st.persona.as_ref().is_some_and(|p| p.id == id);
                if let Some(st) = self
                    .setup
                    .as_mut()
                    .filter(|st| st.opening.is_some() && for_setup(st))
                {
                    if let Some((which, script)) = st.opening.take() {
                        opened(
                            result,
                            which,
                            script,
                            (&mut st.claude_script, &mut st.gh_script),
                            &mut st.error,
                        );
                    }
                } else if self.management.selected.as_ref() == Some(&id) {
                    if let Some((which, script)) = self.management.opening.take() {
                        let m = &mut self.management;
                        opened(
                            result,
                            which,
                            script,
                            (&mut m.claude_script, &mut m.gh_script),
                            &mut m.error,
                        );
                    }
                }
            }
            AppResult::PatSaved(id, result) => {
                let for_setup = |st: &PersonaSetup| st.persona.as_ref().is_some_and(|p| p.id == id);
                // The wizard's token login is the one with `busy` set; otherwise it is Management's.
                if let Some(st) = self.setup.as_mut().filter(|st| st.busy && for_setup(st)) {
                    st.busy = false;
                    match result {
                        Ok(()) => {
                            st.error = None;
                            self.check_due = f64::NEG_INFINITY;
                        }
                        Err(e) => st.error = Some(e),
                    }
                } else if self.management.pat_busy && self.management.selected.as_ref() == Some(&id)
                {
                    self.management.pat_busy = false;
                    match result {
                        Ok(()) => {
                            self.management.error = None;
                            self.management.pat_open = false;
                            self.management.check_due = f64::NEG_INFINITY;
                            self.management.check_pending = Some(id);
                        }
                        Err(e) => self.management.error = Some(e),
                    }
                }
            }
            _ => unreachable!("not a persona result"),
        }
    }

    fn check(&mut self, id: String) {
        self.checking = Some(id.clone());
        self.persona_calls.push(PersonaCall::Check(id.clone()));
        self.background(move |c| AppResult::PersonaChecked(c.check_persona(&id).map_err(|e| e.0)));
    }

    /// Opens `script` in a terminal, found off the UI thread like every other call.
    fn open_login(&mut self, which: Login, script: String) {
        let Some(st) = self.setup.as_mut() else {
            return;
        };
        let Some(id) = st.persona.as_ref().map(|p| p.id.clone()) else {
            return;
        };
        st.opening = Some((which, script.clone()));
        st.error = None;
        self.spawn_terminal(id, which, script);
    }

    /// Runs the token login off the UI thread. The caller has already taken the token out of its field, so from here
    /// it lives only in the worker.
    fn spawn_pat_login(&mut self, p: Persona, gh: String, token: String) {
        self.persona_calls.push(PersonaCall::Pat(p.id.clone()));
        self.background(move |_| {
            let result = login::pat_login(&p, &gh, &token);
            drop(token);
            AppResult::PatSaved(p.id.clone(), result)
        });
    }

    fn spawn_terminal(&mut self, id: String, which: Login, script: String) {
        self.persona_calls.push(PersonaCall::Login(which));
        self.background(move |_| {
            let term = std::env::var("TERMINAL").ok();
            let Some(term) = login::terminal_argv(term.as_deref(), login::on_path) else {
                return AppResult::LoginOpened(id, Err(NO_TERMINAL.into()));
            };
            AppResult::LoginOpened(
                id,
                login::open_in_terminal(&term, &script).map_err(|e| e.to_string()),
            )
        });
    }

    fn setup_action(&mut self, action: SetupAction) {
        let (Some(st), Some(list)) = (self.setup.as_mut(), self.personas.as_ref()) else {
            return;
        };
        let persona = st.persona.clone();
        match action {
            SetupAction::Create(new) => {
                st.busy = true;
                st.error = None;
                self.persona_calls.push(PersonaCall::Create(new.clone()));
                self.background(move |c| {
                    AppResult::PersonaSaved(c.create_persona(&new).map_err(|e| e.0))
                });
            }
            SetupAction::OpenClaudeLogin => {
                if let Some(p) = persona {
                    let script =
                        login::claude_login_script(&p, &list.claude_command, &list.tasks_root);
                    self.open_login(Login::Claude, script);
                }
            }
            SetupAction::OpenGhLogin => {
                if let Some(p) = persona {
                    let script = login::gh_login_script(&p, &list.gh_command);
                    self.open_login(Login::Github, script);
                }
            }
            SetupAction::SubmitPat => {
                let Some(p) = persona else {
                    return;
                };
                let token = std::mem::take(&mut st.pat);
                st.busy = true;
                st.error = None;
                let gh = list.gh_command.clone();
                self.spawn_pat_login(p, gh, token);
            }
            SetupAction::Check => {
                if let Some(p) = persona {
                    if self.checking.is_none() {
                        self.check(p.id);
                    }
                    self.check_due = f64::NEG_INFINITY;
                }
            }
            SetupAction::Finish => {
                self.setup = None;
                self.fetch_personas();
            }
            SetupAction::Cancel => {
                let mode = st.mode;
                match (st.step, persona) {
                    // Resume belongs to Management, where the draft has its own Delete.
                    (SetupStep::Login, Some(p)) if mode != SetupMode::Resume => {
                        let id = p.id;
                        // Off the list at once, so a check still in flight cannot make it active.
                        if let Some(list) = self.personas.as_mut() {
                            list.personas.retain(|l| l.id != id);
                        }
                        if mode == SetupMode::Wizard {
                            st.back_to_identity();
                        } else {
                            self.setup = None;
                        }
                        self.persona_calls.push(PersonaCall::Delete(id.clone()));
                        self.background(move |c| {
                            AppResult::PersonaDeleted(
                                id.clone(),
                                c.delete_persona(&id).map_err(|e| e.0),
                            )
                        });
                    }
                    // The wizard has no way out: the board needs a ready persona.
                    _ if mode == SetupMode::Wizard => {}
                    _ => self.setup = None,
                }
            }
        }
    }

    /// The setup, as a modal over the board, and its checks while it waits on the Log in step.
    pub(super) fn persona_setup(&mut self, ctx: &egui::Context) {
        let Some(st) = self.setup.as_ref() else {
            return;
        };
        let now = ctx.input(|i| i.time);
        let checked = st
            .persona
            .as_ref()
            .filter(|_| st.step == SetupStep::Login)
            .map(|p| p.id.clone());
        if let Some(id) = checked {
            if self.checking.is_none() && now >= self.check_due {
                self.check_due = now + CHECK_EVERY.as_secs_f64();
                self.check(id);
            }
            ctx.request_repaint_after(CHECK_EVERY);
        }
        let (Some(st), Some(list)) = (self.setup.as_mut(), self.personas.as_ref()) else {
            return;
        };
        let wizard = st.mode == SetupMode::Wizard;
        let modal = egui::Modal::new(egui::Id::unique("persona_setup"))
            .frame(modal_frame(ctx))
            .show(ctx, |ui| persona_setup_ui(ui, st, list));
        let action = match modal.inner {
            None if !wizard && modal.should_close() => Some(SetupAction::Cancel),
            a => a,
        };
        if let Some(a) = action {
            self.setup_action(a);
        }
    }
}

impl FleetApp {
    /// Management's Personas tab (the others are drawn by the same call), its dialog and the calls it asks for.
    pub(super) fn management_view(&mut self, ui: &mut egui::Ui) {
        let now = ui.input(|i| i.time);
        self.keep_selection();
        self.keep_checking(ui, now);
        let tab_before = self.management_tab;
        let action = management::main(
            ui,
            &mut self.management_tab,
            self.personas.as_ref(),
            self.definitions.as_deref(),
            &mut self.management,
            &mut self.agent_config,
            &mut self.defs,
        );
        let tab_changed = self.management_tab != tab_before;
        if tab_changed && self.agent_config.load == ConfigLoad::Failed {
            self.fetch_agent_config();
        }
        if let Some(a) = action {
            self.management_action(a);
        }
        self.reset_definition_dialog(ui.ctx());
        if let Some(d) = self.delete_persona.as_mut() {
            match delete_persona_dialog(ui.ctx(), d) {
                None => {}
                Some(Confirm::Cancel) => self.delete_persona = None,
                Some(Confirm::Go) => {
                    let id = d.id.clone();
                    self.delete_persona = None;
                    self.delete(id);
                }
            }
        }
    }

    /// The selection is a persona the list has: the first one when it is none, or was deleted.
    fn keep_selection(&mut self) {
        let Some(list) = &self.personas else {
            return;
        };
        let listed = |id: &String| list.personas.iter().any(|p| &p.id == id);
        if self.management.selected.as_ref().is_some_and(listed) {
            return;
        }
        match list.personas.first().cloned() {
            Some(first) => self.select_persona(first),
            None => self.management.selected = None,
        }
    }

    /// Selecting a persona checks it and loads its remembered repos.
    fn select_persona(&mut self, p: Persona) {
        self.management.select(&p);
        self.management.check_pending = Some(p.id.clone());
        let id = p.id;
        self.persona_calls.push(PersonaCall::Repos(id.clone()));
        let product = self.active_product().map(str::to_string);
        self.background(move |c| {
            AppResult::PersonaRepos(id.clone(), repos_in(c, product.as_deref()).map_err(|e| e.0))
        });
    }

    /// Runs the check a selection or a login asked for once none is in flight, and asks the selected persona again
    /// every few seconds while a login is not ok: the user is logging in in another window.
    fn keep_checking(&mut self, ui: &egui::Ui, now: f64) {
        if self.checking.is_some() {
            return;
        }
        if let Some(id) = self.management.check_pending.take() {
            self.check(id);
            return;
        }
        let Some(p) = self
            .personas
            .as_ref()
            .zip(self.management.selected.as_ref())
            .and_then(|(l, id)| l.personas.iter().find(|p| &p.id == id))
        else {
            return;
        };
        let settled = p.ready
            && [&p.claude, &p.github]
                .into_iter()
                .all(|s| s.as_ref().is_some_and(|s| s.ok));
        if settled {
            return;
        }
        if now >= self.management.check_due {
            self.management.check_due = now + CHECK_EVERY.as_secs_f64();
            let id = p.id.clone();
            self.check(id);
        }
        ui.ctx().request_repaint_after(CHECK_EVERY);
    }

    fn delete(&mut self, id: String) {
        self.persona_calls.push(PersonaCall::Delete(id.clone()));
        self.background(move |c| {
            AppResult::PersonaDeleted(id.clone(), c.delete_persona(&id).map_err(|e| e.0))
        });
    }

    fn update_persona(&mut self, id: String, update: PersonaUpdate, relogin: Option<Login>) {
        self.management.error = None;
        self.persona_calls
            .push(PersonaCall::Update(id.clone(), update.clone()));
        self.background(move |c| {
            AppResult::PersonaUpdated(c.update_persona(&id, &update).map_err(|e| e.0), relogin)
        });
    }

    fn listed(&self, id: &str) -> Option<Persona> {
        self.personas
            .as_ref()?
            .personas
            .iter()
            .find(|p| p.id == id)
            .cloned()
    }

    /// Re-login to GitHub with the persona's auth type as it now is: a terminal for `gh`, the token field for a PAT.
    fn begin_github_login(&mut self, p: Persona) {
        match p.github_auth {
            GithubAuth::Gh => {
                let Some(list) = &self.personas else {
                    return;
                };
                let script = login::gh_login_script(&p, &list.gh_command);
                self.open_management_login(p.id.clone(), Login::Github, script);
            }
            GithubAuth::Pat => {
                self.management.pat.clear();
                self.management.pat_open = true;
                self.management.error = None;
            }
        }
    }

    fn open_management_login(&mut self, id: String, which: Login, script: String) {
        self.management.opening = Some((which, script.clone()));
        self.management.error = None;
        match which {
            Login::Claude => self.management.claude_script = None,
            Login::Github => self.management.gh_script = None,
        }
        self.spawn_terminal(id, which, script);
    }

    fn management_action(&mut self, action: ManagementAction) {
        match action {
            ManagementAction::NewPersona => {
                self.setup = Some(PersonaSetup::new(SetupMode::New));
            }
            ManagementAction::ResumePersona(id) => {
                if let Some(p) = self.listed(&id) {
                    self.setup = Some(PersonaSetup::resume(SetupMode::Resume, p));
                    self.check_due = f64::NEG_INFINITY;
                }
            }
            ManagementAction::DeletePersona(id) => {
                if let Some(p) = self.listed(&id) {
                    self.delete_persona = Some(DeletePersonaDialog {
                        id,
                        name: p.name,
                        focus_cancel: true,
                    });
                }
            }
            ManagementAction::RenamePersona(id, name) => {
                let update = PersonaUpdate {
                    name: Some(name),
                    commit_name: None,
                    commit_email: None,
                    github_auth: None,
                };
                self.update_persona(id, update, None);
            }
            ManagementAction::UpdateIdentity(id, commit_name, commit_email) => {
                let update = PersonaUpdate {
                    name: None,
                    commit_name: Some(commit_name),
                    commit_email: Some(commit_email),
                    github_auth: None,
                };
                self.update_persona(id, update, None);
            }
            ManagementAction::Relogin { id, which } => {
                let Some(p) = self.listed(&id) else {
                    return;
                };
                match which {
                    Login::Claude => {
                        let Some(list) = &self.personas else {
                            return;
                        };
                        let script =
                            login::claude_login_script(&p, &list.claude_command, &list.tasks_root);
                        self.open_management_login(id, Login::Claude, script);
                    }
                    Login::Github if self.management.github_auth != p.github_auth => {
                        let update = PersonaUpdate {
                            name: None,
                            commit_name: None,
                            commit_email: None,
                            github_auth: Some(self.management.github_auth),
                        };
                        self.update_persona(id, update, Some(Login::Github));
                    }
                    Login::Github => self.begin_github_login(p),
                }
            }
            ManagementAction::Check(id) => {
                if self.management.selected.as_ref() != Some(&id) {
                    if let Some(p) = self.listed(&id) {
                        self.select_persona(p);
                    }
                } else {
                    self.management.check_pending = Some(id);
                }
            }
            ManagementAction::SubmitPat(id) => {
                let (Some(p), Some(list)) = (self.listed(&id), &self.personas) else {
                    return;
                };
                let token = std::mem::take(&mut self.management.pat);
                self.management.pat_busy = true;
                self.management.error = None;
                let gh = list.gh_command.clone();
                self.spawn_pat_login(p, gh, token);
            }
            ManagementAction::SaveRepoFleet {
                persona,
                repo,
                fleet,
            } => {
                self.management.fleet_saving = Some(repo.clone());
                self.management.fleet_error = None;
                self.persona_calls.push(PersonaCall::RepoFleet(
                    persona.clone(),
                    repo.clone(),
                    fleet.clone(),
                ));
                let product = self.active_product().map(str::to_string);
                self.background(move |c| {
                    let saved = product
                        .ok_or_else(|| FleetError(PICK_PRODUCT.into()))
                        .and_then(|p| c.set_repo_fleet(&p, &repo, &fleet));
                    AppResult::RepoFleetSaved(persona.clone(), saved.map_err(|e| e.0))
                });
            }
            ManagementAction::SaveInstructions => {
                self.agent_config.instructions_error = None;
                let update = AgentConfigUpdate {
                    instructions: Some(self.agent_config.instructions.clone()),
                    settings: None,
                };
                self.save_agent_config(ConfigPart::Instructions, update);
            }
            ManagementAction::SaveSettings => {
                self.agent_config.settings_error = None;
                match self.agent_config.settings.to_settings() {
                    Ok(settings) => {
                        let update = AgentConfigUpdate {
                            instructions: None,
                            settings: Some(settings),
                        };
                        self.save_agent_config(ConfigPart::Settings, update);
                    }
                    Err(e) => self.agent_config.settings_error = Some(e),
                }
            }
            ManagementAction::RevertInstructions => self.agent_config.revert_instructions(),
            ManagementAction::RevertSettings => self.agent_config.revert_settings(),
            ManagementAction::OpenDefinition(kind, name) => self.open_definition(kind, name),
            ManagementAction::SaveDefinition(kind, name, text) => {
                self.save_definition(kind, name, text)
            }
            ManagementAction::ResetDefinition(kind, name) => self.ask_reset_definition(kind, name),
        }
    }

    pub(super) fn fetch_agent_config(&mut self) {
        if self.agent_config.load == ConfigLoad::Loaded
            || self.agent_config.load == ConfigLoad::Loading
        {
            return;
        }
        self.agent_config.load = ConfigLoad::Loading;
        self.agent_config_calls.push(AgentConfigCall::Load);
        self.background(|c| AppResult::AgentConfig(c.agent_config().map_err(|e| e.0)));
    }

    fn save_agent_config(&mut self, part: ConfigPart, cfg: AgentConfigUpdate) {
        self.agent_config.saving = Some(part);
        self.agent_config_calls
            .push(AgentConfigCall::Save(cfg.clone()));
        self.background(move |c| {
            AppResult::AgentConfigSaved(c.save_agent_config(&cfg).map_err(|e| e.0))
        });
    }

    pub(super) fn handle_agent_config_result(&mut self, r: AppResult) {
        let st = &mut self.agent_config;
        match r {
            AppResult::AgentConfig(Ok(cfg)) => st.loaded(cfg),
            // The editors stay usable and empty: saving a valid config is how a broken file is repaired.
            AppResult::AgentConfig(Err(e)) => {
                st.load = ConfigLoad::Failed;
                st.answered = true;
                st.load_error = Some(e);
            }
            AppResult::AgentConfigSaved(Ok(cfg)) => st.saved(cfg),
            AppResult::AgentConfigSaved(Err(e)) => match st.saving.take() {
                Some(ConfigPart::Settings) => st.settings_error = Some(e),
                Some(ConfigPart::Instructions) => st.instructions_error = Some(e),
                // No save is in flight: an answer to one that is gone.
                None => {}
            },
            _ => {}
        }
    }
}
