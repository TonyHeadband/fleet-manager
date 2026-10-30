//! The two caps on new orchestrated tasks: `max_concurrent_tasks` per product and `max_active_products` across them.
//! Only a start is checked; nothing here stops a task that is already working.
use crate::agents::SpawnError;
use fleet_api::{Agent, AgentId, AgentKind, SettingSource, Status, Task, TaskState};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// What a working task holds a slot under: its product and its task folder's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Slot {
    pub product: String,
    pub name: String,
}

/// The orchestrated tasks that are working, by task id: running, with their lead in any status but Review or
/// Stopped. A lead missing from `agents` holds no slot.
pub(crate) fn working(tasks: &[Task], agents: &[Agent]) -> BTreeMap<AgentId, Slot> {
    let status: HashMap<&AgentId, Status> = agents.iter().map(|a| (&a.id, a.status)).collect();
    tasks
        .iter()
        .filter(|t| t.kind == AgentKind::Orchestrator && t.state == TaskState::Running)
        .filter(|t| {
            status
                .get(t.lead.as_ref().unwrap_or(&t.id))
                .is_some_and(|s| !matches!(s, Status::Review | Status::Stopped))
        })
        .map(|t| {
            (
                t.id.clone(),
                Slot {
                    product: t.product.clone(),
                    name: t.name.clone(),
                },
            )
        })
        .collect()
}

/// Whether a new orchestrated task in `product` may start while `held` are working. A product already holding a
/// slot never counts again toward `max_products`.
pub(crate) fn check_start(
    product: &str,
    held: &BTreeMap<AgentId, Slot>,
    max_tasks: Option<(u32, SettingSource)>,
    max_products: Option<u32>,
) -> Result<(), SpawnError> {
    if let Some((value, source)) = max_tasks {
        let mut working: Vec<String> = held
            .values()
            .filter(|s| s.product == product)
            .map(|s| s.name.clone())
            .collect();
        if working.len() >= value as usize {
            working.sort();
            return Err(SpawnError::TooManyTasks {
                product: product.to_string(),
                working,
                value,
                source,
            });
        }
    }
    if let Some(value) = max_products {
        let active: BTreeSet<&str> = held.values().map(|s| s.product.as_str()).collect();
        if !active.contains(product) && active.len() >= value as usize {
            return Err(SpawnError::TooManyProducts {
                active: active.into_iter().map(String::from).collect(),
                value,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fleet_api::StatusTimes;
    use serde_json::json;

    fn task(id: &str, kind: AgentKind, state: TaskState) -> Task {
        serde_json::from_value(json!({
            "id": id, "product": "p", "release": null, "name": format!("{id}-task"), "goal": "",
            "workdir": "", "repos": [], "state": state, "agents_used": 1,
            "times": StatusTimes::default(), "log": [], "kind": kind,
        }))
        .unwrap()
    }

    fn agent(id: &str, status: Status) -> Agent {
        serde_json::from_value(json!({
            "id": id, "goal": "", "workdir": "", "workspaces": [], "session": "", "status": status,
            "activity": "", "status_since": 0, "has_changes": false, "paused": false, "owner": "",
            "blocked_reason": null,
        }))
        .unwrap()
    }

    fn running(id: &str) -> Task {
        task(id, AgentKind::Orchestrator, TaskState::Running)
    }

    /// A running task in `q` whose lead is `lead`, not the task id.
    fn restarted(id: &str, lead: &str) -> Task {
        Task {
            product: "q".into(),
            lead: Some(AgentId(lead.into())),
            ..running(id)
        }
    }

    fn slot(product: &str, name: &str) -> Slot {
        Slot {
            product: product.into(),
            name: name.into(),
        }
    }

    fn held(slots: &[(&str, &str, &str)]) -> BTreeMap<AgentId, Slot> {
        slots
            .iter()
            .map(|(id, product, name)| (AgentId((*id).into()), slot(product, name)))
            .collect()
    }

    #[test]
    fn working_counts_only_running_orchestrated_leads() {
        let tasks = vec![
            running("working"),
            running("needs"),
            running("review"),
            running("stopped-lead"),
            task("stopped", AgentKind::Orchestrator, TaskState::Stopped),
            task("archived", AgentKind::Orchestrator, TaskState::Archived),
            task("solo", AgentKind::Solo, TaskState::Running),
            running("no-lead"),
            restarted("restarted", "new-lead"),
            restarted("old-lead", "gone-lead"),
        ];
        let agents = vec![
            agent("working", Status::Working),
            agent("needs", Status::NeedsYou),
            agent("review", Status::Review),
            agent("stopped-lead", Status::Stopped),
            agent("stopped", Status::Working),
            agent("archived", Status::Working),
            agent("solo", Status::Working),
            agent("restarted", Status::Stopped),
            agent("new-lead", Status::Working),
            agent("old-lead", Status::Working),
            agent("gone-lead", Status::Stopped),
        ];
        assert_eq!(
            working(&tasks, &agents),
            held(&[
                ("needs", "p", "needs-task"),
                ("restarted", "q", "restarted-task"),
                ("working", "p", "working-task"),
            ])
        );
    }

    #[test]
    fn check_start_refuses_a_full_product() {
        let held = held(&[("b", "p", "zeta"), ("a", "p", "alpha"), ("c", "q", "other")]);
        match check_start("p", &held, Some((2, SettingSource::Product)), None) {
            Err(SpawnError::TooManyTasks {
                product,
                working,
                value,
                r#source,
            }) => {
                assert_eq!(product, "p");
                assert_eq!(working, ["alpha", "zeta"]);
                assert_eq!((value, r#source), (2, SettingSource::Product));
            }
            other => panic!("not a task refusal: {other:?}"),
        }
    }

    #[test]
    fn check_start_lets_an_active_product_start_another() {
        let held = held(&[("a", "p", "one"), ("b", "q", "two")]);
        assert!(check_start("p", &held, None, Some(2)).is_ok());
        match check_start("r", &held, None, Some(2)) {
            Err(SpawnError::TooManyProducts { active, value }) => {
                assert_eq!((active, value), (vec!["p".to_string(), "q".to_string()], 2));
            }
            other => panic!("not a product refusal: {other:?}"),
        }
    }

    #[test]
    fn check_start_without_caps_is_ok() {
        let held = held(&[("a", "p", "one"), ("b", "p", "two"), ("c", "q", "three")]);
        assert!(check_start("p", &held, None, None).is_ok());
        assert!(check_start("r", &held, None, None).is_ok());
    }
}
