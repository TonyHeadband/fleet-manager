//! The orchestrated parts of `AgentService`: what an orchestrator project adds to a solo spawn, and its breakdown.
use super::breakdown::{next_version, render, state_word, BreakdownAnswer, BREAKDOWN_FILE};
use super::defs::{Defs, FleetError, OrchestratorDef, SpecialistDef};
use super::skills;
use crate::agents::{
    fleet_config, instructions_section, one_line, project_repo_rules, repo_section, write_prompt,
    AgentService, ServiceError, SpawnError, Undo, SESSION_PREFIX,
};
use crate::git;
use crate::hooks_cfg::{write_mcp_config, write_settings, RoleSettings};
use crate::personas::PersonaEnv;
use crate::repos::{Resolved, Start};
use crate::store::StoreError;
use fleet_api::{
    project_id, Agent, AgentConfig, AgentId, AgentKind, Approval, BlockedReason, BreakdownVersion,
    DecideBreakdown, DefKind, FleetEvent, FleetEventKind, FleetView, Project, SpawnRequest, Status,
    Step, StepDraft, StepState, StopCause, Workspace,
};
use std::path::Path;
use std::time::Duration;
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

/// The step `n` of the latest version, if a specialist may start on it now. Only the latest version counts: while a
/// newer one waits for a decision, or was sent back, the approved one before it is not the plan any more.
fn ready_step(breakdowns: &[BreakdownVersion], n: u32) -> Result<&Step, ServiceError> {
    let Some(v) = breakdowns.last() else {
        return Err(refuse("no breakdown is approved yet"));
    };
    match v.approval {
        Approval::Approved => {}
        Approval::Proposed => {
            return Err(refuse(format!(
                "breakdown v{} is waiting for approval",
                v.version
            )))
        }
        Approval::ChangesRequested => {
            return Err(refuse(format!(
                "breakdown v{} needs a new proposal",
                v.version
            )))
        }
    }
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
    pub test_command: String,
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

    /// The rest of the checks, once the one repo is resolved: its test command, and every roster name (the one sent,
    /// else the repo's) a specialist that loads.
    pub(crate) fn orchestrator_plan(
        &self,
        def: OrchestratorDef,
        req: &SpawnRequest,
        rows: &[Resolved],
    ) -> Result<OrchestratorPlan, SpawnError> {
        let [Resolved { row, .. }] = rows else {
            return Err(SpawnError::OneRepo);
        };
        let test_command = match row.test_command.as_deref().map(str::trim) {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => return Err(SpawnError::NoTestCommand(row.name.clone())),
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
        Ok(OrchestratorPlan {
            def,
            roster,
            test_command,
        })
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
        let test_command = plan.test_command.clone();
        self.store
            .update_project(id, move |p| {
                p.kind = AgentKind::Orchestrator;
                p.role = Some(role);
                p.roster = roster;
                p.test_command = Some(test_command);
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
        &self,
        orchestrator: &AgentId,
        drafts: Vec<StepDraft>,
    ) -> Result<BreakdownAnswer, ServiceError> {
        let rx = {
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
            let now = self.opts.clock.now();
            let v = next_version(&p.breakdowns, &drafts, &p.roster, now)?;
            let n = v.version;
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
            let stored = v.clone();
            if let Err(e) = self
                .store
                .update_project(&project, |p| p.breakdowns.push(stored))
                .await
            {
                // The card must not wait on a proposal nobody can decide.
                if let Err(e) = self
                    .settle(orchestrator, format!("breakdown v{n} not stored"))
                    .await
                {
                    warn!(agent_id = %orchestrator.0, error = %e, "cannot take the card off the unstored breakdown");
                }
                return Err(e.into());
            }
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
            rx
        };
        match tokio::time::timeout(self.opts.repo_request_hold, rx).await {
            Ok(Ok(answer)) => Ok(answer),
            // Only a stop or a lost session drops the sender unanswered.
            Ok(Err(_)) => Err(ServiceError::NotRunning),
            Err(_) => Err(ServiceError::NoAnswerYet),
        }
    }

    /// The owner's decision on the project's proposed breakdown, handed to the held call or, when none is left,
    /// typed into the orchestrator's session.
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
        // It is typed into a terminal, where a newline or an escape would act.
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
        let Ok(before) = self.find(&lead).await else {
            return Ok(());
        };
        self.settle(&lead, format!("breakdown v{n} {said}")).await?;
        if !self.answer_breakdown(&lead, answer) {
            self.type_line(&before, &format!("fleet-manager: breakdown v{n} {said}"))
                .await;
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
    #[tracing::instrument(skip_all, fields(agent_id = %orchestrator.0, step = n))]
    pub async fn spawn_specialist(
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
        };
        self.store.insert_agent(&agent, &token).await?;
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

    /// The orchestrator's `wait_for_fleet`: the events queued since its last call, or, when there are none, the
    /// first that arrive within the hold. Empty when nothing happened.
    pub async fn wait_for_fleet(
        &self,
        orchestrator: &AgentId,
    ) -> Result<Vec<FleetEvent>, ServiceError> {
        let (_, p) = self.followed_project(orchestrator).await?;
        // Subscribed before the first look, so an event landing between the look and the wait wakes it.
        let mut live = self.store.subscribe();
        let deadline = tokio::time::Instant::now() + self.opts.repo_request_hold;
        loop {
            let events = self.store.take_inbox(&p.id).await?;
            if !events.is_empty() {
                return Ok(events);
            }
            // After the take, so events queued before a stop are still delivered, and the stop is seen on any wake.
            self.followed_project(orchestrator).await?;
            let tick = (tokio::time::Instant::now() + WAIT_TICK).min(deadline);
            match tokio::time::timeout_at(tick, live.recv()).await {
                // A lagged receiver only missed wake-ups, and the inbox is the truth.
                Ok(Ok(_) | Err(broadcast::error::RecvError::Lagged(_))) => {}
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    return Err(ServiceError::NotRunning)
                }
                Err(_) if tokio::time::Instant::now() >= deadline => return Ok(vec![]),
                Err(_) => {}
            }
        }
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
    pub async fn fleet_event_lines(
        &self,
        orchestrator: &AgentId,
        events: &[FleetEvent],
    ) -> Result<Vec<String>, ServiceError> {
        let a = self.find(orchestrator).await?;
        let p = self
            .store
            .project(project_id(&a))
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
                }
            })
            .collect();
        Ok(lines)
    }
}

/// Rewrites `BREAKDOWN.md` in the project folder. Only a copy of the record, so a failure is only logged.
fn write_breakdown(workdir: &str, task: &str, v: &BreakdownVersion) {
    let path = Path::new(workdir).join(BREAKDOWN_FILE);
    if let Err(e) = std::fs::write(&path, render(task, v)) {
        warn!(path = %path.display(), error = %e, "cannot write the breakdown");
    }
}
