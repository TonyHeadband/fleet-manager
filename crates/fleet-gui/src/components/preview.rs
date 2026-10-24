use crate::theme::{palette, tone_color};
use crate::widgets::{button, tabs, ButtonVariant};
use eframe::egui::{self, RichText, ScrollArea, TextStyle, Ui};
use fleet_api::{DiffReport, Tone};

/// A longer line is cut for display so one pathological line can never stall a frame.
const MAX_LINE_CHARS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewTab {
    #[default]
    Terminal,
    Diff,
}

#[derive(Default)]
pub enum DiffView {
    #[default]
    NotLoaded,
    Loading,
    Loaded(DiffReport),
    Failed(String),
}

#[derive(Default)]
pub struct PreviewState {
    pub agent_id: String,
    pub title: String,
    pub tab: PreviewTab,
    pub text: String,
    pub closed: bool,
    pub diff: DiffView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewAction {
    None,
    /// The app should fetch the diff and put the result in `PreviewState::diff`.
    LoadDiff,
    Close,
}

/// The side panel: the agent's terminal text (read-only) and its diff. It draws only what is in `st`.
pub fn preview_pane(ui: &mut Ui, st: &mut PreviewState) -> PreviewAction {
    let mut action = PreviewAction::None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(&st.title).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if button(ui, "Close", ButtonVariant::Default).clicked() {
                action = PreviewAction::Close;
            }
        });
    });
    ui.horizontal(|ui| {
        if let Some(t) = tabs(ui, &[(PreviewTab::Terminal, "Terminal"), (PreviewTab::Diff, "Diff")], st.tab) {
            st.tab = t;
            if t == PreviewTab::Diff && matches!(st.diff, DiffView::NotLoaded) {
                st.diff = DiffView::Loading;
                if action == PreviewAction::None {
                    action = PreviewAction::LoadDiff;
                }
            }
        }
        if st.tab == PreviewTab::Diff && button(ui, "Refresh", ButtonVariant::Default).clicked() {
            st.diff = DiffView::Loading;
            if action == PreviewAction::None {
                action = PreviewAction::LoadDiff;
            }
        }
    });
    ui.separator();
    match st.tab {
        PreviewTab::Terminal => {
            let rows: Vec<Row> = st.text.lines().map(Row::Plain).collect();
            rows_view(ui, "terminal", &rows);
            if st.closed {
                ui.label(RichText::new("Session ended").color(palette(ui.visuals().dark_mode).muted));
            }
        }
        PreviewTab::Diff => match &st.diff {
            DiffView::NotLoaded | DiffView::Loading => {
                ui.label("Loading…");
            }
            DiffView::Failed(message) => {
                ui.label(RichText::new(message).color(palette(ui.visuals().dark_mode).danger));
            }
            DiffView::Loaded(report) => {
                let mut rows: Vec<Row> = Vec::new();
                for w in &report.workspaces {
                    rows.push(Row::Heading(&w.repo));
                    if w.diff.is_empty() {
                        rows.push(Row::Note("No changes"));
                    }
                    rows.extend(w.diff.lines().map(Row::Diff));
                    if w.truncated {
                        rows.push(Row::Note("The diff was cut at the size limit."));
                    }
                }
                rows_view(ui, "diff", &rows);
            }
        },
    }
    action
}

enum Row<'a> {
    Plain(&'a str),
    Diff(&'a str),
    Heading(&'a str),
    Note(&'a str),
}

fn clip(line: &str) -> &str {
    match line.char_indices().nth(MAX_LINE_CHARS) {
        Some((i, _)) => &line[..i],
        None => line,
    }
}

/// Draws only the rows in view, so a megabyte of text costs one screen of layout per frame.
fn rows_view(ui: &mut Ui, id: &str, rows: &[Row<'_>]) {
    let dark = ui.visuals().dark_mode;
    let p = palette(dark);
    let height = ui.text_style_height(&TextStyle::Monospace);
    // One row is one line, however long: a wrapped row would be taller than its slot and break the row grid.
    let line = |ui: &mut Ui, rich: RichText| {
        ui.add(egui::Label::new(rich).wrap_mode(egui::TextWrapMode::Extend));
    };
    ScrollArea::both().id_salt(id).auto_shrink([false, false]).stick_to_bottom(id == "terminal").show_rows(ui, height, rows.len(), |ui, range| {
        for row in &rows[range] {
            match row {
                Row::Plain(l) => line(ui, RichText::new(clip(l)).monospace()),
                Row::Diff(l) => {
                    let colour = if l.starts_with("+++") || l.starts_with("---") {
                        p.muted
                    } else if l.starts_with('+') {
                        tone_color(Tone::Done, dark)
                    } else if l.starts_with('-') {
                        p.danger
                    } else if l.starts_with("@@") {
                        tone_color(Tone::Active, dark)
                    } else {
                        p.text
                    };
                    line(ui, RichText::new(clip(l)).monospace().color(colour));
                }
                Row::Heading(r) => line(ui, RichText::new(*r).strong()),
                Row::Note(n) => line(ui, RichText::new(*n).small().color(p.muted)),
            }
        }
    });
}
