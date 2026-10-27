//! Management → Products: the products on the left, the selected one's form on the right. Saving sends what
//! differs from the product as the daemon has it; the daemon's refusals show next to what caused them.
use super::internal::{internal_section, InternalState};
use super::personas::{error_line, note};
use super::ManagementAction;
use crate::components::{
    caption, product_fields, roster_chips, usable, workflow_line, ProductForm,
};
use crate::theme::{mono, sans, size, tokens, Weight};
use crate::widgets::{button, list_row, ButtonVariant};
use eframe::egui::{
    self, accesskit::Role, Align, Frame, Label, Layout, Margin, Panel, RichText, ScrollArea,
    TextEdit, Ui, WidgetInfo,
};
use fleet_api::{
    DefInfo, DefKind, EffectiveSettings, Persona, Product, ProductOverride, RepoFleet, RepoInfo,
    SettingSource,
};
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

/// The override sections, and the keys of `limits`: what the daemon accepts besides the definition names that
/// `models` takes.
const SECTIONS: [&str; 2] = ["models", "limits"];
const LIMIT_KEYS: [&str; 3] = [
    "max_breakdown_versions",
    "max_steps",
    "max_parallel_specialists",
];

/// An override row as being edited. `error` is the daemon's reason an entry it stores no longer applies; it
/// goes when the row is edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverrideDraft {
    pub section: String,
    pub key: String,
    pub value: String,
    pub error: Option<String>,
}

impl Default for OverrideDraft {
    fn default() -> Self {
        OverrideDraft {
            section: SECTIONS[0].into(),
            key: String::new(),
            value: String::new(),
            error: None,
        }
    }
}

impl From<&ProductOverride> for OverrideDraft {
    fn from(o: &ProductOverride) -> Self {
        OverrideDraft {
            section: o.section.clone(),
            key: o.key.clone(),
            value: o.value.clone(),
            error: o.error.clone(),
        }
    }
}

impl OverrideDraft {
    fn ready(&self) -> bool {
        !self.key.trim().is_empty() && !self.value.trim().is_empty()
    }

    fn to_override(&self) -> ProductOverride {
        ProductOverride {
            section: self.section.clone(),
            key: self.key.trim().to_string(),
            value: self.value.trim().to_string(),
            error: None,
        }
    }

    /// The same setting and value, whatever the daemon said about it.
    fn same_as(&self, other: &OverrideDraft) -> bool {
        (&self.section, &self.key, &self.value) == (&other.section, &other.key, &other.value)
    }
}

fn drafts(p: &Product) -> Vec<OverrideDraft> {
    p.overrides.iter().map(OverrideDraft::from).collect()
}

fn same_rows(a: &[OverrideDraft], b: &[OverrideDraft]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.same_as(y))
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
    /// The overrides as being edited, and as the daemon has them.
    pub overrides: Vec<OverrideDraft>,
    saved_overrides: Vec<OverrideDraft>,
    /// An overrides save is in flight.
    pub overrides_busy: bool,
    /// The daemon's reason for refusing the whole list.
    pub overrides_error: Option<String>,
    pub internal: InternalState,
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
                overrides: drafts(p),
                saved_overrides: drafts(p),
                for_product: Some(p.id.clone()),
                ..ProductsState::default()
            };
        } else if self.saved != fresh {
            if self.form.diff(&p.id, &self.saved).is_empty() {
                self.form = fresh.clone();
            }
            self.saved = fresh;
        }
        // The same rule as the form's: edits are kept when the daemon's copy changes, a clean list follows it.
        let fresh = drafts(p);
        if same_rows(&self.overrides, &self.saved_overrides) {
            self.overrides = fresh.clone();
        }
        self.saved_overrides = fresh;
    }

    fn overrides_dirty(&self) -> bool {
        !same_rows(&self.overrides, &self.saved_overrides)
    }

    /// An overrides save was answered; a refusal keeps the rows and shows the reason. On success both lists take the
    /// stored rows: the daemon sorts and trims them, so the edited order would otherwise stay dirty.
    pub fn overrides_done(&mut self, id: &str, result: Result<&Product, String>) {
        if self.for_product.as_deref() != Some(id) {
            return;
        }
        self.overrides_busy = false;
        match result {
            Ok(saved) => {
                self.overrides = drafts(saved);
                self.saved_overrides = drafts(saved);
                self.overrides_error = None;
            }
            Err(e) => self.overrides_error = Some(e),
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
        // A removal the daemon refused: the row comes back, carrying the reason.
        for stored in &self.saved.repos {
            if self.repo_errors.contains_key(&stored.name)
                && !self.form.repos.iter().any(|r| r.name.trim() == stored.name)
            {
                self.form.repos.push(stored.clone());
            }
        }
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
    effective: Option<&EffectiveSettings>,
    now: i64,
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
                if let Some(a) = detail(ui, product, personas, defs, effective, now, st) {
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
    effective: Option<&EffectiveSettings>,
    now: i64,
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

    if let Some(overrides) = overrides_section(ui, st, defs, effective) {
        action = Some(ManagementAction::SaveOverrides {
            product: product.id.clone(),
            overrides,
        });
    }
    ui.add_space(20.0);

    if let Some(a) = internal_section(ui, &product.id, &mut st.internal, now) {
        action = Some(a);
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

/// The product's overrides: a row per setting, then Add override and Save overrides. Returns the whole list to
/// send when Save overrides is clicked: the daemon replaces what it stores with it.
fn overrides_section(
    ui: &mut Ui,
    st: &mut ProductsState,
    defs: Option<&[DefInfo]>,
    effective: Option<&EffectiveSettings>,
) -> Option<Vec<ProductOverride>> {
    let p = tokens(ui.visuals().dark_mode);
    caption(ui, "Overrides");
    ui.add_space(2.0);
    note(
        ui,
        "Settings this product sets for itself. Every other setting follows the global one.",
    );
    ui.add_space(4.0);
    let models: Vec<&str> = defs.map_or_else(Vec::new, |d| {
        let mut names = usable(d, DefKind::Orchestrator);
        names.extend(usable(d, DefKind::Specialist));
        names
    });
    let mut remove = None;
    for (i, row) in st.overrides.iter_mut().enumerate() {
        ui.push_id(("override", i), |ui| {
            ui.horizontal(|ui| {
                let before = row.clone();
                choice(ui, "section", i, &SECTIONS, &mut row.section, 90.0);
                if row.section != before.section {
                    // The old key means nothing in the other section.
                    row.key.clear();
                }
                let keys: Vec<&str> = if row.section == "limits" {
                    LIMIT_KEYS.to_vec()
                } else {
                    models.clone()
                };
                choice(ui, "key", i, &keys, &mut row.key, 170.0);
                let hint = if row.section == "limits" {
                    "number"
                } else {
                    "model"
                };
                let edit = ui.add(
                    TextEdit::singleline(&mut row.value)
                        .hint_text(hint)
                        .desired_width(110.0),
                );
                edit.widget_info(|| {
                    WidgetInfo::labeled(Role::TextInput, true, format!("Override value {}", i + 1))
                });
                if let Some(text) = replaces(effective, row) {
                    ui.label(RichText::new(text).font(mono(size::META)).color(p.text_low));
                }
                let gone = button(ui, "Remove", ButtonVariant::Ghost);
                gone.widget_info(|| {
                    WidgetInfo::labeled(Role::Button, true, format!("Remove override {}", i + 1))
                });
                if gone.clicked() {
                    remove = Some(i);
                }
                if !row.same_as(&before) {
                    row.error = None;
                }
            });
            if let Some(e) = &row.error {
                error_line(ui, e);
            }
        });
    }
    if let Some(i) = remove {
        st.overrides.remove(i);
    }
    let mut save = None;
    ui.horizontal(|ui| {
        if button(ui, "Add override", ButtonVariant::Default).clicked() {
            st.overrides.push(OverrideDraft::default());
        }
        let can_save = st.overrides_dirty()
            && !st.overrides_busy
            && st.overrides.iter().all(OverrideDraft::ready);
        let clicked = ui
            .add_enabled_ui(can_save, |ui| {
                button(ui, "Save overrides", ButtonVariant::Primary)
            })
            .inner
            .clicked();
        if clicked && can_save {
            save = Some(
                st.overrides
                    .iter()
                    .map(OverrideDraft::to_override)
                    .collect(),
            );
        }
    });
    if let Some(e) = &st.overrides_error {
        error_line(ui, e);
    }
    save
}

/// A dropdown over `options` that sets `current`. A `current` the options do not list (a definition that is gone)
/// is shown as it is.
fn choice(ui: &mut Ui, what: &str, row: usize, options: &[&str], current: &mut String, width: f32) {
    let p = tokens(ui.visuals().dark_mode);
    let shown = if current.is_empty() {
        what
    } else {
        current.as_str()
    };
    let combo = egui::ComboBox::from_id_salt(("override_choice", what, row))
        .selected_text(RichText::new(shown).color(p.text_high))
        .width(width)
        .show_ui(ui, |ui| {
            for option in options {
                let on = current == option;
                if list_row(ui, option, on, |ui| {
                    ui.label(RichText::new(*option).color(p.text_high));
                })
                .clicked()
                {
                    *current = option.to_string();
                }
            }
        });
    let name = format!("Override {what} {}", row + 1);
    combo
        .response
        .widget_info(|| WidgetInfo::labeled(Role::ComboBox, ui.is_enabled(), name.clone()));
}

/// What the row's setting is without the override, and where that value comes from. Nothing until the daemon has
/// answered, or for a setting it does not list.
fn replaces(effective: Option<&EffectiveSettings>, row: &OverrideDraft) -> Option<String> {
    let v = effective?
        .values
        .iter()
        .find(|v| v.section == row.section && v.key == row.key)?;
    Some(match &v.value {
        Some(value) => format!("replaces {value} ({})", source_word(v.source)),
        None => "replaces persona default".to_string(),
    })
}

fn source_word(s: SettingSource) -> &'static str {
    match s {
        SettingSource::Product => "product",
        SettingSource::Global => "global",
        SettingSource::Definition => "definition",
        SettingSource::Default => "default",
    }
}
