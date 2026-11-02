//! What the daemon asks a code forge: who the token is, the PRs of a branch, and new issues. GitHub is the only forge
//! so far.

mod github;
pub mod log;
pub mod token;

use fleet_api::{parse_date, CiState, MergeMethod, PrState};
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

/// How one check (a check run or a commit status) stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    /// Success, and also skipped and neutral.
    Pass,
    /// Failure, cancelled, timed out, action required, error...
    Fail,
    /// Queued, running, or pending.
    Pending,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub outcome: CheckOutcome,
}

/// What GitHub says of an open PR's checks and mergeability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrChecks {
    /// Check runs and commit statuses on the PR's head commit.
    pub checks: Vec<Check>,
    /// The names the base branch requires; empty when it requires none.
    pub required: Vec<String>,
    /// GitHub's `mergeable`; None while it is still computing.
    pub mergeable: Option<bool>,
    pub mergeable_state: String,
}

impl PrChecks {
    /// Whether GitHub says the PR can merge now: mergeable, and not held back by its state (`blocked` by a
    /// required check or review, `behind` its base, a `draft`). A failing non-required check (`unstable`) does not
    /// hold it back; the CI state tells about that.
    pub fn can_merge(&self) -> bool {
        self.mergeable == Some(true)
            && !matches!(
                self.mergeable_state.as_str(),
                "dirty" | "blocked" | "behind" | "draft"
            )
    }

    pub fn ci(&self) -> (CiState, Vec<String>) {
        derive_ci(&self.checks, &self.required)
    }
}

/// The CI state and the failing check names. The counted checks are the required ones, or every check when the
/// branch requires none. Red if a counted check failed; else pending while one is running or a required one has not
/// reported; else green. A PR with no required checks and no check or status reported at all is pending, never
/// green: only a PR that passed something is mergeable.
pub fn derive_ci(checks: &[Check], required: &[String]) -> (CiState, Vec<String>) {
    let counted = |c: &&Check| required.is_empty() || required.contains(&c.name);
    let mut failing: Vec<String> = Vec::new();
    for c in checks
        .iter()
        .filter(counted)
        .filter(|c| c.outcome == CheckOutcome::Fail)
    {
        if !failing.contains(&c.name) {
            failing.push(c.name.clone());
        }
    }
    if !failing.is_empty() {
        return (CiState::Red, failing);
    }
    let running = checks
        .iter()
        .filter(counted)
        .any(|c| c.outcome == CheckOutcome::Pending);
    let missing = required
        .iter()
        .any(|r| !checks.iter().any(|c| c.name == *r));
    if running || missing || (required.is_empty() && checks.is_empty()) {
        (CiState::Pending, failing)
    } else {
        (CiState::Green, failing)
    }
}

/// Which merge methods a repo allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergeMethods {
    pub rebase: bool,
    pub squash: bool,
    pub merge: bool,
}

impl MergeMethods {
    /// The allowed methods in the order to try them: rebase, squash, merge commit.
    pub fn candidates(&self) -> Vec<MergeMethod> {
        [
            (self.rebase, MergeMethod::Rebase),
            (self.squash, MergeMethod::Squash),
            (self.merge, MergeMethod::Merge),
        ]
        .into_iter()
        .filter_map(|(allowed, m)| allowed.then_some(m))
        .collect()
    }

    /// Rebase, else squash, else a merge commit; None when the repo allows none.
    pub fn pick(&self) -> Option<MergeMethod> {
        self.candidates().first().copied()
    }
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
    /// The checks and mergeability of the PR `number`, as of its head commit now.
    fn pr_checks(&self, repo: &RepoRef, number: u64) -> Result<PrChecks, ForgeError>;
    /// The merge methods the repo allows.
    fn merge_methods(&self, repo: &RepoRef) -> Result<MergeMethods, ForgeError>;
    /// Merges the PR `number` with `method`. GitHub refuses (`Refused`) a PR that cannot merge, or a method the repo
    /// does not allow.
    fn merge_pr(&self, repo: &RepoRef, number: u64, method: MergeMethod) -> Result<(), ForgeError>;
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

    fn check(name: &str, outcome: CheckOutcome) -> Check {
        Check {
            name: name.into(),
            outcome,
        }
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn with_no_required_checks_every_check_must_pass() {
        use CheckOutcome::*;
        let all_pass = [check("linux", Pass), check("windows", Pass)];
        assert_eq!(derive_ci(&all_pass, &[]), (CiState::Green, vec![]));
        let one_fails = [check("linux", Pass), check("windows", Fail)];
        assert_eq!(
            derive_ci(&one_fails, &[]),
            (CiState::Red, names(&["windows"]))
        );
    }

    #[test]
    fn with_required_checks_only_those_count() {
        use CheckOutcome::*;
        let checks = [check("linux", Pass), check("windows", Fail)];
        assert_eq!(
            derive_ci(&checks, &names(&["linux"])),
            (CiState::Green, vec![])
        );
        assert_eq!(
            derive_ci(&checks, &names(&["linux", "windows"])),
            (CiState::Red, names(&["windows"]))
        );
    }

    #[test]
    fn a_running_check_is_pending_unless_another_failed() {
        use CheckOutcome::*;
        let running = [check("linux", Pass), check("windows", Pending)];
        assert_eq!(derive_ci(&running, &[]), (CiState::Pending, vec![]));
        let failed_too = [check("linux", Fail), check("windows", Pending)];
        assert_eq!(
            derive_ci(&failed_too, &[]),
            (CiState::Red, names(&["linux"]))
        );
    }

    #[test]
    fn a_required_check_that_has_not_reported_is_pending() {
        use CheckOutcome::*;
        let checks = [check("linux", Pass)];
        assert_eq!(
            derive_ci(&checks, &names(&["linux", "audit"])),
            (CiState::Pending, vec![])
        );
    }

    #[test]
    fn a_check_failing_twice_is_named_once() {
        use CheckOutcome::*;
        let checks = [check("ci", Fail), check("ci", Fail)];
        assert_eq!(derive_ci(&checks, &[]), (CiState::Red, names(&["ci"])));
    }

    #[test]
    fn a_pr_with_no_checks_and_no_required_ones_is_pending() {
        assert_eq!(derive_ci(&[], &[]), (CiState::Pending, vec![]));
    }

    #[test]
    fn the_merge_method_is_rebase_else_squash_else_merge() {
        let allow = |rebase, squash, merge| MergeMethods {
            rebase,
            squash,
            merge,
        };
        assert_eq!(allow(true, true, true).pick(), Some(MergeMethod::Rebase));
        assert_eq!(allow(false, true, true).pick(), Some(MergeMethod::Squash));
        assert_eq!(allow(false, false, true).pick(), Some(MergeMethod::Merge));
        assert_eq!(allow(false, false, false).pick(), None);
    }

    #[test]
    fn the_candidates_are_the_allowed_methods_in_order() {
        let all = MergeMethods {
            rebase: true,
            squash: true,
            merge: true,
        };
        assert_eq!(
            all.candidates(),
            vec![MergeMethod::Rebase, MergeMethod::Squash, MergeMethod::Merge]
        );
    }

    #[test]
    fn github_can_merge_a_clean_or_unstable_pr_only() {
        let pr = |mergeable, state: &str| PrChecks {
            checks: vec![],
            required: vec![],
            mergeable,
            mergeable_state: state.into(),
        };
        assert!(pr(Some(true), "clean").can_merge());
        assert!(pr(Some(true), "unstable").can_merge());
        assert!(!pr(Some(true), "blocked").can_merge());
        assert!(!pr(Some(true), "behind").can_merge());
        assert!(!pr(Some(false), "dirty").can_merge());
        assert!(!pr(None, "unknown").can_merge());
    }
}
