//! Management → Products: the products on the left, the selected one's form on the right. Saving sends what
//! differs from the product as the daemon has it; the daemon's refusals show next to what caused them.
use super::personas::{error_line, note};
use super::ManagementAction;
use crate::components::{
    caption, product_fields, roster_chips, usable, workflow_line, ProductForm,
};
use crate::theme::{mono, sans, size, tokens, Weight};
use crate::widgets::{button, list_row, ButtonVariant};
use eframe::egui::{self, Align, Frame, Label, Layout, Margin, Panel, RichText, ScrollArea, Ui};
use fleet_api::{DefInfo, DefKind, Persona, Product, RepoFleet, RepoInfo};
use std::collections::BTreeMap;

const LIST_WIDTH: f32 = 260.0;
const DETAIL_WIDTH: f32 = 760.0;

/// How a save went, per part: the product's own fields, then each repo that was added, changed or removed. A part
/// that failed does not stop the others.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProductEdit {
    pub id: String,
    pub product_error: Option<String>,
    /// Repo name and the daemon's reason.
    pub repo_errors: Vec<(String, String)>,
}

/// A repo row's roster as being edited. It keeps a name that no longer loads, so the row can show it struck; that
/// one is never sent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FleetDraft {
    pub roster: Vec<String>,
}

impl From<&RepoInfo> for FleetDraft {
    fn from(r: &RepoInfo) -> Self {
        FleetDraft {
            roster: r.roster.clone(),
        }
    }
}

impl FleetDraft {
    /// What Save sends: usable specialists only.
    fn to_fleet(&self, specialists: &[&str]) -> RepoFleet {
        RepoFleet {
            roster: self
                .roster
                .iter()
                .filter(|n| specialists.contains(&n.as_str()))
                .cloned()
                .collect(),
        }
    }
}

/// The tab's state: the selected product, its form, and what the daemon last refused.
#[derive(Default)]
pub struct ProductsState {
    pub selected: Option<String>,
    pub form: ProductForm,
    /// The product as the daemon has it, as a form: what `form` is compared with.
    saved: ProductForm,
    /// The product `form` was made for.
    for_product: Option<String>,
    /// A save is in flight, or its answer is awaiting the refreshed list.
    pub busy: bool,
    /// The daemon's reason for refusing the product's own fields.
    pub error: Option<String>,
    /// By repo name.
    pub repo_errors: BTreeMap<String, String>,
    /// Delete product was clicked once and waits for Confirm delete.
    pub confirm_delete: bool,
    pub delete_error: Option<String>,
    /// Each repo's roster as being edited, by repo name; made from the repo when its row is first drawn.
    pub rosters: BTreeMap<String, FleetDraft>,
    /// The repo whose roster save is in flight.
    pub roster_saving: Option<String>,
    /// The repo whose last roster save was refused, and why.
    pub roster_error: Option<(String, String)>,
}

impl ProductsState {
    /// Keeps the selection on a listed product and the form on that product. A form with unsaved edits is kept when
    /// the daemon's copy changes; a clean one follows it.
    pub fn sync(&mut self, products: &[Product]) {
        if !self
            .selected
            .as_ref()
            .is_some_and(|id| products.iter().any(|p| &p.id == id))
        {
            self.selected = products.first().map(|p| p.id.clone());
        }
        let Some(p) = self
            .selected
            .as_ref()
            .and_then(|id| products.iter().find(|p| &p.id == id))
        else {
            self.for_product = None;
            return;
        };
        let fresh = ProductForm::from(p);
        if self.for_product.as_deref() != Some(p.id.as_str()) {
            *self = ProductsState {
                selected: self.selected.take(),
                form: fresh.clone(),
                saved: fresh,
                for_product: Some(p.id.clone()),
                ..ProductsState::default()
            };
        } else if self.saved != fresh {
            if self.form.diff(&p.id, &self.saved).is_empty() {
                self.form = fresh.clone();
            }
            self.saved = fresh;
        }
    }

    fn dirty(&self) -> bool {
        self.for_product
            .as_deref()
            .is_some_and(|id| !self.form.diff(id, &self.saved).is_empty())
    }

    /// A save was answered. Whatever succeeded is in the list that is about to be fetched.
    pub fn edit_done(&mut self, e: ProductEdit) {
        if self.for_product.as_deref() != Some(e.id.as_str()) {
            return;
        }
        let failed = e.product_error.is_some() || !e.repo_errors.is_empty();
        self.error = e.product_error;
        self.repo_errors = e.repo_errors.into_iter().collect();
        if failed {
            self.busy = false;
        }
    }

    /// The list was fetched (or failed to be): a save no longer waits for it.
    pub fn refreshed(&mut self) {
        self.busy = false;
    }

    /// A roster save of `r` landed: its draft takes the stored values.
    pub fn roster_saved(&mut self, r: &RepoInfo) {
        self.rosters.insert(r.name.clone(), FleetDraft::from(r));
        self.roster_error = None;
        self.roster_saving = None;
    }
}

pub(super) fn products(
    ui: &mut Ui,
    products: Option<&[Product]>,
    personas: &[Persona],
    defs: Option<&[DefInfo]>,
    st: &mut ProductsState,
) -> Option<ManagementAction> {
    let p = tokens(ui.visuals().dark_mode);
    let Some(products) = products else {
        Frame::new()
            .inner_margin(Margin::symmetric(22, 18))
            .show(ui, |ui| {
                ui.label(RichText::new("Loading…").color(p.text_low));
            });
        return None;
    };
    st.sync(products);
    let mut action = None;
    Panel::left("products_list")
        .exact_size(LIST_WIDTH)
        .resizable(false)
        .frame(Frame::new().fill(p.surface).inner_margin(Margin::same(12)))
        .show(ui, |ui| {
            Panel::bottom("products_new")
                .frame(Frame::new().fill(p.surface).inner_margin(Margin {
                    top: 8,
                    ..Margin::ZERO
                }))
                .show(ui, |ui| {
                    ui.with_layout(Layout::top_down_justified(Align::Center), |ui| {
                        if button(ui, "+ New product", ButtonVariant::Default).clicked() {
                            action = Some(ManagementAction::NewProduct);
                        }
                    });
                });
            egui::CentralPanel::default()
                .frame(Frame::new().fill(p.surface))
                .show(ui, |ui| {
                    ScrollArea::vertical().show(ui, |ui| {
                        for product in products {
                            let on = st.selected.as_deref() == Some(product.id.as_str());
                            let row = list_row(ui, &product.name, on, |ui| {
                                ui.add(
                                    Label::new(RichText::new(&product.name).color(p.text_high))
                                        .truncate(),
                                );
                            });
                            if row.clicked() && !on {
                                st.selected = Some(product.id.clone());
                            }
                        }
                    });
                });
        });
    egui::CentralPanel::default()
        .frame(
            Frame::new()
                .fill(p.bg)
                .inner_margin(Margin::symmetric(22, 18)),
        )
        .show(ui, |ui| {
            let Some(product) = st
                .selected
                .as_ref()
                .and_then(|id| products.iter().find(|p| &p.id == id))
            else {
                note(ui, "No product yet. Create one with New product.");
                return;
            };
            ScrollArea::vertical().show(ui, |ui| {
                ui.set_max_width(DETAIL_WIDTH);
                if let Some(a) = detail(ui, product, personas, defs, st) {
                    action = Some(a);
                }
            });
        });
    action
}

fn detail(
    ui: &mut Ui,
    product: &Product,
    personas: &[Persona],
    defs: Option<&[DefInfo]>,
    st: &mut ProductsState,
) -> Option<ManagementAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    ui.label(
        RichText::new(&product.name)
            .font(sans(ui, size::TITLE, Weight::Medium))
            .color(p.text_high),
    );
    ui.add_space(8.0);

    let dirty = st.dirty();
    let ProductsState {
        form,
        repo_errors,
        rosters,
        roster_saving,
        roster_error,
        ..
    } = &mut *st;
    let mut roster_action = None;
    product_fields(ui, form, personas, &mut |ui, _, draft| {
        let name = draft.name.trim();
        if let Some(e) = repo_errors.get(name) {
            error_line(ui, e);
        }
        let Some(repo) = draft
            .existing
            .then(|| product.repos.iter().find(|r| r.name == name))
            .flatten()
        else {
            return;
        };
        let saving = roster_saving.as_deref() == Some(name);
        let error = roster_error
            .as_ref()
            .filter(|(r, _)| r == name)
            .map(|(_, e)| e.as_str());
        let draft = rosters
            .entry(repo.name.clone())
            .or_insert_with(|| repo.into());
        if let Some(fleet) = roster_row(ui, repo, defs, draft, saving, error) {
            roster_action = Some((repo.name.clone(), fleet));
        }
    });
    if let Some((repo, fleet)) = roster_action {
        action = Some(ManagementAction::SaveRepoFleet {
            product: product.id.clone(),
            repo,
            fleet,
        });
    }
    ui.add_space(12.0);

    let can_save = st.form.can_save() && dirty && !st.busy;
    ui.horizontal(|ui| {
        let save = ui
            .add_enabled_ui(can_save, |ui| button(ui, "Save", ButtonVariant::Primary))
            .inner;
        if save.clicked() && can_save {
            action = Some(ManagementAction::SaveProduct(
                st.form.diff(&product.id, &st.saved),
            ));
        }
        let revert = ui
            .add_enabled_ui(dirty && !st.busy, |ui| {
                button(ui, "Revert", ButtonVariant::Default)
            })
            .inner;
        if revert.clicked() {
            st.form = st.saved.clone();
            st.error = None;
            st.repo_errors.clear();
        }
    });
    if let Some(e) = &st.error {
        error_line(ui, e);
    }
    ui.add_space(20.0);

    if st.confirm_delete {
        ui.label(
            RichText::new(format!("Delete {}? This cannot be undone.", product.name))
                .color(p.text_high),
        );
        ui.horizontal(|ui| {
            if button(ui, "Confirm delete", ButtonVariant::QuietDanger).clicked() {
                action = Some(ManagementAction::DeleteProduct(product.id.clone()));
            }
            if button(ui, "Cancel", ButtonVariant::Default).clicked() {
                st.confirm_delete = false;
            }
        });
    } else if button(ui, "Delete product", ButtonVariant::QuietDanger).clicked() {
        st.confirm_delete = true;
        st.delete_error = None;
    }
    if let Some(e) = &st.delete_error {
        error_line(ui, e);
    }
    action
}

/// A stored repo's specialists, the repo's workflow, and Save roster. Returns what to save when it is clicked.
/// Without the definitions the roster cannot be checked, so Save waits.
fn roster_row(
    ui: &mut Ui,
    r: &RepoInfo,
    defs: Option<&[DefInfo]>,
    draft: &mut FleetDraft,
    saving: bool,
    error: Option<&str>,
) -> Option<RepoFleet> {
    let p = tokens(ui.visuals().dark_mode);
    let mut save = None;
    ui.indent(("roster", &r.name), |ui| {
        let specialists = defs.map(|d| usable(d, DefKind::Specialist));
        caption(ui, "Specialists");
        ui.add_space(4.0);
        match &specialists {
            Some(names) => {
                let stale: Vec<String> = draft
                    .roster
                    .iter()
                    .filter(|n| !names.contains(&n.as_str()))
                    .cloned()
                    .collect();
                let stale: Vec<&str> = stale.iter().map(String::as_str).collect();
                roster_chips(ui, names, &mut draft.roster, &stale);
            }
            None => note(ui, "The specialists have not loaded."),
        }
        ui.add_space(6.0);
        let (text, blocks) = workflow_line(&r.name, &r.workflow);
        if blocks {
            error_line(ui, &text);
        } else {
            ui.add(Label::new(RichText::new(text).font(mono(size::META)).color(p.text_low)).wrap());
        }
        ui.add_space(6.0);
        let fleet = specialists.as_deref().map(|names| draft.to_fleet(names));
        let saved = RepoFleet {
            roster: r.roster.clone(),
        };
        let enabled = fleet.as_ref().is_some_and(|f| *f != saved) && !saving;
        if ui
            .add_enabled_ui(enabled, |ui| {
                button(ui, "Save roster", ButtonVariant::Default)
            })
            .inner
            .clicked()
            && enabled
        {
            save = fleet;
        }
        if let Some(e) = error {
            error_line(ui, e);
        }
        ui.add_space(6.0);
    });
    save
}
