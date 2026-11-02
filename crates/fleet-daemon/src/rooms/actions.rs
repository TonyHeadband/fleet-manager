//! Asks that carry an action: the supervisor's `merge_pr` and `archive_task` put a question to the owner, and the
//! daemon takes the action when the answer is yes. The tool itself never acts.
//!
//! The action is stored with the ask (`pending_actions`) as a typed `PendingAction`, so who may answer is decided in
//! one place (`may_decide`) and a later autopilot has one place to supply the answer. The preconditions are checked
//! twice: when the ask is posted, so the owner is not asked something that cannot happen, and again when the answer
//! is yes, since CI and the task's folders move meanwhile.
use super::deliver::notify;
use super::posts::refuse;
use crate::agents::{one_line, AgentService, ServiceError};
use crate::supervisor::tools::str_arg;
use fleet_api::{format_bytes, AgentId, ArchivePlan, MemberRef, Post, PostKind, TaskState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// Why anyone but the owner is refused an ask that carries an action.
pub const ACTION_OWNER_DECIDES: &str = "this ask acts on a yes: only the owner answers it";

/// What the daemon does when the owner says yes to the ask it is attached to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum PendingAction {
    MergePr {
        task: AgentId,
        #[serde(default)]
        repo: Option<String>,
    },
    ArchiveTask {
        task: AgentId,
    },
}

/// Whether `by` may answer an ask that carries an action. Only the owner; the supervisor, who asks, never. This is
/// the one place to change when an autopilot may answer in the owner's stead.
pub fn may_decide(by: &MemberRef) -> bool {
    matches!(by, MemberRef::Owner)
}

/// Whether the answer is a yes: the word, nothing else. A comment or a no does not act.
pub fn is_yes(text: &str) -> bool {
    let t = text.trim().trim_end_matches(['.', '!']).to_lowercase();
    matches!(t.as_str(), "yes" | "y" | "yes please")
}

/// The uncommitted files an archive of this plan would delete, as a refusal, or None when nothing is lost.
fn lost_work(plan: &ArchivePlan) -> Option<String> {
    let n: u32 = plan
        .items
        .iter()
        .filter(|i| !i.stays)
        .map(|i| i.uncommitted_files)
        .sum();
    (n > 0).then(|| {
        format!(
            "{n} uncommitted file{} would be lost; commit or discard {} first",
            if n == 1 { "" } else { "s" },
            if n == 1 { "it" } else { "them" }
        )
    })
}

/// The task, if it is one of the persona's and not archived already.
async fn own_task(
    svc: &AgentService,
    persona: &str,
    args: &Value,
) -> Result<fleet_api::Task, String> {
    let id = AgentId(str_arg(args, "task")?.to_string());
    match svc.store.task(&id).await.map_err(|e| e.to_string())? {
        Some(t) if t.persona == persona => {
            if t.state == TaskState::Archived {
                Err(format!("task {} is archived already", id.0))
            } else {
                Ok(t)
            }
        }
        _ => Err(format!("unknown task: {}", id.0)),
    }
}

fn title_of(t: &fleet_api::Task) -> &str {
    t.title.as_deref().unwrap_or(&t.goal)
}

fn posted(ask: i64, what: &str) -> String {
    format!("asked the owner (ask {ask}); {what} only if they answer yes. Nothing has happened yet")
}

/// `merge_pr`: asks the owner to merge the task's PR, which must be green and mergeable now.
pub async fn ask_merge(
    svc: &AgentService,
    conversation: &AgentId,
    persona: &str,
    args: &Value,
) -> Result<String, String> {
    let t = own_task(svc, persona, args).await?;
    let repo = args.get("repo").and_then(Value::as_str).map(String::from);
    let (pr, number) = svc
        .mergeable_pr(&t.id, repo.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    let text = format!(
        "Merge PR #{number}? {} ({}), CI is green",
        title_of(&t),
        pr.repo
    );
    let action = PendingAction::MergePr {
        task: t.id.clone(),
        repo: Some(pr.repo),
    };
    let ask = svc
        .post_action_ask(conversation, &t.product, &text, &action)
        .await
        .map_err(|e| e.to_string())?;
    Ok(posted(ask, "the daemon merges"))
}

/// `archive_task`: asks the owner to archive the task, which must be stopped, with nothing to lose.
pub async fn ask_archive(
    svc: &AgentService,
    conversation: &AgentId,
    persona: &str,
    args: &Value,
) -> Result<String, String> {
    let t = own_task(svc, persona, args).await?;
    // The app's plan: it refuses a running task and one with an open PR.
    let plan = svc.archive_plan(&t.id).await.map_err(|e| e.to_string())?;
    if let Some(lost) = lost_work(&plan) {
        return Err(lost);
    }
    let text = format!(
        "Archive task {}? It frees {}, and nothing is uncommitted",
        title_of(&t),
        format_bytes(plan.total)
    );
    let action = PendingAction::ArchiveTask { task: t.id.clone() };
    let ask = svc
        .post_action_ask(conversation, &t.product, &text, &action)
        .await
        .map_err(|e| e.to_string())?;
    Ok(posted(ask, "the daemon archives"))
}

impl AgentService {
    /// Posts the supervisor's ask to the owner, in its operation room, else the task's product room, and attaches
    /// the action. Returns the ask's id.
    async fn post_action_ask(
        &self,
        conversation: &AgentId,
        product: &str,
        text: &str,
        action: &PendingAction,
    ) -> Result<i64, ServiceError> {
        let (who, me) = self.member_of(conversation).await?;
        let room = match self.room_of(&who, &me, None).await {
            Ok(r) => r,
            Err(_) => {
                let id = super::product_room(&self.store, product).await?;
                self.room_of(&who, &me, Some(&id)).await?
            }
        };
        let mut p = self.new_post(&room, me, PostKind::Ask, text)?;
        p.to = Some(MemberRef::Owner);
        let p = self.store_post(&room, p).await?;
        let json = serde_json::to_string(action).map_err(|e| refuse(e.to_string()))?;
        self.store.set_pending_action(p.id, &json).await?;
        Ok(p.id)
    }

    /// The action attached to the ask, if any.
    pub(super) async fn pending_action_of(
        &self,
        ask: i64,
    ) -> Result<Option<PendingAction>, ServiceError> {
        Ok(self
            .store
            .pending_action(ask)
            .await?
            .and_then(|j| serde_json::from_str(&j).ok()))
    }

    /// Takes the action if the answer is yes, and tells the room and the asker what came of it. The preconditions
    /// are checked again here, by the same calls the app's buttons use.
    pub(super) async fn settle_action(
        self: &Arc<Self>,
        q: &Post,
        action: &PendingAction,
        answer: &Post,
    ) {
        let outcome = if is_yes(&answer.text) {
            self.take_action(action).await
        } else {
            format!("not done: the owner answered {}", one_line(&answer.text))
        };
        if let Ok(Some(room)) = self.store.room(&q.room).await {
            if let Ok(p) = self.new_post(
                &room,
                MemberRef::Daemon,
                PostKind::Announcement,
                &format!("Ask {}: {outcome}", q.id),
            ) {
                if let Err(e) = self.store_post(&room, p).await {
                    tracing::warn!(ask = q.id, error = %one_line(&e.to_string()), "cannot post the outcome");
                }
            }
        }
        if let MemberRef::Agent(asker) = &q.sender {
            let text = format!("the owner answered your ask {}: {outcome}", q.id);
            if let Err(e) = notify(self, asker, &text).await {
                tracing::warn!(ask = q.id, error = %one_line(&e.to_string()), "cannot tell the asker the outcome");
            }
        }
    }

    async fn take_action(&self, action: &PendingAction) -> String {
        match action {
            PendingAction::MergePr { task, repo } => {
                match self.merge_pr(task, repo.as_deref()).await {
                    Ok(m) => format!("merged PR #{} in {} with {:?}", m.number, m.repo, m.method),
                    Err(e) => format!("not merged: {e}"),
                }
            }
            PendingAction::ArchiveTask { task } => {
                let plan = match self.archive_plan(task).await {
                    Ok(p) => p,
                    Err(e) => return format!("not archived: {e}"),
                };
                if let Some(lost) = lost_work(&plan) {
                    return format!("not archived: {lost}");
                }
                match self.archive(task, &plan.items).await {
                    Ok(r) => format!("archived {}, freed {}", task.0, format_bytes(r.freed)),
                    Err(e) => format!("not archived: {e}"),
                }
            }
        }
    }
}
