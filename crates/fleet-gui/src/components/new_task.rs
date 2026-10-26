use super::{
    modal_frame, persona_choice, repo_picker, roster_chips, usable, FolderPicker, PickerState,
};
use crate::theme::{mono, sans, size, tokens, Weight};
use crate::widgets::{button, chip, icon_button, list_row, ButtonVariant, Icon};
use eframe::egui::{
    self, accesskit::Role, Align, Key, Label, Layout, Modifiers, RichText, TextEdit, TextStyle, Ui,
    WidgetInfo,
};
use fleet_api::{AgentKind, DefInfo, DefKind, Persona, Product, RepoInfo, RepoPick, WorkflowState};
use fleet_fs::Checkout;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

const WIDTH: f32 = 440.0;
const PROMPT_ROWS: usize = 4;

pub struct SpawnState {
    /// The dialog is showing; the drawer's "New task" button sets it.
    pub open: bool,
    /// The persona the task will run under: the active one each time the dialog opens, overridable here.
    pub persona: String,
    /// The repos `persona` remembers, as last fetched for it; `set_persona` empties it.
    pub repos: Vec<RepoInfo>,
    /// Why `repos` could not be fetched for `persona`, shown under the picker.
    pub repos_error: Option<String>,
    pub picker: PickerState,
    pub prompt: String,
    /// The product's release the task joins; None is the backlog.
    pub release: Option<i64>,
    /// Where each picked repo starts, by repo name: `None` from its URL, `Some(path)` from that local checkout. A
    /// repo without an entry starts from its URL, else from its main checkout.
    pub starts: BTreeMap<String, Option<String>>,
    /// Each local path's checkouts, listed when first shown; `reset_choices` forgets them.
    pub checkout_cache: HashMap<String, Result<Vec<Checkout>, String>>,
    /// A spawn is in flight; the app sets this when it starts the call and clears it when the call returns.
    pub busy: bool,
    pub error: Option<String>,
    /// The goal the last spawn sent. A success clears the dialog only while it still holds this text, so a
    /// goal the user reopened and changed in the meantime is kept.
    pub sent: Option<String>,
    /// `Solo`, or `Orchestrator` with `role` naming its type.
    pub kind: AgentKind,
    pub role: Option<String>,
    /// The specialists an orchestrated task may use. Follows the chosen repo's roster until `roster_edited`.
    pub roster: Vec<String>,
    /// The user toggled a specialist since the repo was chosen, so the repo's roster no longer overrides the pick.
    pub roster_edited: bool,
    /// Why repos picked under Solo were dropped on the switch to an orchestrator type.
    pub dropped: Option<String>,
}

impl SpawnState {
    /// `browse` is the dialog's repo picker, built by the app with the paths it knows.
    pub fn new(browse: FolderPicker) -> SpawnState {
        SpawnState {
            open: false,
            persona: String::new(),
            repos: Vec::new(),
            repos_error: None,
            picker: PickerState::new(browse),
            prompt: String::new(),
            release: None,
            starts: BTreeMap::new(),
            checkout_cache: HashMap::new(),
            busy: false,
            error: None,
            sent: None,
            kind: AgentKind::Solo,
            role: None,
            roster: Vec::new(),
            roster_edited: false,
            dropped: None,
        }
    }

    fn orchestrated(&self) -> bool {
        self.kind == AgentKind::Orchestrator
    }

    /// `None` is Solo, else the orchestrator type. An orchestrated task has one repo: the first pick is kept and
    /// the others are dropped, which the dialog then says.
    pub fn set_type(&mut self, role: Option<String>) {
        self.dropped = None;
        match role {
            None => {
                self.kind = AgentKind::Solo;
                self.role = None;
            }
            Some(role) => {
                self.kind = AgentKind::Orchestrator;
                self.role = Some(role);
                if self.picker.chosen.len() > 1 {
                    let rest = self.picker.chosen.split_off(1);
                    let verb = if rest.len() == 1 { "was" } else { "were" };
                    self.dropped = Some(format!(
                        "An orchestrated task takes one repo: {} {verb} dropped.",
                        rest.join(", ")
                    ));
                    self.roster_edited = false;
                }
            }
        }
    }

    /// The chosen repo, when it is one the persona remembers (only those carry a roster).
    fn fleet_repo(&self) -> Option<&RepoInfo> {
        let name = self.picker.chosen.first()?;
        self.repos.iter().find(|r| &r.name == name)
    }

    /// After a pick under an orchestrator type: the newest pick replaces the old one, and a new repo brings its own
    /// roster again.
    fn keep_one_repo(&mut self, before: &[String]) {
        if let Some(newest) = self
            .picker
            .chosen
            .iter()
            .rfind(|c| !before.contains(c))
            .cloned()
        {
            self.picker.chosen = vec![newest];
        }
        if self.picker.chosen != before {
            self.roster_edited = false;
            self.dropped = None;
        }
    }

    /// Forgets the roster pick once a task started, so the next one starts from its repo's roster.
    pub fn sent_ok(&mut self) {
        self.roster_edited = false;
        self.dropped = None;
        self.release = None;
        self.starts.clear();
    }

    /// Called when the dialog opens: the next one starts in the backlog, from the repos' defaults, and lists the
    /// checkouts again (a branch may have been checked out since).
    pub fn reset_choices(&mut self) {
        self.release = None;
        self.starts.clear();
        self.checkout_cache.clear();
    }

    fn checkouts(&mut self, path: &str) -> &Result<Vec<Checkout>, String> {
        self.checkout_cache
            .entry(path.to_string())
            .or_insert_with(|| fleet_fs::checkouts(Path::new(path), &[]).map_err(|e| e.to_string()))
    }

    /// The main checkout of the repo at `path`, as listed; else `path` itself.
    fn main_checkout(&mut self, path: &str) -> String {
        match self.checkouts(path) {
            Ok(list) => list.iter().find(|c| c.main).map_or_else(
                || path.to_string(),
                |c| c.path.to_string_lossy().into_owned(),
            ),
            Err(_) => path.to_string(),
        }
    }

    /// Where `repo` starts: the pick if made, else its URL, else its main checkout. A repo the product does not
    /// know (a typed path or URL) starts as the daemon resolves it.
    fn start_of(&mut self, repo: &str) -> Option<String> {
        let info = self.repos.iter().find(|r| r.name == repo)?;
        let (path, has_url) = (info.path.clone(), info.url.is_some());
        let picked = self.starts.get(repo).cloned().flatten();
        match (picked, path) {
            (Some(local), _) => Some(local),
            (None, Some(path)) if !has_url => Some(self.main_checkout(&path)),
            (None, _) => None,
        }
    }

    /// The repos to send: each picked name with its start.
    fn picks(&mut self) -> Vec<RepoPick> {
        let chosen = self.picker.chosen.clone();
        chosen
            .into_iter()
            .map(|repo| RepoPick {
                local: self.start_of(&repo),
                repo,
            })
            .collect()
    }

    /// Moves the dialog to another persona. Its repos are fetched again; picks among the old persona's remembered
    /// repos are dropped (under another login the name is not there, or not the same repo), typed ones are kept.
    pub fn set_persona(&mut self, id: String) {
        if id == self.persona {
            return;
        }
        let old = std::mem::take(&mut self.repos);
        self.repos_error = None;
        self.picker
            .chosen
            .retain(|c| !old.iter().any(|r| &r.name == c));
        self.persona = id;
    }
}

/// What the dialog asks the app for.
#[derive(Debug, Clone, PartialEq)]
pub enum SpawnAction {
    /// The persona was changed to this id: its repos are to be fetched.
    Persona(String),
    /// Create workflow was clicked: the app closes the dialog and shows Management → Workflow.
    OpenWorkflowTab,
    Start {
        persona: String,
        release: Option<i64>,
        repos: Vec<RepoPick>,
        prompt: String,
        kind: AgentKind,
        role: Option<String>,
        roster: Vec<String>,
    },
}

/// Why Start cannot run under `persona`, if it cannot.
fn persona_blocker(personas: &[Persona], persona: &str) -> Option<&'static str> {
    if personas.iter().any(|p| p.ready && p.id == persona) {
        None
    } else if personas.iter().any(|p| p.ready) {
        Some("Pick a persona")
    } else {
        Some("Set up a persona first")
    }
}

/// What a repo's workflow state says, and whether it is a problem to show (not a block: creation decides). One text for the dialog and the
/// Personas row, so the two never disagree.
pub fn workflow_line(repo: &str, w: &WorkflowState) -> (String, bool) {
    match w {
        WorkflowState::Found { base, steps } => {
            (format!("Checks on {base}: {}", steps.join(", ")), false)
        }
        WorkflowState::Missing { base } => (
            format!("{repo} has no .fleet/workflow.yml on {base}. It lists the checks the gate runs before a pull request, so orchestrated tasks need it."),
            true,
        ),
        WorkflowState::Invalid { base, error } => (
            format!("{repo}: .fleet/workflow.yml on {base} is invalid: {error}"),
            true,
        ),
        WorkflowState::Unreadable { base, error } => (
            format!("{repo}: cannot read .fleet/workflow.yml on {base}: {error}"),
            true,
        ),
    }
}

/// Why an orchestrated task cannot start yet, if it cannot.
fn fleet_blocker(st: &SpawnState) -> Option<String> {
    if !st.orchestrated() {
        return None;
    }
    if !st.picker.typed.trim().is_empty() {
        return Some("Add the typed repo first".into());
    }
    if st.picker.chosen.is_empty() {
        return Some("Pick a repo".into());
    }
    // The workflow line only reflects the clone's last fetch, so it never blocks: creation fetches and refuses.
    if st.roster.is_empty() {
        return Some("Pick at least one specialist".into());
    }
    None
}

/// The Type dropdown: Solo, then each orchestrator type that loads. Returns the pick, `Some(None)` for Solo.
fn type_choice(
    ui: &mut Ui,
    orchestrators: &[&str],
    st: &SpawnState,
    width: f32,
) -> Option<Option<String>> {
    let p = tokens(ui.visuals().dark_mode);
    let current = st.role.as_deref().unwrap_or("Solo");
    let mut picked = None;
    let combo = egui::ComboBox::from_id_salt("new_task_type")
        .selected_text(RichText::new(current).color(p.text_high))
        .width(width)
        .show_ui(ui, |ui| {
            ui.set_min_width(width);
            let options = std::iter::once(None).chain(orchestrators.iter().map(|o| Some(*o)));
            for option in options {
                let on = option == st.role.as_deref();
                let name = option.unwrap_or("Solo");
                let row = list_row(ui, name, on, |ui| {
                    ui.add(Label::new(RichText::new(name).color(p.text_high)).truncate());
                });
                if row.clicked() && !on {
                    picked = Some(option.map(String::from));
                }
            }
        });
    combo
        .response
        .widget_info(|| WidgetInfo::labeled(Role::ComboBox, ui.is_enabled(), "Type"));
    picked
}

/// The Release dropdown: the product's unreleased releases, then the backlog. Returns the pick, `Some(None)` for
/// the backlog.
fn release_choice(
    ui: &mut Ui,
    product: Option<&Product>,
    current: Option<i64>,
    width: f32,
) -> Option<Option<i64>> {
    let p = tokens(ui.visuals().dark_mode);
    let open: Vec<_> = product
        .map(|p| {
            p.releases
                .iter()
                .filter(|r| r.released_at.is_none())
                .collect()
        })
        .unwrap_or_default();
    let text = open
        .iter()
        .find(|r| Some(r.id) == current)
        .map_or("Backlog", |r| r.version.as_str());
    let mut picked = None;
    let combo = egui::ComboBox::from_id_salt("new_task_release")
        .selected_text(RichText::new(text).color(p.text_high))
        .width(width)
        .show_ui(ui, |ui| {
            ui.set_min_width(width);
            let options = std::iter::once((None, "Backlog"))
                .chain(open.iter().map(|r| (Some(r.id), r.version.as_str())));
            for (id, name) in options {
                let on = id == current;
                let row = list_row(ui, name, on, |ui| {
                    ui.add(Label::new(RichText::new(name).color(p.text_high)).truncate());
                });
                if row.clicked() && !on {
                    picked = Some(id);
                }
            }
        });
    combo
        .response
        .widget_info(|| WidgetInfo::labeled(Role::ComboBox, ui.is_enabled(), "Release"));
    picked
}

/// The last two parts of a long path: enough to tell checkouts apart.
fn short_path(path: &Path) -> String {
    let parts: Vec<_> = path.components().collect();
    if parts.len() <= 3 {
        return path.display().to_string();
    }
    let tail: std::path::PathBuf = parts[parts.len() - 2..].iter().collect();
    format!("…/{}", tail.display())
}

fn checkout_label(c: &Checkout) -> String {
    let branch = c.head.branch.as_deref().unwrap_or("detached");
    format!("{branch}  {}", short_path(&c.path))
}

/// For each picked repo the product knows by a path: where the task starts. Both a path and a URL give the choice;
/// a path alone gives the checkout list; a URL alone gives nothing to choose.
fn starts_section(ui: &mut Ui, st: &mut SpawnState) {
    let p = tokens(ui.visuals().dark_mode);
    let width = WIDTH - 64.0;
    for name in st.picker.chosen.clone() {
        let Some(info) = st.repos.iter().find(|r| r.name == name) else {
            continue;
        };
        let (path, has_url) = (info.path.clone(), info.url.is_some());
        let Some(path) = path else { continue };
        let local = st.start_of(&name).is_some();
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!("{name} starts"))
                    .font(sans(ui, size::SMALL, Weight::Regular))
                    .color(p.text_low),
            );
            if has_url {
                if chip(ui, "From URL", !local).clicked() {
                    st.starts.insert(name.clone(), None);
                }
                if chip(ui, "Local checkout", local).clicked() && !local {
                    let main = st.main_checkout(&path);
                    st.starts.insert(name.clone(), Some(main));
                }
            }
        });
        if local {
            let current = st.start_of(&name).unwrap_or_default();
            match st.checkouts(&path).clone() {
                Ok(list) => {
                    let text = list
                        .iter()
                        .find(|c| c.path.to_string_lossy() == current)
                        .map_or_else(|| current.clone(), checkout_label);
                    let combo = egui::ComboBox::from_id_salt(("new_task_start", &name))
                        .selected_text(RichText::new(text).color(p.text_high))
                        .width(width)
                        .show_ui(ui, |ui| {
                            ui.set_min_width(width);
                            for c in &list {
                                let value = c.path.to_string_lossy().into_owned();
                                let on = value == current;
                                let label = checkout_label(c);
                                let row = list_row(ui, &label, on, |ui| {
                                    ui.add(
                                        Label::new(RichText::new(&label).color(p.text_high))
                                            .truncate(),
                                    );
                                });
                                if row.clicked() && !on {
                                    st.starts.insert(name.clone(), Some(value));
                                }
                            }
                        });
                    let what = format!("Checkout of {name}");
                    combo.response.widget_info(|| {
                        WidgetInfo::labeled(Role::ComboBox, ui.is_enabled(), &what)
                    });
                }
                Err(e) => {
                    ui.add(
                        Label::new(
                            RichText::new(format!("Cannot list the checkouts of {name}: {e}"))
                                .size(size::SMALL)
                                .color(p.danger),
                        )
                        .wrap(),
                    );
                }
            }
        }
        if !has_url {
            ui.add(
                Label::new(
                    RichText::new("No URL: no pull request or merge tracking for this repo.")
                        .size(size::SMALL)
                        .color(p.text_low),
                )
                .wrap(),
            );
        }
        ui.add_space(6.0);
    }
}

fn caption(ui: &mut Ui, text: &str) -> egui::Response {
    let low = tokens(ui.visuals().dark_mode).text_low;
    ui.label(
        RichText::new(text)
            .font(sans(ui, size::SMALL, Weight::Medium))
            .color(low),
    )
}

/// The New task dialog: the goal, the type (Solo or an orchestrator type of `defs`), the persona (ready ones of
/// `personas`), the release (an unreleased one of `product`, else the backlog), the repos and where each starts, an orchestrated task's specialists, and Start. Draws nothing unless
/// `st.open`. Returns `Start` when the user starts the task; the app clears the goal when the spawn succeeds, so
/// a failure keeps what was typed. Closing by any route keeps the goal for next time.
pub fn new_task_dialog(
    ctx: &egui::Context,
    personas: &[Persona],
    defs: &[DefInfo],
    product: Option<&Product>,
    st: &mut SpawnState,
) -> Option<SpawnAction> {
    let opened = egui::Id::unique("new_task_opened");
    if !st.open {
        ctx.data_mut(|d| d.remove_temp::<bool>(opened));
        return None;
    }
    // The prompt takes the focus on the first frame of each opening only, so later frames do not steal it back.
    let first_frame = ctx.data_mut(|d| d.get_temp::<bool>(opened)).is_none();
    ctx.data_mut(|d| d.insert_temp(opened, true));

    let p = tokens(ctx.global_style().visuals.dark_mode);
    let frame = modal_frame(ctx);
    // `WIDTH` is the dialog's outside; the content gets what the frame's margin and stroke leave.
    let inner_width = WIDTH - frame.total_margin().sum().x;
    let (mut close, mut action) = (false, None);
    let modal = egui::Modal::new(egui::Id::unique("new_task_dialog"))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(inner_width);
            // Taken before the TextEdit is drawn, or the edit would insert a newline for it.
            let ctrl_enter = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("New task")
                        .text_style(TextStyle::Heading)
                        .color(p.text_high),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    close |= icon_button(ui, Icon::Close, "Close").clicked();
                });
            });
            ui.add_space(12.0);

            let label = caption(ui, "Goal");
            ui.add_space(4.0);
            // Names the box for screen readers and for the UI tests, which refuse unnamed inputs.
            let edit = ui
                .add(
                    TextEdit::multiline(&mut st.prompt)
                        .desired_rows(PROMPT_ROWS)
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(label.id);
            if first_frame {
                edit.request_focus();
            }
            ui.add_space(10.0);

            caption(ui, "Type");
            ui.add_space(4.0);
            if let Some(role) = type_choice(
                ui,
                &usable(defs, DefKind::Orchestrator),
                st,
                inner_width - 24.0,
            ) {
                st.set_type(role);
            }
            ui.add_space(10.0);

            let blocked = persona_blocker(personas, &st.persona);
            caption(ui, "Persona");
            ui.add_space(4.0);
            if let Some(id) =
                persona_choice(ui, "Persona", personas, &st.persona, inner_width - 24.0)
            {
                st.set_persona(id.clone());
                action = Some(SpawnAction::Persona(id));
            }
            if let Some(why) = blocked {
                ui.label(RichText::new(why).size(size::SMALL).color(p.danger));
            }
            ui.add_space(10.0);

            caption(ui, "Release");
            ui.add_space(4.0);
            if let Some(release) = release_choice(ui, product, st.release, inner_width - 24.0) {
                st.release = release;
            }
            ui.add_space(10.0);

            caption(
                ui,
                if st.orchestrated() {
                    "Repo"
                } else {
                    "Repos (optional)"
                },
            );
            ui.add_space(4.0);
            let before = st.picker.chosen.clone();
            if repo_picker(ui, "new_task_repos", &st.repos, &mut st.picker) && st.orchestrated() {
                st.keep_one_repo(&before);
            }
            if let Some(hint) = &st.dropped {
                ui.add(Label::new(RichText::new(hint).size(size::SMALL).color(p.text_low)).wrap());
            }
            if let Some(e) = &st.repos_error {
                ui.label(RichText::new(e).size(size::SMALL).color(p.danger));
            }
            ui.add_space(12.0);
            starts_section(ui, st);
            if st.orchestrated() {
                if let Some(a) = fleet_section(ui, defs, st) {
                    action = Some(a);
                }
            }
            let fleet_blocked = fleet_blocker(st);

            if let Some(e) = &st.error {
                ui.label(RichText::new(e).size(size::SMALL).color(p.danger));
                ui.add_space(8.0);
            }

            let ready = !st.busy
                && !st.prompt.trim().is_empty()
                && blocked.is_none()
                && fleet_blocked.is_none();
            let why = blocked.map(String::from).or(fleet_blocked);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Ctrl+Enter")
                        .font(mono(size::META))
                        .color(p.text_low),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let start = ui
                        .add_enabled_ui(ready, |ui| {
                            button(ui, "Start task", ButtonVariant::Primary)
                        })
                        .inner;
                    if (start
                        .on_disabled_hover_text(why.unwrap_or_default())
                        .clicked()
                        || ctrl_enter)
                        && ready
                    {
                        st.picker.commit_typed();
                        action = Some(SpawnAction::Start {
                            persona: st.persona.clone(),
                            release: st.release,
                            repos: st.picks(),
                            prompt: st.prompt.clone(),
                            kind: st.kind,
                            role: st.role.clone(),
                            roster: if st.orchestrated() {
                                st.roster.clone()
                            } else {
                                Vec::new()
                            },
                        });
                    }
                    close |= button(ui, "Cancel", ButtonVariant::Default).clicked();
                });
            });
        });
    // Escape, a click outside and the dialog's own close request all land here; none of them clears the prompt.
    let started = matches!(action, Some(SpawnAction::Start { .. }));
    if !started && (close || modal.should_close()) {
        st.open = false;
    }
    action
}

/// An orchestrated task's specialists, preset from the repo until edited.
fn fleet_section(ui: &mut Ui, defs: &[DefInfo], st: &mut SpawnState) -> Option<SpawnAction> {
    let specialists = usable(defs, DefKind::Specialist);
    let repo_roster = st
        .fleet_repo()
        .map(|r| r.roster.clone())
        .unwrap_or_default();
    if !st.roster_edited {
        st.roster = repo_roster
            .iter()
            .filter(|n| specialists.contains(&n.as_str()))
            .cloned()
            .collect();
    }
    let stale: Vec<&str> = repo_roster
        .iter()
        .map(String::as_str)
        .filter(|n| !specialists.contains(n))
        .collect();
    caption(ui, "Specialists");
    ui.add_space(4.0);
    if roster_chips(ui, &specialists, &mut st.roster, &stale) {
        st.roster_edited = true;
    }
    ui.add_space(8.0);
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    match st.fleet_repo() {
        Some(r) => {
            let (text, blocks) = workflow_line(&r.name, &r.workflow);
            if blocks {
                ui.add(Label::new(RichText::new(text).size(size::SMALL).color(p.danger)).wrap());
                ui.add_space(4.0);
                if button(ui, "Create workflow", ButtonVariant::Default).clicked() {
                    action = Some(SpawnAction::OpenWorkflowTab);
                }
            } else {
                ui.add(
                    Label::new(RichText::new(text).font(mono(size::META)).color(p.text_low)).wrap(),
                );
            }
        }
        None if !st.picker.chosen.is_empty() => {
            ui.add(
                Label::new(
                    RichText::new("The workflow is checked when the task is created.")
                        .size(size::SMALL)
                        .color(p.text_low),
                )
                .wrap(),
            );
        }
        None => {}
    }
    ui.add_space(12.0);
    action
}
