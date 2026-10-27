//! Plays the demo timeline against a running daemon: spawns the tasks, then acts on them and cues the GUI on
//! schedule.
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fleet_api::{
    Actor, AgentKind, Approval, DecideBreakdown, DefKind, GithubAuth, NewPersona, NewProduct,
    NewRelease, RepoFleet, RepoPick, RepoUpsert, SpawnRequest,
};
use fleet_client::http::FleetClient;
use fleet_demo::scenario::parse_duration;
use fleet_demo::seed::seed_repo;
use fleet_demo::timeline::{
    load_shot, local_today, resolve_day, status_word, Action, Cue, Filter, PersonaSpec,
    ReleaseSpec, Target, Timeline,
};

/// How long a cue waits for a spawn that is still creating its worktree.
const AGENT_WAIT: Duration = Duration::from_secs(10);

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fleet-demo-director: {e}");
            ExitCode::FAILURE
        }
    }
}

type Agents = Arc<Mutex<HashMap<String, String>>>;
/// Release key to the daemon's id.
type Releases = HashMap<String, i64>;

fn sleep_until(deadline: Instant) {
    let now = Instant::now();
    if deadline > now {
        std::thread::sleep(deadline - now);
    }
}

fn agent_id(
    agents: &Agents,
    spawn_error: &Mutex<Option<String>>,
    task: &str,
) -> Result<String, String> {
    let deadline = Instant::now() + AGENT_WAIT;
    loop {
        // A failed spawn is the cause; say it instead of waiting out the clock.
        if let Some(e) = spawn_error.lock().map_err(|e| e.to_string())?.clone() {
            return Err(e);
        }
        if let Some(id) = agents.lock().map_err(|e| e.to_string())?.get(task) {
            return Ok(id.clone());
        }
        if Instant::now() > deadline {
            return Err(format!("task {task} has no agent yet"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The specialist running step `step` of `task`'s breakdown, waited for as a spawn is: the orchestrator starts it
/// some time after the breakdown is approved.
fn step_agent(
    client: &FleetClient,
    task: &str,
    task_id: &str,
    step: u32,
) -> Result<String, String> {
    let deadline = Instant::now() + AGENT_WAIT;
    loop {
        let view = client
            .fleet_view(task_id)
            .map_err(|e| format!("cannot read the breakdown of {task}: {e}"))?;
        // The latest version first: a revised breakdown may give the step to a new specialist.
        let agent = view.breakdowns.iter().rev().find_map(|b| {
            b.steps
                .iter()
                .find(|s| s.n == step)
                .and_then(|s| s.agent.clone())
        });
        if let Some(id) = agent {
            return Ok(id.0);
        }
        if Instant::now() > deadline {
            return Err(format!("task {task} step {step} has no specialist yet"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn append_cue(path: &PathBuf, line: &str) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    // One write per line, so the GUI never reads half a cue.
    file.write_all(format!("{line}\n").as_bytes())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let need = |flag: &str| flag_value(&args, flag).ok_or(format!("missing {flag} <value>"));
    let shot = PathBuf::from(need("--shot")?);
    let mut timeline = load_shot(&shot)?;
    // Taken as given: a shorter clip or another still is the caller's call, so the pacing is not checked again.
    if let Some(v) = flag_value(&args, "--length") {
        timeline.length = parse_duration(&v).map_err(|e| format!("--length: {e}"))?;
    }
    if let Some(v) = flag_value(&args, "--still-at") {
        timeline.still_at = parse_duration(&v).map_err(|e| format!("--still-at: {e}"))?;
    }
    if args.iter().any(|a| a == "--describe") {
        let name = shot
            .canonicalize()
            .map_err(|e| format!("cannot resolve {}: {e}", shot.display()))?
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or("the shot folder has no name")?;
        print!("{}", timeline.describe(&name));
        return Ok(());
    }
    let base_url = need("--base-url")?;
    let cues_path = PathBuf::from(need("--cues")?);
    let work = PathBuf::from(need("--work")?);

    let client = FleetClient::new(&base_url);
    // The recorder sets the world up before the GUI starts (`--setup`), so the GUI never opens its first-persona
    // wizard, then passes the persona's id back with `--persona`.
    let setup = args.iter().any(|a| a == "--setup");
    let persona_id = match flag_value(&args, "--persona") {
        Some(_) if setup => return Err("--setup and --persona cannot be used together".into()),
        Some(id) => id,
        None => {
            let id = create_persona(&client, &timeline.persona)?;
            if setup {
                println!("PERSONA {id}");
            }
            id
        }
    };

    // Before READY, so the clock starts with everything known. The product is made with the persona (before the
    // GUI starts) and named back with `--product`, so the GUI can be told to show it from its first frame.
    let (product, releases) = match flag_value(&args, "--product") {
        Some(id) => {
            let releases = find_releases(&client, &timeline, &id)?;
            (id, releases)
        }
        None => {
            let (id, releases) = create_product(&client, &timeline, &shot, &work)?;
            if setup {
                println!("PRODUCT {id}");
            }
            (id, releases)
        }
    };
    if setup {
        return Ok(());
    }
    let spawns = timeline
        .tasks
        .iter()
        .map(|p| {
            let release = match &p.release {
                None => None,
                Some(key) => Some(
                    *releases
                        .get(key)
                        .ok_or(format!("task {}: release {key} was not set up", p.key))?,
                ),
            };
            let req = SpawnRequest {
                actor: Actor("local".into()),
                product: product.clone(),
                release,
                repos: p
                    .repos
                    .iter()
                    .map(|name| RepoPick {
                        repo: name.clone(),
                        local: Some(work.join(name).to_string_lossy().into_owned()),
                    })
                    .collect(),
                prompt: p.task.clone(),
                slug: None,
                persona: persona_id.clone(),
                kind: p.kind.map_or(AgentKind::Solo, |_| AgentKind::Orchestrator),
                role: p.kind.map(|k| k.as_str().to_string()),
                roster: vec![],
            };
            Ok((p, req))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let agents: Agents = Arc::default();
    let t0 = Instant::now();
    let epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    println!("READY {epoch_ms}");
    std::io::stdout().flush().map_err(|e| e.to_string())?;

    // A spawn clones and fetches, so it runs beside the clock; cues must not slip behind a slow one.
    let spawn_error: Mutex<Option<String>> = Mutex::new(None);
    std::thread::scope(|scope| -> Result<(), String> {
        for (p, req) in &spawns {
            let (client, agents, spawn_error) = (&client, &agents, &spawn_error);
            scope.spawn(move || {
                sleep_until(t0 + p.at);
                match client.spawn(req) {
                    Ok(r) => {
                        if let Ok(mut a) = agents.lock() {
                            a.insert(p.key.clone(), r.agent.id.0);
                        }
                    }
                    Err(e) => {
                        if let Ok(mut slot) = spawn_error.lock() {
                            slot.get_or_insert(format!("cannot spawn {}: {e}", p.key));
                        }
                    }
                }
            });
        }
        let stage = Stage {
            client: &client,
            agents: &agents,
            spawn_error: &spawn_error,
            cues_path: &cues_path,
            product: &product,
            releases,
            personas: HashMap::new(),
        };
        stage.fire_cues(&timeline, t0)
    })?;
    if let Some(e) = spawn_error.into_inner().map_err(|e| e.to_string())? {
        return Err(e);
    }
    sleep_until(t0 + timeline.length);
    println!("DONE");
    Ok(())
}

/// Seeds every repo of the shot under `work`, saves its specialists and adds the repos and releases to a new
/// product. Returns the product's id and its releases' ids by key.
fn create_product(
    client: &FleetClient,
    timeline: &Timeline,
    shot: &Path,
    work: &Path,
) -> Result<(String, Releases), String> {
    let paths = timeline
        .repos
        .iter()
        .map(|r| seed_repo(shot, r, work))
        .collect::<Result<Vec<_>, _>>()?;
    // Before the rosters, which may name them.
    for spec in &timeline.specialists {
        let path = shot.join(&spec.file);
        let text = std::fs::read_to_string(&path).map_err(|e| {
            format!(
                "specialist {}: cannot read {}: {e}",
                spec.name,
                path.display()
            )
        })?;
        client
            .save_definition(DefKind::Specialist, &spec.name, &text)
            .map_err(|e| format!("cannot save specialist {}: {e}", spec.name))?;
    }
    let product = client
        .create_product(&NewProduct {
            name: timeline.product.clone(),
            default_persona: None,
        })
        .map_err(|e| format!("cannot create the product: {e}"))?;
    for (spec, path) in timeline.repos.iter().zip(paths) {
        client
            .upsert_repo(
                &product.id,
                &RepoUpsert {
                    name: spec.name.clone(),
                    path: Some(path.to_string_lossy().into_owned()),
                    url: None,
                },
            )
            .map_err(|e| format!("cannot add repo {} to the product: {e}", spec.name))?;
        if !spec.roster.is_empty() {
            client
                .set_repo_fleet(
                    &product.id,
                    &spec.name,
                    &RepoFleet {
                        roster: spec.roster.clone(),
                    },
                )
                .map_err(|e| format!("cannot set the roster of {}: {e}", spec.name))?;
        }
    }
    let today = local_today();
    let mut releases = Releases::new();
    for spec in &timeline.releases {
        releases.insert(
            spec.key.clone(),
            create_release(client, &product.id, spec, today)?,
        );
    }
    Ok((product.id, releases))
}

/// The ids of the shot's releases in a product set up by an earlier `--setup`, matched by version.
fn find_releases(
    client: &FleetClient,
    timeline: &Timeline,
    product: &str,
) -> Result<Releases, String> {
    if timeline.releases.is_empty() {
        return Ok(Releases::new());
    }
    let products = client
        .products()
        .map_err(|e| format!("cannot list the products: {e}"))?;
    let Some(found) = products.iter().find(|p| p.id == product) else {
        return Err(format!("no product {product}"));
    };
    timeline
        .releases
        .iter()
        .map(|spec| {
            found
                .releases
                .iter()
                .find(|r| r.version == spec.version)
                .map(|r| (spec.key.clone(), r.id))
                .ok_or(format!(
                    "release {} (version {}) is not in product {product}",
                    spec.key, spec.version
                ))
        })
        .collect()
}

/// Creates `spec` with its days resolved against `today`; returns its id.
fn create_release(
    client: &FleetClient,
    product: &str,
    spec: &ReleaseSpec,
    today: i64,
) -> Result<i64, String> {
    let day =
        |offset: &str| resolve_day(offset, today).map_err(|e| format!("release {}: {e}", spec.key));
    let release = client
        .create_release(
            product,
            &NewRelease {
                version: spec.version.clone(),
                start: day(&spec.start)?,
                target: day(&spec.target)?,
            },
        )
        .map_err(|e| format!("cannot create release {}: {e}", spec.key))?;
    Ok(release.id)
}

fn create_persona(client: &FleetClient, spec: &PersonaSpec) -> Result<String, String> {
    let persona = client
        .create_persona(&NewPersona {
            name: spec.name.clone(),
            host: "github.com".into(),
            commit_name: spec.commit_name.clone(),
            commit_email: spec.commit_email.clone(),
            github_auth: GithubAuth::Gh,
        })
        .map_err(|e| format!("cannot create persona {}: {e}", spec.name))?;
    let persona = client
        .check_persona(&persona.id)
        .map_err(|e| format!("cannot check persona {}: {e}", spec.name))?;
    if !persona.ready {
        return Err(format!("persona {} is not ready", persona.name));
    }
    Ok(persona.id)
}

/// What the cues act on and remember while they play.
struct Stage<'a> {
    client: &'a FleetClient,
    agents: &'a Agents,
    spawn_error: &'a Mutex<Option<String>>,
    cues_path: &'a PathBuf,
    product: &'a str,
    /// The shot's releases, then those the cues create.
    releases: Releases,
    /// Persona key to id, for the personas the cues create.
    personas: HashMap<String, String>,
}

impl Stage<'_> {
    fn fire_cues(mut self, timeline: &Timeline, t0: Instant) -> Result<(), String> {
        for cue in &timeline.cues {
            sleep_until(t0 + cue.at);
            self.fire(cue)?;
        }
        Ok(())
    }

    /// The agent a target names: the task's own, or the specialist of one of its steps.
    fn agent(&self, target: &Target) -> Result<String, String> {
        let id = agent_id(self.agents, self.spawn_error, &target.key)?;
        match target.step {
            None => Ok(id),
            Some(n) => step_agent(self.client, &target.key, &id, n),
        }
    }

    /// Fails the recording when the target is not where the shot expects it, before the cue acts on it.
    fn check_expect(&self, cue: &Cue, target: &Target, id: &str) -> Result<(), String> {
        let Some(want) = cue.expect else {
            return Ok(());
        };
        let snapshot = self.client.snapshot().map_err(|e| {
            format!(
                "cue {} ({}): cannot read the board: {e}",
                cue.n,
                cue.action.name()
            )
        })?;
        let found = snapshot
            .agents
            .iter()
            .find(|a| a.id.0 == id)
            .map_or("no agent", |a| status_word(a.status));
        if found != status_word(want) {
            return Err(format!(
                "cue {} ({}) expected {target} {}, found {found}",
                cue.n,
                cue.action.name(),
                status_word(want)
            ));
        }
        Ok(())
    }

    /// The breakdown version waiting for a decision.
    fn waiting_version(&self, task: &Target, id: &str) -> Result<u32, String> {
        let view = self
            .client
            .fleet_view(id)
            .map_err(|e| format!("cannot read the breakdown of {task}: {e}"))?;
        view.breakdowns
            .iter()
            .rev()
            .find(|b| b.approval == Approval::Proposed)
            .map(|b| b.version)
            .ok_or(format!(
                "task {task} has no breakdown waiting for a decision"
            ))
    }

    fn decide(&self, task: &Target, id: &str, comment: Option<&str>) -> Result<(), String> {
        let version = self.waiting_version(task, id)?;
        self.client
            .decide_breakdown(
                id,
                &DecideBreakdown {
                    approve: comment.is_none(),
                    comment: comment.map(String::from),
                    version: Some(version),
                },
            )
            .map_err(|e| format!("cannot decide the breakdown of {task}: {e}"))
    }

    fn release_id(&self, key: &str) -> Result<i64, String> {
        self.releases
            .get(key)
            .copied()
            .ok_or(format!("release {key} is not created yet"))
    }

    fn fire(&mut self, cue: &Cue) -> Result<(), String> {
        let id = match cue.action.target() {
            Some(target) => {
                let id = self.agent(target)?;
                self.check_expect(cue, target, &id)?;
                id
            }
            None => String::new(),
        };
        match &cue.action {
            Action::Answer { task, text } | Action::FollowUp { task, text } => self
                .client
                .answer(&id, text)
                .map_err(|e| format!("cannot answer {task}: {e}")),
            Action::Panel { tab, .. } => {
                append_cue(self.cues_path, &format!("open {id} {}", tab.as_str()))
            }
            Action::PanelClose => append_cue(self.cues_path, "close"),
            Action::Approve { task } => self.decide(task, &id, None),
            Action::RequestChanges { task, text } => self.decide(task, &id, Some(text)),
            Action::CreateRelease(spec) => {
                let release = create_release(self.client, self.product, spec, local_today())?;
                self.releases.insert(spec.key.clone(), release);
                Ok(())
            }
            Action::SetRelease { task, release } => {
                let release = release.as_deref().map(|k| self.release_id(k)).transpose()?;
                self.client
                    .set_task_release(&id, release)
                    .map(drop)
                    .map_err(|e| format!("cannot move {task} to another release: {e}"))
            }
            Action::Stop { task } => self
                .client
                .stop(&id)
                .map_err(|e| format!("cannot stop {task}: {e}")),
            Action::Archive { task } => {
                // The plan the daemon just made is the one confirmed, as the GUI's dialog would.
                let plan = self
                    .client
                    .archive_plan(&id)
                    .map_err(|e| format!("cannot plan the archive of {task}: {e}"))?;
                self.client
                    .archive(&id, &plan)
                    .map(drop)
                    .map_err(|e| format!("cannot archive {task}: {e}"))
            }
            Action::CreatePersona { key, persona } => {
                let persona = create_persona(self.client, persona)?;
                self.personas.insert(key.clone(), persona);
                Ok(())
            }
            Action::View { view, tab } => {
                let line = match tab {
                    Some(tab) => format!("view {view} {tab}"),
                    None => format!("view {view}"),
                };
                append_cue(self.cues_path, &line)
            }
            Action::Filter(filter) => {
                let line = match filter {
                    Filter::All => "filter all".to_string(),
                    Filter::Backlog => "filter backlog".to_string(),
                    Filter::Release(key) => format!("filter {}", self.release_id(key)?),
                };
                append_cue(self.cues_path, &line)
            }
            Action::Persona { key } => {
                let persona = self
                    .personas
                    .get(key)
                    .ok_or(format!("persona {key} is not created yet"))?;
                append_cue(self.cues_path, &format!("persona {persona}"))
            }
            // Only the quit is the GUI's; starting it again is the recorder's part.
            Action::GuiRestart => append_cue(self.cues_path, "quit"),
        }
    }
}
