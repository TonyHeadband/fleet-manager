#![allow(dead_code)]
pub mod net;
use fleet_api::AgentId;
use fleet_daemon::agents::{AgentService, Clock, ServiceOptions};
use fleet_daemon::config::{Config, Repo};
use fleet_daemon::repos::Repos;
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
    pub projects_root: PathBuf,
    pub store: Arc<Store>,
    pub repos: Arc<Repos>,
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
        projects_root: tmp.path().join("fleet-projects"),
        seed_repos: vec![Repo { name: "core".into(), path: repo.clone() }],
        telemetry: Default::default(),
    };
    let projects_root = cfg.projects_root.clone();
    std::fs::create_dir_all(&projects_root).unwrap();
    let store = Arc::new(Store::open_with(":memory:".as_ref(), retain).await.unwrap());
    store.seed_repos(&cfg.seed_repos).await.unwrap();
    let repos = Arc::new(Repos::new(store.clone(), state_dir.clone(), projects_root.clone(), tmp.path().to_path_buf()));
    let sock = socket.unwrap_or_else(|| format!("fleetagents-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let tmux = Tmux::new(Some(sock));
    let clock = Arc::new(TestClock(AtomicI64::new(1_000)));
    let mut opts = ServiceOptions::new("http://127.0.0.1:7878");
    opts.claude = claude;
    opts.owner = "antonio".into();
    opts.clock = clock.clone();
    let svc = Arc::new(AgentService::new(cfg, store.clone(), repos.clone(), tmux.clone(), opts));
    Fx { _tmp: tmp, repo, state_dir, projects_root, store, repos, tmux, clock, svc }
}

/// A committed git repo at `dir`, outside the projects root unless `dir` is inside it.
pub fn make_repo(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("README"), "hi").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "init"]);
    dir.to_path_buf()
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

/// An agent the daemon has never heard of, for "no such agent" cases.
pub fn agent_fixture(_fx: &Fx) -> fleet_api::Agent {
    serde_json::from_value(serde_json::json!({
        "id": "ghost", "task": "t", "workdir": "/w", "tmux_session": "s",
        "workspaces": [{"repo": "core", "branch": "b", "worktree": "/w/core", "base": "abc", "has_changes": false}],
        "status": "working", "activity": "", "status_since": 1, "has_changes": false, "paused": false,
        "owner": "o", "blocked_reason": null
    }))
    .unwrap()
}
