//! Blocking JSON calls to the daemon. A front end runs these on background threads, never on its UI thread.
use crate::schema::{action_request, encode_path_segment};
use fleet_api::{
    ActionDef, Agent, AgentConfig, AgentConfigUpdate, ArchiveOrphanRequest, ArchivePlan,
    ArchiveReport, ArchiveRequest, DecideBreakdown, DefFile, DefInfo, DefKind, DiffReport,
    FleetView, NewPersona, Persona, PersonaList, PersonaUpdate, ProjectList, RepoInfo, RepoList,
    Snapshot, SpawnResponse, StopProjectResponse, UiSchema, Workspace,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::fmt;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(10);
/// The daemon runs the two CLI checks side by side with 10 s each, so ~10 s at worst; this leaves room.
const CHECK_TIMEOUT: Duration = Duration::from_secs(25);
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

/// A request body as JSON; the contract types always serialise, so this fails only on a broken build.
fn to_value(body: &impl serde::Serialize) -> Result<Value, FleetError> {
    serde_json::to_value(body).map_err(|e| FleetError(format!("cannot encode the request: {e}")))
}

fn definition_path(kind: DefKind, name: &str) -> String {
    let kind = match kind {
        DefKind::Orchestrator => "orchestrator",
        DefKind::Specialist => "specialist",
        DefKind::Skill => "skill",
    };
    format!("/api/definitions/{kind}/{}", encode_path_segment(name))
}

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
            "PATCH" => self
                .agent
                .patch(&url)
                .config()
                .timeout_global(Some(timeout))
                .build()
                .send_json(body.unwrap_or(&json!({}))),
            "PUT" => self
                .agent
                .put(&url)
                .config()
                .timeout_global(Some(timeout))
                .build()
                .send_json(body.unwrap_or(&json!({}))),
            "POST" => self
                .agent
                .post(&url)
                .config()
                .timeout_global(Some(timeout))
                .build()
                .send_json(body.unwrap_or(&json!({}))),
            other => return Err(FleetError(format!("unsupported method {other}"))),
        };
        Self::checked(sent)
    }

    /// A PUT whose body is plain text: a definition is a Markdown file, sent as it is.
    fn put_text(
        &self,
        path: &str,
        text: &str,
    ) -> Result<ureq::http::Response<ureq::Body>, FleetError> {
        let url = format!("{}{path}", self.base);
        Self::checked(
            self.agent
                .put(&url)
                .content_type("text/markdown; charset=utf-8")
                .send(text),
        )
    }

    /// The daemon's `{"error": …}` text for an error status, so the user reads its words.
    fn checked(
        sent: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<ureq::http::Response<ureq::Body>, FleetError> {
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

    /// The repos one persona has remembered: what its projects can start with.
    pub fn repos_for(&self, persona: &str) -> Result<Vec<RepoInfo>, FleetError> {
        let path = format!("/api/repos?persona={}", encode_path_segment(persona));
        Ok(self.json::<RepoList>("GET", &path, None)?.repos)
    }

    pub fn personas(&self) -> Result<PersonaList, FleetError> {
        self.json("GET", "/api/personas", None)
    }

    pub fn create_persona(&self, p: &NewPersona) -> Result<Persona, FleetError> {
        self.json("POST", "/api/personas", Some(&to_value(p)?))
    }

    pub fn update_persona(&self, id: &str, p: &PersonaUpdate) -> Result<Persona, FleetError> {
        let path = format!("/api/personas/{}", encode_path_segment(id));
        self.json("PATCH", &path, Some(&to_value(p)?))
    }

    pub fn delete_persona(&self, id: &str) -> Result<(), FleetError> {
        let path = format!("/api/personas/{}", encode_path_segment(id));
        self.send("DELETE", &path, None).map(drop)
    }

    /// Runs the `claude` and `gh` status checks on the daemon, so it takes longer than most calls.
    pub fn check_persona(&self, id: &str) -> Result<Persona, FleetError> {
        let path = format!("/api/personas/{}/check", encode_path_segment(id));
        self.json_within("POST", &path, None, CHECK_TIMEOUT)
    }

    pub fn agent_config(&self) -> Result<AgentConfig, FleetError> {
        self.json("GET", "/api/agent-config", None)
    }

    pub fn save_agent_config(&self, c: &AgentConfigUpdate) -> Result<AgentConfig, FleetError> {
        self.json("PUT", "/api/agent-config", Some(&to_value(c)?))
    }

    pub fn definitions(&self) -> Result<Vec<DefInfo>, FleetError> {
        self.json("GET", "/api/definitions", None)
    }

    pub fn definition(&self, kind: DefKind, name: &str) -> Result<DefFile, FleetError> {
        self.json("GET", &definition_path(kind, name), None)
    }

    /// Validated by the daemon before it is saved; the error names the file and the key or skill at fault.
    pub fn save_definition(
        &self,
        kind: DefKind,
        name: &str,
        text: &str,
    ) -> Result<DefInfo, FleetError> {
        self.put_text(&definition_path(kind, name), text)?
            .body_mut()
            .read_json()
            .map_err(|e| FleetError(format!("unreadable reply: {e}")))
    }

    /// Deletes the user's copy so the built-in applies again; an error when there is no user copy.
    pub fn reset_definition(&self, kind: DefKind, name: &str) -> Result<(), FleetError> {
        self.send("DELETE", &definition_path(kind, name), None)
            .map(drop)
    }

    /// An empty `repos` starts a project with no repo; the agent can ask for one later. The project runs under
    /// `persona` for good.
    pub fn spawn(
        &self,
        persona: &str,
        repos: &[String],
        prompt: &str,
    ) -> Result<SpawnResponse, FleetError> {
        let body = json!({ "actor": "local", "repos": repos, "prompt": prompt, "task_slug": null, "persona": persona });
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

    pub fn projects(&self) -> Result<ProjectList, FleetError> {
        self.json("GET", "/api/projects", None)
    }

    /// Asks the daemon to rescan for orphans and re-measure; it answers at once and the results show up in later
    /// `projects()` calls.
    pub fn refresh_projects(&self) -> Result<(), FleetError> {
        self.send("POST", "/api/projects/refresh", None).map(drop)
    }

    // The plan weighs the folders and the archive removes them, so a big tree takes long: LONG_TIMEOUT, as the
    // daemon finishes the work whether or not the client waits.
    pub fn archive_plan(&self, id: &str) -> Result<ArchivePlan, FleetError> {
        self.json_within(
            "GET",
            &format!("/api/projects/{}/archive-plan", encode_path_segment(id)),
            None,
            LONG_TIMEOUT,
        )
    }

    /// Archives against `plan`, the one the user confirmed. When the folder changed since, the daemon removes nothing
    /// and the error is `fleet_api::PLAN_CHANGED`: fetch a fresh plan and ask again.
    pub fn archive(&self, id: &str, plan: &ArchivePlan) -> Result<ArchiveReport, FleetError> {
        let body = ArchiveRequest {
            items: plan.items.clone(),
        };
        self.json_within(
            "POST",
            &format!("/api/projects/{}/archive", encode_path_segment(id)),
            Some(&to_value(&body)?),
            LONG_TIMEOUT,
        )
    }

    /// `id` is the project's: its record id, which is its first orchestrator's.
    pub fn fleet_view(&self, id: &str) -> Result<FleetView, FleetError> {
        self.json(
            "GET",
            &format!("/api/projects/{}/fleet", encode_path_segment(id)),
            None,
        )
    }

    /// Stops every agent of the project still running, its fleet first; returns how many it stopped.
    pub fn stop_project(&self, id: &str) -> Result<u32, FleetError> {
        let r: StopProjectResponse = self.json(
            "POST",
            &format!("/api/projects/{}/stop", encode_path_segment(id)),
            None,
        )?;
        Ok(r.stopped)
    }

    /// Starts a fresh orchestrator on a project whose orchestrator has stopped; returns it.
    /// Starts a fresh orchestrator on a project whose orchestrator has stopped; returns it. The persona's logins
    /// are checked first, as for a spawn.
    pub fn restart_orchestrator(&self, id: &str) -> Result<Agent, FleetError> {
        self.json_within(
            "POST",
            &format!("/api/projects/{}/restart", encode_path_segment(id)),
            None,
            CHECK_TIMEOUT,
        )
    }

    pub fn decide_breakdown(&self, id: &str, d: &DecideBreakdown) -> Result<(), FleetError> {
        self.send(
            "POST",
            &format!("/api/projects/{}/breakdown/decide", encode_path_segment(id)),
            Some(&to_value(d)?),
        )
        .map(drop)
    }

    pub fn orphan_plan(&self, id: &str) -> Result<ArchivePlan, FleetError> {
        self.json_within(
            "POST",
            "/api/orphans/plan",
            Some(&json!({ "id": id })),
            LONG_TIMEOUT,
        )
    }

    /// `archive` for an orphan, by its id.
    pub fn archive_orphan(
        &self,
        id: &str,
        plan: &ArchivePlan,
    ) -> Result<ArchiveReport, FleetError> {
        let body = ArchiveOrphanRequest {
            id: id.to_string(),
            items: plan.items.clone(),
        };
        self.json_within(
            "POST",
            "/api/orphans/archive",
            Some(&to_value(&body)?),
            LONG_TIMEOUT,
        )
    }

    pub fn end_session(&self, id: &str) -> Result<(), FleetError> {
        self.send(
            "POST",
            "/api/orphans/end-session",
            Some(&json!({ "id": id })),
        )
        .map(drop)
    }
}
