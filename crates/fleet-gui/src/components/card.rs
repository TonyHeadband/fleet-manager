use super::fleet_row::{dashed_outline, specialist_icon};
use super::Command;
use crate::theme::{self, mono, radius, sans, size, tokens, tone_color, Weight};
use crate::widgets::{
    badge, badge_items, button, field, fui, mark, small_icon_button, text_dot, BadgeTone,
    ButtonVariant, Icon,
};
use eframe::egui::{
    self, pos2, vec2, Align, Color32, CornerRadius, CursorIcon, Label, Layout, Margin, Rangef,
    Rect, RichText, Sense, Shape, Stroke, Ui, UiBuilder, WidgetInfo,
};
use fleet_api::{eval, ActionDef, Agent, FieldDef, FieldKind, Status, Tone, UiSchema};
use fleet_client::config::FleetDisplay;
use fleet_client::schema::{
    column_for, column_for_status, field_value, visible_actions, FieldValue, Group,
};

/// How long a destructive action stays armed for its second click.
const CONFIRM_FOR: f64 = 4.0;

/// Per-card UI state that must survive between frames: a pending confirmation of a destructive action, and
/// whether a task card shows its fleet.
pub struct CardState {
    /// The armed action's id and when it was armed (egui's input time).
    confirming: Option<(String, f64)>,
    pub fleet_open: bool,
}

impl Default for CardState {
    fn default() -> CardState {
        CardState {
            confirming: None,
            fleet_open: true,
        }
    }
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
    /// A task card's fleet tally and its fold toggle.
    pub const TALLY: f32 = 17.0;
    /// A full fleet card's `↳ <task>` line.
    pub const PARENT: f32 = 15.0;
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
        ("goal", _) => Slot::Title,
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

/// What a card is besides an agent's card: a task's lead, or a fleet agent drawn in full.
#[derive(Clone, Copy)]
enum Kind<'g, 'a> {
    Solo,
    Lead {
        group: &'g Group<'a>,
        display: FleetDisplay,
    },
    Fleet {
        task: &'g str,
        stuck: bool,
    },
}

/// The status edge's width: wide enough to read at a glance.
const EDGE: f32 = 4.0;
/// How a card's status edge moves: nothing, a visor band at 0..1 down its height, or a pulse of 0..1 strength.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EdgeMotion {
    Still,
    Visor(f32),
    Pulse(f32),
}

/// The edge's motion: a visor while the agent is working and not paused (a paused agent is doing nothing), a pulse
/// while the card's `tone` is Attention. Asking the clock is what keeps the window repainting, so a still edge,
/// an unfocused window or the Animations setting off all cost nothing.
pub(crate) fn edge_motion(ui: &Ui, agent: &Agent, tone: Tone) -> EdgeMotion {
    if agent.status == Status::Working && !agent.paused {
        theme::visor(ui, 1.3).map_or(EdgeMotion::Still, EdgeMotion::Visor)
    } else if tone == Tone::Attention {
        theme::pulse(ui, 1.8).map_or(EdgeMotion::Still, EdgeMotion::Pulse)
    } else {
        EdgeMotion::Still
    }
}

/// Paints the card's left edge in its status `color`, over the frame's left side. A visor adds a light band a
/// quarter of the edge's height, centred at its position down the edge, with a glow beside it in dark; a pulse adds
/// a glow along the whole edge, as strong as the pulse. `round` matches the frame's corner radius.
pub(crate) fn status_edge(ui: &Ui, rect: Rect, color: Color32, motion: EdgeMotion, round: u8) {
    let dark = ui.visuals().dark_mode;
    let painter = ui.painter();
    let edge = Rect::from_min_max(rect.min, pos2(rect.left() + EDGE, rect.bottom()));
    let corners = |top: bool, bottom: bool| CornerRadius {
        nw: if top { round } else { 0 },
        sw: if bottom { round } else { 0 },
        ne: 0,
        se: 0,
    };
    // Light cast onto the card, to the right of `span` along the edge; widest and faintest first.
    let cast = |span: Rangef, layers: [Color32; 3], scale: f32| {
        for (layer, reach) in layers.into_iter().zip([6.0, 4.0, 2.0]) {
            let layer = layer.gamma_multiply(scale);
            let strip = Rect::from_x_y_ranges(edge.right()..=edge.right() + reach, span);
            painter.rect_filled(strip, 0.0, layer);
        }
    };
    match motion {
        EdgeMotion::Pulse(v) if v > 0.0 => cast(
            Rangef::new(edge.top() + round as f32, edge.bottom() - round as f32),
            theme::glow(color, dark),
            v.clamp(0.0, 1.0),
        ),
        _ => {}
    }
    painter.rect_filled(edge, corners(true, true), color);
    let EdgeMotion::Visor(pos) = motion else {
        return;
    };
    let band = edge.height() / 4.0;
    let centre = edge.top() + pos.clamp(0.0, 1.0) * edge.height();
    let (y0, y1) = (
        (centre - band / 2.0).max(edge.top()),
        (centre + band / 2.0).min(edge.bottom()),
    );
    if y1 <= y0 {
        return;
    }
    if dark {
        cast(
            Rangef::new(y0, y1),
            theme::glow(tone_color(Tone::Active, true), true),
            1.0,
        );
    }
    let lit = Rect::from_min_max(pos2(edge.left(), y0), pos2(edge.right(), y1));
    painter.rect_filled(
        lit,
        corners(y0 <= edge.top(), y1 >= edge.bottom()),
        theme::visor_band(dark),
    );
}

/// The lead's badge when the group's column comes from somewhere else than the lead's own status.
fn lead_badge(schema: &UiSchema, group: &Group<'_>) -> Option<(Option<Icon>, String, BadgeTone)> {
    if group.lead_down() {
        return Some((
            None,
            "Orchestrator stopped · Restart?".into(),
            BadgeTone::Attention,
        ));
    }
    let stuck = group.stuck()?;
    let reason = schema
        .card_fields
        .iter()
        .find(|f| f.key == "blocked_reason")
        .and_then(|f| badge_items(f, stuck).into_iter().next())
        .map_or_else(|| "Needs you".to_string(), |(text, _)| text);
    Some((
        Some(specialist_icon(stuck)),
        format!("{} · {reason}", stuck.goal),
        BadgeTone::Attention,
    ))
}

/// One dot and count per status present in the fleet, most urgent first (the roll-up order).
fn tally(ui: &mut Ui, schema: &UiSchema, fleet: &[&Agent]) {
    let dark = ui.visuals().dark_mode;
    let p = tokens(dark);
    let font = sans(ui, size::SMALL, Weight::Regular);
    for status in [
        Status::NeedsYou,
        Status::Working,
        Status::Queued,
        Status::Review,
        Status::Stopped,
    ] {
        let n = fleet
            .iter()
            .filter(|a| fleet_api::rollup([a.status]) == status)
            .count();
        let Some(col) = column_for_status(schema, status) else {
            continue;
        };
        if n == 0 {
            continue;
        }
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let label = format!("{n} {}", col.title);
            let dot = text_dot(ui, tone_color(col.tone, dark), 6.0, &font);
            dot.widget_info(|| WidgetInfo::labeled(egui::accesskit::Role::Image, true, &label));
            dot.on_hover_text(&col.title);
            ui.label(
                RichText::new(n.to_string())
                    .font(font.clone())
                    .color(p.text_low),
            );
        });
        ui.add_space(4.0);
    }
}

/// `▾ N fleet` (open) or `▸ N fleet`; the triangle is painted, as no bundled font carries it. True when clicked.
fn fold_toggle(ui: &mut Ui, n: usize, open: bool) -> bool {
    let p = tokens(ui.visuals().dark_mode);
    let font = sans(ui, size::SMALL, Weight::Regular);
    let text = format!("{n} fleet");
    let galley = ui.painter().layout_no_wrap(text.clone(), font, p.text_low);
    let (rect, response) = ui.allocate_exact_size(
        vec2(galley.size().x + 12.0, row_height::TALLY),
        Sense::click(),
    );
    let ink = if response.hovered() {
        p.text_high
    } else {
        p.text_low
    };
    let c = pos2(rect.left() + 4.0, rect.center().y);
    let points = if open {
        vec![
            c + vec2(-3.5, -2.0),
            c + vec2(3.5, -2.0),
            c + vec2(0.0, 2.5),
        ]
    } else {
        vec![
            c + vec2(-2.0, -3.5),
            c + vec2(2.5, 0.0),
            c + vec2(-2.0, 3.5),
        ]
    };
    ui.painter()
        .add(Shape::convex_polygon(points, ink, Stroke::NONE));
    ui.painter().galley(
        pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
    let label = format!("{} {text}", if open { "▾" } else { "▸" });
    response.widget_info(|| WidgetInfo::labeled(egui::accesskit::Role::Button, true, &label));
    response.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

/// The left half of the actions row: the agent's persona, cut with "…". Draws nothing until personas exist.
fn persona_slot(ui: &mut Ui, persona: Option<&str>) {
    let Some(name) = persona else {
        return;
    };
    let p = tokens(ui.visuals().dark_mode);
    fui::mono_label_truncated(ui, name, p.text_low).on_hover_text(name);
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
    draw(ui, schema, agent, now, state, selected, persona, Kind::Solo)
}

/// An orchestrated task's card: the lead's card with a strong left edge, the orchestrator mark, and a row with
/// the fleet's tally and the toggle that folds it (`state.fleet_open`). A fleet agent that needs you names itself
/// in the badge and unfolds the group.
#[allow(clippy::too_many_arguments)]
pub fn task_card(
    ui: &mut Ui,
    schema: &UiSchema,
    group: &Group<'_>,
    now: i64,
    state: &mut CardState,
    selected: bool,
    persona: Option<&str>,
    display: FleetDisplay,
) -> Vec<Command> {
    if group.stuck().is_some() {
        state.fleet_open = true;
    }
    let kind = Kind::Lead { group, display };
    draw(ui, schema, group.lead, now, state, selected, persona, kind)
}

/// A fleet agent drawn in full (the board's Show): its card with a dashed outline and a `↳ <task>` line, its
/// specialist where a solo card has its persona.
#[allow(clippy::too_many_arguments)]
pub fn fleet_card(
    ui: &mut Ui,
    schema: &UiSchema,
    agent: &Agent,
    task: &str,
    now: i64,
    state: &mut CardState,
    selected: bool,
) -> Vec<Command> {
    let stuck = agent.status == Status::NeedsYou;
    let role = agent.role.as_deref().or(Some("specialist"));
    let kind = Kind::Fleet { task, stuck };
    draw(ui, schema, agent, now, state, selected, role, kind)
}

#[allow(clippy::too_many_arguments)]
fn draw(
    ui: &mut Ui,
    schema: &UiSchema,
    agent: &Agent,
    now: i64,
    state: &mut CardState,
    selected: bool,
    persona: Option<&str>,
    kind: Kind<'_, '_>,
) -> Vec<Command> {
    let mut commands = Vec::new();
    let dark = ui.visuals().dark_mode;
    let p = tokens(dark);
    // A task card is coloured by the column its group sits in, so a stuck fleet agent reads as Needs you.
    let column = match kind {
        Kind::Lead { group, .. } => column_for_status(schema, group.status),
        _ => column_for(schema, agent),
    };
    let tone = column.map_or(Tone::Neutral, |c| c.tone);
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
    let lead = match kind {
        Kind::Lead { group, .. } => lead_badge(schema, group),
        _ => None,
    };
    let state_badge: Option<(Option<Icon>, String, BadgeTone, &str)> = match lead {
        Some((icon, text, tone)) => Some((icon, text, tone, "Task")),
        None => in_slot(Slot::State)
            .min_by_key(|f| state_rank(&f.key))
            .and_then(|f| {
                badge_items(f, agent)
                    .into_iter()
                    .next()
                    .map(|(text, tone)| (None, text, tone, f.label.as_str()))
            }),
    };
    let ltr = Layout::left_to_right(Align::Center);
    let rtl = Layout::right_to_left(Align::Center);
    ui.push_id(&agent.id.0, |ui| {
        let dashed = matches!(kind, Kind::Fleet { .. });
        let frame = egui::Frame::new()
            .fill(p.surface)
            // Same width either way, so a full fleet card lays out like any card; the dashes are painted after.
            .stroke(Stroke::new(
                STROKE,
                if dashed {
                    egui::Color32::TRANSPARENT
                } else {
                    p.border
                },
            ))
            .corner_radius(radius::CARD)
            .inner_margin(Margin::symmetric(9, MARGIN_Y));
        // The Ui itself senses the click and registers before its contents, so the labels and controls stay on
        // top and keep their own clicks and hover tooltips; an overlay added afterwards would swallow them. It
        // wraps the whole frame, so the margins, the actions row and the space beside the icons open the panel too.
        let body = ui
            .scope_builder(
                UiBuilder::new().id_salt("open").sense(Sense::click()),
                |ui| {
                    let framed = frame.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing = vec2(6.0, ROW_GAP);
                        // Labels must not swallow the click that opens the preview.
                        ui.style_mut().interaction.selectable_labels = false;
                        if let Kind::Fleet { task, .. } = kind {
                            row(ui, row_height::PARENT, ltr, |ui| {
                                let text = format!("↳ {task}");
                                let rich = RichText::new(&text)
                                    .font(sans(ui, size::SMALL, Weight::Regular))
                                    .color(p.text_low);
                                ui.add(Label::new(rich).truncate().show_tooltip_when_elided(false))
                                    .on_hover_text(&text);
                            });
                        }
                        // Right to left first, so what sits at the right keeps its room and the left text is cut.
                        row(ui, row_height::TITLE, rtl, |ui| {
                            for f in in_slot(Slot::Duration) {
                                field(ui, f, agent, now);
                            }
                            ui.with_layout(ltr, |ui| {
                                match kind {
                                    Kind::Lead { .. } => {
                                        mark(ui, Icon::Orchestrator, "Orchestrator");
                                    }
                                    Kind::Fleet { .. } => {
                                        mark(ui, specialist_icon(agent), "Specialist");
                                    }
                                    Kind::Solo => {}
                                }
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
                                if let Some((icon, text, tone, tooltip)) = &state_badge {
                                    if let Some(icon) = icon {
                                        mark(ui, *icon, "Specialist");
                                    }
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
                            for f in in_slot(Slot::Activity) {
                                field(ui, f, agent, now);
                            }
                        });
                        if let Kind::Lead { group, display } = kind {
                            row(ui, row_height::TALLY, rtl, |ui| {
                                if display != FleetDisplay::Hide
                                    && !group.fleet.is_empty()
                                    && fold_toggle(ui, group.fleet.len(), state.fleet_open)
                                {
                                    state.fleet_open = !state.fleet_open;
                                }
                                ui.with_layout(ltr, |ui| tally(ui, schema, &group.fleet));
                            });
                        }
                        let lead_of = match kind {
                            Kind::Lead { group, .. } => Some(group),
                            _ => None,
                        };
                        // Input actions (Answer) live in the side panel's reply box, not on the card. A task
                        // card's Stop is the whole task's, drawn below.
                        let actions: Vec<&ActionDef> = visible_actions(schema, agent)
                            .into_iter()
                            .filter(|a| a.input.is_none())
                            .filter(|a| lead_of.is_none() || a.id != "stop")
                            .collect();
                        row(ui, row_height::ACTIONS, ltr, |ui| {
                            let (left, right) = ui.max_rect().split_left_right_at_fraction(0.5);
                            ui.scope_builder(UiBuilder::new().max_rect(left).layout(ltr), |ui| {
                                persona_slot(ui, persona)
                            });
                            ui.scope_builder(UiBuilder::new().max_rect(right).layout(rtl), |ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                if let Some(group) = lead_of {
                                    task_controls(ui, group, &mut commands);
                                }
                                // Right to left: the schema's last action (Stop) ends the row.
                                for action in actions.iter().rev() {
                                    control(ui, action, agent, state, &mut commands);
                                }
                                // Not from the schema: the Tasks view exists only in this client, so the
                                // daemon cannot offer it.
                                if agent.status == Status::Stopped
                                    && small_icon_button(ui, Icon::Info, "Details", false).clicked()
                                {
                                    commands.push(Command::Details(agent.id.0.clone()));
                                }
                            });
                        });
                    });
                    framed.response.rect
                },
            )
            .response;
        let r = body.rect;
        if let Kind::Lead { group, .. } = kind {
            let label = format!("{}, {} fleet agents", agent.goal, group.fleet.len());
            body.widget_info(|| WidgetInfo::labeled(egui::accesskit::Role::Group, true, &label));
        }
        if body.clicked() {
            commands.push(Command::Open(agent.id.0.clone()));
        }
        if selected {
            fui::selection_fade(ui.painter(), r, p.fill, dark, radius::CARD as f32);
            fui::corner_brackets(ui.painter(), r, p.fill);
        }
        // Last, so the edge stays on top of the fade.
        status_edge(
            ui,
            r,
            tone_color(tone, dark),
            edge_motion(ui, agent, tone),
            radius::CARD,
        );
        match kind {
            Kind::Fleet { stuck, .. } => {
                let ink = if stuck {
                    tone_color(Tone::Attention, dark)
                } else if selected {
                    p.fill
                } else {
                    p.dashed
                };
                dashed_outline(ui, r, ink);
            }
            Kind::Lead { .. } | Kind::Solo => {}
        }
    });
    commands
}

/// A task card's own actions, right to left: Stop, which stops the whole task after a confirm, while any of
/// its agents runs; Restart orchestrator once its orchestrator has ended.
fn task_controls(ui: &mut Ui, group: &Group<'_>, commands: &mut Vec<Command>) {
    let task = fleet_api::task_id(group.lead).0.clone();
    let fleet = group
        .fleet
        .iter()
        .filter(|a| a.status != Status::Stopped)
        .count();
    if (group.lead.status != Status::Stopped || fleet > 0)
        && small_icon_button(ui, Icon::Stop, "Stop", false).clicked()
    {
        commands.push(Command::StopTask {
            task: task.clone(),
            fleet,
        });
    }
    if group.can_restart()
        && small_icon_button(ui, Icon::Resume, "Restart orchestrator", false).clicked()
    {
        commands.push(Command::RestartOrchestrator(task));
    }
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
