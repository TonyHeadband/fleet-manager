//! GitHub and GitHub Enterprise over REST.

use super::{parse_timestamp, Forge, ForgeConfig, ForgeError, NewPr, PrInfo, RepoRef};
use fleet_api::PrState;
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Read;
use std::time::Duration;

/// The REST base of a host: `api.github.com` for github.com, `/api/v3` on a GitHub Enterprise host.
pub fn api_base(host: &str) -> String {
    if host == "github.com" {
        "https://api.github.com".into()
    } else {
        format!("https://{host}/api/v3")
    }
}

pub fn github(host: &str, token: String, cfg: &ForgeConfig, timeout: Duration) -> Box<dyn Forge> {
    let base = cfg
        .api_override
        .as_deref()
        .map_or_else(|| api_base(host), |b| b.trim_end_matches('/').to_string());
    // A 4xx body carries GitHub's own `message`, so it must be readable rather than an error.
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .build()
        .into();
    Box::new(GitHub { agent, base, token })
}

struct GitHub {
    agent: ureq::Agent,
    base: String,
    token: String,
}

#[derive(Deserialize)]
struct RawPr {
    html_url: String,
    number: u64,
    state: String,
    merged_at: Option<String>,
}

impl RawPr {
    fn into_info(self) -> PrInfo {
        let merged_at = self.merged_at.as_deref().and_then(parse_timestamp);
        let state = if merged_at.is_some() {
            PrState::Merged
        } else if self.state == "open" {
            PrState::Open
        } else {
            PrState::Closed
        };
        PrInfo {
            url: self.html_url,
            number: self.number,
            state,
            merged_at,
        }
    }
}

/// Everything but unreserved characters, so a branch like `feat/a#1` survives a query string.
fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn transport(e: ureq::Error) -> ForgeError {
    match e {
        ureq::Error::Timeout(_) => ForgeError::TimedOut,
        other => ForgeError::Transport(other.to_string()),
    }
}

impl GitHub {
    fn with_headers<B>(&self, b: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        b.header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "fleet-manager")
    }

    fn send(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, ForgeError> {
        let url = format!("{}{path}", self.base);
        let sent = match method {
            "GET" => self.with_headers(self.agent.get(&url)).call(),
            "POST" => self
                .with_headers(self.agent.post(&url))
                .send_json(body.unwrap_or(Value::Null)),
            "PATCH" => self
                .with_headers(self.agent.patch(&url))
                .send_json(body.unwrap_or(Value::Null)),
            _ => unreachable!("no other method is used"),
        };
        let mut resp = sent.map_err(transport)?;
        let status = resp.status().as_u16();
        let mut text = String::new();
        resp.body_mut()
            .as_reader()
            .read_to_string(&mut text)
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::TimedOut => ForgeError::TimedOut,
                _ => ForgeError::Transport(e.to_string()),
            })?;
        let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        match status {
            200..=299 => Ok(json),
            401 => Err(ForgeError::Unauthorized),
            _ => Err(ForgeError::Refused {
                status,
                message: json["message"]
                    .as_str()
                    .map_or_else(|| format!("HTTP {status}"), str::to_string),
            }),
        }
    }

    fn pulls(&self, repo: &RepoRef, branch: &str, state: &str) -> Result<Vec<PrInfo>, ForgeError> {
        let head = percent_encode(&format!("{}:{branch}", repo.owner));
        let path = format!(
            "/repos/{}/{}/pulls?head={head}&state={state}&per_page=100",
            repo.owner, repo.name
        );
        let raw: Vec<RawPr> = serde_json::from_value(self.send("GET", &path, None)?)
            .map_err(|e| ForgeError::Transport(format!("unreadable reply: {e}")))?;
        Ok(raw.into_iter().map(RawPr::into_info).collect())
    }
}

impl Forge for GitHub {
    fn whoami(&self) -> Result<String, ForgeError> {
        let user = self.send("GET", "/user", None)?;
        user["login"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| ForgeError::Transport("unreadable reply: no login".into()))
    }

    fn open_pr_of(&self, repo: &RepoRef, branch: &str) -> Result<Option<PrInfo>, ForgeError> {
        Ok(self.pulls(repo, branch, "open")?.into_iter().next())
    }

    fn prs_of(&self, repo: &RepoRef, branch: &str) -> Result<Vec<PrInfo>, ForgeError> {
        self.pulls(repo, branch, "all")
    }

    fn create_pr(&self, repo: &RepoRef, pr: &NewPr) -> Result<PrInfo, ForgeError> {
        let body = json!({
            "title": pr.title, "body": pr.body, "head": pr.head, "base": pr.base, "draft": pr.draft,
        });
        let path = format!("/repos/{}/{}/pulls", repo.owner, repo.name);
        let raw: RawPr = serde_json::from_value(self.send("POST", &path, Some(body))?)
            .map_err(|e| ForgeError::Transport(format!("unreadable reply: {e}")))?;
        Ok(raw.into_info())
    }

    fn update_pr_text(
        &self,
        repo: &RepoRef,
        number: u64,
        title: &str,
        body: &str,
    ) -> Result<(), ForgeError> {
        let path = format!("/repos/{}/{}/pulls/{number}", repo.owner, repo.name);
        self.send("PATCH", &path, Some(json!({"title": title, "body": body})))
            .map(drop)
    }

    fn add_labels(&self, repo: &RepoRef, number: u64, labels: &[String]) -> Result<(), ForgeError> {
        let path = format!("/repos/{}/{}/issues/{number}/labels", repo.owner, repo.name);
        self.send("POST", &path, Some(json!({"labels": labels})))
            .map(drop)
    }
}
