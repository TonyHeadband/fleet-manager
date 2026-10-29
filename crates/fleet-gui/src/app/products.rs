//! The app's side of products: the list, the active product the drawer, board and New task act on, and New product.
use super::{AppResult, FleetApp, PersonasLoad};
use crate::components::{
    modal_frame, product_form, ProductForm, ProductFormAction, ProductSave, ReleaseFilter,
};
use crate::views::management::{ManagementAction, ProductEdit};
use crate::views::releases::{self, ReleasesAction};
use eframe::egui::{self, RichText, TextStyle};
use fleet_api::{
    NewProduct, NewRelease, Product, ProductList, ProductOverride, ReleaseUpdate, RepoInfo,
    RepoUpsert,
};
use fleet_client::config::SettingsWriter;
use fleet_client::http::{FleetClient, FleetError};
use fleet_client::schema::usable_form;

/// A product call the GUI made, recorded for tests.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub enum ProductCall {
    /// The product, then the repos added to it once it exists.
    Create(NewProduct, Vec<RepoUpsert>),
    /// An edited product (Management → Products): its changes and its repos'.
    Save(ProductSave),
    Delete(String),
    /// The product, then the whole override list saved.
    Overrides(String, Vec<ProductOverride>),
    /// Product id, the hidden specialists saved.
    Hidden(String, Vec<String>),
    /// Product id: its internal branches were asked for.
    InternalBranches(String),
    /// Product id, branch: the files of the branch.
    InternalFiles(String, String),
    /// Product id, branch, path: the file's text and last commit.
    InternalFile(String, String, String),
}

/// A release call the Releases view made, recorded for tests.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseCall {
    /// The product, then the release.
    Create(String, NewRelease),
    Update(i64, ReleaseUpdate),
    Delete(i64),
}

/// A task call the Tasks view made, recorded for tests.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskCall {
    /// The task, then the release it moved to (None: the backlog).
    Release(String, Option<i64>),
    Done(String),
}

/// New product on screen.
#[derive(Debug, Default)]
pub struct ProductFormState {
    pub form: ProductForm,
    pub busy: bool,
    pub error: Option<String>,
}

pub(super) const PICK_PRODUCT: &str = "Pick a product";

/// `product`'s repos; none while no product is selected.
pub(super) fn repos_in(
    c: &FleetClient,
    product: Option<&str>,
) -> Result<Vec<RepoInfo>, FleetError> {
    match product {
        Some(p) => c.repos_for(p),
        None => Ok(vec![]),
    }
}

impl FleetApp {
    /// The product the board, the Tasks view and New task act on; None when none is selected or the saved one is
    /// no longer listed. Before the list loads, the saved one is taken on trust.
    pub fn active_product(&self) -> Option<&str> {
        let id = self.active_product.as_deref()?;
        self.products
            .as_ref()
            .is_none_or(|l| l.products.iter().any(|p| p.id == id))
            .then_some(id)
    }

    /// Starts with `id` as the active product, unsaved. For tests and embedding: the saved choice is `with_settings`'.
    #[doc(hidden)]
    pub fn with_product(mut self, id: &str) -> FleetApp {
        self.active_product = Some(id.to_string());
        self
    }

    #[doc(hidden)]
    pub fn task_calls(&self) -> &[TaskCall] {
        &self.task_calls
    }

    #[doc(hidden)]
    pub fn release_calls(&self) -> &[ReleaseCall] {
        &self.release_calls
    }

    #[doc(hidden)]
    pub fn product_calls(&self) -> &[ProductCall] {
        &self.product_calls
    }

    pub(super) fn listed_products(&self) -> &[Product] {
        self.products.as_ref().map_or(&[], |l| &l.products)
    }

    /// Why New task cannot run for want of a product. An app that never asked for the list (tests, embedding)
    /// blocks nothing.
    pub(super) fn product_blocker(&self) -> Option<&'static str> {
        match (&self.products, self.products_load) {
            (Some(l), _) if l.products.is_empty() => Some("Create a product first"),
            (Some(_), _) => self.active_product().is_none().then_some(PICK_PRODUCT),
            (None, PersonasLoad::Idle) => None,
            (None, PersonasLoad::Loading) => Some("Loading products…"),
            // A saved choice is enough to start a task; only a missing one needs the list.
            (None, PersonasLoad::Failed) => self
                .active_product
                .is_none()
                .then_some("Could not load the products; retrying"),
        }
    }

    /// A product as a save stored it: the list carries it at once; the fetch only confirms it.
    fn saved_product(&mut self, saved: Product) {
        if let Some(p) = self
            .products
            .as_mut()
            .and_then(|l| l.products.iter_mut().find(|p| p.id == saved.id))
        {
            *p = saved;
        }
        self.fetch_products();
    }

    pub(super) fn fetch_products(&mut self) {
        if !self.calls {
            return;
        }
        if self.products_load == PersonasLoad::Loading {
            // The answer on its way may predate whatever made us ask.
            self.products_stale = true;
            return;
        }
        self.products_load = PersonasLoad::Loading;
        self.background(|c| {
            AppResult::Products(
                c.products()
                    .map(|products| ProductList { products })
                    .map_err(|e| e.message),
            )
        });
    }

    /// The settings as they are without any product's overrides, which each override row sets its value against.
    fn fetch_effective(&mut self) {
        self.background(|c| {
            AppResult::Effective(c.effective_settings(None).map_err(|e| e.message))
        });
    }

    /// A failed first load is asked again on the tasks poll.
    pub(super) fn retry_products(&mut self) {
        if self.products.is_none() && self.products_load == PersonasLoad::Failed {
            self.fetch_products();
        }
    }

    /// Selects and saves the active product, like the persona switcher.
    pub(super) fn set_active_product(&mut self, id: Option<String>) {
        if self.active_product == id {
            return;
        }
        self.active_product = id.clone();
        self.release_filter = ReleaseFilter::All;
        // The repos on offer are the product's.
        self.fetch_spawn_repos();
        let Some(path) = &self.settings_path else {
            return;
        };
        let (tx, ctx) = (self.results_tx.clone(), self.ctx.clone());
        SettingsWriter::shared().update(
            path,
            move |s| s.active_product = id.clone(),
            move |saved| {
                let _ = tx.send(AppResult::SettingsSaved(
                    saved
                        .map(drop)
                        .map_err(|e| format!("Could not save the active product: {e}")),
                ));
                ctx.request_repaint();
            },
        );
    }

    pub(super) fn releases_view(&mut self, ui: &mut egui::Ui) {
        let live = self.live_tasks();
        let active = self.active_product().map(str::to_string);
        let product = self
            .products
            .as_ref()
            .and_then(|l| l.products.iter().find(|p| Some(&p.id) == active.as_ref()));
        let tasks = self.tasks.list.as_ref().map_or(&[][..], |l| &l.tasks[..]);
        let (Some(create), Some(edit)) = (
            usable_form(self.schema.as_ref(), "release_create"),
            usable_form(self.schema.as_ref(), "release_edit"),
        ) else {
            return;
        };
        let action = releases::main(
            ui,
            &mut self.releases,
            active.as_deref(),
            product.map_or(&[], |p| &p.releases),
            tasks,
            &live,
            (self.clock)(),
            releases::ReleaseForms {
                create: &create,
                edit: &edit,
            },
        );
        let Some(action) = action else {
            return;
        };
        match action {
            ReleasesAction::Create(new) => {
                let Some(product) = active else {
                    return;
                };
                self.mark_release_form_busy();
                self.release_calls
                    .push(ReleaseCall::Create(product.clone(), new.clone()));
                self.background(move |c| {
                    AppResult::ReleaseSaved(c.create_release(&product, &new).map_err(|e| e.message))
                });
            }
            ReleasesAction::Update(id, update) => {
                self.mark_release_form_busy();
                self.release_calls
                    .push(ReleaseCall::Update(id, update.clone()));
                self.background(move |c| {
                    AppResult::ReleaseSaved(c.update_release(id, &update).map_err(|e| e.message))
                });
            }
            ReleasesAction::SetReleased(id, released) => {
                let update = ReleaseUpdate {
                    released: Some(released),
                    ..ReleaseUpdate::default()
                };
                self.release_calls
                    .push(ReleaseCall::Update(id, update.clone()));
                self.background(move |c| {
                    AppResult::ReleaseChanged(
                        c.update_release(id, &update)
                            .map(drop)
                            .map_err(|e| e.message),
                    )
                });
            }
            ReleasesAction::Delete(id) => {
                self.release_calls.push(ReleaseCall::Delete(id));
                self.background(move |c| {
                    AppResult::ReleaseChanged(c.delete_release(id).map_err(|e| e.message))
                });
            }
            ReleasesAction::Move(id, release) => {
                self.task_calls.push(TaskCall::Release(id.clone(), release));
                self.background(move |c| {
                    AppResult::TaskChanged(
                        id.clone(),
                        c.set_task_release(&id, release).map_err(|e| e.message),
                    )
                });
            }
        }
    }

    fn mark_release_form_busy(&mut self) {
        if let Some(form) = self.releases.form.as_mut() {
            form.busy = true;
            form.error = None;
        }
    }

    pub(super) fn open_product_form(&mut self) {
        self.product_form = Some(ProductFormState::default());
    }

    pub(super) fn handle_product_result(&mut self, r: AppResult) {
        match r {
            AppResult::Products(Ok(list)) => {
                self.products_tab.refreshed();
                self.products_load = PersonasLoad::Idle;
                self.products_error = None;
                self.products = Some(list);
                self.fetch_effective();
                if std::mem::take(&mut self.products_stale) {
                    self.fetch_products();
                }
            }
            AppResult::Products(Err(e)) => {
                self.products_tab.refreshed();
                self.products_load = PersonasLoad::Failed;
                self.products_stale = false;
                // Retried on every poll: say it once, and again only if the reason changes.
                if self.products_error.as_ref() != Some(&e) {
                    tracing::error!(notify = "error", "Could not load the products: {e}");
                    self.products_error = Some(e);
                }
            }
            AppResult::ProductSaved(Ok(p)) => {
                self.product_form = None;
                if let Some(list) = self.products.as_mut() {
                    if !list.products.iter().any(|l| l.id == p.id) {
                        list.products.push(p.clone());
                    }
                }
                self.set_active_product(Some(p.id));
                self.fetch_products();
            }
            AppResult::ProductEdited(edit) => {
                self.products_tab.edit_done(edit);
                self.fetch_products();
                // The board's and New task's repo lists carry the repos too.
                self.refresh_repos();
            }
            AppResult::OverridesSaved(id, result) => match result {
                Ok(saved) => {
                    self.products_tab.overrides_done(&id, Ok(&saved));
                    self.saved_product(saved);
                }
                Err(e) => self.products_tab.overrides_done(&id, Err(e)),
            },
            AppResult::HiddenSaved(id, result) => match result {
                Ok(saved) => {
                    self.products_tab.hidden_done(&id, Ok(&saved));
                    self.saved_product(saved);
                }
                Err(e) => self.products_tab.hidden_done(&id, Err(e)),
            },
            AppResult::InternalBranches(product, result) => {
                if let Some(branch) = self.products_tab.internal.branches_done(&product, result) {
                    self.fetch_internal_files(product, branch);
                }
            }
            AppResult::InternalFiles(product, branch, result) => {
                if let Some(path) = self
                    .products_tab
                    .internal
                    .files_done(&product, &branch, result)
                {
                    self.fetch_internal_file(product, branch, path);
                }
            }
            AppResult::InternalFile(product, branch, path, result) => self
                .products_tab
                .internal
                .file_done(&product, &branch, &path, result),
            // A failure leaves the last answer; the rows then show no replaced value.
            AppResult::Effective(Ok(settings)) => self.effective = Some(settings),
            AppResult::Effective(Err(_)) => {}
            AppResult::ProductDeleted(id, result) => match result {
                Ok(()) => {
                    if self.active_product.as_deref() == Some(id.as_str()) {
                        self.set_active_product(None);
                    }
                    self.fetch_products();
                }
                Err(e) => {
                    self.products_tab.confirm_delete = false;
                    self.products_tab.delete_error = Some(e);
                }
            },
            AppResult::ReleaseSaved(Ok(_)) => {
                self.releases.form = None;
                self.fetch_products();
            }
            AppResult::ReleaseSaved(Err(e)) => {
                if let Some(form) = self.releases.form.as_mut() {
                    form.busy = false;
                    form.error = Some(e);
                }
            }
            AppResult::ReleaseChanged(result) => {
                if let Err(e) = result {
                    tracing::error!(notify = "error", "Could not update the release: {e}");
                }
                self.fetch_products();
            }
            AppResult::ProductSaved(Err(e)) => match self.product_form.as_mut() {
                Some(st) => {
                    st.busy = false;
                    st.error = Some(e);
                }
                None => tracing::error!(notify = "error", "Could not create the product: {e}"),
            },
            _ => {}
        }
    }

    /// The New product modal, drawn over everything else.
    pub(super) fn product_form_modal(&mut self, ctx: &egui::Context) {
        let Some(st) = self.product_form.as_mut() else {
            return;
        };
        let p = crate::theme::tokens(ctx.global_style().visuals.dark_mode);
        let personas = self.personas.as_ref().map_or(&[][..], |l| &l.personas[..]);
        let modal = egui::Modal::new(egui::Id::unique("product_form"))
            .frame(modal_frame(ctx))
            .show(ctx, |ui| {
                ui.set_max_width(560.0);
                ui.label(
                    RichText::new("New product")
                        .text_style(TextStyle::Heading)
                        .color(p.text_high),
                );
                ui.add_space(8.0);
                let action = product_form(ui, &mut st.form, personas);
                if let Some(e) = &st.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(e).color(p.danger));
                }
                action
            });
        let action = match modal.inner {
            None if modal.should_close() => Some(ProductFormAction::Cancel),
            a => a,
        };
        match action {
            Some(ProductFormAction::Cancel) => self.product_form = None,
            Some(ProductFormAction::Save) => self.save_new_product(),
            None => {}
        }
    }

    /// Creates the product, then adds each repo. A repo that fails after the product exists is reported and the
    /// product is still selected: a retry of the whole form would only meet "name taken".
    fn save_new_product(&mut self) {
        let Some(st) = self.product_form.as_mut().filter(|s| !s.busy) else {
            return;
        };
        if !st.form.can_save() {
            return;
        }
        st.busy = true;
        st.error = None;
        let (new, repos) = (st.form.new_product(), st.form.repo_upserts());
        self.product_calls
            .push(ProductCall::Create(new.clone(), repos.clone()));
        self.background(move |c| {
            AppResult::ProductSaved(
                c.create_product(&new)
                    .map_err(|e| e.message)
                    .inspect(|made| {
                        for repo in &repos {
                            if let Err(e) = c.upsert_repo(&made.id, repo) {
                                tracing::error!(
                                    notify = "error",
                                    "Created {}, but could not add the repo {}: {}",
                                    made.name,
                                    repo.name,
                                    e.message
                                );
                            }
                        }
                    }),
            )
        });
    }

    fn fetch_internal_branches(&mut self, product: String) {
        self.product_calls
            .push(ProductCall::InternalBranches(product.clone()));
        self.background(move |c| {
            AppResult::InternalBranches(
                product.clone(),
                c.internal_branches(&product).map_err(|e| e.message),
            )
        });
    }

    fn fetch_internal_files(&mut self, product: String, branch: String) {
        self.product_calls
            .push(ProductCall::InternalFiles(product.clone(), branch.clone()));
        self.background(move |c| {
            AppResult::InternalFiles(
                product.clone(),
                branch.clone(),
                c.internal_files(&product, &branch).map_err(|e| e.message),
            )
        });
    }

    fn fetch_internal_file(&mut self, product: String, branch: String, path: String) {
        self.product_calls.push(ProductCall::InternalFile(
            product.clone(),
            branch.clone(),
            path.clone(),
        ));
        self.background(move |c| {
            let text = c
                .internal_file(&product, &branch, &path)
                .map_err(|e| e.message);
            // The commit line is a nicety: a failed history shows the text without it.
            let last = text.is_ok().then(|| {
                c.internal_history(&product, &branch, Some(&path))
                    .ok()
                    .and_then(|h| h.into_iter().next())
            });
            AppResult::InternalFile(
                product.clone(),
                branch.clone(),
                path.clone(),
                text.map(|t| (t, last.flatten())),
            )
        });
    }

    /// Management → Products and Overview: what the user asked for there.
    pub(super) fn product_action(&mut self, action: ManagementAction) {
        match action {
            ManagementAction::SetActiveProduct(id) => self.set_active_product(Some(id)),
            ManagementAction::NewProduct => self.open_product_form(),
            ManagementAction::SaveProduct(save) => {
                self.products_tab.busy = true;
                self.products_tab.error = None;
                self.products_tab.repo_errors.clear();
                self.product_calls.push(ProductCall::Save(save.clone()));
                self.background(move |c| {
                    let mut edit = ProductEdit {
                        id: save.id.clone(),
                        ..ProductEdit::default()
                    };
                    if let Some(update) = &save.update {
                        if let Err(e) = c.update_product(&save.id, update) {
                            edit.product_error = Some(e.message);
                        }
                    }
                    for repo in &save.upserts {
                        if let Err(e) = c.upsert_repo(&save.id, repo) {
                            edit.repo_errors.push((repo.name.clone(), e.message));
                        }
                    }
                    for name in &save.removes {
                        if let Err(e) = c.delete_repo(&save.id, name) {
                            edit.repo_errors.push((name.clone(), e.message));
                        }
                    }
                    AppResult::ProductEdited(edit)
                });
            }
            ManagementAction::SaveOverrides { product, overrides } => {
                self.products_tab.overrides_busy = true;
                self.products_tab.overrides_error = None;
                self.product_calls
                    .push(ProductCall::Overrides(product.clone(), overrides.clone()));
                self.background(move |c| {
                    AppResult::OverridesSaved(
                        product.clone(),
                        c.put_product_overrides(&product, overrides)
                            .map_err(|e| e.message),
                    )
                });
            }
            ManagementAction::OpenInternal(product) => self.fetch_internal_branches(product),
            ManagementAction::InternalBranch(product, branch) => {
                self.fetch_internal_files(product, branch)
            }
            ManagementAction::InternalFile(product, branch, path) => {
                self.fetch_internal_file(product, branch, path)
            }
            ManagementAction::DeleteProduct(id) => {
                self.products_tab.delete_error = None;
                self.product_calls.push(ProductCall::Delete(id.clone()));
                self.background(move |c| {
                    AppResult::ProductDeleted(
                        id.clone(),
                        c.delete_product(&id).map_err(|e| e.message),
                    )
                });
            }
            ManagementAction::SaveHidden { product, hidden } => {
                self.products_tab.hidden_busy = true;
                self.products_tab.hidden_error = None;
                self.product_calls
                    .push(ProductCall::Hidden(product.clone(), hidden.clone()));
                self.background(move |c| {
                    AppResult::HiddenSaved(
                        product.clone(),
                        c.put_hidden_specialists(&product, hidden)
                            .map_err(|e| e.message),
                    )
                });
            }
            _ => {}
        }
    }
}
