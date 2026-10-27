//! Setting a persona up: its identity, then its Claude and GitHub logins. The first-run wizard's steps 2 and 3, and
//! Management's New persona / Resume. The app owns the modal around it and every call it asks for.
use super::command_hint;
use crate::theme::{sans, size, tokens, tone_color, Weight};
use crate::widgets::{button, chip, dot, fui, ButtonVariant};
use eframe::egui::{self, Align, Layout, RichText, TextEdit, TextStyle, Ui};
use fleet_api::{GithubAuth, LoginState, NewPersona, Persona, PersonaList, Tone};

/// What the setup is for. The wizard cannot be dismissed: the board needs a ready persona.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupMode {
    Wizard,
    New,
    /// A draft left behind (by a crash, say), opened at its Log in step.
    Resume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupStep {
    Identity,
    Login,
}

/// One of the two logins a persona needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Login {
    Claude,
    Github,
}

pub struct PersonaSetup {
    pub mode: SetupMode,
    pub step: SetupStep,
    /// The wizard's first step, the folders, was shown. An upgrade skips it (the config exists), and the wizard
    /// then counts from 1.
    pub folders_step: bool,
    /// What the Identity step's fields hold; kept after a Cancel on Log in so the user can fix and go again.
    pub draft: NewPersona,
    /// The persona the daemon created, as its last check left it.
    pub persona: Option<Persona>,
    /// The token typed and not yet sent. The app empties it as soon as Save token starts the call.
    pub pat: String,
    /// A create or a token login is in flight.
    pub busy: bool,
    pub error: Option<String>,
    /// Why the last login check failed; the next one that works clears it.
    pub check_error: Option<String>,
    /// The login whose terminal is being opened, with the script it runs.
    pub opening: Option<(Login, String)>,
    /// Scripts to run by hand, because no terminal was found to run them in.
    pub claude_script: Option<String>,
    pub gh_script: Option<String>,
}

impl PersonaSetup {
    pub fn new(mode: SetupMode) -> PersonaSetup {
        PersonaSetup {
            mode,
            step: SetupStep::Identity,
            folders_step: true,
            draft: NewPersona {
                name: String::new(),
                host: String::new(),
                commit_name: String::new(),
                commit_email: String::new(),
                github_auth: GithubAuth::Gh,
            },
            persona: None,
            pat: String::new(),
            busy: false,
            error: None,
            check_error: None,
            opening: None,
            claude_script: None,
            gh_script: None,
        }
    }

    /// Opens on the Log in step of a persona that already exists.
    pub fn resume(mode: SetupMode, p: Persona) -> PersonaSetup {
        let mut st = PersonaSetup::new(mode);
        st.draft = NewPersona {
            name: p.name.clone(),
            host: p.host.clone(),
            commit_name: p.commit_name.clone(),
            commit_email: p.commit_email.clone(),
            github_auth: p.github_auth,
        };
        st.persona = Some(p);
        st.step = SetupStep::Login;
        st
    }

    /// Back to the Identity step, the typed values kept; what belonged to the deleted draft goes.
    pub fn back_to_identity(&mut self) {
        self.step = SetupStep::Identity;
        self.persona = None;
        self.pat.clear();
        self.busy = false;
        self.check_error = None;
        self.opening = None;
        self.claude_script = None;
        self.gh_script = None;
    }

    /// The draft as the daemon gets it: trimmed, and an empty host is the placeholder's `github.com`.
    fn submitted(&self) -> NewPersona {
        let d = &self.draft;
        let host = match d.host.trim() {
            "" => "github.com",
            h => h,
        };
        NewPersona {
            name: d.name.trim().into(),
            host: host.into(),
            commit_name: d.commit_name.trim().into(),
            commit_email: d.commit_email.trim().into(),
            github_auth: d.github_auth,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SetupAction {
    Create(NewPersona),
    OpenClaudeLogin,
    OpenGhLogin,
    SubmitPat,
    Check,
    Finish,
    Cancel,
}

const WIDTH: f32 = 480.0;

/// The token field's id, global so its edit state (and undo history) can be found.
pub fn pat_field_id() -> egui::Id {
    egui::Id::unique("persona_setup_pat")
}

/// The masked token field with Save token, shared by the wizard and Management's re-login. Returns true when Save
/// token was clicked; the caller takes the token out of `pat` and starts the call.
pub fn pat_input(ui: &mut Ui, pat: &mut String, id: egui::Id, busy: bool) -> bool {
    let label = caption(ui, "Personal access token");
    ui.add_space(4.0);
    let can_save = !busy && !pat.trim().is_empty();
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let save = ui
                .add_enabled_ui(can_save, |ui| {
                    button(ui, "Save token", ButtonVariant::Default)
                })
                .inner;
            clicked = save.clicked() && can_save;
            ui.add(
                TextEdit::singleline(pat)
                    .id(id)
                    .password(true)
                    .desired_width(ui.available_width()),
            )
            .labelled_by(label.id);
        });
    });
    // gh checks a classic token's scopes at login; a fine-grained one only fails later, at the first clone or PR.
    ui.add_space(4.0);
    ui.label(
        RichText::new(
            "Fine-grained token: give it the repos this persona works on, with Contents and Pull requests: Read \
             and write (Metadata: Read is added for you). A classic token needs the repo and read:org scopes.",
        )
        .font(sans(ui, size::SMALL, Weight::Regular))
        .color(tokens(ui.visuals().dark_mode).text_low),
    );
    // After the field has drawn this frame: its undo history keeps the typed token, and Ctrl+Z after Save token
    // would bring it back.
    if clicked {
        if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), id) {
            state.clear_undoer();
            state.store(ui.ctx(), id);
        }
    }
    clicked
}

pub fn caption(ui: &mut Ui, text: &str) -> egui::Response {
    let low = tokens(ui.visuals().dark_mode).text_low;
    fui::mono_label(ui, text, low)
}

fn text_field(ui: &mut Ui, label: &str, text: &mut String, hint: &str) {
    let label = caption(ui, label);
    ui.add_space(4.0);
    ui.add(
        TextEdit::singleline(text)
            .hint_text(hint)
            .desired_width(f32::INFINITY),
    )
    .labelled_by(label.id);
    ui.add_space(8.0);
}

fn title(st: &PersonaSetup) -> String {
    let (skipped, total) = if st.folders_step { (1, 3) } else { (0, 2) };
    match (st.mode, st.step) {
        (SetupMode::Wizard, SetupStep::Identity) => {
            format!("Step {} of {total} · Your first persona", 1 + skipped)
        }
        (SetupMode::Wizard, SetupStep::Login) => {
            format!("Step {} of {total} · Log in", 2 + skipped)
        }
        (_, SetupStep::Identity) => "New persona".into(),
        (_, SetupStep::Login) => "Log in".into(),
    }
}

/// The current step's form. Returns what the user asked for; the app makes the calls and feeds the answers back
/// into `st`.
pub fn persona_setup_ui(
    ui: &mut Ui,
    st: &mut PersonaSetup,
    list: &PersonaList,
) -> Option<SetupAction> {
    let p = tokens(ui.visuals().dark_mode);
    ui.set_width(WIDTH);
    ui.label(
        RichText::new(title(st))
            .text_style(TextStyle::Heading)
            .color(p.text_high),
    );
    ui.add_space(10.0);
    match st.step {
        SetupStep::Identity => identity(ui, st),
        SetupStep::Login => logins(ui, st, list),
    }
}

fn identity(ui: &mut Ui, st: &mut PersonaSetup) -> Option<SetupAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    let d = &mut st.draft;
    text_field(ui, "Name", &mut d.name, "");
    text_field(ui, "Commit name", &mut d.commit_name, "");
    text_field(ui, "Commit email", &mut d.commit_email, "");
    text_field(ui, "GitHub host", &mut d.host, "github.com");
    caption(ui, "GitHub auth");
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for (auth, label) in [
            (GithubAuth::Gh, "gh login"),
            (GithubAuth::Pat, "Personal access token"),
        ] {
            if chip(ui, label, d.github_auth == auth).clicked() {
                d.github_auth = auth;
            }
        }
    });
    if let Some(e) = &st.error {
        ui.add_space(8.0);
        ui.add(egui::Label::new(RichText::new(e).size(size::SMALL).color(p.danger)).wrap());
    }
    ui.add_space(12.0);
    let filled = [&d.name, &d.commit_name, &d.commit_email]
        .iter()
        .all(|f| !f.trim().is_empty());
    let ready = filled && !st.busy;
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let next = ui
            .add_enabled_ui(ready, |ui| button(ui, "Next", ButtonVariant::Primary))
            .inner;
        if next.clicked() && ready {
            action = Some(SetupAction::Create(st.submitted()));
        }
        if st.mode != SetupMode::Wizard && button(ui, "Cancel", ButtonVariant::Default).clicked() {
            action = Some(SetupAction::Cancel);
        }
    });
    action
}

/// ✓ account / ✗ detail, the mark painted: no bundled font carries those glyphs.
pub fn login_status(ui: &mut Ui, state: Option<&LoginState>) {
    let p = tokens(ui.visuals().dark_mode);
    let dark = ui.visuals().dark_mode;
    let (tone, text) = match state {
        None => (Tone::Muted, "Not checked yet".to_string()),
        Some(s) if s.ok => (
            Tone::Done,
            s.account.clone().unwrap_or_else(|| "logged in".into()),
        ),
        Some(s) => (
            Tone::Attention,
            s.detail.clone().unwrap_or_else(|| "not logged in".into()),
        ),
    };
    ui.horizontal(|ui| {
        dot(ui, tone_color(tone, dark), 8.0);
        ui.add(egui::Label::new(RichText::new(text).size(size::SMALL).color(p.text)).truncate());
    });
}

/// A login's title row, with `Log in…` on the right when `login` is true.
fn row_head(ui: &mut Ui, name: &str, login: bool) -> bool {
    let p = tokens(ui.visuals().dark_mode);
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(name)
                .font(sans(ui, size::BODY, Weight::Medium))
                .color(p.text_high),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            clicked = login && button(ui, "Log in…", ButtonVariant::Default).clicked();
        });
    });
    clicked
}

fn by_hand(ui: &mut Ui, script: Option<&String>) {
    if let Some(script) = script {
        ui.add_space(4.0);
        command_hint(ui, "No terminal found. Run this in one:", script);
    }
}

fn logins(ui: &mut Ui, st: &mut PersonaSetup, list: &PersonaList) -> Option<SetupAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    let persona = st.persona.as_ref()?;
    ui.label(
        RichText::new(format!(
            "Each opens a terminal running the real CLI. Claude starts in {}.",
            list.tasks_root
        ))
        .size(size::SMALL)
        .color(p.text_low),
    );
    ui.add_space(10.0);

    if row_head(ui, "Claude", true) {
        action = Some(SetupAction::OpenClaudeLogin);
    }
    ui.label(
        RichText::new("Trust the folder (second option), then type /login")
            .size(size::SMALL)
            .color(p.text_low),
    );
    login_status(ui, persona.claude.as_ref());
    by_hand(ui, st.claude_script.as_ref());
    ui.add_space(12.0);

    let gh = persona.github_auth == GithubAuth::Gh;
    if row_head(ui, &format!("GitHub · {}", persona.host), gh) {
        action = Some(SetupAction::OpenGhLogin);
    }
    if !gh && pat_input(ui, &mut st.pat, pat_field_id(), st.busy) {
        action = Some(SetupAction::SubmitPat);
    }
    login_status(ui, persona.github.as_ref());
    by_hand(ui, st.gh_script.as_ref());

    for e in [&st.error, &st.check_error].into_iter().flatten() {
        ui.add_space(8.0);
        ui.add(egui::Label::new(RichText::new(e).size(size::SMALL).color(p.danger)).wrap());
    }
    ui.add_space(12.0);
    let ready = persona.ready;
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let finish = ui
            .add_enabled_ui(ready, |ui| button(ui, "Finish", ButtonVariant::Primary))
            .inner;
        if finish.clicked() && ready {
            action = Some(SetupAction::Finish);
        }
        // A ready persona may already be the active one: Cancel would delete it from under the app.
        if !ready && button(ui, "Cancel", ButtonVariant::Default).clicked() {
            action = Some(SetupAction::Cancel);
        }
        if button(ui, "Check now", ButtonVariant::Ghost).clicked() {
            action = Some(SetupAction::Check);
        }
    });
    action
}
