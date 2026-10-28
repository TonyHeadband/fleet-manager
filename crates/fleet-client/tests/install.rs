use fleet_api::Os;
use fleet_client::install::install_argv;
use std::path::Path;

const MINTTY: &str = r"C:\Git\usr\bin\mintty.exe";

#[test]
fn git_install_on_windows_uses_a_plain_console_even_with_mintty() {
    let argv = install_argv(
        "winget install --id Git.Git -e --source winget",
        Os::Windows,
        Some(Path::new(MINTTY)),
        None,
    )
    .unwrap();
    assert_eq!(
        argv,
        [
            "cmd.exe",
            "/c",
            "winget install --id Git.Git -e --source winget & pause"
        ]
    );
}

#[test]
fn claude_install_on_windows_uses_mintty_when_present() {
    let cmd = "irm https://claude.ai/install.ps1 | iex";
    let argv = install_argv(cmd, Os::Windows, Some(Path::new(MINTTY)), None).unwrap();
    assert_eq!(&argv[..4], [MINTTY, "-o", "ConPTY=true", "-e"]);
    assert!(argv[4].ends_with("bash.exe"));
    assert_eq!(argv[5], "-lc");
    assert_eq!(argv[6], format!("{cmd}; read -p \"Press Enter to close\""));
    assert_eq!(argv.len(), 7);
}

#[test]
fn windows_without_mintty_falls_back_to_the_console() {
    let argv = install_argv("echo hi", Os::Windows, None, None).unwrap();
    assert_eq!(argv[0], "cmd.exe");
    assert_eq!(argv.last().unwrap(), "echo hi & pause");
}

#[test]
fn linux_runs_the_command_through_sh_in_the_terminal() {
    let term = Some(vec!["xterm".to_string(), "-e".to_string()]);
    let cmd = "curl -fsSL https://claude.ai/install.sh | bash";
    let argv = install_argv(cmd, Os::Linux, None, term).unwrap();
    assert_eq!(&argv[..4], ["xterm", "-e", "sh", "-c"]);
    assert_eq!(
        argv[4],
        format!("{cmd}; printf \"\\nPress Enter to close\"; read _")
    );
}

#[test]
fn linux_without_a_terminal_cannot_install() {
    assert_eq!(install_argv("true", Os::Linux, None, None), None);
}

const TRICKY: &str = "powershell -Command \"irm https://x | iex\" && echo $(date) | cat";

#[test]
fn quotes_pipes_and_substitutions_reach_the_shell_element_intact() {
    let term = Some(vec!["xterm".to_string(), "-e".to_string()]);
    let argv = install_argv(TRICKY, Os::Linux, None, term).unwrap();
    assert!(argv[4].starts_with(TRICKY) && argv.len() == 5);
    let argv = install_argv(TRICKY, Os::Windows, Some(Path::new(MINTTY)), None).unwrap();
    assert!(argv[6].starts_with(TRICKY) && argv.len() == 7);
}

/// The exact command line `run_install` uses (minus the new window): cmd.exe parses the quotes and the pipe.
#[cfg(windows)]
#[test]
fn cmd_reads_the_console_command_line_with_quotes_and_a_pipe() {
    use std::process::Stdio;
    let argv = install_argv(
        "echo \"a | b\" | findstr /c:\"a | b\"",
        Os::Windows,
        None,
        None,
    )
    .unwrap();
    let out = fleet_client::install::build_command(&argv, false)
        .unwrap()
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("a | b"));
}

/// Manual check, opens a real console window: `cargo test -p fleet-client --test install -- --ignored`.
#[cfg(windows)]
#[test]
#[ignore]
fn manual_install_window_shows_the_text_and_waits() {
    let argv = install_argv("echo fleet-manager install test", Os::Windows, None, None).unwrap();
    let status = fleet_client::install::run_install(argv)
        .unwrap()
        .wait()
        .unwrap();
    assert!(status.success());
}
