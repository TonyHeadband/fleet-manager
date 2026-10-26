//! The merge watcher: asks GitHub which PRs a task's branches have, so a task is done when they merge. Only repos
//! with a URL are asked about; a task with none finishes through Mark done instead.
use crate::agents::{AgentService, Clock, ServiceError};
use crate::personas::Personas;
use crate::store::Store;
use fleet_api::{
    parse_date, task_id, task_outcome, AgentId, EventKind, PrState, Status, Task, TaskOutcome,
    TaskPr, TaskState,
};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};

/// How often every open task is checked, besides the checks a stop, a Review or a refresh asks for.
pub const MERGE_CHECK_EVERY: Duration = Duration::from_secs(600);

/// Refusal of Mark done on a task GitHub can tell about.
pub const HAS_GITHUB_REPO: &str = "this task has a GitHub repo; it is done when its PR merges";

/// `<host>/<owner>/<repo>` for `gh -R`, so a GitHub Enterprise host is asked, not github.com. None for a URL that
/// does not name exactly an owner and a repo.
pub fn gh_repo(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let parts: Vec<&str> = rest.split('/').collect();
    match parts[..] {
        [host, owner, repo] if [host, owner, repo].iter().all(|p| !p.is_empty()) => {
            Some(format!("{host}/{owner}/{repo}"))
        }
        _ => None,
    }
}

/// One entry of `gh pr list --json url,state,mergedAt`.
#[derive(Deserialize)]
struct GhPr {
    url: String,
    state: String,
    #[serde(rename = "mergedAt")]
    merged_at: Option<String>,
}

/// `2026-10-26T10:00:00Z` as Unix seconds. gh prints UTC; anything else is None.
fn parse_timestamp(s: &str) -> Option<i64> {
    let (date, time) = s.split_once('T')?;
    let time = time.strip_suffix('Z')?;
    let time = time.split_once('.').map_or(time, |(t, _)| t);
    let mut hms = time.split(':').map(|p| p.parse::<i64>().ok());
    let (h, m, sec) = (hms.next()??, hms.next()??, hms.next()??);
    if hms.next().is_some() || h > 23 || m > 59 || sec > 60 {
        return None;
    }
    Some(parse_date(date)? * 86_400 + h * 3600 + m * 60 + sec)
}

fn parse_reply(repo: &str, stdout: &[u8]) -> Result<Vec<TaskPr>, String> {
    let prs: Vec<GhPr> =
        serde_json::from_slice(stdout).map_err(|e| format!("unreadable reply: {e}"))?;
    prs.into_iter()
        .map(|p| {
            let state = match p.state.to_ascii_uppercase().as_str() {
                "OPEN" => PrState::Open,
                "MERGED" => PrState::Merged,
                "CLOSED" => PrState::Closed,
                other => return Err(format!("unknown PR state {other:?}")),
            };
            Ok(TaskPr {
                url: p.url,
                repo: repo.to_string(),
                state,
                merged_at: p.merged_at.as_deref().and_then(parse_timestamp),
            })
        })
        .collect()
}

/// A task the watcher still asks about: not archived, not done.
fn watched(t: &Task) -> bool {
    t.state != TaskState::Archived && t.done_at.is_none() && !t.done_by_hand
}

#[derive(Clone)]
pub(crate) struct MergeChecker {
    store: Arc<Store>,
    personas: Arc<Personas>,
    gh: String,
    clock: Arc<dyn Clock>,
    timeout: Duration,
    runs: Arc<AtomicUsize>,
}

impl MergeChecker {
    pub(crate) fn new(
        store: Arc<Store>,
        personas: Arc<Personas>,
        gh: String,
        clock: Arc<dyn Clock>,
        timeout: Duration,
        runs: Arc<AtomicUsize>,
    ) -> MergeChecker {
        MergeChecker {
            store,
            personas,
            gh,
            clock,
            timeout,
            runs,
        }
    }

    async fn check_all(&self) {
        match self.store.tasks().await {
            Ok(tasks) => {
                for t in tasks.iter().filter(|t| watched(t)) {
                    self.check(t).await;
                }
            }
            Err(e) => warn!(error = %e, "cannot list tasks to check their PRs"),
        }
    }

    async fn check_one(&self, id: &AgentId) {
        match self.store.task(id).await {
            Ok(Some(t)) if watched(&t) => self.check(&t).await,
            Ok(_) => {}
            Err(e) => warn!(task = %id.0, error = %e, "cannot read the task to check its PRs"),
        }
    }

    /// Replaces the task's `prs` with what GitHub says now, and stamps `done_at` once they are merged. Any failure
    /// (persona logged out, gh failing or slow, a reply it cannot read) changes nothing: a wrong count is worse
    /// than a late one.
    async fn check(&self, t: &Task) {
        let mut asks = Vec::new();
        for w in &t.repos {
            match self.store.repo(&t.product, &w.repo).await {
                Ok(Some(row)) => {
                    if let Some(url) = row.url {
                        asks.push((w.repo.clone(), w.branch.clone(), url));
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    warn!(task = %t.id.0, repo = %w.repo, error = %e, "cannot read the repo to check its PRs");
                    return;
                }
            }
        }
        if asks.is_empty() {
            return;
        }
        let persona = match self.personas.ready_env(&t.persona).await {
            Ok(p) => p,
            Err(e) => {
                warn!(task = %t.id.0, error = %e, "cannot check the task's PRs: its persona is not ready");
                return;
            }
        };
        let mut prs = Vec::new();
        for (repo, branch, url) in asks {
            let Some(slug) = gh_repo(&url) else {
                warn!(task = %t.id.0, repo = %repo, "cannot check PRs: the repo URL names no GitHub owner and repo");
                return;
            };
            match self.list(&slug, &branch, &persona.vars).await {
                Ok(out) => match parse_reply(&repo, &out) {
                    Ok(found) => prs.extend(found),
                    Err(e) => {
                        warn!(task = %t.id.0, repo = %repo, error = %e, "cannot read gh pr list");
                        return;
                    }
                },
                Err(e) => {
                    warn!(task = %t.id.0, repo = %repo, error = %e, "cannot check the task's PRs");
                    return;
                }
            }
        }
        let now = self.clock.now();
        let mut changed = false;
        let mut finished = false;
        let wrote = self
            .store
            .update_task(&t.id, |x| {
                // Marked done by hand meanwhile: that stands.
                if x.done_by_hand || x.done_at.is_some() {
                    return;
                }
                if x.prs != prs {
                    x.prs = prs;
                    changed = true;
                }
                if task_outcome(x) == TaskOutcome::Merged {
                    let last_merge = x.prs.iter().filter_map(|p| p.merged_at).max();
                    x.done_at = Some(last_merge.unwrap_or(now));
                    changed = true;
                    finished = true;
                }
            })
            .await;
        match wrote {
            Ok(()) if changed => {
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
            Err(e) => warn!(task = %t.id.0, error = %e, "cannot store the task's PRs"),
        }
    }

    async fn list(
        &self,
        slug: &str,
        branch: &str,
        env: &[(String, String)],
    ) -> Result<Vec<u8>, String> {
        let mut gh = Command::new(&self.gh);
        crate::git::harden(gh.as_std_mut(), env);
        gh.args(["pr", "list", "-R", slug, "--head", branch])
            .args(["--state", "all", "--json", "url,state,mergedAt"])
            .stdin(Stdio::null())
            .kill_on_drop(true);
        match tokio::time::timeout(self.timeout, gh.output()).await {
            Ok(Ok(o)) if o.status.success() => Ok(o.stdout),
            Ok(Ok(o)) => Err(format!(
                "gh pr list failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            )),
            Ok(Err(e)) => Err(format!("gh cannot run: {e}")),
            Err(_) => Err("gh pr list timed out".into()),
        }
    }
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
        tokio::spawn(async move {
            while let Some(job) = rx.recv().await {
                let key = match &job {
                    MergeJob::All => String::new(),
                    MergeJob::One(id) => id.0.clone(),
                };
                // Off the queue before it runs: a request made meanwhile may follow a change this run missed.
                held.lock().expect("merge queue poisoned").remove(&key);
                match job {
                    MergeJob::All => checker.check_all().await,
                    MergeJob::One(id) => checker.check_one(&id).await,
                }
                checker.runs.fetch_add(1, Ordering::SeqCst);
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
        self.merge_checker().check_all().await;
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
                        if let Some(a) = snap.agents.iter().find(|a| a.id == id) {
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
            self.cfg.gh_command.clone(),
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

    #[test]
    fn gh_repo_keeps_the_host() {
        for url in [
            "https://ghe.example.com/acme/core",
            "https://ghe.example.com/acme/core.git",
            "https://ghe.example.com/acme/core/",
            "https://ghe.example.com/acme/core.git/",
        ] {
            assert_eq!(
                gh_repo(url).as_deref(),
                Some("ghe.example.com/acme/core"),
                "{url}"
            );
        }
        assert_eq!(gh_repo("https://github.com/acme"), None);
        assert_eq!(gh_repo("https://gitlab.example.com/a/b/c"), None);
    }

    #[test]
    fn timestamps_are_utc_seconds() {
        assert_eq!(
            parse_timestamp("2026-10-26T10:00:05Z"),
            Some(20752 * 86_400 + 36_005)
        );
        assert_eq!(parse_timestamp("1970-01-01T00:00:00.5Z"), Some(0));
        assert_eq!(parse_timestamp("2026-10-26 10:00:00"), None);
        assert_eq!(parse_timestamp("2026-10-26T25:00:00Z"), None);
    }

    #[test]
    fn reply_parses_every_state_and_refuses_an_unknown_one() {
        let ok = br#"[{"url":"u1","state":"OPEN","mergedAt":null},
                      {"url":"u2","state":"MERGED","mergedAt":"1970-01-01T00:01:00Z"},
                      {"url":"u3","state":"CLOSED","mergedAt":null}]"#;
        let prs = parse_reply("core", ok).unwrap();
        assert_eq!(
            prs.iter().map(|p| p.state).collect::<Vec<_>>(),
            [PrState::Open, PrState::Merged, PrState::Closed]
        );
        assert_eq!(prs[1].merged_at, Some(60));
        assert!(prs.iter().all(|p| p.repo == "core"));
        assert!(parse_reply("core", br#"[{"url":"u","state":"DRAFT","mergedAt":null}]"#).is_err());
        assert!(parse_reply("core", b"not json").is_err());
    }
}
