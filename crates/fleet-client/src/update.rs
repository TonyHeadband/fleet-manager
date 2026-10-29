//! Whether a newer fleet-manager is out: GitHub's latest release, compared with the running version.
use serde::Deserialize;
use std::time::Duration;

/// Drafts and pre-releases are not returned here, so the answer is always a published release.
pub const RELEASES_URL: &str =
    "https://api.github.com/repos/TonyHeadband/fleet-manager/releases/latest";

const TIMEOUT: Duration = Duration::from_secs(10);

/// `FLEET_RELEASES_URL` when set (tests point it at a local server), else GitHub.
pub fn releases_url() -> String {
    std::env::var("FLEET_RELEASES_URL").unwrap_or_else(|_| RELEASES_URL.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The tag without its leading `v`.
    pub version: String,
    /// The release notes, as written on GitHub (Markdown).
    pub notes: String,
    /// The release's page, where the archives are.
    pub page: String,
}

/// What the last check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateState {
    /// No check has answered yet.
    Unknown,
    UpToDate,
    Available(Release),
    /// Offline, rate-limited, or an answer that made no sense.
    Failed,
}

impl UpdateState {
    /// What a check's answer means for a program at version `current`.
    pub fn from_answer(current: &str, answer: &Result<Release, String>) -> UpdateState {
        match answer {
            Ok(r) if newer(current, &r.version) => UpdateState::Available(r.clone()),
            Ok(_) => UpdateState::UpToDate,
            Err(_) => UpdateState::Failed,
        }
    }
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
}

/// The latest release at `url`. Every failure is an error message, never a panic: the caller only shows that the
/// check failed.
pub fn latest(url: &str) -> Result<Release, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let mut resp = agent
        .get(url)
        // GitHub refuses requests without one.
        .header(
            "User-Agent",
            concat!("fleet-manager/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    if status != 200 {
        return Err(format!("GitHub answered HTTP {status}"));
    }
    let r: GithubRelease = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("Unreadable release: {e}"))?;
    Ok(Release {
        version: r.tag_name.trim_start_matches('v').to_string(),
        notes: r.body.unwrap_or_default(),
        page: r.html_url,
    })
}

/// `major.minor.patch` of `v`, or `None` for anything else, a pre-release (`-rc1`) included.
fn core(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.split('+').next()?;
    let mut parts = v.split('.').map(|p| p.parse::<u64>().ok());
    let triple = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(triple)
}

/// `candidate` is a later release than `current`. A pre-release candidate is never newer: only published releases
/// are announced.
pub fn newer(current: &str, candidate: &str) -> bool {
    let Some(candidate) = core(candidate) else {
        return false;
    };
    // A running pre-release (`0.2.0-rc1`) is older than its release.
    let (base, pre) = current.split_once('-').unwrap_or((current, ""));
    match core(base) {
        Some(current) => candidate > current || (candidate == current && !pre.is_empty()),
        None => false,
    }
}
