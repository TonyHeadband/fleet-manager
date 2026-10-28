use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use fleet_api::{ToolError, ToolFix, ToolId, ToolList, ToolStatus};
use fleet_gui::components::{requirements_ui, RequirementsAction, ToolAction};
use std::cell::RefCell;
use std::rc::Rc;

fn found(tool: ToolId, version: &str) -> ToolStatus {
    ToolStatus {
        tool,
        required: true,
        version: Some(version.into()),
        error: None,
    }
}

fn missing(tool: ToolId) -> ToolStatus {
    ToolStatus {
        tool,
        required: true,
        version: None,
        error: Some(ToolError::NotInstalled {
            tool,
            fix: ToolFix {
                install: Some("winget install Anthropic.ClaudeCode".into()),
                link: "https://claude.com/claude-code".into(),
                manager: Some("winget".into()),
            },
        }),
    }
}

fn list(tools: Vec<ToolStatus>) -> ToolList {
    ToolList {
        tools,
        checked_at: 1,
    }
}

type Got = Rc<RefCell<Vec<RequirementsAction>>>;

fn harness(l: ToolList, installing: Option<ToolId>) -> (Harness<'static>, Got) {
    let got = Rc::new(RefCell::new(vec![]));
    let g = got.clone();
    // The theme's fonts: the buttons draw in its families.
    let h = Harness::builder()
        .with_context_setup(fleet_gui::theme::apply)
        .build_ui(move |ui| {
            if let Some(a) = requirements_ui(ui, &l, installing, |_| true) {
                g.borrow_mut().push(a);
            }
        });
    (h, got)
}

#[test]
fn a_found_tool_shows_its_version() {
    let (h, _) = harness(list(vec![found(ToolId::Git, "2.45.1")]), None);
    h.get_by_label("✓ Git 2.45.1");
    h.get_by_label("Continue");
    assert!(h.query_by_label("Continue without it").is_none());
}

#[test]
fn a_missing_tool_shows_its_fix_and_continue_without_it() {
    let (mut h, got) = harness(
        list(vec![found(ToolId::Git, "2.45.1"), missing(ToolId::Claude)]),
        None,
    );
    h.get_by_label("Claude Code is not installed");
    h.get_by_label("Install").click();
    h.run();
    assert_eq!(
        *got.borrow(),
        [RequirementsAction::Tool(
            ToolId::Claude,
            ToolAction::Install("winget install Anthropic.ClaudeCode".into())
        )]
    );
    h.get_by_label("Continue without it").click();
    h.run();
    assert_eq!(got.borrow().last(), Some(&RequirementsAction::Continue));
}

#[test]
fn check_again_asks_for_a_recheck() {
    let (mut h, got) = harness(list(vec![missing(ToolId::Claude)]), None);
    h.get_by_label("Check again").click();
    h.run();
    assert_eq!(*got.borrow(), [RequirementsAction::Recheck]);
}

#[test]
fn while_installing_no_other_install_starts() {
    let (h, _) = harness(list(vec![missing(ToolId::Claude)]), Some(ToolId::Claude));
    assert!(h.query_by_label("Install").is_none());
    h.get_by_label_contains("Installing Claude Code");
}
