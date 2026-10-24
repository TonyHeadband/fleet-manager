use crate::theme::{radius, sans, size, tokens, Weight};
use eframe::egui::{self, pos2, vec2, Frame, Margin, Response, Sense, Shape, Ui, WidgetInfo};

/// A full-width clickable row of a list (the folder picker's folders and checkouts). `content` draws its inside;
/// `label` is its accessible name. Hover lifts it onto `surface`, `selected` onto `raised`.
pub fn list_row(ui: &mut Ui, label: &str, selected: bool, content: impl FnOnce(&mut Ui)) -> Response {
    let p = tokens(ui.visuals().dark_mode);
    // The fill depends on the hover state, which is only known once the content has set the row's size.
    let background = ui.painter().add(Shape::Noop);
    let inner = Frame::new().inner_margin(Margin::symmetric(8, 5)).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        // Selectable text would take the click and the hover away from the row.
        ui.style_mut().interaction.selectable_labels = false;
        content(ui);
    });
    let rect = inner.response.rect;
    let response = ui.interact(rect, inner.response.id.with("list_row"), Sense::click());
    let fill = if selected {
        Some(p.raised)
    } else if response.hovered() {
        Some(p.surface)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().set(background, Shape::rect_filled(rect, radius::CONTROL, fill));
    }
    response.widget_info(|| WidgetInfo::selected(egui::accesskit::Role::Button, ui.is_enabled(), selected, label));
    response
}

/// An entry of the drawer's nav: a full-width row on the drawer's `surface`. `selected` and hover sit on `raised`;
/// the selected one also reads as selected to assistive tech and tests.
pub fn nav_item(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let p = tokens(ui.visuals().dark_mode);
    let font = sans(ui, size::BODY, Weight::Regular);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, p.text);
    let height = galley.size().y + 2.0 * NAV_PAD_Y;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    if selected || response.hovered() {
        ui.painter().rect_filled(rect, radius::CONTROL, p.raised);
    }
    let colour = if selected || response.hovered() { p.text_high } else { p.text };
    ui.painter().galley(pos2(rect.left() + NAV_PAD_X, rect.center().y - galley.size().y / 2.0), galley, colour);
    response.widget_info(|| WidgetInfo::selected(egui::accesskit::Role::Button, ui.is_enabled(), selected, label));
    response
}

const NAV_PAD_X: f32 = 8.0;
const NAV_PAD_Y: f32 = 6.0;
