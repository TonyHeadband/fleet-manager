//! The orchestrated parts of `AgentService`: what an orchestrator task adds to a solo spawn, and its breakdown.
use super::breakdown::{next_version, render, state_word, BreakdownAnswer, BREAKDOWN_FILE};
use super::defs::{Defs, FleetError, OrchestratorDef, SpecialistDef};
use super::gate;
use super::skills;
use super::workflow::{self, WorkflowError};
use crate::agents::{
    agent_insert_error, fleet_config, instructions_section, internal_commit_command,
    internal_section, one_line, repo_section, task_repo_rules, write_prompt, AgentService,
    InternalCopy, ServiceError, SpawnError, Start, Undo, PROMPT_FILE, SESSION_PREFIX,
};
use crate::config::LimitValues;
use crate::git;
use crate::hooks_cfg::{write_mcp_config, write_settings, RoleSettings};
use crate::internal::{Internal, MergeOutcome, MergeReport, PushOutcome};
use crate::overrides;
use crate::personas::PersonaEnv;
use crate::repos::Resolved;
use crate::store::StoreError;
use fleet_api::{
    task_id, Agent, AgentConfig, AgentId, AgentKind, Approval, BlockedReason, BreakdownVersion,
    DecideBreakdown, DefKind, FleetEvent, FleetEventKind, FleetView, GateRun, GateStep, PrState,
    SettingSource, SpawnRequest, Status, Step, StepDraft, StepState, StopCause, Task, TaskPr,
    Workspace,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::sync::{broadcast, oneshot};
use tracing::{info, warn};

/// The orchestrator's own folder inside the task folder: its cwd, holding its worktree, `CLAUDE.md`, prompt and
/// skills. The task folder itself stays empty of anything Claude Code loads, since every fleet agent's folder
/// sits under it too (found in a spike).
pub const ORCHESTRATOR_DIR: &str = "orchestrator";

/// The orchestrated task's checkout of its branch in the product's internal repo, beside `ORCHESTRATOR_DIR` so it
/// can never be mistaken for (or clash with) a repo named `internal` inside it. The repos themselves sit under
/// `state_dir` by this same name.
pub const INTERNAL_DIR: &str = "internal";

/// Each specialist's read-only checkout of the task's internal notes, inside the task folder.
pub const SNAPSHOTS_DIR: &str = "internal-snapshots";

/// The folder of the fleet agents' folders, inside the task folder. Like the task folder it holds nothing
/// Claude Code loads, since it is the parent of every one of them (found in a spike).
pub const FLEET_DIR: &str = "fleet";

/// A step's branch, beside the task branch rather than under it: git cannot keep `fleet/x` and `fleet/x/1-api`
/// both, since one would be a file where the other needs a folder. A slug never holds `--`, so no task's own
/// branch can be named like this.
pub fn step_branch(task_branch: &str, n: u32, slug: &str) -> String {
    format!("{task_branch}--{n}-{slug}")
}

/// The specialist's `CLAUDE.md`: the fleet-wide instructions, its role prompt and its one repo.
fn specialist_claude_md(
    def: &SpecialistDef,
    instructions: &str,
    workspaces: &[Workspace],
    snapshot: Option<&Snapshot>,
) -> String {
    let mut text = instructions_section(instructions);
    text.push_str(def.prompt.trim_end());
    text.push_str("\n\n");
    text.push_str(&repo_section(workspaces, false));
    if let Some(s) = snapshot {
        text.push('\n');
        text.push_str(&internal_section(
            &s.path,
            InternalCopy::Snapshot { sha: &s.sha },
        ));
    }
    text
}

/// The step's assignment, with the goal it serves.
fn specialist_prompt(step: &Step, goal: &str) -> String {
    format!(
        "{}\n\nThe task's goal: {}",
        step.brief.trim_end(),
        goal.trim()
    )
}

/// How long a held `wait_for_fleet` goes without looking at its orchestrator, for a stop that no broadcast announced.
const WAIT_TICK: Duration = Duration::from_millis(500);

/// The breakdown as `fleet_status` reports it: the latest version, and each step with its agent's live status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FleetStatus {
    pub version: u32,
    pub approval: Approval,
    pub steps: Vec<(u32, StepState, Option<Status>)>,
}

fn refuse(text: impl Into<String>) -> ServiceError {
    ServiceError::Fleet(text.into())
}

/// Runs `work` on its own task, so a caller that goes away (an MCP client hanging up) cannot stop it halfway.
async fn detached<T: Send + 'static>(
    work: impl std::future::Future<Output = Result<T, ServiceError>> + Send + 'static,
) -> Result<T, ServiceError> {
    match tokio::spawn(work).await {
        Ok(r) => r,
        // Never aborted, so only a panic ends it early; it goes on as if the work had run inline.
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
}

/// The latest version, if it is approved. Only the latest version counts: while a newer one waits for a decision,
/// or was sent back, the approved one before it is not the plan any more.
fn approved_plan(breakdowns: &[BreakdownVersion]) -> Result<&BreakdownVersion, ServiceError> {
    let Some(v) = breakdowns.last() else {
        return Err(refuse("no breakdown is approved yet"));
    };
    match v.approval {
        Approval::Approved => Ok(v),
        Approval::Proposed => Err(refuse(format!(
            "breakdown v{} is waiting for approval",
            v.version
        ))),
        Approval::ChangesRequested => Err(refuse(format!(
            "breakdown v{} needs a new proposal",
            v.version
        ))),
    }
}

/// The step `n` of the approved plan, if a specialist may start on it now.
fn ready_step(breakdowns: &[BreakdownVersion], n: u32) -> Result<&Step, ServiceError> {
    let v = approved_plan(breakdowns)?;
    let Some(step) = v.steps.iter().find(|s| s.n == n) else {
        return Err(refuse(format!(
            "step {n} is not in breakdown v{}",
            v.version
        )));
    };
    if step.state != StepState::Pending {
        return Err(refuse(format!("step {n} is {}", state_word(step.state))));
    }
    for d in &step.depends_on {
        let done = v
            .steps
            .iter()
            .any(|s| s.n == *d && s.state == StepState::Done);
        if !done {
            return Err(refuse(format!("step {n} waits for step {d}")));
        }
    }
    Ok(step)
}

/// Applies `f` to step `n` of the last approved version, then copies its progress into the same step of any version
/// proposed after it, so that version carries it if it is approved.
fn update_step(p: &mut Task, n: u32, f: impl FnOnce(&mut Step)) {
    let Some(i) = p
        .breakdowns
        .iter()
        .rposition(|v| v.approval == Approval::Approved)
    else {
        return;
    };
    let Some(step) = p.breakdowns[i].steps.iter_mut().find(|s| s.n == n) else {
        return;
    };
    f(step);
    let done = step.clone();
    for v in &mut p.breakdowns[i + 1..] {
        if let Some(s) = v.steps.iter_mut().find(|s| s.n == n) {
            s.state = done.state;
            s.agent = done.agent.clone();
            s.branch = done.branch.clone();
            s.report = done.report.clone();
        }
    }
}

/// A specialist's detached checkout of the task's internal notes.
struct Snapshot {
    path: PathBuf,
    sha: String,
}

/// What a specialist spawn has made so far, for undoing it.
struct Made {
    undo: Undo,
    /// Lives in the task folder beside the step folders, so undoing the step folder does not remove it.
    snapshot: Option<PathBuf>,
    card: Option<AgentId>,
    step_running: bool,
}

/// What a specialist spawn works from, read under the breakdown lock.
struct SpecialistPlan {
    task: Task,
    step: Step,
    def: SpecialistDef,
    lead_workspace: Workspace,
    persona: PersonaEnv,
    fleet: AgentConfig,
}

/// What an orchestrated spawn adds to a solo one, worked out before anything is made.
pub(crate) struct OrchestratorPlan {
    pub def: OrchestratorDef,
    /// Each specialist the orchestrator may use, with its summary, as it is shown them.
    pub specialists: Vec<(String, String)>,
}

impl OrchestratorPlan {
    /// `internal` is the task's checkout of its internal notes, which the agent is allowed to work in.
    pub fn settings(&self, internal: &Path) -> RoleSettings {
        RoleSettings {
            dirs: vec![internal.to_string_lossy().into_owned()],
            ..RoleSettings::orchestrator(&self.def.tools)
        }
    }

    /// The orchestrator's `CLAUDE.md`: the fleet-wide instructions, its type's prompt, its specialists and the repo.
    fn claude_md(
        &self,
        instructions: &str,
        workspaces: &[Workspace],
        internal: &Path,
        task_name: &str,
    ) -> String {
        let mut text = instructions_section(instructions);
        text.push_str(self.def.prompt.trim_end());
        text.push_str("\n\n## Your specialists\n\n");
        for (name, summary) in &self.specialists {
            text.push_str(&format!("- {name}: {summary}\n"));
        }
        text.push('\n');
        text.push_str(&repo_section(workspaces, false));
        text.push('\n');
        text.push_str(&internal_section(
            internal,
            InternalCopy::Writable { task: task_name },
        ));
        text
    }
}

/// The persona's env as the agent's process gets it. A fleet agent's gets more git config entries, appended after
/// the persona's own, that rewrite every push to the persona's host (the one host it has credentials for, in its
/// https and ssh spellings) to a transport that does not exist: a push the deny rules miss (`bash -c 'git push'`, a
/// script, `git push <url>`) then fails too, as do pushes to `origins` (the task repos' own, from `origin_urls`: one
/// added by path may have a local origin). Scoped by URL, not by remote name, so the remotes a repo's own tests make
/// (local paths, often named `origin`) push as they do on CI. Not a boundary: unsetting the env gets past it, and git
/// ignores `pushInsteadOf` for a remote with its own `pushurl`. `open_pr` pushes with the persona's env alone.
pub fn agent_vars(
    persona: &PersonaEnv,
    kind: AgentKind,
    origins: &[String],
) -> Vec<(String, String)> {
    let mut vars = persona.vars.clone();
    if kind == AgentKind::Solo {
        return vars;
    }
    let host = &persona.host;
    let mut blocked = vec![
        format!("https://{host}/"),
        format!("ssh://git@{host}/"),
        format!("git@{host}:"),
    ];
    blocked.extend(origins.iter().cloned());
    let count = vars.iter_mut().find(|(k, _)| k == "GIT_CONFIG_COUNT");
    let n = match count {
        Some((_, v)) => {
            let n: usize = v.parse().unwrap_or(0);
            *v = (n + blocked.len()).to_string();
            n
        }
        None => {
            vars.push(("GIT_CONFIG_COUNT".into(), blocked.len().to_string()));
            0
        }
    };
    for (i, prefix) in blocked.into_iter().enumerate() {
        vars.push((
            format!("GIT_CONFIG_KEY_{}", n + i),
            "url.fleet-push-blocked:///.pushInsteadOf".into(),
        ));
        vars.push((format!("GIT_CONFIG_VALUE_{}", n + i), prefix));
    }
    vars
}

/// What the `open_pr` gate runs the repo's workflow steps under: an orchestrator's env, push block included (a test
/// script that pushes fails like an agent would), less what is the agents' alone. The persona's commit identity and
/// its protocol limit would meet the repo's own tests (a test asserting an author, one fetching over ssh) where CI
/// has neither; the gate's verdict has to be CI's. `GH_CONFIG_DIR` and the cleared credential helpers stay, so the
/// tests never reach the user's own logins.
pub fn gate_vars(persona: &PersonaEnv, origins: &[String]) -> Vec<(String, String)> {
    let mut vars = agent_vars(persona, AgentKind::Orchestrator, origins);
    vars.retain(|(k, _)| {
        !k.starts_with("GIT_AUTHOR_")
            && !k.starts_with("GIT_COMMITTER_")
            && k != "GIT_ALLOW_PROTOCOL"
    });
    vars
}

/// The `origin` URLs `workspaces` are configured with (as written: `pushInsteadOf` matches before any rewriting),
/// for `agent_vars`. A worktree that is gone or has no origin adds nothing.
pub async fn origin_urls(workspaces: &[Workspace]) -> Vec<String> {
    let mut urls = Vec::new();
    for w in workspaces {
        if let Ok(Some(url)) = git::configured_remote_url(Path::new(&w.worktree), "origin").await {
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
    }
    urls
}

impl AgentService {
    fn defs(&self) -> Defs {
        Defs::new(self.cfg.fleet_dir.clone())
    }

    /// The model a definition's agent starts on, read from the product's overrides as they are now, so an override
    /// changed since the task began applies to the next start.
    pub(crate) async fn resolve_model(
        &self,
        product: &str,
        name: &str,
        def_model: Option<&str>,
    ) -> Result<Option<String>, StoreError> {
        let entries = self.store.product_overrides(product).await?;
        Ok(overrides::resolve_model(name, def_model, &entries))
    }

    /// The limits that apply to the product as its overrides are now, so a change applies to the next check.
    async fn limits_of(
        &self,
        product: &str,
    ) -> Result<(LimitValues, [SettingSource; 4]), StoreError> {
        let entries = self.store.product_overrides(product).await?;
        Ok(overrides::resolve_limits(&self.cfg.limits, &entries))
    }

    /// See `ServiceOptions::fleet_gate`.
    async fn fleet_gate(&self) {
        if let Some(gate) = &self.opts.fleet_gate {
            gate.reached.notify_one();
            gate.open.notified().await;
        }
    }

    /// The orchestrator type a spawn asks for, checked before any repo is resolved; None for a solo task.
    pub(crate) fn orchestrator_def(
        &self,
        req: &SpawnRequest,
    ) -> Result<Option<OrchestratorDef>, SpawnError> {
        match req.kind {
            AgentKind::Solo => return Ok(None),
            AgentKind::Specialist => return Err(SpawnError::SpecialistSpawn),
            AgentKind::Orchestrator => {}
        }
        let role = req.role.as_deref().unwrap_or_default();
        if role.is_empty() {
            return Err(FleetError::NotFound {
                kind: DefKind::Orchestrator,
                name: String::new(),
            }
            .into());
        }
        let def = self.defs().orchestrator(role)?;
        let mut inputs: Vec<&str> = req.repos.iter().map(|r| r.repo.trim()).collect();
        inputs.sort();
        inputs.dedup();
        if inputs.len() != 1 {
            return Err(SpawnError::OneRepo);
        }
        Ok(Some(def))
    }

    /// The rest of the checks, once the one repo is resolved: the orchestrator gets every specialist that loads,
    /// less the product's `hidden` ones, and at least one. The repo's workflow is checked after, since that fetches.
    pub(crate) fn orchestrator_plan(
        &self,
        def: OrchestratorDef,
        rows: &[Resolved],
        hidden: &[String],
    ) -> Result<OrchestratorPlan, SpawnError> {
        let [_] = rows else {
            return Err(SpawnError::OneRepo);
        };
        let specialists: Vec<(String, String)> = self
            .defs()
            .list()
            .into_iter()
            .filter(|d| d.kind == DefKind::Specialist && d.error.is_none())
            .filter(|d| !hidden.contains(&d.name))
            .map(|d| (d.name, d.summary))
            .collect();
        if specialists.is_empty() {
            return Err(SpawnError::NoSpecialists);
        }
        Ok(OrchestratorPlan { def, specialists })
    }

    /// Puts the orchestrator's skills and `CLAUDE.md` into its folder. The skills folder is made even when empty,
    /// so it is plain which skills the agent has.
    pub(crate) fn write_orchestrator_files(
        &self,
        plan: &OrchestratorPlan,
        cwd: &Path,
        instructions: &str,
        workspaces: &[Workspace],
        internal: &Path,
        task_name: &str,
    ) -> Result<(), SpawnError> {
        std::fs::create_dir_all(cwd.join(".claude/skills"))?;
        skills::install(&self.defs(), &plan.def.skills, cwd)?;
        std::fs::write(
            cwd.join("CLAUDE.md"),
            plan.claude_md(instructions, workspaces, internal, task_name),
        )?;
        Ok(())
    }

    /// Makes the record the store created for the orchestrator an orchestrated task's: the task folder is the
    /// orchestrator folder's parent, not the agent's cwd.
    pub(crate) async fn record_orchestrator(
        &self,
        id: &AgentId,
        plan: &OrchestratorPlan,
        task_dir: &Path,
    ) -> Result<(), StoreError> {
        let name = task_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let workdir = task_dir.to_string_lossy().into_owned();
        let role = plan.def.name.clone();
        let specialists = plan.specialists.iter().map(|(n, _)| n.clone()).collect();
        self.store
            .update_task(id, move |p| {
                p.kind = AgentKind::Orchestrator;
                p.role = Some(role);
                p.specialists = specialists;
                p.name = name;
                p.workdir = workdir;
            })
            .await
    }
}

impl AgentService {
    /// The orchestrator's `propose_breakdown`: a refused proposal returns at once and changes nothing. Otherwise the
    /// version is stored and rendered, the card goes to Needs you, and the call is held until the owner decides (or
    /// the hold runs out, as `request_repo`'s does).
    pub async fn propose_breakdown(
        self: &Arc<Self>,
        orchestrator: &AgentId,
        drafts: Vec<StepDraft>,
    ) -> Result<BreakdownAnswer, ServiceError> {
        // The store-and-register runs detached, so a client that hangs up cannot leave a stored version with no card
        // or waiter. The hold stays the caller's: a dropped call drops `rx`, and the decision is queued instead.
        let (svc, id) = (self.clone(), orchestrator.clone());
        let rx = detached(async move { svc.store_breakdown(&id, drafts).await }).await?;
        match tokio::time::timeout(self.opts.repo_request_hold, rx).await {
            Ok(Ok(answer)) => Ok(answer),
            // Only a stop or a lost session drops the sender unanswered.
            Ok(Err(_)) => Err(ServiceError::NotRunning),
            Err(_) => Err(ServiceError::BreakdownNoAnswerYet),
        }
    }

    /// `propose_breakdown` up to its hold: stores and renders the version, puts the card on it and registers the
    /// waiter the decision goes to.
    async fn store_breakdown(
        &self,
        orchestrator: &AgentId,
        drafts: Vec<StepDraft>,
    ) -> Result<oneshot::Receiver<BreakdownAnswer>, ServiceError> {
        {
            let _bd = self.breakdown_lock.lock().await;
            let a = self.find(orchestrator).await?;
            if a.kind != AgentKind::Orchestrator {
                return Err(ServiceError::NotAnOrchestrator);
            }
            if a.status == Status::Stopped {
                return Err(ServiceError::NotRunning);
            }
            let task = task_id(&a).clone();
            let p = self
                .store
                .task(&task)
                .await?
                .ok_or(ServiceError::NotFound)?;
            if p.breakdowns
                .last()
                .is_some_and(|v| v.approval == Approval::Proposed)
            {
                return Err(ServiceError::BreakdownPending);
            }
            self.not_gating(&task)?;
            let (limits, sources) = self.limits_of(&p.product).await?;
            if p.breakdowns.len() >= limits.max_breakdown_versions as usize {
                return Err(ServiceError::LimitReached {
                    key: "max_breakdown_versions",
                    value: limits.max_breakdown_versions,
                    source: sources[0],
                });
            }
            // Every proposal restates the carried steps, so its length is the task's whole step count.
            if drafts.len() > limits.max_steps as usize {
                return Err(ServiceError::LimitReached {
                    key: "max_steps",
                    value: limits.max_steps,
                    source: sources[1],
                });
            }
            let now = self.opts.clock.now();
            // Checked before any write, so a refused proposal changes nothing.
            let n = next_version(&p.breakdowns, &drafts, &p.specialists, now)?.version;
            let mut stopped = false;
            self.store
                .update_agent(orchestrator, |x| {
                    // Re-checked in the write: the session can end after the read above.
                    if x.status == Status::Stopped {
                        stopped = true;
                        return;
                    }
                    if x.status != Status::NeedsYou {
                        x.status_since = now;
                    }
                    x.status = Status::NeedsYou;
                    x.blocked_reason = Some(BlockedReason::Breakdown);
                    x.activity = format!("breakdown v{n} ready");
                })
                .await?;
            if stopped {
                return Err(ServiceError::NotRunning);
            }
            self.fleet_gate().await;
            let mut made = None;
            let stored = self
                .store
                .update_task(&task, |p| {
                    // Made again from the record as it is now: a specialist that ended since the read above had its
                    // step failed by the store, and the new version must carry that, not the step as running.
                    let v = next_version(&p.breakdowns, &drafts, &p.specialists, now);
                    if let Ok(v) = &v {
                        p.breakdowns.push(v.clone());
                    }
                    made = Some(v);
                })
                .await
                .map_err(ServiceError::from)
                .and_then(|()| match made {
                    Some(v) => v.map_err(ServiceError::from),
                    None => Err(ServiceError::NotFound),
                });
            let v = match stored {
                Ok(v) => v,
                Err(e) => {
                    // The card must not wait on a proposal nobody can decide.
                    if let Err(e) = self
                        .settle(orchestrator, format!("breakdown v{n} not stored"))
                        .await
                    {
                        warn!(agent_id = %orchestrator.0, error = %e, "cannot take the card off the unstored breakdown");
                    }
                    return Err(e);
                }
            };
            write_breakdown(&p.workdir, &p.goal, &v);
            info!(agent_id = %orchestrator.0, version = n, "breakdown proposed");
            self.log_task(&task, format!("breakdown v{n} proposed"))
                .await;
            let (tx, rx) = oneshot::channel();
            self.breakdown_waiters
                .lock()
                .expect("waiters poisoned")
                .insert(orchestrator.0.clone(), tx);
            // As in `request_repo`: either this sees Stopped, or stop's removal comes after the registration.
            let now_stopped = self
                .find(orchestrator)
                .await
                .map(|a| a.status == Status::Stopped);
            if !matches!(now_stopped, Ok(false)) {
                self.breakdown_waiters
                    .lock()
                    .expect("waiters poisoned")
                    .remove(&orchestrator.0);
                now_stopped?;
                return Err(ServiceError::NotRunning);
            }
            Ok(rx)
        }
    }

    /// The owner's decision on the task's proposed breakdown, handed to the held call or, when none is left,
    /// queued as a fleet event for the lead's `wait_for_fleet`.
    pub async fn decide_breakdown(
        &self,
        task: &AgentId,
        d: DecideBreakdown,
    ) -> Result<(), ServiceError> {
        let _bd = self.breakdown_lock.lock().await;
        let p = self.store.task(task).await?.ok_or(ServiceError::NotFound)?;
        let Some(n) = p
            .breakdowns
            .last()
            .filter(|v| v.approval == Approval::Proposed)
            .map(|v| v.version)
        else {
            return Err(ServiceError::NoPendingBreakdown);
        };
        if let Some(seen) = d.version.filter(|seen| *seen != n) {
            return Err(ServiceError::StaleBreakdown { pending: n, seen });
        }
        // Shown as one line, on the card and in the lead's event.
        let comment = d.comment.as_deref().map(one_line).filter(|c| !c.is_empty());
        let (approval, answer, said) = if d.approve {
            (
                Approval::Approved,
                BreakdownAnswer::Approved,
                "approved".to_string(),
            )
        } else {
            let c = comment.clone().unwrap_or_else(|| "no comment given".into());
            (
                Approval::ChangesRequested,
                BreakdownAnswer::ChangesRequested(c.clone()),
                format!("changes requested: {c}"),
            )
        };
        let now = self.opts.clock.now();
        let mut decided = None;
        self.store
            .update_task(task, |p| {
                if let Some(v) = p.breakdowns.last_mut() {
                    v.approval = approval;
                    v.comment = comment;
                    v.decided_at = Some(now);
                    decided = Some(v.clone());
                }
            })
            .await?;
        if let Some(v) = &decided {
            write_breakdown(&p.workdir, &p.goal, v);
        }
        info!(agent_id = %task.0, version = n, approved = d.approve, "breakdown decided");
        self.log_task(task, format!("breakdown v{n} {said}")).await;
        let lead = p.lead.clone().unwrap_or_else(|| p.id.clone());
        // An archived task has no agent left to tell.
        if self.find(&lead).await.is_err() {
            return Ok(());
        }
        // Only a card still on the breakdown is settled: a prompt that came up since (a permission, a question)
        // owns it until it is answered.
        let mut stored = None;
        self.store
            .update_agent(&lead, |x| {
                stored = Some(x.clone());
                if x.status == Status::Stopped || x.blocked_reason != Some(BlockedReason::Breakdown)
                {
                    return;
                }
                if x.status != Status::Working {
                    x.status_since = now;
                }
                x.status = Status::Working;
                x.blocked_reason = None;
                x.activity = format!("breakdown v{n} {said}");
            })
            .await?;
        let stopped = stored.as_ref().is_some_and(|x| x.status == Status::Stopped);
        // A session that ended is Stopped before its held call is dropped; an answer sent there is never read.
        if stopped {
            self.breakdown_waiters
                .lock()
                .expect("waiters poisoned")
                .remove(&lead.0);
        } else if self.answer_breakdown(&lead, answer) {
            return Ok(());
        }
        // No held call: the lead hears it from `wait_for_fleet`, now if it is held there, else on its next call or
        // in a restart's prompt. The decision itself is never typed: a session parked in `wait_for_fleet` reads no
        // input.
        let detail = if d.approve {
            format!("breakdown v{n} approved")
        } else {
            format!("breakdown v{n}: {said}")
        };
        let ev = FleetEvent {
            at: now,
            step: 0,
            agent: task.clone(),
            kind: FleetEventKind::BreakdownDecided,
            detail,
        };
        self.store.update_task(task, |p| p.inbox.push(ev)).await?;
        // The board derived Needs you from the pending version; this also wakes a held `wait_for_fleet` at once. The
        // decision is already stored and queued, so a failure here only leaves the card stale until its next change.
        // `breakdown_lock` serialises restart against decide, so the lead read at the start is still the current one.
        if let Err(e) = self.store.announce(&lead).await {
            warn!(agent_id = %lead.0, error = %e, "cannot show the decided breakdown on the card");
        }
        // A lead whose turn ended makes no further call, so it would never read the inbox: one line tells it to.
        // Only from Review: Working reaches `wait_for_fleet` on its own, and Needs you may have a prompt up.
        if let Some(a) = stored.filter(|x| x.status == Status::Review) {
            self.type_line(
                &a,
                &format!("Fleet Manager: breakdown v{n} decided; call wait_for_fleet"),
            )
            .await;
        }
        Ok(())
    }

    /// Every breakdown version and gate run of a task, for its side panel.
    pub async fn fleet_view(&self, task: &AgentId) -> Result<FleetView, ServiceError> {
        let p = self.store.task(task).await?.ok_or(ServiceError::NotFound)?;
        Ok(FleetView {
            task: p.id,
            breakdowns: p.breakdowns,
            gate_runs: p.gate_runs,
        })
    }

    /// Hands the decision to the held call; false when there is none any more (gone, timed out, stopped).
    fn answer_breakdown(&self, id: &AgentId, answer: BreakdownAnswer) -> bool {
        let tx = self
            .breakdown_waiters
            .lock()
            .expect("waiters poisoned")
            .remove(&id.0);
        tx.is_some_and(|tx| tx.send(answer).is_ok())
    }
}

impl AgentService {
    /// The orchestrator's `spawn_specialist`: starts the specialist for step `n` of the approved breakdown in its own
    /// folder `<task>/fleet/<n>-<slug>/`, with a worktree on a step branch cut from the task branch's tip as
    /// the orchestrator's checkout has it now. A refusal makes nothing; a failure undoes what was made.
    pub async fn spawn_specialist(
        self: &Arc<Self>,
        orchestrator: &AgentId,
        n: u32,
    ) -> Result<Agent, ServiceError> {
        // Detached: a client that hangs up mid-launch must not leave a folder, worktree or card with no step.
        let (svc, id) = (self.clone(), orchestrator.clone());
        detached(async move { svc.spawn_specialist_now(&id, n).await }).await
    }

    #[tracing::instrument(skip_all, fields(agent_id = %orchestrator.0, step = n))]
    async fn spawn_specialist_now(
        &self,
        orchestrator: &AgentId,
        n: u32,
    ) -> Result<Agent, ServiceError> {
        // Before the lock: the logins are asked through the CLIs, and a task's persona never changes.
        let persona = self.persona_of(&self.find(orchestrator).await?).await?;
        let fleet = fleet_config(&self.cfg.agent_config_dir).map_err(ServiceError::AgentConfig)?;
        // Held through the whole spawn, so two calls for one step never both find it Pending, and no proposal or
        // decision changes the breakdown in between.
        let _bd = self.breakdown_lock.lock().await;
        let lead = self.find(orchestrator).await?;
        let pid = task_id(&lead).clone();
        let task = self.store.task(&pid).await?.ok_or(ServiceError::NotFound)?;
        if lead.kind != AgentKind::Orchestrator
            || task.lead.as_ref().unwrap_or(&task.id) != &lead.id
        {
            return Err(refuse(
                "only the task's orchestrator starts its specialists",
            ));
        }
        if lead.status == Status::Stopped {
            return Err(ServiceError::NotRunning);
        }
        self.not_gating(&pid)?;
        let step = ready_step(&task.breakdowns, n)?.clone();
        // After the step is known to be startable, so a step that is refused anyway says why, not "wait". Counts
        // only what runs now: a lowered limit never stops an agent, it refuses the next spawn.
        let (limits, sources) = self.limits_of(&task.product).await?;
        let running = self
            .store
            .snapshot()
            .await?
            .agents
            .iter()
            .filter(|a| {
                a.kind == AgentKind::Specialist && task_id(a) == &pid && a.status != Status::Stopped
            })
            .count();
        if running >= limits.max_parallel_specialists as usize {
            return Err(ServiceError::TooManyRunning {
                running,
                value: limits.max_parallel_specialists,
                source: sources[2],
            });
        }
        let def = self
            .defs()
            .specialist(&step.specialist)
            .map_err(|e| refuse(e.to_string()))?;
        let [lead_workspace] = &lead.workspaces[..] else {
            return Err(ServiceError::OneRepo);
        };
        // A task made before internal notes existed has no folder: no snapshot, and nothing to refuse.
        let internal = Path::new(&task.workdir).join(INTERNAL_DIR);
        if internal.is_dir()
            && self
                .internal
                .is_dirty(&internal)
                .await
                .map_err(|e| refuse(format!("cannot check the task's internal notes: {e}")))?
        {
            return Err(refuse(format!(
                "commit your internal resources first: {}",
                internal_commit_command(&internal)
            )));
        }
        let plan = SpecialistPlan {
            task,
            step,
            def,
            lead_workspace: lead_workspace.clone(),
            persona,
            fleet,
        };

        let id = uuid::Uuid::new_v4().simple().to_string();
        let slug = git::slugify(&plan.step.title);
        let folder = Path::new(&plan.task.workdir)
            .join(FLEET_DIR)
            .join(format!("{n}-{slug}"));
        // Registered before the folder exists, so neither reconcile nor the orphan scan acts on a half-made agent.
        let _guard = self.spawning_guard(&id, &folder);
        let mut made = Made {
            undo: Undo {
                made: vec![],
                workdir: folder.clone(),
                settings_dir: self.agent_dir(&id),
                internal: None,
            },
            snapshot: None,
            card: None,
            step_running: false,
        };
        // Not create_dir_all for the agent's own folder: one already there is not this spawn's, and undo deletes it.
        let created = std::fs::create_dir_all(folder.parent().expect("under the fleet folder"))
            .and_then(|()| std::fs::create_dir(&folder));
        if let Err(e) = created {
            return Err(e.into());
        }
        match self
            .start_specialist(&plan, &id, &slug, &folder, &mut made)
            .await
        {
            Ok(a) => {
                info!(agent_id = %id, step = n, branch = %a.workspaces[0].branch, "specialist spawned");
                Ok(a)
            }
            Err(e) => {
                self.unmake(&plan.task, n, &made, &e).await;
                Err(e)
            }
        }
    }

    /// Everything of a specialist spawn that can fail once its folder exists, noting in `made` what is there to undo.
    async fn start_specialist(
        &self,
        plan: &SpecialistPlan,
        id: &str,
        slug: &str,
        folder: &Path,
        made: &mut Made,
    ) -> Result<Agent, ServiceError> {
        let n = plan.step.n;
        let lw = &plan.lead_workspace;
        // The checkout the orchestrator's worktree hangs off: the persona's clone or the repo's local path.
        let base = self
            .repos
            .base_of(
                Path::new(&lw.worktree),
                &plan.persona.id,
                &plan.task.product,
                &lw.repo,
            )
            .await
            .ok_or_else(|| refuse(format!("repo {} is no longer on disk", lw.repo)))?;
        let branch = step_branch(&lw.branch, n, slug);
        // The task branch itself, with every step merged so far: the orchestrator's checkout may be detached or
        // on another branch, and its HEAD would then be the wrong start.
        let wt = self
            .repos
            .attach_from_branch(&base, &lw.repo, &lw.branch, &folder.join(&lw.repo), &branch)
            .await?;
        let workspaces = vec![Workspace {
            repo: lw.repo.clone(),
            branch: wt.branch.clone(),
            worktree: wt.path.to_string_lossy().into_owned(),
            base: wt.base.clone(),
            from_branch: wt.from_branch.clone(),
            has_changes: false,
            owned: false,
            changed: false,
        }];
        made.undo.made.push((base, wt));

        // The notes as committed now. Named like the step folder so the two line up; a later commit of the
        // orchestrator's does not reach it.
        let snapshot = if Path::new(&plan.task.workdir).join(INTERNAL_DIR).is_dir() {
            let dest = Path::new(&plan.task.workdir)
                .join(SNAPSHOTS_DIR)
                .join(format!("{n}-{slug}"));
            std::fs::create_dir_all(dest.parent().expect("under the snapshots folder"))?;
            made.snapshot = Some(dest.clone());
            let sha = self
                .internal
                .snapshot(&plan.task.product, &plan.task.name, &dest)
                .await
                .map_err(|e| refuse(format!("cannot snapshot the task's internal notes: {e}")))?;
            Some(Snapshot { path: dest, sha })
        } else {
            None
        };

        let agent_id = AgentId(id.to_string());
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let settings_dir = &made.undo.settings_dir;
        let written = write_settings(
            settings_dir,
            &self.opts.base_url,
            &agent_id,
            &task_repo_rules(&workspaces),
            &plan.fleet.settings,
            &match &snapshot {
                Some(s) => RoleSettings::specialist(&plan.def.tools).with_snapshot(&s.path),
                None => RoleSettings::specialist(&plan.def.tools),
            },
        )
        .and_then(|s| {
            Ok((
                s,
                write_mcp_config(settings_dir, &self.opts.base_url, &agent_id, &token)?,
            ))
        });
        let (settings, mcp) =
            written.map_err(|e| refuse(format!("cannot write the agent's settings: {e}")))?;
        std::fs::create_dir_all(folder.join(".claude/skills"))?;
        skills::install(&self.defs(), &plan.def.skills, folder)
            .map_err(|e| refuse(e.to_string()))?;
        std::fs::write(
            folder.join("CLAUDE.md"),
            specialist_claude_md(
                &plan.def,
                &plan.fleet.instructions,
                &workspaces,
                snapshot.as_ref(),
            ),
        )?;
        write_prompt(folder, &specialist_prompt(&plan.step, &plan.task.goal))?;

        let session = format!("{SESSION_PREFIX}{}", &id[..8]);
        let model = self
            .resolve_model(
                &plan.task.product,
                &plan.def.name,
                plan.def.model.as_deref(),
            )
            .await?;
        // Stored before the session starts: SessionStart can arrive within milliseconds of the process starting.
        let agent = Agent {
            id: agent_id.clone(),
            goal: plan.step.title.chars().take(120).collect(),
            workdir: folder.to_string_lossy().into_owned(),
            workspaces,
            session: session.clone(),
            status: Status::Queued,
            activity: "starting".into(),
            status_since: self.opts.clock.now(),
            has_changes: false,
            paused: false,
            owner: self.opts.owner.clone(),
            blocked_reason: None,
            repo_request: None,
            stop_cause: None,
            disk_bytes: None,
            persona: plan.persona.id.clone(),
            product: plan.task.product.clone(),
            kind: AgentKind::Specialist,
            task: Some(plan.task.id.clone()),
            role: Some(plan.def.name.clone()),
            step: Some(n),
            model,
            attach: None,
        };
        // Re-checks the persona in the insert's transaction, so a persona deleted meanwhile leaves no card behind.
        self.store
            .insert_agent_under_persona(&agent, &token)
            .await
            .map_err(agent_insert_error::<ServiceError>)?;
        made.card = Some(agent_id.clone());

        let (who, step_branch) = (agent_id.clone(), branch.clone());
        let mut latest = None;
        self.store
            .update_task(&plan.task.id, |p| {
                update_step(p, n, |s| {
                    s.state = StepState::Running;
                    s.agent = Some(who);
                    s.branch = Some(step_branch);
                });
                latest = p.breakdowns.last().cloned();
            })
            .await?;
        made.step_running = true;
        if let Some(v) = &latest {
            write_breakdown(&plan.task.workdir, &plan.task.goal, v);
        }

        plan.persona.seed_first_run().await;
        let origins = origin_urls(&agent.workspaces).await;
        let vars = agent_vars(&plan.persona, AgentKind::Specialist, &origins);
        self.launch(
            &agent.id,
            &session,
            folder,
            &settings,
            &mcp,
            &token,
            &vars,
            agent.model.as_deref(),
            Start::New,
        )
        .await?;
        Ok(agent)
    }

    /// Undoes a failed specialist spawn: the step goes back to Pending, the card goes, then the files and worktree.
    async fn unmake(&self, task: &Task, n: u32, made: &Made, error: &ServiceError) {
        if made.step_running {
            let mut latest = None;
            let r = self
                .store
                .update_task(&task.id, |p| {
                    update_step(p, n, |s| {
                        s.state = StepState::Pending;
                        s.agent = None;
                        s.branch = None;
                    });
                    latest = p.breakdowns.last().cloned();
                })
                .await;
            match r {
                Ok(()) => {
                    if let Some(v) = &latest {
                        write_breakdown(&task.workdir, &task.goal, v);
                    }
                }
                Err(e) => warn!(step = n, error = %e, "cannot put the step back to pending"),
            }
        }
        if let Some(id) = &made.card {
            if let Err(e) = self.store.remove_agent(id).await {
                warn!(agent_id = %id.0, error = %e, "cannot remove the failed specialist's card");
            }
        }
        if let Some(dest) = &made.snapshot {
            // remove_worktree needs the repo; a snapshot that never got made is only an empty folder.
            if let Err(e) = self.internal.remove_worktree(&task.product, dest).await {
                warn!(step = n, error = %e, "cannot remove the failed specialist's snapshot");
            }
            let _ = std::fs::remove_dir_all(dest);
            // Only if it is empty: another step's snapshot may be in it.
            let _ = dest.parent().map(std::fs::remove_dir);
        }
        self.undo(&made.undo, None).await;
        warn!(step = n, error = %error, "specialist spawn failed");
        self.log_task(
            &task.id,
            format!("step {n} could not start: {}", one_line(&error.to_string())),
        )
        .await;
    }

    /// A specialist's `finish_step`: refused while its worktree has anything uncommitted. Otherwise the step is Done
    /// with `report`, and the agent stops as Finished, which tells its orchestrator through the inbox.
    pub async fn finish_step(
        self: &Arc<Self>,
        specialist: &AgentId,
        report: &str,
    ) -> Result<(), ServiceError> {
        // Detached: a client that hangs up between the two writes would leave the agent stopped as Finished with its
        // step still running.
        let (svc, id, report) = (self.clone(), specialist.clone(), report.to_string());
        detached(async move { svc.finish_step_now(&id, &report).await }).await
    }

    async fn finish_step_now(
        &self,
        specialist: &AgentId,
        report: &str,
    ) -> Result<(), ServiceError> {
        let a = self.find(specialist).await?;
        if a.kind != AgentKind::Specialist {
            return Err(refuse("only a specialist finishes a step"));
        }
        if a.status == Status::Stopped {
            return Err(ServiceError::NotRunning);
        }
        for w in &a.workspaces {
            if git::is_dirty(Path::new(&w.worktree)).await? {
                return Err(refuse("commit your changes first"));
            }
        }
        let n = a.step.unwrap_or(0);
        let pid = task_id(&a).clone();
        let _bd = self.breakdown_lock.lock().await;
        let now = self.opts.clock.now();
        let mut stopped = false;
        // The agent first: one that ended meanwhile has its step failed by the store, which Done must not undo.
        self.store
            .update_agent(specialist, |x| {
                if x.status == Status::Stopped {
                    stopped = true;
                    return;
                }
                x.status = Status::Stopped;
                x.stop_cause = Some(StopCause::Finished);
                x.activity = format!("finished step {n}");
                x.status_since = now;
                x.paused = false;
                x.blocked_reason = None;
                x.repo_request = None;
            })
            .await?;
        if stopped {
            return Err(ServiceError::NotRunning);
        }
        self.fleet_gate().await;
        let report = report.trim().to_string();
        let mut record = None;
        self.store
            .update_task(&pid, |p| {
                update_step(p, n, |s| {
                    s.state = StepState::Done;
                    s.report = Some(report);
                });
                record = Some((
                    p.workdir.clone(),
                    p.goal.clone(),
                    p.breakdowns.last().cloned(),
                ));
            })
            .await?;
        if let Some((workdir, goal, Some(v))) = &record {
            write_breakdown(workdir, goal, v);
        }
        info!(agent_id = %specialist.0, step = n, "step finished");
        self.log_task(&pid, format!("step {n} finished")).await;
        // After the writes, so the session's end finds the agent already Stopped and names no other cause.
        if let Err(e) = self.kill_or_gone(&a).await {
            warn!(agent_id = %specialist.0, error = %e, "cannot end the finished specialist's session");
        }
        Ok(())
    }
}

impl AgentService {
    /// The orchestrator's task and its record, for a caller that must be that task's current orchestrator.
    async fn followed_task(&self, orchestrator: &AgentId) -> Result<(Agent, Task), ServiceError> {
        let a = self.find(orchestrator).await?;
        if a.kind != AgentKind::Orchestrator {
            return Err(ServiceError::NotAnOrchestrator);
        }
        let p = self
            .store
            .task(task_id(&a))
            .await?
            .ok_or(ServiceError::NotFound)?;
        if p.lead.as_ref().unwrap_or(&p.id) != &a.id {
            return Err(refuse("only the task's orchestrator follows its fleet"));
        }
        if a.status == Status::Stopped {
            return Err(ServiceError::NotRunning);
        }
        Ok((a, p))
    }

    /// Waits for the first events queued for the orchestrator, within the hold; empty when nothing happened. They are
    /// only peeked: the caller acks them once they are delivered.
    async fn peek_fleet(
        &self,
        orchestrator: &AgentId,
    ) -> Result<(Task, Vec<FleetEvent>), ServiceError> {
        let (_, p) = self.followed_task(orchestrator).await?;
        // Subscribed before the first look, so an event landing between the look and the wait wakes it.
        let mut live = self.store.subscribe();
        let deadline = tokio::time::Instant::now() + self.opts.repo_request_hold;
        loop {
            let events = self.store.peek_inbox(&p.id).await?;
            if !events.is_empty() {
                return Ok((p, events));
            }
            // After the peek, so events queued before a stop are still delivered, and the stop is seen on any wake.
            self.followed_task(orchestrator).await?;
            let tick = (tokio::time::Instant::now() + WAIT_TICK).min(deadline);
            match tokio::time::timeout_at(tick, live.recv()).await {
                // A lagged receiver only missed wake-ups, and the inbox is the truth.
                Ok(Ok(_) | Err(broadcast::error::RecvError::Lagged(_))) => {}
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    return Err(refuse("the event stream closed"))
                }
                Err(_) if tokio::time::Instant::now() >= deadline => return Ok((p, vec![])),
                Err(_) => {}
            }
        }
    }

    /// The orchestrator's `wait_for_fleet`: the events queued since its last call, or, when there are none, the
    /// first that arrive within the hold. Empty when nothing happened.
    pub async fn wait_for_fleet(
        &self,
        orchestrator: &AgentId,
    ) -> Result<Vec<FleetEvent>, ServiceError> {
        let (p, events) = self.peek_fleet(orchestrator).await?;
        self.store.ack_inbox(&p.id, events.len()).await?;
        Ok(events)
    }

    /// `wait_for_fleet` as the MCP tool tells it, one line per event. The inbox is acked last, so a dropped request
    /// or a failed render leaves the events for the next call.
    pub async fn wait_for_fleet_lines(
        &self,
        orchestrator: &AgentId,
    ) -> Result<Vec<String>, ServiceError> {
        let (p, events) = self.peek_fleet(orchestrator).await?;
        if events.is_empty() {
            return Ok(vec![]);
        }
        // Under the lock `finish_step` holds across its two writes: its Finished event is queued by the first, and the
        // report it renders comes with the second.
        let lines = {
            let _bd = self.breakdown_lock.lock().await;
            self.fleet_event_lines(&p.id, &events).await?
        };
        self.store.ack_inbox(&p.id, events.len()).await?;
        Ok(lines)
    }

    /// The orchestrator's `fleet_status`: the latest breakdown version with where each step stands.
    pub async fn fleet_status(&self, orchestrator: &AgentId) -> Result<FleetStatus, ServiceError> {
        let (_, p) = self.followed_task(orchestrator).await?;
        let Some(v) = p.breakdowns.last() else {
            return Err(refuse("no breakdown yet"));
        };
        let agents = self.store.task_agents(&p.id).await?;
        let steps = v
            .steps
            .iter()
            .map(|s| {
                let live = s
                    .agent
                    .as_ref()
                    .and_then(|id| agents.iter().find(|a| &a.id == id))
                    .map(|a| a.status);
                (s.n, s.state, live)
            })
            .collect();
        Ok(FleetStatus {
            version: v.version,
            approval: v.approval,
            steps,
        })
    }

    /// One line per event, as `wait_for_fleet` tells the orchestrator.
    async fn fleet_event_lines(
        &self,
        task: &AgentId,
        events: &[FleetEvent],
    ) -> Result<Vec<String>, ServiceError> {
        let p = self.store.task(task).await?.ok_or(ServiceError::NotFound)?;
        let agents = self.store.task_agents(&p.id).await?;
        let lines = events
            .iter()
            .map(|e| {
                // The step as the latest version that still has it describes it; the agent's role is the fallback.
                let step = p
                    .breakdowns
                    .iter()
                    .rev()
                    .find_map(|v| v.steps.iter().find(|s| s.n == e.step));
                let who = step.map(|s| s.specialist.clone()).or_else(|| {
                    agents
                        .iter()
                        .find(|x| x.id == e.agent)
                        .and_then(|x| x.role.clone())
                });
                let head = match &who {
                    Some(w) => format!("step {} ({w})", e.step),
                    None => format!("step {}", e.step),
                };
                match e.kind {
                    FleetEventKind::Finished => {
                        let report = step.and_then(|s| s.report.as_deref()).unwrap_or("");
                        format!("{head}: finished: {}", report.lines().next().unwrap_or(""))
                    }
                    FleetEventKind::NeedsYou => format!("{head}: needs you: {}", e.detail),
                    FleetEventKind::Failed => {
                        format!("{head}: stopped without finish_step ({})", e.detail)
                    }
                    FleetEventKind::IdleWithoutFinish => {
                        format!("step {}: ended its turn without finish_step", e.step)
                    }
                    FleetEventKind::BreakdownDecided => e.detail.clone(),
                }
            })
            .collect();
        Ok(lines)
    }
}

/// Takes the task off `gating` when its `open_pr` ends, however it ends.
struct Gating<'a>(&'a std::sync::Mutex<HashSet<String>>, String);

impl Drop for Gating<'_> {
    fn drop(&mut self) {
        self.0.lock().expect("gating poisoned").remove(&self.1);
    }
}

fn last_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("no output")
        .to_string()
}

/// What the PR merges into: the branch the task started from, else the remote's default branch.
async fn pr_base(w: &Workspace, wt: &Path) -> String {
    let named = |b: &str| b.strip_prefix("origin/").unwrap_or(b).to_string();
    if let Some(b) = &w.from_branch {
        return named(b);
    }
    match git::origin_head(wt).await {
        Ok(Some((_, b))) => named(&b),
        _ => "main".into(),
    }
}

impl AgentService {
    /// Refuses a change to the breakdown while the task's `open_pr` runs: the gate tests what it checked.
    fn not_gating(&self, task: &AgentId) -> Result<(), ServiceError> {
        if self
            .gating
            .lock()
            .expect("gating poisoned")
            .contains(&task.0)
        {
            return Err(refuse(
                "the tests before the PR are running; wait for open_pr",
            ));
        }
        Ok(())
    }

    /// The orchestrator's `open_pr`, once no step of the approved plan is pending or running: runs the steps of the
    /// repo's workflow (read from `origin/<base>`) that the change needs, in its worktree, on the task branch, with
    /// the persona's env, and records the run. Only if every step passes is the task branch pushed and the PR
    /// opened with the persona's `gh`. Returns the PR's URL.
    #[tracing::instrument(skip_all, fields(agent_id = %orchestrator.0))]
    pub async fn open_pr(
        &self,
        orchestrator: &AgentId,
        title: &str,
        body: &str,
    ) -> Result<String, ServiceError> {
        let a = self.find(orchestrator).await?;
        if a.kind != AgentKind::Orchestrator {
            return Err(ServiceError::NotAnOrchestrator);
        }
        // Before the lock: the logins are asked through the CLIs, and a task's persona never changes.
        let persona = self.persona_of(&a).await?;
        let (lead, p, failed, _gating) = {
            let _bd = self.breakdown_lock.lock().await;
            let lead = self.find(orchestrator).await?;
            let p = self
                .store
                .task(task_id(&lead))
                .await?
                .ok_or(ServiceError::NotFound)?;
            if p.lead.as_ref().unwrap_or(&p.id) != &lead.id {
                return Err(refuse("only the task's orchestrator opens its PR"));
            }
            if lead.status == Status::Stopped {
                return Err(ServiceError::NotRunning);
            }
            let plan = approved_plan(&p.breakdowns)?;
            // A failed step is never respawned (its retry is another step), so only open work holds the PR back.
            if let Some(s) = plan
                .steps
                .iter()
                .find(|s| matches!(s.state, StepState::Pending | StepState::Running))
            {
                return Err(refuse(format!("step {} is {}", s.n, state_word(s.state))));
            }
            let failed: Vec<String> = plan
                .steps
                .iter()
                .filter(|s| s.state == StepState::Failed)
                .map(|s| s.n.to_string())
                .collect();
            // Marked under the lock, so no proposal or spawn lands between the check above and the push.
            if !self
                .gating
                .lock()
                .expect("gating poisoned")
                .insert(p.id.0.clone())
            {
                return Err(refuse("open_pr is already running"));
            }
            let gating = Gating(&self.gating, p.id.0.clone());
            (lead, p, failed, gating)
        };
        let [w] = &lead.workspaces[..] else {
            return Err(ServiceError::OneRepo);
        };
        let wt = Path::new(&w.worktree);
        if git::is_dirty(wt).await? || git::merging(wt).await? {
            return Err(refuse("commit or abort the merge first"));
        }
        if git::current_branch(wt).await?.as_deref() != Some(w.branch.as_str()) {
            return Err(refuse(format!("check out {} first", w.branch)));
        }
        let base = pr_base(w, wt).await;
        // Fetched with the persona's credentials, as the push will be: a remote the gate cannot read refuses here.
        let wf = {
            // The worktree's refs live in the base clone, which other tasks fetch into too.
            let checkout = self
                .repos
                .base_of(wt, &lead.persona, &lead.product, &w.repo)
                .await;
            let _base = match &checkout {
                Some(b) => Some(self.repos.lock_base(b).await),
                None => {
                    warn!(repo = %w.repo, "repo checkout not found; fetching the base branch without its lock");
                    None
                }
            };
            workflow::fetch(wt, &base, &persona.vars).await
        };
        let (wf, base_sha) = match wf {
            Ok(x) => x,
            Err(e @ (WorkflowError::Missing { .. } | WorkflowError::Invalid { .. })) => {
                return Err(refuse(format!(
                    "{e}; fix it on {base} and call open_pr again"
                )))
            }
            Err(e @ (WorkflowError::Fetch(_) | WorkflowError::FetchTimedOut)) => {
                return Err(refuse(e.to_string()))
            }
            Err(WorkflowError::Git(e)) => return Err(e.into()),
        };

        // What is tested is what is pushed, whatever the branch does meanwhile.
        let tested = git::head_commit(wt).await?;
        let from = git::merge_base(wt, &base_sha, &tested).await?;
        let selection = workflow::select(wt, &from, &tested, &wf).await?;
        if selection.changed == 0 {
            return Err(refuse(format!("nothing changed against {base}")));
        }
        if selection.run.is_empty() {
            // Only ignored files changed and every step has `paths`: a gate that checks nothing is no gate.
            return Err(refuse(format!(
                "no workflow step runs for these changes (every changed file is ignored); add a step without \
                 `paths` on {base}"
            )));
        }
        let at = self.opts.clock.now();
        let budget = Duration::from_secs(self.cfg.test_timeout_secs);
        let gate_env = gate_vars(&persona, &origin_urls(&lead.workspaces).await);
        // Every step runs under it; on Windows without Git Bash, that is the refusal, with its fix.
        let shell = self
            .opts
            .tools
            .posix_shell()
            .await
            .map_err(|e| ServiceError::Tool(Box::new(e)))?;
        let started = Instant::now();
        let mut steps = Vec::new();
        for picked in selection.run {
            // One budget for the whole gate; a step that starts with none left still gets a moment to fail cleanly.
            let left = budget
                .saturating_sub(started.elapsed())
                .max(Duration::from_secs(1));
            let out = gate::run(&shell, &picked.step.run, wt, &gate_env, left)
                .await
                .map_err(|e| refuse(format!("cannot run step {}: {e}", picked.step.name)))?;
            let ok = out.exit_code == Some(0);
            steps.push(GateStep {
                name: picked.step.name,
                command: picked.step.run,
                because: picked.because,
                exit_code: out.exit_code,
                duration_secs: out.duration_secs,
                output_tail: out.output_tail,
            });
            if !ok {
                break;
            }
        }
        let mut run = GateRun {
            at,
            base: base.clone(),
            steps,
            unmatched: selection.unmatched,
            pr_url: None,
        };
        let stored = run.clone();
        self.store
            .update_task(&p.id, |x| x.gate_runs.push(stored))
            .await?;
        let results: Vec<String> = run
            .steps
            .iter()
            .map(|s| match s.exit_code {
                Some(0) => format!("{} ok", s.name),
                Some(code) => format!("{} exit {code}", s.name),
                None => format!("{} timed out", s.name),
            })
            .collect();
        let secs = started.elapsed().as_secs();
        info!(base = %base, steps = ?results, secs, failed = ?failed, "test gate ran");
        let mut line = format!("tests ran on {base}: {} ({secs}s)", results.join(", "));
        if !run.unmatched.is_empty() {
            line.push_str(&format!(
                "; every step: unmatched {}",
                one_line(&run.unmatched.join(", "))
            ));
        }
        if !failed.is_empty() {
            line.push_str(&format!("; failed steps: {}", failed.join(", ")));
        }
        self.log_task(&p.id, line).await;
        if !run.passed() {
            return Err(ServiceError::GateFailed(run));
        }

        // Only HEAD matters: the pushed sha is pinned, so what the tests left in the tree goes nowhere.
        if git::head_commit(wt).await? != tested {
            return Err(refuse(
                "the branch moved while the tests ran; call open_pr again",
            ));
        }
        // The tests can run for half an hour; an orchestrator stopped meanwhile was stopped from pushing too.
        if self.find(&lead.id).await?.status == Status::Stopped {
            return Err(ServiceError::NotRunning);
        }
        let push = git::push_commit(wt, &tested, &w.branch, &persona.vars);
        match tokio::time::timeout(git::REMOTE_TIMEOUT, push).await {
            Ok(Ok(())) => {
                if let Err(e) = git::set_upstream(wt, &w.branch).await {
                    warn!(error = %e, "cannot set the task branch's upstream");
                }
            }
            Ok(Err(e)) => {
                let detail = match &e {
                    git::GitError::Git { stderr, .. } => last_line(stderr),
                    other => other.to_string(),
                };
                return Err(refuse(format!("push failed: {detail}")));
            }
            Err(_) => return Err(refuse("push failed: timed out")),
        }
        let mut gh = Command::new(&self.cfg.gh_command);
        git::harden(gh.as_std_mut(), &persona.vars);
        gh.args(["pr", "create", "--head", &w.branch, "--base", &base])
            // One argv element each, so a title or body starting with `-` is never read as an option.
            .args([format!("--title={title}"), format!("--body={body}")])
            .current_dir(wt)
            .stdin(Stdio::null())
            .kill_on_drop(true);
        // The branch is up by now; the text says so, as the orchestrator may otherwise push again.
        let url = match tokio::time::timeout(git::REMOTE_TIMEOUT, gh.output()).await {
            Ok(Ok(o))
                if o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty() =>
            {
                last_line(&String::from_utf8_lossy(&o.stdout))
            }
            Ok(Ok(o)) if o.status.success() => {
                return Err(refuse(
                    "push failed: the branch is pushed, but gh pr create printed no URL",
                ))
            }
            Ok(Ok(o)) => {
                return Err(refuse(format!(
                    "push failed: the branch is pushed, but gh pr create said: {}",
                    last_line(&String::from_utf8_lossy(&o.stderr))
                )))
            }
            Ok(Err(e)) => {
                return Err(refuse(format!(
                    "push failed: the branch is pushed, but gh cannot run: {e}"
                )))
            }
            Err(_) => {
                return Err(refuse(
                    "push failed: the branch is pushed, but gh pr create timed out",
                ))
            }
        };

        run.pr_url = Some(url.clone());
        self.store
            .update_task(&p.id, |x| {
                if let Some(r) = x.gate_runs.iter_mut().rev().find(|r| r.at == run.at) {
                    r.pr_url = run.pr_url.clone();
                }
                // The merge watcher's next reply replaces this with what GitHub says; until then it is open.
                if !x.prs.iter().any(|pr| pr.url == url) {
                    x.prs.push(TaskPr {
                        url: url.clone(),
                        repo: w.repo.clone(),
                        state: PrState::Open,
                        merged_at: None,
                    });
                }
            })
            .await?;
        info!(url = %url, "PR opened");
        self.log_task(&p.id, format!("PR opened: {url}")).await;
        self.merge_internal_notes(&p).await;
        let now = self.opts.clock.now();
        self.store
            .update_agent(&lead.id, |x| {
                if x.status == Status::Stopped {
                    return;
                }
                if x.status != Status::Review {
                    x.status_since = now;
                }
                x.status = Status::Review;
                x.blocked_reason = None;
                x.activity = "PR open".into();
            })
            .await?;
        Ok(url)
    }
}

impl AgentService {
    /// Merges the task's internal notes into the product's, once its PR exists. The PR is already open, so a
    /// failure is only logged on the task; the next PR retries the same merge.
    async fn merge_internal_notes(&self, p: &Task) {
        // A task made before internal notes existed has no folder: nothing to merge and nothing to say.
        if !Path::new(&p.workdir).join(INTERNAL_DIR).is_dir() {
            return;
        }
        let env = self.internal_env(&p.product).await;
        let merged = self
            .internal
            .merge_task(
                &p.product,
                &p.name,
                env.as_ref()
                    .ok()
                    .and_then(Option::as_ref)
                    .map(|e| e.vars.as_slice()),
                self.opts.clock.now(),
            )
            .await;
        let line = match merged {
            Ok(mut report) => {
                // Connected, but merged without the remote: the push failed, it is not a local-only product.
                if let Err(why) = env {
                    self.internal.set_push_error(&p.product, why.clone());
                    report.push.outcome = PushOutcome::Failed(why);
                }
                merge_line(&p.name, &report)
            }
            Err(e) => Some(format!("internal notes not merged: {e}")),
        };
        let Some(line) = line else { return };
        self.log_task(&p.id, line).await;
    }
}

const PARKED_NOTE: &str =
    "earlier unpushed notes conflicted with the remote and are on {} to merge by hand";

/// The task-log line for a notes merge, its push and a park; `None` when there is nothing to say.
fn merge_line(task: &str, report: &MergeReport) -> Option<String> {
    let line = outcome_line(task, &report.merge, &report.push.outcome);
    let Some(branch) = &report.push.parked else {
        return line;
    };
    let parked = PARKED_NOTE.replace("{}", branch);
    Some(match line {
        Some(line) => format!("{line}; {parked}"),
        None => format!("internal notes; {parked}"),
    })
}

fn outcome_line(task: &str, merge: &MergeOutcome, push: &PushOutcome) -> Option<String> {
    match (merge, push) {
        (MergeOutcome::Merged, PushOutcome::Pushed) => {
            Some("internal notes merged and pushed".to_string())
        }
        (MergeOutcome::Merged, PushOutcome::Failed(e))
        | (MergeOutcome::UpToDate, PushOutcome::Failed(e)) => Some(format!(
            "internal notes merged, not pushed: {e}; the next merge or a daemon restart retries"
        )),
        (MergeOutcome::Merged, _) => {
            Some("internal notes merged into the product's notes".to_string())
        }
        (MergeOutcome::UpToDate, _) => None,
        (MergeOutcome::Conflict(paths), PushOutcome::TaskBranchPushed) => Some(format!(
            "internal notes not merged: conflict in {}; {} is pushed to resolve by hand",
            paths.join(", "),
            Internal::task_branch(task)
        )),
        (MergeOutcome::Conflict(paths), _) => Some(format!(
            "internal notes not merged: conflict in {}; the next pull request retries",
            paths.join(", ")
        )),
        (MergeOutcome::Stopped(reason), PushOutcome::TaskBranchPushed) => Some(format!(
            "internal notes not merged: {reason}; {} is pushed to resolve by hand",
            Internal::task_branch(task)
        )),
        (MergeOutcome::Stopped(reason), _) => Some(format!(
            "internal notes not merged: {reason}; the next pull request retries"
        )),
    }
}

/// Rewrites `BREAKDOWN.md` in the task folder. Only a copy of the record, so a failure is only logged.
fn write_breakdown(workdir: &str, goal: &str, v: &BreakdownVersion) {
    let path = Path::new(workdir).join(BREAKDOWN_FILE);
    if let Err(e) = std::fs::write(&path, render(goal, v)) {
        warn!(path = %path.display(), error = %e, "cannot write the breakdown");
    }
}

/// The refusal a restart gets while the task's orchestrator has a session.
const STILL_RUNNING: &str = "the orchestrator is still running";

/// The restarted orchestrator's prompt: the goal, then everything the task recorded, since the session it takes
/// over from may have left nothing usable behind.
fn takeover_prompt(p: &Task, events: &[String]) -> String {
    let mut text = format!(
        "{}\n\nYou are taking over a running operation: this task's previous orchestrator session ended. Its \
         specialists keep running and report to you through `wait_for_fleet`. Below is what the task has recorded; \
         check your worktree and `fleet_status` before you act.\n\n## Breakdown versions\n",
        p.goal.trim()
    );
    if p.breakdowns.is_empty() {
        text.push_str("\nNo breakdown has been proposed yet.\n");
    }
    for v in &p.breakdowns {
        text.push('\n');
        text.push_str(render(&p.goal, v).trim_end());
        text.push('\n');
    }
    // Proposing again would be refused while it waits, and the session the decision was for is gone.
    if let Some(v) = p
        .breakdowns
        .last()
        .filter(|v| v.approval == Approval::Proposed)
    {
        text.push_str(&format!(
            "\nBreakdown v{} is waiting for the person's decision. Do not propose again: call wait_for_fleet; the \
             decision arrives there.\n",
            v.version
        ));
    }
    text.push_str("\n## Step reports\n\n");
    // A step's report as the newest version carrying one has it.
    let mut reports: Vec<(u32, &str, &str)> = vec![];
    for v in p.breakdowns.iter().rev() {
        for s in &v.steps {
            if let Some(r) = s.report.as_deref() {
                if !reports.iter().any(|(n, _, _)| *n == s.n) {
                    reports.push((s.n, &s.title, r));
                }
            }
        }
    }
    reports.sort_by_key(|(n, _, _)| *n);
    if reports.is_empty() {
        text.push_str("None yet.\n");
    }
    for (n, title, report) in reports {
        text.push_str(&format!(
            "### Step {n}: {title}\n\n{}\n\n",
            report.trim_end()
        ));
    }
    text.push_str("\n## Fleet events not yet delivered\n\n");
    if events.is_empty() {
        text.push_str("None.\n");
    } else {
        for e in events {
            text.push_str(&format!("- {e}\n"));
        }
        text.push_str("\nYour next `wait_for_fleet` returns these too.\n");
    }
    text
}

impl AgentService {
    /// Stop on the task card: stops every agent of the task still running, the fleet first and its
    /// orchestrator last. Returns how many it stopped; agents that had already ended are not counted.
    #[tracing::instrument(skip_all, fields(task = %task.0))]
    pub async fn stop_task(&self, task: &AgentId) -> Result<u32, ServiceError> {
        // Held throughout, so no specialist starts between the listing and the orchestrator's stop.
        let _bd = self.breakdown_lock.lock().await;
        let p = self.store.task(task).await?.ok_or(ServiceError::NotFound)?;
        let lead = p.lead.clone().unwrap_or_else(|| p.id.clone());
        let mut live: Vec<Agent> = self
            .store
            .task_agents(&p.id)
            .await?
            .into_iter()
            .filter(|a| a.status != Status::Stopped)
            .collect();
        // The orchestrator last: until it stops it is what the card follows, and the record stays Running.
        live.sort_by_key(|a| a.id == lead);
        let mut stopped = 0;
        let mut failed = None;
        for a in &live {
            match self.stop(&a.id).await {
                Ok(()) => stopped += 1,
                Err(e) => {
                    warn!(agent_id = %a.id.0, error = %e, "cannot stop the task's agent");
                    failed.get_or_insert(e);
                }
            }
        }
        info!(stopped, "task stopped");
        match failed {
            Some(e) => Err(e),
            None => Ok(stopped),
        }
    }

    /// Restart orchestrator on the task card, once its orchestrator has stopped (crashed, exited or stopped
    /// alone): a fresh session in the same `orchestrator/` folder and worktree, with the same type, skills and
    /// `CLAUDE.md`, and a prompt carrying the goal and everything the task recorded. It is a new agent on the same
    /// task, which the store makes the lead, so the fleet's events reach it.
    #[tracing::instrument(skip_all, fields(task = %task.0, agent_id = tracing::field::Empty))]
    pub async fn restart_orchestrator(&self, task: &AgentId) -> Result<Agent, ServiceError> {
        let p = self.store.task(task).await?.ok_or(ServiceError::NotFound)?;
        if p.kind != AgentKind::Orchestrator {
            return Err(refuse("a solo task has no orchestrator to restart"));
        }
        let lead = self.find(p.lead.as_ref().unwrap_or(&p.id)).await?;
        // Before the lock: the logins are asked through the CLIs, and a task's persona never changes.
        let persona = self.persona_of(&lead).await?;
        let fleet = fleet_config(&self.cfg.agent_config_dir).map_err(ServiceError::AgentConfig)?;
        let defs = self.defs();
        let def = defs
            .orchestrator(p.role.as_deref().unwrap_or_default())
            .map_err(|e| refuse(e.to_string()))?;
        let specialists = p
            .specialists
            .iter()
            .map(|n| Ok((n.clone(), defs.specialist(n)?.summary)))
            .collect::<Result<Vec<_>, FleetError>>()
            .map_err(|e| refuse(e.to_string()))?;
        let plan = OrchestratorPlan { def, specialists };

        // Held through the start, so two restarts never both find the orchestrator stopped, and the prompt is the
        // breakdown as it is when the new session becomes the lead.
        let _bd = self.breakdown_lock.lock().await;
        let p = self.store.task(task).await?.ok_or(ServiceError::NotFound)?;
        let lead = self.find(p.lead.as_ref().unwrap_or(&p.id)).await?;
        // A Stopped lead whose session still runs would run in the folder the new session would share.
        if lead.status != Status::Stopped || self.sessions.alive(&lead.session) {
            return Err(refuse(STILL_RUNNING));
        }
        let cwd = Path::new(&p.workdir).join(ORCHESTRATOR_DIR);
        if let Some(w) = lead
            .workspaces
            .iter()
            .find(|w| !Path::new(&w.worktree).is_dir())
        {
            return Err(refuse(format!(
                "the orchestrator's worktree {} is gone",
                w.worktree
            )));
        }
        // Like a repo worktree, the folder may have been deleted by hand; unlike one, its notes are on a branch
        // that outlives it, so it is checked out again rather than refused.
        let internal_dir = Path::new(&p.workdir).join(INTERNAL_DIR);
        let warning = self
            .attach_internal(&p.product, &p.name, &internal_dir)
            .await
            .map_err(|e| refuse(format!("cannot check out the task's internal notes: {e}")))?;
        if let Some(warning) = warning {
            self.log_task(&p.id, warning).await;
        }
        let events = self.store.peek_inbox(&p.id).await?;
        let lines = self.fleet_event_lines(&p.id, &events).await?;
        let prompt = takeover_prompt(&p, &lines);

        let id = uuid::Uuid::new_v4().simple().to_string();
        tracing::Span::current().record("agent_id", id.as_str());
        let agent_id = AgentId(id.clone());
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let _guard = self.spawning_guard(&id, &cwd);
        let settings_dir = self.agent_dir(&id);
        let made = self
            .restart_files(
                &plan,
                &agent_id,
                &token,
                &cwd,
                &internal_dir,
                &p.name,
                &lead.workspaces,
                &fleet,
                &prompt,
            )
            .await;
        let (settings, mcp) = match made {
            Ok(x) => x,
            Err(e) => {
                // Only the settings are this restart's own; the folder and its files belong to the task.
                let _ = std::fs::remove_dir_all(&settings_dir);
                return Err(e);
            }
        };

        let session = format!("{SESSION_PREFIX}{}", &id[..8]);
        let model = match self
            .resolve_model(&lead.product, &plan.def.name, plan.def.model.as_deref())
            .await
        {
            Ok(m) => m,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&settings_dir);
                return Err(e.into());
            }
        };
        let agent = Agent {
            id: agent_id.clone(),
            goal: lead.goal.clone(),
            workdir: cwd.to_string_lossy().into_owned(),
            workspaces: lead.workspaces.clone(),
            session: session.clone(),
            status: Status::Queued,
            activity: "starting".into(),
            status_since: self.opts.clock.now(),
            has_changes: lead.has_changes,
            paused: false,
            owner: self.opts.owner.clone(),
            blocked_reason: None,
            repo_request: None,
            stop_cause: None,
            disk_bytes: None,
            persona: lead.persona.clone(),
            product: lead.product.clone(),
            kind: AgentKind::Orchestrator,
            task: Some(p.id.clone()),
            role: Some(plan.def.name.clone()),
            step: None,
            model,
            attach: None,
        };
        if let Err(e) = self.store.insert_agent_under_persona(&agent, &token).await {
            let _ = std::fs::remove_dir_all(&settings_dir);
            return Err(agent_insert_error(e));
        }
        persona.seed_first_run().await;
        let origins = origin_urls(&agent.workspaces).await;
        let vars = agent_vars(&persona, AgentKind::Orchestrator, &origins);
        if let Err(e) = self
            .launch(
                &agent_id,
                &session,
                &cwd,
                &settings,
                &mcp,
                &token,
                &vars,
                agent.model.as_deref(),
                Start::New,
            )
            .await
        {
            // The card stays, ended, so the task can be restarted again; the store logs why.
            let now = self.opts.clock.now();
            let why = format!("restart failed: {}", one_line(&e.to_string()));
            if let Err(e) = self
                .store
                .update_agent(&agent_id, |x| {
                    x.status = Status::Stopped;
                    x.stop_cause = Some(StopCause::Crashed);
                    x.activity = why;
                    x.status_since = now;
                })
                .await
            {
                warn!(agent_id = %id, error = %e, "cannot end the failed restart's card");
            }
            return Err(e.into());
        }
        info!(agent_id = %id, "orchestrator restarted");
        Ok(agent)
    }

    /// Rewrites what the restarted orchestrator starts from: its settings and MCP config (for its new id and token),
    /// its skills afresh, `CLAUDE.md` and `PROMPT.md`. Returns the settings and MCP config paths.
    #[allow(clippy::too_many_arguments)]
    async fn restart_files(
        &self,
        plan: &OrchestratorPlan,
        id: &AgentId,
        token: &str,
        cwd: &Path,
        internal: &Path,
        task_name: &str,
        workspaces: &[Workspace],
        fleet: &AgentConfig,
        prompt: &str,
    ) -> Result<(std::path::PathBuf, std::path::PathBuf), ServiceError> {
        let settings_dir = self.agent_dir(&id.0);
        let settings = write_settings(
            &settings_dir,
            &self.opts.base_url,
            id,
            &task_repo_rules(workspaces),
            &fleet.settings,
            &plan.settings(internal),
        )
        .and_then(|s| {
            Ok((
                s,
                write_mcp_config(&settings_dir, &self.opts.base_url, id, token)?,
            ))
        })
        .map_err(|e| refuse(format!("cannot write the agent's settings: {e}")))?;
        self.write_orchestrator_files(
            plan,
            cwd,
            &fleet.instructions,
            workspaces,
            internal,
            task_name,
        )
        .map_err(|e| refuse(format!("cannot write the orchestrator's files: {e}")))?;
        // `write_prompt` never overwrites, so the old session's prompt goes first.
        match std::fs::remove_file(cwd.join(PROMPT_FILE)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        write_prompt(cwd, prompt)?;
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::merge_line;
    use crate::internal::{MergeOutcome, MergeReport, PushOutcome, PushReport};

    fn line(merge: MergeOutcome, outcome: PushOutcome, parked: Option<&str>) -> Option<String> {
        let report = MergeReport {
            merge,
            push: PushReport {
                outcome,
                parked: parked.map(String::from),
            },
        };
        merge_line("t", &report)
    }

    #[test]
    fn merged_and_pushed() {
        assert_eq!(
            line(MergeOutcome::Merged, PushOutcome::Pushed, None).as_deref(),
            Some("internal notes merged and pushed")
        );
    }

    #[test]
    fn merged_local_only() {
        assert_eq!(
            line(MergeOutcome::Merged, PushOutcome::LocalOnly, None).as_deref(),
            Some("internal notes merged into the product's notes")
        );
    }

    #[test]
    fn merged_push_failed() {
        assert_eq!(
            line(MergeOutcome::Merged, PushOutcome::Failed("offline".into()), None).as_deref(),
            Some("internal notes merged, not pushed: offline; the next merge or a daemon restart retries")
        );
    }

    #[test]
    fn conflict_with_the_task_branch_pushed() {
        assert_eq!(
            line(
                MergeOutcome::Conflict(vec!["a.md".into(), "b.md".into()]),
                PushOutcome::TaskBranchPushed,
                None
            )
            .as_deref(),
            Some("internal notes not merged: conflict in a.md, b.md; task/t is pushed to resolve by hand")
        );
    }

    #[test]
    fn stopped_merge() {
        assert_eq!(
            line(
                MergeOutcome::Stopped("refusing".into()),
                PushOutcome::LocalOnly,
                None
            )
            .as_deref(),
            Some("internal notes not merged: refusing; the next pull request retries")
        );
    }

    #[test]
    fn up_to_date_local_only_says_nothing() {
        assert_eq!(
            line(MergeOutcome::UpToDate, PushOutcome::LocalOnly, None),
            None
        );
    }

    #[test]
    fn a_park_ends_the_line() {
        assert_eq!(
            line(MergeOutcome::Merged, PushOutcome::Pushed, Some("fleet/unpushed-1")).as_deref(),
            Some(
                "internal notes merged and pushed; earlier unpushed notes conflicted with the remote and are on \
                 fleet/unpushed-1 to merge by hand"
            )
        );
    }

    #[test]
    fn a_park_is_said_even_with_nothing_else_to_say() {
        assert_eq!(
            line(MergeOutcome::UpToDate, PushOutcome::Pushed, Some("fleet/unpushed-1")).as_deref(),
            Some(
                "internal notes; earlier unpushed notes conflicted with the remote and are on fleet/unpushed-1 to \
                 merge by hand"
            )
        );
    }
}
