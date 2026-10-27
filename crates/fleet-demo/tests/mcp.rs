use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use fleet_demo::mcp::{HttpMcp, Mcp, McpReply};
use serde_json::{json, Value};

type Seen = Arc<Mutex<Vec<(Vec<String>, Value)>>>;

/// Answers each POST with `reply(body)`, recording the headers and body it got.
fn stub(status: &'static str, reply: impl Fn(&Value) -> String + Send + 'static) -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut headers = Vec::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                headers.push(line.trim().to_string());
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let body: Value = serde_json::from_slice(&body).unwrap();
            let text = reply(&body);
            log.lock().unwrap().push((headers, body));
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            )
            .unwrap();
        }
    });
    (base, seen)
}

fn config(dir: &std::path::Path, url: &str) -> std::path::PathBuf {
    let path = dir.join("mcp.json");
    let cfg = json!({ "mcpServers": { "fleet": {
        "type": "http", "url": url, "headers": { "X-Fleet-Token": "sekrit" },
    }}});
    std::fs::write(&path, cfg.to_string()).unwrap();
    path
}

#[test]
fn http_mcp_posts_a_tools_call_and_reads_the_text() {
    let (base, seen) = stub("200 OK", |body| {
        json!({ "jsonrpc": "2.0", "id": body["id"], "result": {
            "content": [{ "type": "text", "text": "step 2 done" }], "isError": false,
        }})
        .to_string()
    });
    let dir = tempfile::tempdir().unwrap();
    let url = format!("{base}/internal/mcp/a-1");
    let mut mcp = HttpMcp::from_config(&config(dir.path(), &url)).unwrap();
    let first = mcp.call("finish_step", json!({ "step": 2 })).unwrap();
    let second = mcp.call("finish_step", json!({ "step": 2 })).unwrap();
    assert_eq!(
        first,
        McpReply {
            text: "step 2 done".into(),
            is_error: false
        }
    );
    assert_eq!(second, first);
    let seen = seen.lock().unwrap();
    let (headers, body) = &seen[0];
    assert!(
        headers[0].starts_with("POST /internal/mcp/a-1 "),
        "{headers:?}"
    );
    assert!(
        headers
            .iter()
            .any(|h| h.eq_ignore_ascii_case("x-fleet-token: sekrit")),
        "{headers:?}"
    );
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["method"], "tools/call");
    assert_eq!(
        body["params"],
        json!({ "name": "finish_step", "arguments": { "step": 2 } })
    );
    assert_ne!(seen[0].1["id"], seen[1].1["id"], "ids must differ");
}

#[test]
fn http_mcp_reads_is_error() {
    let (base, _) = stub("200 OK", |body| {
        json!({ "jsonrpc": "2.0", "id": body["id"], "result": {
            "content": [{ "type": "text", "text": "no step 9" }], "isError": true,
        }})
        .to_string()
    });
    let dir = tempfile::tempdir().unwrap();
    let mut mcp = HttpMcp::from_config(&config(dir.path(), &base)).unwrap();
    let reply = mcp.call("spawn_specialist", json!({})).unwrap();
    assert!(reply.is_error);
    assert_eq!(reply.text, "no step 9");
}

#[test]
fn http_mcp_turns_a_json_rpc_error_into_err() {
    let (base, _) = stub("200 OK", |body| {
        json!({ "jsonrpc": "2.0", "id": body["id"],
                "error": { "code": -32601, "message": "method not found: x" } })
        .to_string()
    });
    let dir = tempfile::tempdir().unwrap();
    let mut mcp = HttpMcp::from_config(&config(dir.path(), &base)).unwrap();
    let err = mcp.call("open_pr", json!({})).unwrap_err();
    assert!(err.contains("method not found: x"), "{err}");
}

#[test]
fn http_mcp_turns_a_non_2xx_into_err() {
    let (base, _) = stub("401 Unauthorized", |_| "{}".into());
    let dir = tempfile::tempdir().unwrap();
    let mut mcp = HttpMcp::from_config(&config(dir.path(), &base)).unwrap();
    let err = mcp.call("open_pr", json!({})).unwrap_err();
    assert!(err.contains("401"), "{err}");
}

#[test]
fn http_mcp_turns_a_malformed_reply_into_err() {
    let (base, _) = stub("200 OK", |body| {
        json!({ "jsonrpc": "2.0", "id": body["id"], "result": {} }).to_string()
    });
    let dir = tempfile::tempdir().unwrap();
    let mut mcp = HttpMcp::from_config(&config(dir.path(), &base)).unwrap();
    assert!(mcp.call("open_pr", json!({})).is_err());
}

#[test]
fn from_config_without_a_fleet_server_names_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp.json");
    std::fs::write(&path, r#"{"mcpServers":{}}"#).unwrap();
    let err = HttpMcp::from_config(&path).err().unwrap();
    assert!(err.contains("mcp.json"), "{err}");
    assert!(err.contains("mcpServers.fleet.url"), "{err}");
}
