#![allow(dead_code)]
pub mod net;
use fleet_api::{AgentId, RepoPick};
use fleet_daemon::agents::{AgentService, Clock, ServiceOptions};
use fleet_daemon::config::Config;
use fleet_daemon::repos::Repos;
use fleet_daemon::store::{ProductRow, RepoRow, Store};
use fleet_daemon::tmux::Tmux;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

static N: AtomicU32 = AtomicU32::new(0);

/// The persona every fixture makes ready, on `github.com`.
pub const PERSONA: &str = "p";
/// The product every fixture makes, holding the path-only repo `core` (the fixture's `repo`).
pub const PRODUCT: &str = "p";
pub const COMMIT_EMAIL: &str = "tester@example.com";

/// Creates a persona, logs both fake CLIs in for it and checks it, so it is ready. Returns its id.
pub async fn ready_persona(svc: &AgentService, name: &str, host: &str) -> String {
    let p = svc
        .personas()
        .create(fleet_api::NewPersona {
            name: name.into(),
            host: host.into(),
            commit_name: "Tester".into(),
            commit_email: COMMIT_EMAIL.into(),
            github_auth: fleet_api::GithubAuth::Gh,
        })
        .await
        .unwrap();
    std::fs::write(Path::new(&p.claude_dir).join(".fake-login"), "").unwrap();
    std::fs::write(Path::new(&p.gh_dir).join("hosts.yml"), "x").unwrap();
    let checked = svc.personas().check(&p.id).await.unwrap();
    assert!(checked.ready, "{checked:?}");
    p.id
}

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
    let mut cmd = Command::new("git");
    // An agent's environment (where the gate runs) points `remote.origin.pushurl` at a blocked transport through
    // `GIT_CONFIG_*`, which would refuse the fixtures' own pushes.
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(k);
        }
    }
    let out = cmd
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

pub struct Fx {
    pub _tmp: tempfile::TempDir,
    pub repo: PathBuf,
    pub state_dir: PathBuf,
    pub tasks_root: PathBuf,
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

pub fn fake_gh() -> String {
    format!("{}/tests/bin/gh", env!("CARGO_MANIFEST_DIR"))
}

pub async fn fixture() -> Fx {
    fixture_with(fake_claude(), None).await
}

/// `socket` overrides the tmux server name, used to force tmux failures.
pub async fn fixture_with(claude: Vec<String>, socket: Option<String>) -> Fx {
    build(
        claude,
        socket,
        fleet_daemon::store::DEFAULT_RETAINED_EVENTS,
        true,
        |_| {},
    )
    .await
}

/// The default fixture with `tweak` applied to its config first.
pub async fn fixture_config(tweak: impl FnOnce(&mut Config)) -> Fx {
    build(
        fake_claude(),
        None,
        fleet_daemon::store::DEFAULT_RETAINED_EVENTS,
        true,
        tweak,
    )
    .await
}

/// A fixture whose event log keeps only the last `retain` events.
pub async fn fixture_retaining(retain: u64) -> Fx {
    build(fake_claude(), None, retain, true, |_| {}).await
}

/// A fixture with no persona at all.
pub async fn fixture_without_persona() -> Fx {
    build(
        fake_claude(),
        None,
        fleet_daemon::store::DEFAULT_RETAINED_EVENTS,
        false,
        |_| {},
    )
    .await
}

/// A fixture whose store is a file, so a test can open a second connection to break it; returns the file's path.
pub async fn fixture_on_file() -> (Fx, PathBuf) {
    let fx = build_in(
        fake_claude(),
        None,
        fleet_daemon::store::DEFAULT_RETAINED_EVENTS,
        true,
        |_| {},
        true,
        |_| {},
    )
    .await;
    let db = fx._tmp.path().join("fleet.db");
    (fx, db)
}

/// The default fixture with `cfg` applied to its config and `opts` to its service options first.
pub async fn fixture_options(
    cfg: impl FnOnce(&mut Config),
    opts: impl FnOnce(&mut ServiceOptions),
) -> Fx {
    build_in(
        fake_claude(),
        None,
        fleet_daemon::store::DEFAULT_RETAINED_EVENTS,
        true,
        cfg,
        false,
        opts,
    )
    .await
}

async fn build(
    claude: Vec<String>,
    socket: Option<String>,
    retain: u64,
    persona: bool,
    tweak: impl FnOnce(&mut Config),
) -> Fx {
    build_in(claude, socket, retain, persona, tweak, false, |_| {}).await
}

async fn build_in(
    claude: Vec<String>,
    socket: Option<String>,
    retain: u64,
    persona: bool,
    tweak: impl FnOnce(&mut Config),
    file_db: bool,
    opts_tweak: impl FnOnce(&mut ServiceOptions),
) -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("core");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README"), "hi").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let state_dir = tmp.path().join("state");
    let mut cfg = Config {
        listen: "127.0.0.1:7878".parse().unwrap(),
        state_dir: state_dir.clone(),
        branch_prefix: "fleet".into(),
        // The persona checks always ask the fake, so a test can start a missing or odd `claude` under a ready persona.
        claude_command: fake_claude(),
        tmux_command: "tmux".into(),
        tasks_root: tmp.path().join("fleet-tasks"),
        personas_dir: tmp.path().join("personas"),
        gh_command: fake_gh(),
        agent_config_dir: tmp.path().join("agent"),
        fleet_dir: tmp.path().join("fleet"),
        test_timeout_secs: 1800,
        limits: Default::default(),
        telemetry: Default::default(),
        legacy_repos: 0,
    };
    tweak(&mut cfg);
    let tasks_root = cfg.tasks_root.clone();
    std::fs::create_dir_all(&tasks_root).unwrap();
    let db = if file_db {
        tmp.path().join("fleet.db")
    } else {
        PathBuf::from(":memory:")
    };
    let store = Arc::new(Store::open_with(&db, retain).await.unwrap());
    store
        .insert_product(&ProductRow {
            id: PRODUCT.into(),
            name: PRODUCT.into(),
            default_persona: None,
            created_at: 1,
        })
        .await
        .unwrap();
    store
        .upsert_repo(&RepoRow {
            product: PRODUCT.into(),
            name: "core".into(),
            path: Some(repo.clone()),
            url: None,
            roster: vec![],
        })
        .await
        .unwrap();
    let repos = Arc::new(Repos::new(
        store.clone(),
        state_dir.clone(),
        tasks_root.clone(),
        tmp.path().to_path_buf(),
    ));
    let sock = socket.unwrap_or_else(|| {
        format!(
            "fleetagents-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        )
    });
    let tmux = Tmux::new(Some(sock));
    let clock = Arc::new(TestClock(AtomicI64::new(1_000)));
    let mut opts = ServiceOptions::new("http://127.0.0.1:7878");
    opts.claude = claude;
    opts.owner = "antonio".into();
    opts.clock = clock.clone();
    opts_tweak(&mut opts);
    let svc = Arc::new(AgentService::new(
        cfg,
        store.clone(),
        repos.clone(),
        tmux.clone(),
        opts,
    ));
    if persona {
        assert_eq!(ready_persona(&svc, PERSONA, "github.com").await, PERSONA);
    }
    Fx {
        _tmp: tmp,
        repo,
        state_dir,
        tasks_root,
        store,
        repos,
        tmux,
        clock,
        svc,
    }
}

/// `name` as a spawn picks it: the fixture's path-only `core` from its checkout, anything else (a typed path or
/// URL, or a repo with a URL) with no checkout.
pub fn pick(fx: &Fx, name: &str) -> RepoPick {
    RepoPick {
        repo: name.into(),
        local: (name == "core").then(|| fx.repo.to_string_lossy().into_owned()),
    }
}

/// Adds a path-only repo to the fixture's product.
pub async fn add_path_repo(fx: &Fx, name: &str, path: &Path) {
    fx.store
        .upsert_repo(&RepoRow {
            product: PRODUCT.into(),
            name: name.into(),
            path: Some(path.to_path_buf()),
            url: None,
            roster: vec![],
        })
        .await
        .unwrap();
}

/// A committed git repo at `dir`, outside the tasks root unless `dir` is inside it.
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
    wait_until(|| async {
        fx.tmux
            .capture(&name)
            .await
            .map(|c| c.contains(needle))
            .unwrap_or(false)
    })
    .await
}

pub fn session_name(id: &AgentId) -> String {
    format!("fleet-{}", &id.0[..8])
}

pub async fn agent(fx: &Fx, id: &AgentId) -> fleet_api::Agent {
    fx.store
        .snapshot()
        .await
        .unwrap()
        .agents
        .into_iter()
        .find(|a| &a.id == id)
        .unwrap()
}

/// An agent the daemon has never heard of, for "no such agent" cases.
pub fn agent_fixture(_fx: &Fx) -> fleet_api::Agent {
    serde_json::from_value(serde_json::json!({
        "id": "ghost", "goal": "t", "workdir": "/w", "tmux_session": "s",
        "workspaces": [{"repo": "core", "branch": "b", "worktree": "/w/core", "base": "abc", "has_changes": false}],
        "status": "working", "activity": "", "status_since": 1, "has_changes": false, "paused": false,
        "owner": "o", "blocked_reason": null
    }))
    .unwrap()
}

/// Holds a spawn between making its task folder and storing its record: a `post-checkout` hook in `repo` (run by
/// `git worktree add`) waits until the gate opens. Opened on drop, and the hook gives up after 20 s on its own, so a
/// failing test leaves no git process behind.
pub struct SpawnGate {
    reached: PathBuf,
    open: PathBuf,
}

impl SpawnGate {
    pub fn install(repo: &Path, dir: &Path) -> SpawnGate {
        use std::os::unix::fs::PermissionsExt;
        let gate = SpawnGate {
            reached: dir.join("gate-reached"),
            open: dir.join("gate-open"),
        };
        let hook = repo.join(".git/hooks/post-checkout");
        std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
        std::fs::write(
            &hook,
            format!(
                "#!/bin/sh\ntouch '{}'\ni=0\nwhile [ ! -e '{}' ] && [ $i -lt 400 ]; do sleep 0.05; i=$((i+1)); done\n",
                gate.reached.display(),
                gate.open.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        gate
    }

    pub async fn wait_reached(&self) -> bool {
        wait_until(|| async { self.reached.exists() }).await
    }

    pub fn open(&self) {
        let _ = std::fs::write(&self.open, b"");
    }
}

impl Drop for SpawnGate {
    fn drop(&mut self) {
        self.open();
    }
}

/// The environment of the fixture's ready persona, for calling `Repos` directly.
pub async fn fixture_env(fx: &Fx) -> fleet_daemon::personas::PersonaEnv {
    fx.svc.personas().ready_env(PERSONA).await.unwrap()
}

/// Gives `core` a bare `file://` origin holding its `main`, so `open_pr` really fetches and pushes; returns it.
pub fn add_origin(fx: &Fx) -> PathBuf {
    let tmp = fx._tmp.path();
    let bare = tmp.join("origin.git");
    git(
        tmp,
        &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()],
    );
    git(
        &fx.repo,
        &[
            "remote",
            "add",
            "origin",
            &format!("file://{}", bare.display()),
        ],
    );
    git(&fx.repo, &["push", "-q", "origin", "main"]);
    bare
}

fn commit_workflow(fx: &Fx, yaml: &str) {
    std::fs::create_dir_all(fx.repo.join(".fleet")).unwrap();
    std::fs::write(fx.repo.join(".fleet/workflow.yml"), yaml).unwrap();
    git(&fx.repo, &["add", ".fleet/workflow.yml"]);
    // Empty when `yaml` is already there: the caller still gets a new commit on `main`.
    git(
        &fx.repo,
        &["commit", "-q", "--allow-empty", "-m", "workflow"],
    );
}

/// `add_origin`, with `yaml` committed as `core`'s `.fleet/workflow.yml` on `main` first.
pub fn add_origin_with_workflow(fx: &Fx, yaml: &str) -> PathBuf {
    commit_workflow(fx, yaml);
    add_origin(fx)
}

/// Commits `yaml` as `core`'s workflow on `main` and pushes it to the origin `add_origin` made; returns that origin.
pub fn push_workflow(fx: &Fx, yaml: &str) -> PathBuf {
    commit_workflow(fx, yaml);
    git(&fx.repo, &["push", "-q", "origin", "main"]);
    fx._tmp.path().join("origin.git")
}

/// A workflow of one step, `test`, running `command`.
pub fn one_step(command: &str) -> String {
    // A JSON string is a YAML string, whatever the command's quotes and colons.
    format!(
        "steps:\n  test:\n    run: {}\n",
        serde_json::to_string(command).unwrap()
    )
}
