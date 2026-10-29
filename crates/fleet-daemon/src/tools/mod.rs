//! The tools the daemon needs on its machine: what they are, how to find them, how to fix them.

pub mod path;
pub mod pkg;
pub mod table;

pub use path::{merged_path, path_after_marker, PathSource, SystemPath};
pub use pkg::{detect_manager, Manager};
pub use table::{at_least, fix, install_command, parse_version, table, ToolSpec};

use fleet_api::{Os, ToolError, ToolFix, ToolId, ToolList, ToolStatus};
use futures_util::future::{BoxFuture, FutureExt, Shared};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long one tool may take to print its version.
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// No fresh PATH: the one the daemon inherited is kept. The default outside `main`, so tests never read the registry
/// or start a login shell.
pub struct CurrentPath;

impl PathSource for CurrentPath {
    fn user_path(&self) -> Option<OsString> {
        None
    }
}

/// Checks the tools on the user's real PATH and keeps the last answer. Cheap to clone: every clone shares it.
#[derive(Clone)]
pub struct Tools {
    inner: Arc<Inner>,
}

struct Inner {
    os: Os,
    source: Arc<dyn PathSource>,
    home: PathBuf,
    gh_command: String,
    claude_command: Vec<String>,
    /// The PATH the daemon was started with; merged after the fresh one.
    inherited: Option<OsString>,
    /// Test seam: added to every version call's environment. Always empty in the daemon.
    extra_env: Vec<(String, String)>,
    /// Test seam: where Git for Windows is looked for when no git on PATH has a bash beside it. `None`: the real
    /// install folders.
    git_roots: Option<Vec<PathBuf>>,
    /// The PATH is every child's (`fleet_fs::child`), not only the sessions' and the tool calls'. The daemon's own.
    for_children: bool,
    path: Mutex<OsString>,
    last: Mutex<Option<Checked>>,
    /// The check in flight, if any, with its number; callers join it.
    flight: Mutex<Option<(u64, Flight)>>,
    flights: AtomicU64,
}

type Flight = Shared<BoxFuture<'static, ToolList>>;

#[derive(Clone)]
struct Checked {
    list: ToolList,
    /// Where each usable tool is.
    found: Vec<(ToolId, PathBuf)>,
    manager: Option<Manager>,
}

impl Tools {
    /// `gh_command` and `claude_command[0]` are the daemon config's: a file path is used as is, a name is looked up on
    /// the fresh PATH.
    pub fn new(
        os: Os,
        source: Arc<dyn PathSource>,
        home: PathBuf,
        gh_command: String,
        claude_command: Vec<String>,
    ) -> Tools {
        let inherited = std::env::var_os("PATH");
        let path = merged_path(inherited.clone(), None, &home);
        Tools {
            inner: Arc::new(Inner {
                os,
                source,
                home,
                gh_command,
                claude_command,
                inherited,
                extra_env: Vec::new(),
                git_roots: None,
                for_children: false,
                path: Mutex::new(path),
                last: Mutex::new(None),
                flight: Mutex::new(None),
                flights: AtomicU64::new(0),
            }),
        }
    }

    fn setup(&mut self) -> &mut Inner {
        Arc::get_mut(&mut self.inner).expect("a Tools is set up before it is shared")
    }

    /// Test seam: the PATH taken as inherited instead of the daemon's own, so a test's checks cannot find the real
    /// CLIs. Called before the `Tools` is cloned.
    #[doc(hidden)]
    pub fn with_inherited_path(mut self, path: Option<OsString>) -> Tools {
        let inner = self.setup();
        *inner.path.get_mut().expect("tools path poisoned") =
            merged_path(path.clone(), None, &inner.home);
        inner.inherited = path;
        self
    }

    /// Test seam: variables added to every version call. Called before the `Tools` is cloned.
    #[doc(hidden)]
    pub fn with_env(mut self, env: Vec<(String, String)>) -> Tools {
        self.setup().extra_env = env;
        self
    }

    /// Test seam: the Git for Windows install folders looked in for Git Bash, instead of the real ones. Called before
    /// the `Tools` is cloned.
    #[doc(hidden)]
    pub fn with_git_roots(mut self, roots: Vec<PathBuf>) -> Tools {
        self.setup().git_roots = Some(roots);
        self
    }

    /// Makes this PATH the one every child of the daemon gets (its git and gh too), now and after every check, so a
    /// tool installed while the daemon runs is found by all of them. `main` calls it; tests that leave it out keep
    /// the test process's own PATH for git. Called before the `Tools` is cloned.
    pub fn for_children(mut self) -> Tools {
        let inner = self.setup();
        inner.for_children = true;
        fleet_fs::child::set_path(inner.path.get_mut().expect("tools path poisoned").clone());
        self
    }

    /// Stores the PATH a check computed.
    fn set_path(&self, path: OsString) {
        if self.inner.for_children {
            fleet_fs::child::set_path(path.clone());
        }
        *self.inner.path.lock().expect("tools path poisoned") = path;
    }

    /// The PATH sessions and tool calls get: the fresh one from the last check, then the inherited one.
    pub fn path(&self) -> OsString {
        self.inner.path.lock().expect("tools path poisoned").clone()
    }

    /// Where `program` is: a file path as is, else looked up on this PATH; a `.cmd` shim npm puts on PATH is followed
    /// to the exe beside it, which is what runs. `None` when it is not found.
    pub fn locate(&self, program: &str) -> Option<PathBuf> {
        let given = Path::new(program);
        let found = if given.is_file() {
            given.to_path_buf()
        } else {
            let cwd = std::env::current_dir().unwrap_or_else(|_| self.inner.home.clone());
            which::which_in(program, Some(self.path()), cwd).ok()?
        };
        Some(crate::session::resolve::shim_target(&found).unwrap_or(found))
    }

    /// The last check's answer, if one ran.
    pub fn last(&self) -> Option<ToolList> {
        self.checked().map(|c| c.list)
    }

    fn checked(&self) -> Option<Checked> {
        self.inner.last.lock().expect("tools poisoned").clone()
    }

    /// Where `id` is, or why it cannot be used. The last check answers when it found `id` at a file that is still
    /// there; otherwise a fresh check runs, so a tool installed since is found at once.
    // The error carries its fix to the person; it is rare and never on a hot path.
    #[allow(clippy::result_large_err)]
    pub async fn require(&self, id: ToolId) -> Result<PathBuf, ToolError> {
        let checked = match self.checked() {
            Some(c) if c.found.iter().any(|(t, at)| *t == id && at.is_file()) => c,
            _ => {
                self.recheck().await;
                self.checked().expect("a check was just stored")
            }
        };
        if let Some((_, at)) = checked.found.iter().find(|(t, _)| *t == id) {
            return Ok(at.clone());
        }
        let error = checked
            .list
            .tools
            .iter()
            .find(|t| t.tool == id)
            .and_then(|t| t.error.clone());
        // Not a row on this OS (Git Bash on Linux): nothing to find.
        Err(error.unwrap_or_else(|| self.not_installed(id)))
    }

    /// The POSIX shell scripts run under (a gate's steps): `sh` on Linux, Git Bash on Windows, where its absence is
    /// Git Bash's `ToolError`.
    #[allow(clippy::result_large_err)]
    pub async fn posix_shell(&self) -> Result<PathBuf, ToolError> {
        match self.inner.os {
            Os::Windows => self.require(ToolId::GitBash).await,
            Os::Linux => Ok(PathBuf::from("sh")),
        }
    }

    /// `id` is not installed, with this machine's way to install it.
    pub fn not_installed(&self, id: ToolId) -> ToolError {
        ToolError::NotInstalled {
            tool: id,
            fix: self.fix_for(id),
        }
    }

    /// `id` was found but will not start or answer (`detail` says how), with this machine's way to reinstall it.
    pub fn broken(&self, id: ToolId, detail: String) -> ToolError {
        ToolError::Broken {
            tool: id,
            detail,
            fix: self.fix_for(id),
        }
    }

    /// How to install `id` here: the last check's package manager, or one looked for now.
    fn fix_for(&self, id: ToolId) -> ToolFix {
        let manager = match self.checked() {
            Some(c) => c.manager,
            None => {
                let path = self.path();
                let cwd = std::env::current_dir().unwrap_or_else(|_| self.inner.home.clone());
                detect_manager(self.inner.os, |p| {
                    which::which_in(p, Some(&path), &cwd).is_ok()
                })
            }
        };
        fix(id, self.inner.os, manager)
    }

    /// Reads the user's PATH afresh, then finds every tool on it and asks its version. A check already in flight is
    /// joined rather than run twice (the start-up check and the intro's question come together).
    pub async fn check(&self) -> ToolList {
        let flight = {
            let mut slot = self.inner.flight.lock().expect("tools flight poisoned");
            match &*slot {
                Some((_, f)) => f.clone(),
                None => {
                    let id = self.inner.flights.fetch_add(1, Ordering::Relaxed);
                    let me = self.clone();
                    let f = async move {
                        let list = me.run_check().await;
                        let mut slot = me.inner.flight.lock().expect("tools flight poisoned");
                        if slot.as_ref().is_some_and(|(at, _)| *at == id) {
                            *slot = None;
                        }
                        list
                    }
                    .boxed()
                    .shared();
                    *slot = Some((id, f.clone()));
                    f
                }
            }
        };
        flight.await
    }

    /// A check that starts after this call: one already in flight may have looked before a tool was installed, so it
    /// is waited out first. What a recheck and `require` ask for.
    pub async fn recheck(&self) -> ToolList {
        let running = self
            .inner
            .flight
            .lock()
            .expect("tools flight poisoned")
            .as_ref()
            .map(|(_, f)| f.clone());
        if let Some(f) = running {
            f.await;
        }
        self.check().await
    }

    async fn run_check(&self) -> ToolList {
        let i = &self.inner;
        let source = i.source.clone();
        // A login shell can take seconds.
        let fresh = tokio::task::spawn_blocking(move || source.user_path())
            .await
            .unwrap_or(None);
        let path = merged_path(i.inherited.clone(), fresh, &i.home);
        self.set_path(path.clone());
        let cwd = std::env::current_dir().unwrap_or_else(|_| i.home.clone());
        let manager = detect_manager(i.os, |p| which::which_in(p, Some(&path), &cwd).is_ok());
        let specs = table(i.os);
        // Every version call at once: each may take up to its timeout. Git Bash is never run, only looked for.
        let mut asked = futures_util::future::join_all(
            specs
                .iter()
                .filter(|s| s.id != ToolId::GitBash)
                .map(|s| self.check_one(s, &path, &cwd, manager)),
        )
        .await
        .into_iter();
        let mut tools = Vec::new();
        let mut found = Vec::new();
        // Git comes before Git Bash in the table; Git Bash is looked for beside it.
        let mut git: Option<(PathBuf, Option<String>)> = None;
        for spec in specs {
            let (status, at) = match spec.id {
                ToolId::GitBash => self.git_bash(spec, git.as_ref(), manager),
                _ => asked.next().expect("one answer per tool asked"),
            };
            if spec.id == ToolId::Git {
                git = at.clone().map(|p| (p, status.version.clone()));
            }
            if let Some(at) = at {
                found.push((spec.id, at));
            }
            tools.push(status);
        }
        let list = ToolList {
            tools,
            checked_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            path: path.clone().into_string().ok(),
        };
        *i.last.lock().expect("tools poisoned") = Some(Checked {
            list: list.clone(),
            found,
            manager,
        });
        list
    }

    /// What to look for: the config's command for gh and claude, the table's name otherwise.
    fn program(&self, spec: &ToolSpec) -> String {
        match spec.id {
            ToolId::Gh => self.inner.gh_command.clone(),
            ToolId::Claude => self
                .inner
                .claude_command
                .first()
                .cloned()
                .unwrap_or_else(|| spec.program.into()),
            _ => spec.program.into(),
        }
    }

    /// One row, and where the tool is when it can be used.
    async fn check_one(
        &self,
        spec: &ToolSpec,
        path: &OsString,
        cwd: &Path,
        manager: Option<Manager>,
    ) -> (ToolStatus, Option<PathBuf>) {
        let fix = || fix(spec.id, self.inner.os, manager);
        let status = |version: Option<String>, error: Option<ToolError>| ToolStatus {
            tool: spec.id,
            required: spec.required,
            version,
            error,
        };
        let broken = |detail: String| {
            let error = ToolError::Broken {
                tool: spec.id,
                detail,
                fix: fix(),
            };
            (status(None, Some(error)), None)
        };
        let program = self.program(spec);
        let given = Path::new(&program);
        let found = if given.is_file() {
            Some(given.to_path_buf())
        } else {
            which::which_in(&program, Some(path), cwd).ok()
        };
        let Some(found) = found else {
            let error = ToolError::NotInstalled {
                tool: spec.id,
                fix: fix(),
            };
            return (status(None, Some(error)), None);
        };
        // A wrapper (`npx claude`) is not Claude Code itself: it has no version of its own to ask.
        if spec.id == ToolId::Claude && self.inner.claude_command.len() > 1 {
            return (status(None, None), Some(found));
        }
        // Windows cannot start the `.cmd` shim npm puts on PATH: the exe beside it is what runs.
        let exe = match crate::session::resolve::shim_target(&found) {
            Ok(exe) => exe,
            Err(e) => return broken(e.to_string()),
        };
        let stdout = match self.version_of(&exe, spec.version_args, path).await {
            Ok(out) => out,
            Err(detail) => return broken(detail),
        };
        let Some(version) = parse_version(spec.id, &stdout) else {
            let first = stdout.lines().next().unwrap_or("").trim();
            return broken(format!("unexpected version output {first:?}"));
        };
        if let Some(needed) = spec.min_version.filter(|n| !at_least(&version, n)) {
            let error = ToolError::TooOld {
                tool: spec.id,
                found: version.clone(),
                needed: needed.into(),
                fix: fix(),
            };
            return (status(Some(version), Some(error)), None);
        }
        (status(Some(version), None), Some(exe))
    }

    /// Present or not, never run: the bash of the Git for Windows the git found belongs to. Its version is Git's.
    fn git_bash(
        &self,
        spec: &ToolSpec,
        git: Option<&(PathBuf, Option<String>)>,
        manager: Option<Manager>,
    ) -> (ToolStatus, Option<PathBuf>) {
        let roots = match &self.inner.git_roots {
            Some(roots) => roots.clone(),
            None => fleet_fs::shell::install_roots(),
        };
        let bash = fleet_fs::shell::pick(
            std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH").map(PathBuf::from),
            git.map(|(at, _)| at.clone()),
            &roots,
        );
        let error = bash.is_none().then(|| ToolError::NotInstalled {
            tool: spec.id,
            fix: fix(spec.id, self.inner.os, manager),
        });
        let version = bash.as_ref().and_then(|_| git.and_then(|(_, v)| v.clone()));
        let status = ToolStatus {
            tool: spec.id,
            required: spec.required,
            version,
            error,
        };
        (status, bash)
    }

    /// The tool's version output, run with the fresh PATH.
    async fn version_of(
        &self,
        exe: &Path,
        args: &[&str],
        path: &OsString,
    ) -> Result<String, String> {
        let mut cmd = tokio::process::Command::new(exe);
        cmd.args(args)
            .env("PATH", path)
            .envs(self.inner.extra_env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .kill_on_drop(true);
        let what = args.join(" ");
        match tokio::time::timeout(VERSION_TIMEOUT, cmd.output()).await {
            Err(_) => Err(format!("`{what}` timed out")),
            Ok(Err(e)) => Err(format!("cannot run {}: {e}", exe.display())),
            Ok(Ok(o)) if !o.status.success() => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                let why = stderr.lines().next().unwrap_or("").trim().to_string();
                Err(format!("`{what}` failed ({}): {why}", o.status))
            }
            Ok(Ok(o)) => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        }
    }
}
