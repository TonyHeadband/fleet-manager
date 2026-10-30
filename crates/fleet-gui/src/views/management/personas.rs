//! Management → Personas: the personas on the left, the selected one on the right.
use super::ManagementAction;
use crate::components::{
    can_log_in, caption, command_hint, login_status, pat_input, usable, Login,
};
use crate::theme::mono;
use crate::theme::{sans, size, tokens, tone_color, Tokens, Weight};
use crate::widgets::{button, chip, list_row, warn_mark, ButtonVariant};
use eframe::egui::{
    self, vec2, Align, Frame, Layout, Margin, Panel, RichText, ScrollArea, Shape, Stroke, TextEdit,
    TextStyle, Ui,
};
use fleet_api::{DefInfo, DefKind, GithubAuth, Persona, PersonaList, Tone};
use std::collections::BTreeMap;

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
    /// A check wanted while another was in flight.
    pub check_pending: Option<String>,
    /// When (in `ui.input` time) the selected persona is next checked while a login is not ok.
    pub check_due: f64,
    /// The nicknames as being edited, by specialist name; blank means none.
    pub nicknames: BTreeMap<String, String>,
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
            check_pending: None,
            check_due: f64::NEG_INFINITY,
            nicknames: BTreeMap::new(),
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
        self.nicknames = p.nicknames.clone();
        self.pat.clear();
        self.pat_open = false;
        self.pat_busy = false;
        self.error = None;
        self.check_error = None;
        self.opening = None;
        self.claude_script = None;
        self.gh_script = None;
        self.check_due = f64::NEG_INFINITY;
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
            .inner_margin(Margin::symmetric(
                crate::theme::layout::PAGE_X as i8,
                crate::theme::layout::PAGE_Y as i8,
            ))
            .show(ui, |ui| {
                ui.label(RichText::new("Loading…").color(p.text_low));
            });
        return None;
    };
    Panel::left("personas_list")
        .exact_size(crate::theme::layout::MANAGEMENT_LIST)
        .resizable(false)
        .frame(
            Frame::new()
                .fill(p.surface)
                .inner_margin(Margin::same(crate::theme::management::LIST_MARGIN as i8)),
        )
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
        .frame(Frame::new().fill(p.bg).inner_margin(Margin::symmetric(
            crate::theme::layout::PAGE_X as i8,
            crate::theme::layout::PAGE_Y as i8,
        )))
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

fn tasks_text(n: u32) -> String {
    format!("{n} {}", if n == 1 { "task" } else { "tasks" })
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
    note(ui, &format!("Tasks: {}", persona.tasks));
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

    if let Some(a) = nicknames_section(ui, persona, defs, st) {
        action = Some(a);
    }
    ui.add_space(16.0);

    let claude = can_log_in(persona.claude.as_ref()).then_some("Re-login to Claude");
    if login_head(ui, "Claude", claude) {
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

    let github = can_log_in(persona.github.as_ref()).then_some("Re-login to GitHub");
    if login_head(ui, &format!("GitHub · {}", persona.host), github) {
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

    ui.horizontal(|ui| {
        let empty = persona.tasks == 0;
        let delete = ui
            .add_enabled_ui(empty, |ui| button(ui, "Delete", ButtonVariant::QuietDanger))
            .inner
            .on_disabled_hover_text("Delete or archive its tasks first");
        if delete.clicked() && empty {
            action = Some(ManagementAction::DeletePersona(persona.id.clone()));
        }
        if !empty {
            note(ui, &format!("Used by {}", tasks_text(persona.tasks)));
        }
    });
    action
}

/// The nicknames as Save sends them: trimmed, blank ones left out.
fn cleaned(nicknames: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    nicknames
        .iter()
        .map(|(k, v)| (k.clone(), v.trim().to_string()))
        .filter(|(_, v)| !v.is_empty())
        .collect()
}

/// A nickname field per specialist that loads, then Save nicknames. A nickname kept for a specialist that no
/// longer loads stays stored, out of sight, until it loads again.
fn nicknames_section(
    ui: &mut Ui,
    persona: &Persona,
    defs: Option<&[DefInfo]>,
    st: &mut PersonasState,
) -> Option<ManagementAction> {
    let p = tokens(ui.visuals().dark_mode);
    caption(ui, "Specialist nicknames");
    ui.add_space(2.0);
    note(
        ui,
        "Shown in the app wherever this persona's tasks name a specialist. Agents and logs keep the real names.",
    );
    ui.add_space(4.0);
    let Some(defs) = defs else {
        note(ui, "The specialists have not loaded.");
        return None;
    };
    egui::Grid::new(("nicknames", &persona.id))
        .num_columns(2)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            for name in usable(defs, DefKind::Specialist) {
                let label = ui.label(RichText::new(name).font(mono(size::META)).color(p.text));
                let text = st.nicknames.entry(name.to_string()).or_default();
                ui.add(
                    TextEdit::singleline(text)
                        .hint_text("none")
                        .char_limit(32)
                        .desired_width(220.0),
                )
                .labelled_by(label.id);
                ui.end_row();
            }
        });
    ui.add_space(6.0);
    let send = cleaned(&st.nicknames);
    let dirty = send != persona.nicknames;
    let mut action = None;
    if ui
        .add_enabled_ui(dirty, |ui| {
            button(ui, "Save nicknames", ButtonVariant::Default)
        })
        .inner
        .clicked()
        && dirty
    {
        action = Some(ManagementAction::SaveNicknames(persona.id.clone(), send));
    }
    action
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
