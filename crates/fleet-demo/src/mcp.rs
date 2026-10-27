//! Calls the daemon's fleet tools the way Claude Code does: JSON-RPC 2.0 `tools/call` over a plain HTTP POST.
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

/// Same as `MCP_TOOL_TIMEOUT` in fleet-daemon's agents.rs (copied so the binary does not link the daemon): a held
/// `propose_breakdown` or `wait_for_fleet` legitimately blocks until the owner decides or the fleet changes.
const TOOL_TIMEOUT: Duration = Duration::from_millis(2_400_000);

#[derive(Debug, Clone, PartialEq)]
pub struct McpReply {
    pub text: String,
    pub is_error: bool,
}

pub trait Mcp {
    /// `Err` is a transport or protocol failure; a tool that refuses answers `Ok` with `is_error`.
    fn call(&mut self, tool: &str, args: Value) -> Result<McpReply, String>;
}

impl<M: Mcp + ?Sized> Mcp for Box<M> {
    fn call(&mut self, tool: &str, args: Value) -> Result<McpReply, String> {
        (**self).call(tool, args)
    }
}

pub struct HttpMcp {
    url: String,
    headers: Vec<(String, String)>,
    agent: ureq::Agent,
    next_id: u64,
}

impl HttpMcp {
    /// Reads the `mcp.json` the daemon passes as `--mcp-config`, so calls go exactly where a real CLI's would.
    pub fn from_config(path: &Path) -> Result<HttpMcp, String> {
        let shown = path.display();
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("cannot read {shown}: {e}"))?;
        let json: Value =
            serde_json::from_str(&text).map_err(|e| format!("{shown} is not JSON: {e}"))?;
        let server = &json["mcpServers"]["fleet"];
        let url = server["url"]
            .as_str()
            .ok_or_else(|| format!("{shown} has no mcpServers.fleet.url"))?;
        let headers = match &server["headers"] {
            Value::Null => Vec::new(),
            Value::Object(map) => map
                .iter()
                .map(|(k, v)| {
                    v.as_str()
                        .map(|v| (k.clone(), v.to_string()))
                        .ok_or_else(|| format!("{shown}: header {k} is not a string"))
                })
                .collect::<Result<_, _>>()?,
            _ => {
                return Err(format!(
                    "{shown}: mcpServers.fleet.headers is not an object"
                ))
            }
        };
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TOOL_TIMEOUT))
            .http_status_as_error(false)
            .build()
            .into();
        Ok(HttpMcp {
            url: url.to_string(),
            headers,
            agent,
            next_id: 1,
        })
    }
}

impl Mcp for HttpMcp {
    fn call(&mut self, tool: &str, args: Value) -> Result<McpReply, String> {
        let id = self.next_id;
        self.next_id += 1;
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": tool, "arguments": args },
        });
        let mut req = self.agent.post(&self.url);
        for (k, v) in &self.headers {
            req = req.header(k, v);
        }
        let mut res = req
            .send_json(&body)
            .map_err(|e| format!("POST {}: {e}", self.url))?;
        if !res.status().is_success() {
            return Err(format!("POST {}: HTTP {}", self.url, res.status()));
        }
        let reply: Value = res
            .body_mut()
            .read_json()
            .map_err(|e| format!("POST {}: reply is not JSON: {e}", self.url))?;
        if let Some(message) = reply["error"]["message"].as_str() {
            return Err(message.to_string());
        }
        let result = &reply["result"];
        let text = result["content"][0]["text"]
            .as_str()
            .ok_or_else(|| format!("reply has no result.content[0].text: {reply}"))?;
        Ok(McpReply {
            text: text.to_string(),
            is_error: result["isError"].as_bool().unwrap_or(false),
        })
    }
}

/// Test stand-in: answers each tool from its own queue of scripted replies and logs every call.
#[derive(Debug, Clone, Default)]
pub struct ScriptedMcp {
    replies: HashMap<String, VecDeque<McpReply>>,
    pub calls: Vec<(String, Value)>,
}

impl ScriptedMcp {
    pub fn new<'a>(replies: impl IntoIterator<Item = (&'a str, McpReply)>) -> Self {
        let mut out = Self::default();
        for (tool, reply) in replies {
            out.replies
                .entry(tool.to_string())
                .or_default()
                .push_back(reply);
        }
        out
    }
}

impl Mcp for ScriptedMcp {
    fn call(&mut self, tool: &str, args: Value) -> Result<McpReply, String> {
        self.calls.push((tool.to_string(), args));
        self.replies
            .get_mut(tool)
            .and_then(VecDeque::pop_front)
            .ok_or_else(|| format!("no scripted reply left for {tool}"))
    }
}
