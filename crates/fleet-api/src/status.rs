use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Status {
    Queued,
    Working,
    NeedsYou,
    Talking,
    Review,
    /// Not running any more; `Agent::stop_cause` says why. `dead`, the old name, is still read from stored rows.
    Stopped,
}

/// Why an agent is `Stopped`: the owner stopped it, claude exited by itself, the session vanished, or a restart could
/// not resume it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StopCause {
    Stopped,
    Exited,
    Crashed,
    /// A specialist reported its step done with `finish_step`, which ended its session.
    Finished,
    /// A daemon restart could not resume it; the activity says why. Not a crash: no warning, not counted as one.
    Interrupted,
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
    /// The orchestrator proposed a breakdown and waits for the owner's decision.
    Breakdown,
}

impl Status {
    pub const ALL: [Status; 6] = [
        Status::Queued,
        Status::Working,
        Status::NeedsYou,
        Status::Talking,
        Status::Review,
        Status::Stopped,
    ];
}

/// Wire form that still accepts `dead`. ts-rs cannot parse `#[serde(alias)]` on `Status` itself (it would warn on
/// every build), so the alias lives on this private mirror and `Status` deserialises through it by hand.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum StatusWire {
    Queued,
    Working,
    NeedsYou,
    Talking,
    Review,
    #[serde(alias = "dead")]
    Stopped,
}

impl From<StatusWire> for Status {
    fn from(w: StatusWire) -> Self {
        match w {
            StatusWire::Queued => Status::Queued,
            StatusWire::Working => Status::Working,
            StatusWire::NeedsYou => Status::NeedsYou,
            StatusWire::Talking => Status::Talking,
            StatusWire::Review => Status::Review,
            StatusWire::Stopped => Status::Stopped,
        }
    }
}

impl<'de> Deserialize<'de> for Status {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StatusWire::deserialize(d).map(Status::from)
    }
}
