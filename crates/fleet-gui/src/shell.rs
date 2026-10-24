//! The window around the board: starting the daemon, and what closing the window does about it.
use crate::app::FleetApp;
use crate::components::{browse, centred_card, close_dialog, modal_frame, trust_hint, DialogResult, FolderPicker, PickMode};
use crate::theme::{mono, size, tokens, APP_NAME};
use crate::widgets::{button, path_edit, ButtonVariant};
use eframe::egui::{self, RichText, TextStyle, Ui};
use fleet_api::Agent;
use fleet_client::close::{close_action, CloseAction, Summary};
use fleet_client::config::{self, CloseChoice, GuiSettings};
use fleet_client::daemon_control::{Started, StopError};
use fleet_client::http::FleetClient;
use fleet_log::LogHandle;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowStep {
    KeepOpen,
    CloseNow,
}

/// Stops the daemon; runs on a background thread because it can take seconds.
pub type Stopper = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;
/// Makes sure a daemon is up; runs on a background thread.
pub type EnsureFn = Arc<dyn Fn() -> Result<Started, String> + Send + Sync>;

/// What the board needs to show and restart the daemon (Settings → General's Save, Settings → Daemon).
#[derive(Clone)]
pub struct DaemonCtl {
    pub ensure: EnsureFn,
    pub stop: Stopper,
    /// The daemon's `config.toml`, where Save writes the projects folder.
    pub config_path: PathBuf,
    /// Holds `daemon.pid` and the logs.
    pub state_dir: PathBuf,
    /// The daemon's address, as the config gives it.
    pub listen: String,
    /// This window started the daemon that runs now.
    pub we_started: bool,
}

/// Stops the daemon and starts it again; blocking, so it runs on a background thread. A daemon with no pidfile
/// cannot be stopped: `ensure` then starts one only if none answers (after a failed restart, say), and otherwise
/// says `AlreadyRunning`, which tells the caller the old daemon is still the one running.
pub fn restart_daemon(stop: &Stopper, ensure: &EnsureFn) -> Result<Started, String> {
    match stop() {
        Ok(()) => {}
        // `Stopper` reports errors as text; this one is the only one that leaves nothing to stop.
        Err(e) if e == StopError::NoPidfile.to_string() => {}
        Err(e) => return Err(e),
    }
    ensure()
}

enum Phase {
    Idle,
    Asking { summary: Summary, remember: bool },
    Stopping(Receiver<Result<(), String>>),
    StopFailed(String),
}

/// The decision about closing, as a small state machine the window drives: `request_close` when the user
/// closes the window, `dialog_ui` every frame, `take_step` to learn whether the window may now close.
pub struct CloseFlow {
    settings_path: PathBuf,
    settings: GuiSettings,
    we_started: bool,
    stop: Stopper,
    phase: Phase,
    pending: Option<FlowStep>,
    /// A choice "Remember my choice" saved and Settings has not been told yet.
    remembered: Option<Option<CloseChoice>>,
}

impl CloseFlow {
    pub fn new(settings_path: PathBuf, we_started_daemon: bool, stop: Stopper) -> CloseFlow {
        let settings = GuiSettings::load(&settings_path);
        CloseFlow { settings_path, settings, we_started: we_started_daemon, stop, phase: Phase::Idle, pending: None, remembered: None }
    }

    /// A restart from Settings leaves this window owning the daemon, whoever started the first one.
    pub fn set_we_started(&mut self, we_started: bool) {
        self.we_started = we_started;
    }

    /// The user asked to close the window. `CloseNow` means go ahead; `KeepOpen` means the flow now owns the
    /// decision (a dialog, or a daemon being stopped) and will say `CloseNow` through `take_step` when done.
    pub fn request_close(&mut self, agents: &[Agent]) -> FlowStep {
        // Already deciding or stopping: a second click on the window's X must not restart or replace that.
        if !matches!(self.phase, Phase::Idle) {
            return FlowStep::KeepOpen;
        }
        // Settings → General changes the choice while this flow is alive; the file is the one source of truth.
        self.settings = GuiSettings::load(&self.settings_path);
        match close_action(agents, self.we_started, self.settings.close_choice) {
            CloseAction::CloseOnly => FlowStep::CloseNow,
            CloseAction::StopDaemonAndClose => {
                self.begin_stop();
                FlowStep::KeepOpen
            }
            CloseAction::Ask(summary) => {
                self.phase = Phase::Asking { summary, remember: false };
                FlowStep::KeepOpen
            }
        }
    }

    /// Saves through `update` because other parts of the window (Settings) write the same file; a save
    /// of this copy would put back their old values. The file is the only record: `request_close` reads it again,
    /// so a failed save is reported and not kept in memory, where the next close would drop it anyway.
    fn remember(&mut self, choice: Option<CloseChoice>) {
        match GuiSettings::update(&self.settings_path, |s| s.close_choice = choice) {
            Ok(saved) => {
                self.settings = saved;
                self.remembered = Some(choice);
            }
            Err(e) => tracing::error!(notify = "error", "Could not remember the close choice: {e}"),
        }
    }

    /// The choice "Remember my choice" saved since the last call, for Settings → General to show.
    pub fn take_remembered(&mut self) -> Option<Option<CloseChoice>> {
        self.remembered.take()
    }

    fn begin_stop(&mut self) {
        let (tx, rx) = mpsc::channel();
        let stop = self.stop.clone();
        std::thread::spawn(move || {
            let _ = tx.send(stop());
        });
        self.phase = Phase::Stopping(rx);
    }

    pub fn take_step(&mut self) -> FlowStep {
        self.pending.take().unwrap_or(FlowStep::KeepOpen)
    }

    /// The dialog while asking, and the progress or failure while stopping.
    pub fn dialog_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let p = tokens(ctx.global_style().visuals.dark_mode);
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Idle => {}
            Phase::Asking { summary, mut remember } => match close_dialog(&ctx, &summary, &mut remember) {
                None => self.phase = Phase::Asking { summary, remember },
                Some(DialogResult::Cancel) => self.pending = Some(FlowStep::KeepOpen),
                Some(DialogResult::Choose(choice)) => {
                    if remember {
                        self.remember(Some(choice));
                    }
                    match choice {
                        CloseChoice::KeepRunning => self.pending = Some(FlowStep::CloseNow),
                        CloseChoice::StopDaemon => self.begin_stop(),
                    }
                }
            },
            Phase::Stopping(rx) => match rx.try_recv() {
                Ok(Ok(())) => self.pending = Some(FlowStep::CloseNow),
                Ok(Err(e)) => self.phase = Phase::StopFailed(e),
                Err(_) => {
                    egui::Modal::new(egui::Id::unique("stopping")).frame(modal_frame(&ctx)).show(&ctx, |ui| {
                        ui.label(RichText::new("Stopping the daemon…").color(p.text));
                    });
                    ctx.request_repaint();
                    self.phase = Phase::Stopping(rx);
                }
            },
            Phase::StopFailed(message) => {
                let mut leave = false;
                let mut cancel = false;
                egui::Modal::new(egui::Id::unique("stop_failed")).frame(modal_frame(&ctx)).show(&ctx, |ui| {
                    ui.set_max_width(520.0);
                    ui.label(RichText::new("The daemon did not stop").text_style(TextStyle::Heading).color(p.text_high));
                    ui.add_space(6.0);
                    ui.label(RichText::new(&message).color(p.text));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        leave = button(ui, "Close anyway", ButtonVariant::Default).clicked();
                        cancel = button(ui, "Cancel", ButtonVariant::Primary).clicked();
                    });
                });
                if leave {
                    self.pending = Some(FlowStep::CloseNow);
                } else if !cancel {
                    self.phase = Phase::StopFailed(message);
                }
            }
        }
    }
}

const DEFAULT_ROOT: &str = "~/fleet-projects";

enum Stage {
    /// No config file yet: ask where projects live before the daemon is started.
    FirstRun { root: String, error: Option<String> },
    Starting(Receiver<Result<Started, String>>),
    Failed(String),
    Running { app: Box<FleetApp>, flow: Box<CloseFlow>, we_started: bool },
}

/// The whole window: a startup screen while the daemon comes up (or an error with Retry), then the board.
pub struct Root {
    ctx: egui::Context,
    base_url: String,
    settings_path: PathBuf,
    state_dir: PathBuf,
    config_path: PathBuf,
    home: PathBuf,
    ensure: EnsureFn,
    stop: Stopper,
    stage: Stage,
    allow_close: bool,
    /// The first-run field's Browse… dialog. No projects folder exists yet, so nothing is excluded.
    first_run_browse: FolderPicker,
    /// Handed to the board when it starts; `None` for a window built with `running`, whose app has its own.
    log: Option<LogHandle>,
}

impl Root {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ctx: egui::Context,
        base_url: String,
        settings_path: PathBuf,
        state_dir: PathBuf,
        config_path: PathBuf,
        home: PathBuf,
        ensure: EnsureFn,
        stop: Stopper,
        log: LogHandle,
    ) -> Root {
        let stage = if config::exists(&config_path) {
            Stage::Starting(Self::launch(&ensure, &ctx))
        } else {
            Stage::FirstRun { root: DEFAULT_ROOT.to_string(), error: None }
        };
        let first_run_browse = FolderPicker::new(PickMode::Folder, settings_path.clone(), vec![], home.clone());
        Root { ctx, base_url, settings_path, state_dir, config_path, home, ensure, stop, stage, allow_close: false, first_run_browse, log: Some(log) }
    }

    /// A window on the first-run screen whatever is on disk (for tests and embedding).
    #[allow(clippy::too_many_arguments)]
    pub fn first_run(
        ctx: egui::Context,
        base_url: String,
        settings_path: PathBuf,
        state_dir: PathBuf,
        config_path: PathBuf,
        home: PathBuf,
        ensure: EnsureFn,
        stop: Stopper,
        log: LogHandle,
    ) -> Root {
        let mut root = Root::new(ctx, base_url, settings_path, state_dir, config_path, home, ensure, stop, log);
        root.stage = Stage::FirstRun { root: DEFAULT_ROOT.to_string(), error: None };
        root
    }

    /// Creates the folder and the config, so the daemon starts with the chosen root. Any failure is the
    /// message shown under the field.
    fn save_first_run(&self, typed: &str) -> Result<(), String> {
        let dir = config::prepare_projects_root(typed, &self.home)?;
        match config::write_initial(&self.config_path, &dir, &self.home) {
            Ok(()) => Ok(()),
            // Another instance or a hand edit got there first: keep its file and just start.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(e) => Err(format!("Cannot write {}: {e}", self.config_path.display())),
        }
    }

    fn launch(ensure: &EnsureFn, ctx: &egui::Context) -> Receiver<Result<Started, String>> {
        let (tx, rx) = mpsc::channel();
        let (ensure, ctx) = (ensure.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(ensure());
            ctx.request_repaint();
        });
        rx
    }

    /// A window that is already past startup (for tests and embedding): the board and its close flow.
    pub fn running(ctx: egui::Context, app: FleetApp, flow: CloseFlow, we_started: bool) -> Root {
        let ensure: EnsureFn = Arc::new(|| Err("not used".to_string()));
        let stop: Stopper = Arc::new(|| Err("not used".to_string()));
        Root {
            ctx,
            base_url: String::new(),
            settings_path: PathBuf::new(),
            state_dir: PathBuf::new(),
            config_path: PathBuf::new(),
            home: PathBuf::new(),
            ensure,
            stop,
            stage: Stage::Running { app: Box::new(app), flow: Box::new(flow), we_started },
            allow_close: false,
            first_run_browse: FolderPicker::new(PickMode::Folder, PathBuf::new(), vec![], PathBuf::new()),
            log: None,
        }
    }

    pub fn is_first_run(&self) -> bool {
        matches!(self.stage, Stage::FirstRun { .. })
    }

    pub fn is_starting(&self) -> bool {
        matches!(self.stage, Stage::Starting(_))
    }

    /// The text in the first-run field, if that screen is showing.
    pub fn first_run_root(&self) -> Option<&str> {
        match &self.stage {
            Stage::FirstRun { root, .. } => Some(root),
            _ => None,
        }
    }

    /// Pre-fills the first-run field (for tests and embedding); no effect on other screens.
    pub fn set_first_run_root(&mut self, text: &str) {
        if let Stage::FirstRun { root, .. } = &mut self.stage {
            *root = text.to_string();
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.stage, Stage::Running { .. })
    }

    pub fn we_started_daemon(&self) -> bool {
        matches!(self.stage, Stage::Running { we_started: true, .. })
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if let Stage::Starting(rx) = &self.stage {
            match rx.try_recv() {
                Ok(Ok(started)) => {
                    let we_started = started == Started::StartedByUs;
                    // The daemon is up, so the config it started from reads; the pickers need its projects folder.
                    let cfg = match config::load(&self.config_path, &self.home) {
                        Ok(cfg) => cfg,
                        Err(e) => {
                            self.stage = Stage::Failed(e.to_string());
                            return;
                        }
                    };
                    let daemon = DaemonCtl {
                        ensure: self.ensure.clone(),
                        stop: self.stop.clone(),
                        config_path: self.config_path.clone(),
                        state_dir: self.state_dir.clone(),
                        listen: cfg.listen.to_string(),
                        we_started,
                    };
                    let mut app = FleetApp::new(FleetClient::new(&self.base_url), self.ctx.clone(), self.settings_path.clone(), cfg.projects_root, self.home.clone()).with_daemon(daemon);
                    if let Some(log) = &self.log {
                        app = app.with_log(log.clone());
                    }
                    let flow = CloseFlow::new(self.settings_path.clone(), we_started, self.stop.clone());
                    self.stage = Stage::Running { app: Box::new(app), flow: Box::new(flow), we_started };
                }
                Ok(Err(e)) => self.stage = Stage::Failed(e),
                Err(_) => {
                    let p = tokens(ui.visuals().dark_mode);
                    centred_card(ui, |ui| {
                        ui.label(RichText::new("Starting the daemon…").color(p.text_low));
                    });
                    ctx.request_repaint();
                    return;
                }
            }
        }
        // The user closed the window: let the flow decide whether it may go.
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        match &mut self.stage {
            Stage::Starting(_) => {}
            Stage::FirstRun { root, error } => {
                let browse_picker = &mut self.first_run_browse;
                let p = tokens(ui.visuals().dark_mode);
                let mut go = false;
                centred_card(ui, |ui| {
                    ui.label(RichText::new(format!("Welcome to {APP_NAME}")).text_style(TextStyle::Heading).color(p.text_high));
                    ui.add_space(6.0);
                    ui.label(RichText::new("Where should projects live? Each project gets its own folder here.").color(p.text));
                    ui.add_space(10.0);
                    let label = ui.label(RichText::new("Projects folder").size(size::SMALL).color(p.text_low));
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Fills the field only; Continue still creates the folder, so the user can edit the pick.
                            // Drawn first, from the right, so the field takes exactly the rest of the row.
                            let picked = browse(ui, browse_picker, root);
                            let (id, width) = (ui.make_persistent_id("projects_folder"), ui.available_width());
                            let edit = path_edit(ui, id, root, width, None, "", picked).labelled_by(label.id);
                            // Enter in the field is Continue, as in a form.
                            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                go = true;
                            }
                        });
                    });
                    ui.add_space(8.0);
                    trust_hint(ui, root);
                    if let Some(e) = error.as_deref() {
                        ui.add_space(8.0);
                        ui.label(RichText::new(e).size(size::SMALL).color(p.danger));
                    }
                    ui.add_space(10.0);
                    go |= button(ui, "Continue", ButtonVariant::Primary).clicked();
                });
                if go {
                    let typed = root.clone();
                    match self.save_first_run(&typed) {
                        Ok(()) => self.stage = Stage::Starting(Self::launch(&self.ensure, &self.ctx)),
                        Err(e) => self.stage = Stage::FirstRun { root: typed, error: Some(e) },
                    }
                }
            }
            Stage::Failed(message) => {
                let p = tokens(ui.visuals().dark_mode);
                let mut retry = false;
                centred_card(ui, |ui| {
                    ui.label(RichText::new(format!("{APP_NAME} could not start its daemon")).text_style(TextStyle::Heading).color(p.text_high));
                    ui.add_space(6.0);
                    ui.label(RichText::new(message.as_str()).font(mono(size::CODE)).color(p.text));
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("State directory: {}", self.state_dir.display())).size(size::SMALL).color(p.text_low));
                    ui.add_space(10.0);
                    retry = button(ui, "Retry", ButtonVariant::Primary).clicked();
                });
                if retry {
                    self.stage = Stage::Starting(Self::launch(&self.ensure, &self.ctx));
                }
            }
            Stage::Running { app, flow, we_started } => {
                if close_requested && !self.allow_close {
                    if flow.request_close(app.agents()) == FlowStep::KeepOpen {
                        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    } else {
                        self.allow_close = true;
                    }
                }
                app.ui(ui);
                if app.take_restarted() {
                    *we_started = true;
                    flow.set_we_started(true);
                }
                flow.dialog_ui(ui);
                if let Some(choice) = flow.take_remembered() {
                    app.set_close_choice(choice);
                }
                if flow.take_step() == FlowStep::CloseNow {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }
}

/// The command that trusts the folder for Claude, safe to paste into a shell. A leading `~/` stays outside the
/// quotes because the shell does not expand a quoted tilde. An empty field shows the default root.
pub fn trust_command(root: &str) -> String {
    let root = match root.trim() {
        "" => DEFAULT_ROOT,
        r => r,
    };
    let (head, rest) = if root == "~" {
        ("~", "")
    } else if let Some(r) = root.strip_prefix("~/") {
        ("~/", r)
    } else {
        ("", root)
    };
    let plain = !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || "_./-+@%:,=".contains(c));
    let quoted = if plain || rest.is_empty() { rest.to_string() } else { format!("'{}'", rest.replace('\'', "'\\''")) };
    format!("cd {head}{quoted} && claude")
}

impl eframe::App for Root {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        Root::ui(self, ui);
    }
}
