//! Management → Personas: the personas on the left, the selected one on the right.
use super::ManagementAction;
use crate::components::{
    caption, command_hint, login_status, pat_input, roster_chips, usable, Login,
};
use crate::theme::{sans, size, tokens, tone_color, Tokens, Weight};
use crate::widgets::{button, chip, list_row, warn_mark, ButtonVariant};
use eframe::egui::{
    self, vec2, Align, Frame, Layout, Margin, Panel, RichText, ScrollArea, Shape, Stroke, TextEdit,
    TextStyle, Ui,
};
use fleet_api::{DefInfo, DefKind, GithubAuth, Persona, PersonaList, RepoFleet, RepoInfo, Tone};
use std::collections::BTreeMap;

const LIST_WIDTH: f32 = 260.0;
const DETAIL_WIDTH: f32 = 520.0;

/// The tab's own state: which persona is selected and what its fields hold. The app owns it and the calls.
pub struct PersonasState {
    pub selected: Option<String>,
    pub name: String,
    pub commit_name: String,
    pub commit_email: String,
    /// The auth type the next GitHub Re-login uses; starts as the persona's own.
    pub github_auth: GithubAuth,
    /// The token typed and not yet sent. The app empties it as soon as Save token starts the call.
    pub pat: String,
    /// The token field is showing (a PAT re-login is under way).
    pub pat_open: bool,
    pub pat_busy: bool,
    pub error: Option<String>,
    pub check_error: Option<String>,
    /// The login whose terminal is being opened, with the script it runs.
    pub opening: Option<(Login, String)>,
    /// Scripts to run by hand, because no terminal was found to run them in.
    pub claude_script: Option<String>,
    pub gh_script: Option<String>,
    /// The selected persona's remembered repos; `None` while they load.
    pub repos: Option<Result<Vec<RepoInfo>, String>>,
    /// A check wanted while another was in flight.
    pub check_pending: Option<String>,
    /// When (in `ui.input` time) the selected persona is next checked while a login is not ok.
    pub check_due: f64,
    /// Each remembered repo's roster as being edited, by repo name; made from the repo when its
    /// row is first drawn.
    pub fleet: BTreeMap<String, FleetDraft>,
    /// The repo whose row's save is in flight.
    pub fleet_saving: Option<String>,
    /// The repo whose last save was refused, and why.
    pub fleet_error: Option<(String, String)>,
}

/// A repo row's draft. The roster keeps a name that no longer loads, so the row can show it struck; it is never sent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FleetDraft {
    pub roster: Vec<String>,
}

impl From<&RepoInfo> for FleetDraft {
    fn from(r: &RepoInfo) -> Self {
        FleetDraft {
            roster: r.roster.clone(),
        }
    }
}

impl FleetDraft {
    /// What Save sends: usable specialists only.
    fn to_fleet(&self, specialists: &[&str]) -> RepoFleet {
        RepoFleet {
            roster: self
                .roster
                .iter()
                .filter(|n| specialists.contains(&n.as_str()))
                .cloned()
                .collect(),
        }
    }
}

impl Default for PersonasState {
    fn default() -> Self {
        PersonasState {
            selected: None,
            name: String::new(),
            commit_name: String::new(),
            commit_email: String::new(),
            github_auth: GithubAuth::Gh,
            pat: String::new(),
            pat_open: false,
            pat_busy: false,
            error: None,
            check_error: None,
            opening: None,
            claude_script: None,
            gh_script: None,
            repos: None,
            check_pending: None,
            check_due: f64::NEG_INFINITY,
            fleet: BTreeMap::new(),
            fleet_saving: None,
            fleet_error: None,
        }
    }
}

impl PersonasState {
    /// Selects `p`: the fields take its values, and what belonged to the persona before goes.
    pub fn select(&mut self, p: &Persona) {
        self.selected = Some(p.id.clone());
        self.name = p.name.clone();
        self.commit_name = p.commit_name.clone();
        self.commit_email = p.commit_email.clone();
        self.github_auth = p.github_auth;
        self.pat.clear();
        self.pat_open = false;
        self.pat_busy = false;
        self.error = None;
        self.check_error = None;
        self.opening = None;
        self.claude_script = None;
        self.gh_script = None;
        self.repos = None;
        self.check_due = f64::NEG_INFINITY;
        self.fleet.clear();
        self.fleet_saving = None;
        self.fleet_error = None;
    }

    /// A save of `r`'s row landed: the list and the row's draft take the stored values.
    pub fn repo_saved(&mut self, r: &RepoInfo) {
        if let Some(Ok(repos)) = &mut self.repos {
            if let Some(listed) = repos.iter_mut().find(|l| l.name == r.name) {
                *listed = r.clone();
            }
        }
        self.fleet.insert(r.name.clone(), FleetDraft::from(r));
        self.fleet_error = None;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Ok,
    Warn,
    Draft,
}

fn mark_of(p: &Persona) -> Mark {
    let broken = [&p.claude, &p.github].into_iter().flatten().any(|s| !s.ok);
    match (p.ready, broken) {
        (false, _) => Mark::Draft,
        (true, true) => Mark::Warn,
        (true, false) => Mark::Ok,
    }
}

/// The text a row is named by for assistive tech and tests; the mark itself is painted.
fn row_label(p: &Persona) -> String {
    let mark = match mark_of(p) {
        Mark::Ok => "✓",
        Mark::Warn => "⚠",
        Mark::Draft => "draft",
    };
    format!("{} {mark}", p.name)
}

/// ✓ / ⚠ painted (the bundled fonts carry neither), or the word `draft`.
fn paint_mark(ui: &mut Ui, mark: Mark, pal: &Tokens) {
    let dark = ui.visuals().dark_mode;
    match mark {
        Mark::Draft => {
            ui.label(RichText::new("draft").size(size::SMALL).color(pal.text_low));
        }
        Mark::Ok => {
            let (rect, r) = ui.allocate_exact_size(vec2(14.0, 14.0), egui::Sense::hover());
            let c = rect.center();
            ui.painter().add(Shape::line(
                vec![
                    c + vec2(-4.0, 0.5),
                    c + vec2(-1.5, 3.5),
                    c + vec2(4.5, -3.5),
                ],
                Stroke::new(1.8, tone_color(Tone::Done, dark)),
            ));
            r.on_hover_text("Both logins work");
        }
        Mark::Warn => {
            warn_mark(ui).on_hover_text("A login needs attention");
        }
    }
}

pub(super) fn personas(
    ui: &mut Ui,
    list: Option<&PersonaList>,
    defs: Option<&[DefInfo]>,
    st: &mut PersonasState,
) -> Option<ManagementAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    let Some(list) = list else {
        Frame::new()
            .inner_margin(Margin::symmetric(22, 18))
            .show(ui, |ui| {
                ui.label(RichText::new("Loading…").color(p.text_low));
            });
        return None;
    };
    Panel::left("personas_list")
        .exact_size(LIST_WIDTH)
        .resizable(false)
        .frame(Frame::new().fill(p.surface).inner_margin(Margin::same(12)))
        .show(ui, |ui| {
            Panel::bottom("personas_new")
                .frame(Frame::new().fill(p.surface).inner_margin(Margin {
                    top: 8,
                    ..Margin::ZERO
                }))
                .show(ui, |ui| {
                    ui.with_layout(Layout::top_down_justified(Align::Center), |ui| {
                        if button(ui, "+ New persona", ButtonVariant::Default).clicked() {
                            action = Some(ManagementAction::NewPersona);
                        }
                    });
                });
            egui::CentralPanel::default()
                .frame(Frame::new().fill(p.surface))
                .show(ui, |ui| {
                    ScrollArea::vertical().show(ui, |ui| {
                        for persona in &list.personas {
                            let selected = st.selected.as_deref() == Some(&persona.id);
                            let row = list_row(ui, &row_label(persona), selected, |ui| {
                                ui.horizontal(|ui| {
                                    paint_mark(ui, mark_of(persona), &p);
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&persona.name).color(p.text_high),
                                        )
                                        .truncate(),
                                    );
                                });
                            });
                            if row.clicked() {
                                action = Some(ManagementAction::Check(persona.id.clone()));
                            }
                        }
                    });
                });
        });
    egui::CentralPanel::default()
        .frame(
            Frame::new()
                .fill(p.bg)
                .inner_margin(Margin::symmetric(22, 18)),
        )
        .show(ui, |ui| {
            let selected = st
                .selected
                .as_ref()
                .and_then(|id| list.personas.iter().find(|l| &l.id == id));
            match selected {
                None => {
                    ui.label(
                        RichText::new("No persona selected.")
                            .size(size::SMALL)
                            .color(p.text_low),
                    );
                }
                Some(persona) => {
                    ScrollArea::vertical().show(ui, |ui| {
                        ui.set_max_width(DETAIL_WIDTH);
                        if let Some(a) = detail(ui, persona, defs, st) {
                            action = Some(a);
                        }
                    });
                }
            }
        });
    action
}

/// A captioned single-line field; returns whether its text differs from `saved` and is not blank.
fn field(ui: &mut Ui, label: &str, text: &mut String) {
    let label = caption(ui, label);
    ui.add_space(4.0);
    ui.add(TextEdit::singleline(text).desired_width(f32::INFINITY))
        .labelled_by(label.id);
    ui.add_space(8.0);
}

fn changed(edit: &str, saved: &str) -> bool {
    let edit = edit.trim();
    !edit.is_empty() && edit != saved
}

fn projects_text(n: u32) -> String {
    format!("{n} {}", if n == 1 { "project" } else { "projects" })
}

pub(super) fn note(ui: &mut Ui, text: &str) {
    let p = tokens(ui.visuals().dark_mode);
    ui.add(egui::Label::new(RichText::new(text).size(size::SMALL).color(p.text_low)).wrap());
}

pub(super) fn error_line(ui: &mut Ui, text: &str) {
    let p = tokens(ui.visuals().dark_mode);
    ui.add_space(4.0);
    ui.add(egui::Label::new(RichText::new(text).size(size::SMALL).color(p.danger)).wrap());
}

/// A login's title row, with the `relogin` button (named for its login) on the right.
fn login_head(ui: &mut Ui, name: &str, relogin: Option<&str>) -> bool {
    let p = tokens(ui.visuals().dark_mode);
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(name)
                .font(sans(ui, size::BODY, Weight::Medium))
                .color(p.text_high),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(label) = relogin {
                clicked = button(ui, label, ButtonVariant::Default).clicked();
            }
        });
    });
    clicked
}

fn detail(
    ui: &mut Ui,
    persona: &Persona,
    defs: Option<&[DefInfo]>,
    st: &mut PersonasState,
) -> Option<ManagementAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    ui.label(
        RichText::new(&persona.name)
            .text_style(TextStyle::Heading)
            .color(p.text_high),
    );
    ui.add_space(4.0);
    note(ui, &format!("Projects: {}", persona.projects));
    ui.add_space(12.0);

    if !persona.ready {
        action = draft(ui, persona);
        return action;
    }

    field(ui, "Name", &mut st.name);
    let rename = changed(&st.name, &persona.name);
    if ui
        .add_enabled_ui(rename, |ui| button(ui, "Save name", ButtonVariant::Default))
        .inner
        .clicked()
        && rename
    {
        action = Some(ManagementAction::RenamePersona(
            persona.id.clone(),
            st.name.trim().into(),
        ));
    }
    ui.add_space(12.0);

    field(ui, "Commit name", &mut st.commit_name);
    field(ui, "Commit email", &mut st.commit_email);
    let identity = changed(&st.commit_name, &persona.commit_name)
        || changed(&st.commit_email, &persona.commit_email);
    let valid = !st.commit_name.trim().is_empty() && !st.commit_email.trim().is_empty();
    if ui
        .add_enabled_ui(identity && valid, |ui| {
            button(ui, "Save identity", ButtonVariant::Default)
        })
        .inner
        .clicked()
        && identity
        && valid
    {
        action = Some(ManagementAction::UpdateIdentity(
            persona.id.clone(),
            st.commit_name.trim().into(),
            st.commit_email.trim().into(),
        ));
    }
    ui.add_space(16.0);

    if login_head(ui, "Claude", Some("Re-login to Claude")) {
        action = Some(ManagementAction::Relogin {
            id: persona.id.clone(),
            which: Login::Claude,
        });
    }
    login_status(ui, persona.claude.as_ref());
    if let Some(script) = &st.claude_script {
        ui.add_space(4.0);
        command_hint(ui, "No terminal found. Run this in one:", script);
    }
    ui.add_space(14.0);

    if login_head(
        ui,
        &format!("GitHub · {}", persona.host),
        Some("Re-login to GitHub"),
    ) {
        action = Some(ManagementAction::Relogin {
            id: persona.id.clone(),
            which: Login::Github,
        });
    }
    login_status(ui, persona.github.as_ref());
    ui.add_space(6.0);
    caption(ui, "GitHub auth");
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for (auth, label) in [
            (GithubAuth::Gh, "gh login"),
            (GithubAuth::Pat, "Personal access token"),
        ] {
            if chip(ui, label, st.github_auth == auth).clicked() {
                st.github_auth = auth;
            }
        }
    });
    if st.github_auth != persona.github_auth {
        note(ui, "Re-login switches to this.");
    }
    if st.pat_open {
        ui.add_space(8.0);
        if let Some(a) = pat_field(ui, persona, st) {
            action = Some(a);
        }
    }
    if let Some(script) = &st.gh_script {
        ui.add_space(4.0);
        command_hint(ui, "No terminal found. Run this in one:", script);
    }
    for e in [&st.error, &st.check_error].into_iter().flatten() {
        error_line(ui, e);
    }
    ui.add_space(16.0);

    ui.label(
        RichText::new("Remembered repos")
            .font(sans(ui, size::BODY, Weight::Medium))
            .color(p.text_high),
    );
    ui.add_space(4.0);
    match &st.repos {
        None => note(ui, "Loading…"),
        Some(Err(e)) => error_line(ui, &format!("Could not load the repos: {e}")),
        Some(Ok(repos)) if repos.is_empty() => note(ui, "None yet."),
        Some(Ok(repos)) => {
            for r in repos {
                let draft = st.fleet.entry(r.name.clone()).or_insert_with(|| r.into());
                let error = st
                    .fleet_error
                    .as_ref()
                    .filter(|(repo, _)| *repo == r.name)
                    .map(|(_, e)| e.as_str());
                let saving = st.fleet_saving.as_ref() == Some(&r.name);
                if let Some(fleet) = repo_row(ui, r, defs, draft, saving, error) {
                    action = Some(ManagementAction::SaveRepoFleet {
                        persona: persona.id.clone(),
                        repo: r.name.clone(),
                        fleet,
                    });
                }
                ui.add_space(12.0);
            }
        }
    }
    ui.add_space(20.0);

    ui.horizontal(|ui| {
        let empty = persona.projects == 0;
        let delete = ui
            .add_enabled_ui(empty, |ui| button(ui, "Delete", ButtonVariant::QuietDanger))
            .inner
            .on_disabled_hover_text("Delete or archive its projects first");
        if delete.clicked() && empty {
            action = Some(ManagementAction::DeletePersona(persona.id.clone()));
        }
        if !empty {
            note(ui, &format!("Used by {}", projects_text(persona.projects)));
        }
    });
    action
}

/// One remembered repo: its name, the specialists an orchestrated project may use, and Save.
/// Returns what to save when Save is clicked. Without the definitions the roster cannot be checked, so Save waits.
fn repo_row(
    ui: &mut Ui,
    r: &RepoInfo,
    defs: Option<&[DefInfo]>,
    draft: &mut FleetDraft,
    saving: bool,
    error: Option<&str>,
) -> Option<RepoFleet> {
    let p = tokens(ui.visuals().dark_mode);
    let mut save = None;
    ui.push_id(("repo_fleet", &r.name), |ui| {
        ui.add(
            egui::Label::new(
                RichText::new(&r.name)
                    .font(crate::theme::mono(size::CODE))
                    .color(p.text),
            )
            .truncate(),
        )
        .on_hover_text(&r.location);
        ui.add_space(4.0);
        let specialists = defs.map(|d| usable(d, DefKind::Specialist));
        caption(ui, "Specialists");
        ui.add_space(4.0);
        match &specialists {
            Some(names) => {
                let stale: Vec<String> = draft
                    .roster
                    .iter()
                    .filter(|n| !names.contains(&n.as_str()))
                    .cloned()
                    .collect();
                let stale: Vec<&str> = stale.iter().map(String::as_str).collect();
                roster_chips(ui, names, &mut draft.roster, &stale);
            }
            None => note(ui, "The specialists have not loaded."),
        }
        ui.add_space(6.0);
        let fleet = specialists.as_deref().map(|names| draft.to_fleet(names));
        let saved = RepoFleet {
            roster: r.roster.clone(),
        };
        let dirty = fleet.as_ref().is_some_and(|f| *f != saved);
        let enabled = dirty && !saving;
        if ui
            .add_enabled_ui(enabled, |ui| button(ui, "Save", ButtonVariant::Default))
            .inner
            .clicked()
            && enabled
        {
            save = fleet;
        }
        if let Some(e) = error {
            error_line(ui, e);
        }
    });
    save
}

/// A draft's pane: what it was set up with, and Resume or Delete.
fn draft(ui: &mut Ui, persona: &Persona) -> Option<ManagementAction> {
    let mut action = None;
    note(ui, "Setup was not finished.");
    ui.add_space(8.0);
    for (label, value) in [
        ("Commit name", &persona.commit_name),
        ("Commit email", &persona.commit_email),
        ("GitHub host", &persona.host),
    ] {
        caption(ui, label);
        ui.label(value);
        ui.add_space(6.0);
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if button(ui, "Resume", ButtonVariant::Primary).clicked() {
            action = Some(ManagementAction::ResumePersona(persona.id.clone()));
        }
        if button(ui, "Delete", ButtonVariant::QuietDanger).clicked() {
            action = Some(ManagementAction::DeletePersona(persona.id.clone()));
        }
    });
    action
}

/// The masked token field with Save token, as in the wizard.
fn pat_field(ui: &mut Ui, persona: &Persona, st: &mut PersonasState) -> Option<ManagementAction> {
    pat_input(
        ui,
        &mut st.pat,
        egui::Id::unique("management_pat"),
        st.pat_busy,
    )
    .then(|| ManagementAction::SubmitPat(persona.id.clone()))
}
