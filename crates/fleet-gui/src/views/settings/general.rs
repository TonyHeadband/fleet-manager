//! Settings → General: choices that apply at once and are saved at once, and the tasks and personas folders,
//! which need Save because it restarts the daemon.
use super::{SettingsAction, SettingsState};
use crate::components::{browse, FolderPicker};
use crate::theme::{mono_family, settings, size, tokens, ui_family};
use crate::widgets::{button, chip, fui, path_edit, ButtonVariant};
use eframe::egui::{self, accesskit::Role, Response, RichText, Ui, WidgetInfo};
use fleet_api::{Effort, PermissionMode};
use fleet_client::config::{
    parse_labels, CloseChoice, GateBuild, MonoFont, PrSettings, ThemeChoice, UiFont,
};
use fleet_client::shortcuts::{ShortcutState, Which};
use fleet_client::update::UpdateState;

/// A choice the user just made; the app applies and saves it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GeneralChange {
    Theme(ThemeChoice),
    /// `None` is Ask.
    CloseChoice(Option<CloseChoice>),
    StickyAlerts(bool),
    UiFont(UiFont),
    MonoFont(MonoFont),
    Animations(bool),
}

/// What the Shortcuts row shows: what exists, which places are ticked, and what the last change said.
pub struct ShortcutsPane<'a> {
    pub state: ShortcutState,
    pub pick: &'a mut Which,
    pub error: Option<&'a str>,
}

/// A button on the Shortcuts row; the app carries it out for the ticked places.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShortcutsAction {
    Add,
    Remove,
    /// Point the shortcuts that exist at this exe.
    Update,
}

/// A folder field (Tasks or Personas): the app's text and Browse… dialog, why Save cannot run now, if it
/// cannot, and whether a restart this field is concerned by is running.
pub struct FolderField<'a> {
    pub text: &'a mut String,
    pub picker: &'a mut FolderPicker,
    pub blocked: Option<&'static str>,
    pub restarting: bool,
}

/// Folders the setup wizard wrote to the config, waiting for the daemon's restart: what they are, why Restart now
/// cannot run now, if it cannot, and whether the restart is running.
pub struct PendingFolders {
    pub text: String,
    pub blocked: Option<&'static str>,
    pub restarting: bool,
}

/// The limits as typed, in the order of `LIMITS`; why Save cannot run now, if it cannot; and whether a
/// restart this field is concerned by is running.
pub struct LimitsField<'a> {
    pub texts: &'a mut [String; 6],
    pub blocked: Option<&'static str>,
    pub restarting: bool,
}

/// The pull request settings as typed. Labels stay one comma-separated line until they are saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrDraft {
    pub style: String,
    pub attribution: String,
    pub draft: bool,
    pub labels: String,
    pub gate_build: GateBuild,
}

impl From<&PrSettings> for PrDraft {
    fn from(pr: &PrSettings) -> Self {
        PrDraft {
            style: pr.style.clone(),
            attribution: pr.attribution.clone(),
            draft: pr.draft,
            labels: pr.labels.join(", "),
            gate_build: pr.gate_build,
        }
    }
}

impl PrDraft {
    pub fn to_settings(&self) -> PrSettings {
        PrSettings {
            style: self.style.clone(),
            attribution: self.attribution.trim().to_string(),
            draft: self.draft,
            labels: parse_labels(&self.labels),
            gate_build: self.gate_build,
        }
    }
}

/// The Pull requests rows: the settings as typed; the PR styles that load, `None` until the daemon has listed its
/// definitions; why Save cannot run now, if it cannot; and whether a restart this field is concerned by is running.
pub struct PrField<'a> {
    pub typed: &'a mut PrDraft,
    pub styles: Option<Vec<&'a str>>,
    pub blocked: Option<&'static str>,
    pub restarting: bool,
}

/// The default permission mode and effort as typed (mode, then effort); why Save cannot run now, if it cannot; and
/// whether a restart this field is concerned by is running.
pub struct AgentDefaultsField<'a> {
    pub typed: &'a mut (PermissionMode, Effort),
    pub blocked: Option<&'static str>,
    pub restarting: bool,
}

const MODES: [PermissionMode; 4] = [
    PermissionMode::Auto,
    PermissionMode::AcceptEdits,
    PermissionMode::Manual,
    PermissionMode::DontAsk,
];
const EFFORTS: [Effort; 5] = [
    Effort::Low,
    Effort::Medium,
    Effort::High,
    Effort::Xhigh,
    Effort::Max,
];

/// The limit rows: the label, and what the limit does, which is also the field's hover.
pub const LIMITS: [(&str, &str); 6] = [
    (
        "Max breakdown versions",
        "Most breakdown versions one task may accumulate. A new breakdown after that is refused.",
    ),
    ("Max steps", "Most steps one breakdown may declare."),
    (
        "Max parallel specialists",
        "Most specialists one orchestrator may run at once. Starting another is refused.",
    ),
    (
        "Max PR attempts",
        "Most times one task may try to open its pull request. A refused attempt after that is final.",
    ),
    (
        "Max concurrent tasks",
        "Most orchestrated tasks one product may have working at once. Starting another is refused. Empty: no cap.",
    ),
    (
        "Max active products",
        "Most products that may have orchestrated tasks working at once. Starting a task in another is refused. Empty: no cap.",
    ),
];

/// What tells one folder row from the other.
struct Row {
    heading: &'static str,
    id: &'static str,
    /// The Save button's name for assistive tech and tests when "Save" alone is ambiguous on the tab.
    save_name: Option<&'static str>,
    note: &'static str,
}

const TASKS: Row = Row {
    heading: "Tasks folder",
    id: "settings_tasks_folder",
    save_name: None,
    note: "Applies to new tasks only. Saving restarts the daemon; running agents keep going.",
};

// The daemon reads `personas_dir` only at startup and never moves a persona's folder: its logins live there.
const PERSONAS: Row = Row {
    heading: "Personas folder",
    id: "settings_personas_folder",
    save_name: Some("Save personas folder"),
    note: "Saving restarts the daemon. Existing personas' folders, with their Claude and GitHub logins, are not moved: move them by hand or log the personas in again.",
};

const THEMES: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light];
const CLOSE: [(Option<CloseChoice>, &str); 3] = [
    (None, "Ask"),
    (Some(CloseChoice::KeepRunning), "Keep daemon running"),
    (Some(CloseChoice::StopDaemon), "Stop daemon"),
];

pub(super) fn general(ui: &mut Ui, state: SettingsState<'_>) -> Option<SettingsAction> {
    let mut change = None;
    let mut shortcut = None;
    version_line(ui, state.update);
    if let Some(pane) = state.shortcuts {
        ui.add_space(16.0);
        if let Some(a) = shortcuts_row(ui, pane) {
            shortcut = Some(a);
        }
    }
    ui.add_space(16.0);
    heading(ui, "Theme");
    ui.horizontal(|ui| {
        for choice in THEMES {
            if chip(ui, choice.label(), choice == state.theme).clicked() && choice != state.theme {
                change = Some(GeneralChange::Theme(choice));
            }
        }
    });
    ui.add_space(16.0);
    heading(ui, "Font");
    ui.horizontal_wrapped(|ui| {
        for font in UiFont::ALL {
            let label = in_face(ui, font.label(), ui_family(font));
            if chip(ui, label, font == state.fonts.ui).clicked() && font != state.fonts.ui {
                change = Some(GeneralChange::UiFont(font));
            }
        }
    });
    ui.add_space(16.0);
    heading(ui, "Mono font");
    ui.horizontal_wrapped(|ui| {
        for font in MonoFont::ALL {
            let label = in_face(ui, font.label(), mono_family(font));
            if chip(ui, label, font == state.fonts.mono).clicked() && font != state.fonts.mono {
                change = Some(GeneralChange::MonoFont(font));
            }
        }
    });
    ui.add_space(16.0);
    heading(ui, "Closing the window");
    ui.horizontal(|ui| {
        for (choice, label) in CLOSE {
            if chip(ui, label, choice == state.close_choice).clicked()
                && choice != state.close_choice
            {
                change = Some(GeneralChange::CloseChoice(choice));
            }
        }
    });
    ui.add_space(16.0);
    heading(ui, "Footer");
    let mut sticky = state.sticky_alerts;
    if ui
        .checkbox(
            &mut sticky,
            "Keep errors and warnings in the footer until dismissed",
        )
        .clicked()
    {
        change = Some(GeneralChange::StickyAlerts(sticky));
    }
    ui.add_space(16.0);
    heading(ui, "Motion");
    let mut animations = state.animations;
    if ui
        .checkbox(&mut animations, "Animations")
        .on_hover_text("Card edges, the radar and other motion. Off keeps the window idle.")
        .clicked()
    {
        change = Some(GeneralChange::Animations(animations));
    }
    let mut action = change.map(SettingsAction::Change);
    if let Some(a) = shortcut {
        action = Some(SettingsAction::Shortcuts(a));
    }
    if let Some(tasks) = state.tasks {
        ui.add_space(16.0);
        if folder(ui, &TASKS, tasks) {
            action = Some(SettingsAction::SaveTasksRoot);
        }
    }
    if let Some(personas) = state.personas {
        ui.add_space(16.0);
        if folder(ui, &PERSONAS, personas) {
            action = Some(SettingsAction::SavePersonasDir);
        }
    }
    if let Some(pending) = state.pending {
        ui.add_space(settings::PENDING_GAP);
        if pending_restart(ui, &pending) {
            action = Some(SettingsAction::RestartDaemon);
        }
    }
    if let Some(limits) = state.limits {
        ui.add_space(16.0);
        if limits_rows(ui, limits) {
            action = Some(SettingsAction::SaveLimits);
        }
    }
    if let Some(pr) = state.pr {
        ui.add_space(16.0);
        if pr_rows(ui, pr) {
            action = Some(SettingsAction::SavePr);
        }
    }
    if let Some(defaults) = state.agent_defaults {
        ui.add_space(16.0);
        if agent_defaults_rows(ui, defaults) {
            action = Some(SettingsAction::SaveAgentDefaults);
        }
    }
    ui.add_space(16.0);
    if setup_wizard(ui) {
        action = Some(SettingsAction::RelaunchWizard);
    }
    action
}

/// The running version and what the last new-version check found; Download opens the release page.
fn version_line(ui: &mut Ui, update: &UpdateState) {
    let p = tokens(ui.visuals().dark_mode);
    let current = env!("CARGO_PKG_VERSION");
    heading(ui, "Version");
    let text = |t: String| RichText::new(t).color(p.text);
    match update {
        UpdateState::Unknown => {
            ui.label(text(format!("Version v{current}")));
        }
        UpdateState::UpToDate => {
            ui.label(text(format!("Version v{current} — up to date")));
        }
        UpdateState::Available(release) => {
            ui.horizontal(|ui| {
                ui.label(text(format!("v{} available", release.version)));
                ui.label(text("—".into()));
                if ui.link("Download").clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(&release.page));
                }
            });
        }
        UpdateState::Failed => {
            ui.label(text("Could not check for updates".into()));
        }
    }
}

/// Start menu and Desktop shortcuts: which exist, the two places to tick, Add and Remove, and Update when they
/// point at an old location.
fn shortcuts_row(ui: &mut Ui, pane: ShortcutsPane<'_>) -> Option<ShortcutsAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    heading(ui, "Shortcuts");
    let exists = match pane.state {
        ShortcutState::None => "No shortcuts yet".to_string(),
        ShortcutState::Current { menu, desktop } => {
            let places = [(menu, "Start menu"), (desktop, "Desktop")]
                .into_iter()
                .filter_map(|(on, name)| on.then_some(name))
                .collect::<Vec<_>>();
            format!("Shortcuts in: {}", places.join(", "))
        }
        ShortcutState::Stale => "Shortcuts point to an old location —".to_string(),
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(exists).color(p.text));
        if pane.state == ShortcutState::Stale
            && button(ui, "Update", ButtonVariant::Primary).clicked()
        {
            action = Some(ShortcutsAction::Update);
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.checkbox(&mut pane.pick.menu, "Start menu");
        ui.checkbox(&mut pane.pick.desktop, "Desktop");
    });
    ui.add_space(4.0);
    let any = pane.pick.menu || pane.pick.desktop;
    ui.horizontal(|ui| {
        ui.add_enabled_ui(any, |ui| {
            if button(ui, "Add shortcuts", ButtonVariant::Default).clicked() {
                action = Some(ShortcutsAction::Add);
            }
            if button(ui, "Remove shortcuts", ButtonVariant::Default).clicked() {
                action = Some(ShortcutsAction::Remove);
            }
        });
    });
    if let Some(e) = pane.error {
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(e).size(size::SMALL).color(p.danger)).wrap());
    }
    action
}

/// What waits for the restart, and Restart now. True when Restart now was clicked.
fn pending_restart(ui: &mut Ui, pending: &PendingFolders) -> bool {
    let p = tokens(ui.visuals().dark_mode);
    ui.add(
        egui::Label::new(
            RichText::new(format!(
                "Saved: {} — takes effect when the background service restarts.",
                pending.text
            ))
            .color(p.text),
        )
        .wrap(),
    );
    ui.add_space(4.0);
    let mut clicked = false;
    ui.horizontal(|ui| {
        clicked = ui
            .add_enabled_ui(pending.blocked.is_none(), |ui| {
                button(ui, "Restart now", ButtonVariant::Primary)
            })
            .inner
            .on_disabled_hover_text(pending.blocked.unwrap_or_default())
            .clicked();
        if pending.restarting {
            ui.label(
                RichText::new("Restarting the daemon…")
                    .size(size::SMALL)
                    .color(p.text_low),
            );
        }
    });
    clicked
}

/// The setup wizard heading, what it does and Relaunch setup wizard. True when the button was clicked.
pub(super) fn setup_wizard(ui: &mut Ui) -> bool {
    let p = tokens(ui.visuals().dark_mode);
    heading(ui, "Setup wizard");
    let clicked = button(ui, "Relaunch setup wizard", ButtonVariant::Default).clicked();
    ui.add_space(4.0);
    ui.label(
        RichText::new(
            "Checks the tools Fleet Manager needs and installs what is missing; the folders can be changed there too.",
        )
        .size(size::SMALL)
        .color(p.text_low),
    );
    clicked
}

/// The limit fields and Save. A product can override each of them. True when Save was clicked.
fn limits_rows(ui: &mut Ui, f: LimitsField<'_>) -> bool {
    let p = tokens(ui.visuals().dark_mode);
    heading(ui, "Limits");
    for (text, (label, hint)) in f.texts.iter_mut().zip(LIMITS) {
        ui.horizontal(|ui| {
            let name = ui.label(RichText::new(label).color(p.text));
            ui.add(
                egui::TextEdit::singleline(text)
                    .desired_width(settings::LIMIT_FIELD)
                    .id_salt(label),
            )
            .on_hover_text(hint)
            .labelled_by(name.id);
        });
    }
    ui.add_space(4.0);
    let enabled = f.blocked.is_none();
    let save = ui
        .add_enabled_ui(enabled, |ui| button(ui, "Save", ButtonVariant::Primary))
        .inner;
    save.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, "Save limits"));
    let clicked = save
        .on_disabled_hover_text(f.blocked.unwrap_or_default())
        .clicked();
    ui.add_space(4.0);
    let note = if f.restarting {
        "Restarting the daemon…"
    } else {
        "Saving restarts the daemon; running agents keep going. A product can override each limit except Max active products on Management, Products."
    };
    ui.label(RichText::new(note).size(size::SMALL).color(p.text_low));
    clicked
}

/// The pull request style, attribution, draft and labels, and Save. True when Save was clicked.
fn pr_rows(ui: &mut Ui, f: PrField<'_>) -> bool {
    let p = tokens(ui.visuals().dark_mode);
    heading(ui, "Pull requests");
    ui.horizontal(|ui| {
        ui.label(RichText::new("Style").color(p.text));
        // Without the definitions list, the current style is all there is to show.
        let styles = f.styles.unwrap_or_default();
        let combo = egui::ComboBox::from_id_salt("settings_pr_style")
            .selected_text(RichText::new(f.typed.style.as_str()).color(p.text_high))
            .show_ui(ui, |ui| {
                for style in styles {
                    if ui.selectable_label(f.typed.style == style, style).clicked() {
                        f.typed.style = style.to_string();
                    }
                }
            });
        combo
            .response
            .widget_info(|| WidgetInfo::labeled(Role::ComboBox, true, "Pull request style"));
    });
    ui.horizontal(|ui| {
        let name = ui.label(RichText::new("Attribution").color(p.text));
        ui.add(
            egui::TextEdit::singleline(&mut f.typed.attribution)
                .desired_width(settings::PR_FIELD)
                .id_salt("settings_pr_attribution"),
        )
        .on_hover_text("A line added to the end of every pull request body. Empty adds nothing.")
        .labelled_by(name.id);
    });
    ui.checkbox(&mut f.typed.draft, "Draft pull requests");
    ui.horizontal(|ui| {
        let name = ui.label(RichText::new("PR labels").color(p.text));
        ui.add(
            egui::TextEdit::singleline(&mut f.typed.labels)
                .desired_width(settings::PR_FIELD)
                .id_salt("settings_pr_labels"),
        )
        .on_hover_text("Comma-separated labels put on every pull request.")
        .labelled_by(name.id);
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Gate build").color(p.text));
        let combo = egui::ComboBox::from_id_salt("settings_pr_gate_build")
            .selected_text(RichText::new(f.typed.gate_build.as_str()).color(p.text_high))
            .show_ui(ui, |ui| {
                for option in GateBuild::ALL {
                    if ui
                        .selectable_label(f.typed.gate_build == option, option.as_str())
                        .clicked()
                    {
                        f.typed.gate_build = option;
                    }
                }
            });
        combo
            .response
            .on_hover_text(
                "shared: the gate builds in the task's folder, reusing what the agents built. clean: it builds in a fresh folder that is removed after.",
            )
            .widget_info(|| WidgetInfo::labeled(Role::ComboBox, true, "Gate build"));
    });
    ui.add_space(4.0);
    let enabled = f.blocked.is_none();
    let save = ui
        .add_enabled_ui(enabled, |ui| button(ui, "Save", ButtonVariant::Primary))
        .inner;
    save.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, "Save pull requests"));
    let clicked = save
        .on_disabled_hover_text(f.blocked.unwrap_or_default())
        .clicked();
    ui.add_space(4.0);
    let note = if f.restarting {
        "Restarting the daemon…"
    } else {
        "Saving restarts the daemon; running agents keep going. A product can override each of these on Management, Products."
    };
    ui.label(RichText::new(note).size(size::SMALL).color(p.text_low));
    clicked
}

/// The default mode and effort every agent starts with, and Save. True when Save was clicked.
fn agent_defaults_rows(ui: &mut Ui, f: AgentDefaultsField<'_>) -> bool {
    let p = tokens(ui.visuals().dark_mode);
    heading(ui, "Agents");
    let (mode, effort) = f.typed;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Default mode").color(p.text));
        let combo = egui::ComboBox::from_id_salt("settings_default_mode")
            .selected_text(RichText::new(mode.as_flag()).color(p.text_high))
            .show_ui(ui, |ui| {
                for option in MODES {
                    if ui
                        .selectable_label(*mode == option, option.as_flag())
                        .clicked()
                    {
                        *mode = option;
                    }
                }
            });
        combo
            .response
            .widget_info(|| WidgetInfo::labeled(Role::ComboBox, true, "Default mode"));
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Default effort").color(p.text));
        let combo = egui::ComboBox::from_id_salt("settings_default_effort")
            .selected_text(RichText::new(effort.as_flag()).color(p.text_high))
            .show_ui(ui, |ui| {
                for option in EFFORTS {
                    if ui
                        .selectable_label(*effort == option, option.as_flag())
                        .clicked()
                    {
                        *effort = option;
                    }
                }
            });
        combo
            .response
            .widget_info(|| WidgetInfo::labeled(Role::ComboBox, true, "Default effort"));
    });
    ui.add_space(4.0);
    let enabled = f.blocked.is_none();
    let save = ui
        .add_enabled_ui(enabled, |ui| button(ui, "Save", ButtonVariant::Primary))
        .inner;
    save.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, "Save agent defaults"));
    let clicked = save
        .on_disabled_hover_text(f.blocked.unwrap_or_default())
        .clicked();
    ui.add_space(4.0);
    let note = if f.restarting {
        "Restarting the daemon…"
    } else {
        "Used by every agent whose definition and product name neither. Saving restarts the daemon; running agents keep going."
    };
    ui.label(RichText::new(note).size(size::SMALL).color(p.text_low));
    clicked
}

/// The field, Browse… and Save, and the note. Trust is per persona, in its Claude login. True when Save was clicked.
fn folder(ui: &mut Ui, row: &Row, f: FolderField<'_>) -> bool {
    let p = tokens(ui.visuals().dark_mode);
    let label = heading(ui, row.heading);
    let mut save = false;
    // Wide enough for a real path, but not the whole window on a wide screen.
    ui.allocate_ui(
        egui::vec2(ui.available_width().min(settings::FOLDER_ROW), 0.0),
        |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // From the right, so the field takes exactly the rest of the row.
                    let enabled = f.blocked.is_none();
                    let clicked = ui
                        .add_enabled_ui(enabled, |ui| button(ui, "Save", ButtonVariant::Primary))
                        .inner;
                    if let Some(name) = row.save_name {
                        clicked.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, name));
                    }
                    save = clicked
                        .on_disabled_hover_text(f.blocked.unwrap_or_default())
                        .clicked();
                    let picked = browse(ui, f.picker, f.text);
                    let (id, width) = (ui.make_persistent_id(row.id), ui.available_width());
                    path_edit(ui, id, f.text, width, None, "", picked).labelled_by(label.id);
                });
            });
            ui.add_space(4.0);
            let note = if f.restarting {
                "Restarting the daemon…"
            } else {
                row.note
            };
            ui.label(RichText::new(note).size(size::SMALL).color(p.text_low));
        },
    );
    save
}

/// `text` drawn in `family`, so each font choice shows itself. Laying out an unregistered family panics, and
/// widget tests run without `theme::apply`, so it falls back to the default face until the family exists.
fn in_face(ui: &Ui, text: &str, family: egui::FontFamily) -> RichText {
    let installed = ui
        .ctx()
        .fonts(|f| f.definitions().families.contains_key(&family));
    let text = RichText::new(text);
    if installed {
        text.family(family)
    } else {
        text
    }
}

pub(super) fn heading(ui: &mut Ui, text: &str) -> Response {
    let p = tokens(ui.visuals().dark_mode);
    let r = fui::mono_label(ui, text, p.text_low);
    ui.add_space(6.0);
    r
}
