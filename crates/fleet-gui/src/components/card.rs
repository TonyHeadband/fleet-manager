use super::Command;
use crate::theme::{self, radius, sans, size, tokens, tone_color, Weight};
use crate::widgets::{
    badge, badge_items, badge_width, button, field, text_dot, BadgeTone, ButtonVariant,
};
use eframe::egui::{self, vec2, Align, Layout, Margin, Rect, Sense, Stroke, Ui, UiBuilder};
use fleet_api::{eval, ActionDef, Agent, FieldDef, FieldKind, Tone, UiSchema};
use fleet_client::schema::{column_for, field_value, visible_actions, FieldValue};

/// Per-card UI state that must survive between frames: a pending confirmation of a destructive action.
#[derive(Default)]
pub struct CardState {
    confirming: Option<String>,
}

/// The fixed slot heights, top to bottom. Every card has every slot, filled or not, so all cards are the same size
/// whatever their column or content.
mod slot_height {
    use crate::theme::size;
    /// Repos and time in status.
    pub const META: f32 = 15.0;
    /// Two lines of title, at the title's line height.
    pub const TITLE: f32 = 2.0 * size::TITLE * 1.35;
    pub const BADGES: f32 = 17.0;
    pub const ACTIVITY: f32 = 16.0;
    /// A button's minimum height.
    pub const ACTIONS: f32 = 24.0;
}
const SLOT_GAP: f32 = 4.0;
const MARGIN_Y: i8 = 8;
const STROKE: f32 = 1.0;

/// Every card's height: five slots, the gaps between them, the frame's margin and stroke.
pub const CARD_HEIGHT: f32 = slot_height::META
    + slot_height::TITLE
    + slot_height::BADGES
    + slot_height::ACTIVITY
    + slot_height::ACTIONS
    + 4.0 * SLOT_GAP
    + 2.0 * (MARGIN_Y as f32 + STROKE);

/// Where a schema field is drawn on the card.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Repos,
    Duration,
    Title,
    Badges,
    Activity,
    /// Right of the actions (a stopped agent's size on disk).
    Size,
    /// No room on a fixed-size card: the branch (the side panel's header shows it) and fields this client does
    /// not know.
    Hidden,
}

/// The daemon's schema says which fields exist and when they show; how they are laid out is the client's call,
/// so a field is placed by its key and kind.
fn slot_of(f: &FieldDef) -> Slot {
    match (f.key.as_str(), f.kind) {
        ("repos", _) => Slot::Repos,
        (_, FieldKind::Duration) => Slot::Duration,
        ("task", _) => Slot::Title,
        (_, FieldKind::Badge) => Slot::Badges,
        ("activity", _) => Slot::Activity,
        (_, FieldKind::Bytes) => Slot::Size,
        _ => Slot::Hidden,
    }
}

fn has_value(agent: &Agent, f: &FieldDef) -> bool {
    match field_value(agent, &f.key) {
        FieldValue::Absent => false,
        FieldValue::List(items) => !items.is_empty(),
        _ => true,
    }
}

/// A slot exactly `height` tall and as wide as the card, whatever its content: the content is clipped to it and
/// the card advances by exactly the slot, so no text or badge can make one card taller than another.
fn slot<R>(ui: &mut Ui, height: f32, layout: Layout, add: impl FnOnce(&mut Ui) -> R) -> R {
    let rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), height));
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(layout));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let inner = add(&mut child);
    ui.advance_cursor_after_rect(rect);
    inner
}

/// One row of badges. What does not fit collapses into a "+N" badge whose tooltip lists the rest.
fn badge_row(ui: &mut Ui, items: &[(String, BadgeTone, &str)]) {
    let gap = ui.spacing().item_spacing.x;
    let room = ui.available_width();
    let widths: Vec<f32> = items
        .iter()
        .map(|(text, tone, _)| badge_width(ui, text, *tone))
        .collect();
    let all: f32 = widths.iter().map(|w| w + gap).sum::<f32>() - gap;
    let shown = if all <= room {
        items.len()
    } else {
        // The most leading badges that still leave room for the count of the others.
        let mut used = 0.0;
        let mut n = 0;
        for w in &widths {
            let more = badge_width(ui, &format!("+{}", items.len() - n - 1), BadgeTone::Neutral);
            if used + w + gap + more > room {
                break;
            }
            used += w + gap;
            n += 1;
        }
        n
    };
    for (text, tone, tooltip) in &items[..shown] {
        badge(ui, text, tooltip, *tone);
    }
    if shown < items.len() {
        let rest: Vec<&str> = items[shown..].iter().map(|(t, _, _)| t.as_str()).collect();
        badge(
            ui,
            &format!("+{}", items.len() - shown),
            &rest.join("\n"),
            BadgeTone::Neutral,
        );
    }
}

/// One agent's card: its schema fields in fixed slots, then its schema actions. Returns what the user asked for
/// this frame.
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
    let row = Layout::left_to_right(Align::Center);
    ui.push_id(&agent.id.0, |ui| {
        let frame = egui::Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(
                STROKE,
                if selected { p.text_low } else { p.border },
            ))
            .corner_radius(radius::CARD)
            .inner_margin(Margin::symmetric(9, MARGIN_Y));
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(6.0, SLOT_GAP);
            // Labels must not swallow the click that opens the preview.
            ui.style_mut().interaction.selectable_labels = false;
            // The Ui itself senses the click and registers before its contents, so the labels stay on top and
            // keep their hover tooltips; an overlay added afterwards would swallow them.
            let body = ui
                .scope_builder(
                    UiBuilder::new().id_salt("open").sense(Sense::click()),
                    |ui| {
                        slot(ui, slot_height::META, row, |ui| {
                            for f in in_slot(Slot::Repos) {
                                field(ui, f, agent, now);
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                for f in in_slot(Slot::Duration) {
                                    field(ui, f, agent, now);
                                }
                            });
                        });
                        slot(ui, slot_height::TITLE, Layout::top_down(Align::Min), |ui| {
                            for f in in_slot(Slot::Title) {
                                field(ui, f, agent, now);
                            }
                        });
                        slot(ui, slot_height::BADGES, row, |ui| {
                            let items: Vec<(String, BadgeTone, &str)> = in_slot(Slot::Badges)
                                .flat_map(|f| {
                                    badge_items(f, agent)
                                        .into_iter()
                                        .map(|(text, tone)| (text, tone, f.label.as_str()))
                                })
                                .collect();
                            badge_row(ui, &items);
                        });
                        slot(ui, slot_height::ACTIVITY, row, |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            // The column's tone says Working; a card never names a status.
                            if tone == Tone::Active && in_slot(Slot::Activity).next().is_some() {
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
            slot(ui, slot_height::ACTIONS, row, |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for action in actions {
                    control(ui, action, agent, state, &mut commands);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    for f in in_slot(Slot::Size) {
                        field(ui, f, agent, now);
                    }
                });
            });
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
