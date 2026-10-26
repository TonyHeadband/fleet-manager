use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use fleet_demo::scenario::{load_dir, parse, Scenario};
use fleet_demo::timeline::{check_pacing, parse_timeline};
use serde_json::{json, Value};

fn demo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../demo")
}

fn real_scenarios() -> Vec<(PathBuf, Scenario)> {
    load_dir(&demo_dir().join("scenarios")).unwrap()
}

const HEAD: &str = r#"
persona = { name = "Pastry chef", commit_name = "Pastry Chef", commit_email = "chef@pie.example" }
repo = "bakery"
length = "24s"
still_at = "13s"

[[project]]
key = "crust"
task = "Make an all-butter crust"
at = "0s"

[[cue]]
at = "15s"
panel = { project = "crust", tab = "terminal" }

[[cue]]
at = "17s"
panel = { project = "crust", tab = "diff" }

[[cue]]
at = "19s"
panel_close = true
"#;

fn with(extra: &str) -> String {
    format!("{HEAD}{extra}")
}

#[test]
fn the_demo_timeline_passes_pacing() {
    let text = std::fs::read_to_string(demo_dir().join("timeline.toml")).unwrap();
    let t = parse_timeline(&text).unwrap();
    assert_eq!(check_pacing(&t, &real_scenarios()), Vec::<String>::new());
}

#[test]
fn pacing_flags_cues_too_close() {
    let t = parse_timeline(&with(
        r#"
[[cue]]
at = "5.5s"
answer = { project = "crust", text = "1" }

[[cue]]
at = "6.5s"
follow_up = { project = "crust", text = "again" }
"#,
    ))
    .unwrap();
    let v = check_pacing(&t, &real_scenarios());
    assert!(v.iter().any(|m| m.contains("1.5")), "{v:?}");
}

#[test]
fn pacing_flags_an_answer_before_two_seconds_of_waiting() {
    // The crust asks for permission 2.5 s in, so an answer at 4 s leaves 1.5 s to read the reason.
    let t = parse_timeline(&with(
        r#"
[[cue]]
at = "4s"
answer = { project = "crust", text = "1" }
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
fn pacing_flags_a_project_without_scenario() {
    let t = parse_timeline(&with("").replace("Make an all-butter crust", "Mow the lawn")).unwrap();
    let v = check_pacing(&t, &real_scenarios());
    assert!(v.iter().any(|m| m.contains("Mow the lawn")), "{v:?}");
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
    let t = parse_timeline(&HEAD.replace("Make an all-butter crust", "t").replace("[[cue]]\nat = \"15s\"", "[[cue]]\nat = \"3s\"\nanswer = { project = \"crust\", text = \"x\" }\n\n[[cue]]\nat = \"15s\""))
        .unwrap();
    let v = check_pacing(&t, &[(PathBuf::from("t.toml"), s)]);
    // Waits from 2 s, answered at 3 s.
    assert!(v.iter().any(|m| m.contains("2 s")), "{v:?}");
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
        "ready": true, "claude": null, "github": null, "projects": 0
    })
}

fn agent_json(id: &str, task: &str) -> Value {
    json!({
        "id": id, "task": task, "workdir": "/p/t", "workspaces": [], "tmux_session": "fleet-x",
        "status": "queued", "activity": "", "status_since": 1, "has_changes": false, "paused": false,
        "owner": "o", "blocked_reason": null
    })
}

#[test]
fn director_drives_a_stub_daemon() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let (line, body) = read_request(&mut stream);
            let reply = if line.starts_with("POST /api/personas/pastry-chef/check")
                || line.starts_with("POST /api/personas ")
            {
                persona_json()
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
            log.lock().unwrap().push((line, body));
            let text = reply.to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                text.len()
            )
            .unwrap();
        }
    });

    let dir = tempfile::tempdir().unwrap();
    let timeline = dir.path().join("timeline.toml");
    std::fs::write(
        &timeline,
        r#"
persona = { name = "Pastry chef", commit_name = "Pastry Chef", commit_email = "chef@pie.example" }
repo = "bakery"
length = "2.2s"
still_at = "1s"

[[project]]
key = "crust"
task = "Make an all-butter crust"
at = "0s"

[[project]]
key = "bake"
task = "Bake and plate the pie"
at = "0.3s"

[[cue]]
at = "0.8s"
answer = { project = "crust", text = "1" }

[[cue]]
at = "1.0s"
follow_up = { project = "bake", text = "now make it a lattice top" }

[[cue]]
at = "1.2s"
panel = { project = "crust", tab = "terminal" }

[[cue]]
at = "1.5s"
panel = { project = "crust", tab = "diff" }

[[cue]]
at = "1.8s"
panel_close = true
"#,
    )
    .unwrap();
    let cues = dir.path().join("cues");

    let out = Command::new(env!("CARGO_BIN_EXE_fleet-demo-director"))
        .args(["--base-url", &base, "--timeline"])
        .arg(&timeline)
        .arg("--cues")
        .arg(&cues)
        .args(["--repo", "/work/bakery"])
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
    assert!(lines[0]["READY ".len()..].parse::<u64>().is_ok());
    assert_eq!(lines.last(), Some(&"DONE"));

    let requests: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|(line, _)| line.trim_end_matches(" HTTP/1.1").to_string())
        .collect();
    assert_eq!(
        requests,
        [
            "POST /api/personas",
            "POST /api/personas/pastry-chef/check",
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
        .map(|(_, b)| serde_json::from_str(b).unwrap_or(Value::Null))
        .collect();
    assert_eq!(bodies[0]["github_auth"], "gh");
    assert_eq!(bodies[0]["host"], "github.com");
    assert_eq!(bodies[2]["persona"], "pastry-chef");
    assert_eq!(bodies[2]["repos"], json!(["/work/bakery"]));
    assert_eq!(bodies[2]["prompt"], "Make an all-butter crust");
    assert_eq!(bodies[4]["text"], "1");
    assert_eq!(bodies[5]["text"], "now make it a lattice top");

    let cue_text = std::fs::read_to_string(&cues).unwrap();
    assert_eq!(
        cue_text.lines().collect::<Vec<_>>(),
        ["open crust-1 terminal", "open crust-1 diff", "close"]
    );
}
