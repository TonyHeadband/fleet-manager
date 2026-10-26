//! Orchestrated projects in the app: the fleet view behind the Breakdown tab, the decision on a breakdown, Stop
//! and Restart on the project card, and Management's definition files.
use super::{AppResult, FleetApp};
use crate::components::{
    confirm_dialog, pr_url, usable, Confirm, ConfirmDialog, FleetPanel, PreviewTab, TaskPanel,
};
use eframe::egui;
use fleet_api::{
    task_id, Agent, AgentKind, BlockedReason, DecideBreakdown, DefKind, Event, EventKind, FleetView,
};
use fleet_client::schema::groups;
use std::collections::{HashMap, HashSet};

/// A call on an orchestrated project, recorded for tests. Ids are project ids.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub enum FleetCall {
    View(String),
    Decide(String, DecideBreakdown),
    StopTask(String),
    Restart(String),
}

/// A call on a definition file, recorded for tests (the text of a save included).
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub enum DefCall {
    Read(DefKind, String),
    Save(DefKind, String, String),
    Reset(DefKind, String),
}

/// What the side panel adds for `a`: the Breakdown tab on an orchestrator, the link to its project on a fleet
/// agent; nothing on a solo agent.
pub(super) fn panel<'a>(
    a: &'a Agent,
    agents: &'a [Agent],
    views: &'a HashMap<String, Result<FleetView, String>>,
    deciding: &HashMap<String, bool>,
    decided: &HashSet<String>,
    drafts: &'a mut HashMap<String, String>,
) -> Option<FleetPanel<'a>> {
    let task = &task_id(a).0;
    match a.kind {
        AgentKind::Solo => None,
        AgentKind::Orchestrator => Some(FleetPanel::Task(TaskPanel {
            view: views.get(task),
            agents: agents.iter().filter(|m| task_id(m).0 == *task).collect(),
            first: a.blocked_reason == Some(BlockedReason::Breakdown),
            busy: deciding.contains_key(task) || decided.contains(task),
            draft: drafts.entry(task.clone()).or_default(),
        })),
        AgentKind::Specialist => {
            // The project's live orchestrator, which a restart may have replaced since this agent started.
            let lead = groups(agents)
                .into_iter()
                .find(|g| task_id(g.lead).0 == *task)?
                .lead;
            (lead.id != a.id).then_some(FleetPanel::Member {
                task: &lead.goal,
                lead: &lead.id.0,
            })
        }
    }
}

impl FleetApp {
    /// The board's PR mark per project the app has a view of.
    pub(super) fn pr_marks(&self) -> HashMap<String, bool> {
        self.fleet_views
            .iter()
            .filter_map(|(id, v)| Some((id.clone(), pr_url(v.as_ref().ok()?).is_some())))
            .collect()
    }

    #[doc(hidden)]
    pub fn fleet_calls(&self) -> &[FleetCall] {
        &self.fleet_calls
    }

    #[doc(hidden)]
    pub fn definition_calls(&self) -> &[DefCall] {
        &self.def_calls
    }

    /// A project's name for the footer: its lead's task while it is on the board, else the projects list's name.
    fn task_name(&self, task: &str) -> String {
        groups(&self.snapshot.agents)
            .into_iter()
            .find(|g| task_id(g.lead).0 == task)
            .map_or_else(|| self.entry_name(task), |g| g.lead.goal.clone())
    }

    /// The project id of the previewed agent, when it is an orchestrator.
    fn previewed_task(&self) -> Option<String> {
        let id = self.preview_agent()?;
        let a = self.snapshot.agents.iter().find(|a| a.id.0 == id)?;
        (a.kind == AgentKind::Orchestrator).then(|| task_id(a).0.clone())
    }

    /// A panel just opened on `id`: an orchestrator's view is fetched, and its Breakdown tab shows first when the
    /// orchestrator waits on it.
    pub(super) fn opened_fleet_panel(&mut self, id: &str) {
        let Some(a) = self.snapshot.agents.iter().find(|a| a.id.0 == id) else {
            return;
        };
        if a.kind != AgentKind::Orchestrator {
            return;
        }
        let waiting = a.blocked_reason == Some(BlockedReason::Breakdown);
        let task = task_id(a).0.clone();
        if waiting {
            if let Some(p) = self.preview.as_mut() {
                p.tab = PreviewTab::Breakdown;
            }
        }
        self.fetch_fleet_view(task);
    }

    /// Any change of an agent of the previewed orchestrator's project may change its breakdown or gate.
    pub(super) fn follow_fleet_event(&mut self, e: &Event) {
        let id = match &e.kind {
            EventKind::StatusChanged { id, .. } => id,
            EventKind::AgentCreated { agent } => &agent.id,
            _ => return,
        };
        let Some(task) = self.previewed_task() else {
            return;
        };
        let ours = self
            .snapshot
            .agents
            .iter()
            .find(|a| a.id == *id)
            .is_some_and(|a| task_id(a).0 == task);
        if ours {
            self.fetch_fleet_view(task);
        }
    }

    fn fetch_fleet_view(&mut self, task: String) {
        if self.fleet_fetching.contains(&task) {
            self.fleet_stale.insert(task);
            return;
        }
        self.fleet_fetching.insert(task.clone());
        self.fleet_calls.push(FleetCall::View(task.clone()));
        self.background(move |c| {
            AppResult::FleetView(task.clone(), c.fleet_view(&task).map_err(|e| e.0))
        });
    }

    /// Approve (`None`) or request changes on the previewed orchestrator's breakdown, one decision at a time.
    pub(super) fn decide_breakdown(&mut self, comment: Option<String>) {
        let Some(task) = self.previewed_task() else {
            return;
        };
        if self.breakdown_deciding.contains_key(&task) || self.breakdown_decided.contains(&task) {
            return;
        }
        // The version on screen: the daemon refuses the decision if another proposal replaced it meanwhile.
        let Some(version) = self
            .fleet_views
            .get(&task)
            .and_then(|v| v.as_ref().ok())
            .and_then(|v| v.breakdowns.last())
            .map(|v| v.version)
        else {
            return;
        };
        let d = DecideBreakdown {
            approve: comment.is_none(),
            comment,
            version: Some(version),
        };
        self.breakdown_deciding.insert(task.clone(), d.approve);
        self.fleet_calls
            .push(FleetCall::Decide(task.clone(), d.clone()));
        self.background(move |c| {
            AppResult::BreakdownDecided(
                task.clone(),
                c.decide_breakdown(&task, &d).map_err(|e| e.0),
            )
        });
    }

    pub(super) fn ask_stop_task(&mut self, task: String, fleet: usize) {
        let lead_runs = groups(&self.snapshot.agents)
            .into_iter()
            .find(|g| task_id(g.lead).0 == task)
            .is_some_and(|g| g.lead.status != fleet_api::Status::Stopped);
        let agents = match fleet {
            1 => "1 fleet agent".to_string(),
            n => format!("{n} fleet agents"),
        };
        let title = match (lead_runs, fleet) {
            (true, 0) => "Stop the orchestrator?".to_string(),
            (true, _) => format!("Stop the orchestrator and {agents}?"),
            (false, _) => format!("Stop {agents}?"),
        };
        let body =
            "Their sessions end. Folders and branches stay, so the project can be archived later.";
        self.stop_task = Some((task, ConfirmDialog::new(title, body, "Stop all")));
    }

    pub(super) fn stop_task_dialog(&mut self, ctx: &egui::Context) {
        let Some((task, d)) = self.stop_task.as_mut() else {
            return;
        };
        let Some(answer) = confirm_dialog(ctx, d) else {
            return;
        };
        let task = task.clone();
        self.stop_task = None;
        if answer == Confirm::Go {
            self.fleet_calls.push(FleetCall::StopTask(task.clone()));
            self.background(move |c| {
                AppResult::TaskStopped(task.clone(), c.stop_task(&task).map_err(|e| e.0))
            });
        }
    }

    pub(super) fn restart_orchestrator(&mut self, task: String) {
        self.fleet_calls.push(FleetCall::Restart(task.clone()));
        self.background(move |c| {
            AppResult::OrchestratorRestarted(
                task.clone(),
                c.restart_orchestrator(&task).map_err(|e| e.0),
            )
        });
    }

    /// After the definitions changed (a save or reset here, a daemon restart), New project offers only what loads:
    /// a type that no longer does goes back to Solo, and specialists that no longer do leave the roster.
    pub(super) fn keep_spawn_choices(&mut self) {
        let defs = self.definitions.as_deref().unwrap_or_default();
        let orchestrators = usable(defs, DefKind::Orchestrator);
        if self
            .spawn
            .role
            .as_deref()
            .is_some_and(|r| !orchestrators.contains(&r))
        {
            self.spawn.set_type(None);
        }
        let specialists = usable(defs, DefKind::Specialist);
        self.spawn
            .roster
            .retain(|r| specialists.contains(&r.as_str()));
    }

    pub(super) fn open_definition(&mut self, kind: DefKind, name: String) {
        self.def_calls.push(DefCall::Read(kind, name.clone()));
        self.background(move |c| {
            AppResult::Definition(
                kind,
                name.clone(),
                c.definition(kind, &name).map_err(|e| e.0),
            )
        });
    }

    pub(super) fn save_definition(&mut self, kind: DefKind, name: String, text: String) {
        if self.defs.saving.is_some() {
            return;
        }
        self.defs.saving = Some(((kind, name.clone()), text.clone()));
        self.def_calls
            .push(DefCall::Save(kind, name.clone(), text.clone()));
        self.background(move |c| {
            AppResult::DefinitionSaved(
                kind,
                name.clone(),
                c.save_definition(kind, &name, &text).map_err(|e| e.0),
            )
        });
    }

    pub(super) fn ask_reset_definition(&mut self, kind: DefKind, name: String) {
        let d = ConfirmDialog::new(
            format!("Reset {name} to built-in?"),
            "Your copy is deleted and the built-in applies again to agents started after this.",
            "Reset",
        );
        self.reset_definition = Some(((kind, name), d));
    }

    pub(super) fn reset_definition_dialog(&mut self, ctx: &egui::Context) {
        let Some((key, d)) = self.reset_definition.as_mut() else {
            return;
        };
        let Some(answer) = confirm_dialog(ctx, d) else {
            return;
        };
        let (kind, name) = key.clone();
        self.reset_definition = None;
        if answer == Confirm::Go {
            self.def_calls.push(DefCall::Reset(kind, name.clone()));
            self.background(move |c| {
                AppResult::DefinitionReset(
                    kind,
                    name.clone(),
                    c.reset_definition(kind, &name).map_err(|e| e.0),
                )
            });
        }
    }

    pub(super) fn handle_fleet_result(&mut self, r: AppResult) {
        match r {
            AppResult::FleetView(id, result) => {
                self.fleet_fetching.remove(&id);
                match result {
                    Ok(v) => {
                        self.fleet_views.insert(id.clone(), Ok(v));
                        self.breakdown_decided.remove(&id);
                    }
                    // A view already shown stays: one failed refresh does not make it wrong.
                    Err(e) if matches!(self.fleet_views.get(&id), Some(Ok(_))) => {
                        tracing::warn!(notify = "warning", "Could not refresh the breakdown: {e}")
                    }
                    Err(e) => {
                        self.fleet_views.insert(id.clone(), Err(e));
                    }
                }
                if self.fleet_stale.remove(&id) {
                    self.fetch_fleet_view(id);
                }
            }
            AppResult::BreakdownDecided(id, result) => {
                let approved = self.breakdown_deciding.remove(&id).unwrap_or(false);
                match result {
                    Ok(()) => {
                        // The view on screen still shows the version as proposed; deciding it again would only be
                        // refused, so the controls wait for the view that shows the decision.
                        self.breakdown_decided.insert(id.clone());
                        self.breakdown_drafts.remove(&id);
                        let what = if approved {
                            "Breakdown approved"
                        } else {
                            "Changes requested"
                        };
                        tracing::info!(notify = "success", "{what}");
                    }
                    Err(e) => tracing::error!(notify = "error", "{e}"),
                }
                self.fetch_fleet_view(id);
            }
            AppResult::TaskStopped(id, result) => {
                let name = self.task_name(&id);
                match result {
                    Ok(n) => {
                        let agents = if n == 1 { "agent" } else { "agents" };
                        tracing::info!(notify = "success", "{name}: {n} {agents} stopped")
                    }
                    Err(e) => tracing::error!(notify = "error", "Could not stop {name}: {e}"),
                }
                self.fetch_tasks(false);
            }
            AppResult::OrchestratorRestarted(id, result) => match result {
                Ok(a) => {
                    tracing::info!(notify = "success", agent_id = %a.id.0, "Orchestrator restarted")
                }
                Err(e) => tracing::error!(
                    notify = "error",
                    "Could not restart the orchestrator of {}: {e}",
                    self.task_name(&id)
                ),
            },
            AppResult::Definition(kind, name, result) => match result {
                Ok(f) => self.defs.loaded((kind, name), f),
                Err(e) => {
                    self.defs.errors.insert((kind, name), e);
                }
            },
            AppResult::DefinitionSaved(kind, name, result) => {
                match result {
                    Ok(_) => {
                        self.defs.saved();
                        tracing::info!(notify = "success", "{name} saved");
                    }
                    // The message goes under the editor; the draft stays to be fixed.
                    Err(e) => {
                        self.defs.saving = None;
                        self.defs.errors.insert((kind, name), e);
                        return;
                    }
                }
                self.fetch_definitions();
            }
            AppResult::DefinitionReset(kind, name, result) => {
                match result {
                    Ok(()) => {
                        let key = (kind, name.clone());
                        self.defs.reset(&key);
                        tracing::info!(notify = "success", "{name} reset to built-in");
                        if self.defs.selected.as_ref() == Some(&key) {
                            self.open_definition(kind, name);
                        }
                    }
                    // Under the editor, like a failed save.
                    Err(e) => {
                        self.defs.errors.insert((kind, name), e);
                        return;
                    }
                }
                self.fetch_definitions();
            }
            _ => {}
        }
    }
}
