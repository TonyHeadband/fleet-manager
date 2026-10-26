//! The orchestrated parts of `AgentService`: what an orchestrator project adds to a solo spawn, and its breakdown.
use super::breakdown::{next_version, render, state_word, BreakdownAnswer, BREAKDOWN_FILE};
use super::defs::{Defs, FleetError, OrchestratorDef, SpecialistDef};
use super::gate;
use super::skills;
use super::workflow::{self, WorkflowError};
use crate::agents::{
    agent_insert_error, fleet_config, instructions_section, one_line, project_repo_rules,
    repo_section, write_prompt, AgentService, ServiceError, SpawnError, Undo, PROMPT_FILE,
    SESSION_PREFIX,
};
use crate::git;
use crate::hooks_cfg::{write_mcp_config, write_settings, RoleSettings};
use crate::personas::PersonaEnv;
use crate::repos::{Resolved, Start};
use crate::store::StoreError;
use fleet_api::{
    project_id, Agent, AgentConfig, AgentId, AgentKind, Approval, BlockedReason, BreakdownVersion,
    DecideBreakdown, DefKind, FleetEvent, FleetEventKind, FleetView, GateRun, GateStep, Project,
    SpawnRequest, Status, Step, StepDraft, StepState, StopCause, Workspace,
};
use std::collections::HashSet;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::sync::{broadcast, oneshot};
use tracing::{info, warn};

/// The orchestrator's own folder inside the project folder: its cwd, holding its worktree, `CLAUDE.md`, prompt and
/// skills. The project folder itself stays empty of anything Claude Code loads, since every fleet agent's folder
/// sits under it too (docs/superpowers/findings/fleet-spike.md).
pub const ORCHESTRATOR_DIR: &str = "orchestrator";

/// The folder of the fleet agents' folders, inside the project folder. Like the project folder it holds nothing
/// Claude Code loads, since it is the parent of every one of them (docs/superpowers/findings/fleet-spike.md).
pub const FLEET_DIR: &str = "fleet";

/// A step's branch, beside the project branch rather than under it: git cannot keep `fleet/x` and `fleet/x/1-api`
/// both, since one would be a file where the other needs a folder. A slug never holds `--`, so no project's own
/// branch can be named like this.
pub fn step_branch(project_branch: &str, n: u32, slug: &str) -> String {
    format!("{project_branch}--{n}-{slug}")
}

/// The specialist's `CLAUDE.md`: the fleet-wide instructions, its role prompt and its one repo.
fn specialist_claude_md(
    def: &SpecialistDef,
    instructions: &str,
    workspaces: &[Workspace],
) -> String {
    let mut text = instructions_section(instructions);
    text.push_str(def.prompt.trim_end());
    text.push_str("\n\n");
    text.push_str(&repo_section(workspaces, false));
    text
}

/// The step's assignment, with the goal it serves.
fn specialist_prompt(step: &Step, goal: &str) -> String {
    format!(
        "{}\n\nThe project's goal: {}",
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
fn update_step(p: &mut Project, n: u32, f: impl FnOnce(&mut Step)) {
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

/// What a specialist spawn has made so far, for undoing it.
struct Made {
    undo: Undo,
    card: Option<AgentId>,
    step_running: bool,
}

/// What a specialist spawn works from, read under the breakdown lock.
struct SpecialistPlan {
    project: Project,
    step: Step,
    def: SpecialistDef,
    lead_workspace: Workspace,
    persona: PersonaEnv,
    fleet: AgentConfig,
}

/// What an orchestrated spawn adds to a solo one, worked out before anything is made.
pub(crate) struct OrchestratorPlan {
    pub def: OrchestratorDef,
    /// Each roster name with its summary, as the orchestrator is shown them.
    pub roster: Vec<(String, String)>,
}

impl OrchestratorPlan {
    pub fn settings(&self) -> RoleSettings {
        RoleSettings::orchestrator(&self.def.tools)
    }

    /// The orchestrator's `CLAUDE.md`: the fleet-wide instructions, its type's prompt, its roster and the repo.
    fn claude_md(&self, instructions: &str, workspaces: &[Workspace]) -> String {
        let mut text = instructions_section(instructions);
        text.push_str(self.def.prompt.trim_end());
        text.push_str("\n\n## Your roster\n\n");
        for (name, summary) in &self.roster {
            text.push_str(&format!("- {name}: {summary}\n"));
        }
        text.push('\n');
        text.push_str(&repo_section(workspaces, false));
        text
    }
}

/// The persona's env as the agent's process gets it. A fleet agent's gets one more git config entry, appended after
/// the persona's own, that points `origin`'s push URL at a transport that does not exist: a push the deny rules miss
/// (`bash -c 'git push'`, a script) then fails too. Not a boundary (`git push <url>` or unsetting the env gets past
/// it, and `remote.origin.pushurl` is multi-valued: a repo that sets its own origin pushurl still pushes there
/// before this entry fails); `open_pr` pushes with the persona's env alone.
pub fn agent_vars(persona: &PersonaEnv, kind: AgentKind) -> Vec<(String, String)> {
    let mut vars = persona.vars.clone();
    if kind == AgentKind::Solo {
        return vars;
    }
    let count = vars.iter_mut().find(|(k, _)| k == "GIT_CONFIG_COUNT");
    let n = match count {
        Some((_, v)) => {
            let n: usize = v.parse().unwrap_or(0);
            *v = (n + 1).to_string();
            n
        }
        None => {
            vars.push(("GIT_CONFIG_COUNT".into(), "1".into()));
            0
        }
    };
    vars.push((
        format!("GIT_CONFIG_KEY_{n}"),
        "remote.origin.pushurl".into(),
    ));
    vars.push((
        format!("GIT_CONFIG_VALUE_{n}"),
        "fleet-push-blocked:///".into(),
    ));
    vars
}

impl AgentService {
    fn defs(&self) -> Defs {
        Defs::new(self.cfg.fleet_dir.clone())
    }

    /// See `ServiceOptions::fleet_gate`.
    async fn fleet_gate(&self) {
        if let Some(gate) = &self.opts.fleet_gate {
            gate.reached.notify_one();
            gate.open.notified().await;
        }
    }

    /// The orchestrator type a spawn asks for, checked before any repo is resolved; None for a solo project.
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
        let mut inputs: Vec<&str> = req.repos.iter().map(|r| r.trim()).collect();
        inputs.sort();
        inputs.dedup();
        if inputs.len() != 1 {
            return Err(SpawnError::OneRepo);
        }
        Ok(Some(def))
    }

    /// The rest of the checks, once the one repo is resolved: every roster name (the one sent, else the repo's) a
    /// specialist that loads. The repo's workflow is checked after, since that fetches.
    pub(crate) fn orchestrator_plan(
        &self,
        def: OrchestratorDef,
        req: &SpawnRequest,
        rows: &[Resolved],
    ) -> Result<OrchestratorPlan, SpawnError> {
        let [Resolved { row, .. }] = rows else {
            return Err(SpawnError::OneRepo);
        };
        let names = if req.roster.is_empty() {
            &row.roster
        } else {
            &req.roster
        };
        if names.is_empty() {
            return Err(SpawnError::NoRoster);
        }
        let defs = self.defs();
        let roster = names
            .iter()
            .map(|n| Ok((n.clone(), defs.specialist(n)?.summary)))
            .collect::<Result<Vec<_>, FleetError>>()?;
        Ok(OrchestratorPlan { def, roster })
    }

    /// Puts the orchestrator's skills and `CLAUDE.md` into its folder. The skills folder is made even when empty,
    /// so it is plain which skills the agent has.
    pub(crate) fn write_orchestrator_files(
        &self,
        plan: &OrchestratorPlan,
        cwd: &Path,
        instructions: &str,
        workspaces: &[Workspace],
    ) -> Result<(), SpawnError> {
        std::fs::create_dir_all(cwd.join(".claude/skills"))?;
        skills::install(&self.defs(), &plan.def.skills, cwd)?;
        std::fs::write(
            cwd.join("CLAUDE.md"),
            plan.claude_md(instructions, workspaces),
        )?;
        Ok(())
    }

    /// Makes the record the store created for the orchestrator an orchestrated project's: the project folder is the
    /// orchestrator folder's parent, not the agent's cwd.
    pub(crate) async fn record_orchestrator(
        &self,
        id: &AgentId,
        plan: &OrchestratorPlan,
        project_dir: &Path,
    ) -> Result<(), StoreError> {
        let name = project_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let workdir = project_dir.to_string_lossy().into_owned();
        let role = plan.def.name.clone();
        let roster = plan.roster.iter().map(|(n, _)| n.clone()).collect();
        self.store
            .update_project(id, move |p| {
                p.kind = AgentKind::Orchestrator;
                p.role = Some(role);
                p.roster = roster;
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
            let project = project_id(&a).clone();
            let p = self
                .store
                .project(&project)
                .await?
                .ok_or(ServiceError::NotFound)?;
            if p.breakdowns
                .last()
                .is_some_and(|v| v.approval == Approval::Proposed)
            {
                return Err(ServiceError::BreakdownPending);
            }
            self.not_gating(&project)?;
            let now = self.opts.clock.now();
            // Checked before any write, so a refused proposal changes nothing.
            let n = next_version(&p.breakdowns, &drafts, &p.roster, now)?.version;
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
                .update_project(&project, |p| {
                    // Made again from the record as it is now: a specialist that ended since the read above had its
                    // step failed by the store, and the new version must carry that, not the step as running.
                    let v = next_version(&p.breakdowns, &drafts, &p.roster, now);
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
            write_breakdown(&p.workdir, &p.task, &v);
            info!(agent_id = %orchestrator.0, version = n, "breakdown proposed");
            self.log_project(&project, format!("breakdown v{n} proposed"))
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

    /// The owner's decision on the project's proposed breakdown, handed to the held call or, when none is left,
    /// queued as a fleet event for the lead's `wait_for_fleet`.
    pub async fn decide_breakdown(
        &self,
        project: &AgentId,
        d: DecideBreakdown,
    ) -> Result<(), ServiceError> {
        let _bd = self.breakdown_lock.lock().await;
        let p = self
            .store
            .project(project)
            .await?
            .ok_or(ServiceError::NotFound)?;
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
            .update_project(project, |p| {
                if let Some(v) = p.breakdowns.last_mut() {
                    v.approval = approval;
                    v.comment = comment;
                    v.decided_at = Some(now);
                    decided = Some(v.clone());
                }
            })
            .await?;
        if let Some(v) = &decided {
            write_breakdown(&p.workdir, &p.task, v);
        }
        info!(agent_id = %project.0, version = n, approved = d.approve, "breakdown decided");
        self.log_project(project, format!("breakdown v{n} {said}"))
            .await;
        let lead = p.lead.clone().unwrap_or_else(|| p.id.clone());
        // An archived project has no agent left to tell.
        if self.find(&lead).await.is_err() {
            return Ok(());
        }
        // Only a card still on the breakdown is settled: a prompt that came up since (a permission, a question)
        // owns it until it is answered.
        self.store
            .update_agent(&lead, |x| {
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
        if self.answer_breakdown(&lead, answer) {
            return Ok(());
        }
        // No held call: the lead hears it from `wait_for_fleet`, now if it is held there, else on its next call or
        // in a restart's prompt. Never typed: a session parked in `wait_for_fleet` reads no input.
        let detail = if d.approve {
            format!("breakdown v{n} approved")
        } else {
            format!("breakdown v{n}: {said}")
        };
        let ev = FleetEvent {
            at: now,
            step: 0,
            agent: project.clone(),
            kind: FleetEventKind::BreakdownDecided,
            detail,
        };
        self.store
            .update_project(project, |p| p.inbox.push(ev))
            .await?;
        // The board derived Needs you from the pending version; this also wakes a held `wait_for_fleet` at once. The
        // decision is already stored and queued, so a failure here only leaves the card stale until its next change.
        if let Err(e) = self.store.announce(&lead).await {
            warn!(agent_id = %lead.0, error = %e, "cannot show the decided breakdown on the card");
        }
        Ok(())
    }

    /// Every breakdown version and gate run of a project, for its side panel.
    pub async fn fleet_view(&self, project: &AgentId) -> Result<FleetView, ServiceError> {
        let p = self
            .store
            .project(project)
            .await?
            .ok_or(ServiceError::NotFound)?;
        Ok(FleetView {
            project: p.id,
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
    /// folder `<project>/fleet/<n>-<slug>/`, with a worktree on a step branch cut from the project branch's tip as
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
        // Before the lock: the logins are asked through the CLIs, and a project's persona never changes.
        let persona = self.persona_of(&self.find(orchestrator).await?).await?;
        let fleet = fleet_config(&self.cfg.agent_config_dir).map_err(ServiceError::AgentConfig)?;
        // Held through the whole spawn, so two calls for one step never both find it Pending, and no proposal or
        // decision changes the breakdown in between.
        let _bd = self.breakdown_lock.lock().await;
        let lead = self.find(orchestrator).await?;
        let pid = project_id(&lead).clone();
        let project = self
            .store
            .project(&pid)
            .await?
            .ok_or(ServiceError::NotFound)?;
        if lead.kind != AgentKind::Orchestrator
            || project.lead.as_ref().unwrap_or(&project.id) != &lead.id
        {
            return Err(refuse(
                "only the project's orchestrator starts its specialists",
            ));
        }
        if lead.status == Status::Stopped {
            return Err(ServiceError::NotRunning);
        }
        self.not_gating(&pid)?;
        let step = ready_step(&project.breakdowns, n)?.clone();
        let def = self
            .defs()
            .specialist(&step.specialist)
            .map_err(|e| refuse(e.to_string()))?;
        let [lead_workspace] = &lead.workspaces[..] else {
            return Err(ServiceError::OneRepo);
        };
        let plan = SpecialistPlan {
            project,
            step,
            def,
            lead_workspace: lead_workspace.clone(),
            persona,
            fleet,
        };

        let id = uuid::Uuid::new_v4().simple().to_string();
        let slug = git::slugify(&plan.step.title);
        let folder = Path::new(&plan.project.workdir)
            .join(FLEET_DIR)
            .join(format!("{n}-{slug}"));
        // Registered before the folder exists, so neither reconcile nor the orphan scan acts on a half-made agent.
        let _guard = self.spawning_guard(&id, &folder);
        let mut made = Made {
            undo: Undo {
                made: vec![],
                workdir: folder.clone(),
                settings_dir: self.agent_dir(&id),
            },
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
                self.unmake(&plan.project, n, &made, &e).await;
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
        let row = self
            .store
            .repo(&plan.persona.id, &lw.repo)
            .await?
            .ok_or_else(|| refuse(format!("repo {} is no longer remembered", lw.repo)))?;
        let branch = step_branch(&lw.branch, n, slug);
        // The project branch itself, with every step merged so far: the orchestrator's checkout may be detached or
        // on another branch, and its HEAD would then be the wrong start.
        let start = Start::Branch(lw.branch.clone());
        let (wt, warning) = self
            .repos
            .attach(
                &row,
                &start,
                &folder.join(&row.name),
                &branch,
                &plan.persona,
            )
            .await?;
        if let Some(w) = warning {
            warn!(agent_id = %id, warning = %w, "step worktree made with a warning");
        }
        let workspaces = vec![Workspace {
            repo: row.name.clone(),
            branch: wt.branch.clone(),
            worktree: wt.path.to_string_lossy().into_owned(),
            base: wt.base.clone(),
            from_branch: wt.from_branch.clone(),
            has_changes: false,
            owned: false,
        }];
        made.undo.made.push((row.base.clone(), wt));

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
            &project_repo_rules(&workspaces),
            &plan.fleet.settings,
            &RoleSettings::specialist(&plan.def.tools),
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
            specialist_claude_md(&plan.def, &plan.fleet.instructions, &workspaces),
        )?;
        write_prompt(folder, &specialist_prompt(&plan.step, &plan.project.task))?;

        let session = format!("{SESSION_PREFIX}{}", &id[..8]);
        // Stored before tmux starts: SessionStart can arrive within milliseconds of the process starting.
        let agent = Agent {
            id: agent_id.clone(),
            task: plan.step.title.chars().take(120).collect(),
            workdir: folder.to_string_lossy().into_owned(),
            workspaces,
            tmux_session: session.clone(),
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
            kind: AgentKind::Specialist,
            project: Some(plan.project.id.clone()),
            role: Some(plan.def.name.clone()),
            step: Some(n),
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
            .update_project(&plan.project.id, |p| {
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
            write_breakdown(&plan.project.workdir, &plan.project.task, v);
        }

        plan.persona.seed_first_run();
        let vars = agent_vars(&plan.persona, AgentKind::Specialist);
        self.launch(&session, folder, &settings, &mcp, &token, &vars)
            .await?;
        Ok(agent)
    }

    /// Undoes a failed specialist spawn: the step goes back to Pending, the card goes, then the files and worktree.
    async fn unmake(&self, project: &Project, n: u32, made: &Made, error: &ServiceError) {
        if made.step_running {
            let mut latest = None;
            let r = self
                .store
                .update_project(&project.id, |p| {
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
                        write_breakdown(&project.workdir, &project.task, v);
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
        self.undo(&made.undo, None).await;
        warn!(step = n, error = %error, "specialist spawn failed");
        self.log_project(
            &project.id,
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
        let pid = project_id(&a).clone();
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
            .update_project(&pid, |p| {
                update_step(p, n, |s| {
                    s.state = StepState::Done;
                    s.report = Some(report);
                });
                record = Some((
                    p.workdir.clone(),
                    p.task.clone(),
                    p.breakdowns.last().cloned(),
                ));
            })
            .await?;
        if let Some((workdir, task, Some(v))) = &record {
            write_breakdown(workdir, task, v);
        }
        info!(agent_id = %specialist.0, step = n, "step finished");
        self.log_project(&pid, format!("step {n} finished")).await;
        // After the writes, so the session's end finds the agent already Stopped and names no other cause.
        if let Err(e) = self.kill_or_gone(&a).await {
            warn!(agent_id = %specialist.0, error = %e, "cannot end the finished specialist's session");
        }
        Ok(())
    }
}

impl AgentService {
    /// The orchestrator's project and its record, for a caller that must be that project's current orchestrator.
    async fn followed_project(
        &self,
        orchestrator: &AgentId,
    ) -> Result<(Agent, Project), ServiceError> {
        let a = self.find(orchestrator).await?;
        if a.kind != AgentKind::Orchestrator {
            return Err(ServiceError::NotAnOrchestrator);
        }
        let p = self
            .store
            .project(project_id(&a))
            .await?
            .ok_or(ServiceError::NotFound)?;
        if p.lead.as_ref().unwrap_or(&p.id) != &a.id {
            return Err(refuse("only the project's orchestrator follows its fleet"));
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
    ) -> Result<(Project, Vec<FleetEvent>), ServiceError> {
        let (_, p) = self.followed_project(orchestrator).await?;
        // Subscribed before the first look, so an event landing between the look and the wait wakes it.
        let mut live = self.store.subscribe();
        let deadline = tokio::time::Instant::now() + self.opts.repo_request_hold;
        loop {
            let events = self.store.peek_inbox(&p.id).await?;
            if !events.is_empty() {
                return Ok((p, events));
            }
            // After the peek, so events queued before a stop are still delivered, and the stop is seen on any wake.
            self.followed_project(orchestrator).await?;
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
        let (_, p) = self.followed_project(orchestrator).await?;
        let Some(v) = p.breakdowns.last() else {
            return Err(refuse("no breakdown yet"));
        };
        let agents = self.store.project_agents(&p.id).await?;
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
        project: &AgentId,
        events: &[FleetEvent],
    ) -> Result<Vec<String>, ServiceError> {
        let p = self
            .store
            .project(project)
            .await?
            .ok_or(ServiceError::NotFound)?;
        let agents = self.store.project_agents(&p.id).await?;
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

/// Takes the project off `gating` when its `open_pr` ends, however it ends.
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

/// What the PR merges into: the branch the project started from, else the remote's default branch.
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
    /// Refuses a change to the breakdown while the project's `open_pr` runs: the gate tests what it checked.
    fn not_gating(&self, project: &AgentId) -> Result<(), ServiceError> {
        if self
            .gating
            .lock()
            .expect("gating poisoned")
            .contains(&project.0)
        {
            return Err(refuse(
                "the tests before the PR are running; wait for open_pr",
            ));
        }
        Ok(())
    }

    /// The orchestrator's `open_pr`, once no step of the approved plan is pending or running: runs the steps of the
    /// repo's workflow (read from `origin/<base>`) that the change needs, in its worktree, on the project branch, with
    /// the persona's env, and records the run. Only if every step passes is the project branch pushed and the PR
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
        // Before the lock: the logins are asked through the CLIs, and a project's persona never changes.
        let persona = self.persona_of(&a).await?;
        let (lead, p, failed, _gating) = {
            let _bd = self.breakdown_lock.lock().await;
            let lead = self.find(orchestrator).await?;
            let p = self
                .store
                .project(project_id(&lead))
                .await?
                .ok_or(ServiceError::NotFound)?;
            if p.lead.as_ref().unwrap_or(&p.id) != &lead.id {
                return Err(refuse("only the project's orchestrator opens its PR"));
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
            // The worktree's refs live in the base clone, which other projects fetch into too.
            let row = self.store.repo(&lead.persona, &w.repo).await?;
            let _base = match &row {
                Some(r) => Some(self.repos.lock_base(&r.base).await),
                None => {
                    warn!(repo = %w.repo, "repo not remembered; fetching the base branch without its lock");
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
            Err(e @ WorkflowError::Fetch(_)) => return Err(refuse(e.to_string())),
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
        // With the agents' push block: a repo script that pushes during the tests fails like an agent would.
        let gate_env = agent_vars(&persona, AgentKind::Orchestrator);
        let started = Instant::now();
        let mut steps = Vec::new();
        for picked in selection.run {
            // One budget for the whole gate; a step that starts with none left still gets a moment to fail cleanly.
            let left = budget
                .saturating_sub(started.elapsed())
                .max(Duration::from_secs(1));
            let out = gate::run(&picked.step.run, wt, &gate_env, left)
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
            .update_project(&p.id, |x| x.gate_runs.push(stored))
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
        self.log_project(&p.id, line).await;
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
                    warn!(error = %e, "cannot set the project branch's upstream");
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
            .update_project(&p.id, |x| {
                if let Some(r) = x.gate_runs.iter_mut().rev().find(|r| r.at == run.at) {
                    r.pr_url = run.pr_url.clone();
                }
            })
            .await?;
        info!(url = %url, "PR opened");
        self.log_project(&p.id, format!("PR opened: {url}")).await;
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

/// Rewrites `BREAKDOWN.md` in the project folder. Only a copy of the record, so a failure is only logged.
fn write_breakdown(workdir: &str, task: &str, v: &BreakdownVersion) {
    let path = Path::new(workdir).join(BREAKDOWN_FILE);
    if let Err(e) = std::fs::write(&path, render(task, v)) {
        warn!(path = %path.display(), error = %e, "cannot write the breakdown");
    }
}

/// The refusal a restart gets while the project's orchestrator has a session.
const STILL_RUNNING: &str = "the orchestrator is still running";

/// The restarted orchestrator's prompt: the goal, then everything the project recorded, since the session it takes
/// over from may have left nothing usable behind.
fn takeover_prompt(p: &Project, events: &[String]) -> String {
    let mut text = format!(
        "{}\n\nYou are taking over a running operation: this project's previous orchestrator session ended. Its \
         specialists keep running and report to you through `wait_for_fleet`. Below is what the project has recorded; \
         check your worktree and `fleet_status` before you act.\n\n## Breakdown versions\n",
        p.task.trim()
    );
    if p.breakdowns.is_empty() {
        text.push_str("\nNo breakdown has been proposed yet.\n");
    }
    for v in &p.breakdowns {
        text.push('\n');
        text.push_str(render(&p.task, v).trim_end());
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
    /// Stop on the project card: stops every agent of the project still running, the fleet first and its
    /// orchestrator last. Returns how many it stopped; agents that had already ended are not counted.
    #[tracing::instrument(skip_all, fields(project = %project.0))]
    pub async fn stop_project(&self, project: &AgentId) -> Result<u32, ServiceError> {
        // Held throughout, so no specialist starts between the listing and the orchestrator's stop.
        let _bd = self.breakdown_lock.lock().await;
        let p = self
            .store
            .project(project)
            .await?
            .ok_or(ServiceError::NotFound)?;
        let lead = p.lead.clone().unwrap_or_else(|| p.id.clone());
        let mut live: Vec<Agent> = self
            .store
            .project_agents(&p.id)
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
                    warn!(agent_id = %a.id.0, error = %e, "cannot stop the project's agent");
                    failed.get_or_insert(e);
                }
            }
        }
        info!(stopped, "project stopped");
        match failed {
            Some(e) => Err(e),
            None => Ok(stopped),
        }
    }

    /// Restart orchestrator on the project card, once its orchestrator has stopped (crashed, exited or stopped
    /// alone): a fresh session in the same `orchestrator/` folder and worktree, with the same type, skills and
    /// `CLAUDE.md`, and a prompt carrying the goal and everything the project recorded. It is a new agent on the same
    /// project, which the store makes the lead, so the fleet's events reach it.
    #[tracing::instrument(skip_all, fields(project = %project.0, agent_id = tracing::field::Empty))]
    pub async fn restart_orchestrator(&self, project: &AgentId) -> Result<Agent, ServiceError> {
        let p = self
            .store
            .project(project)
            .await?
            .ok_or(ServiceError::NotFound)?;
        if p.kind != AgentKind::Orchestrator {
            return Err(refuse("a solo project has no orchestrator to restart"));
        }
        let lead = self.find(p.lead.as_ref().unwrap_or(&p.id)).await?;
        // Before the lock: the logins are asked through the CLIs, and a project's persona never changes.
        let persona = self.persona_of(&lead).await?;
        let fleet = fleet_config(&self.cfg.agent_config_dir).map_err(ServiceError::AgentConfig)?;
        let defs = self.defs();
        let def = defs
            .orchestrator(p.role.as_deref().unwrap_or_default())
            .map_err(|e| refuse(e.to_string()))?;
        let roster = p
            .roster
            .iter()
            .map(|n| Ok((n.clone(), defs.specialist(n)?.summary)))
            .collect::<Result<Vec<_>, FleetError>>()
            .map_err(|e| refuse(e.to_string()))?;
        let plan = OrchestratorPlan { def, roster };

        // Held through the start, so two restarts never both find the orchestrator stopped, and the prompt is the
        // breakdown as it is when the new session becomes the lead.
        let _bd = self.breakdown_lock.lock().await;
        let p = self
            .store
            .project(project)
            .await?
            .ok_or(ServiceError::NotFound)?;
        let lead = self.find(p.lead.as_ref().unwrap_or(&p.id)).await?;
        // A Stopped lead whose kill failed still runs in the folder the new session would share.
        if lead.status != Status::Stopped || self.tmux.has_session(&lead.tmux_session).await? {
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
                &lead.workspaces,
                &fleet,
                &prompt,
            )
            .await;
        let (settings, mcp) = match made {
            Ok(x) => x,
            Err(e) => {
                // Only the settings are this restart's own; the folder and its files belong to the project.
                let _ = std::fs::remove_dir_all(&settings_dir);
                return Err(e);
            }
        };

        let session = format!("{SESSION_PREFIX}{}", &id[..8]);
        let agent = Agent {
            id: agent_id.clone(),
            task: lead.task.clone(),
            workdir: cwd.to_string_lossy().into_owned(),
            workspaces: lead.workspaces.clone(),
            tmux_session: session.clone(),
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
            kind: AgentKind::Orchestrator,
            project: Some(p.id.clone()),
            role: Some(plan.def.name.clone()),
            step: None,
            attach: None,
        };
        if let Err(e) = self.store.insert_agent_under_persona(&agent, &token).await {
            let _ = std::fs::remove_dir_all(&settings_dir);
            return Err(agent_insert_error(e));
        }
        persona.seed_first_run();
        let vars = agent_vars(&persona, AgentKind::Orchestrator);
        if let Err(e) = self
            .launch(&session, &cwd, &settings, &mcp, &token, &vars)
            .await
        {
            // The card stays, ended, so the project can be restarted again; the store logs why.
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
        workspaces: &[Workspace],
        fleet: &AgentConfig,
        prompt: &str,
    ) -> Result<(std::path::PathBuf, std::path::PathBuf), ServiceError> {
        let settings_dir = self.agent_dir(&id.0);
        let settings = write_settings(
            &settings_dir,
            &self.opts.base_url,
            id,
            &project_repo_rules(workspaces),
            &fleet.settings,
            &plan.settings(),
        )
        .and_then(|s| {
            Ok((
                s,
                write_mcp_config(&settings_dir, &self.opts.base_url, id, token)?,
            ))
        })
        .map_err(|e| refuse(format!("cannot write the agent's settings: {e}")))?;
        self.write_orchestrator_files(plan, cwd, &fleet.instructions, workspaces)
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
