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
                    actor: fleet_api::Actor("local".into()),
                    product: "p".into(),
                    release: None,
                    repos: vec![fleet_api::RepoPick {
                        repo: "https://host/team/core.git".into(),
                        local: None,
                    }],
                    prompt: "t".into(),
                    slug: None,
                    persona: "p".into(),
                    kind: fleet_api::AgentKind::Solo,
                    role: None,
                    roster: vec![],
                })
                .map(drop)
                .map_err(|e| e.0)
            }),
        ),
        (
            "add_repo",
            Box::new(|c| c.add_repo("a1", "core").map(drop).map_err(|e| e.0)),
        ),
        (
            "grant_repo",
            Box::new(|c| c.grant_repo("a1", "core").map(drop).map_err(|e| e.0)),
        ),
        (
            "archive_plan",
            Box::new(|c| c.archive_plan("a1").map(drop).map_err(|e| e.0)),
        ),
        (
            "archive",
            Box::new(|c| c.archive("a1", &empty_plan()).map(drop).map_err(|e| e.0)),
        ),
        (
            "orphan_plan",
            Box::new(|c| c.orphan_plan("worktree:/x").map(drop).map_err(|e| e.0)),
        ),
        (
            "archive_orphan",
            Box::new(|c| {
                c.archive_orphan("worktree:/x", &empty_plan())
                    .map(drop)
                    .map_err(|e| e.0)
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
