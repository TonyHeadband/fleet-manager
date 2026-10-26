use fleet_api::{GithubAuth, Persona};
use fleet_client::login::{
    claude_login_script, gh_login_script, is_loopback_url, pat_login, pat_login_within, shell_join,
    shell_quote, terminal_argv,
};
use std::path::PathBuf;

fn persona(dir: &std::path::Path) -> Persona {
    Persona {
        id: "work".into(),
        name: "Work".into(),
        host: "github.example.invalid".into(),
        commit_name: "T".into(),
        commit_email: "t@example.invalid".into(),
        github_auth: GithubAuth::Pat,
        claude_dir: dir.join("claude dir").display().to_string(),
        gh_dir: dir.join("gh").display().to_string(),
        ready: false,
        claude: None,
        github: None,
        tasks: 0,
    }
}

/// An executable script with `content`, written by a child `sh`, not this process: exec'ing a file fails with ETXTBSY
/// while any process holds it open for writing, and a test thread forking during our own write would hand that fd to
/// its child.
fn script(path: &std::path::Path, content: &str) {
    let ok = std::process::Command::new("sh")
        .args([
            "-c",
            "printf '%s' \"$1\" > \"$2\" && chmod 755 \"$2\"",
            "sh",
            content,
        ])
        .arg(path)
        .status()
        .unwrap();
    assert!(ok.success(), "cannot write {}", path.display());
}

fn fake_gh() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fleet-daemon/tests/bin/gh")
        .display()
        .to_string()
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
fn pat_goes_on_stdin_only() {
    let dir = tempfile::tempdir().unwrap();
    let p = persona(dir.path());
    pat_login(&p, &fake_gh(), "  ghp_secret123\n").unwrap();
    assert!(std::path::Path::new(&p.gh_dir).join("hosts.yml").exists());
    let argv = std::fs::read_to_string(std::path::Path::new(&p.gh_dir).join("argv.log")).unwrap();
    assert!(
        argv.contains(
            "auth login --hostname github.example.invalid --with-token --insecure-storage"
        ),
        "{argv}"
    );
    assert!(!argv.contains("ghp_secret123"), "{argv}");
}

#[test]
fn pat_error_masks_token() {
    let dir = tempfile::tempdir().unwrap();
    let p = persona(dir.path());
    // An empty token makes the fake fail; a gh that echoes the token back must come out masked.
    let echo = dir.path().join("gh-echo");
    script(
        &echo,
        "#!/bin/sh\ntoken=$(cat)\necho \"bad credential $token\" >&2\nexit 1\n",
    );
    let err = pat_login(&p, &echo.display().to_string(), "ghp_secret123").unwrap_err();
    assert!(!err.contains("ghp_secret123"), "{err}");
    assert!(err.contains("bad credential ***"), "{err}");
    let err = pat_login(&p, &fake_gh(), "  ").unwrap_err();
    assert!(err.contains("no token"), "{err}");
}

#[test]
fn pat_login_ignores_an_ambient_token() {
    let dir = tempfile::tempdir().unwrap();
    let p = persona(dir.path());
    let gh = dir.path().join("gh-env");
    script(
        &gh,
        "#!/bin/sh\ncat >/dev/null\n[ -z \"$GH_TOKEN\" ] || { echo leaked >&2; exit 1; }\n",
    );
    // The only test that touches GH_TOKEN; the previous value is put back so the process is left as found.
    let before = std::env::var_os("GH_TOKEN");
    std::env::set_var("GH_TOKEN", "ambient");
    let r = pat_login(&p, &gh.display().to_string(), "ghp_x");
    match before {
        Some(v) => std::env::set_var("GH_TOKEN", v),
        None => std::env::remove_var("GH_TOKEN"),
    }
    r.unwrap();
}

#[test]
fn pat_login_times_out_without_leaking_the_token() {
    let dir = tempfile::tempdir().unwrap();
    let p = persona(dir.path());
    // The fake sleeps when FAKE_GH_SLEEP is set; a wrapper sets it for this one call only.
    let gh = dir.path().join("gh-slow");
    script(
        &gh,
        &format!("#!/bin/sh\nFAKE_GH_SLEEP=5 exec {} \"$@\"\n", fake_gh()),
    );
    let start = std::time::Instant::now();
    let err = pat_login_within(
        &p,
        &gh.display().to_string(),
        "ghp_secret123",
        std::time::Duration::from_secs(1),
    )
    .unwrap_err();
    assert!(start.elapsed() < std::time::Duration::from_secs(4));
    assert_eq!(err, "gh did not answer in 1 s");
}

#[test]
fn shell_join_quotes_each_word() {
    assert_eq!(
        shell_join(&strs(&[
            "tmux",
            "-L",
            "fleet manager",
            "attach",
            "-t",
            "it's"
        ])),
        "tmux -L 'fleet manager' attach -t 'it'\\''s'"
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
