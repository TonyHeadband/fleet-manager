use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Status {
    Queued,
    Working,
    NeedsYou,
    Talking,
    Review,
    Dead,
}

/// Why a Needs you card is blocked, so the owner knows what kind of attention it wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BlockedReason {
    Permission,
    Question,
    Error,
    Startup,
    RepoAccess,
}

impl Status {
    pub const ALL: [Status; 6] = [
        Status::Queued,
        Status::Working,
        Status::NeedsYou,
        Status::Talking,
        Status::Review,
        Status::Dead,
    ];
}
