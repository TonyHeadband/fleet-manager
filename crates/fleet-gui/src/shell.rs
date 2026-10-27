//! The window around the board: starting the daemon, and what closing the window does about it.
use crate::app::FleetApp;
use crate::components::{
    browse, centred_card, close_dialog, modal_frame, DialogResult, FolderPicker, PickMode,
};
use crate::cues::CueFile;
use crate::icon::IconSync;
use crate::theme::{mono, size, tokens, APP_NAME};
use crate::widgets::{button, fui, path_edit, ButtonVariant};
use eframe::egui::{self, RichText, TextStyle, Ui};
use fleet_api::Agent;
use fleet_client::close::{close_action, summarize, CloseAction, Summary};
use fleet_client::config::{self, CloseChoice, GuiSettings, SettingsWriter};
use fleet_client::daemon_control::{pidfile_names, Started, StopError};
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
    /// The daemon's `config.toml`, where Save writes the tasks folder.
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
    /// `state_dir` and the pid of the daemon this window started, when known: checked before a silent stop.
    owner: Option<(PathBuf, u32)>,
    stop: Stopper,
    phase: Phase,
    pending: Option<FlowStep>,
    /// A choice "Remember my choice" saved and Settings has not been told yet.
    remembered: Option<Option<CloseChoice>>,
    /// Failed background saves of a remembered choice, reported on the next frame.
    save_errors: (mpsc::Sender<String>, Receiver<String>),
}

impl CloseFlow {
    pub fn new(settings_path: PathBuf, we_started_daemon: bool, stop: Stopper) -> CloseFlow {
        let settings = GuiSettings::load(&settings_path);
        CloseFlow {
            settings_path,
            settings,
            we_started: we_started_daemon,
            owner: None,
            stop,
            phase: Phase::Idle,
            pending: None,
            remembered: None,
            save_errors: mpsc::channel(),
        }
    }

    /// The daemon this window started is `pid`, whose `daemon.pid` is in `state_dir`.
    pub fn owning(mut self, state_dir: PathBuf, pid: u32) -> CloseFlow {
        self.owner = Some((state_dir, pid));
        self
    }

    /// A restart from Settings leaves this window owning the daemon it started, whoever started the first one.
    pub fn set_started(&mut self, state_dir: PathBuf, pid: u32) {
        self.we_started = true;
        self.owner = Some((state_dir, pid));
    }

    /// Started by this window and not replaced since: someone may have restarted it by hand, and that daemon is
    /// not this window's to stop without asking.
    fn still_ours(&self) -> bool {
        self.we_started
            && self
                .owner
                .as_ref()
                .is_none_or(|(dir, pid)| pidfile_names(dir, *pid))
    }

    /// The user asked to close the window. `CloseNow` means go ahead; `KeepOpen` means the flow now owns the
    /// decision (a dialog, or a daemon being stopped) and will say `CloseNow` through `take_step` when done.
    pub fn request_close(&mut self, agents: &[Agent]) -> FlowStep {
        // Already deciding or stopping: a second click on the window's X must not restart or replace that.
        if !matches!(self.phase, Phase::Idle) {
            return FlowStep::KeepOpen;
        }
        // Settings → General changes the choice while this flow is alive; the file, with the changes still being
        // written, is the one source of truth.
        self.settings = SettingsWriter::shared().current(&self.settings_path);
        match close_action(agents, self.still_ours(), self.settings.close_choice) {
            CloseAction::CloseOnly => FlowStep::CloseNow,
            CloseAction::StopDaemonAndClose => {
                self.begin_stop();
                FlowStep::KeepOpen
            }
            CloseAction::Ask(summary) => {
                self.phase = Phase::Asking {
                    summary,
                    remember: false,
                };
                FlowStep::KeepOpen
            }
        }
    }

    /// Saves in the background through `update`, because other parts of the window (Settings) write the same
    /// file and a save of this copy would put back their old values. `request_close` reads the choice back through
    /// the writer, so it sees it before the write lands; a failed write is reported on the next frame.
    fn remember(&mut self, choice: Option<CloseChoice>) {
        self.settings.close_choice = choice;
        self.remembered = Some(choice);
        let errors = self.save_errors.0.clone();
        SettingsWriter::shared().update(
            &self.settings_path,
            move |s| s.close_choice = choice,
            move |saved| {
                if let Err(e) = saved {
                    let _ = errors.send(e.to_string());
                }
            },
        );
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

    /// Keeps an open close dialog's numbers live: agents finish or start while it waits for an answer.
    pub fn refresh(&mut self, agents: &[Agent]) {
        if let Phase::Asking { summary, .. } = &mut self.phase {
            *summary = summarize(agents);
        }
    }

    pub fn take_step(&mut self) -> FlowStep {
        self.pending.take().unwrap_or(FlowStep::KeepOpen)
    }

    /// The dialog while asking, and the progress or failure while stopping.
    pub fn dialog_ui(&mut self, ui: &mut Ui) {
        while let Ok(e) = self.save_errors.1.try_recv() {
            tracing::error!(notify = "error", "Could not remember the close choice: {e}");
        }
        let ctx = ui.ctx().clone();
        let p = tokens(ctx.global_style().visuals.dark_mode);
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Idle => {}
            Phase::Asking {
                summary,
                mut remember,
            } => match close_dialog(&ctx, &summary, &mut remember) {
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
                    egui::Modal::new(egui::Id::unique("stopping"))
                        .frame(modal_frame(&ctx))
                        .show(&ctx, |ui| {
                            ui.label(RichText::new("Stopping the daemon…").color(p.text));
                        });
                    ctx.request_repaint();
                    self.phase = Phase::Stopping(rx);
                }
            },
            Phase::StopFailed(message) => {
                let mut leave = false;
                let mut cancel = false;
                egui::Modal::new(egui::Id::unique("stop_failed"))
                    .frame(modal_frame(&ctx))
                    .show(&ctx, |ui| {
                        ui.set_max_width(520.0);
                        ui.label(
                            RichText::new("The daemon did not stop")
                                .text_style(TextStyle::Heading)
                                .color(p.text_high),
                        );
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

const DEFAULT_ROOT: &str = "~/fleet-tasks";

enum Stage {
    /// No config file yet: ask where tasks live before the daemon is started.
    FirstRun {
        root: String,
        personas: String,
        error: Option<String>,
    },
    Starting(Receiver<Result<Started, String>>),
    Failed(String),
    Running {
        app: Box<FleetApp>,
        flow: Box<CloseFlow>,
        we_started: bool,
    },
}

/// The whole window: a startup screen while the daemon comes up (or an error with Retry), then the board.
pub struct Root {
    ctx: egui::Context,
    icon: IconSync,
    base_url: String,
    settings_path: PathBuf,
    state_dir: PathBuf,
    config_path: PathBuf,
    home: PathBuf,
    ensure: EnsureFn,
    stop: Stopper,
    stage: Stage,
    /// The folders step was (or is being) shown, so the persona wizard is step 2 of 3; otherwise 1 of 2.
    folders_step: bool,
    allow_close: bool,
    /// The first-run fields' Browse… dialogs. No tasks folder exists yet, so nothing is excluded.
    first_run_browse: FolderPicker,
    personas_browse: FolderPicker,
    /// Handed to the board when it starts; `None` for a window built with `running`, whose app has its own.
    log: Option<LogHandle>,
    /// Handed to the board when it starts, as a `CueFile`.
    cues: Option<PathBuf>,
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
        let folders_step = !config::exists(&config_path);
        let stage = if !folders_step {
            Stage::Starting(Self::launch(&ensure, &ctx))
        } else {
            Stage::FirstRun {
                root: DEFAULT_ROOT.to_string(),
                personas: config::DEFAULT_PERSONAS_DIR.to_string(),
                error: None,
            }
        };
        let picker = || {
            FolderPicker::new(
                PickMode::Folder,
                settings_path.clone(),
                vec![],
                home.clone(),
            )
        };
        let (first_run_browse, personas_browse) = (picker(), picker());
        Root {
            ctx,
            icon: IconSync::default(),
            base_url,
            settings_path,
            state_dir,
            config_path,
            home,
            ensure,
            stop,
            stage,
            folders_step,
            allow_close: false,
            first_run_browse,
            personas_browse,
            log: Some(log),
            cues: None,
        }
    }

    /// The board it starts takes cues from `path`, for scripted runs.
    pub fn with_cues(mut self, path: PathBuf) -> Root {
        self.cues = Some(path);
        self
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
        let mut root = Root::new(
            ctx,
            base_url,
            settings_path,
            state_dir,
            config_path,
            home,
            ensure,
            stop,
            log,
        );
        root.folders_step = true;
        root.stage = Stage::FirstRun {
            root: DEFAULT_ROOT.to_string(),
            personas: config::DEFAULT_PERSONAS_DIR.to_string(),
            error: None,
        };
        root
    }

    /// Creates the tasks folder and the config, so the daemon starts with the chosen folders. The daemon creates
    /// the personas folder itself, with the modes it needs. Any failure is the message shown under the fields.
    fn save_first_run(&self, typed: &str, personas: &str) -> Result<(), String> {
        let personas = match personas.trim() {
            "" => return Err("Enter a folder for the personas".into()),
            p => config::expand_home(p, &self.home),
        };
        let dir = config::prepare_tasks_root(typed, &self.home)?;
        match config::write_initial(&self.config_path, &dir, &personas, &self.home) {
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
            icon: IconSync::default(),
            base_url: String::new(),
            settings_path: PathBuf::new(),
            state_dir: PathBuf::new(),
            config_path: PathBuf::new(),
            home: PathBuf::new(),
            ensure,
            stop,
            stage: Stage::Running {
                app: Box::new(app),
                flow: Box::new(flow),
                we_started,
            },
            folders_step: true,
            allow_close: false,
            first_run_browse: FolderPicker::new(
                PickMode::Folder,
                PathBuf::new(),
                vec![],
                PathBuf::new(),
            ),
            personas_browse: FolderPicker::new(
                PickMode::Folder,
                PathBuf::new(),
                vec![],
                PathBuf::new(),
            ),
            log: None,
            cues: None,
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

    /// The text in the first-run Personas folder field, if that screen is showing.
    pub fn first_run_personas(&self) -> Option<&str> {
        match &self.stage {
            Stage::FirstRun { personas, .. } => Some(personas),
            _ => None,
        }
    }

    /// Pre-fills the first-run Personas folder field (for tests and embedding); no effect on other screens.
    pub fn set_first_run_personas(&mut self, text: &str) {
        if let Stage::FirstRun { personas, .. } = &mut self.stage {
            *personas = text.to_string();
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.stage, Stage::Running { .. })
    }

    /// The daemon address the board talks to.
    #[doc(hidden)]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn we_started_daemon(&self) -> bool {
        matches!(
            self.stage,
            Stage::Running {
                we_started: true,
                ..
            }
        )
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        self.icon.update(&self.ctx, ui.visuals().dark_mode);
        let ctx = ui.ctx().clone();
        if let Stage::Starting(rx) = &self.stage {
            match rx.try_recv() {
                Ok(Ok(started)) => {
                    let we_started = matches!(started, Started::StartedByUs(_));
                    // Read again for the pickers' tasks folder. It can break after the daemon read it (a hand edit),
                    // which is no reason to refuse a board whose daemon runs: fall back to the defaults and say so.
                    let (cfg, unreadable) = match config::load(&self.config_path, &self.home) {
                        // Retry re-reads the config, so a `listen` or `state_dir` fixed after a broken start is the
                        // one the daemon now uses; the board follows it instead of the address read at launch.
                        Ok(cfg) => {
                            self.base_url = cfg.base_url();
                            self.state_dir = cfg.state_dir.clone();
                            (cfg, None)
                        }
                        Err(e) => {
                            let mut cfg = config::defaults(&self.config_path, &self.home);
                            if let Ok(listen) = self.base_url.trim_start_matches("http://").parse()
                            {
                                cfg.listen = listen;
                            }
                            (cfg, Some(e))
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
                    let mut app = FleetApp::new(
                        FleetClient::new(&self.base_url),
                        self.ctx.clone(),
                        self.settings_path.clone(),
                        cfg.tasks_root,
                        cfg.personas_dir,
                        self.home.clone(),
                    )
                    .with_daemon(daemon)
                    .with_folders_step(self.folders_step);
                    if let Some(log) = &self.log {
                        app = app.with_log(log.clone());
                    }
                    if let Some(path) = &self.cues {
                        app = app.with_cues(CueFile::new(path.clone()));
                    }
                    let mut flow =
                        CloseFlow::new(self.settings_path.clone(), we_started, self.stop.clone());
                    if let Started::StartedByUs(pid) = started {
                        flow = flow.owning(self.state_dir.clone(), pid);
                    }
                    self.stage = Stage::Running {
                        app: Box::new(app),
                        flow: Box::new(flow),
                        we_started,
                    };
                    // After `with_log`, so the footer hears it.
                    if let Some(e) = unreadable {
                        tracing::warn!(
                            notify = "warning",
                            "Using the default tasks folder until the next start: {e}"
                        );
                    }
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
            Stage::FirstRun {
                root,
                personas,
                error,
            } => {
                let (root_picker, personas_picker) =
                    (&mut self.first_run_browse, &mut self.personas_browse);
                let p = tokens(ui.visuals().dark_mode);
                let mut go = false;
                centred_card(ui, |ui| {
                    ui.label(
                        RichText::new("Step 1 of 3 · Where fleet-manager lives")
                            .text_style(TextStyle::Heading)
                            .color(p.text_high),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(
                            "Where should tasks live? Each task gets its own folder here.",
                        )
                        .color(p.text),
                    );
                    ui.add_space(10.0);
                    go |= folder_row(ui, "Tasks folder", "tasks_folder", root_picker, root);
                    ui.add_space(10.0);
                    go |= folder_row(
                        ui,
                        "Personas folder",
                        "personas_folder",
                        personas_picker,
                        personas,
                    );
                    ui.add_space(4.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new("Each persona gets its own Claude and GitHub login here. fleet-manager never reads the tokens.")
                                .size(size::SMALL)
                                .color(p.text_low),
                        )
                        .wrap(),
                    );
                    if let Some(e) = error.as_deref() {
                        ui.add_space(8.0);
                        ui.label(RichText::new(e).size(size::SMALL).color(p.danger));
                    }
                    ui.add_space(10.0);
                    go |= button(ui, "Next →", ButtonVariant::Primary).clicked();
                });
                if go {
                    let (typed, personas) = (root.clone(), personas.clone());
                    match self.save_first_run(&typed, &personas) {
                        Ok(()) => {
                            self.stage = Stage::Starting(Self::launch(&self.ensure, &self.ctx))
                        }
                        Err(e) => {
                            self.stage = Stage::FirstRun {
                                root: typed,
                                personas,
                                error: Some(e),
                            }
                        }
                    }
                }
            }
            Stage::Failed(message) => {
                let p = tokens(ui.visuals().dark_mode);
                let mut retry = false;
                centred_card(ui, |ui| {
                    ui.label(
                        RichText::new(format!("{APP_NAME} could not start its daemon"))
                            .text_style(TextStyle::Heading)
                            .color(p.text_high),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(message.as_str())
                            .font(mono(size::CODE))
                            .color(p.text),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!("State directory: {}", self.state_dir.display()))
                            .size(size::SMALL)
                            .color(p.text_low),
                    );
                    ui.add_space(10.0);
                    retry = button(ui, "Retry", ButtonVariant::Primary).clicked();
                });
                if retry {
                    self.stage = Stage::Starting(Self::launch(&self.ensure, &self.ctx));
                }
            }
            Stage::Running {
                app,
                flow,
                we_started,
            } => {
                if close_requested && !self.allow_close {
                    if flow.request_close(app.agents()) == FlowStep::KeepOpen {
                        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    } else {
                        self.allow_close = true;
                    }
                }
                app.ui(ui);
                flow.refresh(app.agents());
                if let Some(pid) = app.take_restarted() {
                    *we_started = true;
                    flow.set_started(self.state_dir.clone(), pid);
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

/// One first-run folder: its caption, the field and Browse…. True when Enter was pressed in the field.
fn folder_row(
    ui: &mut Ui,
    caption: &str,
    id: &str,
    picker: &mut FolderPicker,
    text: &mut String,
) -> bool {
    let p = tokens(ui.visuals().dark_mode);
    let label = fui::mono_label(ui, caption, p.text_low);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Fills the field only; Next still creates the folder, so the user can edit the pick.
            // Drawn first, from the right, so the field takes exactly the rest of the row.
            let picked = browse(ui, picker, text);
            let (id, width) = (ui.make_persistent_id(id), ui.available_width());
            let edit = path_edit(ui, id, text, width, None, "", picked).labelled_by(label.id);
            // Enter in the field is Next, as in a form.
            edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
        })
        .inner
    })
    .inner
}

impl eframe::App for Root {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        Root::ui(self, ui);
    }

    /// Settings are saved in the background; a choice made just before closing must still reach the file.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        SettingsWriter::shared().flush();
    }
}
