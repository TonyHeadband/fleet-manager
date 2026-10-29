use fleet_api::{AgentId, AgentSettings};
use fleet_daemon::hooks_cfg::{write_settings, RoleSettings};
use fleet_daemon::repo_rules::RepoRules;
use fleet_daemon::status::{parse_hook, HookEvent, NotificationKind};
use fleet_demo::hooks::{payload, target_from_settings, Hook, HookTarget, HttpSink, Sink};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, Instant};

#[test]
fn payload_for_each_hook_parses_in_the_daemon() {
    let input = json!({"command": "ls"});
    let cases = [
        (Hook::SessionStart, HookEvent::SessionStart),
        (
            Hook::UserPromptSubmit {
                prompt: "go".into(),
            },
            HookEvent::UserPromptSubmit,
        ),
        (
            Hook::PreToolUse {
                tool: "Edit".into(),
                input: input.clone(),
            },
            HookEvent::PreToolUse {
                tool: "Edit".into(),
            },
        ),
        (
            Hook::PostToolUse {
                tool: "Edit".into(),
                input: input.clone(),
            },
            HookEvent::PostToolUse,
        ),
        (
            Hook::PermissionRequest {
                tool: "Bash".into(),
                input,
            },
            HookEvent::PermissionRequest {
                tool: "Bash".into(),
            },
        ),
        (
            Hook::Question {
                text: "which?".into(),
            },
            HookEvent::Notification {
                kind: NotificationKind::Elicitation,
            },
        ),
        (Hook::Stop, HookEvent::Stop),
        (Hook::SessionEnd, HookEvent::SessionEnd),
    ];
    for (hook, want) in cases {
        let body = payload(&hook, "s1", Path::new("/w/r"));
        assert_eq!(parse_hook(&body).unwrap(), want, "{hook:?}");
        assert_eq!(body["session_id"], "s1");
        assert_eq!(body["cwd"], "/w/r");
        // Joined with the platform's separator, so the expectation is built the same way.
        let transcript = Path::new("/w/r").join(".demo-transcript.jsonl");
        assert_eq!(
            body["transcript_path"],
            transcript.to_string_lossy().as_ref()
        );
    }
}

#[test]
fn question_is_an_elicitation_notification() {
    let body = payload(
        &Hook::Question {
            text: "which?".into(),
        },
        "s",
        Path::new("/w"),
    );
    assert_eq!(body["hook_event_name"], "Notification");
    assert_eq!(body["notification_type"], "elicitation_dialog");
    assert_eq!(body["message"], "which?");
}

#[test]
fn target_is_read_from_daemon_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_settings(
        dir.path(),
        "http://127.0.0.1:7999",
        &AgentId("a-1".into()),
        &RepoRules::default(),
        &AgentSettings::default(),
        &BTreeMap::new(),
        &RoleSettings::default(),
    )
    .unwrap();
    let t = target_from_settings(&path, "tok".into()).unwrap();
    assert_eq!(t.url, "http://127.0.0.1:7999/internal/hooks/a-1");
    assert_eq!(t.token, "tok");
    assert!(target_from_settings(&dir.path().join("nope.json"), "t".into()).is_err());
}

fn read_request(stream: &mut std::net::TcpStream) -> (String, String, String) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).unwrap();
        buf.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buf).to_string();
        if let Some(split) = text.find("\r\n\r\n") {
            let (head, body) = (&text[..split], &text[split + 4..]);
            let len = head
                .lines()
                .find_map(|l| {
                    l.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if body.len() >= len {
                let line = head.lines().next().unwrap().to_string();
                return (line, head.to_lowercase(), body.to_string());
            }
        }
        assert!(n > 0, "connection closed early");
    }
}

#[test]
fn http_sink_posts_with_token_in_order() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for _ in 0..3 {
            let (mut s, _) = listener.accept().unwrap();
            seen.push(read_request(&mut s));
            s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                .unwrap();
        }
        seen
    });
    let mut sink = HttpSink::new(HookTarget {
        url: format!("http://127.0.0.1:{port}/internal/hooks/a"),
        token: "t".into(),
    });
    for i in 0..3 {
        sink.send(json!({ "n": i }));
    }
    let seen = server.join().unwrap();
    for (i, (line, head, body)) in seen.iter().enumerate() {
        assert_eq!(line, "POST /internal/hooks/a HTTP/1.1");
        assert!(head.contains("x-fleet-token: t"), "{head}");
        assert_eq!(serde_json::from_str::<Value>(body).unwrap()["n"], i);
    }
}

#[test]
fn http_sink_survives_a_closed_port() {
    // Port 0 is refused at once by every OS, and no port is reserved and released for something else to take.
    let mut sink = HttpSink::new(HookTarget {
        url: "http://127.0.0.1:0/x".into(),
        token: "t".into(),
    });
    let start = Instant::now();
    sink.send(json!({}));
    sink.send(json!({}));
    assert!(start.elapsed() < Duration::from_secs(6));
}

#[test]
fn vec_sink_collects() {
    let mut v: Vec<Value> = Vec::new();
    v.send(json!(1));
    assert_eq!(v, vec![json!(1)]);
}
