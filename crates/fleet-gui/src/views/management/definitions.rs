//! Management → Orchestrators, Specialists and Skills: every definition of one kind, built-in or the user's, with
//! its file. A built-in is read-only until Override copies it into a draft that saves as the user's file.
use super::personas::{error_line, note};
use super::{save_bar, ManagementAction};
use crate::components::caption;
use crate::theme::{management, mono, size, tokens};
use crate::widgets::{button, list_row, warn_mark, ButtonVariant};
use eframe::egui::{self, Frame, Label, Margin, Panel, RichText, ScrollArea, TextEdit, Ui};
use fleet_api::{DefFile, DefInfo, DefKind};
use std::collections::HashMap;

const ROWS: usize = 24;

pub type DefKey = (DefKind, String);

/// The definitions tabs' state. Drafts are kept per definition, so moving between them or between tabs loses none.
#[derive(Default)]
pub struct DefsState {
    pub selected: Option<DefKey>,
    /// Each opened definition's file as the daemon last sent it.
    pub files: HashMap<DefKey, DefFile>,
    /// Editable text: the user's file being edited, or a built-in being overridden. A built-in without one is shown
    /// read-only.
    pub drafts: HashMap<DefKey, String>,
    /// Why the last load or save of a definition failed.
    pub errors: HashMap<DefKey, String>,
    /// The save in flight and the text it sent.
    pub saving: Option<(DefKey, String)>,
}

impl DefsState {
    /// The daemon's file arrived. A draft being edited stays; a user file gets one to edit.
    pub fn loaded(&mut self, key: DefKey, file: DefFile) {
        if !file.builtin {
            self.drafts
                .entry(key.clone())
                .or_insert_with(|| file.text.clone());
        }
        self.errors.remove(&key);
        self.files.insert(key, file);
    }

    /// A save landed: what it sent is now the user's file.
    pub fn saved(&mut self) {
        if let Some((key, text)) = self.saving.take() {
            if let Some(f) = self.files.get_mut(&key) {
                f.text = text;
                f.builtin = false;
            }
            self.errors.remove(&key);
        }
    }

    /// The user's copy is gone: the built-in is read again, with no draft over it.
    pub fn reset(&mut self, key: &DefKey) {
        self.drafts.remove(key);
        self.files.remove(key);
        self.errors.remove(key);
    }
}

fn tag(d: &DefInfo) -> &'static str {
    match (d.builtin, d.overridden) {
        (_, true) => "overrides built-in",
        (true, false) => "built-in",
        (false, false) => "custom",
    }
}

/// `auto · high`: what the definition's agents start with; nothing for a file that names neither.
fn mode_effort(d: &DefInfo) -> Option<String> {
    let parts: Vec<&str> = d
        .mode
        .map(|m| m.as_flag())
        .into_iter()
        .chain(d.effort.map(|e| e.as_flag()))
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn row_label(d: &DefInfo) -> String {
    let warn = if d.error.is_some() { " ⚠" } else { "" };
    format!("{} {}{warn}", d.name, tag(d))
}

/// One kind's tab: the list on the left, the selected definition's file on the right.
pub(super) fn definitions(
    ui: &mut Ui,
    kind: DefKind,
    defs: Option<&[DefInfo]>,
    st: &mut DefsState,
) -> Option<ManagementAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    let Some(defs) = defs else {
        Frame::new()
            .inner_margin(Margin::symmetric(
                crate::theme::layout::PAGE_X as i8,
                crate::theme::layout::PAGE_Y as i8,
            ))
            .show(ui, |ui| note(ui, "Loading…"));
        return None;
    };
    let listed: Vec<&DefInfo> = defs.iter().filter(|d| d.kind == kind).collect();
    Panel::left(match kind {
        DefKind::Orchestrator => "orchestrators_list",
        DefKind::Specialist => "specialists_list",
        DefKind::Skill => "skills_list",
        DefKind::PrStyle => "pr_styles_list",
    })
    .exact_size(crate::theme::layout::MANAGEMENT_LIST)
    .resizable(false)
    .frame(
        Frame::new()
            .fill(p.surface)
            .inner_margin(Margin::same(crate::theme::management::LIST_MARGIN as i8)),
    )
    .show(ui, |ui| {
        ScrollArea::vertical().show(ui, |ui| {
            if listed.is_empty() {
                note(ui, "None yet.");
            }
            for d in &listed {
                let key = (kind, d.name.clone());
                let selected = st.selected.as_ref() == Some(&key);
                let row = list_row(ui, &row_label(d), selected, |ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.add(
                                Label::new(RichText::new(&d.name).color(p.text_high)).truncate(),
                            );
                            ui.label(RichText::new(tag(d)).size(size::SMALL).color(p.text_low));
                        });
                        if let Some(line) = mode_effort(d) {
                            ui.label(RichText::new(line).size(size::SMALL).color(p.text_low));
                        }
                        if let Some(e) = &d.error {
                            // Painted: the bundled fonts have no ⚠.
                            ui.horizontal_top(|ui| {
                                warn_mark(ui);
                                ui.add(
                                    Label::new(RichText::new(e).size(size::SMALL).color(p.danger))
                                        .wrap(),
                                );
                            });
                        }
                    });
                });
                if row.clicked() {
                    st.selected = Some(key.clone());
                    action = Some(ManagementAction::OpenDefinition(kind, d.name.clone()));
                }
            }
        });
    });
    egui::CentralPanel::default()
        .frame(Frame::new().fill(p.bg).inner_margin(Margin::symmetric(
            crate::theme::layout::PAGE_X as i8,
            crate::theme::layout::PAGE_Y as i8,
        )))
        .show(ui, |ui| {
            let Some(key) = st.selected.clone().filter(|(k, _)| *k == kind) else {
                note(ui, "No definition selected.");
                return;
            };
            let info = listed.iter().find(|d| d.name == key.1).copied();
            ScrollArea::vertical().show(ui, |ui| {
                ui.set_max_width(management::EDITOR_WIDTH);
                if let Some(a) = editor(ui, &key, info, st) {
                    action = Some(a);
                }
            });
        });
    action
}

fn edit(text: TextEdit<'_>) -> TextEdit<'_> {
    text.font(mono(size::CODE))
        .desired_rows(ROWS)
        .desired_width(f32::INFINITY)
}

fn folder(kind: DefKind) -> &'static str {
    match kind {
        DefKind::Orchestrator => "orchestrators",
        DefKind::Specialist => "specialists",
        DefKind::Skill => "skills",
        DefKind::PrStyle => "pr-styles",
    }
}

/// The selected definition's file: read-only with Override for a built-in, else an editor with Save / Revert and,
/// over a built-in, Reset to built-in. The last error stays under it, and the draft with it.
fn editor(
    ui: &mut Ui,
    key: &DefKey,
    info: Option<&DefInfo>,
    st: &mut DefsState,
) -> Option<ManagementAction> {
    let (kind, name) = key;
    let label = caption(ui, &format!("{}/{name}", folder(*kind)));
    ui.add_space(6.0);
    if let Some(e) = info.and_then(|d| d.error.as_deref()) {
        error_line(ui, e);
        ui.add_space(6.0);
    }
    let Some(file) = st.files.get(key) else {
        match st.errors.get(key) {
            Some(e) => error_line(ui, e),
            None => note(ui, "Loading…"),
        }
        return None;
    };
    let Some(draft) = st.drafts.get_mut(key) else {
        // A `&str` buffer: selectable and copyable, never editable.
        let mut text = file.text.as_str();
        ui.add(edit(TextEdit::multiline(&mut text)))
            .labelled_by(label.id);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if button(ui, "Override", ButtonVariant::Primary).clicked() {
                st.drafts.insert(key.clone(), file.text.clone());
            }
            note(
                ui,
                "Built in: Override saves an editable copy as your own file.",
            );
        });
        return None;
    };
    ui.add(edit(TextEdit::multiline(draft)))
        .labelled_by(label.id);
    if let Some(e) = st.errors.get(key) {
        error_line(ui, e);
    }
    ui.add_space(10.0);
    // An override not saved yet is a change in itself: Save writes it, Revert drops it.
    let dirty = *draft != file.text || file.builtin;
    let busy = st.saving.is_some();
    let (save, revert) = save_bar(ui, dirty, false, busy);
    if save {
        st.errors.remove(key);
        return Some(ManagementAction::SaveDefinition(
            *kind,
            name.clone(),
            draft.clone(),
        ));
    }
    if revert {
        if file.builtin {
            st.drafts.remove(key);
        } else {
            *draft = file.text.clone();
        }
        st.errors.remove(key);
    }
    let overrides = info.is_some_and(|d| d.overridden) && !file.builtin;
    if overrides {
        ui.add_space(8.0);
        if button(ui, "Reset to built-in", ButtonVariant::QuietDanger).clicked() {
            return Some(ManagementAction::ResetDefinition(*kind, name.clone()));
        }
    }
    None
}
