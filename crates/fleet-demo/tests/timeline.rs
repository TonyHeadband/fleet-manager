use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use fleet_demo::scenario::{load_dir, parse, Scenario};
use fleet_demo::timeline::{check_pacing, load_shot, parse_timeline};
use serde_json::{json, Value};

fn pie_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../demo/shots/pie")
}

fn real_scenarios() -> Vec<(PathBuf, Scenario)> {
    load_dir(&pie_dir().join("scenarios")).unwrap()
}

/// The repo every shot needs; last, so it cannot swallow the top-level keys of the timeline before it.
const REPO: &str = r#"
[[repo]]
name = "bakery"
message = "Start the bakery"
"#;

/// A shot folder holding `toml` (a pie timeline) and a seed for the bakery.
fn write_shot(dir: &Path, toml: &str) {
    std::fs::create_dir_all(dir.join("seed/bakery")).unwrap();
    std::fs::write(
        dir.join("seed/bakery/README.txt"),
        "Recipes for the bakery.\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("shot.toml"),
        format!("product = \"pie\"\n{toml}{REPO}"),
    )
    .unwrap();
}

const HEAD: &str = r#"
product = "pie"
persona = { name = "Pastry chef", commit_name = "Pastry Chef", commit_email = "chef@pie.example" }
length = "24s"
still_at = "13s"

[[repo]]
name = "bakery"
message = "Start the bakery"

[[task]]
key = "crust"
task = "Make an all-butter crust"
at = "0s"

[[cue]]
at = "15s"
panel = { task = "crust", tab = "terminal" }

[[cue]]
at = "17s"
panel = { task = "crust", tab = "diff" }

[[cue]]
at = "19s"
panel_close = true
"#;

fn with(extra: &str) -> String {
    format!("{HEAD}{extra}")
}

#[test]
fn the_demo_timeline_passes_pacing() {
    let t = load_shot(&pie_dir()).unwrap();
    assert_eq!(check_pacing(&t, &real_scenarios()), Vec::<String>::new());
}

#[test]
fn pacing_flags_cues_too_close() {
    let t = parse_timeline(&with(
        r#"
[[cue]]
at = "5.5s"
answer = { task = "crust", text = "1" }

[[cue]]
at = "6.5s"
follow_up = { task = "crust", text = "again" }
"#,
    ))
    .unwrap();
    let v = check_pacing(&t, &real_scenarios());
    assert!(
        v.iter().any(|m| m.contains("less than 1.5 s apart")),
        "{v:?}"
    );
}

#[test]
fn pacing_flags_an_answer_before_two_seconds_of_waiting() {
    // The crust asks for permission at 3 s (1 s of startup included), so an answer at 4 s leaves 1 s to read the reason.
    let t = parse_timeline(&with(
        r#"
[[cue]]
at = "4s"
answer = { task = "crust", text = "1" }
"#,
    ))
    .unwrap();
    let v = check_pacing(&t, &real_scenarios());
    assert!(
        v.iter().any(|m| m.contains("crust") && m.contains("2 s")),
        "{v:?}"
    );
}

#[test]
fn pacing_flags_a_task_without_scenario() {
    let t = parse_timeline(&with("").replace("Make an all-butter crust", "Mow the lawn")).unwrap();
    let v = check_pacing(&t, &real_scenarios());
    assert!(v.iter().any(|m| m.contains("Mow the lawn")), "{v:?}");
}

#[test]
fn pacing_flags_a_spawn_right_after_another_card_needs_you() {
    // The crust needs you at 3 s; a second card appearing at 4 s lands on it.
    let t = parse_timeline(&with(
        r#"
[[task]]
key = "filling"
task = "Prepare the blueberry filling"
at = "4s"
"#,
    ))
    .unwrap();
    let exact = "board changes at 3.0 s (crust needs you) and 4.0 s (filling appears) are less than 1.5 s apart";
    let v = check_pacing(&t, &real_scenarios());
    assert!(v.iter().any(|m| m == exact), "{v:?}");
}

#[test]
fn pacing_flags_a_terminal_tab_on_a_card_that_is_not_working() {
    let t = parse_timeline(&with("").replace("15s", "3.5s").replace("17s", "5.2s")).unwrap();
    let v = check_pacing(&t, &real_scenarios());
    assert!(
        v.iter()
            .any(|m| m.contains("Terminal") && m.contains("not Working")),
        "{v:?}"
    );
}

#[test]
fn pacing_flags_a_diff_before_the_recipe_is_written() {
    // The crust waits from 3 s, is answered at 5 s and writes its recipe 0.8 s later, at 5.8 s.
    let t = parse_timeline(
        &with(
            r#"
[[cue]]
at = "5s"
answer = { task = "crust", text = "1" }
"#,
        )
        .replace("15s", "3.5s")
        .replace("17s", "5.5s"),
    )
    .unwrap();
    let v = check_pacing(&t, &real_scenarios());
    let exact = "the Diff tab opens on crust at 5.5 s, before its last recipe write (5.8 s)";
    assert!(v.iter().any(|m| m == exact), "{v:?}");
}

#[test]
fn pacing_flags_a_diff_before_the_follow_up_rewrites_the_recipe() {
    // The bake card writes once in its main arc and again, the lattice, in its follow-up.
    let text = std::fs::read_to_string(pie_dir().join("shot.toml")).unwrap();
    let t = parse_timeline(&text.replace("at = \"22.9s\"", "at = \"21.6s\"")).unwrap();
    let v = check_pacing(&t, &real_scenarios());
    assert!(
        v.iter()
            .any(|m| m.contains("Diff") && m.contains("before its last recipe write")),
        "{v:?}"
    );
}

#[test]
fn parsing_rejects_a_still_after_the_end() {
    let e = parse_timeline(&with("").replace("13s", "30s")).unwrap_err();
    assert!(e.contains("still_at"), "{e}");
}

#[test]
fn parsing_rejects_duplicate_task_keys() {
    let e = parse_timeline(&with(
        "\n[[task]]\nkey = \"crust\"\ntask = \"x\"\nat = \"1s\"\n",
    ))
    .unwrap_err();
    assert!(e.contains("duplicate"), "{e}");
}

#[test]
fn pacing_flags_a_length_outside_the_window() {
    let t = parse_timeline(&with("").replace("24s", "40s")).unwrap();
    let v = check_pacing(&t, &real_scenarios());
    assert!(v.iter().any(|m| m.contains("length")), "{v:?}");
}

#[test]
fn a_scenario_with_one_wait_is_measured_from_its_steps() {
    let s = parse(
        "task = \"t\"\n[[step]]\nafter = \"1s\"\nsay = \"x\"\n[[step]]\nafter = \"1s\"\nquestion = \"q?\"\n",
        Path::new("t.toml"),
    )
    .unwrap();
    let t = parse_timeline(&HEAD.replace("Make an all-butter crust", "t").replace("[[cue]]\nat = \"15s\"", "[[cue]]\nat = \"3s\"\nanswer = { task = \"crust\", text = \"x\" }\n\n[[cue]]\nat = \"15s\""))
        .unwrap();
    let v = check_pacing(&t, &[(PathBuf::from("t.toml"), s)]);
    // Waits from 3 s (1 s of startup), answered at 3 s.
    let exact =
        "task crust: answered at 3.0 s, only 0.0 s after its wait begins (3.0 s); needs 2 s";
    assert!(v.iter().any(|m| m == exact), "{v:?}");
}

fn read_request(stream: &mut std::net::TcpStream) -> (String, String) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    let mut length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = v.trim().parse().unwrap();
        }
        if line == "\r\n" {
            break;
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let request_line = first.trim().to_string();
    (request_line, String::from_utf8(body).unwrap())
}

fn persona_json() -> Value {
    json!({
        "id": "pastry-chef", "name": "Pastry chef", "host": "github.com", "commit_name": "Pastry Chef",
        "commit_email": "chef@pie.example", "github_auth": "gh", "claude_dir": "/c", "gh_dir": "/g",
        "ready": true, "claude": null, "github": null, "tasks": 0
    })
}

fn agent_json(id: &str, task: &str) -> Value {
    json!({
        "id": id, "goal": task, "workdir": "/p/t", "workspaces": [], "tmux_session": "fleet-x",
        "status": "queued", "activity": "", "status_since": 1, "has_changes": false, "paused": false,
        "owner": "o", "blocked_reason": null
    })
}

type Seen = Arc<Mutex<Vec<(String, String, u128)>>>;

/// A daemon that records each request with its arrival time (epoch ms); `fail_spawn` makes spawns answer 500.
fn stub_daemon(fail_spawn: bool) -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let (line, body) = read_request(&mut stream);
            let arrived = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis();
            let mut status = "200 OK";
            let reply = if line.starts_with("POST /api/personas/pastry-chef/check")
                || line.starts_with("POST /api/personas ")
            {
                persona_json()
            } else if line.starts_with("POST /api/products ") {
                json!({"id": "pie", "name": "pie", "default_persona": null,
                       "repos": [], "releases": [], "tasks": 0})
            } else if line.starts_with("PUT /api/products/pie/repos ") {
                json!({"product": "pie", "name": "bakery", "path": "/work/bakery", "url": null,
                       "roster": [], "workflow": {"state": "missing", "base": "main"}})
            } else if line.starts_with("PUT /api/products/pie/repos/bakery/fleet ") {
                json!({"product": "pie", "name": "bakery", "path": "/work/bakery", "url": null,
                       "roster": ["baker"], "workflow": {"state": "missing", "base": "main"}})
            } else if line.starts_with("POST /api/agents ") && fail_spawn {
                status = "500 Internal Server Error";
                json!({ "error": "no room for another pie" })
            } else if line.starts_with("POST /api/agents ") {
                let v: Value = serde_json::from_str(&body).unwrap();
                let task = v["prompt"].as_str().unwrap();
                let id = if task.contains("crust") {
                    "crust-1"
                } else {
                    "bake-1"
                };
                json!({ "agent": agent_json(id, task), "warnings": [] })
            } else {
                json!({})
            };
            log.lock().unwrap().push((line, body, arrived));
            let text = reply.to_string();
            write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                text.len()
            )
            .unwrap();
        }
    });
    (base, seen)
}

#[test]
fn director_reports_the_spawn_error_when_a_cue_needs_the_agent() {
    let (base, _seen) = stub_daemon(true);
    let dir = tempfile::tempdir().unwrap();
    write_shot(
        dir.path(),
        r#"persona = { name = "Pastry chef", commit_name = "Pastry Chef", commit_email = "chef@pie.example" }
length = "2s"
still_at = "1s"

[[task]]
key = "crust"
task = "Make an all-butter crust"
at = "0s"

[[cue]]
at = "0.5s"
answer = { task = "crust", text = "1" }
"#,
    );
    let work = dir.path().join("work");
    let out = Command::new(env!("CARGO_BIN_EXE_fleet-demo-director"))
        .args(["--base-url", &base, "--shot"])
        .arg(dir.path())
        .arg("--cues")
        .arg(dir.path().join("cues"))
        .arg("--work")
        .arg(&work)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("cannot spawn crust: no room for another pie"),
        "{err}"
    );
}

#[test]
fn director_drives_a_stub_daemon() {
    let (base, seen) = stub_daemon(false);
    let dir = tempfile::tempdir().unwrap();
    write_shot(
        dir.path(),
        r#"persona = { name = "Pastry chef", commit_name = "Pastry Chef", commit_email = "chef@pie.example" }
length = "2.2s"
still_at = "1s"

[[task]]
key = "crust"
task = "Make an all-butter crust"
at = "0s"

[[task]]
key = "bake"
task = "Bake and plate the pie"
at = "0.3s"

[[cue]]
at = "0.8s"
answer = { task = "crust", text = "1" }

[[cue]]
at = "1.0s"
follow_up = { task = "bake", text = "now make it a lattice top" }

[[cue]]
at = "1.2s"
panel = { task = "crust", tab = "terminal" }

[[cue]]
at = "1.5s"
panel = { task = "crust", tab = "diff" }

[[cue]]
at = "1.8s"
panel_close = true
"#,
    );
    let work = dir.path().join("work");
    let cues = dir.path().join("cues");

    let out = Command::new(env!("CARGO_BIN_EXE_fleet-demo-director"))
        .args(["--base-url", &base, "--shot"])
        .arg(dir.path())
        .arg("--cues")
        .arg(&cues)
        .arg("--work")
        .arg(&work)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines[0].starts_with("READY "), "{lines:?}");
    let ready_ms: u128 = lines[0]["READY ".len()..].parse().unwrap();
    let first_spawn_ms = seen
        .lock()
        .unwrap()
        .iter()
        .find(|(line, _, _)| line.starts_with("POST /api/agents "))
        .map(|(_, _, at)| *at)
        .unwrap();
    assert!(
        ready_ms <= first_spawn_ms,
        "READY {ready_ms} after the first spawn {first_spawn_ms}"
    );
    assert_eq!(lines.last(), Some(&"DONE"));

    let requests: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|(line, _, _)| line.trim_end_matches(" HTTP/1.1").to_string())
        .collect();
    assert_eq!(
        requests,
        [
            "POST /api/personas",
            "POST /api/personas/pastry-chef/check",
            "POST /api/products",
            "PUT /api/products/pie/repos",
            "POST /api/agents",
            "POST /api/agents",
            "POST /api/agents/crust-1/answer",
            "POST /api/agents/bake-1/answer",
        ]
    );
    let bodies: Vec<Value> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|(_, b, _)| serde_json::from_str(b).unwrap_or(Value::Null))
        .collect();
    assert_eq!(bodies[0]["github_auth"], "gh");
    assert_eq!(bodies[0]["host"], "github.com");
    assert_eq!(bodies[2]["name"], "pie");
    assert_eq!(bodies[3]["name"], "bakery");
    assert_eq!(bodies[3]["path"], work.join("bakery").to_str().unwrap());
    assert_eq!(bodies[4]["persona"], "pastry-chef");
    assert_eq!(bodies[4]["product"], "pie");
    assert_eq!(
        bodies[4]["repos"],
        json!([{"repo": "bakery", "local": work.join("bakery").to_str().unwrap()}])
    );
    assert_eq!(bodies[4]["prompt"], "Make an all-butter crust");
    assert_eq!(bodies[6]["text"], "1");
    assert_eq!(bodies[7]["text"], "now make it a lattice top");

    let cue_text = std::fs::read_to_string(&cues).unwrap();
    assert_eq!(
        cue_text.lines().collect::<Vec<_>>(),
        ["open crust-1 terminal", "open crust-1 diff", "close"]
    );
}

const ONE_TASK: &str = r#"
persona = { name = "Pastry chef", commit_name = "Pastry Chef", commit_email = "chef@pie.example" }
length = "0.5s"
still_at = "0.2s"

[[task]]
key = "crust"
task = "Make an all-butter crust"
at = "0s"
"#;

fn run_director(base: &str, dir: &Path, extra: &[&str]) -> std::process::Output {
    write_shot(dir, ONE_TASK);
    Command::new(env!("CARGO_BIN_EXE_fleet-demo-director"))
        .args(["--base-url", base, "--shot"])
        .arg(dir)
        .arg("--cues")
        .arg(dir.join("cues"))
        .arg("--work")
        .arg(dir.join("work"))
        .args(extra)
        .output()
        .unwrap()
}

fn request_lines(seen: &Seen) -> Vec<String> {
    seen.lock()
        .unwrap()
        .iter()
        .map(|(line, _, _)| line.trim_end_matches(" HTTP/1.1").to_string())
        .collect()
}

#[test]
fn director_setup_seeds_the_repo_and_sets_up_the_product_and_stops() {
    let (base, seen) = stub_daemon(false);
    let dir = tempfile::tempdir().unwrap();
    let out = run_director(&base, dir.path(), &["--setup"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "PERSONA pastry-chef\nPRODUCT pie\n"
    );
    assert_eq!(
        request_lines(&seen),
        [
            "POST /api/personas",
            "POST /api/personas/pastry-chef/check",
            "POST /api/products",
            "PUT /api/products/pie/repos"
        ]
    );
}

#[test]
fn director_with_a_persona_id_spawns_as_it_without_creating_one() {
    let (base, seen) = stub_daemon(false);
    let dir = tempfile::tempdir().unwrap();
    let out = run_director(
        &base,
        dir.path(),
        &["--persona", "pastry-chef", "--product", "pie"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(request_lines(&seen), ["POST /api/agents"]);
    let body: Value = serde_json::from_str(&seen.lock().unwrap()[0].1).unwrap();
    assert_eq!(body["persona"], "pastry-chef");
}

#[test]
fn director_refuses_setup_with_a_persona_id() {
    let (base, seen) = stub_daemon(false);
    let dir = tempfile::tempdir().unwrap();
    let out = run_director(&base, dir.path(), &["--setup", "--persona", "pastry-chef"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--setup and --persona"), "{err}");
    assert!(request_lines(&seen).is_empty());
}

#[test]
fn shot_without_a_repo_is_refused() {
    let e = parse_timeline(
        r#"
product = "pie"
persona = { name = "Pastry chef", commit_name = "Pastry Chef", commit_email = "chef@pie.example" }
length = "24s"
still_at = "13s"
"#,
    )
    .unwrap_err();
    assert!(e.contains("at least one [[repo]]"), "{e}");
}

#[test]
fn a_repo_defaults_to_the_baker_without_an_origin_or_roster() {
    let t = parse_timeline(&with("")).unwrap();
    let r = &t.repos[0];
    assert_eq!(t.product, "pie");
    assert_eq!(
        r.author,
        ("Baker".to_string(), "baker@pie.example".to_string())
    );
    assert!(!r.origin);
    assert!(r.roster.is_empty());
}

#[test]
fn director_setup_sets_the_roster_of_a_repo_that_has_one() {
    let (base, seen) = stub_daemon(false);
    let dir = tempfile::tempdir().unwrap();
    write_shot(dir.path(), ONE_TASK);
    let shot = std::fs::read_to_string(dir.path().join("shot.toml")).unwrap();
    std::fs::write(
        dir.path().join("shot.toml"),
        shot.replace(
            "message = \"Start the bakery\"",
            "message = \"Start the bakery\"\nroster = [\"baker\"]",
        ),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_fleet-demo-director"))
        .args(["--base-url", &base, "--shot"])
        .arg(dir.path())
        .arg("--cues")
        .arg(dir.path().join("cues"))
        .arg("--work")
        .arg(dir.path().join("work"))
        .arg("--setup")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        request_lines(&seen).last().map(String::as_str),
        Some("PUT /api/products/pie/repos/bakery/fleet")
    );
    let body: Value = serde_json::from_str(&seen.lock().unwrap().last().unwrap().1).unwrap();
    assert_eq!(body["roster"], json!(["baker"]));
}
