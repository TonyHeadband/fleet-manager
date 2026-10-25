//! The orchestrated parts of `AgentService`: what an orchestrator project adds to a solo spawn.
use super::defs::{Defs, FleetError, OrchestratorDef};
use super::skills;
use crate::agents::{instructions_section, repo_section, AgentService, SpawnError};
use crate::hooks_cfg::RoleSettings;
use crate::personas::PersonaEnv;
use crate::repos::Resolved;
use crate::store::StoreError;
use fleet_api::{AgentId, AgentKind, DefKind, SpawnRequest, Workspace};
use std::path::Path;

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
        if self.roster.is_empty() {
            text.push_str("No specialists: there is no one to give steps to.\n");
        }
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
/// it); `open_pr` pushes with the persona's env alone.
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
