use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use fleet_demo::scenario::{load_dir, parse, Scenario};
use fleet_demo::timeline::{check_pacing, load_shot, local_today, parse_timeline, resolve_day};
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

/// The pie's two opt-in rules, as a shot asks for them.
const PACING: &str = r#"
[pacing]
panel_sequence = true
still = { working = 1, needs_you = 1, review = 1 }
"#;

#[test]
fn every_official_shot_passes_pacing() {
    let shots = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../demo/shots");
    let mut seen = 0;
    for entry in std::fs::read_dir(shots).unwrap() {
        let dir = entry.unwrap().path();
        if !dir.join("shot.toml").is_file() {
            continue;
        }
        let t = load_shot(&dir).unwrap();
        let scenarios = load_dir(&dir.join("scenarios")).unwrap();
        assert_eq!(
            check_pacing(&t, &scenarios),
            Vec::<String>::new(),
            "{}",
            dir.display()
        );
        seen += 1;
    }
    assert!(seen > 0, "no shots found");
}

/// The crust alone, answered at 5 s: no panel cues, so only length and spacing apply.
fn short(length: &str, still_at: &str) -> String {
    let head = HEAD.split("[[cue]]").next().unwrap();
    format!(
        "{}\n[[cue]]\nat = \"5s\"\nanswer = {{ task = \"crust\", text = \"1\" }}\n",
        head.replace("24s", length).replace("13s", still_at)
    )
}

#[test]
fn a_short_without_panel_rules_passes_at_15s() {
    let t = parse_timeline(&short("15s", "12s")).unwrap();
    assert_eq!(check_pacing(&t, &real_scenarios()), Vec::<String>::new());
}

#[test]
fn pacing_flags_a_length_outside_15_to_30() {
    for (len, bad) in [("14s", true), ("15s", false), ("30s", false), ("31s", true)] {
        let t = parse_timeline(&short(len, "12s")).unwrap();
        let v = check_pacing(&t, &real_scenarios());
        assert_eq!(v.iter().any(|m| m.contains("15-30 s")), bad, "{len}: {v:?}");
    }
}

#[test]
fn panel_rules_apply_only_when_asked() {
    // No close cue: wrong for the pie's choreography, fine for a shot that did not ask.
    let broken = format!("{}\n", short("24s", "13s")).replace(
        "[[cue]]\nat = \"5s\"",
        "[[cue]]\nat = \"10s\"\npanel = { task = \"crust\", tab = \"terminal\" }\n\n[[cue]]\nat = \"5s\"",
    );
    let t = parse_timeline(&broken).unwrap();
    assert_eq!(check_pacing(&t, &real_scenarios()), Vec::<String>::new());
    let t = parse_timeline(&format!("{broken}\n[pacing]\npanel_sequence = true\n")).unwrap();
    let v = check_pacing(&t, &real_scenarios());
    assert!(v.iter().any(|m| m.contains("side panel")), "{v:?}");
}

#[test]
fn still_counts_only_when_asked() {
    // At 13 s the lone crust card is in Review, not one of each.
    let text = short("24s", "13s");
    let t = parse_timeline(&text).unwrap();
    assert_eq!(check_pacing(&t, &real_scenarios()), Vec::<String>::new());
    let t = parse_timeline(&format!(
        "{text}\n[pacing]\nstill = {{ working = 1, needs_you = 1, review = 1 }}\n"
    ))
    .unwrap();
    let v = check_pacing(&t, &real_scenarios());
    assert!(v.iter().any(|m| m.contains("still_at")), "{v:?}");
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
    let t = parse_timeline(&with(PACING).replace("15s", "3.5s").replace("17s", "5.2s")).unwrap();
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

[pacing]
panel_sequence = true
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

fn persona_json(id: &str, name: &str) -> Value {
    json!({
        "id": id, "name": name, "host": "github.com", "commit_name": "Pastry Chef",
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
    stub_daemon_with(fail_spawn, |_, _| None)
}

/// As `stub_daemon`, with `extra` answering first: it gets the request line and the body, and `None` falls back to
/// the stock replies.
fn stub_daemon_with(
    fail_spawn: bool,
    extra: impl Fn(&str, &Value) -> Option<Value> + Send + 'static,
) -> (String, Seen) {
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
            let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let reply = if let Some(v) = extra(&line, &parsed) {
                v
            } else if let Some(id) = line
                .strip_prefix("POST /api/personas/")
                .and_then(|r| r.split_once("/check"))
                .map(|(id, _)| id)
            {
                persona_json(id, "Pastry chef")
            } else if line.starts_with("POST /api/personas ") {
                let name = parsed["name"].as_str().unwrap();
                persona_json(&name.to_lowercase().replace(' ', "-"), name)
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
                } else if task.contains("croissant") {
                    "croissant-1"
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

#[test]
fn describe_prints_resolved_output() {
    let out = Command::new(env!("CARGO_BIN_EXE_fleet-demo-director"))
        .arg("--shot")
        .arg(pie_dir())
        .arg("--describe")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    for line in [
        "NAME pie",
        "OUT_DIR docs/assets",
        "STEM demo",
        "STILL board",
        "FORMATS webm gif png",
        "SIZE 1400x800",
        "LENGTH 25.000",
        "STILL_AT 15.000",
    ] {
        assert!(
            text.lines().any(|l| l == line),
            "missing {line:?} in {text}"
        );
    }
}

const PERSONA: &str = r#"persona = { name = "Pastry chef", commit_name = "Pastry Chef", commit_email = "chef@pie.example" }
"#;

/// Runs the director on a shot made of `body` (everything after `product` and the persona), with `extra` flags.
fn direct(base: &str, dir: &Path, body: &str, extra: &[&str]) -> std::process::Output {
    write_shot(dir, &format!("{PERSONA}{body}"));
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

fn succeeded(out: &std::process::Output) {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn cue_lines(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("cues"))
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

/// The body of the first request whose line starts with `prefix`.
fn body_of(seen: &Seen, prefix: &str) -> Value {
    let seen = seen.lock().unwrap();
    let (_, body, _) = seen
        .iter()
        .find(|(line, _, _)| line.starts_with(prefix))
        .unwrap_or_else(|| panic!("no request {prefix}"));
    serde_json::from_str(body).unwrap_or(Value::Null)
}

const CROISSANT: &str = r#"length = "1.5s"
still_at = "0.2s"

[[task]]
key = "croissant"
task = "Laminate the croissant dough"
kind = "direct"
at = "0s"
"#;

fn breakdown(version: u32, approval: &str, step_agent: Option<&str>) -> Value {
    json!({
        "version": version, "approval": approval, "comment": null, "proposed_at": 1, "decided_at": null,
        "steps": [{"n": 1, "title": "Fold the butter", "specialist": "laminator", "brief": "b",
                   "state": if step_agent.is_some() { "running" } else { "pending" }, "agent": step_agent}]
    })
}

fn fleet_view(breakdowns: Vec<Value>) -> Value {
    json!({ "task": "croissant-1", "breakdowns": breakdowns, "gate_runs": [] })
}

fn snapshot(id: &str, status: &str) -> Value {
    let mut agent = agent_json(id, "t");
    agent["status"] = json!(status);
    json!({ "seq": 1, "agents": [agent] })
}

fn task_json(id: &str, release: Option<i64>) -> Value {
    json!({
        "id": id, "product": "pie", "release": release, "name": id, "goal": "g", "workdir": "/w", "repos": [],
        "state": "running", "agents_used": 1, "times": {"queued": 0, "working": 0, "needs_you": 0, "review": 0},
        "log": []
    })
}

#[test]
fn director_spawns_an_orchestrated_task_with_its_role() {
    let (base, seen) = stub_daemon(false);
    let dir = tempfile::tempdir().unwrap();
    succeeded(&direct(&base, dir.path(), CROISSANT, &[]));
    let body = body_of(&seen, "POST /api/agents ");
    assert_eq!(body["kind"], "orchestrator");
    assert_eq!(body["role"], "direct");
    assert_eq!(body["release"], Value::Null);
}

#[test]
fn director_spawns_in_the_repos_a_task_names() {
    let (base, seen) = stub_daemon(false);
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("seed/pantry")).unwrap();
    std::fs::write(dir.path().join("seed/pantry/flour.txt"), "T45\n").unwrap();
    let body = CROISSANT.replace(
        "kind = \"direct\"",
        "kind = \"direct\"\nrepos = [\"pantry\", \"bakery\"]",
    ) + "\n[[repo]]\nname = \"pantry\"\nmessage = \"Stock the pantry\"\n";
    succeeded(&direct(&base, dir.path(), &body, &[]));
    let work = dir.path().join("work");
    assert_eq!(
        body_of(&seen, "POST /api/agents ")["repos"],
        json!([
            {"repo": "pantry", "local": work.join("pantry").to_str().unwrap()},
            {"repo": "bakery", "local": work.join("bakery").to_str().unwrap()},
        ])
    );
}

#[test]
fn director_targets_a_step_by_key_and_number() {
    let (base, _seen) = stub_daemon_with(false, |line, _| {
        line.starts_with("GET /api/tasks/croissant-1/fleet ")
            .then(|| fleet_view(vec![breakdown(1, "approved", Some("a-2"))]))
    });
    let dir = tempfile::tempdir().unwrap();
    let body = format!(
        "{CROISSANT}\n[[cue]]\nat = \"0.3s\"\npanel = {{ task = \"croissant#1\", tab = \"terminal\" }}\n\n[[cue]]\nat = \"0.5s\"\npanel = {{ task = \"croissant\", tab = \"breakdown\" }}\n"
    );
    succeeded(&direct(&base, dir.path(), &body, &[]));
    assert_eq!(
        cue_lines(dir.path()),
        ["open a-2 terminal", "open croissant-1 breakdown"]
    );
}

#[test]
fn director_names_a_step_that_never_spawns() {
    let (base, _seen) = stub_daemon_with(false, |line, _| {
        line.starts_with("GET /api/tasks/croissant-1/fleet ")
            .then(|| fleet_view(vec![breakdown(1, "proposed", None)]))
    });
    let dir = tempfile::tempdir().unwrap();
    let body = format!(
        "{CROISSANT}\n[[cue]]\nat = \"0.3s\"\npanel = {{ task = \"croissant#1\", tab = \"terminal\" }}\n"
    );
    let out = direct(&base, dir.path(), &body, &[]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("task croissant step 1 has no specialist yet"),
        "{err}"
    );
}

#[test]
fn director_fails_on_an_unmet_expect() {
    let (base, seen) = stub_daemon_with(false, |line, _| {
        line.starts_with("GET /api/agents ")
            .then(|| snapshot("croissant-1", "working"))
    });
    let dir = tempfile::tempdir().unwrap();
    let body = format!(
        "{CROISSANT}\n[[cue]]\nat = \"0.3s\"\nexpect = \"needs_you\"\napprove = {{ task = \"croissant\" }}\n"
    );
    let out = direct(&base, dir.path(), &body, &[]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("cue 1 (approve) expected croissant needs_you, found working"),
        "{err}"
    );
    // The check comes before the action, so nothing was decided.
    assert!(!request_lines(&seen)
        .iter()
        .any(|l| l.contains("/breakdown/decide")));
}

#[test]
fn director_fires_a_cue_whose_expect_holds() {
    let (base, _seen) = stub_daemon_with(false, |line, _| {
        line.starts_with("GET /api/agents ")
            .then(|| snapshot("croissant-1", "needs_you"))
    });
    let dir = tempfile::tempdir().unwrap();
    let body = format!(
        "{CROISSANT}\n[[cue]]\nat = \"0.3s\"\nexpect = \"needs_you\"\npanel = {{ task = \"croissant\", tab = \"breakdown\" }}\n"
    );
    succeeded(&direct(&base, dir.path(), &body, &[]));
    assert_eq!(cue_lines(dir.path()), ["open croissant-1 breakdown"]);
}

#[test]
fn director_approves_the_waiting_version() {
    let (base, seen) = stub_daemon_with(false, |line, _| {
        line.starts_with("GET /api/tasks/croissant-1/fleet ")
            .then(|| {
                fleet_view(vec![
                    breakdown(1, "changes_requested", None),
                    breakdown(2, "proposed", None),
                ])
            })
    });
    let dir = tempfile::tempdir().unwrap();
    let body = format!(
        "{CROISSANT}\n[[cue]]\nat = \"0.3s\"\napprove = {{ task = \"croissant\" }}\n\n[[cue]]\nat = \"0.6s\"\nrequest_changes = {{ task = \"croissant\", text = \"more butter\" }}\n"
    );
    succeeded(&direct(&base, dir.path(), &body, &[]));
    let seen = seen.lock().unwrap();
    let decisions: Vec<Value> = seen
        .iter()
        .filter(|(line, _, _)| line.starts_with("POST /api/tasks/croissant-1/breakdown/decide "))
        .map(|(_, b, _)| serde_json::from_str(b).unwrap())
        .collect();
    assert_eq!(
        decisions,
        [
            json!({"approve": true, "comment": null, "version": 2}),
            json!({"approve": false, "comment": "more butter", "version": 2}),
        ]
    );
}

#[test]
fn director_refuses_to_approve_when_nothing_waits() {
    let (base, _seen) = stub_daemon_with(false, |line, _| {
        line.starts_with("GET /api/tasks/croissant-1/fleet ")
            .then(|| fleet_view(vec![breakdown(1, "approved", None)]))
    });
    let dir = tempfile::tempdir().unwrap();
    let body =
        format!("{CROISSANT}\n[[cue]]\nat = \"0.3s\"\napprove = {{ task = \"croissant\" }}\n");
    let out = direct(&base, dir.path(), &body, &[]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("task croissant has no breakdown waiting"),
        "{err}"
    );
}

fn release_json(id: i64, version: &str, start: &str, target: &str) -> Value {
    json!({"id": id, "product": "pie", "version": version, "start": start, "target": target, "released_at": null})
}

/// Releases get ids 7, 8, … in the order they are created; tasks move freely.
fn release_stub() -> (String, Seen) {
    let next = Mutex::new(7);
    stub_daemon_with(false, move |line, body| {
        if line.starts_with("POST /api/products/pie/releases ") {
            let mut id = next.lock().unwrap();
            *id += 1;
            Some(release_json(
                *id - 1,
                body["version"].as_str().unwrap(),
                body["start"].as_str().unwrap(),
                body["target"].as_str().unwrap(),
            ))
        } else if let Some(rest) = line.strip_prefix("PUT /api/tasks/") {
            let id = rest.split('/').next().unwrap();
            Some(task_json(id, body["release"].as_i64()))
        } else {
            None
        }
    })
}

const RELEASES: &str = r#"length = "2s"
still_at = "0.2s"

[[release]]
key = "spring"
version = "1.0"
start = "-3d"
target = "+11d"

[[task]]
key = "crust"
task = "Make an all-butter crust"
release = "spring"
at = "0s"
"#;

#[test]
fn director_creates_and_assigns_a_release() {
    let (base, seen) = release_stub();
    let dir = tempfile::tempdir().unwrap();
    let body = format!(
        r#"{RELEASES}
[[cue]]
at = "0.3s"
create_release = {{ key = "summer", version = "2.0", start = "0d", target = "+7d" }}

[[cue]]
at = "0.6s"
set_release = {{ task = "crust", release = "summer" }}

[[cue]]
at = "0.9s"
filter = "summer"

[[cue]]
at = "1.2s"
set_release = {{ task = "crust", release = "backlog" }}

[[cue]]
at = "1.5s"
filter = "all"
"#
    );
    succeeded(&direct(&base, dir.path(), &body, &[]));
    let today = local_today();
    let day = |offset: &str| resolve_day(offset, today).unwrap();
    let lines = request_lines(&seen);
    let creates: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with("POST /api/products/pie/releases"))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(creates.len(), 2, "{lines:?}");
    let spawn = lines
        .iter()
        .position(|l| l.starts_with("POST /api/agents"))
        .unwrap();
    // The shot's own release exists before the clock starts; the cue's is made when the cue fires.
    assert!(creates[0] < spawn && spawn < creates[1], "{lines:?}");
    let seen_bodies: Vec<(String, Value)> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|(l, b, _)| (l.clone(), serde_json::from_str(b).unwrap_or(Value::Null)))
        .collect();
    let bodies = |prefix: &str| -> Vec<Value> {
        seen_bodies
            .iter()
            .filter(|(l, _)| l.starts_with(prefix))
            .map(|(_, b)| b.clone())
            .collect()
    };
    assert_eq!(
        bodies("POST /api/products/pie/releases "),
        [
            json!({"version": "1.0", "start": day("-3d"), "target": day("+11d")}),
            json!({"version": "2.0", "start": day("0d"), "target": day("+7d")}),
        ]
    );
    assert_eq!(body_of(&seen, "POST /api/agents ")["release"], 7);
    assert_eq!(
        bodies("PUT /api/tasks/crust-1/release "),
        [json!({"release": 8}), json!({"release": null})]
    );
    assert_eq!(cue_lines(dir.path()), ["filter 8", "filter all"]);
}

#[test]
fn director_finds_the_setup_releases_by_version() {
    let (base, seen) = stub_daemon_with(false, |line, _| {
        line.starts_with("GET /api/products ").then(|| {
            json!({"products": [{"id": "pie", "name": "pie", "default_persona": null, "repos": [], "tasks": 0,
                    "releases": [release_json(41, "0.9", "2026-01-01", "2026-02-01"),
                                 release_json(42, "1.0", "2026-10-24", "2026-11-07")]}]})
        })
    });
    let dir = tempfile::tempdir().unwrap();
    let out = direct(
        &base,
        dir.path(),
        &format!("{RELEASES}\n[[cue]]\nat = \"0.3s\"\nfilter = \"spring\"\n"),
        &["--persona", "pastry-chef", "--product", "pie"],
    );
    succeeded(&out);
    assert_eq!(body_of(&seen, "POST /api/agents ")["release"], 42);
    assert_eq!(cue_lines(dir.path()), ["filter 42"]);
    assert!(!request_lines(&seen)
        .iter()
        .any(|l| l.starts_with("POST /api/products/pie/releases")));
}

#[test]
fn director_setup_saves_the_shot_specialists() {
    let (base, seen) = stub_daemon_with(false, |line, _| {
        line.starts_with("PUT /api/definitions/specialist/laminator ")
            .then(|| {
                json!({"kind": "specialist", "name": "laminator", "summary": "s", "builtin": false,
                   "overridden": false})
            })
    });
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("content")).unwrap();
    std::fs::write(
        dir.path().join("content/laminator.md"),
        "---\nname: laminator\n---\nFold.\n",
    )
    .unwrap();
    let body = format!(
        "{CROISSANT}\n[[specialist]]\nname = \"laminator\"\nfile = \"content/laminator.md\"\n"
    );
    succeeded(&direct(&base, dir.path(), &body, &["--setup"]));
    let seen = seen.lock().unwrap();
    let (_, text, _) = seen
        .iter()
        .find(|(l, _, _)| l.starts_with("PUT /api/definitions/specialist/laminator "))
        .expect("the specialist was not saved");
    assert_eq!(text, "---\nname: laminator\n---\nFold.\n");
}

#[test]
fn director_archives_with_the_plan_it_was_given() {
    let plan = json!({"items": [{"path": "/t/crust", "bytes": 12, "uncommitted_files": 0, "stays": false}],
                      "total": 12});
    let reply = plan.clone();
    let (base, seen) = stub_daemon_with(false, move |line, _| {
        if line.starts_with("GET /api/tasks/crust-1/archive-plan ") {
            Some(reply.clone())
        } else if line.starts_with("POST /api/tasks/crust-1/archive ") {
            Some(json!({"removed": ["/t/crust"], "freed": 12, "kept": null}))
        } else {
            None
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let body = r#"length = "1s"
still_at = "0.2s"

[[task]]
key = "crust"
task = "Make an all-butter crust"
at = "0s"

[[cue]]
at = "0.3s"
stop = { task = "crust" }

[[cue]]
at = "0.6s"
archive = { task = "crust" }
"#;
    succeeded(&direct(&base, dir.path(), body, &[]));
    let lines = request_lines(&seen);
    let tail: Vec<&str> = lines
        .iter()
        .rev()
        .take(3)
        .rev()
        .map(String::as_str)
        .collect();
    assert_eq!(
        tail,
        [
            "POST /api/agents/crust-1/stop",
            "GET /api/tasks/crust-1/archive-plan",
            "POST /api/tasks/crust-1/archive"
        ]
    );
    assert_eq!(
        body_of(&seen, "POST /api/tasks/crust-1/archive ")["items"],
        plan["items"]
    );
}

#[test]
fn director_writes_the_gui_cues() {
    let (base, seen) = stub_daemon(false);
    let dir = tempfile::tempdir().unwrap();
    let body = r#"length = "2s"
still_at = "0.2s"

[[cue]]
at = "0.1s"
create_persona = { key = "sous", name = "Sous chef", commit_name = "Sous Chef", commit_email = "sous@pie.example" }

[[cue]]
at = "0.3s"
view = "management personas"

[[cue]]
at = "0.6s"
persona = "sous"

[[cue]]
at = "0.9s"
view = "management agent_settings"

[[cue]]
at = "1.2s"
view = "releases"

[[cue]]
at = "1.5s"
gui_restart = true
"#;
    succeeded(&direct(&base, dir.path(), body, &[]));
    assert_eq!(
        cue_lines(dir.path()),
        [
            "view management personas",
            "persona sous-chef",
            "view management agent_settings",
            "view releases",
            "quit"
        ]
    );
    let lines = request_lines(&seen);
    assert!(
        lines.contains(&"POST /api/personas/sous-chef/check".to_string()),
        "{lines:?}"
    );
    let sous = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|(l, _, _)| l.starts_with("POST /api/personas "))
        .map(|(_, b, _)| serde_json::from_str::<Value>(b).unwrap())
        .nth(1)
        .unwrap();
    assert_eq!(sous["commit_email"], "sous@pie.example");
}

#[test]
fn resolve_day_uses_local_time() {
    let today = fleet_api::parse_date("2026-10-27").unwrap();
    assert_eq!(resolve_day("+11d", today).unwrap(), "2026-11-07");
    assert_eq!(resolve_day("-3d", today).unwrap(), "2026-10-24");
    assert_eq!(resolve_day("0d", today).unwrap(), "2026-10-27");
    for bad in ["11", "+11", "d", "+d", "1.5d", "++1d", ""] {
        assert!(resolve_day(bad, today).is_err(), "{bad:?}");
    }
    // 02:30 UTC on the 28th is still the evening of the 27th in Montreal.
    let at = fleet_api::parse_date("2026-10-28").unwrap() * 86_400 + 2 * 3_600 + 1_800;
    let montreal = fleet_api::today(at, -4 * 3_600);
    assert_eq!(resolve_day("0d", montreal).unwrap(), "2026-10-27");
}

/// `cue` is the body of one `[[cue]]` after its `at`.
fn parse_cue(cue: &str) -> Result<fleet_demo::timeline::Timeline, String> {
    parse_timeline(&format!(
        "{HEAD}\n[[task]]\nkey = \"croissant\"\ntask = \"Laminate\"\nkind = \"direct\"\nat = \"0s\"\n\n[[cue]]\nat = \"20s\"\n{cue}\n"
    ))
}

#[test]
fn parsing_refuses_an_unknown_cue_key_with_its_number() {
    let e = parse_cue("panel_open = { task = \"crust\" }").unwrap_err();
    assert!(e.starts_with("cue 4:"), "{e}");
    assert!(e.contains("panel_open"), "{e}");
}

#[test]
fn parsing_refuses_two_actions_in_one_cue_with_its_number() {
    let e = parse_cue("stop = { task = \"crust\" }\narchive = { task = \"crust\" }").unwrap_err();
    assert!(e.starts_with("cue 4:") && e.contains("exactly one"), "{e}");
}

#[test]
fn parsing_checks_cue_words_and_targets() {
    for (cue, want) in [
        ("view = \"pantry\"", "cue 4: unknown view \"pantry\""),
        (
            "view = \"management cellar\"",
            "cue 4: unknown management tab \"cellar\"",
        ),
        (
            "view = \"kanban personas\"",
            "cue 4: view kanban has no tabs",
        ),
        (
            "expect = \"baking\"\nstop = { task = \"crust\" }",
            "cue 4: unknown status \"baking\"",
        ),
        (
            "expect = \"working\"\nview = \"kanban\"",
            "cue 4: expect needs a task",
        ),
        (
            "panel = { task = \"crust#1\", tab = \"terminal\" }",
            "cue 4: task crust has no steps",
        ),
        (
            "panel = { task = \"croissant#0\", tab = \"terminal\" }",
            "cue 4: step 0",
        ),
        (
            "approve = { task = \"croissant#1\" }",
            "cue 4: approve takes a task, not a step",
        ),
        (
            "set_release = { task = \"crust\", release = \"winter\" }",
            "cue 4: no release \"winter\"",
        ),
        ("filter = \"winter\"", "cue 4: no release \"winter\""),
        ("persona = \"sous\"", "cue 4: no persona \"sous\""),
        ("gui_restart = false", "cue 4: exactly one"),
    ] {
        let e = parse_cue(cue).unwrap_err();
        assert!(e.contains(want), "{cue}: {e}");
    }
    let t = parse_cue("view = \"management workflow\"").unwrap();
    assert!(t.cues.iter().any(|c| c.n == 4));
}

#[test]
fn parsing_checks_tasks_and_releases() {
    let task = "\n[[task]]\nkey = \"croissant\"\ntask = \"Laminate\"\nat = \"0s\"\n";
    for (extra, want) in [
        (
            task.replace("at =", "kind = \"solo\"\nat ="),
            "task croissant: unknown kind \"solo\"",
        ),
        (
            task.replace("at =", "repos = [\"cellar\"]\nat ="),
            "task croissant: no repo \"cellar\"",
        ),
        (
            task.replace("at =", "release = \"winter\"\nat ="),
            "task croissant: no release \"winter\"",
        ),
        (
            "\n[[release]]\nkey = \"spring\"\nversion = \"1\"\nstart = \"3\"\ntarget = \"+1d\"\n"
                .into(),
            "release spring: day offset \"3\"",
        ),
        (
            "\n[[release]]\nkey = \"backlog\"\nversion = \"1\"\nstart = \"0d\"\ntarget = \"+1d\"\n"
                .into(),
            "release key \"backlog\" is reserved",
        ),
    ] {
        let e = parse_timeline(&with(&extra)).unwrap_err();
        assert!(e.contains(want), "{extra}: {e}");
    }
}

#[test]
fn pacing_simulates_an_orchestrated_task_only_as_it_appears() {
    // As a solo task the crust's answer at 4 s comes 1 s into its wait; orchestrated, the daemon drives the card and
    // the director's `expect` checks it at run time.
    let t = parse_timeline(&short("24s", "13s").replace("at = \"5s\"", "at = \"4s\"")).unwrap();
    assert!(!check_pacing(&t, &real_scenarios()).is_empty());
    let orchestrated = short("24s", "13s")
        .replace("at = \"5s\"", "at = \"4s\"")
        .replace("at = \"0s\"", "kind = \"direct\"\nat = \"0s\"");
    let t = parse_timeline(&orchestrated).unwrap();
    assert_eq!(check_pacing(&t, &real_scenarios()), Vec::<String>::new());
    // Its appearance is still a board change, and its prompt still needs a scenario.
    let late = format!(
        "{orchestrated}\n[[task]]\nkey = \"bake\"\ntask = \"Bake and plate the pie\"\nat = \"0.5s\"\n"
    );
    let v = check_pacing(&parse_timeline(&late).unwrap(), &real_scenarios());
    assert!(
        v.iter()
            .any(|m| m.contains("(crust appears)") && m.contains("less than 1.5 s apart")),
        "{v:?}"
    );
    let t =
        parse_timeline(&orchestrated.replace("Make an all-butter crust", "Mow the lawn")).unwrap();
    let v = check_pacing(&t, &real_scenarios());
    assert!(v.iter().any(|m| m.contains("Mow the lawn")), "{v:?}");
}
