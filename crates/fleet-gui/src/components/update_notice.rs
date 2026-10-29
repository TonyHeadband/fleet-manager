//! The new-version notice: a modal used only for this, when a newer release is out.
use super::modal_frame;
use crate::theme::{size, tokens};
use crate::widgets::{button, ButtonVariant};
use eframe::egui::{self, RichText, TextStyle};
use fleet_client::update::Release;

/// The modal's id, so the app can tell whether another modal is on top of it.
pub const NOTICE_ID: &str = "update_notice";

/// How the notice was closed. Either way, a ticked "Skip this version" is saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeChoice {
    /// The release page was opened.
    Download,
    /// Later, Escape or a click outside: the notice comes back at the next check.
    Later,
}

/// The notice for `release`, while a program at `current` runs. `skip` is the "Skip this version" box.
pub fn update_notice(
    ctx: &egui::Context,
    release: &Release,
    current: &str,
    skip: &mut bool,
) -> Option<NoticeChoice> {
    let p = tokens(ctx.global_style().visuals.dark_mode);
    let (mut download, mut later) = (false, false);
    let modal = egui::Modal::new(egui::Id::unique(NOTICE_ID))
        .frame(modal_frame(ctx))
        .show(ctx, |ui| {
            ui.set_max_width(560.0);
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "fleet-manager v{} is available (you have v{current})",
                        release.version
                    ))
                    .text_style(TextStyle::Heading)
                    .color(p.text_high),
                )
                .wrap(),
            );
            if !release.notes.trim().is_empty() {
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .max_height(280.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(release.notes.trim()).color(p.text))
                                .wrap(),
                        );
                    });
            }
            ui.add_space(10.0);
            ui.checkbox(skip, "Skip this version");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                download = button(ui, "Download", ButtonVariant::Primary).clicked();
                later = button(ui, "Later", ButtonVariant::Default).clicked();
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new("Download opens the release page in your browser.")
                    .size(size::SMALL)
                    .color(p.text_low),
            );
        });
    if download {
        ctx.open_url(egui::OpenUrl::new_tab(&release.page));
        Some(NoticeChoice::Download)
    } else if later || modal.should_close() {
        Some(NoticeChoice::Later)
    } else {
        None
    }
}
