//! The fleet tools a supervisor conversation calls over MCP. Each returns the tool's text and whether it failed.
use crate::agents::AgentService;
use fleet_api::{task_id, Actor, AgentId, AgentKind, RepoPick, SpawnRequest, StepState, TaskState};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tracing::info;

pub const NAMES: [&str; 4] = ["fleet_overview", "file_issue", "write_doc", "start_task"];

const PATH_REFUSAL: &str = "path must be specs/<name>.md or plans/<name>.md";
const GH_TIMEOUT: Duration = Duration::from_secs(60);

type Reply = Result<String, String>;

/// Runs `name` for the conversation `id` (offered to supervisors only; the caller checks), logs it and marks the
/// conversation used.
pub async fn call(svc: &AgentService, id: &AgentId, name: &str, args: &Value) -> (String, bool) {
    let agent = match svc.agent(id).await {
        Ok(a) => a,
        Err(e) => return (e.to_string(), true),
    };
    let persona = agent.persona;
    let reply = match name {
        "fleet_overview" => overview(svc, &persona).await,
        "file_issue" => file_issue(svc, &persona, args).await,
        "write_doc" => write_doc(svc, args).await,
        "start_task" => start_task(svc, id, &persona, args).await,
        _ => Err(format!("unknown tool: {name}")),
    };
    info!(tool = name, conversation = %id.0, persona = %persona, ok = reply.is_ok(), "supervisor tool call");
    if let Err(e) = svc
        .store
        .touch_conversation(id, svc.options().clock.now())
        .await
    {
        tracing::warn!(conversation = %id.0, error = %e, "cannot mark the conversation used");
    }
    match reply {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("`{key}` is required, a string"))
}

async fn overview(svc: &AgentService, persona: &str) -> Reply {
    let tasks = svc.store.tasks().await.map_err(|e| e.to_string())?;
    let agents = svc
        .store
        .snapshot()
        .await
        .map_err(|e| e.to_string())?
        .agents;
    let lines: Vec<String> = tasks
        .iter()
        .filter(|t| t.persona == persona && t.state != TaskState::Archived)
        .map(|t| {
            let lead = t.lead.as_ref().unwrap_or(&t.id);
            let status = match agents.iter().find(|a| &a.id == lead) {
                Some(a) => crate::mcp::status_word(a.status),
                None => "stopped",
            };
            let steps = t.breakdowns.last().map_or(&[][..], |b| &b.steps[..]);
            let done = steps.iter().filter(|s| s.state == StepState::Done).count();
            let mut prs: Vec<&str> = t.prs.iter().map(|p| pr_word(p.state)).collect();
            prs.dedup();
            let pr = if prs.is_empty() {
                "none".into()
            } else {
                prs.join("/")
            };
            format!(
                "{} · {} · {status} · steps {done}/{} · PR {pr}",
                t.product,
                t.title.as_deref().unwrap_or(&t.goal),
                steps.len()
            )
        })
        .collect();
    Ok(if lines.is_empty() {
        "no tasks".into()
    } else {
        lines.join("\n")
    })
}

fn pr_word(s: fleet_api::PrState) -> &'static str {
    match s {
        fleet_api::PrState::Open => "open",
        fleet_api::PrState::Merged => "merged",
        fleet_api::PrState::Closed => "closed",
    }
}

/// Refuses a product the daemon does not know, by name.
async fn known_product(svc: &AgentService, product: &str) -> Result<(), String> {
    match svc.store.product(product).await {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(format!("unknown product: {product}")),
        Err(e) => Err(e.to_string()),
    }
}

async fn file_issue(svc: &AgentService, persona: &str, args: &Value) -> Reply {
    let (product, repo) = (str_arg(args, "product")?, str_arg(args, "repo")?);
    let (title, body) = (str_arg(args, "title")?, str_arg(args, "body")?);
    known_product(svc, product).await?;
    let row = match svc.store.repo(product, repo).await {
        Ok(Some(r)) => r,
        Ok(None) => return Err(format!("unknown repo: {repo} (product {product})")),
        Err(e) => return Err(e.to_string()),
    };
    // The persona's own clone is the checkout `gh` reads the remote from; a path-only repo has no other.
    let clone = svc.repos.clone_dir(persona, product, repo);
    let dir: PathBuf = if clone.exists() {
        clone
    } else {
        row.path
            .filter(|p| p.exists())
            .ok_or_else(|| format!("{repo} has no checkout on this machine"))?
    };
    let env = svc
        .personas()
        .gh_env(persona)
        .await
        .map_err(|e| e.to_string())?;
    let mut gh = Command::new(&svc.config().gh_command);
    crate::git::harden(gh.as_std_mut(), &env.vars);
    gh.args(["issue", "create", "--title", title, "--body", body]);
    for label in args
        .get("labels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let label = label.as_str().ok_or("`labels` must be strings")?;
        gh.args(["--label", label]);
    }
    gh.current_dir(dir).stdin(Stdio::null()).kill_on_drop(true);
    match tokio::time::timeout(GH_TIMEOUT, gh.output()).await {
        Ok(Ok(o)) if o.status.success() => {
            Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
        }
        Ok(Ok(o)) => Err(format!(
            "gh issue create failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Ok(Err(e)) => Err(format!("gh cannot run: {e}")),
        Err(_) => Err("gh issue create timed out".into()),
    }
}

/// `specs/<name>.md` or `plans/<name>.md`: one level, so nothing can climb out or reach another folder.
fn doc_path_ok(path: &str) -> bool {
    let Some((dir, name)) = path.split_once('/') else {
        return false;
    };
    matches!(dir, "specs" | "plans")
        && name.ends_with(".md")
        && name.len() > ".md".len()
        && !name.contains(['/', '\\'])
        && name != "README.md"
        && !name.starts_with('.')
}

async fn write_doc(svc: &AgentService, args: &Value) -> Reply {
    let (product, path) = (str_arg(args, "product")?, str_arg(args, "path")?);
    let (content, message) = (str_arg(args, "content")?, str_arg(args, "message")?);
    if !doc_path_ok(path) {
        return Err(PATH_REFUSAL.into());
    }
    known_product(svc, product).await?;
    let sha = svc
        .internal()
        .write_main(product, path, content, message, None)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!("committed {sha} {path}"))
}

async fn start_task(
    svc: &AgentService,
    conversation: &AgentId,
    persona: &str,
    args: &Value,
) -> Reply {
    let product = str_arg(args, "product")?;
    let prompt = str_arg(args, "prompt")?;
    let kind = match str_arg(args, "kind")? {
        "solo" => AgentKind::Solo,
        "orchestrator" => AgentKind::Orchestrator,
        other => return Err(format!("`kind` must be solo or orchestrator, not {other}")),
    };
    known_product(svc, product).await?;
    let mut repos = Vec::new();
    for name in args
        .get("repos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = name.as_str().ok_or("`repos` must be strings")?;
        let row = svc
            .store
            .repo(product, name)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("unknown repo: {name} (product {product})"))?;
        // A path-only repo starts from its checkout; a URL repo from its freshly fetched origin.
        let local = row
            .url
            .is_none()
            .then(|| row.path.map(|p| p.to_string_lossy().into_owned()))
            .flatten();
        repos.push(RepoPick {
            repo: row.name,
            local,
        });
    }
    let mut prompt = prompt.to_string();
    if let Some(plan) = args.get("plan").and_then(Value::as_str) {
        prompt.push_str(&format!("\n\nThe plan is {plan} in the internal repo."));
    }
    let agent = svc
        .spawn(SpawnRequest {
            actor: Actor(format!("supervisor:{}", conversation.0)),
            product: product.to_string(),
            release: None,
            repos,
            prompt,
            slug: None,
            title: args.get("title").and_then(Value::as_str).map(String::from),
            persona: persona.to_string(),
            kind,
            role: args.get("role").and_then(Value::as_str).map(String::from),
        })
        .await
        .map_err(|e| e.to_string())?;
    let task = task_id(&agent).clone();
    let origin = conversation.clone();
    svc.store
        .update_task(&task, |t| t.origin = Some(origin))
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "started {} {}",
        task.0,
        agent.title.as_deref().unwrap_or(&agent.goal)
    ))
}
