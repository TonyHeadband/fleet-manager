//! GitHub and GitHub Enterprise over REST.

use super::{
    parse_timestamp, Check, CheckOutcome, Forge, ForgeConfig, ForgeError, IssueInfo, NewPr,
    PrChecks, PrInfo, RepoRef,
};
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

/// A check run: anything not completed is pending; skipped and neutral pass like success.
fn run_outcome(status: &str, conclusion: Option<&str>) -> CheckOutcome {
    if status != "completed" {
        return CheckOutcome::Pending;
    }
    match conclusion {
        Some("success" | "skipped" | "neutral") => CheckOutcome::Pass,
        _ => CheckOutcome::Fail,
    }
}

fn status_outcome(state: &str) -> CheckOutcome {
    match state {
        "success" => CheckOutcome::Pass,
        "pending" => CheckOutcome::Pending,
        _ => CheckOutcome::Fail,
    }
}

/// Results per page asked of the checks endpoints, and the most pages read of one: a PR with more is cut off.
const CHECKS_PER_PAGE: usize = 100;
const CHECKS_MAX_PAGES: usize = 10;

fn unreadable(what: &str) -> ForgeError {
    ForgeError::Transport(format!("unreadable reply: {what}"))
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

    /// Every item of `field` in the pages of `path`, which must end in a query string.
    fn pages(&self, path: &str, field: &str) -> Result<Vec<Value>, ForgeError> {
        let mut all = Vec::new();
        for page in 1..=CHECKS_MAX_PAGES {
            let reply = self.send(
                "GET",
                &format!("{path}?per_page={CHECKS_PER_PAGE}&page={page}"),
                None,
            )?;
            let items = reply[field].as_array().ok_or_else(|| unreadable(field))?;
            all.extend(items.iter().cloned());
            if items.len() < CHECKS_PER_PAGE {
                break;
            }
        }
        Ok(all)
    }

    /// The contexts the branch requires; none when it is not protected (GitHub answers 404).
    fn required_checks(&self, repo: &RepoRef, branch: &str) -> Result<Vec<String>, ForgeError> {
        let path = format!(
            "/repos/{}/{}/branches/{}/protection/required_status_checks",
            repo.owner,
            repo.name,
            percent_encode(branch)
        );
        let reply = match self.send("GET", &path, None) {
            Ok(r) => r,
            Err(ForgeError::Refused { status: 404, .. }) => return Ok(vec![]),
            Err(e) => return Err(e),
        };
        let mut names: Vec<String> = Vec::new();
        let listed = reply["contexts"].as_array().into_iter().flatten();
        let checks = reply["checks"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| &c["context"]);
        for name in listed.chain(checks).filter_map(Value::as_str) {
            if !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
        Ok(names)
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

    fn pr_checks(&self, repo: &RepoRef, number: u64) -> Result<PrChecks, ForgeError> {
        let (owner, name) = (&repo.owner, &repo.name);
        let pr = self.send(
            "GET",
            &format!("/repos/{owner}/{name}/pulls/{number}"),
            None,
        )?;
        let sha = pr["head"]["sha"]
            .as_str()
            .ok_or_else(|| unreadable("no head commit"))?;
        let base = pr["base"]["ref"]
            .as_str()
            .ok_or_else(|| unreadable("no base branch"))?;
        let mut checks = Vec::new();
        let runs = self.pages(
            &format!("/repos/{owner}/{name}/commits/{sha}/check-runs"),
            "check_runs",
        )?;
        for r in runs {
            let name = r["name"]
                .as_str()
                .ok_or_else(|| unreadable("a check run has no name"))?;
            let status = r["status"].as_str().unwrap_or("queued");
            checks.push(Check {
                name: name.to_string(),
                outcome: run_outcome(status, r["conclusion"].as_str()),
            });
        }
        let statuses = self.pages(
            &format!("/repos/{owner}/{name}/commits/{sha}/status"),
            "statuses",
        )?;
        for st in statuses {
            let context = st["context"]
                .as_str()
                .ok_or_else(|| unreadable("a status has no context"))?;
            let state = st["state"].as_str().unwrap_or("error");
            checks.push(Check {
                name: context.to_string(),
                outcome: status_outcome(state),
            });
        }
        Ok(PrChecks {
            checks,
            required: self.required_checks(repo, base)?,
            mergeable: pr["mergeable"].as_bool(),
            mergeable_state: pr["mergeable_state"]
                .as_str()
                .unwrap_or("unknown")
                .to_string(),
        })
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
            .map(|_| ())
    }

    fn add_labels(&self, repo: &RepoRef, number: u64, labels: &[String]) -> Result<(), ForgeError> {
        let path = format!("/repos/{}/{}/issues/{number}/labels", repo.owner, repo.name);
        self.send("POST", &path, Some(json!({"labels": labels})))
            .map(|_| ())
    }

    fn create_issue(
        &self,
        repo: &RepoRef,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<IssueInfo, ForgeError> {
        let path = format!("/repos/{}/{}/issues", repo.owner, repo.name);
        let reply = self.send(
            "POST",
            &path,
            Some(json!({"title": title, "body": body, "labels": labels})),
        )?;
        match (reply["html_url"].as_str(), reply["number"].as_u64()) {
            (Some(url), Some(number)) => Ok(IssueInfo {
                url: url.into(),
                number,
            }),
            _ => Err(ForgeError::Transport(
                "unreadable reply: no issue URL or number".into(),
            )),
        }
    }
}
