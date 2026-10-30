//! The persona dropdowns: the board's filter, and the one-persona choice of the drawer and the New task dialog.
use crate::theme::{mono, size, tokens};
use crate::widgets::{fui, list_row, warn_mark};
use eframe::egui::{
    self, accesskit::Role, Align, Label, Layout, PopupCloseBehavior, RichText, Ui, WidgetInfo,
};
use fleet_api::Persona;
use std::collections::HashMap;

/// What a persona id is shown as: its name, the id itself once the persona is deleted, "no persona" for a task
/// that predates personas. `names` maps the listed personas' ids to their names.
pub fn persona_label(names: &HashMap<String, String>, id: &str) -> String {
    if id.is_empty() {
        return "no persona".to_string();
    }
    names.get(id).cloned().unwrap_or_else(|| id.to_string())
}

/// What each of `ids` is shown as, by id: its `persona_label`, or "Name (id)" where two or more of them would read
/// the same, so equal names, and a deleted id that reads like a name, stay apart.
pub fn persona_labels<'a>(
    names: &HashMap<String, String>,
    ids: impl IntoIterator<Item = &'a str>,
) -> HashMap<String, String> {
    let mut base: Vec<(&str, String)> = Vec::new();
    for id in ids {
        if !base.iter().any(|(b, _)| *b == id) {
            base.push((id, persona_label(names, id)));
        }
    }
    base.iter()
        .map(|(id, label)| {
            let shared = base.iter().filter(|(_, l)| l == label).count() > 1;
            let shown = if shared && !id.is_empty() {
                format!("{label} ({id})")
            } else {
                label.clone()
            };
            (id.to_string(), shown)
        })
        .collect()
}

/// Ready, and one of its logins was found broken since (expired, revoked).
pub fn login_broken(p: &Persona) -> bool {
    p.ready && [&p.claude, &p.github].into_iter().flatten().any(|s| !s.ok)
}

/// What the closed dropdown shows after "Persona"; `selected` holds ids of `rows`.
fn filter_summary(rows: &[(String, String, usize)], selected: &[String]) -> String {
    match selected {
        [] => "All personas".to_string(),
        [one] => rows
            .iter()
            .find(|(id, _, _)| id == one)
            .map_or_else(|| one.clone(), |(_, label, _)| label.clone()),
        many => format!("{} personas", many.len()),
    }
}

const MENU_MIN_WIDTH: f32 = crate::theme::layout::MENU_MIN;

/// A multi-select dropdown over `(persona id, label, agent count)`, like `repo_filter`. `selected` holds ids, so a
/// rename keeps the selection; an empty one means no filter. Returns true when this frame changed it; personas are
/// kept in the order they were clicked.
pub fn persona_filter(
    ui: &mut Ui,
    personas: &[(String, String, usize)],
    selected: &mut Vec<String>,
) -> bool {
    let p = tokens(ui.visuals().dark_mode);
    let label = fui::mono_text("Persona", p.text_low);
    let summary = filter_summary(personas, selected);
    let value = format!("Persona {summary}");
    let mut changed = false;
    let combo = egui::ComboBox::from_id_salt("persona_filter")
        .selected_text((label, summary))
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            ui.set_min_width(MENU_MIN_WIDTH);
            let mut all = selected.is_empty();
            if ui.checkbox(&mut all, "All personas").clicked() {
                changed = !selected.is_empty();
                selected.clear();
            }
            ui.separator();
            for (id, label, count) in personas {
                let mut on = selected.contains(id);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut on, label).clicked() {
                        if on {
                            selected.push(id.clone());
                        } else {
                            selected.retain(|s| s != id);
                        }
                        changed = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(count.to_string())
                                .font(mono(size::META))
                                .color(p.text_low),
                        );
                    });
                });
            }
        });
    combo
        .response
        .widget_info(|| fui::combo_info(ui.is_enabled(), "Persona filter", value.clone()));
    changed
}

/// The name a choice row goes by: the ⚠ beside it is painted, so the name carries it for assistive tech and tests.
fn row_label(p: &Persona, shown: &str) -> String {
    if login_broken(p) {
        format!("{shown} ⚠")
    } else {
        shown.to_string()
    }
}

/// One persona out of the ready ones in `personas`, named `label` for assistive tech, with ⚠ beside a broken
/// login in the menu and after the dropdown. `selected` is a persona id; one not offered shows as "Pick a persona".
/// Returns the id picked this frame, if another one.
pub fn persona_choice(
    ui: &mut Ui,
    label: &str,
    personas: &[Persona],
    selected: &str,
    width: f32,
) -> Option<String> {
    let p = tokens(ui.visuals().dark_mode);
    let ready: Vec<&Persona> = personas.iter().filter(|p| p.ready).collect();
    let names = ready
        .iter()
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect();
    let labels = persona_labels(&names, ready.iter().map(|p| p.id.as_str()));
    let shown = |p: &Persona| labels.get(&p.id).cloned().unwrap_or_else(|| p.name.clone());
    let current = ready.iter().find(|p| p.id == selected);
    let text = current.map_or_else(|| "Pick a persona".to_string(), |c| shown(c));
    let mut picked = None;
    let combo = egui::ComboBox::from_id_salt(("persona_choice", label))
        .selected_text(RichText::new(text).color(p.text_high))
        .width(width)
        .show_ui(ui, |ui| {
            ui.set_min_width(width);
            for persona in &ready {
                let on = persona.id == selected;
                let name = shown(persona);
                let row = list_row(ui, &row_label(persona, &name), on, |ui| {
                    ui.horizontal(|ui| {
                        // A popup's first (sizing) pass lays a horizontal out at its content's height, the rest at
                        // `interact_size.y`; the popup would keep the shorter first measure and clip its last rows.
                        ui.set_min_height(ui.spacing().interact_size.y);
                        ui.add(Label::new(RichText::new(&name).color(p.text_high)).truncate());
                        if login_broken(persona) {
                            warn_mark(ui).on_hover_text("A login needs attention");
                        }
                    });
                });
                if row.clicked() && !on {
                    picked = Some(persona.id.clone());
                }
            }
        });
    combo
        .response
        .widget_info(|| WidgetInfo::labeled(Role::ComboBox, ui.is_enabled(), label));
    if current.is_some_and(|c| login_broken(c)) {
        warn_mark(ui).on_hover_text("A login needs attention: Management → Personas");
    }
    picked
}
