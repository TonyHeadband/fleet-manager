//! The one form for a product: New product in the drawer, and Management → Products when it edits one.
use super::persona_setup::caption;
use crate::theme::{size, tokens};
use crate::widgets::{button, list_row, ButtonVariant};
use eframe::egui::{
    self, accesskit::Role, Align, Label, Layout, RichText, TextEdit, Ui, WidgetInfo,
};
use fleet_api::{NewProduct, Persona, RepoUpsert};

/// A repo row being typed. A row with nothing in it is ignored; a named one needs a path or a URL.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoDraft {
    pub name: String,
    pub path: String,
    pub url: String,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductFormAction {
    Save,
    Cancel,
}

impl ProductForm {
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
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
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
                for (text, hint) in [
                    (&mut repo.name, "Repo name"),
                    (&mut repo.path, "Local path"),
                    (&mut repo.url, "Clone URL"),
                ] {
                    ui.add(
                        TextEdit::singleline(text)
                            .hint_text(hint)
                            .desired_width(width),
                    );
                }
                if button(ui, "Remove", ButtonVariant::Ghost).clicked() {
                    remove = Some(i);
                }
            });
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
