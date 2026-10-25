#![allow(dead_code)]
use serde_json::Value;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub fn request_text(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> String {
    let mut req = format!("{method} {path} HTTP/1.0\r\n");
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        req += &format!("Host: 127.0.0.1:{}\r\n", addr.port());
    }
    for (k, v) in headers {
        req += &format!("{k}: {v}\r\n");
    }
    if let Some(b) = body {
        req += &format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        );
    }
    req += "\r\n";
    req + body.unwrap_or("")
}

/// One request over HTTP/1.0 (so the body is not chunked); returns status and body text.
pub async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&Value>,
) -> (u16, String) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let body = body.map(|b| b.to_string());
    s.write_all(request_text(addr, method, path, headers, body.as_deref()).as_bytes())
        .await
        .unwrap();
    let mut raw = String::new();
    tokio::time::timeout(Duration::from_secs(20), s.read_to_string(&mut raw))
        .await
        .expect("response timed out (20s)")
        .unwrap();
    let status = raw.split_whitespace().nth(1).unwrap().parse().unwrap();
    (
        status,
        raw.split_once("\r\n\r\n")
            .map(|x| x.1)
            .unwrap_or("")
            .to_string(),
    )
}

pub async fn json_of(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> (u16, Value) {
    let (status, text) = http(addr, method, path, &[], body).await;
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

#[derive(Debug)]
pub struct SseEvent {
    pub id: Option<String>,
    pub event: Option<String>,
    pub data: String,
}

pub struct SseClient {
    reader: BufReader<TcpStream>,
}

impl SseClient {
    pub async fn connect(addr: SocketAddr, path: &str, headers: &[(&str, &str)]) -> SseClient {
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(request_text(addr, "GET", path, headers, None).as_bytes())
            .await
            .unwrap();
        let mut reader = BufReader::new(s);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert!(line.contains("200"), "status line: {line}");
        loop {
            line.clear();
            reader.read_line(&mut line).await.unwrap();
            if line == "\r\n" {
                break;
            }
        }
        SseClient { reader }
    }

    pub async fn next(&mut self) -> SseEvent {
        let mut ev = SseEvent {
            id: None,
            event: None,
            data: String::new(),
        };
        let mut seen = false;
        loop {
            let mut line = String::new();
            let n = tokio::time::timeout(Duration::from_secs(20), self.reader.read_line(&mut line))
                .await
                .expect("no SSE event within 20s")
                .unwrap();
            assert!(n > 0, "stream closed");
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if seen {
                    return ev;
                }
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            seen = true;
            let (k, v) = line.split_once(':').unwrap_or((line, ""));
            let v = v.strip_prefix(' ').unwrap_or(v);
            match k {
                "id" => ev.id = Some(v.into()),
                "event" => ev.event = Some(v.into()),
                "data" => ev.data = v.into(),
                _ => {}
            }
        }
    }
}
