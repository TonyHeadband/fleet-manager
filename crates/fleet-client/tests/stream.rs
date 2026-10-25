use fleet_client::stream::{
    spawn_event_stream, spawn_event_stream_with_silence, spawn_pane_stream, PaneMsg, StreamMsg,
};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

const REMOVED: &str = r#"{"seq":5,"kind":{"type":"agent_removed","id":"x"}}"#;

fn listener() -> (TcpListener, String) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    (l, url)
}

fn wake() -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(|| {})
}

/// Reads the request head the client sent.
fn read_request(s: &mut TcpStream) -> String {
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if s.read(&mut byte).unwrap() == 0 {
            break;
        }
        buf.push(byte[0]);
    }
    String::from_utf8(buf).unwrap()
}

fn sse_head() -> &'static str {
    "HTTP/1.0 200 OK\r\ncontent-type: text/event-stream\r\n\r\n"
}

fn next<T>(rx: &mpsc::Receiver<T>) -> T {
    rx.recv_timeout(Duration::from_secs(5))
        .expect("no message within 5s")
}

#[test]
fn reconnects_with_last_event_id_and_reports_the_gap() {
    let (l, url) = listener();
    let server = std::thread::spawn(move || {
        let (mut a, _) = l.accept().unwrap();
        let first = read_request(&mut a);
        a.write_all(sse_head().as_bytes()).unwrap();
        a.write_all(format!("id: 5\nevent: event\ndata: {REMOVED}\n\n").as_bytes())
            .unwrap();
        drop(a); // the daemon went away
        let (mut b, _) = l.accept().unwrap();
        let second = read_request(&mut b);
        b.write_all(sse_head().as_bytes()).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        (first, second)
    });
    let (tx, rx) = mpsc::channel();
    let _h = spawn_event_stream(&url, tx, wake());
    assert!(matches!(next(&rx), StreamMsg::Connected));
    assert!(matches!(next(&rx), StreamMsg::Event(e) if e.seq == 5));
    assert!(matches!(next(&rx), StreamMsg::Disconnected));
    assert!(matches!(next(&rx), StreamMsg::Connected));
    let (first, second) = server.join().unwrap();
    assert!(first.starts_with("GET /api/events HTTP/1."), "{first}");
    assert!(
        !first.to_ascii_lowercase().contains("last-event-id"),
        "{first}"
    );
    assert!(second.contains("Last-Event-ID: 5"), "{second}");
    assert!(
        first.contains("Host: 127.0.0.1:"),
        "the daemon's Host guard needs it: {first}"
    );
}

#[test]
fn a_snapshot_event_is_decoded() {
    let (l, url) = listener();
    std::thread::spawn(move || {
        let (mut a, _) = l.accept().unwrap();
        read_request(&mut a);
        a.write_all(sse_head().as_bytes()).unwrap();
        a.write_all(b"event: snapshot\nid: 3\ndata: {\"seq\":3,\"agents\":[]}\n\n")
            .unwrap();
        std::thread::sleep(Duration::from_millis(500));
    });
    let (tx, rx) = mpsc::channel();
    let _h = spawn_event_stream(&url, tx, wake());
    assert!(matches!(next(&rx), StreamMsg::Connected));
    assert!(matches!(next(&rx), StreamMsg::Snapshot(s) if s.seq == 3 && s.agents.is_empty()));
}

#[test]
fn a_non_200_answer_is_a_disconnect_not_a_connection() {
    let (l, url) = listener();
    std::thread::spawn(move || {
        let (mut a, _) = l.accept().unwrap();
        read_request(&mut a);
        a.write_all(b"HTTP/1.0 403 Forbidden\r\n\r\n").unwrap();
    });
    let (tx, rx) = mpsc::channel();
    let _h = spawn_event_stream(&url, tx, wake());
    assert!(matches!(next(&rx), StreamMsg::Disconnected));
}

#[test]
fn a_stream_that_goes_silent_is_a_disconnect() {
    let (l, url) = listener();
    std::thread::spawn(move || {
        let (mut a, _) = l.accept().unwrap();
        read_request(&mut a);
        a.write_all(sse_head().as_bytes()).unwrap();
        // Open but silent, like a suspended daemon: no keep-alive ever comes.
        std::thread::sleep(Duration::from_secs(30));
        drop(a);
    });
    let (tx, rx) = mpsc::channel();
    let _h = spawn_event_stream_with_silence(&url, tx, wake(), Duration::from_secs(1));
    assert!(matches!(next(&rx), StreamMsg::Connected));
    let t = Instant::now();
    assert!(matches!(next(&rx), StreamMsg::Disconnected));
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "waited {:?}",
        t.elapsed()
    );
}

#[test]
fn keep_alives_hold_a_quiet_stream_open() {
    let (l, url) = listener();
    std::thread::spawn(move || {
        let (mut a, _) = l.accept().unwrap();
        read_request(&mut a);
        a.write_all(sse_head().as_bytes()).unwrap();
        for _ in 0..12 {
            std::thread::sleep(Duration::from_millis(250));
            if a.write_all(b":\n\n").is_err() {
                return;
            }
        }
    });
    let (tx, rx) = mpsc::channel();
    let _h = spawn_event_stream_with_silence(&url, tx, wake(), Duration::from_secs(1));
    assert!(matches!(next(&rx), StreamMsg::Connected));
    assert!(
        rx.recv_timeout(Duration::from_millis(2500)).is_err(),
        "dropped a stream that kept sending keep-alives"
    );
}

#[test]
fn endless_headers_are_given_up_on() {
    let (l, url) = listener();
    let (seen_tx, seen_rx) = mpsc::channel();
    std::thread::spawn(move || {
        for conn in 0.. {
            let Ok((mut a, _)) = l.accept() else { return };
            read_request(&mut a);
            let _ = seen_tx.send(conn);
            if conn == 0 {
                // Headers that never reach the blank line, until the client hangs up.
                std::thread::spawn(move || {
                    let pad = format!("X-Pad: {}\r\n", "a".repeat(1000));
                    let _ = a.write_all(b"HTTP/1.0 200 OK\r\n");
                    while a.write_all(pad.as_bytes()).is_ok() {}
                });
            }
        }
    });
    let (tx, _rx) = mpsc::channel();
    let _h = spawn_event_stream(&url, tx, wake());
    assert_eq!(seen_rx.recv_timeout(Duration::from_secs(10)), Ok(0));
    assert_eq!(
        seen_rx.recv_timeout(Duration::from_secs(20)),
        Ok(1),
        "kept reading headers instead of reconnecting"
    );
}

#[test]
fn dropping_the_handle_closes_the_connection_quickly() {
    let (l, url) = listener();
    let server = std::thread::spawn(move || {
        let (mut a, _) = l.accept().unwrap();
        read_request(&mut a);
        a.write_all(sse_head().as_bytes()).unwrap();
        a.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let start = Instant::now();
        let mut b = [0u8; 8];
        let closed = matches!(a.read(&mut b), Ok(0) | Err(_));
        (closed, start.elapsed())
    });
    let (tx, rx) = mpsc::channel();
    let h = spawn_event_stream(&url, tx, wake());
    assert!(matches!(next(&rx), StreamMsg::Connected));
    let t = Instant::now();
    drop(h);
    assert!(
        t.elapsed() < Duration::from_millis(100),
        "drop must not block the caller"
    );
    let (closed, took) = server.join().unwrap();
    assert!(
        closed && took < Duration::from_secs(2),
        "connection still open after {took:?}"
    );
}

#[test]
fn a_daemon_that_accepts_but_never_answers_does_not_hang_the_handle() {
    let (l, url) = listener();
    let server = std::thread::spawn(move || {
        let (mut a, _) = l.accept().unwrap();
        read_request(&mut a); // and then say nothing at all
        a.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut b = [0u8; 8];
        matches!(a.read(&mut b), Ok(0) | Err(_))
    });
    let (tx, _rx) = mpsc::channel();
    let h = spawn_event_stream(&url, tx, wake());
    std::thread::sleep(Duration::from_millis(300));
    drop(h);
    assert!(
        server.join().unwrap(),
        "the client kept the silent connection open"
    );
}

#[test]
fn an_unreachable_daemon_gives_disconnected_and_a_responsive_handle() {
    let (l, url) = listener();
    drop(l); // nothing listens on this port now
    let (tx, rx) = mpsc::channel();
    let h = spawn_event_stream(&url, tx, wake());
    assert!(matches!(next(&rx), StreamMsg::Disconnected));
    let t = Instant::now();
    drop(h);
    assert!(t.elapsed() < Duration::from_millis(100));
}

#[test]
fn pane_stream_sends_text_then_closed_and_stops() {
    let (l, url) = listener();
    let server = std::thread::spawn(move || {
        let (mut a, _) = l.accept().unwrap();
        let req = read_request(&mut a);
        a.write_all(sse_head().as_bytes()).unwrap();
        a.write_all(b"event: pane\ndata: \"$ ready\\n> hi\"\n\nevent: closed\ndata:\n\n")
            .unwrap();
        drop(a);
        l.set_nonblocking(true).unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        (req, l.accept().is_ok())
    });
    let (tx, rx) = mpsc::channel();
    let _h = spawn_pane_stream(&url, "a/1", tx, wake());
    assert!(matches!(next(&rx), PaneMsg::Text(t) if t == "$ ready\n> hi"));
    assert!(matches!(next(&rx), PaneMsg::Closed));
    let (req, reconnected) = server.join().unwrap();
    assert!(
        req.starts_with("GET /api/agents/a%2F1/terminal HTTP/1."),
        "{req}"
    );
    assert!(!reconnected, "a closed pane stream must not reconnect");
}
