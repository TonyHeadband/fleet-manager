//! The daemon's answers about the tools, and the installs that change them: shared by the first-run wizard, the
//! wizard relaunched from Settings and the Extras tab. The GUI never looks for a tool itself; it asks the daemon.
use crate::components::{requirements_ui, RequirementsAction, ToolAction};
use crate::theme::{size, tokens};
use eframe::egui::{self, RichText, Ui};
use fleet_api::{Os, ToolId, ToolList};
use fleet_client::http::FleetClient;
use fleet_client::install::{install_argv, run_install};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

/// Asks the daemon about the tools (or to check them again); runs on a background thread.
pub type ToolsFn = Arc<dyn Fn() -> Result<ToolList, String> + Send + Sync>;
/// Runs a tool's install command in a window of its own and returns when that window closes; runs on a background
/// thread.
pub type InstallFn = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// Stand-ins for the daemon's tool answers and the install window (for tests); `None` is the real thing.
#[derive(Clone, Default)]
pub struct ToolSeams {
    pub tools: Option<ToolsFn>,
    pub recheck: Option<ToolsFn>,
    pub install: Option<InstallFn>,
}

impl ToolSeams {
    /// The daemon at `base_url`'s answer about the tools.
    pub fn tools(&self, base_url: &str) -> ToolsFn {
        self.tools.clone().unwrap_or_else(|| {
            let client = FleetClient::new(base_url);
            let local = fleet_client::login::is_loopback_url(base_url);
            Arc::new(move || {
                client
                    .tools()
                    .map(|l| adopt_path(l, local))
                    .map_err(|e| e.to_string())
            })
        })
    }

    /// The daemon at `base_url` checking the tools again.
    pub fn recheck(&self, base_url: &str) -> ToolsFn {
        self.recheck.clone().unwrap_or_else(|| {
            let client = FleetClient::new(base_url);
            let local = fleet_client::login::is_loopback_url(base_url);
            Arc::new(move || {
                client
                    .recheck_tools()
                    .map(|l| adopt_path(l, local))
                    .map_err(|e| e.to_string())
            })
        })
    }

    /// How to run an install here. The command is the daemon's, for the daemon's machine, so only a daemon on this
    /// machine gets one run here; `None` too when there is no terminal to run it in.
    pub fn install(&self, base_url: &str) -> Option<InstallFn> {
        if let Some(f) = &self.install {
            return Some(f.clone());
        }
        if !fleet_client::login::is_loopback_url(base_url) {
            return None;
        }
        let os = Os::current();
        // Windows: the plain console, which runs every install command; the GUI does not go looking for mintty.
        let terminal = match os {
            Os::Windows => None,
            Os::Linux => Some(fleet_client::login::terminal()?),
        };
        Some(Arc::new(move |command: &str| {
            let argv = install_argv(command, os, None, terminal.clone())
                .ok_or("No terminal was found to run the install in")?;
            let mut child =
                run_install(argv).map_err(|e| format!("Could not open the install window: {e}"))?;
            child.wait().map(drop).map_err(|e| e.to_string())
        }))
    }
}

/// A daemon on this machine found the tools on `list.path`: the programs the GUI starts (a login terminal, an
/// install, git) get it too, so a tool installed since the GUI started, or one only a login shell's PATH has (a
/// menu launch on Linux), is found by them. The GUI never looks for a tool itself; this is the daemon's answer.
fn adopt_path(list: ToolList, local: bool) -> ToolList {
    if let Some(path) = list.path.as_ref().filter(|_| local) {
        fleet_fs::child::set_path(path.into());
    }
    list
}

/// The tool list as the daemon last gave it, with an install and a Check again possibly in flight beside each other.
pub struct ToolCheck {
    ctx: egui::Context,
    list: ToolList,
    /// The tool whose install window is open, and the recheck that follows its closing.
    installing: Option<(ToolId, Receiver<Result<ToolList, String>>)>,
    /// A Check again in flight, beside any install.
    rechecking: Option<Receiver<Result<ToolList, String>>>,
    /// Why the list is not the daemon's latest answer, when it is not.
    note: Option<String>,
    recheck: ToolsFn,
    /// `None` when the app cannot run an install itself.
    install: Option<InstallFn>,
}

impl ToolCheck {
    /// `list` as already answered; `note` says why it is not the daemon's answer, when it is not.
    pub fn new(
        ctx: &egui::Context,
        list: ToolList,
        note: Option<String>,
        recheck: ToolsFn,
        install: Option<InstallFn>,
    ) -> ToolCheck {
        ToolCheck {
            ctx: ctx.clone(),
            list,
            installing: None,
            rechecking: None,
            note,
            recheck,
            install,
        }
    }

    /// Nothing known yet: the daemon is asked to check the tools now.
    pub fn checking(
        ctx: &egui::Context,
        recheck: ToolsFn,
        install: Option<InstallFn>,
    ) -> ToolCheck {
        let empty = ToolList {
            tools: vec![],
            checked_at: 0,
            path: None,
        };
        let mut check = ToolCheck::new(ctx, empty, None, recheck, install);
        check.recheck();
        check
    }

    pub fn list(&self) -> &ToolList {
        &self.list
    }

    /// Why the list is not the daemon's latest answer, when it is not.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// A Check again is in flight.
    pub fn is_checking(&self) -> bool {
        self.rechecking.is_some()
    }

    /// The tool whose install window is open.
    pub fn installing(&self) -> Option<ToolId> {
        self.installing.as_ref().map(|(tool, _)| *tool)
    }

    /// Whether Install may be offered now: the app can run one, and nothing is in flight that it would race.
    pub fn can_install(&self) -> bool {
        self.install.is_some() && self.rechecking.is_none() && self.installing.is_none()
    }

    /// Takes in the answers that have come.
    pub fn poll(&mut self) {
        let mut answers = Vec::new();
        if let Some(answer) = self.rechecking.as_ref().and_then(poll_answer) {
            self.rechecking = None;
            answers.push(answer);
        }
        if let Some(answer) = self.installing.as_ref().and_then(|(_, rx)| poll_answer(rx)) {
            self.installing = None;
            answers.push(answer);
        }
        for answer in answers {
            match answer {
                Ok(new) => {
                    self.list = new;
                    self.note = None;
                }
                Err(e) => self.note = Some(e),
            }
        }
    }

    /// Asks the daemon to check again. A second click starts a fresh check; the answer to the first is no longer
    /// waited for.
    pub fn recheck(&mut self) {
        let recheck = self.recheck.clone();
        self.rechecking = Some(spawn_answer(&self.ctx, move || recheck()));
    }

    /// Runs what a tool's row asked for. One install at a time, and none while a check is in flight.
    pub fn act(&mut self, tool: ToolId, action: ToolAction) {
        let Some(command) = copy_or_open(&self.ctx, action) else {
            return;
        };
        // The button is only drawn when this holds.
        let (Some(run), true) = (self.install.clone(), self.can_install()) else {
            return;
        };
        let recheck = self.recheck.clone();
        // When the window closes, whatever happened in it, the daemon says what is there now.
        let rx = spawn_answer(&self.ctx, move || {
            run(&command)?;
            recheck()
        });
        self.installing = Some((tool, rx));
    }

    /// The note, when there is one, in the danger colour.
    pub fn note_ui(&self, ui: &mut Ui) {
        if let Some(note) = self.note.as_deref() {
            let p = tokens(ui.visuals().dark_mode);
            ui.add(egui::Label::new(RichText::new(note).size(size::SMALL).color(p.danger)).wrap());
            ui.add_space(6.0);
        }
    }

    /// The Requirements step's body: the note, a row per tool, Check again and Continue.
    pub fn requirements_ui(&mut self, ui: &mut Ui) -> Option<RequirementsAction> {
        self.poll();
        self.note_ui(ui);
        // Nothing to wait on: an Install click would otherwise race the answer in flight.
        let can_install = self.install.is_some() && self.rechecking.is_none();
        requirements_ui(ui, &self.list, self.installing(), |_| can_install)
    }
}

/// Copies the command or opens the link a tool's row asked for; an Install is handed back, for the caller to run.
pub fn copy_or_open(ctx: &egui::Context, action: ToolAction) -> Option<String> {
    match action {
        ToolAction::CopyCommand(command) => ctx.copy_text(command),
        ToolAction::OpenLink(link) => ctx.open_url(egui::OpenUrl::new_tab(link)),
        ToolAction::Install(command) => return Some(command),
    }
    None
}

/// The answer on `rx` once it has come; a thread that ended without one is an error.
fn poll_answer(rx: &Receiver<Result<ToolList, String>>) -> Option<Result<ToolList, String>> {
    match rx.try_recv() {
        Ok(answer) => Some(answer),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => {
            Some(Err("The check stopped without an answer".to_string()))
        }
    }
}

/// Runs `answer` on a thread; it arrives on the receiver, with a repaint.
pub fn spawn_answer(
    ctx: &egui::Context,
    answer: impl FnOnce() -> Result<ToolList, String> + Send + 'static,
) -> Receiver<Result<ToolList, String>> {
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(answer());
        ctx.request_repaint();
    });
    rx
}
