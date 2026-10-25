use fleet_daemon::tmux::{parse_version, Tmux};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

static N: AtomicU32 = AtomicU32::new(0);

/// A private tmux server per test, killed on drop, so tests never touch the user's sessions.
struct Private(Tmux);

impl Private {
    fn new() -> Private {
        let n = N.fetch_add(1, Ordering::SeqCst);
        Private(Tmux::new(Some(format!(
            "fleettest-{}-{n}",
            std::process::id()
        ))))
    }
}

impl Drop for Private {
    fn drop(&mut self) {
        self.0.kill_server_blocking();
    }
}

async fn wait_until<F, Fut>(mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        if check().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[test]
fn parses_versions_and_rejects_older_than_3_2() {
    assert_eq!(parse_version("tmux 3.8"), Some((3, 8)));
    assert_eq!(parse_version("tmux 3.3a"), Some((3, 3)));
    assert_eq!(parse_version("tmux next-3.5"), Some((3, 5)));
    assert_eq!(parse_version("tmux 2.7"), Some((2, 7)));
    assert_eq!(parse_version("nonsense"), None);
    assert!(fleet_daemon::tmux::is_supported((3, 2)));
    assert!(fleet_daemon::tmux::is_supported((4, 0)));
    assert!(!fleet_daemon::tmux::is_supported((3, 1)));
    assert!(!fleet_daemon::tmux::is_supported((2, 7)));
}

/// A fake `tmux -V` printing `version`. Written by a child `sh`: exec'ing a file another thread's fork may still
/// hold open for writing fails with ETXTBSY.
fn fake_tmux(dir: &Path, version: &str) -> String {
    let p = dir.join("tmux");
    let body = format!("#!/bin/sh\necho 'tmux {version}'\n");
    let ok = std::process::Command::new("sh")
        .args([
            "-c",
            "printf '%s' \"$1\" > \"$2\" && chmod 755 \"$2\"",
            "sh",
            &body,
        ])
        .arg(&p)
        .status()
        .unwrap();
    assert!(ok.success());
    p.to_string_lossy().into_owned()
}

#[tokio::test]
async fn a_configured_tmux_is_the_one_checked_and_named() {
    let dir = tempfile::tempdir().unwrap();
    let old = fake_tmux(dir.path(), "2.7");
    let err = Tmux::new(None)
        .with_program(&old)
        .ensure_supported()
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains(&old) && err.to_string().contains("2.7"),
        "{err}"
    );
    let missing = dir.path().join("nope").to_string_lossy().into_owned();
    let err = Tmux::new(None)
        .with_program(&missing)
        .ensure_supported()
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .starts_with(&format!("cannot run {missing}")),
        "{err}"
    );
    let recent = fake_tmux(dir.path(), "3.4");
    Tmux::new(None)
        .with_program(&recent)
        .ensure_supported()
        .await
        .unwrap();
}

#[tokio::test]
async fn installed_tmux_is_supported() {
    Tmux::new(None).ensure_supported().await.unwrap();
}

#[tokio::test]
async fn session_lifecycle() {
    let t = Private::new();
    t.0.new_session("t1", Path::new("/tmp"), &[], &s(&["sleep", "30"]))
        .await
        .unwrap();
    assert!(t.0.has_session("t1").await.unwrap());
    assert!(
        !t.0.has_session("t").await.unwrap(),
        "must match the exact name, not a prefix"
    );
    t.0.kill("t1").await.unwrap();
    assert!(!t.0.has_session("t1").await.unwrap());
}

#[tokio::test]
async fn has_session_is_false_when_no_server_runs() {
    let t = Private::new();
    assert!(!t.0.has_session("anything").await.unwrap());
    assert!(t.0.list_sessions("").await.unwrap().is_empty());
}

#[tokio::test]
async fn list_sessions_filters_by_prefix() {
    let t = Private::new();
    for n in ["fleet-a", "fleet-b", "other"] {
        t.0.new_session(n, Path::new("/tmp"), &[], &s(&["sleep", "30"]))
            .await
            .unwrap();
    }
    let mut got = t.0.list_sessions("fleet-").await.unwrap();
    got.sort();
    assert_eq!(got, ["fleet-a", "fleet-b"]);
}

#[tokio::test]
async fn send_literal_delivers_metacharacters_verbatim() {
    let t = Private::new();
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("pwned");
    t.0.new_session("c", dir.path(), &[], &s(&["cat"]))
        .await
        .unwrap();
    let text = format!("$(touch {}); 'a\" `id` ; -n", marker.display());
    t.0.send_literal("c", &text).await.unwrap();
    assert!(
        wait_until(|| async { t.0.capture("c").await.unwrap().contains("; -n") }).await,
        "text never reached the pane"
    );
    assert!(t.0.capture("c").await.unwrap().contains(&text));
    assert!(!marker.exists(), "text was executed by a shell");
}

#[tokio::test]
async fn text_starting_with_a_dash_is_not_a_flag() {
    let t = Private::new();
    t.0.new_session("c", Path::new("/tmp"), &[], &s(&["cat"]))
        .await
        .unwrap();
    t.0.send_literal("c", "-n --help").await.unwrap();
    assert!(wait_until(|| async { t.0.capture("c").await.unwrap().contains("-n --help") }).await);
}

#[tokio::test]
async fn multiline_text_arrives_as_separate_lines() {
    let t = Private::new();
    t.0.new_session("c", Path::new("/tmp"), &[], &s(&["cat"]))
        .await
        .unwrap();
    t.0.send_literal("c", "first\nsecond").await.unwrap();
    assert!(
        wait_until(|| async {
            let c = t.0.capture("c").await.unwrap();
            c.contains("first") && c.contains("second")
        })
        .await
    );
}

#[tokio::test]
async fn send_key_interrupts_the_process() {
    let t = Private::new();
    t.0.new_session("c", Path::new("/tmp"), &[], &s(&["cat"]))
        .await
        .unwrap();
    // Ctrl-C sent before cat is running is lost; wait until it echoes a line back.
    t.0.send_literal("c", "up").await.unwrap();
    assert!(
        wait_until(|| async { t.0.capture("c").await.unwrap().matches("up").count() >= 2 }).await
    );
    t.0.send_key("c", "C-c").await.unwrap();
    assert!(wait_until(|| async { !t.0.has_session("c").await.unwrap() }).await);
}

#[tokio::test]
async fn environment_reaches_the_process() {
    let t = Private::new();
    t.0.new_session(
        "e",
        Path::new("/tmp"),
        &[("FLEET_TOKEN", "abc123")],
        &s(&["sh", "-c", "echo token=$FLEET_TOKEN; sleep 30"]),
    )
    .await
    .unwrap();
    assert!(
        wait_until(|| async { t.0.capture("e").await.unwrap().contains("token=abc123") }).await
    );
}

#[tokio::test]
async fn argv_is_not_passed_through_a_shell() {
    let t = Private::new();
    t.0.new_session(
        "a",
        Path::new("/tmp"),
        &[],
        &s(&["sh", "-c", "echo \"[$0]\"; sleep 30", "$(uname);x"]),
    )
    .await
    .unwrap();
    assert!(
        wait_until(|| async { t.0.capture("a").await.unwrap().contains("[$(uname);x]") }).await
    );
}

#[tokio::test]
async fn new_session_with_a_duplicate_name_is_an_error() {
    let t = Private::new();
    t.0.new_session("d", Path::new("/tmp"), &[], &s(&["sleep", "30"]))
        .await
        .unwrap();
    assert!(t
        .0
        .new_session("d", Path::new("/tmp"), &[], &s(&["sleep", "30"]))
        .await
        .is_err());
}
