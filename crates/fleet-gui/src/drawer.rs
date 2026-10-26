//! The drawer: the left column on every view. Brand row, the active persona, New task, the nav and the live
//! status.
use crate::components::persona_choice;
use crate::theme::{layout, size, tokens, tone_color, APP_NAME};
use crate::views::View;
use crate::widgets::{button, dot, list_row, nav_item, ButtonVariant};
use eframe::egui::{
    self, accesskit::Role, Align, Frame, Label, Layout, Margin, Panel, Rect, RichText, Ui,
    WidgetInfo,
};
use fleet_api::{Persona, Product, Tone};

/// What the drawer's footer reports.
#[derive(Clone, Copy, Debug)]
pub struct DrawerStatus {
    pub connected: bool,
    pub agents: usize,
    /// Agents waiting for the user (Needs you).
    pub need_you: usize,
    /// Why New task cannot run now, if it cannot.
    pub new_task_blocked: Option<&'static str>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DrawerAction {
    NewTask,
    NewProduct,
    Show(View),
    /// The switcher picked this persona id as the active one.
    SetPersona(String),
    /// The switcher picked this product id as the active one.
    SetProduct(String),
}

/// The drawer's switchers: the listed personas (it offers the ready ones) and the active persona's id, and the
/// listed products and the active product's id.
#[derive(Clone, Copy)]
pub struct Switcher<'a> {
    pub personas: &'a [Persona],
    pub active: Option<&'a str>,
    pub products: &'a [Product],
    pub product: Option<&'a str>,
}

/// What the drawer did this frame, and where its header and footer rows landed.
pub struct Drawn {
    pub action: Option<DrawerAction>,
    pub header: Rect,
    pub footer: Rect,
}

/// Views listed above the nav's separator; Management and Settings sit below it. A new view goes here.
const VIEWS: [View; 2] = [View::Kanban, View::Tasks];

/// `3 agents · 1 needs you`; the second part only when someone waits.
pub fn agent_summary(agents: usize, need_you: usize) -> String {
    let mut s = format!("{agents} {}", if agents == 1 { "agent" } else { "agents" });
    if need_you > 0 {
        s += &format!(
            " · {need_you} {}",
            if need_you == 1 {
                "needs you"
            } else {
                "need you"
            }
        );
    }
    s
}

/// Draws the drawer as a left panel of `ui`. Its header and footer are exactly `layout::HEADER` and
/// `layout::FOOTER` tall, like the content's, and start at the window's top and bottom, so the rows line up. The
/// switcher shows once a persona is ready.
pub fn drawer(ui: &mut Ui, view: View, status: DrawerStatus, switcher: Switcher<'_>) -> Drawn {
    let p = tokens(ui.visuals().dark_mode);
    let dark = ui.visuals().dark_mode;
    let mut action = None;
    let row = |margin_y: i8| {
        Frame::new()
            .fill(p.surface)
            .inner_margin(Margin::symmetric(14, margin_y))
    };
    let inner = Panel::left("drawer")
        .exact_size(layout::DRAWER)
        .resizable(false)
        .frame(Frame::new().fill(p.surface))
        .show(ui, |ui| {
            // Each row's separator is the panel's own 1 px `border` line, as on the content's rows.
            let header = Panel::top("drawer_header")
                .exact_size(layout::HEADER)
                .frame(row(0))
                .show(ui, |ui| {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.label(
                            RichText::new(APP_NAME)
                                .text_style(egui::TextStyle::Heading)
                                .color(p.text_high),
                        );
                    });
                });
            let footer = Panel::bottom("drawer_footer")
                .exact_size(layout::FOOTER)
                .frame(row(0))
                .show(ui, |ui| {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let (colour, text) = if status.connected {
                            (tone_color(Tone::Done, dark), "live")
                        } else {
                            (tone_color(Tone::Attention, dark), "reconnecting…")
                        };
                        dot(ui, colour, 8.0);
                        ui.label(RichText::new(text).size(size::SMALL).color(p.text));
                        ui.add_space(4.0);
                        // Cut rather than wrapped: the row's height is fixed.
                        ui.add(
                            Label::new(
                                RichText::new(agent_summary(status.agents, status.need_you))
                                    .size(size::SMALL)
                                    .color(p.text_low),
                            )
                            .truncate(),
                        );
                    });
                });
            egui::CentralPanel::default()
                .frame(Frame::new().fill(p.surface).inner_margin(Margin::same(12)))
                .show(ui, |ui| {
                    if switcher.personas.iter().any(|p| p.ready) {
                        ui.horizontal(|ui| {
                            // Room for the ⚠ painted after a broken login.
                            let width = ui.available_width() - 22.0;
                            if let Some(id) = persona_choice(
                                ui,
                                "Active persona",
                                switcher.personas,
                                switcher.active.unwrap_or_default(),
                                width,
                            ) {
                                action = Some(DrawerAction::SetPersona(id));
                            }
                        });
                        ui.add_space(8.0);
                    }
                    if !switcher.products.is_empty() {
                        if let Some(id) = product_choice(ui, switcher.products, switcher.product) {
                            action = Some(DrawerAction::SetProduct(id));
                        }
                        ui.add_space(8.0);
                    }
                    ui.with_layout(Layout::top_down_justified(Align::Center), |ui| {
                        if button(ui, "New product", ButtonVariant::Default).clicked() {
                            action = Some(DrawerAction::NewProduct);
                        }
                        ui.add_space(6.0);
                        let new_task = ui
                            .add_enabled_ui(status.new_task_blocked.is_none(), |ui| {
                                button(ui, "New task", ButtonVariant::Primary)
                            })
                            .inner;
                        if new_task
                            .on_disabled_hover_text(status.new_task_blocked.unwrap_or_default())
                            .clicked()
                        {
                            action = Some(DrawerAction::NewTask);
                        }
                    });
                    ui.add_space(10.0);
                    for v in VIEWS {
                        if nav_item(ui, v.label(), v == view).clicked() && v != view {
                            action = Some(DrawerAction::Show(v));
                        }
                    }
                    ui.separator();
                    for v in [View::Management, View::Settings] {
                        if nav_item(ui, v.label(), v == view).clicked() && v != view {
                            action = Some(DrawerAction::Show(v));
                        }
                    }
                });
            (header.response.rect, footer.response.rect)
        });
    let (header, footer) = inner.inner;
    Drawn {
        action,
        header,
        footer,
    }
}

/// The product dropdown; `selected` is a product id, and one not listed shows as "Pick a product". Returns the id
/// picked this frame, if another one.
fn product_choice(ui: &mut Ui, products: &[Product], selected: Option<&str>) -> Option<String> {
    let p = tokens(ui.visuals().dark_mode);
    let width = ui.available_width();
    let text = products
        .iter()
        .find(|p| Some(p.id.as_str()) == selected)
        .map_or("Pick a product", |p| p.name.as_str());
    let mut picked = None;
    let combo = egui::ComboBox::from_id_salt("product_choice")
        .selected_text(RichText::new(text).color(p.text_high))
        .width(width)
        .show_ui(ui, |ui| {
            ui.set_min_width(width);
            for product in products {
                let on = Some(product.id.as_str()) == selected;
                let row = list_row(ui, &product.name, on, |ui| {
                    ui.horizontal(|ui| {
                        // Same first-pass height rule as the persona rows.
                        ui.set_min_height(ui.spacing().interact_size.y);
                        ui.add(
                            Label::new(RichText::new(&product.name).color(p.text_high)).truncate(),
                        );
                    });
                });
                if row.clicked() && !on {
                    picked = Some(product.id.clone());
                }
            }
        });
    combo
        .response
        .widget_info(|| WidgetInfo::labeled(Role::ComboBox, ui.is_enabled(), "Active product"));
    picked
}
