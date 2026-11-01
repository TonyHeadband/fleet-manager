use fleet_api::{GithubAuth, Persona};
#[cfg(unix)]
use fleet_client::login::terminal_command;
use fleet_client::login::{
    claude_login_script, gh_login_script, is_loopback_url, mintty_argv, shell_join, shell_quote,
    terminal_argv,
};
use std::path::Path;

fn persona(dir: &std::path::Path) -> Persona {
    Persona {
        id: "work".into(),
        name: "Work".into(),
        host: "github.example.invalid".into(),
        commit_name: "T".into(),
        commit_email: "t@example.invalid".into(),
        github_auth: GithubAuth::Pat,
        // Joined with `/`, as a POSIX shell reads them, so the expected scripts are the same on every OS.
        claude_dir: format!("{}/claude dir", dir.display()),
        gh_dir: format!("{}/gh", dir.display()),
        ready: false,
        claude: None,
        github: None,
        tasks: 0,
        nicknames: Default::default(),
    }
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn terminal_prefers_env_then_known_list() {
    let all = |_: &str| true;
    assert_eq!(
        terminal_argv(Some("alacritty"), all),
        Some(strs(&["alacritty", "-e"]))
    );
    assert_eq!(
        terminal_argv(None, all),
        Some(strs(&["x-terminal-emulator", "-e"]))
    );
    assert_eq!(
        terminal_argv(Some(""), all),
        Some(strs(&["x-terminal-emulator", "-e"]))
    );
    assert_eq!(
        terminal_argv(Some("kitty --single-instance"), |n| n == "kitty"),
        Some(strs(&["kitty", "--single-instance", "-e"]))
    );
    // $TERMINAL set but not installed falls through to the list.
    let only = |n: &str| n == "konsole";
    assert_eq!(
        terminal_argv(Some("alacritty"), only),
        Some(strs(&["konsole", "-e"]))
    );
    assert_eq!(
        terminal_argv(None, |n| n == "gnome-terminal"),
        Some(strs(&["gnome-terminal", "--"]))
    );
    assert_eq!(
        terminal_argv(None, |n| n == "xfce4-terminal"),
        Some(strs(&["xfce4-terminal", "-x"]))
    );
    assert_eq!(
        terminal_argv(None, |n| n == "xterm"),
        Some(strs(&["xterm", "-e"]))
    );
}

#[test]
fn none_found_is_none() {
    assert_eq!(terminal_argv(Some("alacritty"), |_| false), None);
    assert_eq!(terminal_argv(None, |_| false), None);
}

#[test]
fn scripts_quote_paths_with_spaces() {
    let p = persona(std::path::Path::new("/data/my home"));
    let claude = strs(&["claude", "--model", "a b"]);
    let s = claude_login_script(&p, &claude, "~/my tasks");
    assert!(s.starts_with("cd ~/'my tasks' && env -u "), "{s}");
    assert!(s.contains("-u GH_TOKEN "), "{s}");
    assert!(s.contains("-u SSH_AUTH_SOCK "), "{s}");
    assert!(
        s.ends_with("CLAUDE_CONFIG_DIR='/data/my home/claude dir' claude --model 'a b'"),
        "{s}"
    );
    let g = gh_login_script(&p, "/opt/my tools/gh");
    assert!(g.starts_with("env -u "), "{g}");
    assert!(g.contains("-u ANTHROPIC_API_KEY "), "{g}");
    assert!(
        g.ends_with(
            "GH_CONFIG_DIR='/data/my home/gh' GIT_CONFIG_GLOBAL='/data/my home/gh/gitconfig' '/opt/my tools/gh' auth login --hostname github.example.invalid --git-protocol https --insecure-storage"
        ),
        "{g}"
    );
}

#[test]
fn shell_quote_cases() {
    assert_eq!(shell_quote("plain-1/x"), "plain-1/x");
    assert_eq!(shell_quote("a b"), "'a b'");
    assert_eq!(shell_quote("it's"), "'it'\\''s'");
    assert_eq!(shell_quote("~/a b"), "~/'a b'");
    assert_eq!(shell_quote("~"), "~");
    assert_eq!(shell_quote(""), "''");
}

#[test]
fn shell_join_quotes_each_word() {
    assert_eq!(
        shell_join(&strs(&[
            "fleet-attach",
            "--dir",
            "fleet manager",
            "--name",
            "it's"
        ])),
        "fleet-attach --dir 'fleet manager' --name 'it'\\''s'"
    );
}

#[test]
fn only_this_machine_is_loopback() {
    for u in [
        "http://127.0.0.1:7878",
        "http://localhost:7878/",
        "http://[::1]:7878",
    ] {
        assert!(is_loopback_url(u), "{u}");
    }
    for u in [
        "http://10.0.0.5:7878",
        "https://fleet.example.com",
        "http://127.0.0.1.evil.com",
    ] {
        assert!(!is_loopback_url(u), "{u}");
    }
}

#[test]
fn mintty_runs_in_conpty_mode_so_console_programs_work() {
    assert_eq!(
        mintty_argv(Path::new("C:/Git/usr/bin/mintty.exe")),
        ["C:/Git/usr/bin/mintty.exe", "-o", "ConPTY=true", "-e"]
    );
}

#[cfg(unix)]
#[test]
fn terminal_command_pins_the_linux_argv() {
    let term = ["xterm".to_string(), "-e".to_string()];
    assert_eq!(
        terminal_command(&term, "echo hi").unwrap(),
        [
            "xterm",
            "-e",
            "sh",
            "-c",
            "echo hi; echo; printf \"Done. Press Enter to close.\"; read _"
        ]
    );
}

/// Git Bash runs a program by a `C:/…` path; with backslashes it would look the whole path up as a name.
#[cfg(windows)]
#[test]
fn a_windows_program_path_runs_in_git_bash() {
    let p = persona(std::path::Path::new("/data/home"));
    let s = claude_login_script(&p, &strs(&[r"C:\Tools\claude.exe", r"a\b"]), "/t");
    assert!(s.ends_with(r" C:/Tools/claude.exe 'a\b'"), "{s}");
    let g = gh_login_script(&p, r"C:\Program Files\GitHub CLI\gh.exe");
    assert!(
        g.contains(" 'C:/Program Files/GitHub CLI/gh.exe' auth login "),
        "{g}"
    );
}

/// Writes `web/src/generated/login.json`: the ambient variables a login unsets, and scripts built here for a few
/// personas. The web builds the same scripts for a browser to copy (`web/src/api/login.ts`), and its test checks
/// them against these; CI fails on a stale file through its `git diff` check.
#[cfg(unix)]
#[test]
fn login_scripts_export() {
    let tricky = Persona {
        host: "ghe.example.invalid".into(),
        claude_dir: "/home/me/it's here/claude".into(),
        gh_dir: "/home/me/it's here/gh/".into(),
        ..persona(Path::new("/home/me/personas/work"))
    };
    let cases: Vec<serde_json::Value> = [
        (
            persona(Path::new("/home/me/personas/work")),
            vec!["claude".to_string()],
            "/usr/bin/gh",
            "~/fleet-tasks",
        ),
        (
            tricky,
            vec![
                "/opt/claude code/claude".to_string(),
                "--model".into(),
                "a b".into(),
            ],
            "gh",
            "/srv/my tasks",
        ),
    ]
    .into_iter()
    .map(|(p, claude, gh, root)| {
        serde_json::json!({
            "persona": { "claude_dir": p.claude_dir, "gh_dir": p.gh_dir, "host": p.host },
            "claude": claude,
            "gh": gh,
            "tasks_root": root,
            "claude_script": claude_login_script(&p, &claude, root),
            "gh_script": gh_login_script(&p, gh),
        })
    })
    .collect();
    let out = serde_json::json!({
        "ambient": fleet_api::ambient_vars().collect::<Vec<_>>(),
        "cases": cases,
    });
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/src/generated/login.json");
    std::fs::write(&path, serde_json::to_string_pretty(&out).unwrap() + "\n").unwrap();
}
