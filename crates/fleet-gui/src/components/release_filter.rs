//! The board's release filter: one choice among All, each release still open, and the backlog.
use crate::theme::{sans, size, tokens, Weight};
use crate::widgets::list_row;
use eframe::egui::{self, accesskit::Role, RichText, Ui, WidgetInfo};
use fleet_api::Release;

/// Which tasks a release filter lets through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReleaseFilter {
    #[default]
    All,
    /// The tasks of the release with this id.
    Release(i64),
    /// The tasks in no release.
    Backlog,
}

impl ReleaseFilter {
    /// Whether a task in `release` (None: the backlog) passes.
    pub fn allows(self, release: Option<i64>) -> bool {
        match self {
            ReleaseFilter::All => true,
            ReleaseFilter::Release(id) => release == Some(id),
            ReleaseFilter::Backlog => release.is_none(),
        }
    }

    /// A filter on a release that is no longer offered (released, deleted, another product) falls back to All.
    pub fn kept(self, releases: &[Release]) -> ReleaseFilter {
        match self {
            ReleaseFilter::Release(id) if !offered(releases).any(|r| r.id == id) => {
                ReleaseFilter::All
            }
            f => f,
        }
    }
}

fn offered(releases: &[Release]) -> impl Iterator<Item = &Release> {
    releases.iter().filter(|r| r.released_at.is_none())
}

fn summary(releases: &[Release], current: ReleaseFilter) -> String {
    match current {
        ReleaseFilter::All => "All releases".into(),
        ReleaseFilter::Backlog => "Backlog".into(),
        ReleaseFilter::Release(id) => releases
            .iter()
            .find(|r| r.id == id)
            .map_or_else(|| "Release".into(), |r| r.version.clone()),
    }
}

/// A dropdown over All releases, the unreleased ones of `releases` and Backlog. Returns true when this frame
/// changed `current`.
pub fn release_filter(ui: &mut Ui, releases: &[Release], current: &mut ReleaseFilter) -> bool {
    let p = tokens(ui.visuals().dark_mode);
    let label = RichText::new("Release")
        .font(sans(ui, size::SMALL, Weight::Regular))
        .color(p.text_low);
    let mut picked = None;
    let combo = egui::ComboBox::from_id_salt("release_filter")
        .selected_text((label, summary(releases, *current)))
        .show_ui(ui, |ui| {
            ui.set_min_width(160.0);
            let mut row = |ui: &mut Ui, f: ReleaseFilter, text: &str| {
                let on = *current == f;
                if list_row(ui, text, on, |ui| {
                    ui.label(RichText::new(text).color(p.text_high));
                })
                .clicked()
                    && !on
                {
                    picked = Some(f);
                }
            };
            row(ui, ReleaseFilter::All, "All releases");
            for r in offered(releases) {
                row(ui, ReleaseFilter::Release(r.id), &r.version);
            }
            row(ui, ReleaseFilter::Backlog, "Backlog");
        });
    combo
        .response
        .widget_info(|| WidgetInfo::labeled(Role::ComboBox, ui.is_enabled(), "Release filter"));
    if let Some(f) = picked {
        *current = f;
    }
    picked.is_some()
}
