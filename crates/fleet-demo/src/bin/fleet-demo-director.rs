//! Plays the demo timeline against a running daemon: spawns the projects, then answers and cues the GUI on schedule.
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fleet_api::{GithubAuth, NewPersona};
use fleet_client::http::FleetClient;
use fleet_demo::timeline::{parse_timeline, Action, Timeline};

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

fn sleep_until(deadline: Instant) {
    let now = Instant::now();
    if deadline > now {
        std::thread::sleep(deadline - now);
    }
}

fn agent_id(
    agents: &Agents,
    spawn_error: &Mutex<Option<String>>,
    project: &str,
) -> Result<String, String> {
    let deadline = Instant::now() + AGENT_WAIT;
    loop {
        // A failed spawn is the cause; say it instead of waiting out the clock.
        if let Some(e) = spawn_error.lock().map_err(|e| e.to_string())?.clone() {
            return Err(e);
        }
        if let Some(id) = agents.lock().map_err(|e| e.to_string())?.get(project) {
            return Ok(id.clone());
        }
        if Instant::now() > deadline {
            return Err(format!("project {project} has no agent yet"));
        }
        std::thread::sleep(Duration::from_millis(50));
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
    let base_url = need("--base-url")?;
    let timeline_path = need("--timeline")?;
    let cues_path = PathBuf::from(need("--cues")?);
    let repo = need("--repo")?;
    let text = std::fs::read_to_string(&timeline_path)
        .map_err(|e| format!("cannot read {timeline_path}: {e}"))?;
    let timeline = parse_timeline(&text)?;

    let client = FleetClient::new(&base_url);
    // The recorder sets the persona up before the GUI starts (`--persona-only`), so the GUI never opens its
    // first-persona wizard, then passes its id back with `--persona`.
    let persona_id = match flag_value(&args, "--persona") {
        Some(id) => id,
        None => {
            let id = create_persona(&client, &timeline)?;
            if args.iter().any(|a| a == "--persona-only") {
                println!("PERSONA {id}");
                return Ok(());
            }
            id
        }
    };

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
        for p in &timeline.projects {
            let (client, agents, persona_id, repo, spawn_error) =
                (&client, &agents, &persona_id, &repo, &spawn_error);
            scope.spawn(move || {
                sleep_until(t0 + p.at);
                match client.spawn(persona_id, std::slice::from_ref(repo), &p.task) {
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
        fire_cues(&timeline, &client, &agents, &spawn_error, &cues_path, t0)
    })?;
    if let Some(e) = spawn_error.into_inner().map_err(|e| e.to_string())? {
        return Err(e);
    }
    sleep_until(t0 + timeline.length);
    println!("DONE");
    Ok(())
}

fn create_persona(client: &FleetClient, timeline: &Timeline) -> Result<String, String> {
    let persona = client
        .create_persona(&NewPersona {
            name: timeline.persona.name.clone(),
            host: "github.com".into(),
            commit_name: timeline.persona.commit_name.clone(),
            commit_email: timeline.persona.commit_email.clone(),
            github_auth: GithubAuth::Gh,
        })
        .map_err(|e| format!("cannot create the persona: {e}"))?;
    let persona = client
        .check_persona(&persona.id)
        .map_err(|e| format!("cannot check the persona: {e}"))?;
    if !persona.ready {
        return Err(format!("persona {} is not ready", persona.name));
    }
    Ok(persona.id)
}

fn fire_cues(
    timeline: &Timeline,
    client: &FleetClient,
    agents: &Agents,
    spawn_error: &Mutex<Option<String>>,
    cues_path: &PathBuf,
    t0: Instant,
) -> Result<(), String> {
    for cue in &timeline.cues {
        sleep_until(t0 + cue.at);
        match &cue.action {
            Action::Answer { project, text } | Action::FollowUp { project, text } => {
                let id = agent_id(agents, spawn_error, project)?;
                client
                    .answer(&id, text)
                    .map_err(|e| format!("cannot answer {project}: {e}"))?;
            }
            Action::Panel { project, tab } => {
                let id = agent_id(agents, spawn_error, project)?;
                append_cue(cues_path, &format!("open {id} {}", tab.as_str()))?;
            }
            Action::PanelClose => append_cue(cues_path, "close")?,
        }
    }
    Ok(())
}
