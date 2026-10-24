//! The application: owns the board data, receives what the background threads send, and draws everything.
//! The UI thread never touches the network. Each call runs on its own thread and answers with an `AppResult`.
use crate::components::{
    board, preview_pane, repo_filter, spawn_form, CardState, Command, DiffView, PreviewAction, PreviewState, SpawnState,
};
use crate::theme::{palette, tone_color};
use crate::widgets::{button, ButtonVariant};
use eframe::egui::{self, RichText, Ui};
use fleet_api::{Agent, DiffReport, Snapshot, Tone, UiSchema};
use fleet_client::board::apply;
use fleet_client::http::FleetClient;
use fleet_client::stream::{spawn_event_stream, spawn_pane_stream, PaneMsg, StreamHandle, StreamMsg};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

/// How often a quiet window redraws so the elapsed-time labels stay current (the web UI refreshes every 15 s too).
pub const REFRESH: std::time::Duration = std::time::Duration::from_secs(15);

/// The answer to one background call.
pub enum AppResult {
    Schema(Result<UiSchema, String>),
    Repos(Result<Vec<String>, String>),
    Action(Result<(), String>),
    Spawn(Result<Agent, String>),
    /// The diff for the agent with this id.
    Diff(String, Result<DiffReport, String>),
}

pub struct FleetApp {
    client: Arc<FleetClient>,
    ctx: egui::Context,
    snapshot: Snapshot,
    schema: Option<UiSchema>,
    repos: Vec<String>,
    connected: bool,
    load_error: Option<String>,
    problem: Option<String>,
    filter: Vec<String>,
    cards: HashMap<String, CardState>,
    spawn: SpawnState,
    preview: Option<PreviewState>,
    clock: Box<dyn Fn() -> i64>,
    results_tx: Sender<AppResult>,
    results_rx: Receiver<AppResult>,
    stream_rx: Option<Receiver<StreamMsg>>,
    pane_rx: Option<Receiver<PaneMsg>>,
    // Held for their Drop: dropping a handle stops its thread.
    _events: Option<StreamHandle>,
    pane: Option<StreamHandle>,
}

fn wall_clock() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl FleetApp {
    /// Starts the event stream and loads the schema and repo list in the background.
    pub fn new(client: FleetClient, ctx: egui::Context) -> FleetApp {
        crate::theme::apply(&ctx);
        let mut app = FleetApp::detached(client, ctx.clone());
        let (tx, rx) = mpsc::channel();
        let wake = ctx.clone();
        app._events = Some(spawn_event_stream(&app.client_url(), tx, Arc::new(move || wake.request_repaint())));
        app.stream_rx = Some(rx);
        app.load();
        app
    }

    /// An app with no background threads, for tests: feed it `handle_stream` and `handle_result` yourself.
    pub fn detached(client: FleetClient, ctx: egui::Context) -> FleetApp {
        let (results_tx, results_rx) = mpsc::channel();
        FleetApp {
            client: Arc::new(client),
            ctx,
            snapshot: Snapshot { seq: 0, agents: vec![] },
            schema: None,
            repos: vec![],
            connected: false,
            load_error: None,
            problem: None,
            filter: vec![],
            cards: HashMap::new(),
            spawn: SpawnState::default(),
            preview: None,
            clock: Box::new(wall_clock),
            results_tx,
            results_rx,
            stream_rx: None,
            pane_rx: None,
            _events: None,
            pane: None,
        }
    }

    fn client_url(&self) -> String {
        self.client.base_url().to_string()
    }

    pub fn set_clock(&mut self, f: Box<dyn Fn() -> i64>) {
        self.clock = f;
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
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

    pub fn spawn_state(&self) -> &SpawnState {
        &self.spawn
    }

    /// The agent id whose preview is open.
    pub fn preview_agent(&self) -> Option<&str> {
        self.preview.as_ref().map(|p| p.agent_id.as_str())
    }

    /// Runs `job` on a new thread and delivers its answer to the UI, waking it.
    fn background(&self, job: impl FnOnce(&FleetClient) -> AppResult + Send + 'static) {
        let (client, tx, ctx) = (self.client.clone(), self.results_tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(job(&client));
            ctx.request_repaint();
        });
    }

    fn load(&mut self) {
        self.load_error = None;
        self.background(|c| AppResult::Schema(c.ui_schema().map_err(|e| e.0)));
        self.background(|c| AppResult::Repos(c.repos().map_err(|e| e.0)));
    }

    pub fn handle_stream(&mut self, m: StreamMsg) {
        match m {
            StreamMsg::Connected => self.connected = true,
            StreamMsg::Disconnected => self.connected = false,
            StreamMsg::Snapshot(s) => self.snapshot = s,
            StreamMsg::Event(e) => apply(&mut self.snapshot, &e),
        }
        // A preview of an agent that no longer exists has nothing to show.
        if let Some(id) = self.preview_agent() {
            if !self.snapshot.agents.iter().any(|a| a.id.0 == id) && self.snapshot.seq > 0 {
                self.close_preview();
            }
        }
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
            AppResult::Action(Ok(())) => self.problem = None,
            AppResult::Action(Err(e)) => self.problem = Some(e),
            AppResult::Spawn(Ok(_)) => {
                self.spawn.busy = false;
                self.spawn.prompt.clear();
                self.spawn.error = None;
            }
            AppResult::Spawn(Err(e)) => {
                self.spawn.busy = false;
                self.spawn.error = Some(e);
            }
            AppResult::Diff(id, result) => {
                if let Some(p) = self.preview.as_mut().filter(|p| p.agent_id == id) {
                    p.diff = match result {
                        Ok(report) => DiffView::Loaded(report),
                        Err(e) => DiffView::Failed(e),
                    };
                }
            }
        }
    }

    fn close_preview(&mut self) {
        self.preview = None;
        self.pane = None;
        self.pane_rx = None;
    }

    fn open_preview(&mut self, id: &str) {
        let Some(agent) = self.snapshot.agents.iter().find(|a| a.id.0 == id) else { return };
        self.preview = Some(PreviewState { agent_id: id.to_string(), title: agent.task.clone(), ..PreviewState::default() });
        let (tx, rx) = mpsc::channel();
        let wake = self.ctx.clone();
        self.pane = Some(spawn_pane_stream(&self.client_url(), id, tx, Arc::new(move || wake.request_repaint())));
        self.pane_rx = Some(rx);
    }

    fn drain(&mut self) {
        while let Some(m) = self.stream_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.handle_stream(m);
        }
        while let Some(m) = self.pane_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.handle_pane(m);
        }
        while let Ok(r) = self.results_rx.try_recv() {
            self.handle_result(r);
        }
    }

    fn run_command(&mut self, c: Command) {
        match c {
            Command::Run { action, agent, input } => {
                self.problem = None;
                self.background(move |client| AppResult::Action(client.run(&action, &agent, input.as_deref()).map_err(|e| e.0)));
            }
            Command::Open(id) => self.open_preview(&id),
        }
    }

    /// Draws the whole window into `ui`.
    pub fn ui(&mut self, ui: &mut Ui) {
        self.drain();
        let dark = ui.visuals().dark_mode;
        let p = palette(dark);
        let now = (self.clock)();
        // Durations such as "5m" must keep counting on a quiet board, where no event would repaint the window.
        ui.ctx().request_repaint_after(REFRESH);

        ui.horizontal(|ui| {
            ui.heading("fleet-board");
            let (dot, text) = if self.connected { (tone_color(Tone::Done, dark), "live") } else { (tone_color(Tone::Attention, dark), "reconnecting…") };
            // Painted, not a font glyph: the bundled fonts have no circle.
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.0, dot);
            ui.label(RichText::new(text).small().color(p.muted));
            let mut all: Vec<String> = self.repos.clone();
            for r in self.snapshot.agents.iter().flat_map(|a| a.workspaces.iter().map(|w| w.repo.clone())) {
                if !all.contains(&r) {
                    all.push(r);
                }
            }
            repo_filter(ui, &all, &mut self.filter);
        });

        if let Some((repo, prompt)) = spawn_form(ui, &self.repos, &mut self.spawn) {
            self.spawn.busy = true;
            self.spawn.error = None;
            self.background(move |client| AppResult::Spawn(client.spawn(&repo, &prompt).map_err(|e| e.0)));
        }
        if let Some(problem) = &self.problem {
            ui.label(RichText::new(problem).color(p.danger));
        }

        let Some(schema) = self.schema.clone() else {
            if let Some(e) = self.load_error.clone() {
                ui.label(RichText::new(format!("Cannot load the board: {e}")).color(p.danger));
                if button(ui, "Retry", ButtonVariant::Default).clicked() {
                    self.load();
                }
            } else {
                ui.label("Loading…");
            }
            return;
        };

        let shown: Vec<Agent> = self
            .snapshot
            .agents
            .iter()
            .filter(|a| self.filter.is_empty() || a.workspaces.iter().any(|w| self.filter.contains(&w.repo)))
            .cloned()
            .collect();

        let mut commands: Vec<Command> = Vec::new();
        let mut preview_action = PreviewAction::None;
        if let Some(prev) = self.preview.as_mut() {
            egui::Panel::right("preview").resizable(true).default_size(460.0).show(ui, |ui| {
                preview_action = preview_pane(ui, prev);
            });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            commands = board(ui, &schema, &shown, now, &mut self.cards);
        });

        for c in commands {
            self.run_command(c);
        }
        match preview_action {
            PreviewAction::None => {}
            PreviewAction::Close => self.close_preview(),
            PreviewAction::LoadDiff => {
                if let Some(id) = self.preview_agent().map(str::to_string) {
                    self.background(move |client| AppResult::Diff(id.clone(), client.diff(&id).map_err(|e| e.0)));
                }
            }
        }
    }
}

impl eframe::App for FleetApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        FleetApp::ui(self, ui);
    }
}
