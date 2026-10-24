//! Blocking JSON calls to the daemon. A front end runs these on background threads, never on its UI thread.
use crate::schema::{action_request, encode_path_segment};
use fleet_api::{ActionDef, Agent, DiffReport, RepoList, Snapshot, UiSchema};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::fmt;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(10);

/// What went wrong, in words fit for a user: the daemon's own `{"error": …}` text, `HTTP <status>`, or the
/// transport error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetError(pub String);

impl fmt::Display for FleetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FleetError {}

pub struct FleetClient {
    base: String,
    agent: ureq::Agent,
}

impl FleetClient {
    pub fn new(base_url: &str) -> FleetClient {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            // Error statuses carry the daemon's message in the body, so read them as normal responses.
            .http_status_as_error(false)
            .build();
        FleetClient { base: base_url.trim_end_matches('/').to_string(), agent: config.into() }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    fn send(&self, method: &str, path: &str, body: Option<&Value>) -> Result<ureq::http::Response<ureq::Body>, FleetError> {
        let url = format!("{}{path}", self.base);
        let sent = match method {
            "GET" => self.agent.get(&url).call(),
            "DELETE" => self.agent.delete(&url).call(),
            "POST" => self.agent.post(&url).send_json(body.unwrap_or(&json!({}))),
            other => return Err(FleetError(format!("unsupported method {other}"))),
        };
        let mut resp = sent.map_err(|e| FleetError(e.to_string()))?;
        let status = resp.status().as_u16();
        if status >= 400 {
            let message = resp
                .body_mut()
                .read_json::<Value>()
                .ok()
                .and_then(|v| v.get("error").and_then(Value::as_str).map(String::from))
                .unwrap_or_else(|| format!("HTTP {status}"));
            return Err(FleetError(message));
        }
        Ok(resp)
    }

    fn json<T: DeserializeOwned>(&self, method: &str, path: &str, body: Option<&Value>) -> Result<T, FleetError> {
        self.send(method, path, body)?.body_mut().read_json::<T>().map_err(|e| FleetError(format!("unreadable reply: {e}")))
    }

    pub fn snapshot(&self) -> Result<Snapshot, FleetError> {
        self.json("GET", "/api/agents", None)
    }

    pub fn ui_schema(&self) -> Result<UiSchema, FleetError> {
        self.json("GET", "/api/ui-schema", None)
    }

    pub fn repos(&self) -> Result<Vec<String>, FleetError> {
        Ok(self.json::<RepoList>("GET", "/api/repos", None)?.repos)
    }

    pub fn spawn(&self, repo: &str, prompt: &str) -> Result<Agent, FleetError> {
        let body = json!({ "actor": "local", "repo": repo, "prompt": prompt, "task_slug": null });
        self.json("POST", "/api/agents", Some(&body))
    }

    /// Runs any action from the daemon's schema; the caller never names an action itself.
    pub fn run(&self, action: &ActionDef, agent: &Agent, input: Option<&str>) -> Result<(), FleetError> {
        let r = action_request(action, agent, input);
        self.send(&r.method, &r.path, r.body.as_ref()).map(drop)
    }

    pub fn diff(&self, agent_id: &str) -> Result<DiffReport, FleetError> {
        self.json("GET", &format!("/api/agents/{}/diff", encode_path_segment(agent_id)), None)
    }
}
