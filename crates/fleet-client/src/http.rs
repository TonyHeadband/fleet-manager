//! Blocking JSON calls to the daemon. A front end runs these on background threads, never on its UI thread.
use crate::schema::{action_request, encode_path_segment};
use fleet_api::{
    ActionDef, Agent, DiffReport, RepoInfo, RepoList, Snapshot, SpawnResponse, UiSchema, Workspace,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::fmt;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(10);
/// For calls that may clone or fetch a repo (spawn, add, grant): a big clone over a slow
/// link takes minutes, and giving up early only hides the result, since the daemon finishes the work anyway.
const LONG_TIMEOUT: Duration = Duration::from_secs(30 * 60);

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
        FleetClient {
            base: base_url.trim_end_matches('/').to_string(),
            agent: config.into(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<ureq::http::Response<ureq::Body>, FleetError> {
        self.send_within(method, path, body, TIMEOUT)
    }

    fn send_within(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<ureq::http::Response<ureq::Body>, FleetError> {
        let url = format!("{}{path}", self.base);
        let sent = match method {
            "GET" => self
                .agent
                .get(&url)
                .config()
                .timeout_global(Some(timeout))
                .build()
                .call(),
            "DELETE" => self
                .agent
                .delete(&url)
                .config()
                .timeout_global(Some(timeout))
                .build()
                .call(),
            "POST" => self
                .agent
                .post(&url)
                .config()
                .timeout_global(Some(timeout))
                .build()
                .send_json(body.unwrap_or(&json!({}))),
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

    fn json<T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<T, FleetError> {
        self.json_within(method, path, body, TIMEOUT)
    }

    fn json_within<T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<T, FleetError> {
        self.send_within(method, path, body, timeout)?
            .body_mut()
            .read_json::<T>()
            .map_err(|e| FleetError(format!("unreadable reply: {e}")))
    }

    pub fn snapshot(&self) -> Result<Snapshot, FleetError> {
        self.json("GET", "/api/agents", None)
    }

    pub fn ui_schema(&self) -> Result<UiSchema, FleetError> {
        self.json("GET", "/api/ui-schema", None)
    }

    pub fn repos(&self) -> Result<Vec<RepoInfo>, FleetError> {
        Ok(self.json::<RepoList>("GET", "/api/repos", None)?.repos)
    }

    /// An empty `repos` starts a project with no repo; the agent can ask for one later.
    pub fn spawn(&self, repos: &[String], prompt: &str) -> Result<SpawnResponse, FleetError> {
        let body = json!({ "actor": "local", "repos": repos, "prompt": prompt, "task_slug": null });
        self.json_within("POST", "/api/agents", Some(&body), LONG_TIMEOUT)
    }

    pub fn add_repo(&self, id: &str, repo: &str) -> Result<Workspace, FleetError> {
        let body = json!({ "actor": "local", "repo": repo });
        self.json_within(
            "POST",
            &format!("/api/agents/{}/repos", encode_path_segment(id)),
            Some(&body),
            LONG_TIMEOUT,
        )
    }

    pub fn grant_repo(&self, id: &str, repo: &str) -> Result<Workspace, FleetError> {
        let body = json!({ "actor": "local", "repo": repo });
        self.json_within(
            "POST",
            &format!("/api/agents/{}/repo-request/grant", encode_path_segment(id)),
            Some(&body),
            LONG_TIMEOUT,
        )
    }

    pub fn refuse_repo(&self, id: &str, reason: &str) -> Result<(), FleetError> {
        let body = json!({ "actor": "local", "reason": reason });
        self.send(
            "POST",
            &format!(
                "/api/agents/{}/repo-request/refuse",
                encode_path_segment(id)
            ),
            Some(&body),
        )
        .map(drop)
    }

    pub fn stop(&self, id: &str) -> Result<(), FleetError> {
        self.send(
            "POST",
            &format!("/api/agents/{}/stop", encode_path_segment(id)),
            None,
        )
        .map(drop)
    }

    /// Runs any action from the daemon's schema; the caller never names an action itself. Returns a message for
    /// the front end to show when an action has one; none does for now.
    pub fn run(
        &self,
        action: &ActionDef,
        agent: &Agent,
        input: Option<&str>,
    ) -> Result<Option<String>, FleetError> {
        let r = action_request(action, agent, input);
        self.send_within(&r.method, &r.path, r.body.as_ref(), TIMEOUT)?;
        Ok(None)
    }

    pub fn diff(&self, agent_id: &str) -> Result<DiffReport, FleetError> {
        self.json(
            "GET",
            &format!("/api/agents/{}/diff", encode_path_segment(agent_id)),
            None,
        )
    }
}
