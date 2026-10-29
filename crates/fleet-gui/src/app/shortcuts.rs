//! The app's side of Settings → General's Shortcuts row and the wizard's Start menu checkbox.
use crate::views::settings::{ShortcutsAction, ShortcutsPane};
use fleet_client::shortcuts::{self, Places, ShortcutState, Which};
use std::path::PathBuf;

pub(super) struct Shortcuts {
    places: Places,
    exe: PathBuf,
    state: ShortcutState,
    /// The ticked places on the row.
    pick: Which,
    error: Option<String>,
}

impl Shortcuts {
    pub(super) fn new(places: Places, exe: PathBuf) -> Shortcuts {
        let state = shortcuts::state(&places, &exe);
        Shortcuts {
            places,
            exe,
            state,
            pick: Which {
                menu: true,
                desktop: false,
            },
            error: None,
        }
    }

    pub(super) fn pane(&mut self) -> ShortcutsPane<'_> {
        ShortcutsPane {
            state: self.state,
            pick: &mut self.pick,
            error: self.error.as_deref(),
        }
    }

    pub(super) fn apply(&mut self, action: ShortcutsAction) {
        let result = match action {
            ShortcutsAction::Add => shortcuts::add(&self.places, self.pick, &self.exe),
            ShortcutsAction::Remove => shortcuts::remove(&self.places, self.pick),
            ShortcutsAction::Update => {
                shortcuts::add(&self.places, shortcuts::existing(&self.places), &self.exe)
            }
        };
        self.finish(result);
    }

    /// The wizard's checkbox: only the Start menu.
    pub(super) fn apply_menu_only(&mut self) {
        let menu = Which {
            menu: true,
            desktop: false,
        };
        let result = shortcuts::add(&self.places, menu, &self.exe);
        self.finish(result);
    }

    fn finish(&mut self, result: std::io::Result<()>) {
        self.error = result
            .err()
            .map(|e| format!("Could not change the shortcuts: {e}"));
        self.state = shortcuts::state(&self.places, &self.exe);
    }
}
