use super::Command;
use crate::theme::{self, radius, sans, size, tokens, tone_color, Weight};
use crate::widgets::{button, field, text_dot, ButtonVariant};
use eframe::egui::{self, vec2, Align, Layout, Margin, Sense, Stroke, Ui, UiBuilder};
use fleet_api::{eval, ActionDef, Agent, FieldDef, FieldKind, Tone, UiSchema};
use fleet_client::schema::{column_for, field_value, visible_actions, FieldValue};

/// Per-card UI state that must survive between frames: a pending kill confirmation.
#[derive(Default)]
pub struct CardState {
    confirming: Option<String>,
}

/// Where a schema field is drawn on the card; the variants are in drawing order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Repos,
    Duration,
    Title,
    Badges,
    Activity,
    Branch,
    Body,
}

/// The daemon's schema says which fields exist and when they show; how they are laid out is the client's call,
/// so a field is placed by its key and kind. A field it does not know falls into the body.
fn slot_of(f: &FieldDef) -> Slot {
    match (f.key.as_str(), f.kind) {
        ("repos", _) => Slot::Repos,
        (_, FieldKind::Duration) => Slot::Duration,
        ("task", _) => Slot::Title,
        (_, FieldKind::Badge) => Slot::Badges,
        ("activity", _) => Slot::Activity,
        (_, FieldKind::Mono) => Slot::Branch,
        _ => Slot::Body,
    }
}

fn has_value(agent: &Agent, f: &FieldDef) -> bool {
    match field_value(agent, &f.key) {
        FieldValue::Absent => false,
        FieldValue::List(items) => !items.is_empty(),
        _ => true,
    }
}

/// A row as tall as its content: `horizontal` would pad small text up to the 18 px interact height.
fn tight_row<R>(ui: &mut Ui, wrapped: bool, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = 0.0;
        if wrapped {
            ui.horizontal_wrapped(add).inner
        } else {
            ui.horizontal(add).inner
        }
    })
    .inner
}

/// One agent's card: its schema fields, then its schema actions. Returns what the user asked for this frame.
pub fn card(
    ui: &mut Ui,
    schema: &UiSchema,
    agent: &Agent,
    now: i64,
    state: &mut CardState,
    selected: bool,
) -> Vec<Command> {
    let mut commands = Vec::new();
    let dark = ui.visuals().dark_mode;
    let p = tokens(dark);
    let tone = column_for(schema, agent).map_or(Tone::Neutral, |c| c.tone);
    // Fields with nothing to show draw nothing, so they must not get a row either.
    let shown: Vec<(Slot, &FieldDef)> = schema
        .card_fields
        .iter()
        .filter(|f| eval(&f.show_when, agent) && has_value(agent, f))
        .map(|f| (slot_of(f), f))
        .collect();
    let in_slot = |slot: Slot| {
        shown
            .iter()
            .filter(move |(s, _)| *s == slot)
            .map(|(_, f)| *f)
    };
    ui.push_id(&agent.id.0, |ui| {
        let frame = egui::Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(
                1.0,
                if selected { p.text_low } else { p.border },
            ))
            .corner_radius(radius::CARD)
            .inner_margin(Margin::symmetric(9, 8));
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            // Labels must not swallow the click that opens the preview.
            ui.style_mut().interaction.selectable_labels = false;
            // The Ui itself senses the click and registers before its contents, so the labels stay on top and
            // keep their hover tooltips; an overlay added afterwards would swallow them.
            let body = ui
                .scope_builder(
                    UiBuilder::new().id_salt("open").sense(Sense::click()),
                    |ui| {
                        if in_slot(Slot::Repos)
                            .chain(in_slot(Slot::Duration))
                            .next()
                            .is_some()
                        {
                            tight_row(ui, false, |ui| {
                                for f in in_slot(Slot::Repos) {
                                    field(ui, f, agent, now);
                                }
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    for f in in_slot(Slot::Duration) {
                                        field(ui, f, agent, now);
                                    }
                                });
                            });
                        }
                        for f in in_slot(Slot::Title) {
                            field(ui, f, agent, now);
                        }
                        if in_slot(Slot::Badges).next().is_some() {
                            ui.horizontal_wrapped(|ui| {
                                for f in in_slot(Slot::Badges) {
                                    field(ui, f, agent, now);
                                }
                            });
                        }
                        if in_slot(Slot::Activity).next().is_some() {
                            tight_row(ui, true, |ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                // The column's tone says Working; a card never names a status.
                                if tone == Tone::Active {
                                    // A paused agent is doing nothing, so its dot rests instead of breathing.
                                    let alpha = if agent.paused {
                                        1.0
                                    } else {
                                        theme::pulse(ui, 1.6)
                                    };
                                    let font = sans(ui, size::SMALL, Weight::Regular);
                                    text_dot(
                                        ui,
                                        tone_color(tone, dark).gamma_multiply(alpha),
                                        5.0,
                                        &font,
                                    );
                                }
                                for f in in_slot(Slot::Activity) {
                                    field(ui, f, agent, now);
                                }
                            });
                        }
                        for f in in_slot(Slot::Branch).chain(in_slot(Slot::Body)) {
                            field(ui, f, agent, now);
                        }
                    },
                )
                .response;
            if body.clicked() {
                commands.push(Command::Open(agent.id.0.clone()));
            }
            // Input actions (Answer) live in the side panel's reply box, not on the card.
            let actions: Vec<&ActionDef> = visible_actions(schema, agent)
                .into_iter()
                .filter(|a| a.input.is_none())
                .collect();
            if !actions.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for action in actions {
                        control(ui, action, agent, state, &mut commands);
                    }
                });
            }
        });
    });
    commands
}

fn control(
    ui: &mut Ui,
    action: &ActionDef,
    agent: &Agent,
    state: &mut CardState,
    commands: &mut Vec<Command>,
) {
    let run = || Command::Run {
        action: action.clone(),
        agent: Box::new(agent.clone()),
        input: None,
    };
    if action.destructive {
        if state.confirming.as_deref() == Some(action.id.as_str()) {
            if button(
                ui,
                &format!("Confirm {}", action.label),
                ButtonVariant::Danger,
            )
            .clicked()
            {
                state.confirming = None;
                commands.push(run());
            }
        } else if button(ui, &action.label, ButtonVariant::QuietDanger).clicked() {
            state.confirming = Some(action.id.clone());
        }
    } else if button(ui, &action.label, ButtonVariant::Ghost).clicked() {
        commands.push(run());
    }
}
