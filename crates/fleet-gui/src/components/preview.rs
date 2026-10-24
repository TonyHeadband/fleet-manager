use super::reply::reply_box;
use crate::theme::{mono, sans, size, tokens, tone_color, Weight};
use crate::widgets::{button, icon_button, tabs, ButtonVariant, Icon};
use eframe::egui::{self, vec2, Align, Frame, Label, Layout, Margin, Panel, RichText, ScrollArea, Sense, TextStyle, Ui};
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
    /// `repo / branch`, drawn in the panel header.
    pub path: String,
    pub tab: PreviewTab,
    pub text: String,
    pub closed: bool,
    pub diff: DiffView,
    /// The reply box takes the cursor on the first frame the panel draws, if it has a box then; the panel clears
    /// this on that frame either way, so a box that appears later cannot steal the cursor from another field.
    pub focus_reply: bool,
}

/// The reply box's inputs: the agent's input action label (the field's hint) and the text kept for this agent.
pub struct Reply<'a> {
    pub label: &'a str,
    pub text: &'a mut String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewAction {
    None,
    /// The app should fetch the diff and put the result in `PreviewState::diff`.
    LoadDiff,
    Close,
    /// The user sent this text from the reply box; the app runs the input action and clears the box.
    Reply(String),
}

/// The side panel: the agent's terminal text and its diff, and a reply box when `reply` is given. It draws only
/// what is in `st`.
pub fn preview_pane(ui: &mut Ui, st: &mut PreviewState, reply: Option<Reply<'_>>) -> PreviewAction {
    let mut action = PreviewAction::None;
    let p = tokens(ui.visuals().dark_mode);
    let item_spacing = ui.spacing().item_spacing;
    // The bands and their rules touch; only the body gets the usual spacing back.
    ui.spacing_mut().item_spacing.y = 0.0;

    let focus_reply = std::mem::take(&mut st.focus_reply);
    // Shown before the body so the body fills only what is left; the panel's own separator is the top border.
    if let Some(Reply { label, text }) = reply {
        Panel::bottom("reply").frame(Frame::new().fill(p.surface).inner_margin(Margin::same(12))).show(ui, |ui| {
            ui.spacing_mut().item_spacing = item_spacing;
            // Salted by agent: egui keys the cursor and undo history by widget id, and one agent's draft must not
            // be reachable (Ctrl+Z) from another's box.
            ui.push_id(&st.agent_id, |ui| {
                if let Some(sent) = reply_box(ui, label, text, focus_reply) {
                    action = PreviewAction::Reply(sent);
                }
            });
        });
    }

    Frame::new().inner_margin(Margin::symmetric(12, 9)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, Icon::Close, "Close").clicked() {
                    action = PreviewAction::Close;
                }
                // Truncated text shows the whole path in egui's own elision tooltip.
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(Label::new(RichText::new(&st.path).font(mono(size::SMALL)).color(p.text_high)).truncate());
                });
            });
        });
    });
    rule(ui);

    Frame::new().inner_margin(Margin { left: 12, right: 12, top: 10, bottom: 6 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(Label::new(RichText::new(&st.title).font(sans(ui, size::BODY, Weight::Medium)).color(p.text_high)).wrap());
    });

    Frame::new().inner_margin(Margin::symmetric(12, 0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
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
            if st.tab == PreviewTab::Diff {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Refresh", ButtonVariant::Ghost).clicked() {
                        st.diff = DiffView::Loading;
                        if action == PreviewAction::None {
                            action = PreviewAction::LoadDiff;
                        }
                    }
                });
            }
        });
    });
    rule(ui);

    Frame::new().fill(p.bg).inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.spacing_mut().item_spacing = item_spacing;
        ui.set_min_size(ui.available_size());
        let note = |ui: &mut Ui, text: &str| {
            ui.label(RichText::new(text).size(size::SMALL).color(p.text_low));
        };
        match st.tab {
            PreviewTab::Terminal => {
                // Before the rows: they fill the rest of the pane, so anything after them is pushed out of sight.
                if st.closed {
                    note(ui, "Session ended");
                }
                // tmux sends the whole pane, empty rows under short output included; kept, they would be what the
                // bottom-stuck view shows instead of the text.
                let rows: Vec<Row> = st.text.trim_end().lines().map(Row::Plain).collect();
                rows_view(ui, "terminal", &rows);
            }
            PreviewTab::Diff => match &st.diff {
                DiffView::NotLoaded | DiffView::Loading => note(ui, "Loading…"),
                DiffView::Failed(message) => {
                    ui.label(RichText::new(message).color(p.danger));
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
    });
    action
}

/// The 1 px line under a band, edge to edge.
fn rule(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, tokens(ui.visuals().dark_mode).border);
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
    let p = tokens(dark);
    let height = ui.text_style_height(&TextStyle::Monospace);
    // Code size, not body size, so a heading still fits its slot in the row grid.
    let heading_font = sans(ui, size::CODE, Weight::Medium);
    // One row is one line, however long: a wrapped row would be taller than its slot and break the row grid.
    let line = |ui: &mut Ui, rich: RichText| {
        ui.add(egui::Label::new(rich).wrap_mode(egui::TextWrapMode::Extend));
    };
    ScrollArea::both().id_salt(id).auto_shrink([false, false]).stick_to_bottom(id == "terminal").show_rows(ui, height, rows.len(), |ui, range| {
        for row in &rows[range] {
            match row {
                Row::Plain(l) => line(ui, RichText::new(clip(l)).monospace().color(p.text)),
                Row::Diff(l) => {
                    let colour = if l.starts_with("+++") || l.starts_with("---") {
                        p.text_low
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
                Row::Heading(r) => line(ui, RichText::new(*r).font(heading_font.clone()).color(p.text_high)),
                Row::Note(n) => line(ui, RichText::new(*n).size(size::SMALL).color(p.text_low)),
            }
        }
    });
}
