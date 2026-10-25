use super::Command;
use crate::theme::{self, mono, radius, sans, size, tokens, tone_color, Weight};
use crate::widgets::{
    badge, badge_items, button, field, small_icon_button, text_dot, BadgeTone, ButtonVariant, Icon,
};
use eframe::egui::{
    self, vec2, Align, Label, Layout, Margin, Rect, RichText, Sense, Stroke, Ui, UiBuilder,
    WidgetInfo,
};
use fleet_api::{eval, ActionDef, Agent, FieldDef, FieldKind, Status, Tone, UiSchema};
use fleet_client::schema::{column_for, field_value, visible_actions, FieldValue};

/// How long a destructive action stays armed for its second click.
const CONFIRM_FOR: f64 = 4.0;

/// Per-card UI state that must survive between frames: a pending confirmation of a destructive action.
#[derive(Default)]
pub struct CardState {
    /// The armed action's id and when it was armed (egui's input time).
    confirming: Option<(String, f64)>,
}

/// The fixed row heights, top to bottom. Every card has every row, filled or not, so all cards are the same size
/// whatever their column or content.
mod row_height {
    use crate::theme::size;
    /// One line of title, at the title's line height, and the time in status.
    pub const TITLE: f32 = size::TITLE * 1.35;
    /// The state badge, the changes dot, the repo and the size.
    pub const STATE: f32 = 17.0;
    pub const ACTIVITY: f32 = 16.0;
    /// A text button's minimum height, for an action the client has no icon for.
    pub const ACTIONS: f32 = 24.0;
}
const ROW_GAP: f32 = 4.0;
const MARGIN_Y: i8 = 8;
const STROKE: f32 = 1.0;

/// Every card's height: four rows, the gaps between them, the frame's margin and stroke.
pub const CARD_HEIGHT: f32 = row_height::TITLE
    + row_height::STATE
    + row_height::ACTIVITY
    + row_height::ACTIONS
    + 3.0 * ROW_GAP
    + 2.0 * (MARGIN_Y as f32 + STROKE);

/// Where a schema field is drawn on the card.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Title,
    Duration,
    /// Candidates for the one state badge.
    State,
    Changes,
    Repos,
    Activity,
    /// Right of the state row (a stopped agent's size on disk).
    Size,
    /// No room on a fixed-size card: the branch and fields this client does not know. The side panel's details
    /// list them.
    Panel,
}

/// The daemon's schema says which fields exist and when they show; how they are laid out is the client's call,
/// so a field is placed by its key and kind.
fn slot_of(f: &FieldDef) -> Slot {
    match (f.key.as_str(), f.kind) {
        ("repos", _) => Slot::Repos,
        ("task", _) => Slot::Title,
        ("activity", _) => Slot::Activity,
        ("has_changes", _) => Slot::Changes,
        (_, FieldKind::Duration) => Slot::Duration,
        (_, FieldKind::Badge) => Slot::State,
        (_, FieldKind::Bytes) => Slot::Size,
        _ => Slot::Panel,
    }
}

/// Which badge field wins the card's one badge: why it stopped, then why it is blocked, then paused. A badge field
/// this client does not know comes last, so a newer daemon's badge still shows when nothing else does.
fn state_rank(key: &str) -> u8 {
    match key {
        "stop_cause" => 0,
        "blocked_reason" => 1,
        "paused" => 2,
        _ => 3,
    }
}

fn has_value(agent: &Agent, f: &FieldDef) -> bool {
    match field_value(agent, &f.key) {
        FieldValue::Absent => false,
        FieldValue::List(items) => !items.is_empty(),
        _ => true,
    }
}

/// A row exactly `height` tall and as wide as the card, whatever its content: the content is clipped to it and
/// the card advances by exactly the row, so no text or badge can make one card taller than another.
fn row<R>(ui: &mut Ui, height: f32, layout: Layout, add: impl FnOnce(&mut Ui) -> R) -> R {
    let rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), height));
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(layout));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let inner = add(&mut child);
    ui.advance_cursor_after_rect(rect);
    inner
}

/// The least of a repo name worth showing beside its "+N": a letter and the "…".
const MIN_NAME: f32 = 16.0;

/// The first repo, cut with "…" when the row runs out, then "+N" for the others. Both name every repo on hover.
fn repos(ui: &mut Ui, names: &[String]) {
    let Some(first) = names.first() else {
        return;
    };
    let p = tokens(ui.visuals().dark_mode);
    let all = names.join("\n");
    let text = |s: &str| RichText::new(s).font(mono(size::META)).color(p.text_low);
    let more = (names.len() > 1).then(|| format!("+{}", names.len() - 1));
    // The count keeps its room; only the name is cut.
    let more_width = more.as_ref().map_or(0.0, |m| {
        let galley = ui
            .painter()
            .layout_no_wrap(m.clone(), mono(size::META), p.text_low);
        galley.size().x + ui.spacing().item_spacing.x
    });
    // Too narrow for a scrap of the name beside the count: the cut name alone, rather than a half-drawn count; with
    // no room even for that, nothing (the side panel lists the repos).
    let (more, more_width) = if ui.available_width() >= more_width + MIN_NAME {
        (more, more_width)
    } else if ui.available_width() >= MIN_NAME {
        (None, 0.0)
    } else {
        return;
    };
    ui.scope(|ui| {
        ui.set_max_width((ui.available_width() - more_width).max(0.0));
        ui.add(
            Label::new(text(first))
                .truncate()
                .show_tooltip_when_elided(false),
        )
        .on_hover_text(&all);
    });
    if let Some(m) = more {
        ui.label(text(&m)).on_hover_text(&all);
    }
}

/// The left half of the actions row: the agent's persona, cut with "…". Draws nothing until personas exist.
fn persona_slot(ui: &mut Ui, persona: Option<&str>) {
    let Some(name) = persona else {
        return;
    };
    let p = tokens(ui.visuals().dark_mode);
    let rich = RichText::new(name)
        .font(sans(ui, size::SMALL, Weight::Regular))
        .color(p.text_low);
    ui.add(Label::new(rich).truncate().show_tooltip_when_elided(false))
        .on_hover_text(name);
}

/// One agent's card: four fixed rows (title and time; state badge, changes, repo and size; activity; persona and
/// actions). Returns what the user asked for this frame.
pub fn card(
    ui: &mut Ui,
    schema: &UiSchema,
    agent: &Agent,
    now: i64,
    state: &mut CardState,
    selected: bool,
    persona: Option<&str>,
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
    let state_badge: Option<(String, BadgeTone, &str)> = in_slot(Slot::State)
        .min_by_key(|f| state_rank(&f.key))
        .and_then(|f| {
            badge_items(f, agent)
                .into_iter()
                .next()
                .map(|(text, tone)| (text, tone, f.label.as_str()))
        });
    let ltr = Layout::left_to_right(Align::Center);
    let rtl = Layout::right_to_left(Align::Center);
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
            ui.spacing_mut().item_spacing = vec2(6.0, ROW_GAP);
            // Labels must not swallow the click that opens the preview.
            ui.style_mut().interaction.selectable_labels = false;
            // The Ui itself senses the click and registers before its contents, so the labels stay on top and
            // keep their hover tooltips; an overlay added afterwards would swallow them.
            let body = ui
                .scope_builder(
                    UiBuilder::new().id_salt("open").sense(Sense::click()),
                    |ui| {
                        // Right to left first, so what sits at the right keeps its room and the left text is cut.
                        row(ui, row_height::TITLE, rtl, |ui| {
                            for f in in_slot(Slot::Duration) {
                                field(ui, f, agent, now);
                            }
                            ui.with_layout(ltr, |ui| {
                                for f in in_slot(Slot::Title) {
                                    field(ui, f, agent, now);
                                }
                            });
                        });
                        row(ui, row_height::STATE, rtl, |ui| {
                            for f in in_slot(Slot::Size) {
                                field(ui, f, agent, now);
                            }
                            ui.with_layout(ltr, |ui| {
                                // A "+N" or a badge with no room left must not paint over the size.
                                ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                                if let Some((text, tone, tooltip)) = &state_badge {
                                    badge(ui, text, tooltip, *tone);
                                }
                                for f in in_slot(Slot::Changes) {
                                    let font = sans(ui, size::SMALL, Weight::Regular);
                                    let dot = text_dot(ui, p.text, 6.0, &font);
                                    dot.widget_info(|| {
                                        WidgetInfo::labeled(
                                            egui::accesskit::Role::Image,
                                            true,
                                            &f.label,
                                        )
                                    });
                                    dot.on_hover_text(&f.label);
                                }
                                for f in in_slot(Slot::Repos) {
                                    if let FieldValue::List(names) = field_value(agent, &f.key) {
                                        repos(ui, &names);
                                    }
                                }
                            });
                        });
                        row(ui, row_height::ACTIVITY, ltr, |ui| {
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
            row(ui, row_height::ACTIONS, ltr, |ui| {
                let (left, right) = ui.max_rect().split_left_right_at_fraction(0.5);
                ui.scope_builder(UiBuilder::new().max_rect(left).layout(ltr), |ui| {
                    persona_slot(ui, persona)
                });
                ui.scope_builder(UiBuilder::new().max_rect(right).layout(rtl), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    // Right to left: the schema's last action (Stop) ends the row.
                    for action in actions.iter().rev() {
                        control(ui, action, agent, state, &mut commands);
                    }
                    // Not from the schema: the Projects view exists only in this client, so the daemon cannot
                    // offer it.
                    if agent.status == Status::Stopped
                        && small_icon_button(ui, Icon::Info, "Details", false).clicked()
                    {
                        commands.push(Command::Details(agent.id.0.clone()));
                    }
                });
            });
        });
    });
    commands
}

/// The icon a known action id is drawn with; any other action is a text button, so nothing the schema offers
/// is lost.
fn icon_of(action: &ActionDef) -> Option<Icon> {
    match action.id.as_str() {
        "stop" => Some(Icon::Stop),
        "pause" => Some(Icon::Pause),
        "resume" => Some(Icon::Resume),
        _ => None,
    }
}

/// One action. A destructive one takes two clicks: the first arms it (red), the second runs it; a press anywhere
/// else or `CONFIRM_FOR` seconds disarm it.
fn control(
    ui: &mut Ui,
    action: &ActionDef,
    agent: &Agent,
    state: &mut CardState,
    commands: &mut Vec<Command>,
) {
    let time = ui.input(|i| i.time);
    if let Some((id, since)) = &state.confirming {
        if *id == action.id {
            let left = since + CONFIRM_FOR - time;
            if left > 0.0 {
                // Nothing else may wake the window to see the timeout.
                ui.ctx().request_repaint_after_secs(left as f32);
            } else {
                state.confirming = None;
            }
        }
    }
    let armed = action.destructive
        && state
            .confirming
            .as_ref()
            .is_some_and(|(id, _)| *id == action.id);
    let response = match (icon_of(action), armed) {
        (Some(icon), true) => small_icon_button(
            ui,
            icon,
            &format!("Click again to {}", action.label.to_lowercase()),
            true,
        ),
        (Some(icon), false) => small_icon_button(ui, icon, &action.label, false),
        (None, true) => button(
            ui,
            &format!("Confirm {}", action.label),
            ButtonVariant::Danger,
        ),
        (None, false) if action.destructive => {
            button(ui, &action.label, ButtonVariant::QuietDanger)
        }
        (None, false) => button(ui, &action.label, ButtonVariant::Ghost),
    };
    if response.clicked() {
        if action.destructive && !armed {
            state.confirming = Some((action.id.clone(), time));
        } else {
            state.confirming = None;
            commands.push(Command::Run {
                action: action.clone(),
                agent: Box::new(agent.clone()),
                input: None,
            });
        }
    } else if armed && ui.input(|i| i.pointer.any_pressed()) && !response.contains_pointer() {
        state.confirming = None;
    }
}
