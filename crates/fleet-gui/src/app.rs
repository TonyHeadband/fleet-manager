//! The application: owns the board data, receives what the background threads send, and draws everything.
//! The UI thread never touches the network. Each call runs on its own thread and answers with an `AppResult`.
use crate::components::{
    board, centred_card, new_agent_dialog, preview_pane, repo_filter, CardState, Command, DiffView, FolderPicker, PickMode, PreviewAction, PreviewState, RepoPanel, Reply, RequestDraft, SpawnState,
};
use crate::theme::{self, size, tokens, tone_color};
use crate::widgets::{button, dot, icon_button, ButtonVariant, Icon};
use eframe::egui::{self, Align, Frame, Layout, Margin, Panel, RichText, TextStyle, Ui};
use fleet_api::{ActionDef, Agent, DiffReport, RepoInfo, Snapshot, Tone, UiSchema};
use fleet_client::board::apply;
use fleet_client::config::{GuiSettings, ThemeChoice};
use fleet_client::http::FleetClient;
use fleet_client::schema::{field_value, visible_actions, FieldValue};
use fleet_client::stream::{spawn_event_stream, spawn_pane_stream, PaneMsg, StreamHandle, StreamMsg};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

/// How often a quiet window redraws so the elapsed-time labels stay current (the web UI refreshes every 15 s too).
pub const REFRESH: std::time::Duration = std::time::Duration::from_secs(15);

/// Starts the top bar's line for a spawn that failed out of sight; a later success clears only a line like it.
const SPAWN_FAILED: &str = "Could not start the agent:";

/// The answer to one background call.
pub enum AppResult {
    Schema(Result<UiSchema, String>),
    Repos(Result<Vec<RepoInfo>, String>),
    Action(Result<Option<String>, String>),
    /// One `add_repo` finished for the agent with this id.
    RepoAdded(String, Result<(), String>),
    /// A grant or refusal finished for the agent with this id.
    RequestDecided(String, Result<(), String>),
    Spawn(Result<Agent, String>),
    /// The diff for the agent with this id.
    Diff(String, Result<DiffReport, String>),
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
    projects_root: PathBuf,
    home: PathBuf,
}

impl Locations {
    fn repo_picker(&self) -> FolderPicker {
        FolderPicker::new(PickMode::Repo, self.settings.clone(), Some(self.projects_root.clone()), self.home.clone())
    }
}

pub struct FleetApp {
    client: Arc<FleetClient>,
    ctx: egui::Context,
    snapshot: Snapshot,
    schema: Option<UiSchema>,
    repos: Vec<RepoInfo>,
    connected: bool,
    load_error: Option<String>,
    problem: Option<String>,
    filter: Vec<String>,
    cards: HashMap<String, CardState>,
    spawn: SpawnState,
    preview: Option<PreviewState>,
    /// The half-written reply for each agent id, kept while the user looks at other cards.
    replies: HashMap<String, String>,
    /// The grant target and refusal reason typed for each agent with a pending repo request.
    requests: HashMap<String, RequestText>,
    /// The Browse… dialog beside each pending request's target, kept with its draft.
    grant_pickers: HashMap<String, FolderPicker>,
    locations: Locations,
    /// Agents with a grant or refusal in flight: a second would find the request gone and show a false error.
    deciding: HashSet<String>,
    /// Repos still to add per agent. They go one at a time, so the problem line sees their results in order.
    add_queue: HashMap<String, VecDeque<String>>,
    /// Agents with an `add_repo` in flight.
    adding: HashSet<String>,
    /// Agents whose current batch already failed: the first error is the one shown.
    add_failed: HashSet<String>,
    theme: ThemeChoice,
    /// Where the theme choice is saved; `None` keeps it for this run only.
    settings_path: Option<PathBuf>,
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
    /// `settings_path` is `gui.toml`; `projects_root` and `home` are what the Browse… pickers exclude and start from.
    pub fn new(client: FleetClient, ctx: egui::Context, settings_path: PathBuf, projects_root: PathBuf, home: PathBuf) -> FleetApp {
        crate::theme::apply(&ctx);
        let mut app = FleetApp::detached(client, ctx.clone()).with_locations(settings_path.clone(), projects_root, home).with_settings(settings_path);
        let (tx, rx) = mpsc::channel();
        let wake = ctx.clone();
        app._events = Some(spawn_event_stream(&app.client_url(), tx, Arc::new(move || wake.request_repaint())));
        app.stream_rx = Some(rx);
        app.load();
        app
    }

    fn with_locations(mut self, settings: PathBuf, projects_root: PathBuf, home: PathBuf) -> FleetApp {
        self.locations = Locations { settings, projects_root, home };
        self.spawn = SpawnState::new(self.locations.repo_picker());
        self
    }

    /// An app with no background threads, for tests: feed it `handle_stream` and `handle_result` yourself. Its
    /// Browse… pickers know no folders; tests that browse build their own.
    pub fn detached(client: FleetClient, ctx: egui::Context) -> FleetApp {
        let (results_tx, results_rx) = mpsc::channel();
        let locations = Locations { settings: PathBuf::new(), projects_root: PathBuf::new(), home: PathBuf::new() };
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
            spawn: SpawnState::new(locations.repo_picker()),
            preview: None,
            replies: HashMap::new(),
            requests: HashMap::new(),
            grant_pickers: HashMap::new(),
            locations,
            deciding: HashSet::new(),
            add_queue: HashMap::new(),
            adding: HashSet::new(),
            add_failed: HashSet::new(),
            theme: ThemeChoice::default(),
            settings_path: None,
            clock: Box::new(wall_clock),
            results_tx,
            results_rx,
            stream_rx: None,
            pane_rx: None,
            _events: None,
            pane: None,
        }
    }

    /// Reads the saved theme choice from `path` and saves later changes there. `main` applies the stored
    /// preference before the first frame, so this only has to remember it for the switch to cycle from.
    pub fn with_settings(mut self, path: PathBuf) -> FleetApp {
        self.theme = GuiSettings::load(&path).theme;
        self.settings_path = Some(path);
        self
    }

    pub fn theme_choice(&self) -> ThemeChoice {
        self.theme
    }

    fn cycle_theme(&mut self) {
        self.theme = self.theme.next();
        self.ctx.set_theme(theme::preference(self.theme));
        if let Some(path) = &self.settings_path {
            let choice = self.theme;
            if let Err(e) = GuiSettings::update(path, |s| s.theme = choice) {
                self.problem = Some(format!("Could not save the theme: {e}"));
            }
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

    /// The reply typed so far for this agent; empty once sent.
    pub fn reply_text(&self, agent_id: &str) -> &str {
        self.replies.get(agent_id).map_or("", String::as_str)
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
        self.refresh_repos();
    }

    /// The daemon remembers a repo whenever one is attached (picked, added, granted or `git init`ed by an agent),
    /// so the list is fetched again wherever it is about to be offered or may just have grown.
    fn refresh_repos(&self) {
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
            AppResult::Action(Ok(kept)) => self.problem = kept,
            AppResult::Action(Err(e)) => self.problem = Some(e),
            AppResult::RepoAdded(id, result) => {
                if result.is_ok() {
                    self.refresh_repos();
                }
                // A later success must not wipe an earlier failure of the same batch.
                if let Err(e) = result {
                    if self.add_failed.insert(id.clone()) {
                        self.problem = Some(e);
                    }
                }
                self.adding.remove(&id);
                if !self.send_next_add(&id) {
                    self.add_failed.remove(&id);
                }
            }
            AppResult::RequestDecided(id, result) => {
                if result.is_ok() {
                    self.refresh_repos();
                }
                self.deciding.remove(&id);
                self.problem = result.err();
            }
            AppResult::Spawn(Ok(_)) => {
                self.refresh_repos();
                self.spawn.busy = false;
                self.spawn.error = None;
                if self.spawn.sent.take().is_some_and(|sent| sent == self.spawn.prompt) {
                    self.spawn.open = false;
                    self.spawn.prompt.clear();
                    // The picker stays: it holds the Browse… dialog, which is not part of what was sent.
                    self.spawn.picker.chosen.clear();
                    self.spawn.picker.typed.clear();
                }
                if self.problem.as_deref().is_some_and(|p| p.starts_with(SPAWN_FAILED)) {
                    self.problem = None;
                }
            }
            AppResult::Spawn(Err(e)) => {
                self.spawn.busy = false;
                // Closed while in flight: the dialog's error line is out of sight, so the top bar says it too.
                if !self.spawn.open {
                    self.problem = Some(format!("{SPAWN_FAILED} {e}"));
                }
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

    /// True while a grant or refusal for this agent is in flight.
    pub fn deciding(&self, agent_id: &str) -> bool {
        self.deciding.contains(agent_id)
    }

    /// Queues `repo` for the previewed agent; the queue is sent one call at a time. The board changes through the
    /// daemon's `WorkspacesChanged` events, so a success has nothing to apply here.
    fn add_repo(&mut self, repo: String) {
        let Some(id) = self.preview_agent().map(str::to_string) else { return };
        let idle = !self.adding.contains(&id);
        if idle {
            // A new batch starts clean; mid-batch the line is left alone so a failure stays visible.
            self.problem = None;
        }
        self.add_queue.entry(id.clone()).or_default().push_back(repo);
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
        self.background(move |client| AppResult::RepoAdded(id.clone(), client.add_repo(&id, &repo).map(drop).map_err(|e| e.0)));
        true
    }

    /// Runs a grant or refusal for the previewed agent, unless one is already in flight.
    fn decide(&mut self, call: impl FnOnce(&FleetClient, &str) -> Result<(), fleet_client::http::FleetError> + Send + 'static) {
        let Some(id) = self.preview_agent().map(str::to_string) else { return };
        if !self.deciding.insert(id.clone()) {
            return;
        }
        self.problem = None;
        self.background(move |client| AppResult::RequestDecided(id.clone(), call(client, &id).map_err(|e| e.0)));
    }

    fn close_preview(&mut self) {
        self.preview = None;
        self.pane = None;
        self.pane_rx = None;
    }

    fn open_preview(&mut self, id: &str) {
        let Some(agent) = self.snapshot.agents.iter().find(|a| a.id.0 == id) else { return };
        let path = panel_path(agent);
        self.preview = Some(PreviewState { agent_id: id.to_string(), title: agent.task.clone(), path, focus_reply: true, ..PreviewState::new(self.locations.repo_picker()) });
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
        // egui's own transitions (hover fades, collapsing) follow the same focus gate as the app's animations.
        let transition = if theme::animate(ui.ctx()) { theme::ANIMATION_TIME } else { 0.0 };
        if ui.ctx().global_style().animation_time != transition {
            ui.ctx().all_styles_mut(|s| s.animation_time = transition);
        }
        self.panels(ui);
        // After the panels, and even when they stopped early on a missing schema: the dialog only needs the repos.
        if let Some((repos, prompt)) = new_agent_dialog(ui.ctx(), &self.repos, &mut self.spawn) {
            self.spawn.busy = true;
            self.spawn.error = None;
            self.spawn.sent = Some(prompt.clone());
            self.background(move |client| AppResult::Spawn(client.spawn(&repos, &prompt).map_err(|e| e.0)));
        }
    }

    fn panels(&mut self, ui: &mut Ui) {
        self.drain();
        let dark = ui.visuals().dark_mode;
        let p = tokens(dark);
        let now = (self.clock)();
        // Durations such as "5m" must keep counting on a quiet board, where no event would repaint the window.
        ui.ctx().request_repaint_after(REFRESH);

        // Repos the user configured, then any an agent uses that is not configured, each with its agent count.
        let mut repos: Vec<(String, usize)> = self.repos.iter().map(|r| (r.name.clone(), 0)).collect();
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

        let mut cycle_theme = false;
        Panel::top("top_bar")
            // The 1 px bottom border is the panel's separator, which the theme draws in the `border` token.
            .frame(Frame::new().fill(p.surface).inner_margin(Margin::symmetric(14, 10)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("fleet-manager").text_style(TextStyle::Heading).color(p.text_high));
                    let (colour, text) = if self.connected { (tone_color(Tone::Done, dark), "live") } else { (tone_color(Tone::Attention, dark), "reconnecting…") };
                    dot(ui, colour, 8.0);
                    ui.label(RichText::new(text).small().color(p.text_low));
                    repo_filter(ui, &repos, &mut self.filter);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if button(ui, "New project", ButtonVariant::Primary).clicked() {
                            self.spawn.open = true;
                            self.refresh_repos();
                        }
                        let label = format!("Theme: {}", self.theme.label());
                        cycle_theme = icon_button(ui, Icon::Theme(self.theme), &label).clicked();
                    });
                });
                if let Some(problem) = &self.problem {
                    ui.label(RichText::new(problem).size(size::SMALL).color(p.danger));
                }
            });
        if cycle_theme {
            self.cycle_theme();
        }

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

        let shown: Vec<Agent> = self
            .snapshot
            .agents
            .iter()
            .filter(|a| self.filter.is_empty() || a.workspaces.iter().any(|w| self.filter.contains(&w.repo)))
            .cloned()
            .collect();

        let mut commands: Vec<Command> = Vec::new();
        let mut preview_action = PreviewAction::None;
        // The previewed agent's first input action (Answer on Needs you) gets the panel's reply box.
        let reply_target: Option<(ActionDef, Agent)> = self.preview.as_ref().and_then(|prev| {
            let agent = self.snapshot.agents.iter().find(|a| a.id.0 == prev.agent_id)?;
            let action = visible_actions(&schema, agent).into_iter().find(|a| a.input.is_some())?;
            Some((action.clone(), agent.clone()))
        });
        // A draft lives only while its request does; a failed grant keeps it, so the edited target is not lost.
        self.requests.retain(|id, d| self.snapshot.agents.iter().any(|a| a.id.0 == *id && a.repo_request.as_ref().is_some_and(|r| r.repo == d.for_repo)));
        self.grant_pickers.retain(|id, _| self.requests.contains_key(id));
        let previewed: Option<Agent> = self.preview.as_ref().and_then(|prev| self.snapshot.agents.iter().find(|a| a.id.0 == prev.agent_id).cloned());
        if let Some(prev) = self.preview.as_mut() {
            // Every frame: an add or a grant changes it while the panel is open.
            if let Some(a) = &previewed {
                prev.path = panel_path(a);
            }
            let pending = previewed.as_ref().and_then(|a| a.repo_request.as_ref().map(|r| (a, r)));
            // The request box replaces the reply box, whatever the schema says.
            let reply = reply_target.as_ref().filter(|_| pending.is_none()).map(|(action, agent)| Reply { label: &action.label, text: self.replies.entry(agent.id.0.clone()).or_default() });
            let request = pending.map(|(agent, req)| {
                let d = self.requests.entry(agent.id.0.clone()).or_insert_with(|| RequestText { for_repo: req.repo.clone(), target: req.repo.clone(), reason: String::new() });
                let browse = self.grant_pickers.entry(agent.id.0.clone()).or_insert_with(|| self.locations.repo_picker());
                RequestDraft { request: req, target: &mut d.target, reason: &mut d.reason, browse, busy: self.deciding.contains(&agent.id.0) }
            });
            let panel = previewed.as_ref().map(|a| RepoPanel { workspaces: &a.workspaces, remembered: &self.repos, request });
            // Like the top bar, the panel's own separator is the 1 px `border` line (on its left edge).
            egui::Panel::right("preview").resizable(true).default_size(420.0).frame(Frame::new().fill(p.surface)).show(ui, |ui| {
                preview_action = preview_pane(ui, prev, reply, panel);
            });
        }
        // No margin: the column headers sit flush under the top bar and the dividers reach the window's bottom edge.
        egui::CentralPanel::default().frame(Frame::new().fill(p.bg)).show(ui, |ui| {
            commands = board(ui, &schema, &shown, now, &mut self.cards, self.preview.as_ref().map(|p| p.agent_id.as_str()));
        });

        for c in commands {
            self.run_command(c);
        }
        match preview_action {
            PreviewAction::None => {}
            PreviewAction::Close => self.close_preview(),
            PreviewAction::Reply(input) => {
                if let Some((action, agent)) = reply_target {
                    self.replies.remove(&agent.id.0);
                    self.run_command(Command::Run { action, agent, input: Some(input) });
                }
            }
            PreviewAction::AddRepo(repo) => self.add_repo(repo),
            PreviewAction::Grant(repo) => self.decide(move |c, id| c.grant_repo(id, &repo).map(drop)),
            PreviewAction::Refuse(reason) => self.decide(move |c, id| c.refuse_repo(id, &reason)),
            PreviewAction::LoadDiff => {
                if let Some(id) = self.preview_agent().map(str::to_string) {
                    self.background(move |client| AppResult::Diff(id.clone(), client.diff(&id).map_err(|e| e.0)));
                }
            }
        }
    }
}

/// The panel header: `repos / branches`, or the project folder while the project has no repo.
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
}
