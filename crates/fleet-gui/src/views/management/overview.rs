//! Management → Overview: every product at a glance, read-only. Each row has the product's unreleased releases with
//! their two bars and its tasks counted by where they stand. The only action is picking the active product.
use super::{ManagementAction, ManagementData};
use crate::theme::{sans, size, tokens, tone_color, Weight};
use crate::views::releases::release_bars;
use crate::widgets::{button, fui, list_row, ButtonVariant};
use eframe::egui::{self, Frame, Margin, RichText, ScrollArea, Ui};
use fleet_api::{release_progress, task_outcome, today, Product, Status, Task, TaskOutcome, Tone};
use std::collections::HashMap;

use crate::theme::radius;

/// The filter row's choices. Product is the only one so far; token and time filters join it in the same strip.
#[derive(Debug, Default)]
pub struct OverviewState {
    /// The one product shown; None is All.
    pub product: Option<String>,
}

/// A product's tasks by where they stand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub working: u32,
    pub needs_you: u32,
    pub review: u32,
    pub merged: u32,
}

/// Merged and done-by-hand tasks are merged, as the release bars count them; any other task with a live agent is
/// counted by that agent's board status. `statuses` maps agent ids, which are the tasks' ids, to their status.
pub fn counts(product: &str, tasks: &[Task], statuses: &HashMap<String, Status>) -> Counts {
    let mut c = Counts::default();
    for t in tasks.iter().filter(|t| t.product == product) {
        match task_outcome(t) {
            TaskOutcome::Merged | TaskOutcome::DoneByHand => c.merged += 1,
            TaskOutcome::Open | TaskOutcome::Dropped => match statuses.get(&t.id.0) {
                Some(Status::Working) => c.working += 1,
                Some(Status::NeedsYou) => c.needs_you += 1,
                Some(Status::Review) => c.review += 1,
                _ => {}
            },
        }
    }
    c
}

pub(super) fn overview(
    ui: &mut Ui,
    st: &mut OverviewState,
    data: &ManagementData,
) -> Option<ManagementAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    Frame::new()
        .inner_margin(Margin::symmetric(22, 18))
        .show(ui, |ui| {
            let Some(products) = data.products else {
                ui.label(RichText::new("Loading…").color(p.text_low));
                return;
            };
            // A product that was deleted no longer narrows anything.
            if st
                .product
                .as_ref()
                .is_some_and(|id| !products.iter().any(|p| &p.id == id))
            {
                st.product = None;
            }
            filter_row(ui, st, products);
            ui.add_space(10.0);
            if products.is_empty() {
                ui.label(
                    RichText::new("No products yet")
                        .size(size::SMALL)
                        .color(p.text_low),
                );
            }
            ScrollArea::vertical()
                .id_salt("overview")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for product in products
                        .iter()
                        .filter(|p| st.product.as_ref().is_none_or(|id| *id == p.id))
                    {
                        let row = ui
                            .push_id(&product.id, |ui| product_row(ui, product, data))
                            .inner;
                        if row.is_some() {
                            action = row;
                        }
                        ui.add_space(10.0);
                    }
                });
        });
    action
}

/// One horizontal strip, so the next filters sit beside the first.
fn filter_row(ui: &mut Ui, st: &mut OverviewState, products: &[Product]) {
    let p = tokens(ui.visuals().dark_mode);
    ui.horizontal(|ui| {
        let label = fui::mono_text("Product", p.text_low);
        let shown = st
            .product
            .as_ref()
            .and_then(|id| products.iter().find(|p| &p.id == id))
            .map_or("All products", |p| p.name.as_str());
        let value = format!("Product {shown}");
        let mut picked = None;
        let combo = egui::ComboBox::from_id_salt("overview_product")
            .selected_text((label, shown))
            .show_ui(ui, |ui| {
                ui.set_min_width(160.0);
                let mut row = |ui: &mut Ui, id: Option<&str>, name: &str, a11y: String| {
                    let on = st.product.as_deref() == id;
                    if list_row(ui, &a11y, on, |ui| {
                        ui.label(RichText::new(name).color(p.text_high));
                    })
                    .clicked()
                        && !on
                    {
                        picked = Some(id.map(str::to_string));
                    }
                };
                row(ui, None, "All products", "Show all products".into());
                for product in products {
                    row(
                        ui,
                        Some(&product.id),
                        &product.name,
                        format!("Show {}", product.name),
                    );
                }
            });
        combo
            .response
            .widget_info(|| fui::combo_info(ui.is_enabled(), "Product", value.clone()));
        if let Some(choice) = picked {
            st.product = choice;
        }
    });
}

fn product_row(ui: &mut Ui, product: &Product, data: &ManagementData) -> Option<ManagementAction> {
    let p = tokens(ui.visuals().dark_mode);
    let dark = ui.visuals().dark_mode;
    let mut action = None;
    Frame::new()
        .fill(p.surface)
        .corner_radius(radius::CARD)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if button(ui, &product.name, ButtonVariant::Ghost).clicked() {
                    action = Some(ManagementAction::SetActiveProduct(product.id.clone()));
                }
                if data.active == Some(product.id.as_str()) {
                    ui.label(RichText::new("active").size(size::SMALL).color(p.text_low));
                }
                ui.add_space(12.0);
                let c = counts(&product.id, data.tasks, data.statuses);
                for (name, n, tone) in [
                    ("Working", c.working, Tone::Active),
                    ("Needs you", c.needs_you, Tone::Attention),
                    ("Review", c.review, Tone::Done),
                    ("Merged", c.merged, Tone::Done),
                ] {
                    let colour = if n > 0 {
                        tone_color(tone, dark)
                    } else {
                        p.text_low
                    };
                    ui.label(
                        RichText::new(format!("{name} {n}"))
                            .size(size::SMALL)
                            .color(colour),
                    );
                }
            });
            let day = today(data.now);
            for r in product.releases.iter().filter(|r| r.released_at.is_none()) {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&r.version)
                            .font(sans(ui, size::BODY, Weight::Medium))
                            .color(p.text_high),
                    );
                    release_bars(ui, r, &release_progress(r, data.tasks, day));
                });
            }
        });
    action
}
