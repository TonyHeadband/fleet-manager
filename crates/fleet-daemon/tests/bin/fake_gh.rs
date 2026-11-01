//! Test double for the gh CLI. Like the real one it treats GH_TOKEN as a login, so a test can prove the daemon
//! strips ambient tokens before asking. Every call's argv goes to $GH_CONFIG_DIR/argv.log, one line each, so a test
//! can prove a token never rode on the command line.
use std::io::Read;
use std::path::PathBuf;
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
    // FAKE_GH_VERSION sets the line. A copy named `git` stands in for git and answers like it (FAKE_GIT_VERSION).
    if a(0) == "--version" {
        let as_git = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_stem().map(|s| s == "git"))
            .unwrap_or(false);
        let (var, default) = if as_git {
            ("FAKE_GIT_VERSION", "git version 2.43.0")
        } else {
            ("FAKE_GH_VERSION", "gh version 2.45.0 (2024-01-01)")
        };
        println!(
            "{}",
            std::env::var(var).unwrap_or_else(|_| default.to_string())
        );
        exit(0);
    }
    let dir = std::env::var_os("GH_CONFIG_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from);
    if let Some(d) = &dir {
        let _ = std::fs::create_dir_all(d);
        use std::io::Write;
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(d.join("argv.log"))
            .unwrap();
        let _ = writeln!(log, "{}", args.join(" "));
    }
    // The login lives in the config dir's `token` file, which keeps a test off the shared process environment.
    let stored = dir
        .as_ref()
        .and_then(|d| std::fs::read_to_string(d.join("token")).ok());
    let login = std::env::var("GH_TOKEN")
        .ok()
        .or(stored)
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    match (a(0), a(1)) {
        ("auth", "status") => {
            if login.is_some() {
                println!("Logged in to {} account tester", a(3));
                exit(0);
            }
            eprintln!("You are not logged into any GitHub hosts. To log in, run: gh auth login");
            exit(1);
        }
        // The token gh holds for the host, else a failure with no output like gh with no login.
        ("auth", "token") => match login {
            Some(t) => {
                println!("{t}");
                exit(0);
            }
            None => exit(1),
        },
        // `--with-token` reads the token from stdin and stores it, as gh does; a `login-fail` file refuses it.
        ("auth", "login") => {
            let mut stdin = String::new();
            let _ = std::io::stdin().read_to_string(&mut stdin);
            let token = stdin.trim();
            let d = dir.expect("GH_CONFIG_DIR");
            if d.join("login-fail").exists() || token.is_empty() {
                eprintln!("error validating token: HTTP 401: Bad credentials");
                exit(1);
            }
            std::fs::write(d.join("token"), token).unwrap();
            exit(0);
        }
        // The issue a supervisor files; the argv in argv.log is what a test checks.
        ("issue", "create") => {
            println!("https://example.test/issue/1");
            exit(0);
        }
        _ => {
            eprintln!("fake gh: unsupported: {}", args.join(" "));
            exit(2);
        }
    }
}
