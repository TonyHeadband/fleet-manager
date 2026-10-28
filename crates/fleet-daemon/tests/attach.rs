mod common;

use common::*;
use fleet_daemon::http::{bind_loopback, router, AppState};
use fleet_daemon::session::{Size, DEFAULT_SIZE};
use futures_util::{SinkExt, StreamExt};
use std::num::NonZeroU16;
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn nz(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n).unwrap()
}

/// Serves `fx` on an ephemeral loopback port; returns it with the port.
async fn serve_fx(fx: Fx) -> (Fx, u16) {
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();
    let state = AppState::new(fx.svc.clone(), fx.store.clone(), fx.sessions.clone(), addr);
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    (fx, port)
}

async fn serve() -> (Fx, u16) {
    serve_fx(fixture().await).await
}

/// A server whose agents' claude runs with `env` added.
async fn serve_with_env(env: &[(&str, &str)]) -> (Fx, u16) {
    let env: Vec<(String, String)> = env
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    serve_fx(fixture_options(|_| {}, |o| o.extra_env = env).await).await
}

async fn connect(port: u16, id: &str) -> WsStream {
    let url = format!("ws://127.0.0.1:{port}/api/agents/{id}/attach");
    tokio_tungstenite::connect_async(url).await.unwrap().0
}

/// Reads frames, feeding binary ones to `screen`, until `pred` holds on its text (10 s). Returns the first text frame
/// if one arrives before that.
async fn read_until(
    ws: &mut WsStream,
    screen: &mut vt100::Parser,
    pred: impl Fn(&str) -> bool,
) -> Option<String> {
    let read = async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Binary(b))) => {
                    screen.process(&b);
                    if pred(&screen.screen().contents()) {
                        return None;
                    }
                }
                Some(Ok(Message::Text(t))) => return Some(t.to_string()),
                Some(Ok(_)) => {}
                other => panic!(
                    "the socket ended: {other:?}; screen:\n{}",
                    screen.screen().contents()
                ),
            }
        }
    };
    match tokio::time::timeout(Duration::from_secs(10), read).await {
        Ok(text) => text,
        Err(_) => panic!("timed out; screen:\n{}", screen.screen().contents()),
    }
}

#[tokio::test]
async fn attaching_draws_the_screen_then_streams_and_types() {
    let (fx, port) = serve().await;
    let a = spawn_ready(&fx).await;
    let mut ws = connect(port, &a.id.0).await;
    let mut screen = vt100::Parser::new(nz(40), nz(120), 0);
    // The snapshot alone shows it.
    assert_eq!(
        read_until(&mut ws, &mut screen, |s| s.contains("ready")).await,
        None
    );
    ws.send(Message::Binary(b"abc".to_vec().into()))
        .await
        .unwrap();
    assert_eq!(
        read_until(&mut ws, &mut screen, |s| s.contains("abc")).await,
        None
    );
}

#[tokio::test]
async fn a_cursor_report_from_the_terminal_does_not_reach_the_agent() {
    let (fx, port) = serve().await;
    let a = spawn_ready(&fx).await;
    let mut ws = connect(port, &a.id.0).await;
    let mut screen = vt100::Parser::new(nz(40), nz(120), 0);
    read_until(&mut ws, &mut screen, |s| s.contains("ready")).await;
    // A report split across two frames, between ordinary keys.
    for frame in [&b"x\x1b[12;"[..], b"34Ry"] {
        ws.send(Message::Binary(frame.to_vec().into()))
            .await
            .unwrap();
    }
    read_until(&mut ws, &mut screen, |s| s.contains("xy")).await;
    let shown = fx.sessions.screen_text(&a.session).unwrap();
    assert!(!shown.contains("^["), "{shown}");
}

#[tokio::test]
async fn a_resize_message_resizes_the_session() {
    let (fx, port) = serve().await;
    let a = spawn_ready(&fx).await;
    let mut ws = connect(port, &a.id.0).await;
    ws.send(Message::Text(r#"{"resize":{"cols":90,"rows":25}}"#.into()))
        .await
        .unwrap();
    assert!(
        wait_until(|| async {
            fx.sessions.size(&a.session).unwrap() == Size { cols: 90, rows: 25 }
        })
        .await
    );
    drop(ws);
    assert!(wait_until(|| async { fx.sessions.size(&a.session).unwrap() == DEFAULT_SIZE }).await);
}

#[tokio::test]
async fn the_agent_exiting_sends_the_code_and_closes() {
    let (fx, port) = serve().await;
    let a = spawn_ready(&fx).await;
    let mut ws = connect(port, &a.id.0).await;
    fx.sessions.kill(&a.session).unwrap();
    let text = read_until(&mut ws, &mut vt100::Parser::new(nz(40), nz(120), 0), |_| {
        false
    })
    .await
    .expect("a text frame");
    assert!(text.starts_with(r#"{"exited":"#), "{text}");
    let end = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(end.is_ok(), "the socket was not closed");
}

/// A solo agent on `core`, returned once its burst has begun: "ready" scrolls off at once, so `spawn_ready` would
/// wait for it in vain.
async fn spawn_bursting(fx: &Fx) -> fleet_api::Agent {
    let a = fx
        .svc
        .spawn(fleet_api::SpawnRequest {
            actor: fleet_api::Actor("local".into()),
            product: PRODUCT.into(),
            release: None,
            repos: vec![pick(fx, "core")],
            prompt: "do the thing".into(),
            slug: None,
            persona: PERSONA.into(),
            kind: fleet_api::AgentKind::Solo,
            role: None,
            roster: vec![],
        })
        .await
        .unwrap();
    wait_screen_in(&fx.sessions, &a.session, |s| s.contains("burst")).await;
    a
}

/// Long enough to still be running when the terminal attaches, and to overflow the socket's buffers and the backlog
/// behind them. ConPTY delivers far smaller chunks than a Unix terminal, so it needs far fewer lines.
#[cfg(windows)]
const BURST: &str = "20000";
#[cfg(not(windows))]
const BURST: &str = "1000000";

#[tokio::test]
async fn a_lagging_terminal_is_redrawn_not_broken() {
    let (fx, port) = serve_with_env(&[("FAKE_CLAUDE_BURST", BURST)]).await;
    let a = spawn_bursting(&fx).await;
    let mut ws = connect(port, &a.id.0).await;
    let last = format!("burst {BURST}");
    // Nothing is read until the burst is over, so the backlog overflows. A million lines take a while on a slow
    // machine, hence its own 60 s rather than the shared 20 s.
    let until = std::time::Instant::now() + Duration::from_secs(60);
    while !fx
        .sessions
        .screen_text(&a.session)
        .unwrap_or_default()
        .contains(&last)
    {
        assert!(
            std::time::Instant::now() < until,
            "the burst did not end within 60 s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut screen = vt100::Parser::new(nz(40), nz(120), 0);
    let mut redrawn = false;
    let read = async {
        while !screen.screen().contents().contains(&last) {
            match ws.next().await {
                Some(Ok(Message::Binary(b))) => {
                    redrawn |= b.starts_with(b"\x1b[H\x1b[2J");
                    screen.process(&b);
                }
                Some(Ok(Message::Text(t))) => panic!("unexpected text frame {t}"),
                Some(Ok(_)) => {}
                other => panic!("the socket ended: {other:?}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(60), read)
        .await
        .expect("the burst's end never arrived");
    assert!(redrawn, "the terminal never lagged: raise the burst");
    assert_eq!(
        screen.screen().contents(),
        fx.sessions.screen_text(&a.session).unwrap()
    );
}

#[tokio::test]
async fn unknown_and_stopped_agents_are_refused_before_the_upgrade() {
    let (fx, port) = serve().await;
    let e =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/agents/nope/attach"))
            .await
            .unwrap_err();
    assert!(e.to_string().contains("404"), "{e}");
    let a = spawn_ready(&fx).await;
    fx.svc.stop(&a.id).await.unwrap();
    let e = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{port}/api/agents/{}/attach",
        a.id.0
    ))
    .await
    .unwrap_err();
    assert!(e.to_string().contains("409"), "{e}");
}

#[tokio::test]
async fn a_web_page_cannot_attach() {
    let (fx, port) = serve().await;
    let a = spawn_ready(&fx).await;
    let mut req = format!("ws://127.0.0.1:{port}/api/agents/{}/attach", a.id.0)
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("Origin", "https://evil.example".parse().unwrap());
    let e = tokio_tungstenite::connect_async(req).await.unwrap_err();
    // The Host/Origin guard answers 403 Forbidden.
    assert!(e.to_string().contains("403"), "{e}");
}
