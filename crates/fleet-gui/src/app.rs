//! The application: owns the board data, receives what the background threads send, and draws everything.
//! The UI thread never touches the network. Each call runs on its own thread and answers with an `AppResult`.
use crate::components::{
    archive_dialog, centred_card, end_session_dialog, footer, modal_frame, new_task_dialog,
    panel_layout, persona_labels, preview_pane, AgentDetails, ArchiveDialog, CardState, Command,
    Confirm, ConfirmDialog, DeletePersonaDialog, DiffView, EndSessionDialog, FolderPicker,
    FooterAction, Login, PanelLayout, PassportAction, PersonaSetup, PickMode, PlanView,
    PreviewAction, PreviewState, PreviewTab, ReleaseFilter, Reply, RepoPanel, RequestDraft,
    SpawnAction, SpawnState,
};
use crate::cues::{Cue, CueFile};
use crate::drawer::{drawer, DrawerAction, DrawerStatus, Switcher};
use crate::messages::{Message, Messages};
use crate::shell::{restart_daemon, DaemonCtl};
use crate::theme::{self, layout, tokens};
use crate::views::management::{
    self, ConfigState, DefsState, ManagementTab, OverviewState, PersonasState, ProductsState,
};
use crate::views::releases::{self, ReleasesState};
use crate::views::settings::{
    self, DaemonRows, FolderField, GeneralChange, LogFeed, LogPane, LogView, SettingsAction,
    SettingsState, SettingsTab,
};
use crate::views::tasks::{self, Live, TaskFilter, TasksAction, TasksState};
use crate::views::{kanban, View};
use crate::widgets::{button, ButtonVariant};
use eframe::egui::{self, Align, Frame, Layout, Margin, Panel, Rect, RichText, Ui};
use fleet_api::{
    format_bytes, ActionDef, Agent, ArchivePlan, ArchiveReport, DefFile, DefInfo, DefKind,
    DiffReport, Event, EventKind, FleetView, NewPersona, Persona, PersonaList, PersonaUpdate,
    Product, ProductList, Release, RepoInfo, Snapshot, SpawnRequest, SpawnResponse, Status,
    StopCause, TaskList, UiSchema, PLAN_CHANGED,
};
use fleet_client::board::apply;
use fleet_client::config::{
    self, CloseChoice, FleetDisplay, GuiSettings, SettingsWriter, ThemeChoice,
};
use fleet_client::daemon_control::{Started, StopError};
use fleet_client::http::{FleetClient, FleetError};
use fleet_client::login;
use fleet_client::schema::{field_value, groups, visible_actions, FieldValue};
use fleet_client::stream::{
    spawn_event_stream, spawn_pane_stream, PaneMsg, StreamHandle, StreamMsg,
};
use fleet_log::{Entry, LogHandle};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

mod fleet;
mod products;
mod setup;

pub use fleet::{DefCall, FleetCall};
pub use products::{ProductCall, ProductFormState, ReleaseCall, TaskCall};

/// How often a quiet window redraws so the elapsed-time labels stay current (the web UI refreshes every 15 s too).
pub const REFRESH: std::time::Duration = std::time::Duration::from_secs(15);

/// How often the app fetches the tasks list, whatever the view: the Kanban header shows its counts. Sizes are
/// measured by the daemon in the background, so a fetch is cheap and a new measurement shows up within this.
pub const TASKS_EVERY: std::time::Duration = std::time::Duration::from_secs(10);

/// How soon after a Refresh the list is fetched again: the daemon measures in the background, so the list the
/// Refresh itself brings back still has the old sizes.
pub const REFETCH_AFTER_REFRESH: std::time::Duration = std::time::Duration::from_secs(3);

/// Most log entries kept in memory; the file on disk has the rest.
const LOG_CAP: usize = 5_000;

/// Starts the footer's message for a spawn that failed after its dialog was closed, so no error line was in sight.
const SPAWN_FAILED: &str = "Could not start the agent:";

/// How often the setup's Log in step checks the persona: the user logs in in another window.
pub const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(3);

/// `LoginOpened`'s error when no terminal was found: the setup then shows the script to run by hand.
pub const NO_TERMINAL: &str = "No terminal found";

/// Where the persona list's fetch stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PersonasLoad {
    Idle,
    Loading,
    Failed,
}

/// A persona call the setup made, recorded for tests (a token is never recorded).
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub enum PersonaCall {
    Create(NewPersona),
    Check(String),
    Delete(String),
    Login(Login),
    Pat(String),
    Update(String, PersonaUpdate),
    Repos(String),
}

/// A call on the fleet-wide agent config, recorded for tests.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub enum AgentConfigCall {
    Load,
    Save(fleet_api::AgentConfigUpdate),
}

/// The answer to one background call.
// One value lives at a time, briefly; boxing `Spawn` would only churn every construction site.
#[allow(clippy::large_enum_variant)]
pub enum AppResult {
    Schema(Result<UiSchema, String>),
    Repos(Result<Vec<RepoInfo>, String>),
    Action(Result<Option<String>, String>),
    /// The Stop action finished for the agent whose task is `task`; a success is announced by name.
    Stopped {
        goal: String,
        result: Result<Option<String>, String>,
    },
    /// A reply sent to the agent with this id finished; `sent` is the text that went, so the draft is cleared only
    /// if it still holds exactly that.
    Replied {
        agent_id: String,
        sent: String,
        result: Result<Option<String>, String>,
    },
    /// One `add_repo` finished for the agent with this id.
    RepoAdded(String, Result<(), String>),
    /// A grant or refusal finished for the agent with this id.
    RequestDecided(String, Result<(), String>),
    Spawn(Result<SpawnResponse, String>),
    /// The diff for the agent with this id.
    Diff(String, Result<DiffReport, String>),
    /// Save wrote this tasks folder to the daemon's config; `Restarted` follows. On an error nothing was
    /// written and nothing follows.
    RootSaved(Result<PathBuf, String>),
    /// Save wrote this personas folder to the daemon's config, as `RootSaved` does for the tasks folder.
    PersonasDirSaved(Result<PathBuf, String>),
    /// A Settings → General change reached `gui.toml`, or could not (the message says what was being saved).
    SettingsSaved(Result<(), String>),
    /// The daemon restart that Save or Restart daemon asked for. `AlreadyRunning` means it could not be stopped
    /// (no pidfile) and the old daemon still runs.
    Restarted(Result<Started, String>),
    /// The Tasks view's list.
    Tasks(Result<TaskList, String>),
    /// The archive plan for this task or orphan id, answering the request with this sequence number.
    ArchivePlan(String, u32, Result<ArchivePlan, String>),
    /// The archive of this task or orphan id finished.
    Archived(String, Result<ArchiveReport, String>),
    /// End session on this orphan id finished.
    SessionEnded(String, Result<(), String>),
    /// Mark done or a release move on this task finished: the task as it is now, or why the daemon refused.
    TaskChanged(String, Result<fleet_api::Task, String>),
    Personas(Result<PersonaList, String>),
    /// The listed products, with their repos and releases.
    Products(Result<ProductList, String>),
    /// New product finished: the product as created, or why it was refused.
    ProductSaved(Result<Product, String>),
    /// Saving an edited product (Management → Products) finished, part by part.
    ProductEdited(management::ProductEdit),
    /// Delete product finished: the product id, and the daemon's reason when it refused.
    ProductDeleted(String, Result<(), String>),
    /// The Releases form's create or edit finished: the release as stored, or the daemon's reason.
    ReleaseSaved(Result<fleet_api::Release, String>),
    /// Mark released, Unrelease or Delete finished.
    ReleaseChanged(Result<(), String>),
    /// The setup's create finished.
    /// The fleet-wide agent config (Management → Instructions and Agent settings).
    AgentConfig(Result<fleet_api::AgentConfig, String>),
    /// A save of it finished: the daemon's copy, or why it refused.
    AgentConfigSaved(Result<fleet_api::AgentConfig, String>),
    PersonaSaved(Result<Persona, String>),
    PersonaChecked(Result<Persona, String>),
    /// The delete of this persona id finished.
    PersonaDeleted(String, Result<(), String>),
    /// An update of a persona finished; `relogin` is the login to open next (a switch of GitHub auth).
    PersonaUpdated(Result<Persona, String>, Option<Login>),
    /// The active product's repos, fetched for New task under this persona id.
    PersonaRepos(String, Result<Vec<RepoInfo>, String>),
    /// A login terminal was opened for the persona with this id, or could not be (`NO_TERMINAL` when none was found).
    LoginOpened(String, Result<(), String>),
    /// A token login finished for the persona with this id.
    PatSaved(String, Result<(), String>),
    /// Orchestrator types, specialists and skills, each with its error when it does not load.
    Definitions(Result<Vec<DefInfo>, String>),
    /// A repo's roster was saved for the product with this id: the repo as now stored.
    RepoFleetSaved(String, Result<RepoInfo, String>),
    /// The breakdowns and gate runs of this task id.
    FleetView(String, Result<FleetView, String>),
    /// Approve or Request changes on this task id's breakdown finished.
    BreakdownDecided(String, Result<(), String>),
    /// Stopping this task id finished: how many agents it stopped.
    TaskStopped(String, Result<u32, String>),
    /// A fresh orchestrator for this task id, or why there is none.
    OrchestratorRestarted(String, Result<Agent, String>),
    /// A definition's file, for Management.
    Definition(DefKind, String, Result<DefFile, String>),
    /// A save of a definition's file finished; an error is the daemon's validation message.
    DefinitionSaved(DefKind, String, Result<DefInfo, String>),
    /// Reset to built-in finished for this definition.
    DefinitionReset(DefKind, String, Result<(), String>),
}

/// What asked for the restart in flight; they differ in what a daemon that could not be stopped means.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RestartFor {
    Save(Folder),
    Restart,
}

/// The Settings → General folder a Save writes to the daemon's config.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Folder {
    Tasks,
    Personas,
}

/// A repo request's draft. `for_repo` is the request it was prefilled for: a new request starts afresh.
struct RequestText {
    for_repo: String,
    target: String,
    reason: String,
}

/// What the Browse… pickers need to know about the machine.
struct Locations {
    /// `gui.toml`, where the last folder picked from is remembered.
    settings: PathBuf,
    /// Folders under it are agents' copies, never offered as a repo.
    tasks_root: PathBuf,
    home: PathBuf,
}

impl Locations {
    /// Excludes every task folder as well as the root: a task made before the root moved keeps its folder
    /// under the old one.
    fn repo_picker(&self, agents: &[Agent]) -> FolderPicker {
        let mut exclude = vec![self.tasks_root.clone()];
        // Every path starts with an empty one, so an agent without a real folder must not grey out everything.
        exclude.extend(
            agents
                .iter()
                .map(|a| PathBuf::from(&a.workdir))
                .filter(|d| d.is_absolute()),
        );
        FolderPicker::new(
            PickMode::Repo,
            self.settings.clone(),
            exclude,
            self.home.clone(),
        )
    }
}

/// Where the shell's rows landed last frame, for tests that check the drawer and content rows line up.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct LayoutRects {
    pub drawer_header: Rect,
    pub header: Rect,
    /// The board's strip under the header; `Rect::NOTHING` on views without one.
    pub strip: Rect,
    pub drawer_footer: Rect,
    pub footer: Rect,
}

pub struct FleetApp {
    client: Arc<FleetClient>,
    ctx: egui::Context,
    snapshot: Snapshot,
    schema: Option<UiSchema>,
    repos: Vec<RepoInfo>,
    /// `None` until the daemon has answered; New task then offers Solo only.
    definitions: Option<Vec<DefInfo>>,
    connected: bool,
    load_error: Option<String>,
    messages: Messages,
    filter: Vec<String>,
    /// The board's persona filter, by shown name; not saved, and not the active persona.
    persona_filter: Vec<String>,
    cards: HashMap<String, CardState>,
    spawn: SpawnState,
    /// The tasks folder changed while a spawn was in flight, so New task's picker still excludes the old one.
    spawn_picker_stale: bool,
    preview: Option<PreviewState>,
    /// The user chose the full-board panel; never saved, and reset when the panel closes.
    panel_fullscreen: bool,
    /// A widget had keyboard focus when the last frame ended.
    field_had_focus: bool,
    /// The side panel's width on the last frame, to tell whether it leaves a quarter of the board.
    preview_width: f32,
    /// The half-written reply for each agent id, kept while the user looks at other cards and until a send of it
    /// succeeds.
    replies: HashMap<String, String>,
    /// Agents with a reply in flight: a second Send would type the answer into the session twice.
    replying: HashSet<String>,
    /// The grant target and refusal reason typed for each agent with a pending repo request.
    requests: HashMap<String, RequestText>,
    /// The Browse… dialog beside each pending request's target, kept with its draft.
    grant_pickers: HashMap<String, FolderPicker>,
    locations: Locations,
    /// Agents with a grant or refusal in flight: a second would find the request gone and show a false error.
    deciding: HashSet<String>,
    /// Repos still to add per agent. They go one at a time, so their results reach the footer in order.
    add_queue: HashMap<String, VecDeque<String>>,
    /// Agents with an `add_repo` in flight.
    adding: HashSet<String>,
    /// Agents whose in-flight decision is a grant: only that one is announced as "Repo granted".
    granting: HashSet<String>,
    /// Starts and stops the daemon for Settings; `None` (tests, embedding) leaves those settings out.
    daemon: Option<DaemonCtl>,
    /// Settings → General's Tasks folder field and its Browse… dialog.
    root_text: String,
    root_picker: FolderPicker,
    /// Settings → General's Personas folder field, its Browse… dialog, and the folder the daemon was started with.
    personas_text: String,
    personas_picker: FolderPicker,
    personas_dir: PathBuf,
    /// The saved field's text when Save was clicked: the saved folder replaces only that, never a later edit.
    saving_text: Option<String>,
    restarting: Option<RestartFor>,
    /// A Save or Restart daemon waiting for the user to accept interrupting a pending repo request.
    confirm: Option<RestartFor>,
    /// The pid of a daemon a restart started, until the caller has been told.
    restarted: Option<u32>,
    view: View,
    /// The Settings tab last shown, so coming back to Settings opens it again.
    settings_tab: SettingsTab,
    /// The Management tab last shown, likewise.
    management_tab: ManagementTab,
    theme: ThemeChoice,
    close_choice: Option<CloseChoice>,
    sticky_alerts: bool,
    /// Where Settings → General saves; `None` keeps the choices for this run only.
    settings_path: Option<PathBuf>,
    layout: LayoutRects,
    clock: Box<dyn Fn() -> i64>,
    results_tx: Sender<AppResult>,
    /// False in tests that inject every result themselves: a real call answering too would race them.
    calls: bool,
    results_rx: Receiver<AppResult>,
    stream_rx: Option<Receiver<StreamMsg>>,
    pane_rx: Option<Receiver<PaneMsg>>,
    // Held for their Drop: dropping a handle stops its thread.
    _events: Option<StreamHandle>,
    pane: Option<StreamHandle>,
    log_rx: Option<Receiver<Entry>>,
    log: Vec<Entry>,
    /// How many entries have reached `log`, for the Log tab (`log` is capped, so its length alone says too little).
    log_pushed: u64,
    log_view: LogView,
    tasks: TasksState,
    /// A tasks fetch is in flight; `tasks_stale` asks for another once it lands, because something changed
    /// after it left, and `tasks_refresh_queued` makes that one a refresh. Never two at once: an older list
    /// landing last would overwrite a newer one.
    tasks_fetching: bool,
    tasks_stale: bool,
    tasks_refresh_queued: bool,
    /// Fetches sent and how many of them were refreshes, for tests.
    tasks_sent: (u32, u32),
    /// When (in `ui.input` time) the tasks list is next polled.
    tasks_due: f64,
    archive: Option<ArchiveDialog>,
    /// How many plan requests were made, sent or not; also the newest request's sequence number.
    plans_sent: u32,
    /// Archives in flight: id to the name the footer will use.
    archiving: HashMap<String, String>,
    end_session: Option<EndSessionDialog>,
    /// Why the log file is off, until the notice about it has been logged (once, on the first frame).
    file_error: Option<String>,
    /// `None` until the daemon has answered.
    personas: Option<PersonaList>,
    personas_load: PersonasLoad,
    /// The first-run folders step came before the persona wizard; false on an upgrade, which has its config.
    folders_step: bool,
    /// Persona ids the list was re-fetched for because an agent named them and the list did not: once per id, so
    /// an id that stays unknown is not asked about on every event.
    personas_refetched: HashSet<String>,
    /// Ids the persona fetch in flight was asked for: settled into `personas_refetched` by its answer, forgotten
    /// on a failure so the next look asks again.
    personas_refetching: Vec<String>,
    /// How many persona list fetches were asked for.
    personas_sent: u32,
    /// The last load failure, so a retry that fails the same way says nothing new.
    personas_error: Option<String>,
    /// The persona setup on screen, as a modal over the board.
    setup: Option<PersonaSetup>,
    /// The persona whose check is in flight, and when (in `ui.input` time) the setup's next one is due.
    checking: Option<String>,
    check_due: f64,
    /// The persona new tasks default to, as saved in `gui.toml`.
    active_persona: Option<String>,
    /// How the board draws fleets, as saved in `gui.toml`.
    fleet_display: FleetDisplay,
    persona_calls: Vec<PersonaCall>,
    /// `None` until the daemon has answered.
    products: Option<ProductList>,
    products_load: PersonasLoad,
    /// The list was asked for again while a fetch was in flight.
    products_stale: bool,
    products_error: Option<String>,
    /// The product the drawer, board and New task act on, as saved; see `active_product`.
    active_product: Option<String>,
    /// The board's release filter; not saved.
    release_filter: ReleaseFilter,
    /// New product on screen, as a modal.
    product_form: Option<ProductFormState>,
    product_calls: Vec<ProductCall>,
    task_calls: Vec<TaskCall>,
    /// The Releases view: its form and the pending delete.
    releases: ReleasesState,
    release_calls: Vec<ReleaseCall>,
    /// The terminal emulator Open in terminal uses, looked up once at start-up.
    terminal: Option<Vec<String>>,
    /// The command lines handed to the terminal, for tests: they run without a daemon or a display.
    terminal_launches: Vec<String>,
    /// Management → Personas: the selection, its fields and its re-login.
    management: PersonasState,
    overview: OverviewState,
    products_tab: ProductsState,
    /// Management → Instructions and Agent settings: the daemon's config and the drafts.
    agent_config: ConfigState,
    agent_config_calls: Vec<AgentConfigCall>,
    delete_persona: Option<DeletePersonaDialog>,
    /// Each orchestrated task's fleet view, by task id, as last fetched (an error only until one arrives).
    fleet_views: HashMap<String, Result<FleetView, String>>,
    /// Fleet views in flight; `fleet_stale` asks for another once one lands, so an older view never lands last.
    fleet_fetching: HashSet<String>,
    fleet_stale: HashSet<String>,
    /// The Request changes text per task id, kept until a decision is sent.
    breakdown_drafts: HashMap<String, String>,
    /// Decisions in flight, by task id: true for an approval.
    breakdown_deciding: HashMap<String, bool>,
    /// Decisions that landed while the view on screen still shows the version as proposed, by task id.
    breakdown_decided: HashSet<String>,
    /// The Stop task confirmation on screen, with its task id.
    stop_task: Option<(String, ConfirmDialog)>,
    fleet_calls: Vec<FleetCall>,
    /// Management → Orchestrators, Specialists and Skills.
    defs: DefsState,
    /// Reset to built-in, asked and not yet answered.
    reset_definition: Option<((DefKind, String), ConfirmDialog)>,
    def_calls: Vec<DefCall>,
    /// A scripted run's cue file; `None` outside the demo recorder.
    cues: Option<CueFile>,
}

/// `req` started in `product`, which must be picked: the dialog already chose each repo's start.
fn spawn_in_product(
    c: &FleetClient,
    product: Option<String>,
    mut req: SpawnRequest,
) -> Result<SpawnResponse, FleetError> {
    req.product = product.ok_or_else(|| FleetError(products::PICK_PRODUCT.into()))?;
    c.spawn(&req)
}

fn wall_clock() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl FleetApp {
    /// Starts the event stream and loads the schema and repo list in the background.
    /// `settings_path` is `gui.toml`; `tasks_root` and `home` are what the Browse… pickers exclude and start from;
    /// `personas_dir` is the daemon's, for Settings → General.
    pub fn new(
        client: FleetClient,
        ctx: egui::Context,
        settings_path: PathBuf,
        tasks_root: PathBuf,
        personas_dir: PathBuf,
        home: PathBuf,
    ) -> FleetApp {
        crate::theme::apply(&ctx);
        let mut app = FleetApp::detached(client, ctx.clone())
            .with_locations(settings_path.clone(), tasks_root, personas_dir, home)
            .with_settings(settings_path);
        let (tx, rx) = mpsc::channel();
        let wake = ctx.clone();
        app._events = Some(spawn_event_stream(
            &app.client_url(),
            tx,
            Arc::new(move || wake.request_repaint()),
        ));
        app.stream_rx = Some(rx);
        app.load();
        app
    }

    /// Where `gui.toml`, the tasks and personas folders and home are, for the Browse… pickers and Settings →
    /// General.
    pub fn with_locations(
        mut self,
        settings: PathBuf,
        tasks_root: PathBuf,
        personas_dir: PathBuf,
        home: PathBuf,
    ) -> FleetApp {
        self.root_text = config::shorten_home(&tasks_root, &home);
        self.personas_text = config::shorten_home(&personas_dir, &home);
        self.personas_dir = personas_dir;
        // No exclusion: the current folder is a fine place to start looking for the next one.
        let picker = || FolderPicker::new(PickMode::Folder, settings.clone(), vec![], home.clone());
        self.root_picker = picker();
        self.personas_picker = picker();
        self.locations = Locations {
            settings,
            tasks_root,
            home,
        };
        self.spawn = SpawnState::new(self.locations.repo_picker(&self.snapshot.agents));
        self
    }

    /// Lets Settings restart the daemon and show what it knows about it.
    pub fn with_daemon(mut self, d: DaemonCtl) -> FleetApp {
        self.daemon = Some(d);
        self
    }

    /// Whether the window showed the first-run folders step, which sets how the persona wizard counts its steps.
    pub fn with_folders_step(mut self, shown: bool) -> FleetApp {
        self.folders_step = shown;
        self
    }

    /// Takes cues from `cues` every frame, for scripted runs such as the demo recording.
    pub fn with_cues(mut self, cues: CueFile) -> FleetApp {
        self.cues = Some(cues);
        self
    }

    /// Once after a restart this window made, the pid of the daemon it started: that one is this window's to stop.
    pub fn take_restarted(&mut self) -> Option<u32> {
        self.restarted.take()
    }

    /// An app with no background threads, for tests: feed it `handle_stream` and `handle_result` yourself. Its
    /// Browse… pickers know no folders; tests that browse build their own.
    pub fn detached(client: FleetClient, ctx: egui::Context) -> FleetApp {
        let (results_tx, results_rx) = mpsc::channel();
        let locations = Locations {
            settings: PathBuf::new(),
            tasks_root: PathBuf::new(),
            home: PathBuf::new(),
        };
        FleetApp {
            client: Arc::new(client),
            ctx,
            snapshot: Snapshot {
                seq: 0,
                agents: vec![],
            },
            schema: None,
            repos: vec![],
            definitions: None,
            connected: false,
            load_error: None,
            messages: Messages::new(true),
            filter: vec![],
            persona_filter: vec![],
            cards: HashMap::new(),
            spawn: SpawnState::new(locations.repo_picker(&[])),
            spawn_picker_stale: false,
            preview: None,
            panel_fullscreen: false,
            field_had_focus: false,
            preview_width: 420.0,
            replies: HashMap::new(),
            replying: HashSet::new(),
            requests: HashMap::new(),
            grant_pickers: HashMap::new(),
            locations,
            deciding: HashSet::new(),
            add_queue: HashMap::new(),
            adding: HashSet::new(),
            granting: HashSet::new(),
            daemon: None,
            root_text: String::new(),
            root_picker: FolderPicker::new(
                PickMode::Folder,
                PathBuf::new(),
                vec![],
                PathBuf::new(),
            ),
            personas_text: String::new(),
            personas_picker: FolderPicker::new(
                PickMode::Folder,
                PathBuf::new(),
                vec![],
                PathBuf::new(),
            ),
            personas_dir: PathBuf::new(),
            saving_text: None,
            restarting: None,
            confirm: None,
            restarted: None,
            view: View::Kanban,
            settings_tab: SettingsTab::General,
            management_tab: ManagementTab::Overview,
            theme: ThemeChoice::default(),
            close_choice: None,
            sticky_alerts: true,
            settings_path: None,
            layout: LayoutRects {
                drawer_header: Rect::NOTHING,
                header: Rect::NOTHING,
                strip: Rect::NOTHING,
                drawer_footer: Rect::NOTHING,
                footer: Rect::NOTHING,
            },
            clock: Box::new(wall_clock),
            results_tx,
            calls: true,
            results_rx,
            stream_rx: None,
            pane_rx: None,
            _events: None,
            pane: None,
            log_rx: None,
            log: Vec::new(),
            log_pushed: 0,
            log_view: LogView::new(),
            tasks: TasksState::default(),
            tasks_fetching: false,
            tasks_stale: false,
            tasks_refresh_queued: false,
            tasks_sent: (0, 0),
            tasks_due: f64::NEG_INFINITY,
            archive: None,
            plans_sent: 0,
            archiving: HashMap::new(),
            end_session: None,
            file_error: None,
            personas: None,
            setup: None,
            management: PersonasState::default(),
            overview: OverviewState::default(),
            products_tab: ProductsState::default(),
            agent_config: ConfigState::default(),
            agent_config_calls: Vec::new(),
            delete_persona: None,
            checking: None,
            personas_load: PersonasLoad::Idle,
            folders_step: true,
            personas_refetched: HashSet::new(),
            personas_refetching: Vec::new(),
            personas_sent: 0,
            personas_error: None,
            check_due: f64::NEG_INFINITY,
            active_persona: None,
            fleet_display: FleetDisplay::default(),
            persona_calls: Vec::new(),
            products: None,
            products_load: PersonasLoad::Idle,
            products_stale: false,
            products_error: None,
            active_product: None,
            release_filter: ReleaseFilter::All,
            product_form: None,
            product_calls: Vec::new(),
            task_calls: Vec::new(),
            releases: ReleasesState::default(),
            release_calls: Vec::new(),
            terminal: login::terminal(),
            terminal_launches: Vec::new(),
            fleet_views: HashMap::new(),
            fleet_fetching: HashSet::new(),
            fleet_stale: HashSet::new(),
            breakdown_drafts: HashMap::new(),
            breakdown_deciding: HashMap::new(),
            breakdown_decided: HashSet::new(),
            stop_task: None,
            fleet_calls: Vec::new(),
            defs: DefsState::default(),
            reset_definition: None,
            def_calls: Vec::new(),
            cues: None,
        }
    }

    /// Collects what the process logs from now on, for the Log tab and the footer. Without it (tests) the log stays
    /// empty.
    pub fn with_log(mut self, handle: LogHandle) -> FleetApp {
        self.log_rx = Some(handle.subscribe());
        self.file_error = handle.file_error().map(str::to_string);
        self
    }

    #[doc(hidden)]
    pub fn log_file_reads(&self) -> u64 {
        self.log_view.reads()
    }

    #[doc(hidden)]
    pub fn log_entries(&self) -> &[Entry] {
        &self.log
    }

    /// Reads the saved choices from `path` and saves later changes there. `main` applies the stored theme before
    /// the first frame, so this only has to show it in Settings.
    pub fn with_settings(mut self, path: PathBuf) -> FleetApp {
        let saved = GuiSettings::load(&path);
        self.theme = saved.theme;
        self.close_choice = saved.close_choice;
        self.sticky_alerts = saved.sticky_alerts;
        self.messages.set_sticky(saved.sticky_alerts);
        self.active_persona = saved.active_persona;
        self.active_product = saved.active_product;
        self.fleet_display = saved.fleet_display;
        self.settings_path = Some(path);
        self
    }

    pub fn fleet_display(&self) -> FleetDisplay {
        self.fleet_display
    }

    /// Applies the board's fleet switch at once and saves it in the background, like a Settings choice.
    fn set_fleet_display(&mut self, display: FleetDisplay) {
        self.fleet_display = display;
        let Some(path) = &self.settings_path else {
            return;
        };
        let (tx, ctx) = (self.results_tx.clone(), self.ctx.clone());
        SettingsWriter::shared().update(
            path,
            move |s| s.fleet_display = display,
            move |saved| {
                let _ = tx.send(AppResult::SettingsSaved(
                    saved
                        .map(drop)
                        .map_err(|e| format!("Could not save the fleet display: {e}")),
                ));
                ctx.request_repaint();
            },
        );
    }

    pub fn theme_choice(&self) -> ThemeChoice {
        self.theme
    }

    /// The Settings tab shown, or last shown.
    pub fn settings_tab(&self) -> SettingsTab {
        self.settings_tab
    }

    /// The Management tab shown, or last shown.
    pub fn management_tab(&self) -> ManagementTab {
        self.management_tab
    }

    pub fn view(&self) -> View {
        self.view
    }

    pub fn set_view(&mut self, v: View) {
        if matches!(v, View::Tasks | View::Releases) && self.view != v {
            self.tasks_due = f64::NEG_INFINITY;
        }
        if v == View::Settings && self.view != View::Settings {
            // The close dialog's "Remember my choice" writes the same setting behind this copy's back.
            if let Some(path) = &self.settings_path {
                self.close_choice = SettingsWriter::shared().current(path).close_choice;
            }
        }
        if v == View::Management && self.view != View::Management {
            // The task counts change behind Management's back.
            self.fetch_personas();
            self.fetch_products();
            self.management.check_due = f64::NEG_INFINITY;
            // Loaded when first opened, not at start; a failed load is asked again on each way back in.
            self.fetch_agent_config();
        }
        if v != View::Management {
            self.delete_persona = None;
            self.reset_definition = None;
        }
        self.view = v;
    }

    /// Shows the Tasks view with `filter` applied and, when given, the task or orphan with id `select`
    /// selected (the board's counts and Details buttons open it this way).
    pub fn open_tasks(&mut self, filter: TaskFilter, select: Option<String>) {
        self.tasks.filter = filter;
        if select.is_some() && select != self.tasks.selected {
            self.tasks.selected = select;
            self.tasks.confirm_stop = false;
        }
        self.set_view(View::Tasks);
    }

    /// The task or orphan id selected in the Tasks view.
    #[doc(hidden)]
    pub fn tasks_selected(&self) -> Option<&str> {
        self.tasks.selected.as_deref()
    }

    /// How many tasks fetches were sent, and how many of them asked the daemon to refresh first.
    #[doc(hidden)]
    pub fn tasks_fetches(&self) -> (u32, u32) {
        self.tasks_sent
    }

    /// How many archive plan requests were made, sent or not.
    #[doc(hidden)]
    pub fn plan_fetches(&self) -> u32 {
        self.plans_sent
    }

    /// How many persona list fetches were asked for, sent or not.
    #[doc(hidden)]
    pub fn personas_fetches(&self) -> u32 {
        self.personas_sent
    }

    /// Where the Tasks view's list and passport landed last frame.
    #[doc(hidden)]
    pub fn tasks_rects(&self) -> (Rect, Rect) {
        self.tasks.rects
    }

    /// The close dialog saved this choice to the same file; Settings → General shows it from now on.
    pub fn set_close_choice(&mut self, choice: Option<CloseChoice>) {
        self.close_choice = choice;
    }

    #[doc(hidden)]
    pub fn layout_rects(&self) -> LayoutRects {
        self.layout
    }

    /// Applies a Settings → General choice at once and saves it in the background through `update`, which keeps
    /// what the close flow wrote to the same file. A failed save still applies for this run.
    fn apply_general(&mut self, change: GeneralChange) {
        let what: &'static str = match change {
            GeneralChange::Theme(choice) => {
                self.theme = choice;
                self.ctx.set_theme(theme::preference(choice));
                "the theme"
            }
            GeneralChange::CloseChoice(choice) => {
                self.close_choice = choice;
                "the close choice"
            }
            GeneralChange::StickyAlerts(on) => {
                self.sticky_alerts = on;
                self.messages.set_sticky(on);
                "the footer setting"
            }
        };
        let Some(path) = &self.settings_path else {
            return;
        };
        let (tx, ctx) = (self.results_tx.clone(), self.ctx.clone());
        SettingsWriter::shared().update(
            path,
            move |s| match change {
                GeneralChange::Theme(choice) => s.theme = choice,
                GeneralChange::CloseChoice(choice) => s.close_choice = choice,
                GeneralChange::StickyAlerts(on) => s.sticky_alerts = on,
            },
            move |saved| {
                let _ = tx.send(AppResult::SettingsSaved(
                    saved
                        .map(drop)
                        .map_err(|e| format!("Could not save {what}: {e}")),
                ));
                ctx.request_repaint();
            },
        );
    }

    fn client_url(&self) -> String {
        self.client.base_url().to_string()
    }

    pub fn set_clock(&mut self, f: Box<dyn Fn() -> i64>) {
        self.clock = f;
    }

    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub fn agents(&self) -> &[Agent] {
        &self.snapshot.agents
    }

    pub fn connected(&self) -> bool {
        self.connected
    }

    #[doc(hidden)]
    pub fn persona_filter(&self) -> &[String] {
        &self.persona_filter
    }

    pub fn spawn_state(&self) -> &SpawnState {
        &self.spawn
    }

    /// The agent id whose preview is open.
    pub fn preview_agent(&self) -> Option<&str> {
        self.preview.as_ref().map(|p| p.agent_id.as_str())
    }

    /// The reply typed so far for this agent; empty once a send of it succeeded.
    pub fn reply_text(&self, agent_id: &str) -> &str {
        self.replies.get(agent_id).map_or("", String::as_str)
    }

    /// Runs `job` on a new thread and delivers its answer to the UI, waking it.
    /// Makes no daemon calls: background jobs are dropped, so only the results a test hands to `handle_result`
    /// arrive.
    #[doc(hidden)]
    pub fn without_calls(mut self) -> FleetApp {
        self.calls = false;
        self
    }

    /// Replaces the terminal emulator lookup; `None` is a machine with none.
    pub fn with_terminal(mut self, term: Option<Vec<String>>) -> FleetApp {
        self.terminal = term;
        self
    }

    /// The command lines Open in terminal has run, oldest first.
    pub fn terminal_launches(&self) -> &[String] {
        &self.terminal_launches
    }

    /// Why Open in terminal cannot run for `a`, if it cannot.
    fn terminal_blocked(
        term: Option<&[String]>,
        base_url: &str,
        a: &Agent,
    ) -> Option<&'static str> {
        if a.status == Status::Stopped || a.attach.is_none() {
            Some("the agent is stopped")
        } else if !login::is_loopback_url(base_url) {
            Some("the daemon runs on another machine")
        } else if term.is_none() {
            Some("no terminal emulator found")
        } else {
            None
        }
    }

    fn open_terminal(&mut self) {
        let Some(agent) = self
            .preview
            .as_ref()
            .and_then(|p| self.snapshot.agents.iter().find(|a| a.id.0 == p.agent_id))
        else {
            return;
        };
        if Self::terminal_blocked(self.terminal.as_deref(), self.client.base_url(), agent).is_some()
        {
            return;
        }
        let (Some(attach), Some(term)) = (agent.attach.as_ref(), self.terminal.clone()) else {
            return;
        };
        let script = login::shell_join(attach);
        self.terminal_launches.push(script.clone());
        self.background(move |_| {
            AppResult::Action(
                login::open_in_terminal(&term, &script)
                    .map(|()| None)
                    .map_err(|e| format!("Could not open a terminal: {e}")),
            )
        });
    }

    fn background(&self, job: impl FnOnce(&FleetClient) -> AppResult + Send + 'static) {
        if !self.calls {
            return;
        }
        let (client, tx, ctx) = (
            self.client.clone(),
            self.results_tx.clone(),
            self.ctx.clone(),
        );
        std::thread::spawn(move || {
            let _ = tx.send(job(&client));
            ctx.request_repaint();
        });
    }

    fn load(&mut self) {
        self.load_error = None;
        self.background(|c| AppResult::Schema(c.ui_schema().map_err(|e| e.0)));
        self.fetch_definitions();
        self.refresh_repos();
        self.fetch_personas();
        self.fetch_products();
    }

    fn fetch_definitions(&mut self) {
        self.background(|c| AppResult::Definitions(c.definitions().map_err(|e| e.0)));
    }

    /// The daemon remembers a repo whenever one is attached (picked, added, granted or `git init`ed by an agent),
    /// so the list is fetched again wherever it is about to be offered or may just have grown.
    fn refresh_repos(&mut self) {
        self.background(|c| AppResult::Repos(c.repos().map_err(|e| e.0)));
        self.fetch_spawn_repos();
    }

    pub fn handle_stream(&mut self, m: StreamMsg) {
        match m {
            StreamMsg::Connected => self.connected = true,
            StreamMsg::Disconnected => self.connected = false,
            StreamMsg::Snapshot(s) => {
                self.snapshot = s;
                // A reconnect or a daemon restart: the header's counts and the orphan rows are stale.
                self.fetch_tasks(false);
            }
            StreamMsg::Event(e) => {
                // The header's counts change with these; the next poll would show them up to 10 s late.
                let counts_changed = self.announce_stop(&e)
                    || matches!(
                        e.kind,
                        EventKind::AgentCreated { .. } | EventKind::AgentRemoved { .. }
                    );
                apply(&mut self.snapshot, &e);
                if counts_changed {
                    self.fetch_tasks(false);
                }
                self.follow_fleet_event(&e);
            }
        }
        self.refetch_for_unknown_persona();
        // A preview of an agent that no longer exists has nothing to show.
        if let Some(id) = self.preview_agent() {
            if !self.snapshot.agents.iter().any(|a| a.id.0 == id) && self.snapshot.seq > 0 {
                self.close_preview();
            }
        }
    }

    /// A persona made after the list was fetched (another window, the CLI) shows as its raw id until the list is
    /// fetched again. Without a loaded list there is nothing to compare against, and a fetch under way will bring it.
    fn refetch_for_unknown_persona(&mut self) {
        let Some(list) = &self.personas else {
            return;
        };
        if self.personas_load == PersonasLoad::Loading {
            return;
        }
        let unknown = self
            .snapshot
            .agents
            .iter()
            .map(|a| a.persona.as_str())
            .find(|p| {
                !p.is_empty()
                    && !list.personas.iter().any(|l| l.id == *p)
                    && !self.personas_refetched.contains(*p)
                    && !self.personas_refetching.iter().any(|r| r == p)
            })
            .map(str::to_string);
        if let Some(id) = unknown {
            self.personas_refetching.push(id);
            self.fetch_personas();
        }
    }

    /// Posts the footer line for an agent that stopped on its own: a crash is a warning, an exit is news. Only on
    /// the change into Stopped, read before the event is applied, so a snapshot of agents that were already stopped
    /// and a repeated event say nothing. A stop by the user is announced by the Stop action's own result. True when
    /// the agent went into Stopped, whatever the cause.
    fn announce_stop(&self, e: &Event) -> bool {
        let EventKind::StatusChanged {
            id,
            status: Status::Stopped,
            activity,
            stop_cause,
            ..
        } = &e.kind
        else {
            return false;
        };
        let Some(agent) = self
            .snapshot
            .agents
            .iter()
            .find(|a| a.id == *id && a.status != Status::Stopped)
        else {
            return false;
        };
        let goal = &agent.goal;
        // The agent id lets a click on the line find the card.
        match stop_cause {
            Some(StopCause::Crashed) => {
                // The card says "crashed while …"; after "crashed:" the line names only the last activity.
                let last = activity.strip_prefix("crashed while ").unwrap_or(activity);
                tracing::warn!(notify = "warning", agent_id = %id.0, "{goal} crashed: {last}")
            }
            Some(StopCause::Exited) => {
                tracing::info!(notify = "info", agent_id = %id.0, "{goal} exited")
            }
            Some(StopCause::Stopped | StopCause::Finished) | None => {}
        }
        true
    }

    pub fn handle_pane(&mut self, m: PaneMsg) {
        if let Some(p) = &mut self.preview {
            match m {
                PaneMsg::Text(t) => p.text = t,
                PaneMsg::Closed => p.closed = true,
            }
        }
    }

    pub fn handle_result(&mut self, r: AppResult) {
        match r {
            AppResult::Schema(Ok(s)) => self.schema = Some(s),
            AppResult::Schema(Err(e)) => self.load_error = Some(e),
            AppResult::Repos(Ok(r)) => self.repos = r,
            AppResult::Repos(Err(e)) => self.load_error = Some(e),
            AppResult::Definitions(Ok(d)) => {
                self.definitions = Some(d);
                self.keep_spawn_choices();
            }
            AppResult::Definitions(Err(e)) => {
                tracing::error!(
                    notify = "error",
                    "Could not load the orchestrators and specialists: {e}"
                )
            }
            AppResult::Action(Ok(_)) => {}
            AppResult::Action(Err(e)) => tracing::error!(notify = "error", "{e}"),
            AppResult::Stopped { goal, result } => {
                if result.is_ok() {
                    tracing::info!(notify = "success", "{goal} stopped");
                }
                self.fetch_tasks(false);
                self.handle_result(AppResult::Action(result));
            }
            AppResult::Replied {
                agent_id,
                sent,
                result,
            } => {
                self.replying.remove(&agent_id);
                if result.is_ok() && self.replies.get(&agent_id).is_some_and(|d| *d == sent) {
                    self.replies.remove(&agent_id);
                }
                self.handle_result(AppResult::Action(result));
            }
            AppResult::RepoAdded(id, result) => {
                match result {
                    Ok(()) => {
                        self.refresh_repos();
                        tracing::info!(notify = "success", agent_id = %id, "Repo added");
                    }
                    Err(e) => tracing::error!(notify = "error", agent_id = %id, "{e}"),
                }
                self.adding.remove(&id);
                self.send_next_add(&id);
            }
            AppResult::RequestDecided(id, result) => {
                let granted = self.granting.remove(&id);
                match result {
                    Ok(()) => {
                        self.refresh_repos();
                        if granted {
                            tracing::info!(notify = "success", agent_id = %id, "Repo granted");
                        }
                    }
                    Err(e) => tracing::error!(notify = "error", agent_id = %id, "{e}"),
                }
                self.deciding.remove(&id);
            }
            AppResult::Spawn(Ok(SpawnResponse {
                agent: started,
                warnings,
            })) => {
                tracing::info!(notify = "success", agent_id = %started.id.0, "Agent started");
                // After the success line, so the footer ends on what still needs a look.
                for w in warnings {
                    tracing::warn!(notify = "warning", agent_id = %started.id.0, "{w}");
                }
                self.refresh_repos();
                self.spawn.busy = false;
                self.spawn.error = None;
                if self
                    .spawn
                    .sent
                    .take()
                    .is_some_and(|sent| sent == self.spawn.prompt)
                {
                    self.spawn.open = false;
                    self.spawn.prompt.clear();
                    // The picker stays: it holds the Browse… dialog, which is not part of what was sent.
                    self.spawn.picker.chosen.clear();
                    self.spawn.picker.typed.clear();
                    self.spawn.sent_ok();
                }
            }
            AppResult::Spawn(Err(e)) => {
                self.spawn.busy = false;
                // Closed while in flight: the dialog's error line is out of sight, so the footer says it.
                if !self.spawn.open {
                    tracing::error!(notify = "error", "{SPAWN_FAILED} {e}");
                }
                self.spawn.error = Some(e);
            }
            AppResult::Diff(id, result) => {
                let view = |r: &Result<DiffReport, String>| match r {
                    Ok(report) => DiffView::Loaded(report.clone()),
                    Err(e) => DiffView::Failed(e.clone()),
                };
                if let Some(p) = self.preview.as_mut().filter(|p| p.agent_id == id) {
                    p.diff = view(&result);
                }
                if let Some((_, d)) = self.tasks.diff.as_mut().filter(|(d, _)| *d == id) {
                    *d = view(&result);
                }
            }
            AppResult::Tasks(result) => {
                self.tasks_fetching = false;
                match result {
                    Ok(list) => {
                        self.tasks.list = Some(list);
                        self.tasks.error = None;
                    }
                    Err(e) => self.tasks.error = Some(e),
                }
                if std::mem::take(&mut self.tasks_stale) {
                    let refresh = std::mem::take(&mut self.tasks_refresh_queued);
                    self.fetch_tasks(refresh);
                }
            }
            AppResult::ArchivePlan(id, seq, result) => {
                // Only the request the dialog waits for: an older one (an earlier open, or the plan before a
                // PLAN_CHANGED) may answer late with what the task held back then.
                if let Some(d) = self.archive.as_mut().filter(|d| {
                    d.id == id && d.plan_seq == seq && matches!(d.plan, PlanView::Measuring)
                }) {
                    d.plan = match result {
                        Ok(plan) => PlanView::Ready(plan),
                        Err(e) => PlanView::Failed(e),
                    };
                }
            }
            AppResult::Archived(id, result) => {
                let name = self
                    .archiving
                    .remove(&id)
                    .unwrap_or_else(|| self.entry_name(&id));
                // Nothing was removed: the dialog stays and shows the plan as it is now, to confirm again.
                if matches!(&result, Err(e) if e == PLAN_CHANGED) {
                    if let Some(d) = self.archive.as_mut().filter(|d| d.id == id) {
                        d.archiving = false;
                        d.plan = PlanView::Measuring;
                        d.notice = Some(
                            "This task changed since the plan was made. Review it again.".into(),
                        );
                        let orphan = d.orphan;
                        self.fetch_plan(id, orphan);
                        self.fetch_tasks(false);
                        return;
                    }
                }
                if self.archive.as_ref().is_some_and(|d| d.id == id) {
                    self.archive = None;
                }
                match result {
                    Ok(report) => {
                        tracing::info!(notify = "success", "{}", archived_text(&name, &report))
                    }
                    Err(e) => tracing::error!(notify = "error", "Could not archive {name}: {e}"),
                }
                self.fetch_tasks(false);
            }
            AppResult::TaskChanged(id, result) => {
                if let Err(e) = result {
                    let name = self.entry_name(&id);
                    tracing::error!(notify = "error", "Could not update {name}: {e}");
                }
                self.fetch_tasks(false);
            }
            AppResult::SessionEnded(id, result) => {
                let name = self.entry_name(&id);
                match result {
                    Ok(()) => tracing::info!(notify = "success", "Session {name} ended"),
                    Err(e) => tracing::error!(notify = "error", "Could not end {name}: {e}"),
                }
                self.fetch_tasks(false);
            }
            AppResult::AgentConfig(_) | AppResult::AgentConfigSaved(_) => {
                self.handle_agent_config_result(r)
            }
            AppResult::Products(_)
            | AppResult::ProductSaved(_)
            | AppResult::ProductEdited(_)
            | AppResult::ProductDeleted(..)
            | AppResult::RepoFleetSaved(..)
            | AppResult::ReleaseSaved(_)
            | AppResult::ReleaseChanged(_) => self.handle_product_result(r),
            AppResult::Personas(_)
            | AppResult::PersonaSaved(_)
            | AppResult::PersonaChecked(_)
            | AppResult::PersonaDeleted(..)
            | AppResult::PersonaUpdated(..)
            | AppResult::PersonaRepos(..)
            | AppResult::LoginOpened(..)
            | AppResult::PatSaved(..) => self.handle_persona_result(r),
            AppResult::FleetView(..)
            | AppResult::BreakdownDecided(..)
            | AppResult::TaskStopped(..)
            | AppResult::OrchestratorRestarted(..)
            | AppResult::Definition(..)
            | AppResult::DefinitionSaved(..)
            | AppResult::DefinitionReset(..) => self.handle_fleet_result(r),
            AppResult::RootSaved(Ok(root)) => {
                if self.saving_text.take().is_some_and(|t| t == self.root_text) {
                    self.root_text = config::shorten_home(&root, &self.locations.home);
                }
                self.locations.tasks_root = root;
                // Every repo picker built with the old exclusion. These are drawn only on the board, out of sight
                // while Settings saves; the grant pickers are made again when their box next draws.
                self.grant_pickers.clear();
                if let Some(prev) = self.preview.as_mut() {
                    prev.picker.browse = self.locations.repo_picker(&self.snapshot.agents);
                }
                // Rebuilding drops the picker's Browse… state, which an in-flight spawn's dialog may still show; it
                // is rebuilt when New task next opens the dialog with no spawn in flight (the only way in).
                if self.spawn.busy {
                    self.spawn_picker_stale = true;
                } else {
                    self.spawn.picker.browse = self.locations.repo_picker(&self.snapshot.agents);
                }
            }
            AppResult::PersonasDirSaved(Ok(dir)) => {
                if self
                    .saving_text
                    .take()
                    .is_some_and(|t| t == self.personas_text)
                {
                    self.personas_text = config::shorten_home(&dir, &self.locations.home);
                }
                self.personas_dir = dir;
            }
            AppResult::SettingsSaved(Ok(())) => {
                tracing::info!(notify = "success", "Settings saved")
            }
            AppResult::SettingsSaved(Err(e)) => tracing::error!(notify = "error", "{e}"),
            AppResult::RootSaved(Err(e)) | AppResult::PersonasDirSaved(Err(e)) => {
                self.saving_text = None;
                self.restarting = None;
                tracing::error!(notify = "error", "{e}");
            }
            AppResult::Restarted(result) => {
                let asked = self.restarting.take();
                match result {
                    Ok(Started::StartedByUs(pid)) => {
                        if let Some(d) = &mut self.daemon {
                            d.we_started = true;
                        }
                        self.restarted = Some(pid);
                        // The event stream reconnects on its own; the schema and repos are fetched afresh from the new daemon.
                        self.load();
                        tracing::info!(notify = "success", "Daemon restarted");
                    }
                    Ok(Started::AlreadyRunning) if matches!(asked, Some(RestartFor::Save(_))) => {
                        tracing::info!(
                            notify = "info",
                            "Saved. Restart the daemon to use the new folder."
                        );
                    }
                    Ok(Started::AlreadyRunning) => tracing::error!(
                        notify = "error",
                        "Could not restart the daemon: {}",
                        StopError::NoPidfile
                    ),
                    Err(e) => tracing::error!(notify = "error", "{e}"),
                }
            }
        }
    }

    /// Fetches the tasks list, after asking the daemon to rescan and re-measure when `refresh`. With a fetch
    /// already in flight, another follows it: that one may have left before the change being fetched for.
    fn fetch_tasks(&mut self, refresh: bool) {
        if self.tasks_fetching {
            self.tasks_stale = true;
            self.tasks_refresh_queued |= refresh;
            return;
        }
        self.tasks_fetching = true;
        self.tasks_sent.0 += 1;
        self.tasks_sent.1 += u32::from(refresh);
        self.background(move |c| {
            if refresh {
                if let Err(e) = c.refresh_tasks() {
                    return AppResult::Tasks(Err(e.0));
                }
            }
            AppResult::Tasks(c.tasks().map_err(|e| e.0))
        });
    }

    /// A task's or orphan's name for a message, from the list as last fetched.
    fn entry_name(&self, id: &str) -> String {
        let Some(list) = &self.tasks.list else {
            return id.to_string();
        };
        list.tasks
            .iter()
            .find(|p| p.id.0 == id)
            .map(|p| p.name.clone())
            .or_else(|| {
                list.orphans
                    .iter()
                    .find(|o| o.id == id)
                    .map(tasks::orphan_name)
            })
            .unwrap_or_else(|| id.to_string())
    }

    /// Opens the Archive dialog and measures a fresh plan for it.
    fn ask_archive(&mut self, id: String, name: String, orphan: bool) {
        if self.archiving.contains_key(&id) {
            return;
        }
        self.archive = Some(ArchiveDialog::new(id.clone(), name, orphan));
        self.fetch_plan(id, orphan);
    }

    /// Measures a plan for the open dialog. Always a new request: a reused one may have left before the change
    /// that made the dialog ask again, and one that never answers must not leave later dialogs on Measuring. The
    /// dialog remembers the request's sequence number and takes only its answer, so an older request that answers
    /// late is dropped.
    fn fetch_plan(&mut self, id: String, orphan: bool) {
        self.plans_sent += 1;
        let seq = self.plans_sent;
        if let Some(d) = self.archive.as_mut().filter(|d| d.id == id) {
            d.plan_seq = seq;
        }
        self.background(move |c| {
            let plan = if orphan {
                c.orphan_plan(&id)
            } else {
                c.archive_plan(&id)
            };
            AppResult::ArchivePlan(id.clone(), seq, plan.map_err(|e| e.0))
        });
    }

    fn archive_dialogs(&mut self, ctx: &egui::Context) {
        if let Some(d) = self.archive.as_mut() {
            match archive_dialog(ctx, d) {
                None => {}
                Some(Confirm::Cancel) => self.archive = None,
                Some(Confirm::Go) => {
                    // Archive is enabled only with a plan on screen: that plan is what the daemon checks against.
                    let PlanView::Ready(plan) = &d.plan else {
                        return;
                    };
                    let plan = plan.clone();
                    d.archiving = true;
                    let (id, orphan) = (d.id.clone(), d.orphan);
                    self.archiving.insert(id.clone(), d.name.clone());
                    self.background(move |c| {
                        let report = if orphan {
                            c.archive_orphan(&id, &plan)
                        } else {
                            c.archive(&id, &plan)
                        };
                        AppResult::Archived(id.clone(), report.map_err(|e| e.0))
                    });
                }
            }
        }
        if let Some(d) = self.end_session.as_mut() {
            match end_session_dialog(ctx, d) {
                None => {}
                Some(Confirm::Cancel) => self.end_session = None,
                Some(Confirm::Go) => {
                    let id = d.id.clone();
                    self.end_session = None;
                    self.background(move |c| {
                        AppResult::SessionEnded(id.clone(), c.end_session(&id).map_err(|e| e.0))
                    });
                }
            }
        }
    }

    /// Running tasks' agents, as the Tasks view shows them: board status and whether Stop is offered.
    fn live_tasks(&self) -> HashMap<String, Live> {
        let Some(schema) = &self.schema else {
            return HashMap::new();
        };
        self.snapshot
            .agents
            .iter()
            .map(|a| {
                let column = schema
                    .columns
                    .iter()
                    .find(|c| c.statuses.contains(&a.status));
                let live = Live {
                    status: column.map_or_else(|| format!("{:?}", a.status), |c| c.title.clone()),
                    tone: column.map_or(fleet_api::Tone::Neutral, |c| c.tone),
                    can_stop: stop_action(schema, a).is_some(),
                };
                (a.id.0.clone(), live)
            })
            .collect()
    }

    fn tasks_view(&mut self, ui: &mut Ui) {
        let now = ui.input(|i| i.time);
        let live = self.live_tasks();
        let names = self.persona_names();
        let labels = persona_labels(
            &names,
            names.keys().map(String::as_str).chain(
                self.tasks
                    .list
                    .iter()
                    .flat_map(|l| &l.tasks)
                    .map(|p| p.persona.as_str()),
            ),
        );
        let active = self.active_product().map(str::to_string);
        // Field by field: the view borrows `self.tasks` mutably beside the scope.
        let product = self
            .products
            .as_ref()
            .and_then(|l| l.products.iter().find(|p| Some(&p.id) == active.as_ref()));
        let scope = tasks::Scope {
            product: active.as_deref(),
            repos: product.map_or(&[], |p| &p.repos),
            releases: product.map_or(&[], |p| &p.releases),
            repos_known: product.is_some(),
            release: self.release_filter,
        };
        let action = tasks::main(ui, &mut self.tasks, &live, (self.clock)(), &labels, scope);
        match action {
            None => {}
            Some(TasksAction::Retry) => self.fetch_tasks(false),
            Some(TasksAction::Passport {
                id,
                name,
                orphan,
                action,
            }) => match action {
                PassportAction::OpenOnBoard => self.select_card(&id),
                PassportAction::Stop => {
                    let found = self.schema.as_ref().and_then(|schema| {
                        let agent = self.snapshot.agents.iter().find(|a| a.id.0 == id)?;
                        Some((stop_action(schema, agent)?.clone(), agent.clone()))
                    });
                    if let Some((action, agent)) = found {
                        self.run_command(Command::Run {
                            action,
                            agent: Box::new(agent),
                            input: None,
                        });
                    }
                }
                PassportAction::Archive => self.ask_archive(id, name, orphan),
                PassportAction::EndSession => {
                    self.end_session = Some(EndSessionDialog {
                        id,
                        session: name,
                        focus_cancel: true,
                    })
                }
                PassportAction::Refresh => {
                    self.fetch_tasks(true);
                    self.tasks_due = self
                        .tasks_due
                        .min(now + REFETCH_AFTER_REFRESH.as_secs_f64());
                }
                PassportAction::MarkDone => {
                    self.task_calls.push(TaskCall::Done(id.clone()));
                    self.background(move |c| {
                        AppResult::TaskChanged(id.clone(), c.mark_done(&id).map_err(|e| e.0))
                    });
                }
                PassportAction::SetRelease(release) => {
                    self.task_calls.push(TaskCall::Release(id.clone(), release));
                    self.background(move |c| {
                        AppResult::TaskChanged(
                            id.clone(),
                            c.set_task_release(&id, release).map_err(|e| e.0),
                        )
                    });
                }
                PassportAction::LoadDiff => {
                    self.tasks.diff = Some((id.clone(), DiffView::Loading));
                    self.background(move |client| {
                        AppResult::Diff(id.clone(), client.diff(&id).map_err(|e| e.0))
                    });
                }
            },
        }
        self.archive_dialogs(ui.ctx());
    }

    /// Why calls that a restart would cut short (New task, Add repo, Grant, Refuse) cannot run now, if they cannot.
    fn daemon_blocker(&self) -> Option<&'static str> {
        self.restarting
            .is_some()
            .then_some("The daemon is restarting")
    }

    /// Why Save and Restart daemon cannot run now, if they cannot. A restart marks a Queued agent whose tmux
    /// session does not exist yet as Dead, and its startup sweep removes a clone this window has in flight.
    fn restart_blocker(&self) -> Option<&'static str> {
        if self.restarting.is_some() {
            Some("The daemon is restarting")
        } else if self
            .snapshot
            .agents
            .iter()
            .any(|a| a.status == Status::Queued)
        {
            Some("An agent is still starting; wait until it runs")
        } else if self.spawn.busy {
            Some("A task is being started")
        } else if !self.adding.is_empty() {
            Some("A repo is being added")
        } else if !self.deciding.is_empty() {
            Some("A repo request is being answered")
        } else {
            None
        }
    }

    fn save_blocker(&self, folder: Folder) -> Option<&'static str> {
        let (text, current, reason) = match folder {
            Folder::Tasks => (
                &self.root_text,
                &self.locations.tasks_root,
                "This is the current tasks folder",
            ),
            Folder::Personas => (
                &self.personas_text,
                &self.personas_dir,
                "This is the current personas folder",
            ),
        };
        // Path equality ignores a trailing slash, so "~/x/" is the current "~/x" too.
        if config::expand_home(text.trim(), &self.locations.home) == *current {
            return Some(reason);
        }
        self.restart_blocker()
    }

    /// Whether this folder's note shows the restart: one its Save or Restart daemon asked for.
    fn restarting_for(&self, folder: Folder) -> bool {
        match self.restarting {
            Some(RestartFor::Restart) => true,
            Some(RestartFor::Save(f)) => f == folder,
            None => false,
        }
    }

    /// Writes the typed folder (for a Save) and restarts the daemon, on one background thread so the restart never
    /// runs before the write.
    fn start_restart(&mut self, kind: RestartFor) {
        let Some(d) = &self.daemon else { return };
        let (stop, ensure, config_path) = (d.stop.clone(), d.ensure.clone(), d.config_path.clone());
        let typed = match kind {
            RestartFor::Save(Folder::Tasks) => Some((Folder::Tasks, self.root_text.clone())),
            RestartFor::Save(Folder::Personas) => {
                Some((Folder::Personas, self.personas_text.clone()))
            }
            RestartFor::Restart => None,
        };
        self.saving_text = typed.as_ref().map(|(_, t)| t.clone());
        let home = self.locations.home.clone();
        let (tx, ctx) = (self.results_tx.clone(), self.ctx.clone());
        self.restarting = Some(kind);
        std::thread::spawn(move || {
            let mut out = RestartOutcome {
                tx,
                ctx,
                done: false,
            };
            if let Some((folder, typed)) = typed {
                let cannot_write =
                    |e: std::io::Error| format!("Cannot write {}: {e}", config_path.display());
                let (ok, result) = match folder {
                    Folder::Tasks => {
                        let saved = config::prepare_tasks_root(&typed, &home).and_then(|root| {
                            config::set_tasks_root(&config_path, &root, &home)
                                .map(|()| root)
                                .map_err(cannot_write)
                        });
                        (saved.is_ok(), AppResult::RootSaved(saved))
                    }
                    Folder::Personas => {
                        let saved = config::prepare_personas_dir(&typed, &home).and_then(|dir| {
                            config::set_personas_dir(&config_path, &dir, &home)
                                .map(|()| dir)
                                .map_err(cannot_write)
                        });
                        (saved.is_ok(), AppResult::PersonasDirSaved(saved))
                    }
                };
                out.send(result);
                if !ok {
                    // The `Err` result already ends the restart.
                    out.done = true;
                    return;
                }
            }
            let result = restart_daemon(&stop, &ensure);
            out.done = true;
            out.send(AppResult::Restarted(result));
        });
    }

    /// Save or Restart daemon, after asking when a pending repo request would be cut short by the restart.
    fn ask_restart(&mut self, kind: RestartFor) {
        if self
            .snapshot
            .agents
            .iter()
            .any(|a| a.repo_request.is_some())
        {
            self.confirm = Some(kind);
        } else {
            self.start_restart(kind);
        }
    }

    fn confirm_restart_dialog(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.confirm else { return };
        let p = tokens(ctx.global_style().visuals.dark_mode);
        let (mut go, mut cancel) = (false, false);
        let modal = egui::Modal::new(egui::Id::unique("confirm_restart"))
            .frame(modal_frame(ctx))
            .show(ctx, |ui| {
                ui.set_max_width(520.0);
                ui.label(
                    RichText::new("Restart the daemon?")
                        .text_style(egui::TextStyle::Heading)
                        .color(p.text_high),
                );
                ui.add_space(6.0);
                ui.add(egui::Label::new(RichText::new(confirm_text(kind)).color(p.text)).wrap());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    go = button(ui, "Restart anyway", ButtonVariant::Default).clicked();
                    cancel = button(ui, "Cancel", ButtonVariant::Primary).clicked();
                });
            });
        if go || cancel || modal.should_close() {
            self.confirm = None;
        }
        // An agent may have been queued while the dialog was open; the button then stays greyed out with the reason.
        let blocker = match kind {
            RestartFor::Save(folder) => self.save_blocker(folder),
            RestartFor::Restart => self.restart_blocker(),
        };
        if go && blocker.is_none() {
            self.start_restart(kind);
        }
    }

    /// True while a grant or refusal for this agent is in flight.
    pub fn deciding(&self, agent_id: &str) -> bool {
        self.deciding.contains(agent_id)
    }

    /// Queues `repo` for the previewed agent; the queue is sent one call at a time. The board changes through the
    /// daemon's `WorkspacesChanged` events, so a success has nothing to apply here.
    fn add_repo(&mut self, repo: String) {
        let Some(id) = self.preview_agent().map(str::to_string) else {
            return;
        };
        let idle = !self.adding.contains(&id);
        self.add_queue
            .entry(id.clone())
            .or_default()
            .push_back(repo);
        if idle {
            self.send_next_add(&id);
        }
    }

    /// Sends the agent's next queued repo; false when there is none.
    fn send_next_add(&mut self, id: &str) -> bool {
        let Some(repo) = self.add_queue.get_mut(id).and_then(VecDeque::pop_front) else {
            self.add_queue.remove(id);
            return false;
        };
        self.adding.insert(id.to_string());
        let id = id.to_string();
        self.background(move |client| {
            AppResult::RepoAdded(
                id.clone(),
                client.add_repo(&id, &repo).map(drop).map_err(|e| e.0),
            )
        });
        true
    }

    /// Runs a grant or refusal for the previewed agent, unless one is already in flight.
    fn decide(
        &mut self,
        grant: bool,
        call: impl FnOnce(&FleetClient, &str) -> Result<(), fleet_client::http::FleetError>
            + Send
            + 'static,
    ) {
        let Some(id) = self.preview_agent().map(str::to_string) else {
            return;
        };
        if !self.deciding.insert(id.clone()) {
            return;
        }
        if grant {
            self.granting.insert(id.clone());
        }
        self.background(move |client| {
            AppResult::RequestDecided(id.clone(), call(client, &id).map_err(|e| e.0))
        });
    }

    fn close_preview(&mut self) {
        self.preview = None;
        self.panel_fullscreen = false;
        self.pane = None;
        self.pane_rx = None;
    }

    fn open_preview(&mut self, id: &str) {
        let Some(agent) = self.snapshot.agents.iter().find(|a| a.id.0 == id) else {
            return;
        };
        let path = panel_path(agent);
        self.preview = Some(PreviewState {
            agent_id: id.to_string(),
            title: agent.goal.clone(),
            path,
            focus_reply: true,
            ..PreviewState::new(self.locations.repo_picker(&self.snapshot.agents))
        });
        self.opened_fleet_panel(id);
        let (tx, rx) = mpsc::channel();
        let wake = self.ctx.clone();
        self.pane = Some(spawn_pane_stream(
            &self.client_url(),
            id,
            tx,
            Arc::new(move || wake.request_repaint()),
        ));
        self.pane_rx = Some(rx);
    }

    /// Shows the board with this agent's card selected; an agent no longer on the board (archived since) selects
    /// nothing.
    fn select_card(&mut self, id: &str) {
        self.set_view(View::Kanban);
        if self.preview_agent() != Some(id) {
            self.open_preview(id);
        }
    }

    /// Does what the clicks would: a card click, then a tab click. An agent not on the board changes nothing.
    pub fn apply_cue(&mut self, cue: Cue) {
        match cue {
            Cue::Open { agent, tab } => {
                if !self.snapshot.agents.iter().any(|a| a.id.0 == agent) {
                    tracing::warn!("GUI cue names agent {agent}, which is not on the board");
                    return;
                }
                self.select_card(&agent);
                let Some(prev) = self.preview.as_mut() else {
                    return;
                };
                prev.tab = tab;
                if tab == PreviewTab::Diff {
                    prev.diff = DiffView::Loading;
                    self.load_diff();
                }
            }
            Cue::Close => self.close_preview(),
        }
    }

    fn load_diff(&mut self) {
        if let Some(id) = self.preview_agent().map(str::to_string) {
            self.background(move |client| {
                AppResult::Diff(id.clone(), client.diff(&id).map_err(|e| e.0))
            });
        }
    }

    fn drain(&mut self, now: f64) {
        // Logged here rather than in `with_log` so it is a frame event the footer can pick up like any other.
        if let Some(reason) = self.file_error.take() {
            tracing::warn!(notify = "warning", "Logging to file is off: {reason}");
        }
        while let Some(m) = self.stream_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.handle_stream(m);
        }
        while let Some(m) = self.pane_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.handle_pane(m);
        }
        while let Ok(r) = self.results_rx.try_recv() {
            self.handle_result(r);
        }
        // After the stream, so a cue can name an agent whose snapshot arrived this frame.
        if let Some(cues) = self.cues.as_mut() {
            for cue in cues.poll() {
                self.apply_cue(cue);
            }
            // Nothing else wakes an idle window when the recorder appends a line.
            self.ctx.request_repaint_after(theme::ANIMATION_FRAME);
        }
        // Last: handling the results above logs the notices about them, which must reach the footer this frame.
        while let Some(e) = self.log_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            // Only the app's own crates: a dependency's `notify` field is not a message for the user.
            if let Some(notice) = e.notice().filter(|_| e.target.starts_with("fleet_")) {
                self.messages.push(
                    Message {
                        notice,
                        text: e.message.clone(),
                        time: e.time,
                        agent_id: e
                            .fields
                            .iter()
                            .find(|(k, _)| k == "agent_id")
                            .map(|(_, v)| v.clone()),
                    },
                    now,
                );
            }
            self.log_pushed += 1;
            self.log.push(e);
        }
        if self.log.len() > LOG_CAP {
            self.log.drain(..self.log.len() - LOG_CAP);
        }
    }

    fn run_command(&mut self, c: Command) {
        match c {
            Command::Run {
                action,
                agent,
                input,
            } => {
                // Stop's success names the agent, which only this side knows: the daemon answers with nothing.
                let stop = action.id == "stop";
                self.background(move |client| {
                    let result = client
                        .run(&action, &agent, input.as_deref())
                        .map_err(|e| e.0);
                    if stop {
                        AppResult::Stopped {
                            goal: agent.goal.clone(),
                            result,
                        }
                    } else {
                        AppResult::Action(result)
                    }
                });
            }
            Command::Open(id) => self.open_preview(&id),
            Command::Details(id) => self.open_tasks(TaskFilter::All, Some(id)),
            Command::StopTask { task, fleet } => self.ask_stop_task(task, fleet),
            Command::RestartOrchestrator(task) => self.restart_orchestrator(task),
        }
    }

    /// Draws the whole window into `ui`.
    pub fn ui(&mut self, ui: &mut Ui) {
        if self.cues.is_some() {
            theme::force_animation(ui.ctx());
        }
        // egui's own transitions (hover fades, collapsing) follow the same focus gate as the app's animations.
        let transition = if theme::animate(ui.ctx()) {
            theme::ANIMATION_TIME
        } else {
            0.0
        };
        if ui.ctx().global_style().animation_time != transition {
            ui.ctx().all_styles_mut(|s| s.animation_time = transition);
        }
        self.panels(ui);
        // After the panels, and even when they stopped early on a missing schema: the dialog only needs the repos.
        let listed = self.personas.as_ref().map_or(&[][..], |l| &l.personas[..]);
        let defs = self.definitions.as_deref().unwrap_or_default();
        let active = self.active_product().map(str::to_string);
        let product = self
            .products
            .as_ref()
            .and_then(|l| l.products.iter().find(|p| Some(&p.id) == active.as_ref()));
        match new_task_dialog(ui.ctx(), listed, defs, product, &mut self.spawn) {
            None => {}
            Some(SpawnAction::Persona(_)) => self.fetch_spawn_repos(),
            Some(SpawnAction::OpenWorkflowTab) => {
                self.spawn.open = false;
                self.management_tab = ManagementTab::Workflow;
                self.set_view(View::Management);
            }
            Some(SpawnAction::Start {
                persona,
                release,
                repos,
                prompt,
                kind,
                role,
                roster,
            }) => {
                self.spawn.busy = true;
                self.spawn.error = None;
                self.spawn.sent = Some(prompt.clone());
                let product = self.active_product().map(str::to_string);
                self.background(move |client| {
                    let req = SpawnRequest {
                        actor: fleet_api::Actor("local".into()),
                        product: String::new(),
                        release,
                        repos,
                        prompt,
                        slug: None,
                        persona,
                        kind,
                        role,
                        roster,
                    };
                    AppResult::Spawn(spawn_in_product(client, product, req).map_err(|e| e.0))
                });
            }
        }
        // Last, so they sit above everything else drawn this frame.
        self.product_form_modal(ui.ctx());
        self.persona_setup(ui.ctx());
    }

    /// Repos the user configured, then any an agent uses that is not configured, each with its agent count.
    fn repo_counts(&self) -> Vec<(String, usize)> {
        let mut repos: Vec<(String, usize)> =
            self.repos.iter().map(|r| (r.name.clone(), 0)).collect();
        for agent in &self.snapshot.agents {
            let mut seen: Vec<&str> = Vec::new();
            for w in &agent.workspaces {
                if seen.contains(&w.repo.as_str()) {
                    continue;
                }
                seen.push(&w.repo);
                match repos.iter_mut().find(|(r, _)| *r == w.repo) {
                    Some((_, n)) => *n += 1,
                    None => repos.push((w.repo.clone(), 1)),
                }
            }
        }
        repos
    }

    /// The shell, in the order egui needs: the drawer and the content's footer and header claim their edges
    /// before the view's detail panel and main area take what is left.
    fn panels(&mut self, ui: &mut Ui) {
        let now = ui.input(|i| i.time);
        self.drain(now);
        let p = tokens(ui.visuals().dark_mode);
        // Durations such as "5m" must keep counting on a quiet board, where no event would repaint the window.
        ui.ctx().request_repaint_after(REFRESH);

        let need_you = self
            .snapshot
            .agents
            .iter()
            .filter(|a| a.status == Status::NeedsYou)
            .count();
        let new_task_blocked = self
            .daemon_blocker()
            .or(self.persona_blocker())
            .or(self.product_blocker());
        let drawn = drawer(
            ui,
            self.view,
            DrawerStatus {
                connected: self.connected,
                agents: self.snapshot.agents.len(),
                need_you,
                new_task_blocked,
            },
            Switcher {
                personas: self.personas.as_ref().map_or(&[], |l| &l.personas),
                active: self.active_persona.as_deref(),
                products: self.listed_products(),
                product: self.active_product(),
            },
        );
        self.layout.drawer_header = drawn.header;
        self.layout.drawer_footer = drawn.footer;
        // Before the content is drawn, so a nav click shows its view on this frame.
        match drawn.action {
            None => {}
            Some(DrawerAction::NewTask) if new_task_blocked.is_none() => {
                if self.spawn_picker_stale && !self.spawn.busy {
                    self.spawn.picker.browse = self.locations.repo_picker(&self.snapshot.agents);
                    self.spawn_picker_stale = false;
                }
                self.open_spawn_dialog();
                self.refresh_repos();
            }
            Some(DrawerAction::NewTask) => {}
            Some(DrawerAction::NewProduct) => self.open_product_form(),
            Some(DrawerAction::SetProduct(id)) => self.set_active_product(Some(id)),
            Some(DrawerAction::Show(v)) => self.set_view(v),
            Some(DrawerAction::SetPersona(id)) => self.set_active(Some(id)),
        }

        // The rows' 1 px borders are the panels' own separators, which the theme draws in the `border` token.
        let row = Frame::new()
            .fill(p.surface)
            .inner_margin(Margin::symmetric(14, 0));
        let footer = Panel::bottom("footer")
            .exact_size(layout::FOOTER)
            .frame(row)
            .show(ui, |ui| footer(ui, &mut self.messages, now));
        self.layout.footer = footer.response.rect;
        match footer.inner {
            FooterAction::None => {}
            FooterAction::OpenLog => {
                self.settings_tab = SettingsTab::Log;
                self.set_view(View::Settings);
            }
            FooterAction::Select(id) => self.select_card(&id),
        }

        // Whatever the view: the board's header shows the counts, and the Tasks view the list.
        if now >= self.tasks_due {
            self.tasks_due = now + TASKS_EVERY.as_secs_f64();
            self.fetch_tasks(false);
            self.retry_personas();
            self.retry_products();
        }
        ui.ctx()
            .request_repaint_after_secs((self.tasks_due - now).max(0.0) as f32);

        let repos = self.repo_counts();
        let personas = self.persona_rows();
        // A persona no longer offered (deleted, its agents gone) must not keep hiding cards out of sight.
        self.persona_filter
            .retain(|id| personas.iter().any(|(p, _, _)| p == id));
        let releases: Vec<Release> = self
            .active_product()
            .and_then(|id| self.listed_products().iter().find(|p| p.id == id))
            .map_or_else(Vec::new, |p| p.releases.clone());
        // A release no longer offered (marked released, deleted) must not keep hiding cards out of sight.
        self.release_filter = self.release_filter.kept(&releases);
        let header = Panel::top("header")
            .exact_size(layout::HEADER)
            .frame(row)
            .show(ui, |ui| {
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| match self.view {
                    View::Kanban => kanban::header(
                        ui,
                        &repos,
                        &mut self.filter,
                        &personas,
                        &mut self.persona_filter,
                        &releases,
                        &mut self.release_filter,
                        self.fleet_display,
                    ),
                    View::Tasks => {
                        tasks::header(
                            ui,
                            &mut self.tasks.show_archived,
                            &releases,
                            &mut self.release_filter,
                        );
                        None
                    }
                    View::Releases => {
                        releases::header(ui);
                        None
                    }
                    View::Management => {
                        management::header(ui);
                        None
                    }
                    View::Settings => {
                        settings::header(ui);
                        None
                    }
                })
            });
        if let Some(display) = header.inner.inner {
            self.set_fleet_display(display);
        }
        self.layout.header = header.response.rect;
        self.layout.strip = Rect::NOTHING;
        // Before the detail panel, so the strip spans the board and the panel alike.
        if self.view == View::Kanban {
            // The schema may recolour or drop the column; the dot follows it, and Muted is the default's.
            let stopped_tone = self
                .schema
                .as_ref()
                .and_then(|s| {
                    s.columns
                        .iter()
                        .find(|c| c.statuses.contains(&Status::Stopped))
                })
                .map_or(fleet_api::Tone::Muted, |c| c.tone);
            let strip = Panel::top("strip")
                .exact_size(layout::STRIP)
                .frame(row)
                .show(ui, |ui| {
                    kanban::strip(ui, self.tasks.list.as_ref(), stopped_tone)
                });
            self.layout.strip = strip.response.rect;
            if let Some(filter) = strip.inner {
                self.open_tasks(filter, None);
            }
        }

        match self.view {
            View::Kanban => self.kanban(ui),
            View::Tasks => self.tasks_view(ui),
            View::Releases => self.releases_view(ui),
            View::Management => self.management_view(ui),
            View::Settings => {
                let (tasks_blocked, personas_blocked, restart_blocked) = (
                    self.save_blocker(Folder::Tasks),
                    self.save_blocker(Folder::Personas),
                    self.restart_blocker(),
                );
                let restarting = self.restarting.is_some();
                let (tasks_restarting, personas_restarting) = (
                    self.restarting_for(Folder::Tasks),
                    self.restarting_for(Folder::Personas),
                );
                let has_daemon = self.daemon.is_some();
                let state = SettingsState {
                    theme: self.theme,
                    close_choice: self.close_choice,
                    sticky_alerts: self.sticky_alerts,
                    tasks: has_daemon.then_some(FolderField {
                        text: &mut self.root_text,
                        picker: &mut self.root_picker,
                        blocked: tasks_blocked,
                        restarting: tasks_restarting,
                    }),
                    personas: has_daemon.then_some(FolderField {
                        text: &mut self.personas_text,
                        picker: &mut self.personas_picker,
                        blocked: personas_blocked,
                        restarting: personas_restarting,
                    }),
                    daemon: self.daemon.as_ref().map(|ctl| DaemonRows {
                        ctl,
                        blocked: restart_blocked,
                        restarting,
                    }),
                    log: LogPane {
                        view: &mut self.log_view,
                        feed: LogFeed {
                            live: &self.log,
                            live_pushed: self.log_pushed,
                            state_dir: self.daemon.as_ref().map(|ctl| ctl.state_dir.as_path()),
                        },
                    },
                };
                match settings::main(ui, &mut self.settings_tab, state) {
                    None => {}
                    Some(SettingsAction::Change(change)) => self.apply_general(change),
                    Some(SettingsAction::SaveTasksRoot) => {
                        self.ask_restart(RestartFor::Save(Folder::Tasks))
                    }
                    Some(SettingsAction::SavePersonasDir) => {
                        self.ask_restart(RestartFor::Save(Folder::Personas))
                    }
                    Some(SettingsAction::RestartDaemon) => self.ask_restart(RestartFor::Restart),
                }
                self.confirm_restart_dialog(ui.ctx());
            }
        }
    }

    /// The board, with the selected agent in the detail panel.
    fn kanban(&mut self, ui: &mut Ui) {
        let p = tokens(ui.visuals().dark_mode);
        let now = (self.clock)();
        let Some(schema) = self.schema.clone() else {
            if let Some(e) = self.load_error.clone() {
                let mut retry = false;
                centred_card(ui, |ui| {
                    ui.label(RichText::new(format!("Cannot load the board: {e}")).color(p.danger));
                    ui.add_space(10.0);
                    retry = button(ui, "Retry", ButtonVariant::Primary).clicked();
                });
                if retry {
                    self.load();
                }
            } else {
                centred_card(ui, |ui| {
                    ui.label(RichText::new("Loading…").color(p.text_low));
                });
            }
            return;
        };

        let labels: HashMap<String, String> = self
            .persona_rows()
            .into_iter()
            .map(|(id, label, _)| (id, label))
            .collect();
        // With no product selected there is nothing to narrow by, and the whole board shows.
        let product = self.active_product();
        // An agent's release is its task's; a task with no record in the list reads as the backlog.
        let releases: HashMap<&str, Option<i64>> = self
            .tasks
            .list
            .iter()
            .flat_map(|l| &l.tasks)
            .map(|t| (t.id.0.as_str(), t.release))
            .collect();
        // A task passes the filters by its lead, and its fleet comes along: a fleet agent never shows without it.
        let shown: Vec<Agent> = groups(&self.snapshot.agents)
            .into_iter()
            .filter(|g| product.is_none_or(|p| g.lead.product == p))
            .filter(|g| {
                let release = releases
                    .get(fleet_api::task_id(g.lead).0.as_str())
                    .copied()
                    .flatten();
                self.release_filter.allows(release)
            })
            .filter(|g| {
                self.filter.is_empty()
                    || g.lead
                        .workspaces
                        .iter()
                        .any(|w| self.filter.contains(&w.repo))
            })
            .filter(|g| {
                self.persona_filter.is_empty() || self.persona_filter.contains(&g.lead.persona)
            })
            .flat_map(|g| std::iter::once(g.lead).chain(g.fleet))
            .cloned()
            .collect();
        // Tags only tell cards apart when the board shows more than one persona.
        let mut tags: HashMap<String, String> = shown
            .iter()
            .map(|a| {
                let label = labels.get(&a.persona).cloned();
                (
                    a.persona.clone(),
                    label.unwrap_or_else(|| a.persona.clone()),
                )
            })
            .collect();
        if tags.len() < 2 {
            tags.clear();
        }

        let mut preview_action = PreviewAction::None;
        let mut covers_board = false;
        // The previewed agent's first input action (Answer on Needs you) gets the panel's reply box.
        let reply_target: Option<(ActionDef, Agent)> = self.preview.as_ref().and_then(|prev| {
            let agent = self
                .snapshot
                .agents
                .iter()
                .find(|a| a.id.0 == prev.agent_id)?;
            let action = visible_actions(&schema, agent)
                .into_iter()
                .find(|a| a.input.is_some())?;
            Some((action.clone(), agent.clone()))
        });
        // A draft lives only while its request does; a failed grant keeps it, so the edited target is not lost.
        self.requests.retain(|id, d| {
            self.snapshot.agents.iter().any(|a| {
                a.id.0 == *id
                    && a.repo_request
                        .as_ref()
                        .is_some_and(|r| r.repo == d.for_repo)
            })
        });
        self.grant_pickers
            .retain(|id, _| self.requests.contains_key(id));
        let daemon_blocked = self.daemon_blocker();
        let previewed: Option<Agent> = self.preview.as_ref().and_then(|prev| {
            self.snapshot
                .agents
                .iter()
                .find(|a| a.id.0 == prev.agent_id)
                .cloned()
        });
        if let Some(prev) = self.preview.as_mut() {
            // Every frame: an add or a grant changes it while the panel is open.
            if let Some(a) = &previewed {
                prev.path = panel_path(a);
            }
            // A stopped agent's leftover request has nobody waiting on it, so it gets no box.
            let pending = previewed
                .as_ref()
                .filter(|a| a.status != Status::Stopped)
                .and_then(|a| a.repo_request.as_ref().map(|r| (a, r)));
            // The request box replaces the reply box, whatever the schema says.
            let reply =
                reply_target
                    .as_ref()
                    .filter(|_| pending.is_none())
                    .map(|(action, agent)| Reply {
                        label: &action.label,
                        text: self.replies.entry(agent.id.0.clone()).or_default(),
                    });
            let request = pending.map(|(agent, req)| {
                let d = self
                    .requests
                    .entry(agent.id.0.clone())
                    .or_insert_with(|| RequestText {
                        for_repo: req.repo.clone(),
                        target: req.repo.clone(),
                        reason: String::new(),
                    });
                let browse = self
                    .grant_pickers
                    .entry(agent.id.0.clone())
                    .or_insert_with(|| self.locations.repo_picker(&self.snapshot.agents));
                let blocked = daemon_blocked.or_else(|| {
                    self.deciding
                        .contains(&agent.id.0)
                        .then_some("A decision is being sent")
                });
                RequestDraft {
                    request: req,
                    target: &mut d.target,
                    reason: &mut d.reason,
                    browse,
                    blocked,
                }
            });
            // The daemon resolves an added name within the agent's product, so another product's repos would only
            // be refused.
            let remembered: Vec<RepoInfo> = previewed.as_ref().map_or_else(Vec::new, |a| {
                self.repos
                    .iter()
                    .filter(|r| r.product == a.product)
                    .cloned()
                    .collect()
            });
            let panel = previewed.as_ref().map(|a| RepoPanel {
                workspaces: &a.workspaces,
                remembered: &remembered,
                request,
                add_blocked: daemon_blocked,
                stopped: a.status == Status::Stopped,
            });
            let details = previewed.as_ref().map(|agent| AgentDetails {
                schema: &schema,
                agent,
                now,
            });
            let fleet = previewed.as_ref().and_then(|a| {
                fleet::panel(
                    a,
                    &self.snapshot.agents,
                    &self.fleet_views,
                    &self.breakdown_deciding,
                    &self.breakdown_decided,
                    &mut self.breakdown_drafts,
                )
            });
            let board_width = ui.available_width();
            // From the last frame: egui drops the focus at the start of the frame an Esc arrives in, so this
            // frame's focus can no longer tell whether that Esc was a text field's.
            let typing = self.field_had_focus;
            let layout = panel_layout(board_width, self.preview_width, self.panel_fullscreen);
            prev.layout = layout;
            let (term, base) = (self.terminal.as_deref(), self.client.base_url());
            prev.terminal_blocked = previewed
                .as_ref()
                .and_then(|a| FleetApp::terminal_blocked(term, base, a));
            match layout {
                PanelLayout::Side => {
                    // Like the header, the panel's own separator is the 1 px `border` line (on its left edge).
                    // A drag stops short of the point where `panel_layout` would force Full: a window that
                    // cannot grow would trap the person there. The 1 px margin keeps rounding from flipping it.
                    let cap = board_width * 0.75 - 1.0;
                    let shown = egui::Panel::right("preview")
                        .resizable(true)
                        .default_size(420.0)
                        .max_size(cap)
                        .frame(Frame::new().fill(p.surface))
                        .show(ui, |ui| {
                            preview_action = preview_pane(ui, prev, reply, panel, details, fleet);
                        });
                    // Recorded only while under the cap: at the cap the width may be the one a shrinking window
                    // clamped the panel to, not the one the person chose, and only the chosen one lets a shrink
                    // force Full.
                    let width = shown.response.rect.width();
                    if width < cap - 0.5 {
                        self.preview_width = width;
                    }
                }
                PanelLayout::Full { .. } => {
                    covers_board = true;
                    egui::CentralPanel::default()
                        .frame(Frame::new().fill(p.surface))
                        .show(ui, |ui| {
                            preview_action = preview_pane(ui, prev, reply, panel, details, fleet);
                        });
                    // Esc backs out of a chosen Full, but not while a field has the keys or a dialog is up: that
                    // Esc closes the dialog. The modal layer is the last frame's, so a dialog this Esc just closed
                    // still counts.
                    let dialog = ui.ctx().memory(|m| m.top_modal_layer().is_some());
                    if layout == (PanelLayout::Full { forced: false })
                        && !typing
                        && !dialog
                        && ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        self.panel_fullscreen = false;
                    }
                }
            }
            self.field_had_focus = ui.ctx().memory(|m| m.focused().is_some());
        }
        let prs = self.pr_marks();
        let commands = if covers_board {
            Vec::new()
        } else {
            kanban::main(
                ui,
                &schema,
                &shown,
                now,
                &mut self.cards,
                self.preview.as_ref().map(|p| p.agent_id.as_str()),
                &tags,
                self.fleet_display,
                &prs,
            )
        };

        for c in commands {
            self.run_command(c);
        }
        self.stop_task_dialog(ui.ctx());
        match preview_action {
            PreviewAction::None => {}
            PreviewAction::Close => self.close_preview(),
            PreviewAction::ToggleFullscreen => self.panel_fullscreen = !self.panel_fullscreen,
            PreviewAction::OpenTerminal => self.open_terminal(),
            PreviewAction::Reply(input) => {
                if let Some((action, agent)) =
                    reply_target.filter(|(_, a)| !self.replying.contains(&a.id.0))
                {
                    // The draft stays until the send succeeds: a failed one (daemon down, agent gone) keeps it.
                    let agent_id = agent.id.0.clone();
                    self.replying.insert(agent_id.clone());
                    self.background(move |client| {
                        let result = client.run(&action, &agent, Some(&input)).map_err(|e| e.0);
                        AppResult::Replied {
                            agent_id,
                            sent: input,
                            result,
                        }
                    });
                }
            }
            // The picker's chosen entries leave one per frame, so a restart may begin halfway: the rest wait for it.
            PreviewAction::AddRepo(repo) if daemon_blocked.is_some() => {
                if let Some(prev) = self.preview.as_mut() {
                    prev.queue.push_front(repo);
                }
            }
            PreviewAction::AddRepo(repo) => self.add_repo(repo),
            PreviewAction::Grant(repo) => {
                self.decide(true, move |c, id| c.grant_repo(id, &repo).map(drop))
            }
            PreviewAction::Refuse(reason) => {
                self.decide(false, move |c, id| c.refuse_repo(id, &reason))
            }
            PreviewAction::Approve => self.decide_breakdown(None),
            PreviewAction::RequestChanges(comment) => self.decide_breakdown(Some(comment)),
            PreviewAction::Open(id) => self.open_preview(&id),
            PreviewAction::LoadDiff => self.load_diff(),
        }
    }
}

/// The restart thread's line back to the window. Dropped before it sent the end of the restart (a panic in `stop`,
/// `ensure` or the config write), it sends a failure, so `restarting` cannot stay set and block Save for good.
struct RestartOutcome {
    tx: Sender<AppResult>,
    ctx: egui::Context,
    done: bool,
}

impl RestartOutcome {
    fn send(&self, r: AppResult) {
        let _ = self.tx.send(r);
        self.ctx.request_repaint();
    }
}

impl Drop for RestartOutcome {
    fn drop(&mut self) {
        if !self.done {
            self.send(AppResult::Restarted(Err("restart thread panicked".into())));
        }
    }
}

fn confirm_text(kind: RestartFor) -> &'static str {
    match kind {
        RestartFor::Save(_) => "An agent is waiting for an answer about a repo. Saving restarts the daemon, which ends that request: it is marked interrupted by daemon restart, and the agent has to ask again.",
        RestartFor::Restart => "An agent is waiting for an answer about a repo. Restarting the daemon ends that request: it is marked interrupted by daemon restart, and the agent has to ask again.",
    }
}

/// The schema's Stop for this agent, when it offers one now.
fn stop_action<'a>(schema: &'a UiSchema, agent: &Agent) -> Option<&'a ActionDef> {
    visible_actions(schema, agent)
        .into_iter()
        .find(|a| a.id == "stop")
}

/// The footer line for a finished archive: what it freed, or the first thing it kept (the daemon's note is
/// "kept <path>: <why>", several joined by "; ").
fn archived_text(name: &str, report: &ArchiveReport) -> String {
    match report.kept.as_deref() {
        Some(kept) => {
            let first = kept.split("; ").next().unwrap_or(kept);
            let first = first.strip_prefix("kept ").unwrap_or(first);
            let path = first.split_once(": ").map_or(first, |(path, _)| path);
            format!("{name} archived, kept {path}")
        }
        None => format!("{name} archived, {} freed", format_bytes(report.freed)),
    }
}

/// The panel header: `repos / branches`, or the task folder while the task has no repo.
fn panel_path(agent: &Agent) -> String {
    if agent.workspaces.is_empty() {
        return agent.workdir.clone();
    }
    let joined = |key: &str| match field_value(agent, key) {
        FieldValue::List(items) => items.join(", "),
        _ => String::new(),
    };
    format!("{} / {}", joined("repos"), joined("branch"))
}

impl eframe::App for FleetApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        FleetApp::ui(self, ui);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        SettingsWriter::shared().flush();
    }
}
