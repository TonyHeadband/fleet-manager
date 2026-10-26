use super::card::{status_edge, working_sweep};
use super::Command;
use crate::theme::{radius, sans, size, tokens, tone_color, Weight};
use crate::widgets::{mark, warn_mark, Icon};
use eframe::egui::{
    self, vec2, Align, Color32, Label, Layout, Rect, RichText, Sense, Shape, Stroke, Ui, UiBuilder,
    WidgetInfo,
};
use fleet_api::{Agent, AgentKind, Status, Tone, UiSchema};
use fleet_client::schema::{column_for_status, format_duration};

/// A compact fleet row's height.
pub const FLEET_ROW_HEIGHT: f32 = 26.0;

/// The mark a fleet agent is drawn with: what kind of work its specialist does. An earlier orchestrator of the
/// project, kept as an ended row, keeps the orchestrator's.
pub fn specialist_icon(agent: &Agent) -> Icon {
    if agent.kind == AgentKind::Orchestrator {
        return Icon::Orchestrator;
    }
    match agent.role.as_deref() {
        Some("rust" | "typescript-react") => Icon::Code,
        Some("reviewer") => Icon::Eye,
        Some("test-writer") => Icon::Flask,
        _ => Icon::Specialist,
    }
}

/// A status as a screen reader says it; the board itself only shows it as a column or a dot.
fn status_word(s: Status) -> &'static str {
    match s {
        Status::NeedsYou => "needs you",
        Status::Working | Status::Talking => "working",
        Status::Queued => "queued",
        Status::Review => "review",
        Status::Stopped => "stopped",
    }
}

/// A 1 px dashed outline just inside `rect`: what tells a fleet agent, which ends with its step, from a project.
pub(crate) fn dashed_outline(ui: &Ui, rect: Rect, ink: Color32) {
    let r = rect.shrink(0.5);
    let path = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    ui.painter()
        .extend(Shape::dashed_line(&path, Stroke::new(1.0, ink), 3.0, 2.0));
}

/// One fleet agent in one row: specialist mark, title, then ⚠ when it needs you (`stuck`), a PR mark
/// when the project has a PR open (`pr`), and its time in status. Clicking it opens the agent like a card.
#[allow(clippy::too_many_arguments)]
pub fn fleet_row(
    ui: &mut Ui,
    schema: &UiSchema,
    agent: &Agent,
    now: i64,
    selected: bool,
    stuck: bool,
    pr: bool,
) -> Option<Command> {
    let dark = ui.visuals().dark_mode;
    let p = tokens(dark);
    let tone = column_for_status(schema, agent.status).map_or(Tone::Neutral, |c| c.tone);
    let rect = Rect::from_min_size(
        ui.cursor().min,
        vec2(ui.available_width(), FLEET_ROW_HEIGHT),
    );
    let ltr = Layout::left_to_right(Align::Center);
    // Reserved before the contents so the fill sits under them; its size is known only once they are laid out.
    let background = ui.painter().add(Shape::Noop);
    let response = ui
        .scope_builder(
            UiBuilder::new()
                .id_salt(("fleet_row", &agent.id.0))
                .max_rect(rect)
                .layout(ltr)
                .sense(Sense::click()),
            |ui| {
                ui.set_min_size(rect.size());
                ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                // Labels must not swallow the click that opens the agent.
                ui.style_mut().interaction.selectable_labels = false;
                ui.spacing_mut().item_spacing.x = 6.0;
                // Clear of the 4 px status edge painted over the row's left end.
                ui.add_space(4.0);
                let role = agent.role.as_deref().unwrap_or("specialist");
                mark(ui, specialist_icon(agent), role);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(1.0);
                    let font = sans(ui, size::SMALL, Weight::Regular);
                    ui.label(
                        RichText::new(format_duration(now - agent.status_since))
                            .font(font)
                            .color(p.text_low),
                    );
                    if pr {
                        mark(ui, Icon::PullRequest, "PR open");
                    }
                    if stuck {
                        warn_mark(ui).on_hover_text("Needs you");
                    }
                    ui.with_layout(ltr, |ui| {
                        let title = RichText::new(&agent.goal)
                            .font(sans(ui, size::SMALL, Weight::Medium))
                            .color(p.text_high);
                        ui.add(Label::new(title).truncate().show_tooltip_when_elided(false))
                            .on_hover_text(&agent.goal);
                    });
                });
            },
        )
        .response;
    let label = format!(
        "{} {}, {}",
        agent.role.as_deref().unwrap_or("specialist"),
        agent.goal,
        status_word(agent.status)
    );
    response.widget_info(|| WidgetInfo::labeled(egui::accesskit::Role::Button, true, &label));
    let ink = if stuck {
        tone_color(Tone::Attention, dark)
    } else if selected {
        p.text_low
    } else {
        p.dashed
    };
    ui.painter().set(
        background,
        Shape::rect_filled(response.rect, radius::CONTROL, p.surface),
    );
    dashed_outline(ui, response.rect, ink);
    status_edge(
        ui,
        response.rect,
        tone_color(tone, dark),
        working_sweep(ui, agent),
        radius::CONTROL,
    );
    response
        .clicked()
        .then(|| Command::Open(agent.id.0.clone()))
}
