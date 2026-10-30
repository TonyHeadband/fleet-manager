//! Blocking JSON calls to the daemon. A front end runs these on background threads, never on its UI thread.
use crate::schema::{action_request, encode_path_segment};
use fleet_api::{
    ActionDef, Agent, AgentConfig, AgentConfigUpdate, ArchiveOrphanRequest, ArchivePlan,
    ArchiveReport, ArchiveRequest, CheckoutInfo, CheckoutList, DecideBreakdown, DefFile, DefInfo,
    DefKind, DiffReport, EffectiveSettings, FleetView, HiddenSpecialistsPut, InternalBranches,
    InternalCommit, InternalFile, InternalFiles, InternalHistory, NewPersona, NewProduct,
    NewRelease, OverridesPut, Persona, PersonaList, PersonaUpdate, Product, ProductList,
    ProductOverride, ProductUpdate, Release, ReleaseUpdate, RepoInfo, RepoList, RepoUpsert,
    SetTaskRelease, Snapshot, SpawnRequest, SpawnResponse, StopTaskResponse, Task, TaskList,
    ToolError, ToolList, UiSchema, VersionInfo, Workspace,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::fmt;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(10);
/// The daemon runs the two CLI checks side by side with 10 s each, so ~10 s at worst; this leaves room.
const CHECK_TIMEOUT: Duration = Duration::from_secs(25);
/// A tool check reads the login shell's PATH (5 s at most) and then asks every tool's version at once (10 s each);
/// a recheck may first wait out a check already in flight. A GET answers at once once a check has run, but the first
/// one waits for it.
const TOOLS_TIMEOUT: Duration = Duration::from_secs(45);
/// For calls that may clone or fetch a repo (spawn, add, grant, a product save that connects its internal repo): a
/// big clone over a slow link takes minutes, and giving up early only hides the result, since the daemon finishes
/// the work anyway.
const LONG_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// What went wrong, in words fit for a user: the daemon's own `{"error": …}` text, `HTTP <status>`, or the
/// transport error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetError {
    pub message: String,
    /// Boxed to keep every `Result` small. Set when the daemon refused because a tool is missing, too old or not logged in (HTTP 424).
    pub tool: Option<Box<ToolError>>,
    /// The form input the refusal is about, when the daemon names one.
    pub field: Option<String>,
}

impl FleetError {
    pub fn new(message: impl Into<String>) -> FleetError {
        FleetError {
            message: message.into(),
            tool: None,
            field: None,
        }
    }
}

impl fmt::Display for FleetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FleetError {}

impl From<String> for FleetError {
    fn from(message: String) -> FleetError {
        FleetError::new(message)
    }
}

impl From<&str> for FleetError {
    fn from(message: &str) -> FleetError {
        FleetError::new(message)
    }
}

/// A request body as JSON; the contract types always serialise, so this fails only on a broken build.
fn to_value(body: &impl serde::Serialize) -> Result<Value, FleetError> {
    serde_json::to_value(body)
        .map_err(|e| FleetError::new(format!("cannot encode the request: {e}")))
}

fn definition_path(kind: DefKind, name: &str) -> String {
    let kind = match kind {
        DefKind::Orchestrator => "orchestrator",
        DefKind::Specialist => "specialist",
        DefKind::Skill => "skill",
        DefKind::PrStyle => "pr_style",
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
            other => return Err(FleetError::new(format!("unsupported method {other}"))),
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
        let mut resp = sent.map_err(|e| FleetError::new(e.to_string()))?;
        let status = resp.status().as_u16();
        if status >= 400 {
            let body = resp.body_mut().read_json::<Value>().ok();
            let message = body
                .as_ref()
                .and_then(|v| v.get("error").and_then(Value::as_str).map(String::from))
                .unwrap_or_else(|| format!("HTTP {status}"));
            let field = body
                .as_ref()
                .and_then(|v| v.get("field").and_then(Value::as_str).map(String::from));
            let tool = body
                .and_then(|v| v.get("tool").cloned())
                .and_then(|t| serde_json::from_value(t).ok())
                .map(Box::new);
            return Err(FleetError {
                message,
                tool,
                field,
            });
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
            .map_err(|e| FleetError::new(format!("unreadable reply: {e}")))
    }

    pub fn snapshot(&self) -> Result<Snapshot, FleetError> {
        self.json("GET", "/api/agents", None)
    }

    pub fn ui_schema(&self) -> Result<UiSchema, FleetError> {
        self.json("GET", "/api/ui-schema", None)
    }

    /// What the daemon found of each tool it needs, from its last check; the daemon checks first when none has run
    /// yet, so this may take as long as a recheck.
    pub fn tools(&self) -> Result<ToolList, FleetError> {
        self.json_within("GET", "/api/tools", None, TOOLS_TIMEOUT)
    }

    /// Checks the tools again now; runs their version commands on the daemon, so it takes longer than most calls.
    pub fn recheck_tools(&self) -> Result<ToolList, FleetError> {
        self.json_within("POST", "/api/tools/recheck", None, TOOLS_TIMEOUT)
    }

    /// The daemon's version; `None` for an older daemon that has no such endpoint (404).
    pub fn version(&self) -> Result<Option<VersionInfo>, FleetError> {
        let url = format!("{}/api/version", self.base);
        let resp = self
            .agent
            .get(&url)
            .call()
            .map_err(|e| FleetError::new(e.to_string()))?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        Self::checked(Ok(resp))?
            .body_mut()
            .read_json::<VersionInfo>()
            .map(Some)
            .map_err(|e| FleetError::new(format!("unreadable reply: {e}")))
    }

    pub fn repos(&self) -> Result<Vec<RepoInfo>, FleetError> {
        Ok(self.json::<RepoList>("GET", "/api/repos", None)?.repos)
    }

    /// The repos of one product: what its tasks can start with.
    pub fn repos_for(&self, product: &str) -> Result<Vec<RepoInfo>, FleetError> {
        let path = format!("/api/repos?product={}", encode_path_segment(product));
        Ok(self.json::<RepoList>("GET", &path, None)?.repos)
    }

    /// The live checkouts of a product repo's local folder on the daemon's machine, main first.
    pub fn checkouts(&self, product: &str, name: &str) -> Result<Vec<CheckoutInfo>, FleetError> {
        let path = format!(
            "/api/products/{}/repos/{}/checkouts",
            encode_path_segment(product),
            encode_path_segment(name)
        );
        Ok(self.json::<CheckoutList>("GET", &path, None)?.checkouts)
    }

    /// Every product with its repos and releases, oldest first.
    pub fn products(&self) -> Result<Vec<Product>, FleetError> {
        Ok(self
            .json::<ProductList>("GET", "/api/products", None)?
            .products)
    }

    pub fn create_product(&self, p: &NewProduct) -> Result<Product, FleetError> {
        self.json_within("POST", "/api/products", Some(&to_value(p)?), LONG_TIMEOUT)
    }

    pub fn update_product(&self, id: &str, u: &ProductUpdate) -> Result<Product, FleetError> {
        let path = format!("/api/products/{}", encode_path_segment(id));
        self.json_within("PATCH", &path, Some(&to_value(u)?), LONG_TIMEOUT)
    }

    /// Replaces the product's overrides with `overrides`; one bad entry refuses the whole list.
    pub fn put_product_overrides(
        &self,
        id: &str,
        overrides: Vec<ProductOverride>,
    ) -> Result<Product, FleetError> {
        let path = format!("/api/products/{}/overrides", encode_path_segment(id));
        self.json("PUT", &path, Some(&to_value(&OverridesPut { overrides })?))
    }

    /// Replaces the specialists the product hides from its orchestrators; a name that is not a usable specialist
    /// refuses the whole list, named.
    pub fn put_hidden_specialists(
        &self,
        id: &str,
        hidden: Vec<String>,
    ) -> Result<Product, FleetError> {
        let path = format!(
            "/api/products/{}/hidden-specialists",
            encode_path_segment(id)
        );
        self.json(
            "PUT",
            &path,
            Some(&to_value(&HiddenSpecialistsPut { hidden })?),
        )
    }

    /// The value and source of every overridable setting, with `product`'s overrides applied when given.
    pub fn effective_settings(
        &self,
        product: Option<&str>,
    ) -> Result<EffectiveSettings, FleetError> {
        let path = match product {
            Some(p) => format!("/api/settings/effective?product={}", encode_path_segment(p)),
            None => "/api/settings/effective".to_string(),
        };
        self.json("GET", &path, None)
    }

    /// The branches of the product's internal repo (none yet when it has none) and where it pushes to.
    pub fn internal_branches(&self, product: &str) -> Result<InternalBranches, FleetError> {
        let path = format!(
            "/api/products/{}/internal/branches",
            encode_path_segment(product)
        );
        self.json("GET", &path, None)
    }

    pub fn internal_files(&self, product: &str, branch: &str) -> Result<Vec<String>, FleetError> {
        let path = format!(
            "/api/products/{}/internal/files?branch={}",
            encode_path_segment(product),
            encode_path_segment(branch)
        );
        Ok(self.json::<InternalFiles>("GET", &path, None)?.files)
    }

    /// The text of one file as committed on `branch`.
    pub fn internal_file(
        &self,
        product: &str,
        branch: &str,
        path: &str,
    ) -> Result<String, FleetError> {
        let url = format!(
            "/api/products/{}/internal/file?branch={}&path={}",
            encode_path_segment(product),
            encode_path_segment(branch),
            encode_path_segment(path)
        );
        Ok(self.json::<InternalFile>("GET", &url, None)?.text)
    }

    /// Newest first; `path` narrows it to the commits that touched that file.
    pub fn internal_history(
        &self,
        product: &str,
        branch: &str,
        path: Option<&str>,
    ) -> Result<Vec<InternalCommit>, FleetError> {
        let mut url = format!(
            "/api/products/{}/internal/history?branch={}",
            encode_path_segment(product),
            encode_path_segment(branch)
        );
        if let Some(p) = path {
            url.push_str(&format!("&path={}", encode_path_segment(p)));
        }
        Ok(self.json::<InternalHistory>("GET", &url, None)?.commits)
    }

    /// Refused while the product has task records, archived ones included.
    pub fn delete_product(&self, id: &str) -> Result<(), FleetError> {
        let path = format!("/api/products/{}", encode_path_segment(id));
        self.send("DELETE", &path, None).map(drop)
    }

    /// Adds a repo to the product, or fills in the path or URL a repo of that name lacks.
    pub fn upsert_repo(&self, product: &str, r: &RepoUpsert) -> Result<RepoInfo, FleetError> {
        let path = format!("/api/products/{}/repos", encode_path_segment(product));
        self.json("PUT", &path, Some(&to_value(r)?))
    }

    pub fn delete_repo(&self, product: &str, name: &str) -> Result<(), FleetError> {
        let path = format!(
            "/api/products/{}/repos/{}",
            encode_path_segment(product),
            encode_path_segment(name)
        );
        self.send("DELETE", &path, None).map(drop)
    }

    pub fn create_release(&self, product: &str, r: &NewRelease) -> Result<Release, FleetError> {
        let path = format!("/api/products/{}/releases", encode_path_segment(product));
        self.json("POST", &path, Some(&to_value(r)?))
    }

    pub fn update_release(&self, id: i64, u: &ReleaseUpdate) -> Result<Release, FleetError> {
        self.json("PATCH", &format!("/api/releases/{id}"), Some(&to_value(u)?))
    }

    /// The release's tasks go back to the backlog.
    pub fn delete_release(&self, id: i64) -> Result<(), FleetError> {
        self.send("DELETE", &format!("/api/releases/{id}"), None)
            .map(drop)
    }

    /// Moves a task into a release of its product, or to the backlog with `None`.
    pub fn set_task_release(&self, task: &str, release: Option<i64>) -> Result<Task, FleetError> {
        let path = format!("/api/tasks/{}/release", encode_path_segment(task));
        self.json("PUT", &path, Some(&to_value(&SetTaskRelease { release })?))
    }

    /// Marks a task with no GitHub repo done; the daemon refuses (409) a task with one.
    pub fn mark_done(&self, task: &str) -> Result<Task, FleetError> {
        let path = format!("/api/tasks/{}/done", encode_path_segment(task));
        self.json("POST", &path, None)
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
            .map_err(|e| FleetError::new(format!("unreadable reply: {e}")))
    }

    /// Deletes the user's copy so the built-in applies again; an error when there is no user copy.
    pub fn reset_definition(&self, kind: DefKind, name: &str) -> Result<(), FleetError> {
        self.send("DELETE", &definition_path(kind, name), None)
            .map(drop)
    }

    /// Starts a task in `req.product`, under `req.persona` for good. An empty `repos` starts it with no repo; the
    /// agent can ask for one later. An orchestrated task (`kind` `Orchestrator`, `role` its type) takes exactly one
    /// repo; its orchestrator may use every specialist the product does not hide.
    pub fn spawn(&self, req: &SpawnRequest) -> Result<SpawnResponse, FleetError> {
        self.json_within("POST", "/api/agents", Some(&to_value(req)?), LONG_TIMEOUT)
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

    /// Types `text` into the agent's pane, as a click on its card's answer box does.
    pub fn answer(&self, id: &str, text: &str) -> Result<(), FleetError> {
        let body = json!({ "actor": "local", "text": text });
        self.send(
            "POST",
            &format!("/api/agents/{}/answer", encode_path_segment(id)),
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

    pub fn tasks(&self) -> Result<TaskList, FleetError> {
        self.json("GET", "/api/tasks", None)
    }

    /// Asks the daemon to rescan for orphans and re-measure; it answers at once and the results show up in later
    /// `tasks()` calls.
    pub fn refresh_tasks(&self) -> Result<(), FleetError> {
        self.send("POST", "/api/tasks/refresh", None).map(drop)
    }

    // The plan weighs the folders and the archive removes them, so a big tree takes long: LONG_TIMEOUT, as the
    // daemon finishes the work whether or not the client waits.
    pub fn archive_plan(&self, id: &str) -> Result<ArchivePlan, FleetError> {
        self.json_within(
            "GET",
            &format!("/api/tasks/{}/archive-plan", encode_path_segment(id)),
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
            &format!("/api/tasks/{}/archive", encode_path_segment(id)),
            Some(&to_value(&body)?),
            LONG_TIMEOUT,
        )
    }

    /// `id` is the task's: its record id, which is its first orchestrator's.
    pub fn fleet_view(&self, id: &str) -> Result<FleetView, FleetError> {
        self.json(
            "GET",
            &format!("/api/tasks/{}/fleet", encode_path_segment(id)),
            None,
        )
    }

    /// Stops every agent of the task still running, its fleet first; returns how many it stopped.
    pub fn stop_task(&self, id: &str) -> Result<u32, FleetError> {
        let r: StopTaskResponse = self.json(
            "POST",
            &format!("/api/tasks/{}/stop", encode_path_segment(id)),
            None,
        )?;
        Ok(r.stopped)
    }

    /// Starts a fresh orchestrator on a task whose orchestrator has stopped; returns it. The persona's logins
    /// are checked first, as for a spawn.
    pub fn restart_orchestrator(&self, id: &str) -> Result<Agent, FleetError> {
        self.json_within(
            "POST",
            &format!("/api/tasks/{}/restart", encode_path_segment(id)),
            None,
            CHECK_TIMEOUT,
        )
    }

    pub fn decide_breakdown(&self, id: &str, d: &DecideBreakdown) -> Result<(), FleetError> {
        self.send(
            "POST",
            &format!("/api/tasks/{}/breakdown/decide", encode_path_segment(id)),
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
}
