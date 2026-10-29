use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// One commit in a product's internal resources repo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct InternalCommit {
    pub sha: String,
    /// Unix seconds.
    #[ts(type = "number")]
    pub at: i64,
    pub author: String,
    pub subject: String,
}

/// The branches of a product's internal repo: `main` first, then `task/<name>` branches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct InternalBranches {
    pub branches: Vec<String>,
    #[serde(default)]
    pub remote: InternalRemote,
}

/// Where a product's internal repo pushes to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct InternalRemote {
    /// The remote, credentials redacted; None while the repo is local-only.
    pub url: Option<String>,
    /// Commits on `main` the remote does not have yet.
    pub unpushed: u32,
    /// The last push failure, until a push succeeds.
    pub error: Option<String>,
    /// `fleet/unpushed-*` branches on the remote: earlier notes that conflicted with the remote's `main`, waiting to
    /// be merged by hand. As of the daemon's last fetch.
    #[serde(default)]
    pub parked: Vec<String>,
}

/// Every file on one branch, as repo-relative paths.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct InternalFiles {
    pub branch: String,
    pub files: Vec<String>,
}

/// One text file as committed on a branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct InternalFile {
    pub branch: String,
    pub path: String,
    pub text: String,
}

/// Commits of a branch or of one path on it, newest first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct InternalHistory {
    pub commits: Vec<InternalCommit>,
}
