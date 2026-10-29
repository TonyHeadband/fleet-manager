//! Orchestrated tasks: the orchestrator types, specialists and skills agents are launched with.
pub mod breakdown;
pub mod defs;
pub mod gate;
pub mod ops;
pub mod skills;
pub mod workflow;

/// Names without repeats, in first-seen order.
pub fn unique_names(names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(names.len());
    for n in names {
        if !out.contains(n) {
            out.push(n.clone());
        }
    }
    out
}
