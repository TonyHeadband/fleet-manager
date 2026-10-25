use super::card::{card, fleet_card, project_card, CardState};
use super::fleet_row::fleet_row;
use super::Command;
use crate::theme::{layout, sans, size, tokens, tone_color, Weight};
use crate::widgets::dot;
use eframe::egui::{self, Align, Layout, Margin, RichText, ScrollArea, Stroke, Ui};
use fleet_api::{ColumnDef, Status, UiSchema};
use fleet_client::config::FleetDisplay;
use fleet_client::schema::Group;
use std::collections::HashMap;

/// The fleet sits a little in from its project card, so a group reads as one.
const FLEET_INDENT: f32 = 10.0;

/// One column: title, count and its groups (a solo card, or a project card with its fleet), longest-waiting first,
/// scrolling on its own. The count is of groups, so a project counts once. `tags` is the board's.
#[allow(clippy::too_many_arguments)]
pub fn column(
    ui: &mut Ui,
    def: &ColumnDef,
    schema: &UiSchema,
    groups: &[&Group<'_>],
    now: i64,
    states: &mut HashMap<String, CardState>,
    selected: Option<&str>,
    tags: &HashMap<String, String>,
    display: FleetDisplay,
) -> Vec<Command> {
    let dark = ui.visuals().dark_mode;
    let p = tokens(dark);
    let mut cards: Vec<&Group<'_>> = groups.to_vec();
    cards.sort_by_key(|g| g.since());
    let mut commands = Vec::new();
    // The header and the body touch; the board draws no gaps of its own.
    ui.spacing_mut().item_spacing.y = 0.0;

    let header = egui::Frame::new()
        .fill(p.surface)
        .inner_margin(Margin::symmetric(10, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Fixed, not content-sized: the detail panel's top row shares it, and their bottom lines meet.
            ui.set_height(layout::COLUMN_HEADER);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                dot(ui, tone_color(def.tone, dark), 7.0);
                ui.label(
                    RichText::new(&def.title)
                        .font(sans(ui, size::TITLE, Weight::Medium))
                        .color(p.text_high),
                );
                ui.label(
                    RichText::new(cards.len().to_string())
                        .font(sans(ui, size::SMALL, Weight::Regular))
                        .color(p.text_low),
                );
            });
        });
    // Inside the header's last pixel row, so nothing the body paints can cover it; the body is the panel's `bg`.
    let r = header.response.rect;
    ui.painter()
        .hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.border));

    if def.dim {
        // Behind the cards, down to the bottom of the board: the column of agents that are no longer running.
        ui.painter()
            .rect_filled(ui.available_rect_before_wrap(), 0.0, p.sunken);
    }
    ScrollArea::vertical()
        .id_salt(&def.id)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new().inner_margin(8).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 6.0;
                if cards.is_empty() {
                    ui.label(
                        RichText::new(&def.empty_hint)
                            .font(sans(ui, size::SMALL, Weight::Regular))
                            .color(p.text_low),
                    );
                }
                for g in cards {
                    let a = g.lead;
                    let persona = tags.get(&a.persona).map(String::as_str);
                    let is_selected = |a: &fleet_api::Agent| selected == Some(a.id.0.as_str());
                    if !g.orchestrated() {
                        commands.extend(card(
                            ui,
                            schema,
                            a,
                            now,
                            states.entry(a.id.0.clone()).or_default(),
                            is_selected(a),
                            persona,
                        ));
                        continue;
                    }
                    let state = states.entry(a.id.0.clone()).or_default();
                    commands.extend(project_card(
                        ui,
                        schema,
                        g,
                        now,
                        state,
                        is_selected(a),
                        persona,
                        display,
                    ));
                    if display == FleetDisplay::Hide || !state.fleet_open || g.fleet.is_empty() {
                        continue;
                    }
                    // The board has no project record; `open_pr` leaves this on the lead.
                    let pr = a.activity == "PR open";
                    ui.scope(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.spacing_mut().indent = FLEET_INDENT;
                        ui.indent(("fleet", &a.id.0), |ui| {
                            for f in &g.fleet {
                                let stuck = f.status == Status::NeedsYou;
                                if display == FleetDisplay::Show {
                                    commands.extend(fleet_card(
                                        ui,
                                        schema,
                                        f,
                                        &a.task,
                                        now,
                                        states.entry(f.id.0.clone()).or_default(),
                                        is_selected(f),
                                    ));
                                } else {
                                    commands.extend(fleet_row(
                                        ui,
                                        schema,
                                        f,
                                        now,
                                        is_selected(f),
                                        stuck,
                                        pr,
                                    ));
                                }
                            }
                        });
                    });
                }
            });
        });
    commands
}
