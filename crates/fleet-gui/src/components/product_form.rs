//! The one form for a product: New product in the drawer, and Management → Products when it edits one.
use super::persona_setup::caption;
use crate::theme::{size, tokens};
use crate::widgets::{button, list_row, ButtonVariant};
use eframe::egui::{
    self, accesskit::Role, Align, Label, Layout, RichText, TextEdit, Ui, WidgetInfo,
};
use fleet_api::{NewProduct, Persona, Product, ProductUpdate, RepoUpsert};

/// A repo row being typed. A row with nothing in it is ignored; a named one needs a path or a URL.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoDraft {
    pub name: String,
    pub path: String,
    pub url: String,
    /// The product already has this repo: its name is fixed (a rename is Remove and Add).
    pub existing: bool,
    /// The stored repo has a URL: it cannot be changed or cleared (Remove the repo and add it again).
    pub had_url: bool,
    /// The stored repo has a path: it can be changed, but not cleared.
    pub had_path: bool,
}

impl RepoDraft {
    fn blank(&self) -> bool {
        [&self.name, &self.path, &self.url]
            .iter()
            .all(|s| s.trim().is_empty())
    }

    fn valid(&self) -> bool {
        !self.name.trim().is_empty() && !(self.path.trim().is_empty() && self.url.trim().is_empty())
    }

    fn upsert(&self) -> RepoUpsert {
        let some = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
        RepoUpsert {
            name: self.name.trim().to_string(),
            path: some(&self.path),
            url: some(&self.url),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProductForm {
    pub name: String,
    /// A persona id; None is "No default".
    pub default_persona: Option<String>,
    pub repos: Vec<RepoDraft>,
}

/// What saving an edited product sends: the product's own fields, then the repos that changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProductSave {
    pub id: String,
    pub update: Option<ProductUpdate>,
    pub upserts: Vec<RepoUpsert>,
    pub removes: Vec<String>,
}

impl ProductSave {
    pub fn is_empty(&self) -> bool {
        self.update.is_none() && self.upserts.is_empty() && self.removes.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductFormAction {
    Save,
    Cancel,
}

impl From<&Product> for ProductForm {
    fn from(p: &Product) -> Self {
        ProductForm {
            name: p.name.clone(),
            default_persona: p.default_persona.clone(),
            repos: p
                .repos
                .iter()
                .map(|r| RepoDraft {
                    name: r.name.clone(),
                    path: r.path.clone().unwrap_or_default(),
                    url: r.url.clone().unwrap_or_default(),
                    existing: true,
                    had_url: r.url.is_some(),
                    had_path: r.path.is_some(),
                })
                .collect(),
        }
    }
}

impl ProductForm {
    /// What differs from `saved` (the product as the daemon has it). Emptying a path or URL of a stored repo is not
    /// a change: the daemon only fills them in.
    pub fn diff(&self, id: &str, saved: &ProductForm) -> ProductSave {
        let name = self.name.trim();
        let mut update = ProductUpdate::default();
        if !name.is_empty() && name != saved.name {
            update.name = Some(name.to_string());
        }
        if self.default_persona != saved.default_persona {
            update.default_persona = Some(self.default_persona.clone());
        }
        let mut upserts = Vec::new();
        for r in self.repos.iter().filter(|r| !r.blank()) {
            let Some(stored) = saved.repos.iter().find(|s| s.name == r.name.trim()) else {
                upserts.push(r.upsert());
                continue;
            };
            let mut up = r.upsert();
            up.path = up.path.filter(|p| *p != stored.path);
            up.url = up.url.filter(|u| *u != stored.url);
            if up.path.is_some() || up.url.is_some() {
                upserts.push(up);
            }
        }
        let removes = saved
            .repos
            .iter()
            .filter(|s| !self.repos.iter().any(|r| r.name.trim() == s.name))
            .map(|s| s.name.clone())
            .collect();
        ProductSave {
            id: id.to_string(),
            update: (update != ProductUpdate::default()).then_some(update),
            upserts,
            removes,
        }
    }

    /// Saving needs a name, and every row that has anything in it a name and a path or URL.
    pub fn can_save(&self) -> bool {
        !self.name.trim().is_empty() && self.repos.iter().all(|r| r.blank() || r.valid())
    }

    pub fn new_product(&self) -> NewProduct {
        NewProduct {
            name: self.name.trim().to_string(),
            default_persona: self.default_persona.clone(),
        }
    }

    /// The rows to send, without the blank ones.
    pub fn repo_upserts(&self) -> Vec<RepoUpsert> {
        self.repos
            .iter()
            .filter(|r| !r.blank())
            .map(RepoDraft::upsert)
            .collect()
    }
}

/// Draws the form's fields and its Save and Cancel buttons; the caller frames it (a modal) and titles it.
/// `personas` are the listed ones; the default persona is picked among the ready ones.
pub fn product_form(
    ui: &mut Ui,
    f: &mut ProductForm,
    personas: &[Persona],
) -> Option<ProductFormAction> {
    let mut action = None;
    product_fields(ui, f, personas, &mut |_, _, _| {});
    ui.add_space(12.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let save = ui
            .add_enabled_ui(f.can_save(), |ui| {
                button(ui, "Save", ButtonVariant::Primary)
            })
            .inner;
        if save.clicked() {
            action = Some(ProductFormAction::Save);
        }
        if button(ui, "Cancel", ButtonVariant::Default).clicked() {
            action = Some(ProductFormAction::Cancel);
        }
    });
    action
}

/// The form's fields without its buttons, for a caller that has its own (Management → Products). `row_extra` is
/// drawn under each repo row, with the row's index and draft.
pub fn product_fields(
    ui: &mut Ui,
    f: &mut ProductForm,
    personas: &[Persona],
    row_extra: &mut dyn FnMut(&mut Ui, usize, &RepoDraft),
) {
    let p = tokens(ui.visuals().dark_mode);
    let label = caption(ui, "Name");
    ui.add_space(4.0);
    ui.add(
        TextEdit::singleline(&mut f.name)
            .desired_width(f32::INFINITY)
            .hint_text("What the product is called"),
    )
    .labelled_by(label.id);
    ui.add_space(8.0);

    caption(ui, "Default persona");
    ui.add_space(4.0);
    let ready: Vec<&Persona> = personas.iter().filter(|p| p.ready).collect();
    let shown = f
        .default_persona
        .as_deref()
        .and_then(|id| ready.iter().find(|p| p.id == id))
        .map_or("No default", |p| p.name.as_str());
    let combo = egui::ComboBox::from_id_salt("product_default_persona")
        .selected_text(RichText::new(shown).color(p.text_high))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            let none = f.default_persona.is_none();
            if list_row(ui, "No default", none, |ui| {
                ui.label(RichText::new("No default").color(p.text_high));
            })
            .clicked()
            {
                f.default_persona = None;
            }
            for persona in &ready {
                let on = f.default_persona.as_deref() == Some(persona.id.as_str());
                if list_row(ui, &persona.name, on, |ui| {
                    ui.add(Label::new(RichText::new(&persona.name).color(p.text_high)).truncate());
                })
                .clicked()
                {
                    f.default_persona = Some(persona.id.clone());
                }
            }
        });
    combo
        .response
        .widget_info(|| WidgetInfo::labeled(Role::ComboBox, ui.is_enabled(), "Default persona"));
    ui.add_space(8.0);

    caption(ui, "Repos");
    ui.add_space(4.0);
    let mut remove = None;
    for (i, repo) in f.repos.iter_mut().enumerate() {
        ui.push_id(("product_repo", i), |ui| {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 90.0) / 3.0;
                let (locked, had_url) = (repo.existing, repo.had_url);
                for (text, hint, fixed) in [
                    (&mut repo.name, "Repo name", locked),
                    (&mut repo.path, "Local path", false),
                    (&mut repo.url, "Clone URL", had_url),
                ] {
                    let edit = ui.add(
                        TextEdit::singleline(text)
                            .hint_text(hint)
                            .interactive(!fixed)
                            .desired_width(width),
                    );
                    edit.widget_info(|| {
                        WidgetInfo::labeled(Role::TextInput, true, format!("{hint} {}", i + 1))
                    });
                    if fixed && hint == "Clone URL" {
                        edit.on_hover_text("Remove the repo and add it again to change its URL");
                    }
                }
                let gone = button(ui, "Remove", ButtonVariant::Ghost);
                gone.widget_info(|| {
                    WidgetInfo::labeled(Role::Button, true, format!("Remove repo {}", i + 1))
                });
                if gone.clicked() {
                    remove = Some(i);
                }
            });
            if repo.had_path && repo.path.trim().is_empty() {
                ui.label(
                    RichText::new("A path cannot be cleared; Revert restores it.")
                        .size(size::SMALL)
                        .color(p.text_low),
                );
            }
            row_extra(ui, i, repo);
        });
    }
    if let Some(i) = remove {
        f.repos.remove(i);
    }
    if button(ui, "Add repo", ButtonVariant::Default).clicked() {
        f.repos.push(RepoDraft::default());
    }
    if f.repos.iter().any(|r| !r.blank() && !r.valid()) {
        ui.label(
            RichText::new("A repo needs a name and a local path or a clone URL.")
                .size(size::SMALL)
                .color(p.text_low),
        );
    }
}
