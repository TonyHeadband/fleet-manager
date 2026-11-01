//! The merge watcher: asks GitHub which PRs a task's branches have, and git which repos the task changed, so a task
//! is done when every changed repo's PR has merged. Only repos with a URL are asked about; a task with none finishes
//! through Mark done instead.
use crate::agents::{AgentService, Clock, ServiceError};
use crate::forge::log::{said, CallCtx, PrCalls};
use crate::forge::{self, Forge, ForgeConfig, PrInfo, RepoRef};
use crate::personas::Personas;
use crate::repos::Repos;
use crate::store::Store;
use crate::supervisor;
use fleet_api::{
    task_id, task_outcome, AgentId, AgentKind, EventKind, PrState, Status, Task, TaskEventKind,
    TaskOutcome, TaskPr, TaskState,
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};

/// How often every open task is checked, besides the checks a stop, a Review or a refresh asks for.
pub const MERGE_CHECK_EVERY: Duration = Duration::from_secs(600);

/// How long one PR list may take. Archive waits on it per repo and the queue runs one check at a time, so a
/// hung call must give up well before git's own remote timeout.
pub const MERGE_CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// Refusal of Mark done on a task GitHub can tell about.
pub const HAS_GITHUB_REPO: &str = "this task has a GitHub repo; it is done when its PR merges";

fn task_pr(repo: &str, p: PrInfo) -> TaskPr {
    TaskPr {
        url: p.url,
        repo: repo.to_string(),
        state: p.state,
        merged_at: p.merged_at,
    }
}

/// A task the watcher still asks about: not archived, not done.
fn watched(t: &Task) -> bool {
    t.state != TaskState::Archived && t.done_at.is_none() && !t.done_by_hand
}

/// Each persona's forge client, or why there is none, built once per pass.
type Forges = HashMap<String, Result<Arc<dyn Forge>, String>>;

/// One repo of a task the watcher asks about.
struct Ask {
    repo: String,
    branch: String,
    base: String,
    url: String,
    /// Where the branch may live: the repo's local path, then the persona's clone of its URL.
    checkouts: Vec<std::path::PathBuf>,
}

#[derive(Clone)]
pub(crate) struct MergeChecker {
    store: Arc<Store>,
    personas: Arc<Personas>,
    repos: Arc<Repos>,
    forge: Arc<ForgeConfig>,
    clock: Arc<dyn Clock>,
    timeout: Duration,
    runs: Arc<AtomicUsize>,
}

impl MergeChecker {
    pub(crate) fn new(
        store: Arc<Store>,
        personas: Arc<Personas>,
        repos: Arc<Repos>,
        forge: Arc<ForgeConfig>,
        clock: Arc<dyn Clock>,
        timeout: Duration,
        runs: Arc<AtomicUsize>,
    ) -> MergeChecker {
        MergeChecker {
            store,
            personas,
            repos,
            forge,
            clock,
            timeout,
            runs,
        }
    }

    /// The tasks whose `prs` changed while they were checked, to check again.
    async fn check_all(&self) -> Vec<AgentId> {
        match self.store.tasks().await {
            Ok(tasks) => {
                let mut stale = Vec::new();
                let mut forges = Forges::new();
                for t in tasks.iter().filter(|t| watched(t)) {
                    if self.check(t, &mut forges).await == Checked::Stale {
                        stale.push(t.id.clone());
                    }
                }
                stale
            }
            Err(e) => {
                warn!(error = %e, "cannot list tasks to check their PRs");
                vec![]
            }
        }
    }

    pub(crate) async fn check_one(&self, id: &AgentId) -> Checked {
        match self.store.task(id).await {
            Ok(Some(t)) if watched(&t) => self.check(&t, &mut Forges::new()).await,
            Ok(_) => Checked::Done,
            Err(e) => {
                warn!(task = %id.0, error = %e, "cannot read the task to check its PRs");
                Checked::Failed(format!("cannot read the task: {e}"))
            }
        }
    }

    /// The task's repos with a URL, each with where its branch may live.
    async fn asks(&self, t: &Task) -> Result<Vec<Ask>, String> {
        let mut asks = Vec::new();
        for w in &t.repos {
            let row = self
                .store
                .repo(&t.product, &w.repo)
                .await
                .map_err(|e| format!("cannot read repo {}: {e}", w.repo))?;
            if let Some(row) = row {
                if let Some(url) = row.url {
                    let mut checkouts: Vec<_> = row.path.into_iter().collect();
                    checkouts.push(self.repos.clone_dir(&t.persona, &t.product, &w.repo));
                    asks.push(Ask {
                        repo: w.repo.clone(),
                        branch: w.branch.clone(),
                        base: w.base.clone(),
                        url,
                        checkouts,
                    });
                }
            }
        }
        Ok(asks)
    }

    /// Whether the task's branch has commits beyond its start in a repo with a URL: a PR could exist there. A
    /// branch git cannot count counts as changed only if a check saw it changed.
    pub(crate) async fn changed_github_repo(&self, t: &Task) -> bool {
        let Ok(asks) = self.asks(t).await else {
            return t.repos.iter().any(|w| w.changed);
        };
        for ask in &asks {
            let changed = match changed_in(ask).await {
                Count::Changed(c) => c,
                _ => t.repos.iter().any(|w| w.repo == ask.repo && w.changed),
            };
            if changed {
                return true;
            }
        }
        false
    }

    /// Replaces the task's `prs` with what GitHub says now and each repo's `changed` with what git says, and stamps
    /// `done_at` once every changed repo's PR has merged. Any failure (persona logged out, GitHub failing or slow, a branch git cannot count) changes nothing: a wrong count is worse than a late one.
    async fn check(&self, t: &Task, forges: &mut Forges) -> Checked {
        let asks = match self.asks(t).await {
            Ok(a) => a,
            Err(e) => {
                warn!(task = %t.id.0, error = %e, "cannot read the repo to check its PRs");
                return Checked::Failed(e);
            }
        };
        if asks.is_empty() {
            return Checked::Done;
        }
        if !forges.contains_key(&t.persona) {
            let forge = self.forge_of(&t.persona).await;
            forges.insert(t.persona.clone(), forge);
        }
        let forge = match &forges[&t.persona] {
            Ok(f) => f,
            Err(e) => {
                warn!(task = %t.id.0, error = %e, "cannot check the task's PRs: its persona's GitHub login is not good");
                return Checked::Failed(e.clone());
            }
        };
        let mut prs = Vec::new();
        let mut changed_repos = Vec::new();
        for ask in &asks {
            let repo = &ask.repo;
            let Some(repo_ref) = RepoRef::from_url(&ask.url) else {
                warn!(task = %t.id.0, repo = %repo, "cannot check PRs: the repo URL names no GitHub owner and repo");
                return Checked::Failed(format!(
                    "the URL of {repo} names no GitHub owner and repo"
                ));
            };
            let counted = match changed_in(ask).await {
                Count::Unknown => {
                    warn!(task = %t.id.0, repo = %repo, "cannot count the branch's commits: no checkout of the repo, or git cannot run");
                    return Checked::Failed(format!("cannot count the commits of {repo}"));
                }
                c => c,
            };
            let calls = PrCalls {
                store: &self.store,
                ctx: CallCtx {
                    persona: t.persona.clone(),
                    task: Some(t.id.clone()),
                    agent: None,
                },
                forge: forge.clone(),
                repo: repo_ref,
            };
            let branch = ask.branch.clone();
            match calls.call("prs_of", move |f, r| f.prs_of(r, &branch)).await {
                Ok(found) => {
                    let found: Vec<TaskPr> = found.into_iter().map(|p| task_pr(repo, p)).collect();
                    let changed = match counted {
                        Count::Changed(c) => c,
                        // The branch or its start is gone from every checkout (a merged branch deleted by hand).
                        // GitHub refuses a PR with no commits, so a PR proves a change.
                        _ => {
                            let before = t.repos.iter().any(|w| w.repo == *repo && w.changed);
                            before || !found.is_empty()
                        }
                    };
                    changed_repos.push((repo.clone(), changed));
                    prs.extend(found);
                }
                Err(e) => {
                    warn!(task = %t.id.0, repo = %repo, error = %e, "cannot check the task's PRs");
                    return Checked::Failed(said(&e));
                }
            }
        }
        let now = self.clock.now();
        let mut wrote_any = false;
        let mut finished = false;
        let mut stale = false;
        let mut newly_merged = Vec::new();
        let wrote = self
            .store
            .update_task(&t.id, |x| {
                // Marked done by hand meanwhile: that stands.
                if x.done_by_hand || x.done_at.is_some() {
                    return;
                }
                // A PR recorded since the check started (`open_pr`) may be missing from a reply asked for before it
                // existed: writing this one would drop it.
                if x.prs != t.prs {
                    stale = true;
                    return;
                }
                if x.prs != prs {
                    newly_merged = prs
                        .iter()
                        .filter(|n| {
                            n.state == PrState::Merged
                                && !x
                                    .prs
                                    .iter()
                                    .any(|o| o.url == n.url && o.state == PrState::Merged)
                        })
                        .map(|n| n.url.clone())
                        .collect();
                    x.prs = prs;
                    wrote_any = true;
                }
                for w in &mut x.repos {
                    if let Some((_, c)) = changed_repos.iter().find(|(r, _)| *r == w.repo) {
                        if w.changed != *c {
                            w.changed = *c;
                            wrote_any = true;
                        }
                    }
                }
                if task_outcome(x) == TaskOutcome::Merged {
                    let last_merge = x.prs.iter().filter_map(|p| p.merged_at).max();
                    x.done_at = Some(last_merge.unwrap_or(now));
                    wrote_any = true;
                    finished = true;
                }
            })
            .await;
        if stale {
            info!(task = %t.id.0, "task's PRs changed during the check; checking again");
            return Checked::Stale;
        }
        match wrote {
            Ok(()) if wrote_any => {
                for url in newly_merged {
                    let text = format!("PR merged · {url}");
                    if let Err(e) = supervisor::route_with(
                        &self.store,
                        now,
                        &t.id,
                        TaskEventKind::PrMerged,
                        text,
                    )
                    .await
                    {
                        warn!(task = %t.id.0, error = %e, "cannot store the PR's event");
                    }
                }
                if finished {
                    info!(task = %t.id.0, "task's PRs merged");
                }
                // The record is not streamed; the lead's card is, and a client refetches the list on its events.
                let card = t.lead.clone().unwrap_or_else(|| t.id.clone());
                if let Err(e) = self.store.announce(&card).await {
                    warn!(task = %t.id.0, error = %e, "cannot announce the task's PR change");
                }
            }
            Ok(()) => {}
            Err(e) => {
                warn!(task = %t.id.0, error = %e, "cannot store the task's PRs");
                return Checked::Failed(format!("cannot store the task's PRs: {e}"));
            }
        }
        Checked::Done
    }

    /// The persona's client on its own host, or why it has none: a missing token is a logged-out GitHub.
    async fn forge_of(&self, persona: &str) -> Result<Arc<dyn Forge>, String> {
        let token = self
            .personas
            .token(persona)
            .await
            .map_err(|e| e.to_string())?;
        let host = self
            .personas
            .get(persona)
            .await
            .map_err(|e| e.to_string())?
            .host;
        Ok(Arc::from(forge::github(
            &host,
            token,
            &self.forge,
            self.timeout,
        )))
    }
}

/// What git says about a task's branch in one repo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Count {
    /// Whether the branch has commits beyond its start.
    Changed(bool),
    /// A checkout exists, but none has the branch or its start commit.
    Unresolved,
    /// No checkout of the repo exists, or git cannot run.
    Unknown,
}

/// Counts in the first checkout that can tell: the repo's base checkout and its clone both hold the branch, even
/// after archive, unless someone deleted it.
async fn changed_in(ask: &Ask) -> Count {
    let mut found = Count::Unknown;
    for dir in ask.checkouts.iter().filter(|d| d.exists()) {
        match crate::git::commits_beyond(dir, &ask.base, &ask.branch).await {
            Ok(n) => return Count::Changed(n > 0),
            Err(crate::git::GitError::Io(e)) => {
                warn!(repo = %ask.repo, error = %e, "cannot run git to count the branch's commits");
                return Count::Unknown;
            }
            Err(e) => {
                tracing::debug!(repo = %ask.repo, dir = %dir.display(), error = %e, "cannot count the branch's commits here");
                found = Count::Unresolved;
            }
        }
    }
    found
}

/// How a check of one task ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Checked {
    /// Written, nothing to write, or nothing to ask about.
    Done,
    /// Failed, and so left as it was: why.
    Failed(String),
    /// Its `prs` changed while GitHub was asked: nothing was written, and it needs checking again.
    Stale,
}

/// A queued check: every watched task, or one.
enum MergeJob {
    All,
    One(AgentId),
}

/// The one task that runs checks, one at a time, so a refresh and the timer never ask GitHub twice at once.
/// `queued` holds the jobs waiting (`""` for All), so a burst of requests for the same thing runs once.
#[derive(Clone)]
pub(crate) struct MergeQueue {
    tx: mpsc::UnboundedSender<MergeJob>,
    queued: Arc<Mutex<HashSet<String>>>,
}

impl MergeQueue {
    /// Must be called inside the tokio runtime.
    pub(crate) fn start(checker: MergeChecker) -> MergeQueue {
        let (tx, mut rx) = mpsc::unbounded_channel::<MergeJob>();
        let queued = Arc::new(Mutex::new(HashSet::new()));
        let held = queued.clone();
        // Weak, so the worker alone never keeps the channel open.
        let again = tx.downgrade();
        tokio::spawn(async move {
            while let Some(job) = rx.recv().await {
                let key = match &job {
                    MergeJob::All => String::new(),
                    MergeJob::One(id) => id.0.clone(),
                };
                // Off the queue before it runs: a request made meanwhile may follow a change this run missed.
                held.lock().expect("merge queue poisoned").remove(&key);
                let stale = match job {
                    MergeJob::All => checker.check_all().await,
                    MergeJob::One(id) => match checker.check_one(&id).await {
                        Checked::Stale => vec![id],
                        Checked::Done | Checked::Failed(_) => vec![],
                    },
                };
                checker.runs.fetch_add(1, Ordering::SeqCst);
                if let Some(tx) = again.upgrade() {
                    let queue = MergeQueue {
                        tx,
                        queued: held.clone(),
                    };
                    for id in stale {
                        queue.push(Some(id));
                    }
                }
            }
        });
        MergeQueue { tx, queued }
    }

    fn push(&self, task: Option<AgentId>) {
        let key = task.as_ref().map_or_else(String::new, |id| id.0.clone());
        if self
            .queued
            .lock()
            .expect("merge queue poisoned")
            .insert(key)
        {
            let _ = self.tx.send(task.map_or(MergeJob::All, MergeJob::One));
        }
    }
}

impl AgentService {
    /// Checks every task that is not archived and not done, now, and returns when done.
    pub async fn check_merges(&self) {
        for id in self.merge_checker().check_all().await {
            self.request_merge_check(Some(id));
        }
    }

    /// Queues a check of one task, or of every task with None, and returns at once. Must be called inside the tokio
    /// runtime.
    pub fn request_merge_check(&self, task: Option<AgentId>) {
        self.merge_queue().push(task);
    }

    /// Queued checks finished so far.
    pub fn merge_checks_run(&self) -> usize {
        self.merge_runs.load(Ordering::SeqCst)
    }

    /// Checks every task now and every `every` after, and a task whenever one of its agents reaches Stopped or
    /// Review. Must be called inside the tokio runtime, once.
    pub fn start_merge_watcher(&self, every: Duration) {
        let queue = self.merge_queue().clone();
        let timer = queue.clone();
        tokio::spawn(async move {
            loop {
                timer.push(None);
                tokio::time::sleep(every).await;
            }
        });
        let mut events = self.store.subscribe();
        let store = self.store.clone();
        tokio::spawn(async move {
            // The status each agent was last seen in: an event that only changes the activity is not a new stop.
            let mut seen: HashMap<AgentId, Status> = HashMap::new();
            loop {
                let ev = match events.recv().await {
                    Ok(ev) => ev,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        seen.clear();
                        queue.push(None);
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let (id, status) = match ev.kind {
                    EventKind::StatusChanged { id, status, .. } => (id, status),
                    EventKind::AgentRemoved { id } => {
                        seen.remove(&id);
                        continue;
                    }
                    _ => continue,
                };
                if seen.insert(id.clone(), status) == Some(status)
                    || !matches!(status, Status::Stopped | Status::Review)
                {
                    continue;
                }
                match store.snapshot().await {
                    Ok(snap) => {
                        // A supervisor conversation has no task, so no PRs to check.
                        let found = snap.agents.iter().find(|a| a.id == id);
                        if let Some(a) = found.filter(|a| a.kind != AgentKind::Supervisor) {
                            queue.push(Some(task_id(a).clone()));
                        }
                    }
                    Err(e) => warn!(error = %e, "cannot read the agent to check its task's PRs"),
                }
            }
        });
    }

    /// Marks a task with no GitHub repo done. A task with one is done only when its PR merges.
    pub async fn mark_done(&self, id: &AgentId) -> Result<Task, ServiceError> {
        let t = self.store.task(id).await?.ok_or(ServiceError::NotFound)?;
        for w in &t.repos {
            let url = self
                .store
                .repo(&t.product, &w.repo)
                .await?
                .and_then(|r| r.url);
            if url.is_some() {
                return Err(ServiceError::Refused(HAS_GITHUB_REPO.into()));
            }
        }
        let now = self.opts.clock.now();
        self.store
            .update_task(id, |x| {
                if !x.done_by_hand {
                    x.done_by_hand = true;
                    x.done_at = Some(now);
                }
            })
            .await?;
        self.log_task(id, "marked done".into()).await;
        let t = self.store.task(id).await?.ok_or(ServiceError::NotFound)?;
        let card = t.lead.clone().unwrap_or_else(|| t.id.clone());
        if let Err(e) = self.store.announce(&card).await {
            warn!(task = %id.0, error = %e, "cannot announce the task's change");
        }
        Ok(t)
    }

    pub(crate) fn merge_checker(&self) -> MergeChecker {
        MergeChecker::new(
            self.store.clone(),
            self.personas().clone(),
            self.repos.clone(),
            self.opts.forge.clone(),
            self.opts.clock.clone(),
            self.opts.merge_check_timeout,
            self.merge_runs.clone(),
        )
    }

    fn merge_queue(&self) -> &MergeQueue {
        self.merge_queue
            .get_or_init(|| MergeQueue::start(self.merge_checker()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The archive check waits on it per repo, and one hung call holds up every queued check.
    #[test]
    fn a_merge_check_gives_up_after_30_seconds() {
        let opts = crate::agents::ServiceOptions::new("http://127.0.0.1:1");
        assert_eq!(opts.merge_check_timeout, Duration::from_secs(30));
    }
}
