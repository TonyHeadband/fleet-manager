//! The window around the board: starting the daemon, and what closing the window does about it.
use crate::app::FleetApp;
use crate::components::{centred_card, close_dialog, modal_frame, DialogResult};
use crate::theme::{mono, size, tokens};
use crate::widgets::{button, ButtonVariant};
use eframe::egui::{self, Frame, Margin, Panel, RichText, TextEdit, TextStyle, Ui};
use fleet_api::Agent;
use fleet_client::close::{close_action, CloseAction, Summary};
use fleet_client::config::{self, CloseChoice, GuiSettings};
use fleet_client::daemon_control::Started;
use fleet_client::http::FleetClient;
use std::path::{Path, PathBuf};
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

enum Phase {
    Idle,
    Asking { summary: Summary, remember: bool },
    Stopping(Receiver<Result<(), String>>),
    StopFailed(String),
}

/// The decision about closing, as a small state machine the window drives: `request_close` when the user
/// closes the window, `ui` every frame, `take_step` to learn whether the window may now close.
pub struct CloseFlow {
    settings_path: PathBuf,
    settings: GuiSettings,
    we_started: bool,
    stop: Stopper,
    phase: Phase,
    pending: Option<FlowStep>,
}

impl CloseFlow {
    pub fn new(settings_path: PathBuf, we_started_daemon: bool, stop: Stopper) -> CloseFlow {
        let settings = GuiSettings::load(&settings_path);
        CloseFlow { settings_path, settings, we_started: we_started_daemon, stop, phase: Phase::Idle, pending: None }
    }

    /// The user asked to close the window. `CloseNow` means go ahead; `KeepOpen` means the flow now owns the
    /// decision (a dialog, or a daemon being stopped) and will say `CloseNow` through `take_step` when done.
    pub fn request_close(&mut self, agents: &[Agent]) -> FlowStep {
        // Already deciding or stopping: a second click on the window's X must not restart or replace that.
        if !matches!(self.phase, Phase::Idle) {
            return FlowStep::KeepOpen;
        }
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

    /// Saves through `update` because other parts of the window (the theme switch) write the same file; a save
    /// of this copy would put back their old values. A failed save still applies for this session.
    fn remember(&mut self, choice: Option<CloseChoice>) {
        match GuiSettings::update(&self.settings_path, |s| s.close_choice = choice) {
            Ok(saved) => self.settings = saved,
            Err(_) => self.settings.close_choice = choice,
        }
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

    /// Both parts below, for a flow shown on its own.
    pub fn ui(&mut self, ui: &mut Ui) {
        self.settings_ui(ui);
        self.dialog_ui(ui);
    }

    /// The remembered-choice footer. Draw it before the board: a bottom panel is laid out before the central
    /// panel takes the height left, while a line drawn after the board lands below the window.
    pub fn settings_ui(&mut self, ui: &mut Ui) {
        let Some(choice) = self.settings.close_choice else { return };
        let p = tokens(ui.visuals().dark_mode);
        let mut ask_again = false;
        // The panel's own separator is the 1 px `border` line on its top edge, as for the top bar.
        Panel::bottom("close_choice").frame(Frame::new().fill(p.surface).inner_margin(Margin::symmetric(14, 6))).show(ui, |ui| {
            ui.horizontal(|ui| {
                let what = match choice {
                    CloseChoice::KeepRunning => "keep the daemon running",
                    CloseChoice::StopDaemon => "stop the daemon",
                };
                ui.label(RichText::new(format!("When you close the window: {what}")).size(size::SMALL).color(p.text_low));
                ask_again = button(ui, "Ask me again", ButtonVariant::Ghost).clicked();
            });
        });
        if ask_again {
            self.remember(None);
        }
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
    ) -> Root {
        let stage = if config::exists(&config_path) {
            Stage::Starting(Self::launch(&ensure, &ctx))
        } else {
            Stage::FirstRun { root: DEFAULT_ROOT.to_string(), error: None }
        };
        Root { ctx, base_url, settings_path, state_dir, config_path, home, ensure, stop, stage, allow_close: false }
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
    ) -> Root {
        let mut root = Root::new(ctx, base_url, settings_path, state_dir, config_path, home, ensure, stop);
        root.stage = Stage::FirstRun { root: DEFAULT_ROOT.to_string(), error: None };
        root
    }

    /// Creates the folder and the config, so the daemon starts with the chosen root. Any failure is the
    /// message shown under the field.
    fn save_first_run(&self, typed: &str) -> Result<(), String> {
        let typed = typed.trim();
        if typed.is_empty() {
            return Err("Enter a folder for the projects".into());
        }
        let dir = expand_home(typed, &self.home);
        if !dir.is_absolute() {
            return Err(format!("{} is not an absolute path", dir.display()));
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
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
                    let app = FleetApp::new(FleetClient::new(&self.base_url), self.ctx.clone()).with_settings(self.settings_path.clone());
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
                let p = tokens(ui.visuals().dark_mode);
                let mut go = false;
                centred_card(ui, |ui| {
                    ui.label(RichText::new("Welcome to fleet-board").text_style(TextStyle::Heading).color(p.text_high));
                    ui.add_space(6.0);
                    ui.label(RichText::new("Where should projects live? Each project gets its own folder here.").color(p.text));
                    ui.add_space(10.0);
                    let label = ui.label(RichText::new("Projects folder").size(size::SMALL).color(p.text_low));
                    ui.add_space(4.0);
                    let edit = ui.add(TextEdit::singleline(root).desired_width(f32::INFINITY)).labelled_by(label.id);
                    // Enter in the field is Continue, as in a form.
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        go = true;
                    }
                    ui.add_space(8.0);
                    let command = trust_command(root);
                    // Claude's trust dialog pre-selects "No, exit": "accept" or typing 1 there ends the session.
                    ui.add(
                        egui::Label::new(
                            RichText::new("Trust it once for Claude: run this, choose \"Yes, I trust this folder\" (the second option), then /exit.")
                                .size(size::SMALL)
                                .color(p.text_low),
                        )
                        .wrap(),
                    );
                    ui.add_space(4.0);
                    // Copy first, from the right, so it keeps its size; a long command is cut instead, with egui's
                    // elision tooltip showing it whole.
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if button(ui, "Copy", ButtonVariant::Ghost).clicked() {
                                ui.ctx().copy_text(command.clone());
                            }
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.add(egui::Label::new(RichText::new(&command).font(mono(size::CODE)).color(p.text)).truncate());
                            });
                        });
                    });
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
                    ui.label(RichText::new("fleet-board could not start its daemon").text_style(TextStyle::Heading).color(p.text_high));
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
            Stage::Running { app, flow, .. } => {
                if close_requested && !self.allow_close {
                    if flow.request_close(app.agents()) == FlowStep::KeepOpen {
                        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    } else {
                        self.allow_close = true;
                    }
                }
                flow.settings_ui(ui);
                app.ui(ui);
                flow.dialog_ui(ui);
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

fn expand_home(p: &str, home: &Path) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if p == "~" => home.to_path_buf(),
        None => PathBuf::from(p),
    }
}

impl eframe::App for Root {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        Root::ui(self, ui);
    }
}
