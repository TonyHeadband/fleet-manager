//! Management → Workflow: what `.fleet/workflow.yml` is. Read-only until the editor exists.
use super::personas::note;
use crate::theme::{mono, size, tokens};
use eframe::egui::{self, Frame, Margin, RichText, ScrollArea, TextStyle, Ui};

/// The two-step example the paragraph above it describes.
const EXAMPLE: &str = "\
ignore: [docs/**, \"**/*.md\"]
steps:
  fmt:
    run: cargo fmt --all --check
  daemon:
    run: cargo test -p fleet-daemon
    paths: [crates/fleet-daemon/**]";

pub(super) fn workflow(ui: &mut Ui) {
    let p = tokens(ui.visuals().dark_mode);
    Frame::new()
        .inner_margin(Margin::symmetric(crate::theme::layout::PAGE_X as i8, crate::theme::layout::PAGE_Y as i8))
        .show(ui, |ui| {
            ScrollArea::vertical().show(ui, |ui| {
                ui.set_max_width(crate::theme::management::WORKFLOW_WIDTH);
                ui.label(
                    RichText::new("Workflow")
                        .text_style(TextStyle::Heading)
                        .color(p.text_high),
                );
                ui.add_space(crate::theme::management::HEAD_GAP);
                ui.add(
                    egui::Label::new(
                        RichText::new("Each repo lists its checks in .fleet/workflow.yml on its base branch. Before an orchestrator opens a pull request, the app runs the steps whose paths the task changed (every step when a changed file is not covered). Agents cannot change it: it is read from the remote.")
                            .color(p.text),
                    )
                    .wrap(),
                );
                ui.add_space(crate::theme::management::PARA_GAP);
                Frame::new()
                    .fill(p.surface)
                    .inner_margin(Margin::same(crate::theme::management::BOX_MARGIN as i8))
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(EXAMPLE).font(mono(size::CODE)).color(p.text),
                            )
                            .wrap(),
                        );
                    });
                ui.add_space(crate::theme::management::PARA_GAP);
                note(
                    ui,
                    "An editor for this file is coming. For now, add the file to the repo and push it.",
                );
            });
        });
}
