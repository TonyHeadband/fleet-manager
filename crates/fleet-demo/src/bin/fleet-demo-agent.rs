//! Stands in for the `claude` CLI in the demo: same argv, same hooks, scripted behaviour.
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fleet_demo::hooks::{target_from_settings, HttpSink};
use fleet_demo::input::spawn_stdin_reader;
use fleet_demo::player::Player;
use fleet_demo::scenario::{find, load_dir};

/// Same value as `fleet_daemon::agents::PROMPT_FILE`; copied so the binary does not link the daemon.
const PROMPT_FILE: &str = "PROMPT.md";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // tmux closes the pane on exit, so stderr alone would vanish; show it on the pane and linger.
            eprintln!("fleet-demo-agent: {e}");
            println!("fleet-demo-agent: {e}");
            std::thread::sleep(Duration::from_secs(3));
            ExitCode::FAILURE
        }
    }
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.windows(2).any(|w| w[0] == "auth" && w[1] == "status") {
        println!(r#"{{"loggedIn":true,"email":"baker@pie.example"}}"#);
        return Ok(());
    }
    let shot = flag_value(&args, "--shot").ok_or("missing --shot <dir>")?;
    let settings = flag_value(&args, "--settings").ok_or("missing --settings <file>")?;
    let token = std::env::var("FLEET_TOKEN").map_err(|_| "FLEET_TOKEN is not set")?;
    let cwd = std::env::current_dir().map_err(|e| format!("no working directory: {e}"))?;
    let prompt = std::fs::read_to_string(cwd.join(PROMPT_FILE))
        .map_err(|e| format!("cannot read {PROMPT_FILE}: {e}"))?;
    let shot = PathBuf::from(shot);
    let scenarios = shot.join("scenarios");
    let all = load_dir(&scenarios).map_err(|e| e.to_string())?;
    let scenario = find(&all, &prompt)
        .ok_or_else(|| {
            format!(
                "no scenario in {} matches the prompt {:?}",
                scenarios.display(),
                prompt.trim()
            )
        })?
        .clone();
    let target = target_from_settings(&PathBuf::from(settings), token)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let session_id = format!("demo-{}-{nanos}", std::process::id());
    let input = spawn_stdin_reader();
    let mut player = Player::new(
        scenario,
        HttpSink::new(target),
        std::io::stdout(),
        cwd,
        session_id,
        shot,
        std::thread::sleep as fn(Duration),
    );
    player.run(&input)
}
