use fleet_daemon::session::{Key, Recv, SessionHost, Size, Spec, DEFAULT_SIZE};
use std::time::{Duration, Instant};

fn fake() -> Vec<String> {
    vec![env!("CARGO_BIN_EXE_fake-claude").to_string()]
}

fn start(host: &SessionHost, name: &str, env: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let argv = fake();
    host.start(
        name,
        Spec {
            argv: &argv,
            cwd: dir.path(),
            env,
            remove_env: &[],
        },
    )
    .unwrap();
    dir
}

/// Polls the screen until `pred` holds, failing after 10 s with the screen in the message.
fn wait_screen(host: &SessionHost, name: &str, pred: impl Fn(&str) -> bool) -> String {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let s = host.screen_text(name).unwrap_or_default();
        if pred(&s) {
            return s;
        }
        assert!(Instant::now() < until, "timed out; screen:\n{s}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_session_answers_the_cursor_query_and_runs_with_its_env_and_cwd() {
    let host = SessionHost::new();
    let dir = start(&host, "fleet-a", &[("FLEET_TOKEN", "tok")]);
    let s = wait_screen(&host, "fleet-a", |s| s.contains("ready"));
    assert!(s.contains("token=tok"), "{s}");
    let cwd = fleet_fs::canonical(dir.path()).unwrap();
    assert!(s.contains(&format!("cwd={}", cwd.display())), "{s}");
    assert!(host.alive("fleet-a"));
    assert_eq!(host.list(), vec!["fleet-a".to_string()]);
    host.kill("fleet-a").unwrap();
}

#[test]
fn a_query_split_across_two_reads_is_still_answered() {
    let host = SessionHost::new();
    let _d = start(&host, "fleet-split", &[("FAKE_CLAUDE_SPLIT_DSR", "1")]);
    wait_screen(&host, "fleet-split", |s| s.contains("ready"));
    host.kill("fleet-split").unwrap();
}

#[test]
fn removed_variables_never_reach_the_program() {
    let host = SessionHost::new();
    let dir = tempfile::tempdir().unwrap();
    let argv = fake();
    // SAFETY: set before any thread of this test reads the environment.
    unsafe { std::env::set_var("GH_TOKEN", "ambient") };
    host.start(
        "fleet-env",
        Spec {
            argv: &argv,
            cwd: dir.path(),
            env: &[],
            remove_env: &["GH_TOKEN"],
        },
    )
    .unwrap();
    let s = wait_screen(&host, "fleet-env", |s| s.contains("ready"));
    assert!(s.contains("gh_token=unset"), "{s}");
    host.kill("fleet-env").unwrap();
}

#[test]
fn keys_and_text_arrive_as_typed() {
    let host = SessionHost::new();
    let _d = start(&host, "fleet-k", &[]);
    wait_screen(&host, "fleet-k", |s| s.contains("ready"));
    host.write_key("fleet-k", Key::CtrlU).unwrap();
    host.write_text("fleet-k", "hi there").unwrap();
    host.write_key("fleet-k", Key::Escape).unwrap();
    wait_screen(&host, "fleet-k", |s| s.contains("^Uhi there^["));
    host.kill("fleet-k").unwrap();
}

#[test]
fn an_exit_ends_the_session_and_tells_watchers() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let host = SessionHost::new();
    let dir = tempfile::tempdir().unwrap();
    let argv = fake();
    host.start(
        "fleet-x",
        Spec {
            argv: &argv,
            cwd: dir.path(),
            env: &[("FAKE_CLAUDE_EXIT", "3"), ("FAKE_CLAUDE_NO_DSR", "1")],
            remove_env: &[],
        },
    )
    .unwrap();
    let mut rx = host.watch("fleet-x").unwrap();
    let code = rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(fleet_daemon::session::Output::Exited(c)) = rx.recv().await {
                    return c;
                }
            }
        })
        .await
        .unwrap()
    });
    assert_eq!(code, 3);
    let until = Instant::now() + Duration::from_secs(5);
    while host.alive("fleet-x") {
        assert!(Instant::now() < until, "session still listed after exit");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(host.list().is_empty());
}

#[test]
fn kill_ends_the_program_and_frees_the_name() {
    let host = SessionHost::new();
    let _d = start(&host, "fleet-kill", &[]);
    wait_screen(&host, "fleet-kill", |s| s.contains("ready"));
    host.kill("fleet-kill").unwrap();
    assert!(!host.alive("fleet-kill"));
    assert!(matches!(
        host.kill("fleet-kill"),
        Err(fleet_daemon::session::SessionError::Missing(_))
    ));
    let _d2 = start(&host, "fleet-kill", &[]);
    wait_screen(&host, "fleet-kill", |s| s.contains("ready"));
    host.shutdown_all();
    assert!(host.list().is_empty());
}

#[test]
fn a_second_start_under_the_same_name_is_refused() {
    let host = SessionHost::new();
    let _d = start(&host, "fleet-dup", &[]);
    let argv = fake();
    let e = host
        .start(
            "fleet-dup",
            Spec {
                argv: &argv,
                cwd: _d.path(),
                env: &[],
                remove_env: &[],
            },
        )
        .unwrap_err();
    assert!(matches!(e, fleet_daemon::session::SessionError::Exists(_)));
    host.kill("fleet-dup").unwrap();
}

#[test]
fn a_program_that_cannot_start_is_an_error_not_a_session() {
    let host = SessionHost::new();
    let dir = tempfile::tempdir().unwrap();
    let argv = vec![dir.path().join("missing-program").display().to_string()];
    let e = host
        .start(
            "fleet-bad",
            Spec {
                argv: &argv,
                cwd: dir.path(),
                env: &[],
                remove_env: &[],
            },
        )
        .unwrap_err();
    assert!(
        matches!(e, fleet_daemon::session::SessionError::Spawn { .. }),
        "{e:?}"
    );
    assert!(!host.alive("fleet-bad"));
}

#[test]
fn the_snapshot_redraws_the_same_screen() {
    let host = SessionHost::new();
    let _d = start(&host, "fleet-snap", &[]);
    let before = wait_screen(&host, "fleet-snap", |s| s.contains("ready"));
    let att = host.attach("fleet-snap", DEFAULT_SIZE).unwrap();
    let mut p = vt100::Parser::new(nz(40), nz(120), 0);
    p.process(&att.snapshot());
    assert_eq!(p.screen().contents(), before);
    drop(att);
    host.kill("fleet-snap").unwrap();
}

#[test]
fn the_latest_terminal_sets_the_size_and_detaching_hands_it_back() {
    let host = SessionHost::new();
    let _d = start(&host, "fleet-size", &[]);
    wait_screen(&host, "fleet-size", |s| s.contains("ready"));
    let a = host
        .attach(
            "fleet-size",
            Size {
                cols: 100,
                rows: 30,
            },
        )
        .unwrap();
    assert_eq!(
        host.size("fleet-size").unwrap(),
        Size {
            cols: 100,
            rows: 30
        }
    );
    let b = host
        .attach("fleet-size", Size { cols: 80, rows: 24 })
        .unwrap();
    assert_eq!(
        host.size("fleet-size").unwrap(),
        Size { cols: 80, rows: 24 }
    );
    a.resize(Size { cols: 90, rows: 20 });
    assert_eq!(
        host.size("fleet-size").unwrap(),
        Size { cols: 90, rows: 20 }
    );
    drop(a);
    assert_eq!(
        host.size("fleet-size").unwrap(),
        Size { cols: 80, rows: 24 }
    );
    drop(b);
    assert_eq!(host.size("fleet-size").unwrap(), DEFAULT_SIZE);
    host.kill("fleet-size").unwrap();
}

#[test]
fn an_attachment_gets_bytes_then_the_exit() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let host = SessionHost::new();
    let _d = start(&host, "fleet-att", &[]);
    wait_screen(&host, "fleet-att", |s| s.contains("ready"));
    let mut att = host.attach("fleet-att", DEFAULT_SIZE).unwrap();
    host.write_text("fleet-att", "x").unwrap();
    rt.block_on(async {
        let got = tokio::time::timeout(Duration::from_secs(10), att.recv())
            .await
            .unwrap();
        assert!(matches!(got, Some(Recv::Bytes(b)) if b.contains(&b'x')));
    });
    host.kill("fleet-att").unwrap();
    rt.block_on(async {
        loop {
            match tokio::time::timeout(Duration::from_secs(10), att.recv())
                .await
                .unwrap()
            {
                Some(Recv::Exited(_)) | None => break,
                _ => {}
            }
        }
    });
}

fn nz(n: u16) -> std::num::NonZeroU16 {
    std::num::NonZeroU16::new(n).unwrap()
}
