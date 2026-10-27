//! The director's script: when each task starts and what is typed or cued afterwards, all in one file so the
//! shot is tuned without code changes.
use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fleet_api::{format_day, Status};
use serde::Deserialize;

use crate::scenario::{find, parse_duration, Scenario, StepKind};

/// Two board changes closer than this read as one jump.
const MIN_CHANGE_GAP: Duration = Duration::from_millis(1500);
/// A panel cue is not a column change, so it may sit closer to one; two panel cues still need time to be read.
const MIN_PANEL_GAP: Duration = Duration::from_millis(750);
const MIN_PANEL_STEP: Duration = Duration::from_millis(1500);
/// Between the daemon spawning a task and the agent's first hook: what `fleet-demo-agent` and tmux take. Added
/// to every time the scenario decides.
pub const AGENT_STARTUP: Duration = Duration::from_millis(1000);
/// Float parsing of `0.3s`-style delays can land a nanosecond short.
const EPS: Duration = Duration::from_millis(1);
/// The time to read why a card needs you, before the answer arrives.
const MIN_WAIT: Duration = Duration::from_secs(2);
/// The span of the official shorts; the pie sits inside it.
const LENGTH_RANGE: (Duration, Duration) = (Duration::from_secs(15), Duration::from_secs(30));

#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    /// The product the director creates; the GUI opens on its board.
    pub product: String,
    /// At least one; the first is where tasks are spawned.
    pub repos: Vec<RepoSpec>,
    pub persona: PersonaSpec,
    pub length: Duration,
    /// The moment the still is cut from.
    pub still_at: Duration,
    /// The window's inner size, `WxH`.
    pub size: String,
    pub output: Output,
    pub tasks: Vec<Task>,
    pub cues: Vec<Cue>,
    pub pacing: Pacing,
    /// Created during setup, before the clock starts.
    pub releases: Vec<ReleaseSpec>,
    /// Saved during setup, so a roster can name them.
    pub specialists: Vec<SpecialistSpec>,
}

/// A release of the product. `start` and `target` are day offsets from today (`"-3d"`, `"+11d"`, `"0d"`), resolved
/// when the release is created so the shot never goes stale.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseSpec {
    /// How tasks and cues name it; the daemon's id is only known once it exists.
    pub key: String,
    pub version: String,
    pub start: String,
    pub target: String,
}

/// A specialist definition the shot brings; `file` is relative to the shot folder.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecialistSpec {
    pub name: String,
    pub file: String,
}

/// The pacing rules a shot opts into on top of the ones every shot obeys (length, spacing, answers).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pacing {
    /// Exactly one terminal, diff, close sequence on one task.
    pub panel_sequence: bool,
    /// The exact card counts at `still_at`.
    pub still: Option<StillCounts>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StillCounts {
    pub working: usize,
    pub needs_you: usize,
    pub review: usize,
}

/// A repo the shot starts with. Its files are `<shot>/seed/<name>/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSpec {
    pub name: String,
    /// The subject of the one commit that holds the seed.
    pub message: String,
    /// (name, email) of that commit.
    pub author: (String, String),
    /// Gives the repo a bare `origin` with `main` pushed, for a shot that shows a remote.
    pub origin: bool,
    /// Specialists the repo offers orchestrated tasks; empty leaves the roster alone.
    pub roster: Vec<String>,
}

/// Where an official shot's media goes. A shot without `[output]` has no home: the caller names a folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// Relative to the repo root.
    pub dir: Option<String>,
    /// File stem of the video; the shot folder's name when unset.
    pub stem: Option<String>,
    /// File stem of the still; the video's stem when unset.
    pub still: Option<String>,
    pub formats: Vec<Format>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Webm,
    Gif,
    Png,
    Mp4,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Webm => "webm",
            Format::Gif => "gif",
            Format::Png => "png",
            Format::Mp4 => "mp4",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaSpec {
    pub name: String,
    pub commit_name: String,
    pub commit_email: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub key: String,
    /// Sent as the spawn prompt; the demo agent finds its scenario by this exact text.
    pub task: String,
    pub at: Duration,
    /// Set for an orchestrated task: spawned as an orchestrator with this role.
    pub kind: Option<Orchestration>,
    /// The repo names it starts with; the first `[[repo]]` when the shot names none.
    pub repos: Vec<String>,
    /// A `[[release]]` key; `None` is the backlog.
    pub release: Option<String>,
}

/// The orchestrator types a task can be spawned as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orchestration {
    Direct,
    Structured,
}

impl Orchestration {
    /// The role the daemon knows it by.
    pub fn as_str(self) -> &'static str {
        match self {
            Orchestration::Direct => "direct",
            Orchestration::Structured => "structured",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    /// Its place in the file, from 1: how errors name it, since cues are played sorted by time.
    pub n: usize,
    pub at: Duration,
    pub action: Action,
    /// The target's status the cue needs before it fires; a mismatch stops the recording.
    pub expect: Option<Status>,
}

/// What a cue acts on: a task, or with `#<n>` the specialist running step n of its breakdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub key: String,
    pub step: Option<u32>,
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.step {
            Some(n) => write!(f, "{}#{n}", self.key),
            None => f.write_str(&self.key),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelTab {
    Terminal,
    Diff,
    Breakdown,
}

impl PanelTab {
    pub fn as_str(self) -> &'static str {
        match self {
            PanelTab::Terminal => "terminal",
            PanelTab::Diff => "diff",
            PanelTab::Breakdown => "breakdown",
        }
    }
}

/// Which releases the board shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    All,
    Backlog,
    /// A release key.
    Release(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Answer {
        task: Target,
        text: String,
    },
    FollowUp {
        task: Target,
        text: String,
    },
    Panel {
        task: Target,
        tab: PanelTab,
    },
    PanelClose,
    /// Approves the breakdown version waiting for a decision.
    Approve {
        task: Target,
    },
    RequestChanges {
        task: Target,
        text: String,
    },
    CreateRelease(ReleaseSpec),
    /// `release` is a release key; `None` is the backlog.
    SetRelease {
        task: Target,
        release: Option<String>,
    },
    Stop {
        task: Target,
    },
    Archive {
        task: Target,
    },
    CreatePersona {
        key: String,
        persona: PersonaSpec,
    },
    /// A GUI view, and for Management one of its tabs.
    View {
        view: String,
        tab: Option<String>,
    },
    Filter(Filter),
    /// A persona key, shown in the GUI's Personas tab.
    Persona {
        key: String,
    },
    GuiRestart,
}

impl Action {
    /// Its key in `shot.toml`, for errors.
    pub fn name(&self) -> &'static str {
        match self {
            Action::Answer { .. } => "answer",
            Action::FollowUp { .. } => "follow_up",
            Action::Panel { .. } => "panel",
            Action::PanelClose => "panel_close",
            Action::Approve { .. } => "approve",
            Action::RequestChanges { .. } => "request_changes",
            Action::CreateRelease(_) => "create_release",
            Action::SetRelease { .. } => "set_release",
            Action::Stop { .. } => "stop",
            Action::Archive { .. } => "archive",
            Action::CreatePersona { .. } => "create_persona",
            Action::View { .. } => "view",
            Action::Filter(_) => "filter",
            Action::Persona { .. } => "persona",
            Action::GuiRestart => "gui_restart",
        }
    }

    pub fn target(&self) -> Option<&Target> {
        match self {
            Action::Answer { task, .. }
            | Action::FollowUp { task, .. }
            | Action::Panel { task, .. }
            | Action::Approve { task }
            | Action::RequestChanges { task, .. }
            | Action::SetRelease { task, .. }
            | Action::Stop { task }
            | Action::Archive { task } => Some(task),
            _ => None,
        }
    }

    /// Whether the action is the whole task's (its breakdown, release or folder), so a step cannot be its target.
    fn task_only(&self) -> bool {
        matches!(
            self,
            Action::Approve { .. }
                | Action::RequestChanges { .. }
                | Action::SetRelease { .. }
                | Action::Archive { .. }
        )
    }
}

/// The GUI's views, as cue lines name them.
const VIEWS: [&str; 5] = ["kanban", "tasks", "releases", "management", "settings"];
/// The Management view's tabs, as cue lines name them.
const MANAGEMENT_TABS: [&str; 9] = [
    "overview",
    "products",
    "personas",
    "instructions",
    "agent_settings",
    "orchestrators",
    "specialists",
    "skills",
    "workflow",
];
/// Words a `filter` cue uses for itself, so no release can be called that.
const RESERVED_RELEASE_KEYS: [&str; 2] = ["all", "backlog"];

/// The status word `expect` uses, as the daemon spells it.
pub fn status_word(s: Status) -> &'static str {
    match s {
        Status::Queued => "queued",
        Status::Working => "working",
        Status::NeedsYou => "needs_you",
        Status::Talking => "talking",
        Status::Review => "review",
        Status::Stopped => "stopped",
    }
}

fn parse_status(word: &str) -> Option<Status> {
    [
        Status::Queued,
        Status::Working,
        Status::NeedsYou,
        Status::Talking,
        Status::Review,
        Status::Stopped,
    ]
    .into_iter()
    .find(|s| status_word(*s) == word)
}

/// `YYYY-MM-DD` of a day offset (`"+11d"`, `"-3d"`, `"0d"`) from `today`, a day number from
/// `fleet_api::today`.
pub fn resolve_day(offset: &str, today: i64) -> Result<String, String> {
    let bad = || format!("day offset {offset:?} is not like \"+3d\", \"-2d\" or \"0d\"");
    let n = offset.strip_suffix('d').ok_or_else(bad)?;
    let (sign, digits) = match n.as_bytes().first() {
        Some(b'+') => (1, &n[1..]),
        Some(b'-') => (-1, &n[1..]),
        _ => (1, n),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    let days: i64 = digits.parse().map_err(|_| bad())?;
    let day = today.checked_add(sign * days).ok_or_else(bad)?;
    Ok(format_day(day))
}

/// Today's day number on the local clock, as the GUI counts it, so a release the director creates spans the days
/// the GUI calls today and after.
pub fn local_today() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    fleet_api::today(now, utc_offset(now))
}

/// Seconds east of UTC on the local clock at `at`, read for that instant so DST follows the date. 0 (UTC) when the
/// system cannot say. The GUI has the same function: fleet-api is types only, so it cannot hold a libc call both
/// could share.
fn utc_offset(at: i64) -> i64 {
    let t = at as libc::time_t;
    let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
    // SAFETY: `localtime_r` writes only the `tm` it is given (unlike `localtime`, no shared buffer), and on success
    // it has filled it.
    unsafe {
        if libc::localtime_r(&t, tm.as_mut_ptr()).is_null() {
            return 0;
        }
        tm.assume_init().tm_gmtoff
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTimeline {
    product: String,
    #[serde(default)]
    repo: Vec<RawRepo>,
    persona: PersonaSpec,
    length: String,
    still_at: String,
    #[serde(default = "default_size")]
    size: String,
    output: Option<RawOutput>,
    #[serde(default)]
    task: Vec<RawTask>,
    /// Read one by one, so an error names the cue.
    #[serde(default)]
    cue: Vec<toml::Value>,
    #[serde(default)]
    pacing: RawPacing,
    #[serde(default)]
    release: Vec<ReleaseSpec>,
    #[serde(default)]
    specialist: Vec<SpecialistSpec>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawPacing {
    #[serde(default)]
    panel_sequence: bool,
    still: Option<StillCounts>,
}

fn default_size() -> String {
    "1400x800".into()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOutput {
    dir: Option<String>,
    stem: Option<String>,
    still: Option<String>,
    #[serde(default = "default_formats")]
    formats: Vec<Format>,
}

fn default_formats() -> Vec<Format> {
    vec![Format::Mp4, Format::Png]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRepo {
    name: String,
    message: String,
    author: Option<RawAuthor>,
    #[serde(default)]
    origin: bool,
    #[serde(default)]
    roster: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAuthor {
    name: String,
    email: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTask {
    key: String,
    task: String,
    at: String,
    kind: Option<String>,
    repos: Option<Vec<String>>,
    release: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCue {
    at: String,
    expect: Option<String>,
    answer: Option<RawText>,
    follow_up: Option<RawText>,
    panel: Option<RawPanel>,
    panel_close: Option<bool>,
    approve: Option<RawTarget>,
    request_changes: Option<RawText>,
    create_release: Option<ReleaseSpec>,
    set_release: Option<RawSetRelease>,
    stop: Option<RawTarget>,
    archive: Option<RawTarget>,
    create_persona: Option<RawPersona>,
    view: Option<String>,
    filter: Option<String>,
    persona: Option<String>,
    gui_restart: Option<bool>,
}

impl RawCue {
    fn actions(&self) -> usize {
        [
            self.answer.is_some(),
            self.follow_up.is_some(),
            self.panel.is_some(),
            self.panel_close.is_some(),
            self.approve.is_some(),
            self.request_changes.is_some(),
            self.create_release.is_some(),
            self.set_release.is_some(),
            self.stop.is_some(),
            self.archive.is_some(),
            self.create_persona.is_some(),
            self.view.is_some(),
            self.filter.is_some(),
            self.persona.is_some(),
            self.gui_restart.is_some(),
        ]
        .into_iter()
        .filter(|given| *given)
        .count()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTarget {
    task: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSetRelease {
    task: String,
    release: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPersona {
    key: String,
    name: String,
    commit_name: String,
    commit_email: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawText {
    task: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPanel {
    task: String,
    tab: PanelTab,
}

/// Reads `<dir>/shot.toml`.
pub fn load_shot(dir: &Path) -> Result<Timeline, String> {
    let path = dir.join("shot.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    parse_timeline(&text)
}

pub fn parse_timeline(text: &str) -> Result<Timeline, String> {
    let raw: RawTimeline = toml::from_str(text).map_err(|e| e.to_string())?;
    if raw.repo.is_empty() {
        return Err("a shot needs at least one [[repo]]".into());
    }
    let repos: Vec<RepoSpec> = raw
        .repo
        .into_iter()
        .map(|r| RepoSpec {
            name: r.name,
            message: r.message,
            author: r.author.map_or_else(
                || ("Baker".into(), "baker@pie.example".into()),
                |a| (a.name, a.email),
            ),
            origin: r.origin,
            roster: r.roster,
        })
        .collect();
    let repo_names: Vec<String> = repos.iter().map(|r| r.name.clone()).collect();

    let mut release_keys = HashSet::new();
    for r in &raw.release {
        check_release(r, &mut release_keys).map_err(|e| format!("release {}: {e}", r.key))?;
    }
    // Setup creates the shot's releases; a task cannot start in one a cue makes later.
    let setup_releases = release_keys.clone();

    let mut tasks = Vec::new();
    for p in raw.task {
        let err = |e: String| format!("task {}: {e}", p.key);
        let kind = match p.kind.as_deref() {
            None => None,
            Some("direct") => Some(Orchestration::Direct),
            Some("structured") => Some(Orchestration::Structured),
            Some(other) => {
                return Err(err(format!(
                    "unknown kind {other:?}; use direct or structured"
                )))
            }
        };
        let task_repos = p.repos.unwrap_or_else(|| vec![repo_names[0].clone()]);
        if task_repos.is_empty() {
            return Err(err("repos is empty".into()));
        }
        if let Some(r) = task_repos.iter().find(|r| !repo_names.contains(r)) {
            return Err(err(format!("no repo {r:?}")));
        }
        if let Some(r) = p
            .release
            .as_ref()
            .filter(|r| !setup_releases.contains(r.as_str()))
        {
            return Err(err(format!("no release {r:?} among the [[release]]s")));
        }
        tasks.push(Task {
            at: parse_duration(&p.at).map_err(err)?,
            key: p.key,
            task: p.task,
            kind,
            repos: task_repos,
            release: p.release,
        });
    }
    let mut seen = HashSet::new();
    if let Some(p) = tasks.iter().find(|p| !seen.insert(p.key.as_str())) {
        return Err(format!("duplicate task key {:?}", p.key));
    }

    // Every key a cue makes, gathered first so a cue may name one made by a cue written after it.
    let mut raw_cues = Vec::new();
    let mut persona_keys = HashSet::new();
    for (i, value) in raw.cue.into_iter().enumerate() {
        let n = i + 1;
        let c: RawCue = value.try_into().map_err(|e| format!("cue {n}: {e}"))?;
        if let Some(r) = &c.create_release {
            check_release(r, &mut release_keys)
                .map_err(|e| format!("cue {n}: release {}: {e}", r.key))?;
        }
        if let Some(p) = &c.create_persona {
            if !persona_keys.insert(p.key.clone()) {
                return Err(format!("cue {n}: duplicate persona key {:?}", p.key));
            }
        }
        raw_cues.push((n, c));
    }
    let mut cues = Vec::new();
    for (n, c) in raw_cues {
        let cue = parse_cue(c, &tasks, &release_keys, &persona_keys)
            .map_err(|e| format!("cue {n}: {e}"))?;
        cues.push(Cue { n, ..cue });
    }
    // Stable, so cues at the same instant keep the order they were written in.
    cues.sort_by_key(|c| c.at);
    let length = parse_duration(&raw.length).map_err(|e| format!("length: {e}"))?;
    let still_at = parse_duration(&raw.still_at).map_err(|e| format!("still_at: {e}"))?;
    if still_at >= length {
        return Err("still_at must be before length".into());
    }
    let (w, h) = raw.size.split_once('x').unwrap_or_default();
    if w.parse::<u32>().is_err() || h.parse::<u32>().is_err() {
        return Err(format!("size {:?} is not WxH", raw.size));
    }
    if raw.output.as_ref().is_some_and(|o| o.formats.is_empty()) {
        return Err("output.formats needs at least one format".into());
    }
    let output = raw.output.map_or_else(
        || Output {
            formats: default_formats(),
            ..Output::default()
        },
        |o| Output {
            dir: o.dir,
            stem: o.stem,
            still: o.still,
            formats: o.formats,
        },
    );
    Ok(Timeline {
        product: raw.product,
        repos,
        persona: raw.persona,
        length,
        still_at,
        size: raw.size,
        output,
        tasks,
        cues,
        pacing: Pacing {
            panel_sequence: raw.pacing.panel_sequence,
            still: raw.pacing.still,
        },
        releases: raw.release,
        specialists: raw.specialist,
    })
}

/// Checks a release's key is new and its days are offsets; adds the key to `keys`.
fn check_release(r: &ReleaseSpec, keys: &mut HashSet<String>) -> Result<(), String> {
    if RESERVED_RELEASE_KEYS.contains(&r.key.as_str()) {
        return Err(format!("release key {:?} is reserved", r.key));
    }
    if !keys.insert(r.key.clone()) {
        return Err(format!("duplicate release key {:?}", r.key));
    }
    resolve_day(&r.start, 0)?;
    resolve_day(&r.target, 0)?;
    Ok(())
}

/// One cue, checked against the shot's tasks and the release and persona keys it may name. Its `n` is the
/// caller's to fill in.
fn parse_cue(
    c: RawCue,
    tasks: &[Task],
    releases: &HashSet<String>,
    personas: &HashSet<String>,
) -> Result<Cue, String> {
    let at = parse_duration(&c.at)?;
    if c.actions() != 1 || c.panel_close == Some(false) || c.gui_restart == Some(false) {
        return Err(
            "exactly one of answer, follow_up, panel, panel_close, approve, request_changes, \
             create_release, set_release, stop, archive, create_persona, view, filter, persona, gui_restart"
                .into(),
        );
    }
    let target = |raw: &str| parse_target(raw, tasks);
    let release = |key: &str| {
        if releases.contains(key) {
            Ok(key.to_string())
        } else {
            Err(format!("no release {key:?}"))
        }
    };
    let action = if let Some(a) = c.answer {
        Action::Answer {
            task: target(&a.task)?,
            text: a.text,
        }
    } else if let Some(f) = c.follow_up {
        Action::FollowUp {
            task: target(&f.task)?,
            text: f.text,
        }
    } else if let Some(p) = c.panel {
        Action::Panel {
            task: target(&p.task)?,
            tab: p.tab,
        }
    } else if c.panel_close.is_some() {
        Action::PanelClose
    } else if let Some(a) = c.approve {
        Action::Approve {
            task: target(&a.task)?,
        }
    } else if let Some(r) = c.request_changes {
        Action::RequestChanges {
            task: target(&r.task)?,
            text: r.text,
        }
    } else if let Some(r) = c.create_release {
        Action::CreateRelease(r)
    } else if let Some(s) = c.set_release {
        Action::SetRelease {
            task: target(&s.task)?,
            release: match s.release.as_str() {
                "backlog" => None,
                key => Some(release(key)?),
            },
        }
    } else if let Some(s) = c.stop {
        Action::Stop {
            task: target(&s.task)?,
        }
    } else if let Some(a) = c.archive {
        Action::Archive {
            task: target(&a.task)?,
        }
    } else if let Some(p) = c.create_persona {
        Action::CreatePersona {
            key: p.key,
            persona: PersonaSpec {
                name: p.name,
                commit_name: p.commit_name,
                commit_email: p.commit_email,
            },
        }
    } else if let Some(v) = c.view {
        parse_view(&v)?
    } else if let Some(f) = c.filter {
        Action::Filter(match f.as_str() {
            "all" => Filter::All,
            "backlog" => Filter::Backlog,
            key => Filter::Release(release(key)?),
        })
    } else if let Some(key) = c.persona {
        if !personas.contains(&key) {
            return Err(format!("no persona {key:?} among the create_persona cues"));
        }
        Action::Persona { key }
    } else {
        Action::GuiRestart
    };
    if let Some(t) = action
        .target()
        .filter(|t| t.step.is_some() && action.task_only())
    {
        return Err(format!("{} takes a task, not a step ({t})", action.name()));
    }
    let expect = match c.expect {
        None => None,
        Some(_) if action.target().is_none() => {
            return Err(format!(
                "expect needs a task to check, and {} has none",
                action.name()
            ))
        }
        Some(word) => Some(parse_status(&word).ok_or_else(|| {
            format!(
                "unknown status {word:?}; use queued, working, needs_you, talking, review or stopped"
            )
        })?),
    };
    Ok(Cue {
        n: 0,
        at,
        action,
        expect,
    })
}

/// `"<key>"` or `"<key>#<n>"`, the specialist of step n; a step needs an orchestrated task.
fn parse_target(raw: &str, tasks: &[Task]) -> Result<Target, String> {
    let (key, step) = match raw.split_once('#') {
        None => (raw, None),
        Some((key, n)) => {
            let n: u32 = n
                .parse()
                .map_err(|_| format!("{raw:?} is not <task> or <task>#<step>"))?;
            if n == 0 {
                return Err(format!("step 0 of {key}: steps count from 1"));
            }
            (key, Some(n))
        }
    };
    let Some(task) = tasks.iter().find(|t| t.key == key) else {
        return Err(format!("no task {key:?}"));
    };
    if step.is_some() && task.kind.is_none() {
        return Err(format!("task {key} has no steps: it is not orchestrated"));
    }
    Ok(Target {
        key: key.to_string(),
        step,
    })
}

/// `"<view>"`, or `"management <tab>"`.
fn parse_view(raw: &str) -> Result<Action, String> {
    let mut words = raw.split_whitespace();
    let (Some(view), tab, None) = (words.next(), words.next(), words.next()) else {
        return Err(format!("view {raw:?} is not <view> [<tab>]"));
    };
    if !VIEWS.contains(&view) {
        return Err(format!("unknown view {view:?}; use {}", VIEWS.join(", ")));
    }
    match tab {
        Some(tab) if view != "management" => Err(format!("view {view} has no tabs ({tab:?})")),
        Some(tab) if !MANAGEMENT_TABS.contains(&tab) => Err(format!(
            "unknown management tab {tab:?}; use {}",
            MANAGEMENT_TABS.join(", ")
        )),
        _ => Ok(Action::View {
            view: view.to_string(),
            tab: tab.map(String::from),
        }),
    }
}

impl Timeline {
    /// The resolved settings the recorder reads, one `KEY value` per line. `name` is the shot folder's.
    pub fn describe(&self, name: &str) -> String {
        let stem = self.output.stem.as_deref().unwrap_or(name);
        let still = self.output.still.as_deref().unwrap_or(stem);
        let formats: Vec<&str> = self.output.formats.iter().map(|f| f.as_str()).collect();
        format!(
            "NAME {name}\nSIZE {}\nLENGTH {:.3}\nSTILL_AT {:.3}\nOUT_DIR {}\nSTEM {stem}\nSTILL {still}\nFORMATS {}\n",
            self.size,
            self.length.as_secs_f64(),
            self.still_at.as_secs_f64(),
            self.output.dir.as_deref().unwrap_or(""),
            formats.join(" ")
        )
    }
}

fn secs(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Queued,
    Working,
    NeedsYou,
    Review,
}

/// What one task does on the board, as the scenario and the cues together would drive it.
#[derive(Default)]
struct Run {
    /// Column changes: (time, what).
    changes: Vec<(Duration, String)>,
    states: Vec<(Duration, State)>,
    /// The Diff tab should show the recipe as it ends up, so it is timed against the card's latest write.
    last_write: Option<Duration>,
    /// The daemon drives an orchestrated card after it appears, so `states` stops there; `expect` checks the rest
    /// at run time.
    orchestrated: bool,
}

impl Run {
    fn state_at(&self, t: Duration) -> Option<State> {
        self.states
            .iter()
            .take_while(|(at, _)| *at <= t)
            .last()
            .map(|(_, s)| *s)
    }
}

fn is_wait(kind: &StepKind) -> bool {
    matches!(
        kind,
        StepKind::Permission { .. } | StepKind::Question { .. }
    )
}

/// Walks the main arc, then each follow-up the cues send. The agent starts `AGENT_STARTUP` after the spawn; after
/// an answer or a follow-up it reacts at once, so those arcs run from the cue's own time.
fn simulate(t: &Timeline, p: &Task, scenario: &Scenario, bad: &mut Vec<String>) -> Run {
    let mut run = Run::default();
    run.changes.push((p.at, format!("{} appears", p.key)));
    run.states.push((p.at, State::Queued));
    let mut clock = p.at + AGENT_STARTUP;
    run.states.push((clock, State::Working));

    let mut answers = t.cues.iter().filter_map(|c| match &c.action {
        Action::Answer { task, .. } if task.key == p.key => Some(c.at),
        _ => None,
    });
    for step in &scenario.main {
        clock += step.after;
        match &step.kind {
            k if is_wait(k) => {
                run.changes.push((clock, format!("{} needs you", p.key)));
                run.states.push((clock, State::NeedsYou));
                let Some(at) = answers.next() else {
                    bad.push(format!(
                        "task {}: the wait at {} s is never answered",
                        p.key,
                        secs(clock)
                    ));
                    return run;
                };
                if at < clock + MIN_WAIT {
                    bad.push(format!(
                        "task {}: answered at {} s, only {} s after its wait begins ({} s); needs 2 s",
                        p.key,
                        secs(at),
                        secs(at.saturating_sub(clock)),
                        secs(clock)
                    ));
                }
                clock = at.max(clock);
                run.changes.push((clock, format!("{} answered", p.key)));
                run.states.push((clock, State::Working));
            }
            StepKind::Write { .. } => {
                run.last_write = run.last_write.max(Some(clock));
            }
            StepKind::Stop => {
                run.changes
                    .push((clock, format!("{} reaches Review", p.key)));
                run.states.push((clock, State::Review));
            }
            _ => {}
        }
    }
    if answers.next().is_some() {
        bad.push(format!("task {}: more answers than waits", p.key));
    }

    for c in &t.cues {
        let Action::FollowUp { task, text } = &c.action else {
            continue;
        };
        if task.key != p.key {
            continue;
        }
        run.changes.push((c.at, format!("{} follows up", p.key)));
        run.states.push((c.at, State::Working));
        let Some(arc) = scenario
            .follow_ups
            .iter()
            .find(|f| f.on.trim() == text.trim())
        else {
            bad.push(format!("task {}: no follow-up arc for {text:?}", p.key));
            continue;
        };
        let mut clock = c.at;
        for step in &arc.steps {
            clock += step.after;
            match &step.kind {
                StepKind::Write { .. } => {
                    run.last_write = run.last_write.max(Some(clock));
                }
                StepKind::Stop => {
                    run.changes
                        .push((clock, format!("{} reaches Review", p.key)));
                    run.states.push((clock, State::Review));
                }
                _ => {}
            }
        }
    }
    run.states.sort_by_key(|(at, _)| *at);
    run
}

/// An orchestrated task as pacing sees it: a card that appears.
fn appears(p: &Task) -> Run {
    Run {
        changes: vec![(p.at, format!("{} appears", p.key))],
        states: vec![(p.at, State::Queued)],
        last_write: None,
        orchestrated: true,
    }
}

/// The pacing rules as a list of violations; empty means the shot is as calm as the spec asks.
pub fn check_pacing(t: &Timeline, scenarios: &[(PathBuf, Scenario)]) -> Vec<String> {
    let mut bad = Vec::new();

    if t.length < LENGTH_RANGE.0 || t.length > LENGTH_RANGE.1 {
        bad.push(format!("length {} s is outside 15-30 s", secs(t.length)));
    }

    let mut runs: Vec<(&Task, Run)> = Vec::new();
    for p in &t.tasks {
        match find(scenarios, &p.task) {
            // Its scenario is still needed: the orchestrator's prompt is how the demo agent finds it.
            Some(_) if p.kind.is_some() => runs.push((p, appears(p))),
            Some(scenario) => runs.push((p, simulate(t, p, scenario, &mut bad))),
            None => bad.push(format!("task {}: no scenario for task {:?}", p.key, p.task)),
        }
    }

    let mut changes: Vec<&(Duration, String)> = runs.iter().flat_map(|(_, r)| &r.changes).collect();
    changes.sort_by_key(|(at, _)| *at);
    for pair in changes.windows(2) {
        if pair[1].0 < pair[0].0 + MIN_CHANGE_GAP - EPS {
            bad.push(format!(
                "board changes at {} s ({}) and {} s ({}) are less than 1.5 s apart",
                secs(pair[0].0),
                pair[0].1,
                secs(pair[1].0),
                pair[1].1
            ));
        }
    }

    let panel: Vec<&Cue> = t
        .cues
        .iter()
        .filter(|c| matches!(c.action, Action::Panel { .. } | Action::PanelClose))
        .collect();
    if t.pacing.panel_sequence {
        match panel.as_slice() {
            [a, b, c] => match (&a.action, &b.action, &c.action) {
                (
                    Action::Panel {
                        task: p1,
                        tab: PanelTab::Terminal,
                    },
                    Action::Panel {
                        task: p2,
                        tab: PanelTab::Diff,
                    },
                    Action::PanelClose,
                ) if p1 == p2 => {
                    if let Some((_, run)) = runs
                        .iter()
                        .find(|(p, r)| p.key == p1.key && p1.step.is_none() && !r.orchestrated)
                    {
                        if run.state_at(a.at) != Some(State::Working) {
                            bad.push(format!(
                                "the Terminal tab opens on {p1} at {} s, when it is not Working",
                                secs(a.at)
                            ));
                        }
                        match run.last_write {
                            Some(w) if b.at >= w => {}
                            other => bad.push(format!(
                                "the Diff tab opens on {p1} at {} s, before its last recipe write ({})",
                                secs(b.at),
                                other.map_or("never".into(), |w| format!("{} s", secs(w)))
                            )),
                        }
                    }
                    for pair in [(a, b), (b, c)] {
                        if pair.1.at < pair.0.at + MIN_PANEL_STEP - EPS {
                            bad.push(format!(
                                "panel cues at {} s and {} s are less than 1.5 s apart",
                                secs(pair.0.at),
                                secs(pair.1.at)
                            ));
                        }
                    }
                }
                _ => bad.push(PANEL_SEQUENCE.into()),
            },
            _ => bad.push(PANEL_SEQUENCE.into()),
        }
    }
    for cue in &panel {
        if let Some((at, what)) = changes
            .iter()
            .find(|(at, _)| at.abs_diff(cue.at) + EPS < MIN_PANEL_GAP)
            .map(|c| (c.0, &c.1))
        {
            bad.push(format!(
                "panel cue at {} s is less than 0.75 s from the board change at {} s ({what})",
                secs(cue.at),
                secs(at)
            ));
        }
    }

    let count = |state: State| {
        runs.iter()
            .filter(|(_, r)| r.state_at(t.still_at) == Some(state))
            .count()
    };
    if let Some(want) = t.pacing.still {
        let (w, n, r) = (
            count(State::Working),
            count(State::NeedsYou),
            count(State::Review),
        );
        if (w, n, r) != (want.working, want.needs_you, want.review) {
            bad.push(format!(
                "still_at {} s shows {w} Working, {n} Needs you, {r} Review; it wants {}, {}, {}",
                secs(t.still_at),
                want.working,
                want.needs_you,
                want.review
            ));
        }
    }
    if let Some((at, what)) = changes
        .iter()
        .find(|(at, _)| at.abs_diff(t.still_at) + EPS < MIN_PANEL_GAP)
        .map(|c| (c.0, &c.1))
    {
        bad.push(format!(
            "still_at {} s is less than 0.75 s from the board change at {} s ({what})",
            secs(t.still_at),
            secs(at)
        ));
    }
    bad
}

const PANEL_SEQUENCE: &str =
    "the side panel needs exactly one sequence: terminal, diff, close, on one task";
