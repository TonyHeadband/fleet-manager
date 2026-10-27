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
