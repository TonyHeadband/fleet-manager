//! Orchestrated tasks: the orchestrator types, specialists and skills agents are launched with.
pub mod breakdown;
pub mod defs;
pub mod gate;
pub mod ops;
pub mod skills;
pub mod workflow;

/// A roster without repeats, in first-seen order: a name listed twice would put the specialist twice in the
/// orchestrator's `CLAUDE.md`.
pub fn unique_roster(names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(names.len());
    for n in names {
        if !out.contains(n) {
            out.push(n.clone());
        }
    }
    out
}
