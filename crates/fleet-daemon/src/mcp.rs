//! The minimal MCP server each agent reaches at `/internal/mcp/{id}`: JSON-RPC 2.0 over plain HTTP POSTs with
//! `application/json` replies (no SSE, no sessions), which is all Claude Code needs (docs/superpowers/findings/mcp-spike.md).
use crate::agents::{AgentService, RepoAnswer};
use crate::fleet::breakdown::{state_word, BreakdownAnswer};
use fleet_api::{AgentId, AgentKind, Approval, Status, StepDraft};
use serde_json::{json, Value};
use std::sync::Arc;

/// Offered when a client sends no version of its own.
const DEFAULT_PROTOCOL: &str = "2025-06-18";
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_REQUEST: i64 = -32600;
const INTERNAL_ERROR: i64 = -32603;

/// Answers one JSON-RPC message. A notification (no `id`) gets `Value::Null`, which the route sends as 202.
pub async fn handle(svc: &Arc<AgentService>, id: &AgentId, body: Value) -> Value {
    let Some(rpc_id) = body.get("id").cloned() else {
        return Value::Null;
    };
    let Some(method) = body.get("method").and_then(Value::as_str) else {
        return error(rpc_id, INVALID_REQUEST, "invalid request");
    };
    let params = body.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_PROTOCOL);
            ok(
                rpc_id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "fleet-manager", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
        }
        "ping" => ok(rpc_id, json!({})),
        "tools/list" => match svc.agent(id).await {
            Ok(a) => ok(rpc_id, json!({ "tools": tools(a.kind) })),
            Err(e) => error(rpc_id, INTERNAL_ERROR, &e.to_string()),
        },
        "tools/call" => ok(rpc_id, call(svc, id, &params).await),
        _ => error(
            rpc_id,
            METHOD_NOT_FOUND,
            &format!("method not found: {method}"),
        ),
    }
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn text(text: impl Into<String>, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }], "isError": is_error })
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required },
    })
}

fn list_repos_tool() -> Value {
    json!({
        "name": "list_repos",
        "description": "List the repos fleet-manager remembers (name and source) and the ones this task already has (name, path and branch).",
        "inputSchema": { "type": "object", "properties": {} },
    })
}

/// The tools an agent of `kind` is offered: a solo agent asks for repos, an orchestrator runs its fleet, a
/// specialist reports its step. A call to any other is refused (see `call`).
pub fn tools(kind: AgentKind) -> Value {
    let mut list = vec![list_repos_tool()];
    match kind {
        AgentKind::Solo => list.push(tool(
            "request_repo",
            "Ask the owner for access to a repo this task does not have yet. Waits for the decision, which can take a while; \
                on a grant the repo is checked out inside the task folder at the path returned. Never clone or copy repos yourself.",
            json!({
                "repo": { "type": "string", "description": "A remembered repo name (see list_repos), a local path or a git URL." },
                "why": { "type": "string", "description": "What you need it for, shown to the owner." },
            }),
            &["repo", "why"],
        )),
        AgentKind::Orchestrator => list.extend([
            tool(
                "propose_breakdown",
                "Propose the steps of this task for the owner to approve. Waits for the decision: `approved` or \
                    `changes requested: <text>`. Any later change to the steps is a new proposal.",
                json!({ "steps": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "n": { "type": "integer", "description": "The number of a step carried over from the last version; leave out for a new step. \
                                New steps are numbered after the highest number any version has used, in the order given, and depends_on uses those numbers." },
                            "title": { "type": "string" },
                            "specialist": { "type": "string", "description": "A name from your roster." },
                            "brief": { "type": "string", "description": "What the specialist builds and how it knows it is done." },
                            "depends_on": { "type": "array", "items": { "type": "integer" } },
                        },
                        "required": ["title", "specialist", "brief"],
                    },
                }}),
                &["steps"],
            ),
            tool(
                "spawn_specialist",
                "Start the specialist for one step of the approved breakdown, in its own worktree on a branch cut from the task branch.",
                json!({ "step": { "type": "integer" } }),
                &["step"],
            ),
            tool(
                "wait_for_fleet",
                "Wait until a specialist finishes, fails or needs the owner, and say what happened. Returns `no change` after a while; call it again.",
                json!({}),
                &[],
            ),
            tool(
                "fleet_status",
                "The breakdown's version and approval, and each step's state and agent.",
                json!({}),
                &[],
            ),
            tool(
                "open_pr",
                "Run the repo's workflow steps the change needs on the task branch; if they pass, push the branch and open the pull request. \
                    The only way to push.",
                json!({
                    "title": { "type": "string" },
                    "body": { "type": "string" },
                }),
                &["title", "body"],
            ),
        ]),
        AgentKind::Specialist => list.push(tool(
            "finish_step",
            "Report your step as done once everything is committed. Your session ends right after.",
            json!({ "report": { "type": "string", "description": "What you did, for the orchestrator." } }),
            &["report"],
        )),
    }
    Value::Array(list)
}

fn offers(kind: AgentKind, name: &str) -> bool {
    tools(kind)
        .as_array()
        .is_some_and(|l| l.iter().any(|t| t["name"] == name))
}

const ALL_TOOLS: [&str; 8] = [
    "list_repos",
    "request_repo",
    "propose_breakdown",
    "spawn_specialist",
    "wait_for_fleet",
    "fleet_status",
    "open_pr",
    "finish_step",
];

async fn call(svc: &Arc<AgentService>, id: &AgentId, params: &Value) -> Value {
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return text("tools/call needs a tool name", true);
    };
    if !ALL_TOOLS.contains(&name) {
        return text(format!("unknown tool: {name}"), true);
    }
    let kind = match svc.agent(id).await {
        Ok(a) => a.kind,
        Err(e) => return text(e.to_string(), true),
    };
    // Said plainly, so the agent stops asking rather than retrying a tool it was never offered.
    match (kind, name) {
        (AgentKind::Orchestrator, "request_repo") => {
            return text("an orchestrated task has one repo", true)
        }
        (AgentKind::Specialist, "request_repo") => {
            return text("this task has one repo; ask the orchestrator", true)
        }
        _ if !offers(kind, name) => {
            return text(format!("{name} is not available to this agent"), true)
        }
        _ => {}
    }
    match name {
        "list_repos" => list_repos(svc, id).await,
        "request_repo" => {
            let (Some(repo), Some(why)) = (
                args.get("repo").and_then(Value::as_str),
                args.get("why").and_then(Value::as_str),
            ) else {
                return text("request_repo needs `repo` and `why`, both strings", true);
            };
            match svc.request_repo(id, repo, why).await {
                Ok(RepoAnswer::Granted(w)) => text(
                    format!(
                        "granted: {} at {} (branch {})",
                        w.repo, w.worktree, w.branch
                    ),
                    false,
                ),
                Ok(RepoAnswer::Refused(reason)) => text(format!("refused: {reason}"), false),
                Err(e) => text(e.to_string(), true),
            }
        }
        "propose_breakdown" => {
            let Some(steps) = args.get("steps") else {
                return text("propose_breakdown needs `steps`, a list of steps", true);
            };
            let drafts: Vec<StepDraft> = match serde_json::from_value(steps.clone()) {
                Ok(d) => d,
                Err(e) => return text(format!("bad steps: {e}"), true),
            };
            match svc.propose_breakdown(id, drafts).await {
                Ok(BreakdownAnswer::Approved) => text("approved", false),
                Ok(BreakdownAnswer::ChangesRequested(c)) => {
                    text(format!("changes requested: {c}"), false)
                }
                Err(e) => text(e.to_string(), true),
            }
        }
        "spawn_specialist" => {
            let Some(n) = args
                .get("step")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
            else {
                return text("spawn_specialist needs `step`, a step number", true);
            };
            match svc.spawn_specialist(id, n).await {
                Ok(a) => {
                    let branch = a.workspaces.first().map_or("", |w| w.branch.as_str());
                    text(
                        format!("step {n} started: agent {}, branch {branch}", a.id.0),
                        false,
                    )
                }
                Err(e) => text(e.to_string(), true),
            }
        }
        "finish_step" => {
            let Some(report) = args.get("report").and_then(Value::as_str) else {
                return text("finish_step needs `report`, a string", true);
            };
            let n = match svc.agent(id).await {
                Ok(a) => a.step.unwrap_or(0),
                Err(e) => return text(e.to_string(), true),
            };
            match svc.finish_step(id, report).await {
                Ok(()) => text(format!("step {n} done"), false),
                Err(e) => text(e.to_string(), true),
            }
        }
        "wait_for_fleet" => match svc.wait_for_fleet_lines(id).await {
            Ok(lines) if lines.is_empty() => text("no change; call wait_for_fleet again", false),
            Ok(lines) => text(lines.join("\n"), false),
            Err(e) => text(e.to_string(), true),
        },
        "fleet_status" => match svc.fleet_status(id).await {
            Ok(st) => {
                let mut out = format!("breakdown v{} ({})", st.version, approval_word(st.approval));
                for (n, state, live) in st.steps {
                    out.push_str(&format!("\nstep {n}: {}", state_word(state)));
                    if let Some(s) = live {
                        out.push_str(&format!(" (agent {})", status_word(s)));
                    }
                }
                text(out, false)
            }
            Err(e) => text(e.to_string(), true),
        },
        "open_pr" => {
            let (Some(title), Some(body)) = (
                args.get("title").and_then(Value::as_str),
                args.get("body").and_then(Value::as_str),
            ) else {
                return text("open_pr needs `title` and `body`, both strings", true);
            };
            // A failed gate's text is its error message, so every failure reads the same way.
            match svc.open_pr(id, title, body).await {
                Ok(url) => text(url, false),
                Err(e) => text(e.to_string(), true),
            }
        }
        _ => text(format!("unknown tool: {name}"), true),
    }
}

fn approval_word(a: Approval) -> &'static str {
    match a {
        Approval::Proposed => "proposed",
        Approval::Approved => "approved",
        Approval::ChangesRequested => "changes requested",
    }
}

fn status_word(s: Status) -> &'static str {
    match s {
        Status::Queued => "queued",
        Status::Working => "working",
        Status::NeedsYou => "needs you",
        Status::Talking => "talking",
        Status::Review => "idle",
        Status::Stopped => "stopped",
    }
}

async fn list_repos(svc: &AgentService, id: &AgentId) -> Value {
    let agent = match svc.agent(id).await {
        Ok(a) => a,
        Err(e) => return text(e.to_string(), true),
    };
    // Only what a grant could attach: the task's product's repos.
    let rows = match svc.remembered_repos(Some(&agent.product)).await {
        Ok(r) => r,
        Err(e) => return text(e.to_string(), true),
    };
    let listing = json!({
        "remembered": rows.iter().map(|r| json!({ "name": r.name, "source": if r.url.is_some() { "url" } else { "path" } })).collect::<Vec<_>>(),
        "this_task": agent
            .workspaces
            .iter()
            .map(|w| json!({ "name": w.repo, "path": w.worktree, "branch": w.branch }))
            .collect::<Vec<_>>(),
    });
    text(
        serde_json::to_string_pretty(&listing).unwrap_or_default(),
        false,
    )
}
