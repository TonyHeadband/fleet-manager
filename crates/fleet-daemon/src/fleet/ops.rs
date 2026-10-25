//! The orchestrated parts of `AgentService`: what an orchestrator project adds to a solo spawn, and its breakdown.
use super::breakdown::{next_version, render, BreakdownAnswer, BREAKDOWN_FILE};
use super::defs::{Defs, FleetError, OrchestratorDef};
use super::skills;
use crate::agents::{
    instructions_section, one_line, repo_section, AgentService, ServiceError, SpawnError,
};
use crate::hooks_cfg::RoleSettings;
use crate::personas::PersonaEnv;
use crate::repos::Resolved;
use crate::store::StoreError;
use fleet_api::{
    project_id, AgentId, AgentKind, Approval, BlockedReason, BreakdownVersion, DecideBreakdown,
    DefKind, FleetView, SpawnRequest, Status, StepDraft, Workspace,
};
use std::path::Path;
use tokio::sync::oneshot;
use tracing::{info, warn};

/// The orchestrator's own folder inside the project folder: its cwd, holding its worktree, `CLAUDE.md`, prompt and
/// skills. The project folder itself stays empty of anything Claude Code loads, since every fleet agent's folder
/// sits under it too (docs/superpowers/findings/fleet-spike.md).
pub const ORCHESTRATOR_DIR: &str = "orchestrator";

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

/// Rewrites `BREAKDOWN.md` in the project folder. Only a copy of the record, so a failure is only logged.
fn write_breakdown(workdir: &str, task: &str, v: &BreakdownVersion) {
    let path = Path::new(workdir).join(BREAKDOWN_FILE);
    if let Err(e) = std::fs::write(&path, render(task, v)) {
        warn!(path = %path.display(), error = %e, "cannot write the breakdown");
    }
}
