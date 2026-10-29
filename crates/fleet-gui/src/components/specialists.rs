use crate::theme::{size, tokens};
use crate::widgets::chip;
use eframe::egui::{RichText, Ui};
use fleet_api::{DefInfo, DefKind, Persona};
use std::collections::{BTreeMap, HashMap};

/// Each persona's nicknames for its specialists, by persona id. Only what a person reads uses them; the agents
/// and the daemon know specialists by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Nicknames(HashMap<String, BTreeMap<String, String>>);

static NO_NICKNAMES: BTreeMap<String, String> = BTreeMap::new();

impl Nicknames {
    pub fn new(personas: &[Persona]) -> Nicknames {
        Nicknames(
            personas
                .iter()
                .filter(|p| !p.nicknames.is_empty())
                .map(|p| (p.id.clone(), p.nicknames.clone()))
                .collect(),
        )
    }

    /// The persona's nicknames; none for one that is not listed.
    pub fn of(&self, persona: &str) -> &BTreeMap<String, String> {
        self.0.get(persona).unwrap_or(&NO_NICKNAMES)
    }
}

/// What a person reads for `specialist`: `Ferris (rust)` when it has a nickname, else its name. The name stays in
/// view because briefs, prompts and logs use it.
pub fn specialist_label(nicknames: &BTreeMap<String, String>, specialist: &str) -> String {
    match nicknames.get(specialist) {
        Some(nick) => format!("{nick} ({specialist})"),
        None => specialist.to_string(),
    }
}

/// The names of the `kind` definitions that load, in the daemon's order. A broken one is never offered.
pub fn usable(defs: &[DefInfo], kind: DefKind) -> Vec<&str> {
    defs.iter()
        .filter(|d| d.kind == kind && d.error.is_none())
        .map(|d| d.name.as_str())
        .collect()
}

/// A toggle chip per usable specialist, adding to or removing from `picked`, then each name of `stale` struck
/// through: a specialist the list names that no longer loads, so it is shown rather than lost silently, and never
/// sent. Returns true when the user toggled a chip.
pub fn specialist_chips(
    ui: &mut Ui,
    specialists: &[&str],
    picked: &mut Vec<String>,
    stale: &[&str],
) -> bool {
    let low = tokens(ui.visuals().dark_mode).text_low;
    let mut toggled = false;
    ui.horizontal_wrapped(|ui| {
        for &name in specialists {
            let on = picked.iter().any(|r| r == name);
            if chip(ui, name, on).clicked() {
                if on {
                    picked.retain(|r| r != name);
                } else {
                    picked.push(name.to_string());
                }
                toggled = true;
            }
        }
        for &name in stale {
            ui.label(
                RichText::new(name)
                    .size(size::SMALL)
                    .strikethrough()
                    .color(low),
            )
            .on_hover_text("This specialist no longer loads: see Management");
        }
    });
    toggled
}
