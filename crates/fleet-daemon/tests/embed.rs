mod common;

use common::net::*;
use common::*;
use fleet_daemon::http::{bind_loopback, router, AppState};

async fn start() -> (std::net::SocketAddr, Fx) {
    let fx = fixture().await;
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(fx.svc.clone(), fx.store.clone(), fx.tmux.clone(), addr.port());
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    (addr, fx)
}

#[tokio::test]
async fn serves_index_html_and_falls_back_for_client_routes() {
    let (addr, _fx) = start().await;
    let (s1, root) = http(addr, "GET", "/", &[], None).await;
    assert_eq!(s1, 200);
    assert!(root.contains("<div id=\"root\">"), "{root}");
    let (s2, deep) = http(addr, "GET", "/agents/some/route", &[], None).await;
    assert_eq!((s2, deep), (200, root), "unknown non-asset paths must serve the app shell");
}

#[tokio::test]
async fn content_types_and_caching_follow_the_file() {
    let (addr, _fx) = start().await;
    let raw = |path: &'static str| async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(request_text(addr, "GET", path, &[], None).as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out.split("\r\n\r\n").next().unwrap().to_ascii_lowercase()
    };
    let index = raw("/").await;
    assert!(index.contains("content-type: text/html"), "{index}");
    assert!(index.contains("cache-control: no-cache"), "{index}");
}

#[tokio::test]
async fn missing_assets_and_unknown_api_paths_are_404_not_the_app_shell() {
    let (addr, _fx) = start().await;
    for p in ["/assets/missing-123.js", "/favicon.ico", "/api/nope", "/internal/nope"] {
        let (s, body) = http(addr, "GET", p, &[], None).await;
        assert_eq!(s, 404, "{p}");
        assert!(!body.contains("<div id=\"root\">"), "{p} served the app shell");
    }
}

#[tokio::test]
async fn static_files_are_behind_the_same_host_guard() {
    let (addr, _fx) = start().await;
    let (s, _) = http(addr, "GET", "/", &[("Host", "evil.example")], None).await;
    assert_eq!(s, 403);
}
