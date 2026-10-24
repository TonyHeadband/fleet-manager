//! Contract-only route descriptions. The daemon implements these; clients and docs read them.
use crate::{Agent, AnswerRequest, ControlRequest, DiffReport, Event, RepoList, Snapshot, SpawnRequest, UiSchema};

#[utoipa::path(get, path = "/api/agents", responses((status = 200, body = Snapshot)))]
pub fn list_agents() {}

#[utoipa::path(post, path = "/api/agents", request_body = SpawnRequest, responses((status = 200, body = Agent)))]
pub fn spawn_agent() {}

#[utoipa::path(delete, path = "/api/agents/{id}", params(("id" = String, Path)), responses((status = 204)))]
pub fn kill_agent() {}

#[utoipa::path(post, path = "/api/agents/{id}/answer", params(("id" = String, Path)), request_body = AnswerRequest, responses((status = 204)))]
pub fn answer_agent() {}

#[utoipa::path(post, path = "/api/agents/{id}/pause", params(("id" = String, Path)), request_body = ControlRequest, responses((status = 204)))]
pub fn pause_agent() {}

#[utoipa::path(post, path = "/api/agents/{id}/resume", params(("id" = String, Path)), request_body = ControlRequest, responses((status = 204)))]
pub fn resume_agent() {}

#[utoipa::path(get, path = "/api/agents/{id}/terminal", params(("id" = String, Path)), responses((status = 200, content_type = "text/event-stream", body = String)))]
pub fn terminal_stream() {}

#[utoipa::path(get, path = "/api/events", responses((status = 200, content_type = "text/event-stream", body = Event)))]
pub fn event_stream() {}

#[utoipa::path(get, path = "/api/ui-schema", responses((status = 200, body = UiSchema)))]
pub fn ui_schema() {}

#[utoipa::path(get, path = "/api/repos", responses((status = 200, body = RepoList)))]
pub fn list_repos() {}

#[utoipa::path(get, path = "/api/agents/{id}/diff", params(("id" = String, Path)), responses((status = 200, body = DiffReport)))]
pub fn agent_diff() {}
