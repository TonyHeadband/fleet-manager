//! The in-app folder picker behind every Browse… button. It reads the disk on background threads only (see
//! `Scanner`): `/mnt/users` is a network home where a listing or a `git status` can take seconds.
use super::{modal_frame, Scanner};
use crate::theme::{mono, radius, sans, size, tokens, Weight};
use crate::widgets::{badge, button, icon_button, list_row, path_edit, path_label, prefixed_path_label, BadgeTone, ButtonVariant, Icon};
use eframe::egui::{self, Align, Frame, Key, Label, Layout, Margin, RichText, ScrollArea, TextStyle, Ui};
use fleet_client::config::GuiSettings;
use fleet_fs::{Checkout, EntryKind, Listing};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

const WIDTH: f32 = 560.0;
/// The list keeps one height, so the dialog does not jump as folders of different lengths load.
const LIST_HEIGHT: f32 = 320.0;
const SHORT_HASH: usize = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickMode {
    /// Any folder; Select takes the folder being shown.
    Folder,
    /// One checkout of a git repo; Select takes the chosen checkout.
    Repo,
}

/// A background result. Each carries the folder or repo it was computed for, so one that arrives after the user
/// moved on is dropped.
#[derive(Debug)]
pub enum Scan {
    /// Where an opening picker starts, worked out off the UI thread; taken only if the user has not moved yet.
    Opened { start: PathBuf },
    Listed(Listing),
    Failed { dir: PathBuf, error: String },
    Checkouts { repo: PathBuf, list: Result<Vec<Checkout>, String> },
    Dirty { path: PathBuf, count: usize },
}

/// A repo row opened to show its checkouts; `checkouts` is None until they arrive.
struct Expanded {
    repo: PathBuf,
    checkouts: Option<Result<Vec<Checkout>, String>>,
}

pub struct FolderPicker {
    /// Several pickers live on one screen (New project, Add repos, the Grant box); each modal needs its own id.
    id: egui::Id,
    open: bool,
    mode: PickMode,
    /// The folder being shown, or being loaded when it differs from `listing`'s.
    dir: PathBuf,
    path_text: String,
    /// Set when the path bar's text was replaced, so the field is drawn scrolled to the end of the new path.
    show_tail: bool,
    listing: Option<Listing>,
    expanded: Option<Expanded>,
    chosen: Option<PathBuf>,
    dirty: HashMap<PathBuf, usize>,
    error: Option<String>,
    tx: Sender<Scan>,
    rx: Receiver<Scan>,
    /// Bumped on every navigation and expansion, so a thread still counting dirty files for a repo the user left
    /// stops instead of running `git status` on every one of its checkouts.
    generation: Arc<AtomicU64>,
    scans: bool,
    settings_path: PathBuf,
    exclude: Vec<PathBuf>,
    home: PathBuf,
}

impl FolderPicker {
    /// `exclude` is the projects root and every project folder: folders under them are agents' copies, greyed out
    /// and never picked in repo mode. `settings_path` is `gui.toml`, where the last folder picked from is
    /// remembered.
    pub fn new(mode: PickMode, settings_path: PathBuf, exclude: Vec<PathBuf>, home: PathBuf) -> FolderPicker {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let (tx, rx) = channel();
        FolderPicker {
            id: egui::Id::unique(("folder_picker", NEXT.fetch_add(1, Ordering::Relaxed))),
            open: false,
            mode,
            dir: home.clone(),
            path_text: String::new(),
            show_tail: false,
            listing: None,
            expanded: None,
            chosen: None,
            dirty: HashMap::new(),
            error: None,
            tx,
            rx,
            generation: Arc::new(AtomicU64::new(0)),
            scans: true,
            settings_path,
            exclude,
            home,
        }
    }

    /// Starts no background scans; tests `deliver` every result themselves.
    #[doc(hidden)]
    pub fn without_scans(mut self) -> FolderPicker {
        self.scans = false;
        self
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    #[doc(hidden)]
    pub fn deliver(&mut self, scan: Scan) {
        match scan {
            Scan::Opened { start } => {
                if self.dir.as_os_str().is_empty() {
                    self.path_text = start.display().to_string();
                    self.show_tail = true;
                    self.dir = start;
                }
            }
            Scan::Listed(listing) => {
                if listing.dir == self.dir {
                    self.listing = Some(listing);
                    self.error = None;
                }
            }
            Scan::Failed { dir, error } => {
                if dir == self.dir {
                    self.error = Some(error);
                    // Back to the folder still listed, so Select never takes one that could not be opened. The
                    // path bar keeps the failed text for the user to correct.
                    if let Some(listing) = &self.listing {
                        self.dir = listing.dir.clone();
                    }
                }
            }
            Scan::Checkouts { repo, list } => {
                if let Some(x) = self.expanded.as_mut().filter(|x| x.repo == repo) {
                    x.checkouts = Some(list);
                }
            }
            Scan::Dirty { path, count } => {
                let known = self.expanded_checkouts().iter().any(|c| c.path == path);
                if known {
                    self.dirty.insert(path, count);
                }
            }
        }
    }

    fn expanded_checkouts(&self) -> &[Checkout] {
        match self.expanded.as_ref().and_then(|x| x.checkouts.as_ref()) {
            Some(Ok(list)) => list,
            _ => &[],
        }
    }

    fn expand_tilde(&self, text: &str) -> PathBuf {
        match text.strip_prefix('~') {
            Some("") => self.home.clone(),
            Some(rest) if rest.starts_with('/') => self.home.join(rest.trim_start_matches('/')),
            _ => PathBuf::from(text),
        }
    }

    fn open_at(&mut self, ctx: &egui::Context, text: &str) {
        self.open = true;
        self.listing = None;
        let typed = Some(text.trim()).filter(|t| !t.is_empty()).map(|t| self.expand_tilde(t));
        let (settings, home, exclude) = (self.settings_path.clone(), self.home.clone(), self.exclude.clone());
        let repo_mode = self.mode == PickMode::Repo;
        let resolve = move || start_dir(typed, &settings, &home, repo_mode.then_some(exclude.as_slice()));
        if !self.scans {
            self.navigate(ctx, resolve());
            return;
        }
        // Even `is_dir` blocks on a hung automount or NFS server, so the start folder is worked out with the listing,
        // on the scan thread. Until it arrives the dialog shows an empty placeholder folder.
        self.path_text = text.trim().to_string();
        self.dir = PathBuf::new();
        self.error = None;
        self.collapse();
        let (tx, ctx, exclude) = (self.tx.clone(), ctx.clone(), self.exclude.clone());
        Scanner::shared().run(move || {
            let start = resolve();
            let _ = tx.send(Scan::Opened { start: start.clone() });
            let _ = tx.send(scan_dir(start, &exclude));
            ctx.request_repaint();
        });
    }

    fn navigate(&mut self, ctx: &egui::Context, dir: PathBuf) {
        let dir = fleet_fs::normalise(&dir);
        self.path_text = dir.display().to_string();
        self.show_tail = true;
        self.dir = dir.clone();
        self.error = None;
        self.collapse();
        if !self.scans {
            return;
        }
        let (tx, ctx, exclude) = (self.tx.clone(), ctx.clone(), self.exclude.clone());
        Scanner::shared().run(move || {
            let _ = tx.send(scan_dir(dir, &exclude));
            ctx.request_repaint();
        });
    }

    fn collapse(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.expanded = None;
        self.chosen = None;
        self.dirty.clear();
    }

    fn toggle_repo(&mut self, ctx: &egui::Context, repo: PathBuf) {
        let same = self.expanded.as_ref().is_some_and(|x| x.repo == repo);
        self.collapse();
        if same {
            return;
        }
        self.expanded = Some(Expanded { repo: repo.clone(), checkouts: None });
        if !self.scans {
            return;
        }
        let (tx, ctx, exclude) = (self.tx.clone(), ctx.clone(), self.exclude.clone());
        let (generation, mine) = (self.generation.clone(), self.generation.load(Ordering::Relaxed));
        Scanner::shared().run(move || {
            let list = fleet_fs::checkouts(&repo, &exclude).map_err(|e| e.to_string());
            let paths: Vec<PathBuf> = list.iter().flatten().map(|c| c.path.clone()).collect();
            let _ = tx.send(Scan::Checkouts { repo, list });
            ctx.request_repaint();
            // Sent one by one after the list: `git status` is the slow part, and the rows are useful without it.
            for path in paths {
                if generation.load(Ordering::Relaxed) != mine {
                    return;
                }
                if let Ok(count) = fleet_fs::dirty_count(&path) {
                    let _ = tx.send(Scan::Dirty { path, count });
                    ctx.request_repaint();
                }
            }
        });
    }

    /// The folder whose listing is on screen, once the one being shown has loaded.
    fn loaded(&self) -> bool {
        self.listing.as_ref().is_some_and(|l| l.dir == self.dir)
    }

    fn selection(&self) -> Option<PathBuf> {
        match self.mode {
            PickMode::Folder => self.loaded().then(|| self.dir.clone()),
            PickMode::Repo => self.chosen.clone(),
        }
    }
}

/// The typed folder if it is one, else the last folder picked from, else home. Reads the disk: never on the UI
/// thread outside tests. `exclude` is set in repo mode, where a folder under it has nothing pickable (the rule
/// `list` greys it by), so reopening where the last pick was made would start on a dead end.
fn start_dir(typed: Option<PathBuf>, settings: &Path, home: &Path, exclude: Option<&[PathBuf]>) -> PathBuf {
    if let Some(t) = typed.filter(|t| t.is_dir()) {
        return fleet_fs::normalise(&t);
    }
    let canonical = |p: &PathBuf| p.canonicalize().unwrap_or_else(|_| p.clone());
    let dead_end = |d: &PathBuf| exclude.is_some_and(|x| fleet_fs::is_under(&canonical(d), &x.iter().map(canonical).collect::<Vec<_>>()));
    GuiSettings::load(settings).last_pick_dir.filter(|d| d.is_dir() && !dead_end(d)).unwrap_or_else(|| home.to_path_buf())
}

/// `list` canonicalises; the picker keeps the path it asked for, so the stale check, the path bar and Up all follow
/// the path the user went to (through symlinks such as an automounted home).
fn scan_dir(dir: PathBuf, exclude: &[PathBuf]) -> Scan {
    match fleet_fs::list(&dir, exclude) {
        Ok(listing) => Scan::Listed(Listing { dir, ..listing }),
        Err(e) => Scan::Failed { error: format!("Cannot open {}: {e}", dir.display()), dir },
    }
}

fn dirty_note(count: usize) -> String {
    match count {
        1 => "1 uncommitted file won't be included".to_owned(),
        n => format!("{n} uncommitted files won't be included"),
    }
}

/// How to bring a project branch back into the branch it started from. One copy of the text for the picker and the chips.
pub(super) fn merge_note(project_branch: &str, from_branch: &str) -> String {
    format!("Merge {project_branch} into {from_branch} first (a fast-forward if you did not commit there meanwhile), then merge {from_branch} as usual.")
}

/// The Browse… button. When the picker is open, it also draws the modal. On Select it writes the chosen path
/// into `text`, saves the folder it was chosen from as `last_pick_dir`, closes, and returns true.
pub fn browse(ui: &mut Ui, picker: &mut FolderPicker, text: &mut String) -> bool {
    if button(ui, "Browse…", ButtonVariant::Default).clicked() {
        picker.open_at(ui.ctx(), text);
    }
    while let Ok(scan) = picker.rx.try_recv() {
        picker.deliver(scan);
    }
    if !picker.open {
        return false;
    }
    let Some(path) = dialog(ui.ctx(), picker) else { return false };
    *text = path.display().to_string();
    // Losing the remembered folder only costs the next opening place; it must not undo the pick.
    let from = picker.dir.clone();
    let _ = GuiSettings::update(&picker.settings_path, |s| s.last_pick_dir = Some(from));
    picker.open = false;
    true
}

/// What a click in the dialog asks for; applied after drawing, so the listing is not changed under the loop.
enum Go {
    Dir(PathBuf),
    Repo(PathBuf),
    Choose(PathBuf),
}

fn dialog(ctx: &egui::Context, picker: &mut FolderPicker) -> Option<PathBuf> {
    let p = tokens(ctx.global_style().visuals.dark_mode);
    let frame = modal_frame(ctx);
    let inner_width = WIDTH - frame.total_margin().sum().x;
    let title = match picker.mode {
        PickMode::Folder => "Choose a folder",
        PickMode::Repo => "Choose a repo",
    };
    let (mut close, mut selected, mut go) = (false, None, None);
    let modal = egui::Modal::new(picker.id)
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(inner_width);
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).text_style(TextStyle::Heading).color(p.text_high));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    close |= icon_button(ui, Icon::Close, "Close").clicked();
                });
            });
            ui.add_space(12.0);

            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Home", ButtonVariant::Default).clicked() {
                        go = Some(Go::Dir(picker.home.clone()));
                    }
                    let parent = picker.dir.parent().map(Path::to_path_buf);
                    let up = ui.add_enabled_ui(parent.is_some(), |ui| button(ui, "Up", ButtonVariant::Default)).inner;
                    if up.clicked() {
                        go = parent.map(Go::Dir);
                    }
                    let tail = std::mem::take(&mut picker.show_tail);
                    let (id, width) = (picker.id.with("path_bar"), ui.available_width());
                    let edit = path_edit(ui, id, &mut picker.path_text, width, Some(mono(size::CODE)), "", tail);
                    edit.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Folder path"));
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        let typed = picker.expand_tilde(picker.path_text.trim());
                        // A relative path is taken from the folder on screen, as a shell would.
                        go = Some(Go::Dir(picker.dir.join(typed)));
                    }
                });
            });
            if let Some(e) = &picker.error {
                ui.add_space(6.0);
                ui.label(RichText::new(e).size(size::SMALL).color(p.danger));
            }
            ui.add_space(8.0);

            Frame::new().fill(p.bg).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(radius::CARD).inner_margin(Margin::same(4)).show(ui, |ui| {
                ScrollArea::vertical().max_height(LIST_HEIGHT).min_scrolled_height(LIST_HEIGHT).auto_shrink(false).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    if let Some(g) = entries(ui, picker) {
                        go = Some(g);
                    }
                });
            });
            ui.add_space(12.0);

            let target = picker.selection();
            ui.horizontal(|ui| {
                let loading = picker.listing.is_some() && !picker.loaded();
                let chosen_path = picker.chosen.as_ref().filter(|_| !loading && picker.mode == PickMode::Repo).map(|c| c.display().to_string());
                let hint = if loading {
                    "Reading folder…"
                } else {
                    match picker.mode {
                        PickMode::Folder => "Select takes the folder shown above",
                        PickMode::Repo => "Open a repo, then choose one of its checkouts",
                    }
                };
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let select = ui.add_enabled_ui(target.is_some(), |ui| button(ui, "Select", ButtonVariant::Primary)).inner;
                    if select.clicked() {
                        selected = target.clone();
                    }
                    close |= button(ui, "Cancel", ButtonVariant::Default).clicked();
                    // Last, so a long path truncates in the space the buttons leave instead of pushing them out.
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let font = sans(ui, size::SMALL, Weight::Regular);
                        match &chosen_path {
                            // One label, as before; only the path part elides.
                            Some(path) => {
                                prefixed_path_label(ui, "Select: ", path, font, p.text_low);
                            }
                            None => {
                                ui.add(Label::new(RichText::new(hint).font(font).color(p.text_low)).truncate());
                            }
                        }
                    });
                });
            });
        });

    match go {
        Some(Go::Dir(dir)) => picker.navigate(ctx, dir),
        Some(Go::Repo(repo)) => picker.toggle_repo(ctx, repo),
        Some(Go::Choose(path)) => picker.chosen = Some(path),
        None => {}
    }
    // Escape and a click outside land in `should_close`; neither touches the text box.
    if selected.is_none() && (close || modal.should_close()) {
        picker.open = false;
    }
    selected
}

/// The rows of the listing, a repo's checkouts under it when expanded. Returns what a click asked for.
fn entries(ui: &mut Ui, picker: &FolderPicker) -> Option<Go> {
    let p = tokens(ui.visuals().dark_mode);
    let Some(listing) = &picker.listing else {
        // The first folder failed, so nothing is coming; the error above says why.
        placeholder(ui, if picker.error.is_some() { "Can't open this folder" } else { "Reading folder…" });
        return None;
    };
    if listing.entries.is_empty() {
        placeholder(ui, "No folders here");
        return None;
    }
    let mut go = None;
    for entry in &listing.entries {
        let excluded = entry.kind == EntryKind::Excluded;
        let expanded = picker.expanded.as_ref().is_some_and(|x| x.repo == entry.path);
        let row = list_row(ui, &entry.name, expanded, |ui| {
            ui.horizontal(|ui| {
                let colour = if excluded { p.text_low } else { p.text };
                ui.add(Label::new(RichText::new(&entry.name).font(sans(ui, size::BODY, Weight::Regular)).color(colour)).truncate());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| match &entry.kind {
                    EntryKind::Checkout { branch } => {
                        badge(ui, branch.as_deref().unwrap_or("detached"), "Current branch", BadgeTone::Neutral);
                    }
                    EntryKind::Excluded => {
                        ui.label(RichText::new("fleet project").font(sans(ui, size::META, Weight::Regular)).color(p.text_low));
                    }
                    EntryKind::Folder => {}
                });
            });
        });
        let row = if excluded { row.on_hover_text("Inside a project folder: these are agents' copies") } else { row };
        // The second click of a double click lands on whatever row the new folder put under the pointer; it must
        // not open that one too.
        if row.clicked() && !row.double_clicked() {
            go = match (&entry.kind, picker.mode) {
                (EntryKind::Excluded, PickMode::Repo) => None,
                (EntryKind::Checkout { .. }, PickMode::Repo) => Some(Go::Repo(entry.path.clone())),
                _ => Some(Go::Dir(entry.path.clone())),
            };
        }
        if expanded {
            if let Some(g) = checkouts(ui, picker) {
                go = Some(g);
            }
        }
    }
    go
}

fn placeholder(ui: &mut Ui, text: &str) {
    let low = tokens(ui.visuals().dark_mode).text_low;
    ui.add_space(12.0);
    ui.vertical_centered(|ui| ui.label(RichText::new(text).font(sans(ui, size::SMALL, Weight::Regular)).color(low)));
}

fn checkouts(ui: &mut Ui, picker: &FolderPicker) -> Option<Go> {
    let p = tokens(ui.visuals().dark_mode);
    let mut go = None;
    // Indented under their repo, so they read as its children.
    Frame::new().inner_margin(Margin { left: 16, right: 0, top: 1, bottom: 4 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        let list = match picker.expanded.as_ref().and_then(|x| x.checkouts.as_ref()) {
            None => return placeholder(ui, "Reading checkouts…"),
            Some(Err(e)) => {
                ui.label(RichText::new(e).size(size::SMALL).color(p.danger));
                return;
            }
            Some(Ok(list)) => list,
        };
        if list.is_empty() {
            return placeholder(ui, "No checkouts outside the projects folder");
        }
        for c in list {
            let label = c.path.display().to_string();
            let chosen = picker.chosen.as_ref() == Some(&c.path);
            let row = list_row(ui, &label, chosen, |ui| {
                ui.horizontal(|ui| {
                    let branch = c.head.branch.as_deref().unwrap_or("detached");
                    ui.label(RichText::new(branch).font(sans(ui, size::BODY, Weight::Medium)).color(p.text_high));
                    match &c.head.commit {
                        Some(commit) => {
                            ui.label(RichText::new(&commit[..SHORT_HASH.min(commit.len())]).font(mono(size::META)).color(p.text_low));
                            ui.add(Label::new(RichText::new(&c.head.subject).font(sans(ui, size::SMALL, Weight::Regular)).color(p.text)).truncate());
                        }
                        None => {
                            ui.label(RichText::new("no commits yet").font(sans(ui, size::SMALL, Weight::Regular)).color(p.text_low));
                        }
                    }
                });
                path_label(ui, &label, mono(size::META), p.text_low);
                if let Some(&n) = picker.dirty.get(&c.path).filter(|&&n| n > 0) {
                    ui.add_space(2.0);
                    badge(ui, &dirty_note(n), "Uncommitted changes stay in this checkout", BadgeTone::Attention);
                }
            });
            let row = match &c.head.branch {
                // No project exists yet while picking, so its branch is the placeholder the user will recognise.
                Some(b) => row.on_hover_text(merge_note("fleet/<project>", b)),
                None => row,
            };
            if row.clicked() && !row.double_clicked() {
                go = Some(Go::Choose(c.path.clone()));
            }
        }
    });
    go
}
