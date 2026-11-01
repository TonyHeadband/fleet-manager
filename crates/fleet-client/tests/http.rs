use fleet_client::http::FleetClient;
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::time::Duration;

/// Longer than the client's usual 10 s timeout, as a clone or fetch behind a spawn or grant easily is.
const SLOW: Duration = Duration::from_secs(12);

type Call = Box<dyn FnOnce(&FleetClient) -> Result<(), String> + Send>;

fn agent_json() -> serde_json::Value {
    json!({
        "id": "a1", "goal": "t", "workdir": "/p/t", "workspaces": [], "session": "fleet-a1",
        "status": "queued", "activity": "", "status_since": 1, "has_changes": false, "paused": false,
        "owner": "o", "blocked_reason": null
    })
}

fn empty_plan() -> fleet_api::ArchivePlan {
    fleet_api::ArchivePlan {
        items: vec![],
        total: 0,
    }
}

fn workspace_json() -> serde_json::Value {
    json!({"repo": "core", "branch": "fleet/t", "worktree": "/p/t/core", "base": "abc", "has_changes": false})
}

/// Answers every request after `SLOW`, each on its own thread so the calls overlap.
fn slow_daemon() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
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
                reader.read_exact(&mut vec![0; length]).unwrap();
                std::thread::sleep(SLOW);
                let body = if first.starts_with("POST /api/agents ") {
                    json!({"agent": agent_json(), "warnings": []})
                } else if first.starts_with("DELETE") {
                    json!({"kept": null})
                } else if first.contains("archive-plan")
                    || first.starts_with("POST /api/orphans/plan")
                {
                    json!({"items": [], "total": 0})
                } else if first.contains("archive") {
                    json!({"removed": [], "freed": 0, "kept": null})
                } else if first.starts_with("POST") {
                    workspace_json()
                } else {
                    json!({"seq": 0, "agents": []})
                }
                .to_string();
                let mut stream = stream;
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            });
        }
    });
    base
}

#[test]
fn calls_that_may_clone_or_fetch_wait_past_the_usual_timeout() {
    let base = slow_daemon();
    let calls: Vec<(&str, Call)> = vec![
        (
            "spawn",
            Box::new(|c| {
                c.spawn(&fleet_api::SpawnRequest {
                    plan: None,
                    actor: fleet_api::Actor("local".into()),
                    product: "p".into(),
                    release: None,
                    repos: vec![fleet_api::RepoPick {
                        repo: "https://host/team/core.git".into(),
                        local: None,
                    }],
                    prompt: "t".into(),
                    slug: None,
                    title: None,
                    persona: "p".into(),
                    kind: fleet_api::AgentKind::Solo,
                    role: None,
                })
                .map(drop)
                .map_err(|e| e.message)
            }),
        ),
        (
            "add_repo",
            Box::new(|c| c.add_repo("a1", "core").map(drop).map_err(|e| e.message)),
        ),
        (
            "grant_repo",
            Box::new(|c| c.grant_repo("a1", "core").map(drop).map_err(|e| e.message)),
        ),
        (
            "archive_plan",
            Box::new(|c| c.archive_plan("a1").map(drop).map_err(|e| e.message)),
        ),
        (
            "archive",
            Box::new(|c| {
                c.archive("a1", &empty_plan())
                    .map(drop)
                    .map_err(|e| e.message)
            }),
        ),
        (
            "orphan_plan",
            Box::new(|c| {
                c.orphan_plan("worktree:/x")
                    .map(drop)
                    .map_err(|e| e.message)
            }),
        ),
        (
            "archive_orphan",
            Box::new(|c| {
                c.archive_orphan("worktree:/x", &empty_plan())
                    .map(drop)
                    .map_err(|e| e.message)
            }),
        ),
    ];
    let threads: Vec<_> = calls
        .into_iter()
        .map(|(name, call)| {
            let base = base.clone();
            std::thread::spawn(move || (name, call(&FleetClient::new(&base))))
        })
        .collect();
    // The control: a plain read keeps the short timeout, so a hung daemon still shows up quickly.
    let snapshot = FleetClient::new(&base).snapshot();
    assert!(snapshot.is_err(), "a snapshot waited {SLOW:?}");
    for t in threads {
        let (name, result) = t.join().unwrap();
        assert!(result.is_ok(), "{name}: {result:?}");
    }
}

/// Answers every request at once with `status` and `body`.
fn canned_daemon(status: &'static str, body: serde_json::Value) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let body = body.to_string();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
            let mut stream = stream;
            let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    base
}

#[test]
fn a_424_body_carries_its_tool_error() {
    let tool = fleet_api::ToolError::NotLoggedIn {
        tool: fleet_api::ToolId::Gh,
        persona: Some("work".into()),
    };
    let base = canned_daemon(
        "424 Failed Dependency",
        json!({"error": "gh is not logged in", "tool": tool}),
    );
    let err = FleetClient::new(&base).tools().unwrap_err();
    assert_eq!(err.to_string(), "gh is not logged in");
    assert_eq!(err.tool.as_deref(), Some(&tool));
}

#[test]
fn a_plain_error_body_has_no_tool_error() {
    let base = canned_daemon("409 Conflict", json!({"error": "nope"}));
    let err = FleetClient::new(&base).tools().unwrap_err();
    assert_eq!((err.message.as_str(), err.tool), ("nope", None));
}

#[test]
fn version_is_none_on_an_old_daemon() {
    let base = canned_daemon("404 Not Found", json!({"error": "not found"}));
    assert_eq!(FleetClient::new(&base).version().unwrap(), None);
}

#[test]
fn version_reads_a_current_daemon() {
    let base = canned_daemon("200 OK", json!({"version": "1.2.3"}));
    let v = FleetClient::new(&base).version().unwrap().unwrap();
    assert_eq!(v.version, "1.2.3");
}

#[test]
fn a_refusal_keeps_the_field_it_names() {
    let base = canned_daemon("400 Bad Request", json!({"error": "x", "field": "target"}));
    let err = FleetClient::new(&base).tools().unwrap_err();
    assert_eq!(
        (err.message.as_str(), err.field.as_deref()),
        ("x", Some("target"))
    );
}

#[test]
fn set_persona_token_sends_the_token_in_the_body_not_the_url() {
    let persona = fleet_api::Persona {
        id: "work".into(),
        name: "Work".into(),
        host: "github.com".into(),
        commit_name: "T".into(),
        commit_email: "t@example.invalid".into(),
        github_auth: fleet_api::GithubAuth::Pat,
        claude_dir: "/p/claude".into(),
        gh_dir: "/p/gh".into(),
        ready: true,
        claude: None,
        github: None,
        tasks: 0,
        nicknames: Default::default(),
    };
    let reply = serde_json::to_string(&persona).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut head = String::new();
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
            head.push_str(&line);
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let mut stream = stream;
        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
        (head, String::from_utf8(body).unwrap())
    });
    let got = FleetClient::new(&base)
        .set_persona_token("work", "ghp_secret123")
        .unwrap();
    assert_eq!(got, persona);
    let (head, body) = server.join().unwrap();
    assert!(head.starts_with("PUT /api/personas/work/token "), "{head}");
    assert!(!head.contains("ghp_secret123"), "{head}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap(),
        json!({"token": "ghp_secret123"})
    );
}
