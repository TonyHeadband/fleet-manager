use eframe::egui::{accesskit::Role, Vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use fleet_api::instructions;
use fleet_gui::components::form::{self, FormValues};

#[test]
fn markdown_and_help_are_drawn() {
    let h = Harness::builder()
        .with_size(Vec2::new(800.0, 600.0))
        .build_ui_state(
            |ui, values: &mut FormValues| {
                form::fields(ui, &instructions(), values, None, &form::Sources::default());
            },
            FormValues::new(),
        );
    h.get_by_role_and_label(Role::MultilineTextInput, "Instructions for every agent");
    h.get_by_label(
        "Markdown, put at the top of each agent's CLAUDE.md under \"# Instructions for every agent\".",
    );
}
