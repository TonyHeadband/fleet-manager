#![allow(dead_code)]
pub mod net;
use fleet_api::AgentId;
use fleet_daemon::agents::{AgentService, Clock, ServiceOptions};
use fleet_daemon::config::{Config, Repo};
use fleet_daemon::store::Store;
use fleet_daemon::tmux::Tmux;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

static N: AtomicU32 = AtomicU32::new(0);

pub struct TestClock(pub AtomicI64);

impl TestClock {
    pub fn advance(&self, secs: i64) {
        self.0.fetch_add(secs, Ordering::SeqCst);
    }
    pub fn now_for_test(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

impl Clock for TestClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

pub struct Fx {
    pub _tmp: tempfile::TempDir,
    pub repo: PathBuf,
    pub state_dir: PathBuf,
    pub store: Arc<Store>,
    pub tmux: Tmux,
    pub clock: Arc<TestClock>,
    pub svc: Arc<AgentService>,
}

impl Drop for Fx {
    fn drop(&mut self) {
        self.tmux.kill_server_blocking();
    }
}

pub fn fake_claude() -> Vec<String> {
    vec![format!("{}/tests/bin/claude", env!("CARGO_MANIFEST_DIR"))]
}

pub async fn fixture() -> Fx {
    fixture_with(fake_claude(), None).await
}

/// `socket` overrides the tmux server name, used to force tmux failures.
pub async fn fixture_with(claude: Vec<String>, socket: Option<String>) -> Fx {
    build(claude, socket, fleet_daemon::store::DEFAULT_RETAINED_EVENTS).await
}

/// A fixture whose event log keeps only the last `retain` events.
pub async fn fixture_retaining(retain: u64) -> Fx {
    build(fake_claude(), None, retain).await
}

async fn build(claude: Vec<String>, socket: Option<String>, retain: u64) -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("core");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README"), "hi").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let state_dir = tmp.path().join("state");
    let cfg = Config {
        listen: "127.0.0.1:7878".parse().unwrap(),
        state_dir: state_dir.clone(),
        branch_prefix: "fleet".into(),
        claude_command: vec!["claude".into()],
        repos: vec![Repo { name: "core".into(), path: repo.clone() }],
    };
    let store = Arc::new(Store::open_with(":memory:".as_ref(), retain).await.unwrap());
    let sock = socket.unwrap_or_else(|| format!("fleetagents-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let tmux = Tmux::new(Some(sock));
    let clock = Arc::new(TestClock(AtomicI64::new(1_000)));
    let mut opts = ServiceOptions::new("http://127.0.0.1:7878");
    opts.claude = claude;
    opts.owner = "antonio".into();
    opts.clock = clock.clone();
    let svc = Arc::new(AgentService::new(cfg, store.clone(), tmux.clone(), opts));
    Fx { _tmp: tmp, repo, state_dir, store, tmux, clock, svc }
}

pub async fn wait_until<F, Fut>(mut check: F) -> bool
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

pub async fn pane_contains(fx: &Fx, id: &AgentId, needle: &str) -> bool {
    let name = session_name(id);
    wait_until(|| async { fx.tmux.capture(&name).await.map(|c| c.contains(needle)).unwrap_or(false) }).await
}

pub fn session_name(id: &AgentId) -> String {
    format!("fleet-{}", &id.0[..8])
}

pub async fn agent(fx: &Fx, id: &AgentId) -> fleet_api::Agent {
    fx.store.snapshot().await.unwrap().agents.into_iter().find(|a| &a.id == id).unwrap()
}
