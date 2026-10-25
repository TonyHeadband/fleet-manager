//! The minimal MCP server each agent reaches at `/internal/mcp/{id}`: JSON-RPC 2.0 over plain HTTP POSTs with
//! `application/json` replies (no SSE, no sessions), which is all Claude Code needs (docs/superpowers/findings/mcp-spike.md).
use crate::agents::{AgentService, RepoAnswer};
use fleet_api::AgentId;
use serde_json::{json, Value};

/// Offered when a client sends no version of its own.
const DEFAULT_PROTOCOL: &str = "2025-06-18";
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_REQUEST: i64 = -32600;

/// Answers one JSON-RPC message. A notification (no `id`) gets `Value::Null`, which the route sends as 202.
pub async fn handle(svc: &AgentService, id: &AgentId, body: Value) -> Value {
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
        "tools/list" => ok(rpc_id, json!({ "tools": tools() })),
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

fn tools() -> Value {
    json!([
        {
            "name": "list_repos",
            "description": "List the repos fleet-manager remembers (name and source) and the ones this project already has (name, path and branch).",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "request_repo",
            "description": "Ask the owner for access to a repo this project does not have yet. Waits for the decision, which can take a while; \
                on a grant the repo is checked out inside the project folder at the path returned. Never clone or copy repos yourself.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repo": { "type": "string", "description": "A remembered repo name (see list_repos), a local path or a git URL." },
                    "why": { "type": "string", "description": "What you need it for, shown to the owner." },
                },
                "required": ["repo", "why"],
            },
        },
    ])
}

async fn call(svc: &AgentService, id: &AgentId, params: &Value) -> Value {
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    match params.get("name").and_then(Value::as_str) {
        Some("list_repos") => list_repos(svc, id).await,
        Some("request_repo") => {
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
        Some(other) => text(format!("unknown tool: {other}"), true),
        None => text("tools/call needs a tool name", true),
    }
}

async fn list_repos(svc: &AgentService, id: &AgentId) -> Value {
    let (rows, agent) = match (svc.remembered_repos().await, svc.agent(id).await) {
        (Ok(r), Ok(a)) => (r, a),
        (Err(e), _) | (_, Err(e)) => return text(e.to_string(), true),
    };
    let listing = json!({
        "remembered": rows.iter().map(|r| json!({ "name": r.name, "source": r.source })).collect::<Vec<_>>(),
        "this_project": agent
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
