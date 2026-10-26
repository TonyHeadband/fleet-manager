//! Archive, the one destructive operation: removes a stopped task's worktrees and the daemon's files for it,
//! then its folder if nothing else is left, and writes the record one last time. Orphans go the same way and get a
//! record for it. Branches are never touched; nothing is deleted outside the task's recorded folder and the
//! daemon's own files, and nothing through a symbolic link.
use crate::agents::{AgentService, ServiceError, CLAUDE_MD_HEADER, PROMPT_FILE};
use crate::fleet::breakdown::BREAKDOWN_FILE;
use crate::fleet::ops::{FLEET_DIR, ORCHESTRATOR_DIR};
use crate::footprint;
use crate::git;
use crate::orphans;
use fleet_api::{
    format_bytes, AgentId, AgentKind, ArchivePlan, ArchiveReport, Footprint, FootprintItem,
    LogEntry, Orphan, OrphanKind, PlanItem, PrState, StatusTimes, Task, TaskState,
};
use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
use tracing::{info, warn};

const DAEMON_FILES: &str = "daemon files";
const OTHER_FILES: &str = "other files";

/// A worktree an archive removes.
#[derive(Clone, Debug)]
struct Tree {
    repo: String,
    path: PathBuf,
    /// The repo it was checked out from, when known; otherwise read from the worktree's own `.git` file.
    base: Option<PathBuf>,
}

/// Everything one archive touches.
#[derive(Clone, Debug)]
struct Target {
    /// A worktree is deleted by hand only when it lies inside this, once every link is resolved.
    boundary: PathBuf,
    /// Removed last, and only when that leaves it empty.
    folder: Option<PathBuf>,
    /// Folders the daemon made inside `folder` (an agent's own folder, its `.claude`, `fleet`): each removed only
    /// when that leaves it empty, deepest first, before `folder`.
    inner: Vec<PathBuf>,
    trees: Vec<Tree>,
    /// Repos the agent made in the folder: weighed, never removed.
    kept_repos: Vec<(String, PathBuf)>,
    /// Folders the daemon owns whole, removed whole: its own folder for each agent, and the skills it installed.
    daemon_dirs: Vec<PathBuf>,
    /// Files the daemon wrote into the task folder.
    daemon_files: Vec<PathBuf>,
}

enum TreeOutcome {
    Removed,
    /// Nothing on disk any more; at most git's record of it was cleared.
    Absent,
    Kept(String),
}

#[derive(Default)]
struct Removal {
    removed: Vec<String>,
    kept: Vec<String>,
}

impl Removal {
    /// What was freed is what the plan measured for each item that went: every path the plan lists as going is one
    /// thing `remove` deletes whole, so it is freed exactly when it is in `removed`.
    fn report(self, plan: &ArchivePlan) -> ArchiveReport {
        let freed = plan
            .items
            .iter()
            .filter(|i| !i.stays && self.removed.contains(&i.path))
            .map(|i| i.bytes)
            .sum();
        ArchiveReport {
            removed: self.removed,
            freed,
            kept: (!self.kept.is_empty()).then(|| self.kept.join("; ")),
        }
    }
}

/// The record's last log line, as the footer shows it after the task name.
fn archive_line(r: &ArchiveReport) -> String {
    match &r.kept {
        Some(kept) => format!("archived, {kept}"),
        None => format!("archived, {} freed", format_bytes(r.freed)),
    }
}

/// Deletes `dir` only when it is empty, so nothing the daemon did not make is ever removed; otherwise says what is left.
fn remove_if_empty(dir: &Path) -> Option<String> {
    let err = match std::fs::remove_dir(dir) {
        Ok(()) => return None,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => e,
    };
    let mut names: Vec<String> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => vec![],
    };
    if names.is_empty() {
        return Some(format!("kept {}: {err}", dir.display()));
    }
    names.sort();
    Some(format!(
        "kept {}: it contains {}",
        dir.display(),
        names.join(", ")
    ))
}

/// True when `path` is inside `boundary` both as recorded and once every link is resolved, so deleting it cannot
/// reach anything else.
fn resolves_inside(path: &Path, boundary: &Path) -> bool {
    if !path.starts_with(boundary) || path == boundary {
        return false;
    }
    match (std::fs::canonicalize(path), std::fs::canonicalize(boundary)) {
        (Ok(p), Ok(b)) => p != b && p.starts_with(&b),
        _ => false,
    }
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

fn removable(path: &Path, boundary: &Path) -> bool {
    !is_symlink(path) && resolves_inside(path, boundary)
}

/// An absolute path with no `.` or `..` in it: an orphan id comes from the client, and `<root>/x/..` has `<root>`
/// as its parent while naming the root's own parent.
fn is_plain_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
}

/// A full repository (a `.git` directory) rather than a worktree (a `.git` file): its branches may exist nowhere
/// else, so it is never removed, even when it sits where the task's worktree was.
fn is_repository(path: &Path) -> bool {
    !is_symlink(path) && real_dir(&path.join(".git"))
}

fn real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

/// Every regular file under `path` but git's own: what a worktree loses when git cannot say what is uncommitted.
fn count_files(path: &Path) -> u32 {
    let mut n = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(p) = pending.pop() {
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.is_file() {
            n += 1;
        } else if meta.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&p) {
                pending.extend(
                    entries
                        .flatten()
                        .filter(|e| e.file_name() != ".git")
                        .map(|e| e.path()),
                );
            }
        }
    }
    n
}

/// A file the daemon wrote is a file or a link; a folder by that name was made by someone else and is left alone.
fn is_daemon_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() || m.file_type().is_symlink())
}

/// Each of the daemon's own paths an archive deletes, with its size: its folders, then each file it wrote into the
/// task folder. Only what is there now, judged as `remove` judges it. Blocking.
fn daemon_items(t: &Target) -> Vec<(String, u64)> {
    t.daemon_dirs
        .iter()
        .filter(|d| std::fs::symlink_metadata(d).is_ok())
        .chain(t.daemon_files.iter().filter(|f| is_daemon_file(f)))
        .map(|p| (p.to_string_lossy().into_owned(), footprint::dir_bytes(p)))
        .collect()
}

/// Where the footprint files the daemon's own weight: its first folder (the lead's), or for an orphan folder the
/// first file the daemon wrote into it; never the task folder, whose plan item is what stays.
fn daemon_item_path(t: &Target) -> Option<String> {
    t.daemon_dirs
        .first()
        .or(t.daemon_files.first())
        .map(|p| p.to_string_lossy().into_owned())
}

/// The target's weight, laid out as `footprint::measure` lays out a task's: each worktree with its build output,
/// the daemon's files, and whatever else the folder holds. Blocking.
fn measure(t: &Target, now: i64) -> Footprint {
    let mut items: Vec<FootprintItem> = t
        .trees
        .iter()
        .map(|w| (w.repo.as_str(), w.path.as_path()))
        .chain(t.kept_repos.iter().map(|(r, p)| (r.as_str(), p.as_path())))
        .filter_map(|(repo, path)| footprint::worktree_item(repo, path))
        .collect();
    if let Some(path) = daemon_item_path(t) {
        let bytes = t
            .daemon_dirs
            .iter()
            .chain(&t.daemon_files)
            .map(|p| footprint::dir_bytes(p))
            .sum();
        items.push(FootprintItem {
            label: DAEMON_FILES.into(),
            path,
            bytes,
            build_bytes: None,
        });
    }
    if let Some(folder) = &t.folder {
        let other = other_bytes(t, folder);
        if other > 0 {
            items.push(FootprintItem {
                label: OTHER_FILES.into(),
                path: folder.to_string_lossy().into_owned(),
                bytes: other,
                build_bytes: None,
            });
        }
    }
    let total = items.iter().map(|i| i.bytes).sum();
    Footprint {
        measured_at: now,
        items,
        total,
    }
}

/// What `dir` holds that the target does not list, looking inside the daemon's own inner folders rather than
/// weighing them whole, since most of what they hold is listed. Blocking.
fn other_bytes(t: &Target, dir: &Path) -> u64 {
    let listed = |p: &Path| {
        t.trees.iter().any(|w| w.path == p)
            || t.kept_repos.iter().any(|(_, r)| r == p)
            || t.daemon_files.iter().any(|f| f == p)
            || t.daemon_dirs.iter().any(|d| d == p)
    };
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| !listed(p))
                .map(|p| {
                    if t.inner.contains(&p) && real_dir(&p) {
                        other_bytes(t, &p)
                    } else {
                        footprint::dir_bytes(&p)
                    }
                })
                .sum()
        })
        .unwrap_or(0)
}

/// A `CLAUDE.md` the daemon wrote: it carries the header, after the fleet-wide instructions or a role's prompt when
/// there are any. Any other is the user's.
fn is_daemon_claude_md(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| text.contains(CLAUDE_MD_HEADER))
}

/// The real folders directly in `<task>/fleet`, links left out.
fn fleet_folders(task: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(task.join(FLEET_DIR)) else {
        return vec![];
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs
}

impl Target {
    /// Adds what the daemon made in an agent's own folder inside the task folder (an orchestrator's or a fleet
    /// agent's): its `CLAUDE.md`, `PROMPT.md` and installed skills go, and the folder itself once empty. Nothing
    /// from a folder that is a link or leads outside the boundary, as everything in it would be deleted through that.
    fn add_agent_folder(&mut self, folder: &Path) {
        if !removable(folder, &self.boundary)
            || !real_dir(folder)
            || self.inner.iter().any(|d| d == folder)
        {
            return;
        }
        self.daemon_files.push(folder.join("CLAUDE.md"));
        self.daemon_files.push(folder.join(PROMPT_FILE));
        let skills = folder.join(".claude/skills");
        if removable(&skills, &self.boundary) && real_dir(&skills) {
            self.daemon_dirs.push(skills);
        }
        self.inner.push(folder.join(".claude"));
        self.inner.push(folder.to_path_buf());
    }

    /// Adds `<task>/fleet` as a folder to remove once its agents' folders are gone.
    fn add_fleet_dir(&mut self, task: &Path) {
        let fleet = task.join(FLEET_DIR);
        if removable(&fleet, &self.boundary) && real_dir(&fleet) {
            self.inner.push(fleet);
        }
    }
}

/// What the user confirmed is what would happen now: the same paths, each going or staying as shown, losing the same
/// uncommitted files. Sizes are not compared; a build or a log moves them without changing what is lost.
fn same_plan(confirmed: &[PlanItem], now: &[PlanItem]) -> bool {
    let what = |items: &[PlanItem]| {
        items
            .iter()
            .map(|i| (i.path.clone(), i.stays, i.uncommitted_files))
            .collect::<BTreeSet<_>>()
    };
    confirmed.len() == now.len() && what(confirmed) == what(now)
}

/// The plan read off a measurement: each worktree that will go (or the repository that stays in its place), each of
/// the daemon's paths, then the folder with what stays in it (its other files and the repos the agent made).
fn plan_of(
    t: &Target,
    fp: &Footprint,
    daemon: &[(String, u64)],
    uncommitted: &HashMap<PathBuf, u32>,
) -> ArchivePlan {
    let item = |path: &str| fp.items.iter().find(|i| i.path == path);
    let mut items = Vec::new();
    for w in &t.trees {
        let Some(i) = item(&w.path.to_string_lossy()) else {
            continue;
        };
        if is_repository(&w.path) {
            items.push(PlanItem {
                path: i.path.clone(),
                bytes: i.bytes,
                uncommitted_files: 0,
                stays: true,
            });
        } else if let Some(n) = uncommitted.get(&w.path) {
            items.push(PlanItem {
                path: i.path.clone(),
                bytes: i.bytes,
                uncommitted_files: *n,
                stays: false,
            });
        }
    }
    items.extend(daemon.iter().map(|(path, bytes)| PlanItem {
        path: path.clone(),
        bytes: *bytes,
        uncommitted_files: 0,
        stays: false,
    }));
    if let Some(folder) = &t.folder {
        let other = fp
            .items
            .iter()
            .find(|i| i.label == OTHER_FILES)
            .map_or(0, |i| i.bytes);
        let kept: u64 = t
            .kept_repos
            .iter()
            .filter_map(|(_, p)| item(&p.to_string_lossy()))
            .map(|i| i.bytes)
            .sum();
        items.push(PlanItem {
            path: folder.to_string_lossy().into_owned(),
            bytes: other + kept,
            uncommitted_files: 0,
            stays: true,
        });
    }
    let total = items.iter().filter(|i| !i.stays).map(|i| i.bytes).sum();
    ArchivePlan { items, total }
}

impl AgentService {
    /// What archiving the task would remove, measured now. Only for a stopped task with no open PR.
    pub async fn archive_plan(&self, id: &AgentId) -> Result<ArchivePlan, ServiceError> {
        let t = self.task_target(id).await?;
        self.check_merge_now(id).await;
        self.refuse_open_pr(id).await?;
        Ok(self.plan(&t).await?.1)
    }

    /// One merge check of the task, so a PR merged or closed since the last pass is not refused on stale data.
    /// A failed check leaves the record as it was, and the caller decides on that.
    async fn check_merge_now(&self, id: &AgentId) {
        if self.merge_checker().check_one(id).await == crate::merges::Checked::Stale {
            self.request_merge_check(Some(id.clone()));
        }
    }

    /// Refuses while the task's record has an open PR: archiving would drop a task whose PR may still merge.
    async fn refuse_open_pr(&self, id: &AgentId) -> Result<(), ServiceError> {
        let Some(t) = self.store.task(id).await? else {
            return Err(ServiceError::NotFound);
        };
        match t.prs.iter().find(|p| p.state == PrState::Open) {
            Some(p) => Err(ServiceError::OpenPr(p.url.clone())),
            None => Ok(()),
        }
    }

    /// Removes a stopped task's worktrees (their branches stay), the daemon's files and then the folder if that
    /// leaves it empty; takes its card off the board and marks its record archived. `confirmed` is the plan the user
    /// confirmed: the plan is made again first and nothing is removed unless it matches (`PlanChanged`). When
    /// anything fails the card and the record stay as they were but for an `archive failed: …` log line, and the next
    /// plan shows what is left.
    pub async fn archive(
        &self,
        id: &AgentId,
        confirmed: &[PlanItem],
    ) -> Result<ArchiveReport, ServiceError> {
        // A running task is refused as the plan refuses it, before GitHub is asked. Any other target error is left to
        // `archive_inner`, which logs it. The merge check runs outside the workspaces lock, since it may wait on
        // GitHub; the record is read again under the lock, for a PR opened meanwhile.
        if let Err(
            e
            @ (ServiceError::NotFound | ServiceError::NotStopped | ServiceError::SessionStillLive),
        ) = self.task_target(id).await
        {
            return Err(e);
        }
        self.check_merge_now(id).await;
        self.refuse_open_pr(id).await?;
        let result = self.archive_inner(id, confirmed).await;
        match &result {
            // Refused before anything was tried: the caller was told why, and the task is as it was.
            Err(
                ServiceError::NotFound
                | ServiceError::NotStopped
                | ServiceError::SessionStillLive
                | ServiceError::PlanChanged
                | ServiceError::OpenPr(_),
            )
            | Ok(_) => {}
            Err(e) => {
                warn!(agent_id = %id.0, error = %e, "archive failed");
                self.log_task(id, format!("archive failed: {e}")).await;
            }
        }
        result
    }

    async fn archive_inner(
        &self,
        id: &AgentId,
        confirmed: &[PlanItem],
    ) -> Result<ArchiveReport, ServiceError> {
        let (fp, report) = {
            // Planned, compared and removed under one lock, so no repo can be attached between what the user
            // confirmed and what goes.
            let _ws = self.workspaces_lock.lock().await;
            let t = self.task_target(id).await?;
            self.refuse_open_pr(id).await?;
            let (fp, plan) = self.plan(&t).await?;
            if !same_plan(confirmed, &plan.items) {
                info!(agent_id = %id.0, "archive refused: the plan changed since it was confirmed");
                return Err(ServiceError::PlanChanged);
            }
            let report = self.remove(&t, &id.0).await?.report(&plan);
            (fp, report)
        };
        let now = self.opts.clock.now();
        let line = archive_line(&report);
        let written = report.clone();
        self.store
            .archive_task(id, move |p| {
                p.state = TaskState::Archived;
                p.archived_at = Some(now);
                p.archive = Some(written);
                p.footprint = Some(fp);
                p.log.push(LogEntry {
                    at: now,
                    text: line,
                });
            })
            .await?;
        info!(agent_id = %id.0, freed = report.freed, kept = report.kept.as_deref().unwrap_or(""), "task archived");
        Ok(report)
    }

    /// What archiving an orphan from the last scan would remove. Not for a session, which is ended instead.
    pub async fn orphan_plan(&self, orphan_id: &str) -> Result<ArchivePlan, ServiceError> {
        let (_, t) = self.orphan_target(orphan_id).await?;
        Ok(self.plan(&t).await?.1)
    }

    /// Archives an orphan from the last scan as `archive` does a task, against the plan the user `confirmed`,
    /// then gives it an archived record so it stays in the history. It is checked again against the records and the
    /// spawns in flight right before anything is removed: something a record or a spawn has claimed since the scan
    /// is left alone.
    pub async fn archive_orphan(
        &self,
        orphan_id: &str,
        confirmed: &[PlanItem],
    ) -> Result<ArchiveReport, ServiceError> {
        let (o, fp, report) = {
            let _pick = self.pick_lock.lock().await;
            // An orphan has no task log: the daemon's is the only place a failure shows.
            let failed =
                |e: &ServiceError| warn!(orphan = orphan_id, error = %e, "orphan archive failed");
            let (o, t) = self.orphan_target(orphan_id).await.inspect_err(failed)?;
            let (fp, plan) = self.plan(&t).await.inspect_err(failed)?;
            if !same_plan(confirmed, &plan.items) {
                info!(
                    orphan = orphan_id,
                    "orphan archive refused: the plan changed since it was confirmed"
                );
                return Err(ServiceError::PlanChanged);
            }
            match self.remove(&t, orphan_id).await {
                Ok(r) => (o, fp, r.report(&plan)),
                Err(e) => {
                    failed(&e);
                    return Err(e);
                }
            }
        };
        let now = self.opts.clock.now();
        let path = Path::new(&o.path);
        let record = Task {
            id: AgentId(uuid::Uuid::new_v4().simple().to_string()),
            // An orphan belongs to no product.
            product: String::new(),
            release: None,
            name: path
                .file_name()
                .map_or_else(|| o.path.clone(), |n| n.to_string_lossy().into_owned()),
            goal: String::new(),
            workdir: o.path.clone(),
            repos: vec![],
            state: TaskState::Archived,
            stop_cause: None,
            created_at: None,
            stopped_at: None,
            archived_at: Some(now),
            agents_used: 0,
            times: StatusTimes::default(),
            log: vec![
                LogEntry {
                    at: now,
                    text: "found as orphan".into(),
                },
                LogEntry {
                    at: now,
                    text: archive_line(&report),
                },
            ],
            footprint: Some(fp),
            last_screen: None,
            archive: Some(report.clone()),
            // An orphan belongs to no persona.
            persona: String::new(),
            kind: fleet_api::AgentKind::Solo,
            role: None,
            roster: vec![],
            lead: None,
            breakdowns: vec![],
            inbox: vec![],
            gate_runs: vec![],
            prs: vec![],
            done_at: None,
            done_by_hand: false,
        };
        // No record exists to log to, so the daemon's own log is all there is for a failure here.
        if let Err(e) = self.store.insert_task(&record).await {
            warn!(orphan = orphan_id, error = %e, "orphan archived, but its record was not written");
            return Err(e.into());
        }
        info!(orphan = orphan_id, task = %record.id.0, freed = report.freed, kept = report.kept.as_deref().unwrap_or(""), "orphan archived");
        self.rescan_orphans().await;
        Ok(report)
    }

    /// Everything archiving the task `id` touches. Every agent of it must be stopped and its session gone: an
    /// orchestrated task's folder holds its fleet's folders and worktrees too, and its orchestrators' (a restart
    /// is one more agent in the same folder).
    async fn task_target(&self, id: &AgentId) -> Result<Target, ServiceError> {
        // A fleet agent's id names no record: only a task is archived.
        let Some(p) = self.store.task(id).await? else {
            return Err(ServiceError::NotFound);
        };
        let agents = self.store.task_agents(id).await?;
        if agents.is_empty() {
            return Err(ServiceError::NotFound);
        }
        if agents
            .iter()
            .any(|a| a.status != fleet_api::Status::Stopped)
        {
            return Err(ServiceError::NotStopped);
        }
        // A stop whose kill failed leaves the agent Stopped over a running claude.
        for a in &agents {
            if self.tmux.has_session(&a.tmux_session).await? {
                return Err(ServiceError::SessionStillLive);
            }
        }
        let orchestrated = p.kind == AgentKind::Orchestrator;
        // An orchestrated task's folder is the record's; each agent's own is inside it.
        let workdir = if orchestrated {
            PathBuf::from(&p.workdir)
        } else {
            PathBuf::from(&agents[0].workdir)
        };
        // Everything below is deleted by path inside it; through a link that would be someone else's folder.
        if is_symlink(&workdir) {
            return Err(ServiceError::Refused(format!(
                "{} is a symbolic link; nothing was removed",
                workdir.display()
            )));
        }
        let mut t = Target {
            boundary: workdir.clone(),
            folder: Some(workdir.clone()),
            inner: vec![],
            trees: vec![],
            kept_repos: vec![],
            daemon_dirs: agents.iter().map(|a| self.agent_dir(&a.id.0)).collect(),
            daemon_files: vec![],
        };
        for a in &agents {
            for w in &a.workspaces {
                let path = PathBuf::from(&w.worktree);
                // Every orchestrator of the task works in the same worktree.
                if t.trees.iter().any(|x| x.path == path)
                    || t.kept_repos.iter().any(|(_, x)| *x == path)
                {
                    continue;
                }
                if w.owned {
                    t.kept_repos.push((w.repo.clone(), path));
                } else {
                    let base = self
                        .repos
                        .base_of(&path, &a.persona, &a.product, &w.repo)
                        .await;
                    t.trees.push(Tree {
                        repo: w.repo.clone(),
                        path,
                        base,
                    });
                }
            }
        }
        if orchestrated {
            t.daemon_files.push(workdir.join(BREAKDOWN_FILE));
            for a in &agents {
                t.add_agent_folder(Path::new(&a.workdir));
            }
            t.add_fleet_dir(&workdir);
        } else {
            t.daemon_files = vec![workdir.join("CLAUDE.md"), workdir.join(PROMPT_FILE)];
        }
        Ok(t)
    }

    /// The orphan `orphan_id` from the last scan, checked again now: still where an orphan of its kind lies, and
    /// owned by no record and no spawn in flight. Only what the user was shown can be archived, and only while
    /// nothing has claimed it since.
    async fn orphan_target(&self, orphan_id: &str) -> Result<(Orphan, Target), ServiceError> {
        let o = self
            .orphans()
            .into_iter()
            .find(|o| o.id == orphan_id)
            .ok_or(ServiceError::NotAnOrphan)?;
        if o.kind == OrphanKind::Session {
            return Err(ServiceError::SessionNotArchived);
        }
        let path = PathBuf::from(&o.path);
        let root = self.cfg.tasks_root.clone();
        let agents_dir = self.cfg.state_dir.join("agents");
        let placed = is_plain_absolute(&path)
            && match o.kind {
                OrphanKind::TaskFolder => path.parent() == Some(root.as_path()) && real_dir(&path),
                OrphanKind::AgentFolder => {
                    path.parent() == Some(agents_dir.as_path()) && real_dir(&path)
                }
                OrphanKind::Worktree => path.starts_with(&root) && path != root,
                OrphanKind::Session => false,
            };
        if !placed {
            return Err(ServiceError::NotAnOrphan);
        }
        // Read after the folder was seen on disk, so a spawn that made it is in the spawn set or has its record.
        if self.owned_now().await?.owns(&o) {
            return Err(ServiceError::NotAnOrphan);
        }
        let bases = self.repos.all_bases().await?;
        let scan_root = root.clone();
        let registered =
            tokio::task::spawn_blocking(move || orphans::worktrees_under(&scan_root, &bases))
                .await
                .map_err(std::io::Error::other)?;
        let tree = |(p, base): (PathBuf, PathBuf)| Tree {
            repo: p
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            path: p,
            base: Some(base),
        };
        let t = match o.kind {
            OrphanKind::TaskFolder => {
                let mut t = Target {
                    boundary: path.clone(),
                    folder: Some(path.clone()),
                    inner: vec![],
                    trees: registered
                        .into_iter()
                        .filter(|(w, _)| w.starts_with(&path) && *w != path)
                        .map(tree)
                        .collect(),
                    kept_repos: vec![],
                    daemon_dirs: vec![],
                    daemon_files: vec![],
                };
                // A folder the daemon made says so in its CLAUDE.md; any other's CLAUDE.md and PROMPT.md are the
                // user's. An orchestrated task's says so in each agent's folder instead.
                if is_daemon_claude_md(&path.join("CLAUDE.md")) {
                    t.daemon_files = vec![path.join("CLAUDE.md"), path.join(PROMPT_FILE)];
                }
                let lead = path.join(ORCHESTRATOR_DIR);
                if is_daemon_claude_md(&lead.join("CLAUDE.md")) {
                    t.add_agent_folder(&lead);
                    t.daemon_files.push(path.join(BREAKDOWN_FILE));
                }
                let fleet: Vec<PathBuf> = fleet_folders(&path)
                    .into_iter()
                    .filter(|f| is_daemon_claude_md(&f.join("CLAUDE.md")))
                    .collect();
                if !fleet.is_empty() {
                    for f in &fleet {
                        t.add_agent_folder(f);
                    }
                    t.add_fleet_dir(&path);
                }
                t
            }
            OrphanKind::AgentFolder => Target {
                boundary: agents_dir,
                folder: None,
                inner: vec![],
                trees: vec![],
                kept_repos: vec![],
                daemon_dirs: vec![path.clone()],
                daemon_files: vec![],
            },
            OrphanKind::Worktree => {
                // Only a worktree git still registers is known to be one; a plain folder at its path is not.
                let Some(found) = registered.into_iter().find(|(w, _)| *w == path) else {
                    return Err(ServiceError::NotAnOrphan);
                };
                Target {
                    boundary: root,
                    folder: None,
                    inner: vec![],
                    trees: vec![tree(found)],
                    kept_repos: vec![],
                    daemon_dirs: vec![],
                    daemon_files: vec![],
                }
            }
            OrphanKind::Session => unreachable!("refused above"),
        };
        Ok((o, t))
    }

    async fn plan(&self, t: &Target) -> Result<(Footprint, ArchivePlan), ServiceError> {
        let now = self.opts.clock.now();
        let owned = t.clone();
        let (fp, daemon) =
            tokio::task::spawn_blocking(move || (measure(&owned, now), daemon_items(&owned)))
                .await
                .map_err(std::io::Error::other)?;
        let mut uncommitted = HashMap::new();
        for w in &t.trees {
            if !real_dir(&w.path) || !removable(&w.path, &t.boundary) || is_repository(&w.path) {
                continue;
            }
            let n = match git::uncommitted_files(&w.path).await {
                Ok(n) => n,
                Err(e) => {
                    // A plan must not say nothing is lost because git cannot tell: then every file is.
                    warn!(worktree = %w.path.display(), error = %e, "cannot count uncommitted files; counting every file");
                    let p = w.path.clone();
                    tokio::task::spawn_blocking(move || count_files(&p))
                        .await
                        .map_err(std::io::Error::other)?
                }
            };
            uncommitted.insert(w.path.clone(), n);
        }
        let plan = plan_of(t, &fp, &daemon, &uncommitted);
        Ok((fp, plan))
    }

    /// Worktrees first, then the daemon's files, then the folder if empty. Stops at the first failure, leaving
    /// the rest in place.
    async fn remove(&self, t: &Target, who: &str) -> Result<Removal, ServiceError> {
        let mut r = Removal::default();
        for w in &t.trees {
            match self.remove_tree(w, &t.boundary, who).await? {
                TreeOutcome::Removed => r.removed.push(w.path.to_string_lossy().into_owned()),
                TreeOutcome::Absent => {}
                TreeOutcome::Kept(note) => r.kept.push(note),
            }
        }
        for f in &t.daemon_files {
            if !is_daemon_file(f) {
                continue;
            }
            match std::fs::remove_file(f) {
                Ok(()) => r.removed.push(f.to_string_lossy().into_owned()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(ServiceError::Removal(format!(
                        "cannot delete {}: {e}",
                        f.display()
                    )))
                }
            }
        }
        for dir in &t.daemon_dirs {
            // remove_dir_all does not follow links: a link here goes, what it points to stays.
            let gone = match std::fs::symlink_metadata(dir) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(e),
                Ok(m) if m.is_dir() => std::fs::remove_dir_all(dir).map(|()| true),
                Ok(_) => std::fs::remove_file(dir).map(|()| true),
            };
            match gone {
                Ok(true) => r.removed.push(dir.to_string_lossy().into_owned()),
                Ok(false) => {}
                Err(e) => {
                    return Err(ServiceError::Removal(format!(
                        "cannot delete {}: {e}",
                        dir.display()
                    )))
                }
            }
        }
        let mut inner = t.inner.clone();
        inner.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
        let mut kept_dirs: Vec<PathBuf> = vec![];
        for dir in inner.iter().chain(&t.folder) {
            let existed = std::fs::symlink_metadata(dir).is_ok();
            match remove_if_empty(dir) {
                None if existed && Some(dir) == t.folder.as_ref() => {
                    r.removed.push(dir.to_string_lossy().into_owned())
                }
                None => {}
                Some(note) => {
                    // A folder kept only because a folder in it was is said once, where the files are.
                    let only_kept = std::fs::read_dir(dir).is_ok_and(|entries| {
                        entries.flatten().all(|e| kept_dirs.contains(&e.path()))
                    });
                    if !only_kept {
                        r.kept.push(note);
                    }
                    kept_dirs.push(dir.clone());
                }
            }
        }
        Ok(r)
    }

    /// Removes one worktree, even when git cannot: its repo is no longer remembered (the base is then read from
    /// the worktree's own `.git` file), or `git worktree remove` fails (locked, broken, base moved). The fallback
    /// deletes the folder by hand, then prunes. Neither runs on a link, on a repository (a `.git` directory) or on
    /// anything outside `boundary`: those are kept and the report says so. The branch is never touched, so committed work survives.
    async fn remove_tree(
        &self,
        w: &Tree,
        boundary: &Path,
        who: &str,
    ) -> Result<TreeOutcome, ServiceError> {
        let path = &w.path;
        match std::fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Its folder is gone already: only git's record of it is left, which keeps the branch checked out.
                if let Some(base) = &w.base {
                    let _base = self.repos.lock_base(base).await;
                    if let Err(e) = git::forget_worktree(base, path).await {
                        warn!(who, repo = %w.repo, error = %e, "cannot prune worktrees");
                    }
                }
                return Ok(TreeOutcome::Absent);
            }
            Err(e) => {
                return Err(ServiceError::Removal(format!(
                    "cannot read {}: {e}",
                    path.display()
                )))
            }
            Ok(m) if m.file_type().is_symlink() => {
                warn!(who, worktree = %path.display(), "worktree is a symbolic link; leaving it in place");
                return Ok(TreeOutcome::Kept(format!(
                    "kept {}: it is a symbolic link",
                    path.display()
                )));
            }
            Ok(_) => {}
        }
        // Checked before git too: `git worktree remove` refuses a repository, and the by-hand fallback would then
        // delete it with branches that may exist only there.
        if is_repository(path) {
            warn!(who, worktree = %path.display(), "a repository is where the worktree was; leaving it in place");
            return Ok(TreeOutcome::Kept(format!(
                "kept {}: it is a repository, not a worktree",
                path.display()
            )));
        }
        if !resolves_inside(path, boundary) {
            warn!(who, worktree = %path.display(), "worktree is outside the task folder; leaving it in place");
            return Ok(TreeOutcome::Kept(format!(
                "kept {}: it is outside {}",
                path.display(),
                boundary.display()
            )));
        }
        let base = match &w.base {
            Some(b) => Some(b.clone()),
            None => {
                let p = path.clone();
                match git::blocking(move || fleet_fs::main_root(&p)).await {
                    Ok(Some(fleet_fs::MainRoot::Worktree(root))) => Some(root),
                    _ => None,
                }
            }
        };
        let mut failure = None;
        if let Some(base) = &base {
            let _base = self.repos.lock_base(base).await;
            match git::remove_worktree_at(base, path).await {
                Ok(()) => return Ok(TreeOutcome::Removed),
                Err(e) => failure = Some(e.to_string()),
            }
        }
        warn!(who, repo = %w.repo, worktree = %path.display(), git_error = failure.as_deref().unwrap_or("repo not found"), "git could not remove the worktree; deleting its folder");
        match std::fs::remove_dir_all(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(ServiceError::Removal(format!(
                    "cannot delete {}: {e}",
                    path.display()
                )))
            }
        }
        if let Some(base) = &base {
            let _base = self.repos.lock_base(base).await;
            if let Err(e) = git::forget_worktree(base, path).await {
                warn!(who, repo = %w.repo, error = %e, "cannot prune worktrees");
            }
        }
        Ok(TreeOutcome::Removed)
    }
}
