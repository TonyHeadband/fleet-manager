//! What the daemon asks a code forge: who the token is, and the PRs of a branch. GitHub is the only forge so far.

mod github;
pub mod log;

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
}

#[derive(Clone, Debug, Default)]
pub struct ForgeConfig {
    /// When set, every host's API base is this URL (tests point it at a fake forge).
    pub api_override: Option<String>,
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
