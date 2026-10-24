//! The drawer: the left column on every view. Brand row, New project, the nav and the live status.
use crate::theme::{layout, size, tokens, tone_color, APP_NAME};
use crate::views::View;
use crate::widgets::{button, dot, nav_item, ButtonVariant};
use eframe::egui::{self, Align, Frame, Label, Layout, Margin, Panel, Rect, RichText, Ui};
use fleet_api::Tone;

/// What the drawer's footer reports.
#[derive(Clone, Copy, Debug)]
pub struct DrawerStatus {
    pub connected: bool,
    pub agents: usize,
    /// Agents waiting for the user (Needs you).
    pub need_you: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DrawerAction {
    NewProject,
    Show(View),
}

/// What the drawer did this frame, and where its header and footer rows landed.
pub struct Drawn {
    pub action: Option<DrawerAction>,
    pub header: Rect,
    pub footer: Rect,
}

/// Views listed above the nav's separator; Settings sits alone below it. A new view goes here.
const VIEWS: [View; 1] = [View::Kanban];

/// `3 agents · 1 needs you`; the second part only when someone waits.
pub fn agent_summary(agents: usize, need_you: usize) -> String {
    let mut s = format!("{agents} {}", if agents == 1 { "agent" } else { "agents" });
    if need_you > 0 {
        s += &format!(" · {need_you} {}", if need_you == 1 { "needs you" } else { "need you" });
    }
    s
}

/// Draws the drawer as a left panel of `ui`. Its header and footer are exactly `layout::HEADER` and
/// `layout::FOOTER` tall, like the content's, and start at the window's top and bottom, so the rows line up.
pub fn drawer(ui: &mut Ui, view: View, status: DrawerStatus) -> Drawn {
    let p = tokens(ui.visuals().dark_mode);
    let dark = ui.visuals().dark_mode;
    let mut action = None;
    let row = |margin_y: i8| Frame::new().fill(p.surface).inner_margin(Margin::symmetric(14, margin_y));
    let inner = Panel::left("drawer").exact_size(layout::DRAWER).resizable(false).frame(Frame::new().fill(p.surface)).show(ui, |ui| {
        // Each row's separator is the panel's own 1 px `border` line, as on the content's rows.
        let header = Panel::top("drawer_header").exact_size(layout::HEADER).frame(row(0)).show(ui, |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.label(RichText::new(APP_NAME).text_style(egui::TextStyle::Heading).color(p.text_high));
            });
        });
        let footer = Panel::bottom("drawer_footer").exact_size(layout::FOOTER).frame(row(0)).show(ui, |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let (colour, text) = if status.connected { (tone_color(Tone::Done, dark), "live") } else { (tone_color(Tone::Attention, dark), "reconnecting…") };
                dot(ui, colour, 8.0);
                ui.label(RichText::new(text).size(size::SMALL).color(p.text));
                ui.add_space(4.0);
                // Cut rather than wrapped: the row's height is fixed.
                ui.add(Label::new(RichText::new(agent_summary(status.agents, status.need_you)).size(size::SMALL).color(p.text_low)).truncate());
            });
        });
        egui::CentralPanel::default().frame(Frame::new().fill(p.surface).inner_margin(Margin::same(12))).show(ui, |ui| {
            ui.with_layout(Layout::top_down_justified(Align::Center), |ui| {
                if button(ui, "New project", ButtonVariant::Primary).clicked() {
                    action = Some(DrawerAction::NewProject);
                }
            });
            ui.add_space(10.0);
            for v in VIEWS {
                if nav_item(ui, v.label(), v == view).clicked() && v != view {
                    action = Some(DrawerAction::Show(v));
                }
            }
            ui.separator();
            if nav_item(ui, View::Settings.label(), view == View::Settings).clicked() && view != View::Settings {
                action = Some(DrawerAction::Show(View::Settings));
            }
        });
        (header.response.rect, footer.response.rect)
    });
    let (header, footer) = inner.inner;
    Drawn { action, header, footer }
}
