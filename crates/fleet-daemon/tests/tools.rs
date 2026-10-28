use fleet_api::{Os, ToolId};
use fleet_daemon::tools::{at_least, detect_manager, fix, parse_version, Manager};

#[test]
fn windows_uses_winget_only_when_present() {
    assert_eq!(detect_manager(Os::Windows, |_| true), Some(Manager::Winget));
    assert_eq!(detect_manager(Os::Windows, |_| false), None);
}

#[test]
fn linux_picks_the_first_manager_found() {
    assert_eq!(
        detect_manager(Os::Linux, |p| p == "dnf"),
        Some(Manager::Dnf)
    );
    assert_eq!(
        detect_manager(Os::Linux, |p| p == "apt-get" || p == "dnf"),
        Some(Manager::Apt)
    );
    assert_eq!(detect_manager(Os::Linux, |_| false), None);
}

#[test]
fn no_manager_means_link_only_except_claude_on_linux() {
    let gh = fix(ToolId::Gh, Os::Linux, None);
    assert_eq!(gh.install, None);
    assert_eq!(gh.link, "https://cli.github.com/");
    let claude = fix(ToolId::Claude, Os::Linux, None);
    assert!(claude.install.unwrap().contains("claude.ai/install.sh"));
}

#[test]
fn versions_are_parsed_from_each_tool() {
    assert_eq!(
        parse_version(ToolId::Git, "git version 2.43.0\n"),
        Some("2.43.0".into())
    );
    assert_eq!(
        parse_version(ToolId::Git, "git version 2.43.0.windows.1\n"),
        Some("2.43.0".into())
    );
    assert_eq!(
        parse_version(ToolId::Claude, "2.0.14 (Claude Code)\n"),
        Some("2.0.14".into())
    );
    assert_eq!(
        parse_version(
            ToolId::Gh,
            "gh version 2.40.1 (2023-12-13)\nhttps://github.com/cli/cli/releases/tag/v2.40.1\n"
        ),
        Some("2.40.1".into())
    );
    assert_eq!(parse_version(ToolId::Git, "garbage"), None);
    assert_eq!(parse_version(ToolId::Gh, "garbage"), None);
}

#[test]
fn gh_2_39_is_too_old_and_2_40_is_enough() {
    assert!(!at_least("2.39.9", "2.40"));
    assert!(at_least("2.40", "2.40"));
    assert!(at_least("2.100.0", "2.40"));
}
