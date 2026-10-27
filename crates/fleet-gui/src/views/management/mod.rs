//! Management: where the fleet itself is configured (personas, instructions and settings for every agent, and the
//! orchestrator, specialist and skill definitions), one tab per area. Settings is the app's and the daemon's options.
mod agent_settings;
mod definitions;
mod instructions;
mod internal;
mod overview;
mod personas;
mod products;
mod workflow;

pub use definitions::{DefKey, DefsState};
pub use internal::InternalState;
pub use overview::{counts, Counts, OverviewState};
pub use personas::PersonasState;
pub use products::{FleetDraft, OverrideDraft, ProductEdit, ProductsState};

use super::{title, View};
use crate::components::{Login, ProductSave};
use crate::theme::tokens;
use crate::widgets::{button, tab_bar, ButtonVariant};
use eframe::egui::{self, Frame, Margin, ScrollArea, Ui};
use fleet_api::{
    AgentConfig, AgentSettings, DefInfo, DefKind, EffectiveSettings, PersonaList, Product,
    ProductOverride, RepoFleet, Status, Task,
};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ManagementTab {
    Overview,
    Products,
    Personas,
    Instructions,
    AgentSettings,
    Orchestrators,
    Specialists,
    Skills,
    Workflow,
}

const TABS: [(ManagementTab, &str); 9] = [
    (ManagementTab::Overview, "Overview"),
    (ManagementTab::Products, "Products"),
    (ManagementTab::Personas, "Personas"),
    (ManagementTab::Instructions, "Instructions"),
    (ManagementTab::AgentSettings, "Agent settings"),
    (ManagementTab::Orchestrators, "Orchestrators"),
    (ManagementTab::Specialists, "Specialists"),
    (ManagementTab::Skills, "Skills"),
    (ManagementTab::Workflow, "Workflow"),
];

/// What the user asked for in Management; the app carries it out.
#[derive(Clone, Debug, PartialEq)]
pub enum ManagementAction {
    NewPersona,
    ResumePersona(String),
    DeletePersona(String),
    RenamePersona(String, String),
    /// Persona id, commit name, commit email.
    UpdateIdentity(String, String, String),
    Relogin {
        id: String,
        which: Login,
    },
    /// Selecting a persona (or asking again) checks its logins.
    Check(String),
    /// Save token was clicked for this persona id; the token itself stays in the state's field until the app takes it.
    SubmitPat(String),
    /// A repo row's roster was saved: the product it belongs to, and the roster (usable specialists only).
    SaveRepoFleet {
        product: String,
        repo: String,
        fleet: RepoFleet,
    },
    /// A product name on Overview was clicked.
    SetActiveProduct(String),
    NewProduct,
    SaveProduct(ProductSave),
    /// Save overrides was clicked: the product's whole list, which replaces what the daemon stores.
    SaveOverrides {
        product: String,
        overrides: Vec<ProductOverride>,
    },
    /// The Internal resources section of this product was opened: its branches are fetched.
    OpenInternal(String),
    /// Product, then the branch picked: its files are fetched.
    InternalBranch(String, String),
    /// Product, branch, then the file picked: its text and last commit are fetched.
    InternalFile(String, String, String),
    /// Confirm delete was clicked on this product id.
    DeleteProduct(String),
    SaveInstructions,
    RevertInstructions,
    SaveSettings,
    RevertSettings,
    /// A definition was selected: its file is fetched.
    OpenDefinition(DefKind, String),
    /// Save this text as the user's file of the definition.
    SaveDefinition(DefKind, String, String),
    /// Reset was clicked on a definition overriding a built-in; the app asks first.
    ResetDefinition(DefKind, String),
}

pub(crate) fn header(ui: &mut Ui) {
    title(ui, View::Management);
}

/// What the tabs read and do not own: the daemon's lists and the clock.
pub(crate) struct ManagementData<'a> {
    /// `None` until the daemon has answered.
    pub list: Option<&'a PersonaList>,
    pub defs: Option<&'a [DefInfo]>,
    pub products: Option<&'a [Product]>,
    /// The settings without any product's overrides, for the value an override replaces.
    pub effective: Option<&'a EffectiveSettings>,
    pub tasks: &'a [Task],
    /// Each live agent's board status, by agent id (which is its task's id).
    pub statuses: &'a HashMap<String, Status>,
    pub active: Option<&'a str>,
    /// The clock in seconds.
    pub now: i64,
}

/// The tabs' own state; the app owns it and the calls.
pub(crate) struct ManagementStates<'a> {
    pub personas: &'a mut PersonasState,
    pub config: &'a mut ConfigState,
    pub defs: &'a mut DefsState,
    pub overview: &'a mut OverviewState,
    pub products: &'a mut ProductsState,
}

/// The tab bar and the selected tab. `tab` is the app's, so the last tab is kept while the app runs.
pub(crate) fn main(
    ui: &mut Ui,
    tab: &mut ManagementTab,
    data: &ManagementData,
    st: ManagementStates,
) -> Option<ManagementAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    egui::CentralPanel::default()
        .frame(Frame::new().fill(p.bg))
        .show(ui, |ui| {
            if let Some(picked) = tab_bar(ui, &TABS, *tab) {
                *tab = picked;
            }
            match *tab {
                ManagementTab::Overview => action = overview::overview(ui, st.overview, data),
                ManagementTab::Products => {
                    let personas = data.list.map_or(&[][..], |l| &l.personas[..]);
                    action = products::products(
                        ui,
                        data.products,
                        personas,
                        data.defs,
                        data.effective,
                        data.now,
                        st.products,
                    )
                }
                ManagementTab::Personas => action = personas::personas(ui, data.list, st.personas),
                ManagementTab::Instructions => action = instructions::instructions(ui, st.config),
                ManagementTab::AgentSettings => {
                    action = agent_settings::agent_settings(ui, st.config)
                }
                ManagementTab::Orchestrators => {
                    action = definitions::definitions(ui, DefKind::Orchestrator, data.defs, st.defs)
                }
                ManagementTab::Specialists => {
                    action = definitions::definitions(ui, DefKind::Specialist, data.defs, st.defs)
                }
                ManagementTab::Skills => {
                    action = definitions::definitions(ui, DefKind::Skill, data.defs, st.defs)
                }
                ManagementTab::Workflow => workflow::workflow(ui),
            }
        });
    action
}

/// Which half of the fleet-wide config a save was for; the other half is sent as last saved, so an unsaved draft on
/// the other tab never goes out by accident.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigPart {
    Instructions,
    Settings,
}

/// The settings as being edited. Env is a list of rows, not a map, so a row with an empty or repeated key stays
/// while it is typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsDraft {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl From<&AgentSettings> for SettingsDraft {
    fn from(s: &AgentSettings) -> Self {
        SettingsDraft {
            allow: s.allow.clone(),
            deny: s.deny.clone(),
            env: s.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        }
    }
}

impl SettingsDraft {
    /// The settings to send. Blank rows are dropped; a nameless or repeated variable is an error to show, not send.
    pub fn to_settings(&self) -> Result<AgentSettings, String> {
        let rules = |rows: &[String]| {
            rows.iter()
                .map(|r| r.trim().to_string())
                .filter(|r| !r.is_empty())
                .collect::<Vec<_>>()
        };
        let mut env = BTreeMap::new();
        for (key, value) in &self.env {
            let key = key.trim();
            if key.is_empty() {
                if value.is_empty() {
                    continue;
                }
                return Err("An environment variable needs a name.".into());
            }
            if env.insert(key.to_string(), value.clone()).is_some() {
                return Err(format!("{key} is set twice."));
            }
        }
        Ok(AgentSettings {
            allow: rules(&self.allow),
            deny: rules(&self.deny),
            env,
        })
    }
}

/// Where the fetch of the fleet-wide config stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigLoad {
    NotLoaded,
    Loading,
    Loaded,
    Failed,
}

/// The Instructions and Agent settings tabs' state: the config as the daemon has it, and the drafts. The drafts
/// outlive leaving a tab.
pub struct ConfigState {
    pub saved: AgentConfig,
    pub instructions: String,
    pub settings: SettingsDraft,
    pub load: ConfigLoad,
    /// The daemon has answered at least once (with the config or an error), so the editors can show.
    pub answered: bool,
    pub load_error: Option<String>,
    pub instructions_error: Option<String>,
    pub settings_error: Option<String>,
    /// The save in flight.
    pub saving: Option<ConfigPart>,
}

impl Default for ConfigState {
    fn default() -> Self {
        ConfigState {
            saved: AgentConfig::default(),
            instructions: String::new(),
            settings: SettingsDraft::default(),
            load: ConfigLoad::NotLoaded,
            answered: false,
            load_error: None,
            instructions_error: None,
            settings_error: None,
            saving: None,
        }
    }
}

impl ConfigState {
    pub fn instructions_dirty(&self) -> bool {
        self.instructions != self.saved.instructions
    }

    pub fn settings_dirty(&self) -> bool {
        self.settings != SettingsDraft::from(&self.saved.settings)
    }

    pub fn revert_instructions(&mut self) {
        self.instructions = self.saved.instructions.clone();
        self.instructions_error = None;
    }

    pub fn revert_settings(&mut self) {
        self.settings = SettingsDraft::from(&self.saved.settings);
        self.settings_error = None;
    }

    /// The daemon's config arrived. A part being edited keeps its draft; a clean one follows the daemon.
    pub fn loaded(&mut self, cfg: AgentConfig) {
        let (keep_instructions, keep_settings) = (self.instructions_dirty(), self.settings_dirty());
        self.saved = cfg;
        if !keep_instructions {
            self.revert_instructions();
        }
        if !keep_settings {
            self.revert_settings();
        }
        self.load = ConfigLoad::Loaded;
        self.answered = true;
        self.load_error = None;
    }

    /// A save landed: the daemon's copy is the saved one. The part that was saved takes it as its draft; the other
    /// keeps its draft if it was being edited.
    pub fn saved(&mut self, cfg: AgentConfig) {
        let part = self.saving.take();
        let keep_instructions = part != Some(ConfigPart::Instructions) && self.instructions_dirty();
        let keep_settings = part != Some(ConfigPart::Settings) && self.settings_dirty();
        self.saved = cfg;
        if !keep_instructions {
            self.revert_instructions();
        }
        if !keep_settings {
            self.revert_settings();
        }
    }
}

/// Save and Revert. Revert is enabled only while the draft differs from what is saved; Save also when `force`
/// (a settings file that is broken on disk is repaired by saving). Returns (save, revert) clicked.
pub(super) fn save_bar(ui: &mut Ui, dirty: bool, force: bool, busy: bool) -> (bool, bool) {
    let mut out = (false, false);
    ui.horizontal(|ui| {
        let save = ui
            .add_enabled_ui((dirty || force) && !busy, |ui| {
                button(ui, "Save", ButtonVariant::Primary)
            })
            .inner;
        let revert = ui
            .add_enabled_ui(dirty, |ui| button(ui, "Revert", ButtonVariant::Default))
            .inner;
        out = (
            save.clicked() && (dirty || force) && !busy,
            revert.clicked() && dirty,
        );
        personas::note(ui, "Applies to agents started after saving.");
    });
    out
}

/// The frame both config tabs share: a note while the first answer is awaited, the load error (if any), then `body`.
pub(super) fn config_pane(
    ui: &mut Ui,
    cfg: &mut ConfigState,
    width: f32,
    body: impl FnOnce(&mut Ui, &mut ConfigState) -> Option<ManagementAction>,
) -> Option<ManagementAction> {
    let mut action = None;
    Frame::new()
        .inner_margin(Margin::symmetric(22, 18))
        .show(ui, |ui| {
            if !cfg.answered {
                personas::note(ui, "Loading…");
                return;
            }
            ScrollArea::vertical().show(ui, |ui| {
                ui.set_max_width(width);
                if let Some(e) = &cfg.load_error {
                    personas::error_line(ui, e);
                    ui.add_space(8.0);
                }
                action = body(ui, cfg);
            });
        });
    action
}
