//! What the daemon asks a code forge: who the token is, the PRs of a branch, and new issues. GitHub is the only forge
//! so far.

mod github;
pub mod log;
pub mod token;

use fleet_api::{parse_date, PrState};
use std::fmt;

pub use github::{api_base, github};

/// A repo on a forge host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoRef {
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl RepoRef {
    /// None for a URL that does not name exactly an owner and a repo.
    pub fn from_url(url: &str) -> Option<RepoRef> {
        let rest = url.split_once("://").map_or(url, |(_, r)| r);
        let rest = rest.trim_end_matches('/');
        let rest = rest.strip_suffix(".git").unwrap_or(rest);
        let parts: Vec<&str> = rest.split('/').collect();
        match parts[..] {
            [host, owner, name] if [host, owner, name].iter().all(|p| !p.is_empty()) => {
                Some(RepoRef {
                    host: host.into(),
                    owner: owner.into(),
                    name: name.into(),
                })
            }
            _ => None,
        }
    }

    /// `<host>/<owner>/<repo>`, the form `gh -R` takes.
    pub fn slug(&self) -> String {
        format!("{}/{}/{}", self.host, self.owner, self.name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrInfo {
    pub url: String,
    pub number: u64,
    pub state: PrState,
    /// Unix seconds.
    pub merged_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueInfo {
    pub url: String,
    pub number: u64,
}

pub struct NewPr<'a> {
    pub head: &'a str,
    pub base: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub draft: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForgeError {
    /// The forge answered with a refusal; `message` is its own.
    Refused {
        status: u16,
        message: String,
    },
    Unauthorized,
    Transport(String),
    TimedOut,
}

impl fmt::Display for ForgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ForgeError::Refused { message, .. } => f.write_str(message),
            ForgeError::Unauthorized => f.write_str("the token was refused"),
            ForgeError::Transport(e) => f.write_str(e),
            ForgeError::TimedOut => f.write_str("the forge did not answer in time"),
        }
    }
}

impl std::error::Error for ForgeError {}

/// Blocking calls: run them off the async runtime (see `git::blocking`).
pub trait Forge: Send + Sync {
    /// The login the token belongs to.
    fn whoami(&self) -> Result<String, ForgeError>;
    /// The open PR whose head is `branch`.
    fn open_pr_of(&self, repo: &RepoRef, branch: &str) -> Result<Option<PrInfo>, ForgeError>;
    /// Every PR, in any state, whose head is `branch`.
    fn prs_of(&self, repo: &RepoRef, branch: &str) -> Result<Vec<PrInfo>, ForgeError>;
    fn create_pr(&self, repo: &RepoRef, pr: &NewPr) -> Result<PrInfo, ForgeError>;
    fn update_pr_text(
        &self,
        repo: &RepoRef,
        number: u64,
        title: &str,
        body: &str,
    ) -> Result<(), ForgeError>;
    fn add_labels(&self, repo: &RepoRef, number: u64, labels: &[String]) -> Result<(), ForgeError>;
    fn create_issue(
        &self,
        repo: &RepoRef,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<IssueInfo, ForgeError>;
}

#[derive(Clone, Debug, Default)]
pub struct ForgeConfig {
    /// When set, every host's API base is this URL (tests point it at a fake forge).
    pub api_override: Option<String>,
}

/// Whether `url` is a plain-http loopback address (`http://127.0.0.1:<port>` or `http://localhost:<port>`, with an
/// optional path). The only kind of API override the daemon accepts: a token is sent to it.
pub fn is_loopback_api(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let (authority, _path) = rest.split_once('/').unwrap_or((rest, ""));
    let Some((host, port)) = authority.split_once(':') else {
        return false;
    };
    matches!(host, "127.0.0.1" | "localhost")
        && !port.is_empty()
        && port.bytes().all(|b| b.is_ascii_digit())
}

/// `2026-10-26T10:00:00Z` as Unix seconds. Forges print UTC; anything else is None.
pub(crate) fn parse_timestamp(s: &str) -> Option<i64> {
    let (date, time) = s.split_once('T')?;
    let time = time.strip_suffix('Z')?;
    let time = time.split_once('.').map_or(time, |(t, _)| t);
    let mut hms = time.split(':').map(|p| p.parse::<i64>().ok());
    let (h, m, sec) = (hms.next()??, hms.next()??, hms.next()??);
    if hms.next().is_some() || h > 23 || m > 59 || sec > 60 {
        return None;
    }
    Some(parse_date(date)? * 86_400 + h * 3600 + m * 60 + sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_http_is_an_api_override() {
        for ok in [
            "http://127.0.0.1:8080",
            "http://localhost:1",
            "http://127.0.0.1:8080/api/v3",
        ] {
            assert!(is_loopback_api(ok), "{ok}");
        }
        for bad in [
            "",
            "https://127.0.0.1:8080",
            "http://127.0.0.1",
            "http://127.0.0.1:",
            "http://example.com:80",
            "http://localhost@evil.test:80",
            "http://127.0.0.1.evil.test:80",
            "http://127.0.0.1:80@evil.test",
            "http://localhost:80x",
            "ftp://localhost:80",
        ] {
            assert!(!is_loopback_api(bad), "{bad}");
        }
    }

    #[test]
    fn timestamps_are_utc_seconds() {
        assert_eq!(
            parse_timestamp("2026-10-26T10:00:05Z"),
            Some(20752 * 86_400 + 36_005)
        );
        assert_eq!(parse_timestamp("1970-01-01T00:00:00.5Z"), Some(0));
        assert_eq!(parse_timestamp("2026-10-26 10:00:00"), None);
        assert_eq!(parse_timestamp("2026-10-26T25:00:00Z"), None);
    }
}
