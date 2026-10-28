use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use fleet_api::{ToolError, ToolFix, ToolId};
use fleet_gui::components::{tool_error_ui, ToolAction};
use std::cell::RefCell;
use std::rc::Rc;

fn not_installed(install: Option<&str>) -> ToolError {
    ToolError::NotInstalled {
        tool: ToolId::Gh,
        fix: ToolFix {
            install: install.map(Into::into),
            link: "https://cli.github.com".into(),
            manager: install.map(|_| "winget".into()),
        },
    }
}

fn harness(e: ToolError, can_install: bool) -> (Harness<'static>, Rc<RefCell<Vec<ToolAction>>>) {
    let got = Rc::new(RefCell::new(vec![]));
    let g = got.clone();
    let h = Harness::new_ui(move |ui| {
        if let Some(a) = tool_error_ui(ui, &e, can_install) {
            g.borrow_mut().push(a);
        }
    });
    (h, got)
}

#[test]
fn an_error_without_an_install_command_shows_only_the_link() {
    let (h, _) = harness(not_installed(None), true);
    assert!(h.query_by_label("Install").is_none());
    assert!(h.query_by_label("Copy").is_none());
    h.get_by_label("https://cli.github.com");
}

#[test]
fn install_click_returns_the_command() {
    let (mut h, got) = harness(not_installed(Some("winget install GitHub.cli")), true);
    h.get_by_label("winget install GitHub.cli");
    h.get_by_label("Install").click();
    h.run();
    assert_eq!(
        *got.borrow(),
        [ToolAction::Install("winget install GitHub.cli".into())]
    );
}

#[test]
fn without_permission_to_install_the_command_is_only_copyable() {
    let (mut h, got) = harness(not_installed(Some("winget install GitHub.cli")), false);
    assert!(h.query_by_label("Install").is_none());
    h.get_by_label("Copy").click();
    h.run();
    assert_eq!(
        *got.borrow(),
        [ToolAction::CopyCommand("winget install GitHub.cli".into())]
    );
}

#[test]
fn the_link_click_returns_the_link() {
    let (mut h, got) = harness(not_installed(None), true);
    h.get_by_label("https://cli.github.com").click();
    h.run();
    assert_eq!(
        *got.borrow(),
        [ToolAction::OpenLink("https://cli.github.com".into())]
    );
}

#[test]
fn not_logged_in_offers_no_install() {
    let (h, _) = harness(
        ToolError::NotLoggedIn {
            tool: ToolId::Claude,
            persona: None,
        },
        true,
    );
    assert!(h.query_by_label("Install").is_none());
    h.get_by_label("Log in again from the persona's setup.");
}
