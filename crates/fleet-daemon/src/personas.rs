//! Personas: a Claude config dir, a gh config dir on one GitHub host, and a commit identity. fleet never reads a
//! token; whether a persona is logged in is only what the `claude` and `gh` CLIs say about its own folders.
use crate::config::is_safe_dir_name;
use crate::store::{PersonaRow, Store, StoreError};
use fleet_api::{LoginState, NewPersona, Persona, PersonaList, PersonaUpdate, RepoSource};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::Command;

const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

pub use fleet_api::ambient_vars;

#[derive(Debug, thiserror::Error)]
pub enum PersonaError {
    #[error("no such persona: {0}")]
    NotFound(String),
    #[error("the persona needs a name")]
    BadName,
    #[error("the persona needs a commit name and email")]
    BadIdentity,
    #[error("{0:?} is not a GitHub host name")]
    BadHost(String),
    #[error("{0} has not finished logging in to Claude and GitHub")]
    NotReady(String),
    #[error("Log in to {persona}'s {which} again ({detail})")]
    LoginBroken {
        persona: String,
        which: &'static str,
        detail: String,
    },
    #[error("the persona still has {0} task(s); archive them first")]
    HasTasks(u32),
    #[error("cannot write the persona folder: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Lowercase, runs of anything but `[a-z0-9]` become one `-`, trimmed; `persona` when nothing is left.
pub fn persona_id(name: &str) -> String {
    let mut id = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            id.push(c);
        } else if !id.is_empty() && !id.ends_with('-') {
            id.push('-');
        }
    }
    let id = id.trim_end_matches('-');
    if id.is_empty() {
        return "persona".into();
    }
    debug_assert!(is_safe_dir_name(id));
    id.into()
}

/// What a user pastes for a host: `https://GitHub.com/`, `ghe.example.invalid:443`, `github.com.`.
pub fn normalize_host(input: &str) -> Result<String, PersonaError> {
    let bad = || PersonaError::BadHost(input.to_string());
    let t = input.trim();
    if t.chars().any(|c| c.is_whitespace() || c == '@') {
        return Err(bad());
    }
    let t = t.split_once("://").map_or(t, |(_, rest)| rest);
    let t = t.split(['/', '?', '#']).next().unwrap_or("");
    let t = t.split(':').next().unwrap_or("");
    let t = t.trim_end_matches('.').to_ascii_lowercase();
    let t = t.strip_prefix("www.").unwrap_or(&t);
    if t.is_empty()
        || !t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
    {
        return Err(bad());
    }
    Ok(t.to_string())
}

fn identity(value: &str) -> Result<String, PersonaError> {
    let v = value.trim();
    if v.is_empty() {
        return Err(PersonaError::BadIdentity);
    }
    Ok(v.to_string())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonaEnv {
    pub id: String,
    pub host: String,
    pub vars: Vec<(String, String)>,
}

impl PersonaEnv {
    /// The persona's `CLAUDE_CONFIG_DIR`.
    pub fn claude_dir(&self) -> Option<PathBuf> {
        self.vars
            .iter()
            .find(|(k, _)| k == "CLAUDE_CONFIG_DIR")
            .map(|(_, v)| PathBuf::from(v))
    }

    /// Pre-answers Claude Code's first-run prompts in that dir; a failure is logged, never fatal. Off the runtime
    /// threads, since the write is fsynced.
    pub async fn seed_first_run(&self) {
        let Some(dir) = self.claude_dir() else { return };
        let done =
            tokio::task::spawn_blocking(move || crate::first_run::seed_first_run(&dir)).await;
        match done {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                tracing::warn!(persona = %self.id, error = %e, "cannot pre-answer first-run prompts")
            }
            Err(e) => tracing::warn!(persona = %self.id, error = %e, "first-run seeding panicked"),
        }
    }
}

/// The environment a persona's agent (and its git and gh) runs under. No token goes in: `gh` finds its login in
/// `GH_CONFIG_DIR`, and git asks `gh` for credentials.
pub fn persona_env(row: &PersonaRow, personas_dir: &Path, gh: &str) -> PersonaEnv {
    let dir = personas_dir.join(&row.id);
    let path = |p: PathBuf| p.to_string_lossy().into_owned();
    let v = |k: &str, val: String| (k.to_string(), val);
    PersonaEnv {
        id: row.id.clone(),
        host: row.host.clone(),
        vars: vec![
            v("CLAUDE_CONFIG_DIR", path(dir.join("claude"))),
            v("GH_CONFIG_DIR", path(dir.join("gh"))),
            v("GH_HOST", row.host.clone()),
            v("GIT_AUTHOR_NAME", row.commit_name.clone()),
            v("GIT_AUTHOR_EMAIL", row.commit_email.clone()),
            v("GIT_COMMITTER_NAME", row.commit_name.clone()),
            v("GIT_COMMITTER_EMAIL", row.commit_email.clone()),
            v("GIT_CONFIG_COUNT", "2".into()),
            // Empty value first: it clears every helper inherited from the user's own git config.
            v("GIT_CONFIG_KEY_0", "credential.helper".into()),
            v("GIT_CONFIG_VALUE_0", String::new()),
            // Scoped to the host, so gh is never asked for another one even if a URL slipped past the host rule.
            v(
                "GIT_CONFIG_KEY_1",
                format!("credential.https://{}.helper", row.host),
            ),
            v(
                "GIT_CONFIG_VALUE_1",
                format!("!{} auth git-credential", sh_quote(gh)),
            ),
            v("GIT_TERMINAL_PROMPT", "0".into()),
            // Set and empty: git then asks no askpass program at all, not even core.askPass or SSH_ASKPASS.
            v("GIT_ASKPASS", String::new()),
            // The user's own `url.<x>.insteadOf` could turn a checked https URL into ssh with their own key.
            v("GIT_ALLOW_PROTOCOL", "https:file".into()),
        ],
    }
}

/// Quotes `s` for `sh -c`, which is how git runs a `!` helper: a gh path with a space would otherwise split.
/// fleet-client has the same function, but the daemon only takes that crate for its tests.
fn sh_quote(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./-+@%:,=".contains(c));
    if plain {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// A status-check command: no stdin, killed when dropped, and none of the ambient tokens.
pub fn check_command(program: &str, args: &[&str], dir_var: &str, dir: &Path) -> Command {
    let mut c = Command::new(program);
    c.args(args)
        .env(dir_var, dir)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    for k in ambient_vars() {
        c.env_remove(k);
    }
    c
}

struct Output {
    ok: bool,
    text: String,
}

async fn run_check(mut cmd: Command, program: &str) -> Result<Output, String> {
    let child = cmd.output();
    match tokio::time::timeout(CHECK_TIMEOUT, child).await {
        Err(_) => Err("timed out".into()),
        Ok(Err(e)) => Err(format!("cannot run {program}: {e}")),
        Ok(Ok(o)) => {
            let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
            text.push('\n');
            text.push_str(&String::from_utf8_lossy(&o.stderr));
            Ok(Output {
                ok: o.status.success(),
                text,
            })
        }
    }
}

fn last_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .map(String::from)
}

fn failed(detail: String) -> LoginState {
    LoginState {
        ok: false,
        account: None,
        detail: Some(detail),
    }
}

/// `account <name>` in `gh auth status` output; best effort.
fn gh_account(text: &str) -> Option<String> {
    let mut words = text.split_whitespace();
    words.find(|w| *w == "account")?;
    words.next().map(String::from)
}

async fn check_github(gh: &str, host: &str, dir: &Path) -> LoginState {
    let cmd = check_command(
        gh,
        &["auth", "status", "--hostname", host],
        "GH_CONFIG_DIR",
        dir,
    );
    match run_check(cmd, gh).await {
        Err(e) => failed(e),
        Ok(o) if o.ok => LoginState {
            ok: true,
            account: gh_account(&o.text),
            detail: None,
        },
        Ok(o) => failed(last_line(&o.text).unwrap_or_else(|| "not logged in".into())),
    }
}

/// `claude auth status` prints JSON and exits 1 when logged out; exit 0 alone is not trusted, since the
/// JSON is what says whether there is a login.
async fn check_claude(claude: &[String], dir: &Path) -> LoginState {
    let Some((program, wrapper_args)) = claude.split_first() else {
        return failed("claude_command is empty".into());
    };
    // The whole command, as a spawn runs it: a wrapper like `npx claude` must see its own arguments first.
    let mut args: Vec<&str> = wrapper_args.iter().map(String::as_str).collect();
    args.extend(["auth", "status"]);
    let cmd = check_command(program, &args, "CLAUDE_CONFIG_DIR", dir);
    let o = match run_check(cmd, program).await {
        Err(e) => return failed(e),
        Ok(o) => o,
    };
    let json = o
        .text
        .lines()
        .find_map(|l| serde_json::from_str::<serde_json::Value>(l.trim()).ok())
        .or_else(|| serde_json::from_str(o.text.trim()).ok());
    let logged_in = json
        .as_ref()
        .and_then(|j| j.get("loggedIn"))
        .and_then(|v| v.as_bool());
    if o.ok && logged_in == Some(true) {
        let j = json.unwrap_or_default();
        let account = [
            j.get("email"),
            j.get("account").and_then(|a| a.get("email")),
            j.get("emailAddress"),
        ]
        .into_iter()
        .flatten()
        .find_map(|v| v.as_str())
        .map(String::from);
        return LoginState {
            ok: true,
            account,
            detail: None,
        };
    }
    failed(match logged_in {
        Some(false) => "not logged in".into(),
        _ => last_line(&o.text).unwrap_or_else(|| "not logged in".into()),
    })
}

pub struct Personas {
    store: Arc<Store>,
    personas_dir: PathBuf,
    claude: Vec<String>,
    gh: String,
    tasks_root: PathBuf,
    /// Base clones live under `<state_dir>/repos/<persona>`.
    state_dir: PathBuf,
    /// The last check of each persona, (claude, github): what the list shows until the next check.
    checked: Mutex<HashMap<String, (LoginState, LoginState)>>,
    /// Held while an id is picked and its row written, so two creates never take the same id.
    create_lock: tokio::sync::Mutex<()>,
}

impl Personas {
    pub fn new(
        store: Arc<Store>,
        personas_dir: PathBuf,
        claude: Vec<String>,
        gh: String,
        tasks_root: PathBuf,
        state_dir: PathBuf,
    ) -> Personas {
        Personas {
            store,
            personas_dir,
            claude,
            gh,
            tasks_root,
            state_dir,
            checked: Mutex::new(HashMap::new()),
            create_lock: tokio::sync::Mutex::new(()),
        }
    }

    async fn row(&self, id: &str) -> Result<PersonaRow, PersonaError> {
        self.store
            .persona(id)
            .await?
            .ok_or_else(|| PersonaError::NotFound(id.to_string()))
    }

    async fn view(&self, row: PersonaRow) -> Result<Persona, PersonaError> {
        let dir = self.personas_dir.join(&row.id);
        let (claude, github) = match self.checked.lock().unwrap().get(&row.id) {
            Some((c, g)) => (Some(c.clone()), Some(g.clone())),
            None => (None, None),
        };
        Ok(Persona {
            tasks: self.store.persona_task_count(&row.id).await?,
            claude_dir: dir.join("claude").to_string_lossy().into_owned(),
            gh_dir: dir.join("gh").to_string_lossy().into_owned(),
            id: row.id,
            name: row.name,
            host: row.host,
            commit_name: row.commit_name,
            commit_email: row.commit_email,
            github_auth: row.github_auth,
            ready: row.ready,
            claude,
            github,
        })
    }

    pub async fn list(&self) -> Result<PersonaList, PersonaError> {
        let mut personas = Vec::new();
        for row in self.store.personas().await? {
            personas.push(self.view(row).await?);
        }
        Ok(PersonaList {
            personas,
            claude_command: self.claude.clone(),
            gh_command: self.gh.clone(),
            tasks_root: self.tasks_root.to_string_lossy().into_owned(),
        })
    }

    pub async fn get(&self, id: &str) -> Result<Persona, PersonaError> {
        self.view(self.row(id).await?).await
    }

    pub async fn create(&self, req: NewPersona) -> Result<Persona, PersonaError> {
        let name = req.name.trim().to_string();
        if name.is_empty() {
            return Err(PersonaError::BadName);
        }
        let host = normalize_host(&req.host)?;
        // git refuses to commit with an empty identity, so the persona would only fail later, in an agent.
        let commit_name = identity(&req.commit_name)?;
        let commit_email = identity(&req.commit_email)?;
        let _guard = self.create_lock.lock().await;
        let base = persona_id(&name);
        let mut id = base.clone();
        let mut n = 2;
        // A leftover folder counts as taken too: a new persona must not inherit someone's login.
        while self.store.persona(&id).await?.is_some() || self.personas_dir.join(&id).exists() {
            id = format!("{base}-{n}");
            n += 1;
        }
        let dir = self.personas_dir.join(&id);
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut b = std::fs::DirBuilder::new();
            b.recursive(true).mode(0o700);
            // Both folders or neither: a half-made persona folder would be skipped as "taken" forever.
            if let Err(e) = b
                .create(dir.join("claude"))
                .and_then(|()| b.create(dir.join("gh")))
            {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(e.into());
            }
        }
        let row = PersonaRow {
            id,
            name,
            host,
            commit_name,
            commit_email,
            github_auth: req.github_auth,
            ready: false,
            created_at: crate::store::now_secs(),
        };
        if let Err(e) = self.store.insert_persona(&row).await {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e.into());
        }
        self.view(row).await
    }

    /// The id and the host never change: both are baked into folders and into logins.
    pub async fn update(&self, id: &str, req: PersonaUpdate) -> Result<Persona, PersonaError> {
        let mut row = self.row(id).await?;
        if let Some(name) = req.name {
            let name = name.trim().to_string();
            if name.is_empty() {
                return Err(PersonaError::BadName);
            }
            row.name = name;
        }
        if let Some(n) = req.commit_name {
            row.commit_name = identity(&n)?;
        }
        if let Some(e) = req.commit_email {
            row.commit_email = identity(&e)?;
        }
        if let Some(a) = req.github_auth {
            row.github_auth = a;
        }
        self.store.update_persona(&row).await?;
        self.view(row).await
    }

    pub async fn delete(&self, id: &str) -> Result<(), PersonaError> {
        self.row(id).await?;
        let n = self.store.delete_persona_if_unused(id).await?;
        if n > 0 {
            return Err(PersonaError::HasTasks(n));
        }
        self.checked.lock().unwrap().remove(id);
        let clones = self.remove_clones(id).await;
        match std::fs::remove_dir_all(self.personas_dir.join(id)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => clones,
        }
    }

    /// Removes the deleted persona's base clones. Archiving keeps a task's branches, and they live in these
    /// clones, so the folder stays while any archived record of the persona is left. Clones from before personas sit
    /// at `repos/<name>`, beside the per-persona folders, so one may share the id's name: it is kept while any
    /// remembered repo still lives there.
    async fn remove_clones(&self, id: &str) -> Result<(), PersonaError> {
        let dir = self.state_dir.join("repos").join(id);
        if self.store.persona_has_records(id).await? {
            if dir.exists() {
                tracing::warn!(persona = id, dir = %dir.display(), "kept: its clones hold archived tasks' branches");
            }
            return Ok(());
        }
        if self
            .store
            .repos()
            .await?
            .iter()
            .any(|r| r.base.starts_with(&dir))
        {
            tracing::warn!(persona = id, dir = %dir.display(), "kept: another repo's clone lives there");
            return Ok(());
        }
        match std::fs::remove_dir_all(&dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    async fn run_checks(&self, row: &PersonaRow) -> (LoginState, LoginState) {
        let dir = self.personas_dir.join(&row.id);
        let (claude_dir, gh_dir) = (dir.join("claude"), dir.join("gh"));
        let (claude, github) = tokio::join!(
            check_claude(&self.claude, &claude_dir),
            check_github(&self.gh, &row.host, &gh_dir),
        );
        self.checked
            .lock()
            .unwrap()
            .insert(row.id.clone(), (claude.clone(), github.clone()));
        (claude, github)
    }

    /// Asks both CLIs and remembers the answer. The first time both are logged in the persona becomes ready and
    /// takes over everything that predates personas.
    pub async fn check(&self, id: &str) -> Result<Persona, PersonaError> {
        let row = self.row(id).await?;
        let (claude, github) = self.run_checks(&row).await;
        self.apply(&row, &claude, &github).await?;
        self.get(id).await
    }

    /// Makes a persona ready once both logins are good. The store re-checks the row, since the CLIs took a while
    /// and the persona may have been deleted meanwhile.
    async fn apply(
        &self,
        row: &PersonaRow,
        claude: &LoginState,
        github: &LoginState,
    ) -> Result<(), PersonaError> {
        if !(claude.ok && github.ok && !row.ready) {
            return Ok(());
        }
        let paths = self.adoptable_paths(row).await?;
        if self.store.ready_and_adopt(&row.id, &paths).await?.is_none() {
            self.checked.lock().unwrap().remove(&row.id);
            return Err(PersonaError::NotFound(row.id.clone()));
        }
        Ok(())
    }

    /// The unadopted path repos `row` may take: their origin passes its host rule, or they have none. A checkout
    /// that is gone or whose origin cannot be read stays unadopted; it is claimed when typed under its persona.
    /// Only reads the local config (`git remote`), never the network.
    async fn adoptable_paths(&self, row: &PersonaRow) -> Result<Vec<String>, PersonaError> {
        let env = PersonaEnv {
            id: row.id.clone(),
            host: row.host.clone(),
            vars: Vec::new(),
        };
        let mut paths = Vec::new();
        for r in self.store.repos().await? {
            if r.persona.is_some() || r.source != RepoSource::Path || !r.base.is_dir() {
                continue;
            }
            match crate::git::remote_url(&r.base, "origin").await {
                Ok(None) => paths.push(r.location),
                Ok(Some(url)) if crate::repos::check_host(&url, &env).is_ok() => {
                    paths.push(r.location)
                }
                Ok(Some(_)) => {}
                Err(e) => {
                    tracing::warn!(repo = %r.location, error = %e, "cannot read origin; left unadopted")
                }
            }
        }
        Ok(paths)
    }

    /// Checks every persona at once, for the daemon's start-up; the answers land in the cache. The CLIs run
    /// concurrently but the results are applied oldest persona first, so what predates personas goes to the first
    /// one created.
    pub async fn check_all(self: &Arc<Self>) -> Result<(), PersonaError> {
        let mut tasks = Vec::new();
        for row in self.store.personas().await? {
            let me = self.clone();
            tasks.push(tokio::spawn(async move {
                let states = me.run_checks(&row).await;
                (row, states)
            }));
        }
        for t in tasks {
            match t.await {
                Ok((row, (claude, github))) => {
                    if let Err(e) = self.apply(&row, &claude, &github).await {
                        tracing::warn!(persona = %row.id, error = %e, "persona check failed");
                    }
                }
                Err(e) => tracing::warn!(error = %e, "persona check task failed"),
            }
        }
        Ok(())
    }

    /// The environment to start an agent under, after asking the CLIs again: a login can lapse any time.
    pub async fn ready_env(&self, id: &str) -> Result<PersonaEnv, PersonaError> {
        let row = self.row(id).await?;
        if !row.ready {
            return Err(PersonaError::NotReady(row.name));
        }
        let (claude, github) = self.run_checks(&row).await;
        for (which, state) in [("Claude", claude), ("GitHub", github)] {
            if !state.ok {
                return Err(PersonaError::LoginBroken {
                    persona: row.name,
                    which,
                    detail: state.detail.unwrap_or_default(),
                });
            }
        }
        Ok(persona_env(&row, &self.personas_dir, &self.gh))
    }
}
