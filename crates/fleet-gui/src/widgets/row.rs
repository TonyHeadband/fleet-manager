use crate::theme::{radius, tokens};
use eframe::egui::{self, Frame, Margin, Response, Sense, Shape, Ui, WidgetInfo};

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
