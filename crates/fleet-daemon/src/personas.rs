//! Personas: a Claude config dir, a gh config dir on one GitHub host, a GitHub token the daemon holds, and a commit
//! identity. Whether a persona is logged in to Claude is what the `claude` CLI says about its own folder; to GitHub,
//! whether the forge accepts its token.
use crate::config::is_safe_dir_name;
use crate::forge::log::{recorded, CallCtx};
use crate::forge::token::{
    read_token, token_from_gh, token_path, write_token, write_token_if_absent,
};
use crate::forge::{self, ForgeConfig, ForgeError, RepoRef};
use crate::store::{PersonaRow, Store, StoreError};
use crate::tools::Tools;
use fleet_api::{
    GithubAuth, LoginState, NewPersona, Persona, PersonaList, PersonaUpdate, ToolError, ToolId,
};
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::Command;

const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the login check and a token being set wait for the forge to say whose the token is: under the 25 s
/// fleet-client gives a check, so a slow forge is an answer rather than a client timeout.
const WHOAMI_TIMEOUT: Duration = Duration::from_secs(15);

/// The persona's nicknames for its specialists, in its folder beside its logins: only the app shows them.
const NICKNAMES_FILE: &str = "nicknames.json";
const MAX_NICKNAME: usize = 32;

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
    /// Its CLI is missing, timed out or could not start, or says the persona is not logged in.
    #[error(transparent)]
    Tool(Box<ToolError>),
    /// Its CLI's login check answered something unexpected.
    #[error("Log in to {persona}'s {which} again ({detail})")]
    LoginBroken {
        persona: String,
        which: &'static str,
        detail: String,
    },
    #[error("no GitHub token: set one in the persona")]
    NoToken,
    #[error("no token given")]
    EmptyToken,
    /// The forge did not take the token being set.
    #[error(transparent)]
    Forge(#[from] ForgeError),
    #[error("the persona still has {0} task(s); archive them first")]
    HasTasks(u32),
    #[error("the nickname of {0} must be one line of at most 32 characters")]
    BadNickname(String),
    #[error("cannot write the persona folder: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// The stored nicknames. A missing file is none, and so is one that does not parse (logged): a hand edit gone
/// wrong costs the nicknames, not the persona.
fn read_nicknames(dir: &Path) -> BTreeMap<String, String> {
    let path = dir.join(NICKNAMES_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            tracing::warn!(file = %path.display(), error = %e, "nicknames ignored: not a JSON object of strings");
            BTreeMap::new()
        }),
        Err(_) => BTreeMap::new(),
    }
}

/// Trimmed, blank ones dropped; each is one line of at most `MAX_NICKNAME` characters.
fn clean_nicknames(
    names: BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, PersonaError> {
    let mut out = BTreeMap::new();
    for (specialist, nick) in names {
        let nick = nick.trim();
        if nick.is_empty() {
            continue;
        }
        if nick.contains(['\n', '\r']) || nick.chars().count() > MAX_NICKNAME {
            return Err(PersonaError::BadNickname(specialist));
        }
        out.insert(specialist, nick.to_string());
    }
    Ok(out)
}

/// Replaces the file through a sibling, so a reader never sees half of it; none left means no file.
fn write_nicknames(dir: &Path, names: &BTreeMap<String, String>) -> Result<(), PersonaError> {
    let path = dir.join(NICKNAMES_FILE);
    if names.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{NICKNAMES_FILE}.tmp"));
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(names).expect("a map of strings serializes"),
    )?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
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

/// The empty folder beside a persona's `gh` one that fleet agents' `GH_CONFIG_DIR` points at, given the persona's own.
pub fn gh_agent_dir(gh_dir: &str) -> String {
    Path::new(gh_dir)
        .with_file_name("gh-agent")
        .to_string_lossy()
        .into_owned()
}

/// Makes sure the persona's `gh-agent` folder exists: personas made before it was added have none.
fn ensure_gh_agent(env: &PersonaEnv) -> Result<(), PersonaError> {
    if let Some((_, gh)) = env.vars.iter().find(|(k, _)| k == "GH_CONFIG_DIR") {
        std::fs::create_dir_all(gh_agent_dir(gh))?;
    }
    Ok(())
}

/// The environment a persona's agent (and its git and gh) runs under. No token goes in: `gh` finds its login in
/// `GH_CONFIG_DIR`, and git asks the daemon binary, which reads the token file, for credentials.
pub fn persona_env(row: &PersonaRow, personas_dir: &Path, daemon_exe: &Path) -> PersonaEnv {
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
            // The owner's HOME stays, so without these git reads ~/.gitconfig, ~/.config/git/config and /etc/gitconfig.
            // A file that does not exist reads as empty.
            v("GIT_CONFIG_GLOBAL", path(dir.join("gitconfig"))),
            v("GIT_CONFIG_NOSYSTEM", "1".into()),
            v("GIT_CONFIG_COUNT", "2".into()),
            // Empty value first: it clears every helper inherited from the user's own git config.
            v("GIT_CONFIG_KEY_0", "credential.helper".into()),
            v("GIT_CONFIG_VALUE_0", String::new()),
            // Scoped to the host, so the token is never offered to another one even if a URL slipped past the host rule.
            v(
                "GIT_CONFIG_KEY_1",
                format!("credential.https://{}.helper", row.host),
            ),
            v(
                "GIT_CONFIG_VALUE_1",
                format!(
                    "!{} git-credential {}",
                    sh_quote(&daemon_exe.to_string_lossy()),
                    sh_quote(&path(token_path(personas_dir, &row.id)))
                ),
            ),
            v("GIT_TERMINAL_PROMPT", "0".into()),
            // Set and empty: git then asks no askpass program at all, not even core.askPass or SSH_ASKPASS.
            v("GIT_ASKPASS", String::new()),
            // The user's own `url.<x>.insteadOf` could turn a checked https URL into ssh with their own key.
            v("GIT_ALLOW_PROTOCOL", "https:file".into()),
        ],
    }
}

/// Stores a token read from gh, unless one was set meanwhile: that one wins.
fn migrate(path: &Path, token: String) -> Result<String, PersonaError> {
    if write_token_if_absent(path, &token)? {
        return Ok(token);
    }
    read_token(path).ok_or(PersonaError::NoToken)
}

/// Quotes `s` for `sh -c`, which is how git runs a `!` helper: a path with a space would otherwise split.
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

/// A status-check command run with `path` as PATH: no stdin, killed when dropped, and none of the ambient tokens.
/// `None` when `program` is neither a file nor found on `path`.
pub fn check_command(
    program: &str,
    args: &[&str],
    dir_var: &str,
    dir: &Path,
    path: &OsStr,
) -> Option<Command> {
    let given = Path::new(program);
    let found = if given.is_file() {
        given.to_path_buf()
    } else {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        which::which_in(program, Some(path), cwd).ok()?
    };
    // Windows cannot start a process from the `.cmd` shim npm puts on PATH: run the exe beside it, as a session does.
    let program = crate::session::resolve::shim_target(&found).unwrap_or(found);
    let mut c = Command::new(program);
    c.args(args)
        .env(dir_var, dir)
        .env("PATH", path)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    for k in ambient_vars() {
        c.env_remove(k);
    }
    Some(c)
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

/// One CLI's answer about a persona's login.
struct Login {
    state: LoginState,
    /// What to refuse with instead of the state's tool problem.
    refusal: Option<PersonaError>,
}

impl Login {
    fn ok(account: Option<String>) -> Login {
        Login {
            state: LoginState {
                ok: true,
                account,
                detail: None,
                tool: None,
            },
            refusal: None,
        }
    }

    /// The CLI answered something the check cannot read.
    fn unreadable(detail: String) -> Login {
        Login {
            state: failed(detail, None),
            refusal: None,
        }
    }

    fn tool(e: ToolError) -> Login {
        Login {
            state: failed(e.to_string(), Some(e)),
            refusal: None,
        }
    }

    fn missing(tools: &Tools, id: ToolId) -> Login {
        Login::tool(tools.not_installed(id))
    }

    /// The CLI is there but timed out or could not be started: a tool problem, which logging in again cannot fix.
    fn cannot_run(tools: &Tools, id: ToolId, detail: String) -> Login {
        Login::tool(tools.broken(id, detail))
    }

    fn logged_out(detail: String, id: ToolId, persona: &str) -> Login {
        let e = ToolError::NotLoggedIn {
            tool: id,
            persona: Some(persona.to_string()),
        };
        Login {
            state: failed(detail, Some(e)),
            refusal: None,
        }
    }

    /// The persona's GitHub token is missing or refused: logged out, so the list offers a login, but refused as
    /// itself, since gh may have nothing to do with it.
    fn no_token(e: PersonaError, persona: &str) -> Login {
        let mut login = Login::logged_out(e.to_string(), ToolId::Gh, persona);
        login.refusal = Some(e);
        login
    }

    /// The refusal for a login that is not ok.
    fn refusal(self, persona: &str, which: &'static str) -> PersonaError {
        if let Some(e) = self.refusal {
            return e;
        }
        match self.state.tool {
            Some(e) => PersonaError::Tool(Box::new(e)),
            None => PersonaError::LoginBroken {
                persona: persona.to_string(),
                which,
                detail: self.state.detail.unwrap_or_default(),
            },
        }
    }
}

fn failed(detail: String, tool: Option<ToolError>) -> LoginState {
    LoginState {
        ok: false,
        account: None,
        detail: Some(detail),
        tool,
    }
}

/// `claude auth status` prints JSON and exits 1 when logged out; exit 0 alone is not trusted, since the
/// JSON is what says whether there is a login.
async fn check_claude(tools: &Tools, claude: &[String], dir: &Path, persona: &str) -> Login {
    let Some((program, wrapper_args)) = claude.split_first() else {
        return Login::unreadable("claude_command is empty".into());
    };
    // The whole command, as a spawn runs it: a wrapper like `npx claude` must see its own arguments first.
    let mut args: Vec<&str> = wrapper_args.iter().map(String::as_str).collect();
    args.extend(["auth", "status"]);
    let Some(cmd) = check_command(program, &args, "CLAUDE_CONFIG_DIR", dir, &tools.path()) else {
        return Login::missing(tools, ToolId::Claude);
    };
    let o = match run_check(cmd, program).await {
        Err(e) => return Login::cannot_run(tools, ToolId::Claude, e),
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
        return Login::ok(account);
    }
    match logged_in {
        Some(false) => Login::logged_out("not logged in".into(), ToolId::Claude, persona),
        _ => Login::unreadable(last_line(&o.text).unwrap_or_else(|| "not logged in".into())),
    }
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
    /// The PATH the CLIs are looked for on, and how to install one that is missing.
    tools: Tools,
    forge: Arc<ForgeConfig>,
    /// What git runs as the credential helper.
    daemon_exe: PathBuf,
}

impl Personas {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<Store>,
        personas_dir: PathBuf,
        claude: Vec<String>,
        gh: String,
        tasks_root: PathBuf,
        state_dir: PathBuf,
        tools: Tools,
        forge: Arc<ForgeConfig>,
        daemon_exe: PathBuf,
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
            tools,
            forge,
            daemon_exe,
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
            nicknames: read_nicknames(&dir),
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
            claude_path: self.claude.first().and_then(|c| self.located(c)),
            gh_path: self.located(&self.gh),
        })
    }

    /// Where the daemon finds `program`, for a login terminal to run. Not a Windows batch file (a wrapper's `.cmd`
    /// that is no npm shim): the terminal's bash cannot run one by its path, only by its name.
    fn located(&self, program: &str) -> Option<String> {
        let at = self.tools.locate(program)?;
        let batch = at
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
        (!batch).then(|| at.to_string_lossy().into_owned())
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
            let mut b = std::fs::DirBuilder::new();
            b.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                b.mode(0o700);
            }
            // All folders or neither: a half-made persona folder would be skipped as "taken" forever.
            if let Err(e) = b
                .create(dir.join("claude"))
                .and_then(|()| b.create(dir.join("gh")))
                .and_then(|()| b.create(dir.join("gh-agent")))
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
        if let Some(n) = req.nicknames {
            write_nicknames(&self.personas_dir.join(&row.id), &clean_nicknames(n)?)?;
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

    /// Removes the deleted persona's clones: its `repos/<id>/<product>` folders, for products the store has.
    /// Archiving keeps a task's branches, and they live in these clones, so nothing goes while any archived record
    /// of the persona is left. Anything else there (a clone from before products, `repos/<id>/<repo>`, even one named
    /// like a product, or one from before personas named like the id) can hold branches that exist nowhere else: it is kept, and said so. The
    /// products' repos are not touched.
    async fn remove_clones(&self, id: &str) -> Result<(), PersonaError> {
        let dir = self.state_dir.join("repos").join(id);
        if self.store.persona_has_records(id).await? {
            if dir.exists() {
                tracing::warn!(persona = id, dir = %dir.display(), "kept: its clones hold archived tasks' branches");
            }
            return Ok(());
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        let products: Vec<String> = self
            .store
            .products()
            .await?
            .into_iter()
            .map(|p| p.id)
            .collect();
        // A clone from before personas sits at `repos/<name>` itself, so `dir` may be one.
        let legacy = std::fs::symlink_metadata(dir.join(".git")).is_ok();
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            // A clone from before products whose repo is named like a product is a repo itself, not a product folder.
            let is_repo = std::fs::symlink_metadata(path.join(".git")).is_ok();
            if !legacy && is_dir && !is_repo && products.contains(&name) {
                if let Err(e) = std::fs::remove_dir_all(&path) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        return Err(e.into());
                    }
                }
            } else {
                tracing::warn!(persona = id, path = %path.display(), "kept: not a clone of a product; delete it if you no longer need it");
            }
        }
        // Gone only when nothing was kept in it.
        let _ = std::fs::remove_dir(&dir);
        Ok(())
    }

    /// The persona's token: the stored one, else the one its gh login holds, which is then stored. Today's logins,
    /// PATs included, live in gh's hosts file whatever `github_auth` says, so the move needs no other condition.
    pub async fn token(&self, id: &str) -> Result<String, PersonaError> {
        self.token_of(&self.row(id).await?).await
    }

    async fn token_of(&self, row: &PersonaRow) -> Result<String, PersonaError> {
        let path = token_path(&self.personas_dir, &row.id);
        if let Some(token) = read_token(&path) {
            return Ok(token);
        }
        let token = self.gh_token(row).await.ok_or(PersonaError::NoToken)?;
        migrate(&path, token)
    }

    async fn gh_token(&self, row: &PersonaRow) -> Option<String> {
        let gh_dir = self.personas_dir.join(&row.id).join("gh");
        token_from_gh(&self.gh, &gh_dir, &row.host, &self.tools.path()).await
    }

    /// Who the forge says the persona's token belongs to. For a persona that logs in through gh, the stored token is
    /// a cache of gh's: when gh now holds another one (a re-login), that one is asked about first and stored if the
    /// forge takes it. A PAT persona's token is the one set for it, so gh is not read once there is one.
    async fn github_login(&self, row: &PersonaRow) -> Result<String, PersonaError> {
        if row.github_auth != GithubAuth::Gh {
            let token = self.token_of(row).await?;
            return Ok(self.whoami(row, token).await?);
        }
        let path = token_path(&self.personas_dir, &row.id);
        let stored = read_token(&path);
        let from_gh = self.gh_token(row).await;
        let stored = match (stored, from_gh) {
            (Some(stored), Some(gh)) if stored != gh => {
                match self.whoami(row, gh.clone()).await {
                    Ok(login) => {
                        write_token(&path, &gh)?;
                        return Ok(login);
                    }
                    // gh's token is no better: the stored one may still be good.
                    Err(ForgeError::Unauthorized) => stored,
                    Err(e) => return Err(e.into()),
                }
            }
            (Some(stored), _) => stored,
            (None, Some(gh)) => migrate(&path, gh)?,
            (None, None) => return Err(PersonaError::NoToken),
        };
        Ok(self.whoami(row, stored).await?)
    }

    /// Asks the forge who `token` belongs to, as a recorded call of the persona.
    async fn whoami(&self, row: &PersonaRow, token: String) -> Result<String, ForgeError> {
        let forge = forge::github(&row.host, token, &self.forge, WHOAMI_TIMEOUT);
        let ctx = CallCtx {
            persona: row.id.clone(),
            task: None,
            agent: None,
        };
        // The call has no repo; the row then reads `<host>//`.
        let repo = RepoRef {
            host: row.host.clone(),
            owner: String::new(),
            name: String::new(),
        };
        recorded(&self.store, &ctx, "github", &repo, "whoami", move || {
            forge.whoami()
        })
        .await
    }

    /// Stores a token the forge accepts, and answers the persona as a new check finds it.
    pub async fn set_token(&self, id: &str, token: &str) -> Result<Persona, PersonaError> {
        let token = token.trim();
        if token.is_empty() {
            return Err(PersonaError::EmptyToken);
        }
        let row = self.row(id).await?;
        self.whoami(&row, token.to_string()).await?;
        write_token(&token_path(&self.personas_dir, &row.id), token)?;
        self.check(id).await
    }

    /// Whether the forge accepts the persona's token.
    async fn check_github(&self, row: &PersonaRow) -> Login {
        match self.github_login(row).await {
            Ok(login) => Login::ok(Some(login)),
            Err(e @ (PersonaError::NoToken | PersonaError::Forge(ForgeError::Unauthorized))) => {
                Login::no_token(e, &row.name)
            }
            Err(e) => Login::unreadable(e.to_string()),
        }
    }

    async fn run_checks(&self, row: &PersonaRow) -> (Login, Login) {
        let dir = self.personas_dir.join(&row.id);
        let claude_dir = dir.join("claude");
        let (claude, github) = tokio::join!(
            check_claude(&self.tools, &self.claude, &claude_dir, &row.name),
            self.check_github(row),
        );
        self.checked
            .lock()
            .unwrap()
            .insert(row.id.clone(), (claude.state.clone(), github.state.clone()));
        (claude, github)
    }

    /// Asks both CLIs and remembers the answer. The first time both are logged in the persona becomes ready.
    pub async fn check(&self, id: &str) -> Result<Persona, PersonaError> {
        let row = self.row(id).await?;
        let (claude, github) = self.run_checks(&row).await;
        self.apply(&row, &claude.state, &github.state).await?;
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
        if !self.store.set_persona_ready(&row.id).await? {
            self.checked.lock().unwrap().remove(&row.id);
            return Err(PersonaError::NotFound(row.id.clone()));
        }
        Ok(())
    }

    /// Checks every persona at once, for the daemon's start-up; the answers land in the cache. The CLIs run
    /// concurrently but the results are applied oldest persona first.
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
                    if let Err(e) = self.apply(&row, &claude.state, &github.state).await {
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
        for (which, login) in [("Claude", claude), ("GitHub", github)] {
            if !login.state.ok {
                return Err(login.refusal(&row.name, which));
            }
        }
        let env = persona_env(&row, &self.personas_dir, &self.daemon_exe);
        ensure_gh_agent(&env)?;
        Ok(env)
    }

    /// The persona's environment as stored, without asking either CLI: enough to find what its sessions left on disk,
    /// even once a login has lapsed.
    pub async fn env(&self, id: &str) -> Result<PersonaEnv, PersonaError> {
        let row = self.row(id).await?;
        Ok(persona_env(&row, &self.personas_dir, &self.daemon_exe))
    }

    /// The environment to ask GitHub under, after asking `gh` alone: watching for merges needs no Claude login, so a
    /// lapsed one must not stop it.
    pub async fn gh_env(&self, id: &str) -> Result<PersonaEnv, PersonaError> {
        let row = self.row(id).await?;
        if !row.ready {
            return Err(PersonaError::NotReady(row.name));
        }
        let github = self.check_github(&row).await;
        if let Some((_, cached)) = self.checked.lock().unwrap().get_mut(&row.id) {
            *cached = github.state.clone();
        }
        if !github.state.ok {
            return Err(github.refusal(&row.name, "GitHub"));
        }
        let env = persona_env(&row, &self.personas_dir, &self.daemon_exe);
        ensure_gh_agent(&env)?;
        Ok(env)
    }
}
