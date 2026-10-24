//! Background readers for the daemon's two SSE streams. Each runs on its own thread, reconnects by itself, and
//! reports through a channel plus a `wake` callback (a front end uses it to repaint). Dropping the handle
//! stops the thread within about half a second without blocking the caller.
use crate::schema::encode_path_segment;
use crate::sse::{SseMessage, SseParser};
use fleet_api::{Event, Snapshot};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// How often a blocked read wakes up to look at the stop flag.
const READ_SLICE: Duration = Duration::from_millis(500);
const BACKOFF_START: Duration = Duration::from_millis(1000);
const BACKOFF_MAX: Duration = Duration::from_millis(5000);

#[derive(Debug)]
pub enum StreamMsg {
    Connected,
    Disconnected,
    Snapshot(Snapshot),
    Event(Event),
}

#[derive(Debug)]
pub enum PaneMsg {
    Text(String),
    Closed,
}

pub struct StreamHandle {
    stop: Arc<AtomicBool>,
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

type Wake = Arc<dyn Fn() + Send + Sync>;

enum End {
    Stopped,
    /// The server closed the stream or something failed; the caller reconnects.
    Ended,
}

fn host_port(base_url: &str) -> String {
    base_url.trim_start_matches("http://").trim_end_matches('/').to_string()
}

/// Opens one stream and feeds every message to `on_msg` until it ends or `stop` is set. HTTP/1.0 keeps the body
/// un-chunked, so the bytes go straight into the SSE parser.
fn run_connection(
    hostport: &str,
    path: &str,
    last_id: &mut Option<String>,
    stop: &AtomicBool,
    on_connected: &mut dyn FnMut(),
    on_msg: &mut dyn FnMut(SseMessage) -> bool,
) -> End {
    let Some(addr) = hostport.to_socket_addrs().ok().and_then(|mut a| a.next()) else { return End::Ended };
    let Ok(mut s) = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) else { return End::Ended };
    let _ = s.set_read_timeout(Some(READ_SLICE));
    let _ = s.set_write_timeout(Some(CONNECT_TIMEOUT));
    let mut req = format!("GET {path} HTTP/1.0\r\nHost: {hostport}\r\nAccept: text/event-stream\r\n");
    if let Some(id) = last_id {
        req.push_str(&format!("Last-Event-ID: {id}\r\n"));
    }
    req.push_str("Connection: close\r\n\r\n");
    if s.write_all(req.as_bytes()).is_err() {
        return End::Ended;
    }

    let mut head: Vec<u8> = Vec::new();
    let mut head_done = false;
    let mut parser = SseParser::new();
    let mut buf = [0u8; 8192];
    loop {
        if stop.load(Ordering::SeqCst) {
            return End::Stopped;
        }
        let n = match s.read(&mut buf) {
            Ok(0) => return End::Ended,
            Ok(n) => n,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted) => continue,
            Err(_) => return End::Ended,
        };
        let body: &[u8] = &buf[..n];
        if !head_done {
            head.extend_from_slice(body);
            let Some(end) = head.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
            let status_line = String::from_utf8_lossy(&head[..head.iter().position(|&b| b == b'\r').unwrap_or(0)]).to_string();
            if status_line.split_whitespace().nth(1) != Some("200") {
                return End::Ended;
            }
            head_done = true;
            on_connected();
            let rest = head.split_off(end + 4);
            for m in parser.feed(&rest) {
                if let Some(id) = &m.id {
                    *last_id = Some(id.clone());
                }
                if !on_msg(m) {
                    return End::Stopped;
                }
            }
            continue;
        }
        for m in parser.feed(body) {
            if let Some(id) = &m.id {
                *last_id = Some(id.clone());
            }
            if !on_msg(m) {
                return End::Stopped;
            }
        }
    }
}

/// Sleeps for `d` in slices, returning early (true) as soon as `stop` is set.
fn sleep_unless_stopped(d: Duration, stop: &AtomicBool) -> bool {
    let mut left = d;
    while left > Duration::ZERO {
        if stop.load(Ordering::SeqCst) {
            return true;
        }
        let slice = left.min(Duration::from_millis(100));
        std::thread::sleep(slice);
        left -= slice;
    }
    stop.load(Ordering::SeqCst)
}

pub fn spawn_event_stream(base_url: &str, tx: mpsc::Sender<StreamMsg>, wake: Wake) -> StreamHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let hostport = host_port(base_url);
    std::thread::spawn(move || {
        let send = |m: StreamMsg| -> bool {
            let ok = tx.send(m).is_ok();
            wake();
            ok
        };
        let mut last_id: Option<String> = None;
        let mut backoff = BACKOFF_START;
        while !flag.load(Ordering::SeqCst) {
            let mut was_connected = false;
            let end = run_connection(
                &hostport,
                "/api/events",
                &mut last_id,
                &flag,
                &mut || {
                    was_connected = true;
                    let _ = send(StreamMsg::Connected);
                },
                &mut |m| match m.event.as_deref() {
                    Some("snapshot") => serde_json::from_str::<Snapshot>(&m.data).map_or(true, |s| send(StreamMsg::Snapshot(s))),
                    Some("event") => serde_json::from_str::<Event>(&m.data).map_or(true, |e| send(StreamMsg::Event(e))),
                    _ => true,
                },
            );
            if matches!(end, End::Stopped) || flag.load(Ordering::SeqCst) {
                break;
            }
            if !send(StreamMsg::Disconnected) {
                break;
            }
            if was_connected {
                backoff = BACKOFF_START;
            }
            if sleep_unless_stopped(backoff, &flag) {
                break;
            }
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
    });
    StreamHandle { stop }
}

pub fn spawn_pane_stream(base_url: &str, agent_id: &str, tx: mpsc::Sender<PaneMsg>, wake: Wake) -> StreamHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let hostport = host_port(base_url);
    let path = format!("/api/agents/{}/terminal", encode_path_segment(agent_id));
    std::thread::spawn(move || {
        let send = |m: PaneMsg| -> bool {
            let ok = tx.send(m).is_ok();
            wake();
            ok
        };
        let mut backoff = BACKOFF_START;
        while !flag.load(Ordering::SeqCst) {
            let mut closed = false;
            let end = run_connection(&hostport, &path, &mut None, &flag, &mut || {}, &mut |m| match m.event.as_deref() {
                Some("pane") => serde_json::from_str::<String>(&m.data).map_or(true, |t| send(PaneMsg::Text(t))),
                Some("closed") => {
                    closed = true;
                    let _ = send(PaneMsg::Closed);
                    false
                }
                _ => true,
            });
            // The agent's session ended: there is nothing to reconnect to.
            if closed || matches!(end, End::Stopped) || flag.load(Ordering::SeqCst) {
                break;
            }
            if sleep_unless_stopped(backoff, &flag) {
                break;
            }
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
    });
    StreamHandle { stop }
}
