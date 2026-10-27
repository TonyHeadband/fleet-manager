//! Management → Products → Internal resources: the product's private notes repo, read-only. Branches, the files of
//! one branch, and one file's text with the commit that last touched it. Every answer is for a product, branch and
//! path the screen still shows; a late answer for anything else is dropped.
use super::personas::{error_line, note};
use super::ManagementAction;
use crate::components::ago;
use crate::theme::{mono, size, tokens};
use crate::widgets::list_row;
use eframe::egui::{
    self, accesskit::Role, CollapsingHeader, Label, RichText, ScrollArea, Ui, WidgetInfo,
};
use fleet_api::InternalCommit;

const FILES_HEIGHT: f32 = 160.0;
const TEXT_HEIGHT: f32 = 320.0;

/// What the section shows for the selected product.
#[derive(Default)]
pub struct InternalState {
    /// The product the rest belongs to.
    product: Option<String>,
    /// The branches were asked for, which happens once, when the section is first opened.
    asked: bool,
    branches: Option<Vec<String>>,
    branch: Option<String>,
    files: Option<Vec<String>>,
    file: Option<String>,
    text: Option<String>,
    last: Option<InternalCommit>,
    error: Option<String>,
}

impl InternalState {
    fn follow(&mut self, product: &str) {
        if self.product.as_deref() != Some(product) {
            *self = InternalState {
                product: Some(product.to_string()),
                ..InternalState::default()
            };
        }
    }

    /// Switches branch: what belonged to the old one goes.
    fn select_branch(&mut self, branch: &str) {
        self.branch = Some(branch.to_string());
        self.files = None;
        self.file = None;
        self.text = None;
        self.last = None;
        self.error = None;
    }

    fn current(&self, product: &str) -> bool {
        self.product.as_deref() == Some(product)
    }

    /// The branches arrived. Returns the branch whose files to fetch: `main` when there is one.
    pub fn branches_done(
        &mut self,
        product: &str,
        result: Result<Vec<String>, String>,
    ) -> Option<String> {
        if !self.current(product) || self.branches.is_some() {
            return None;
        }
        match result {
            Ok(mut branches) => {
                // The daemon's order is by name; main leads.
                branches.sort_by_key(|b| b != "main");
                let first = branches.first().cloned();
                self.branches = Some(branches);
                if let Some(b) = &first {
                    self.select_branch(b);
                }
                first
            }
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    pub fn files_done(&mut self, product: &str, branch: &str, result: Result<Vec<String>, String>) {
        if !self.current(product) || self.branch.as_deref() != Some(branch) {
            return;
        }
        match result {
            Ok(files) => self.files = Some(files),
            Err(e) => self.error = Some(e),
        }
    }

    pub fn file_done(
        &mut self,
        product: &str,
        branch: &str,
        path: &str,
        result: Result<(String, Option<InternalCommit>), String>,
    ) {
        if !self.current(product)
            || self.branch.as_deref() != Some(branch)
            || self.file.as_deref() != Some(path)
        {
            return;
        }
        match result {
            Ok((text, last)) => {
                self.text = Some(text);
                self.last = last;
            }
            Err(e) => self.error = Some(e),
        }
    }
}

pub(super) fn internal_section(
    ui: &mut Ui,
    product: &str,
    st: &mut InternalState,
    now: i64,
) -> Option<ManagementAction> {
    let p = tokens(ui.visuals().dark_mode);
    st.follow(product);
    let mut action = None;
    CollapsingHeader::new(RichText::new("Internal resources").color(p.text_high))
        .id_salt(("internal_resources", product))
        .show(ui, |ui| {
            note(
                ui,
                "Designs, plans and decisions the orchestrators keep for this product. Read only.",
            );
            ui.add_space(4.0);
            if !st.asked {
                st.asked = true;
                action = Some(ManagementAction::OpenInternal(product.to_string()));
            }
            if let Some(e) = &st.error {
                error_line(ui, e);
            }
            let Some(branches) = st.branches.clone() else {
                return;
            };
            if branches.is_empty() {
                note(ui, "No internal resources yet");
                return;
            }
            let mut picked = None;
            let current = st.branch.clone().unwrap_or_default();
            let combo = egui::ComboBox::from_id_salt(("internal_branch", product))
                .selected_text(RichText::new(&current).color(p.text_high))
                .show_ui(ui, |ui| {
                    for b in &branches {
                        if list_row(ui, b, *b == current, |ui| {
                            ui.label(RichText::new(b).color(p.text_high));
                        })
                        .clicked()
                        {
                            picked = Some(b.clone());
                        }
                    }
                });
            combo.response.widget_info(|| {
                WidgetInfo::labeled(Role::ComboBox, ui.is_enabled(), "Internal branch")
            });
            if let Some(b) = picked.filter(|b| *b != current) {
                st.select_branch(&b);
                action = Some(ManagementAction::InternalBranch(product.to_string(), b));
            }
            ui.add_space(6.0);
            let (Some(branch), Some(files)) = (st.branch.clone(), st.files.clone()) else {
                return;
            };
            if files.is_empty() {
                note(ui, "No files on this branch");
                return;
            }
            ScrollArea::vertical()
                .id_salt(("internal_files", product))
                .max_height(FILES_HEIGHT)
                .show(ui, |ui| {
                    for f in &files {
                        let on = st.file.as_deref() == Some(f.as_str());
                        let row = list_row(ui, f, on, |ui| {
                            ui.add(
                                Label::new(
                                    RichText::new(f).font(mono(size::META)).color(p.text_high),
                                )
                                .truncate(),
                            );
                        });
                        if row.clicked() && !on {
                            st.file = Some(f.clone());
                            st.text = None;
                            st.last = None;
                            st.error = None;
                            action = Some(ManagementAction::InternalFile(
                                product.to_string(),
                                branch.clone(),
                                f.clone(),
                            ));
                        }
                    }
                });
            if let Some(text) = &st.text {
                ui.add_space(6.0);
                if let Some(c) = &st.last {
                    note(
                        ui,
                        &format!("{} · {} · {}", c.subject, c.author, ago(now, c.at)),
                    );
                }
                ScrollArea::both()
                    .id_salt(("internal_text", product))
                    .max_height(TEXT_HEIGHT)
                    .show(ui, |ui| {
                        ui.add(
                            Label::new(
                                RichText::new(text)
                                    .font(mono(size::META))
                                    .color(p.text_high),
                            )
                            .selectable(true),
                        );
                    });
            }
        });
    action
}
