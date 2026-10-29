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
    let has = |name: &str| dir.as_ref().is_some_and(|d| d.join(name).exists());
    // FAKE_GH_SLEEP=<s> makes a call hang, for timeout tests.
    if let Ok(s) = std::env::var("FAKE_GH_SLEEP") {
        if !s.is_empty() {
            std::thread::sleep(std::time::Duration::from_secs_f64(s.parse().unwrap_or(0.0)));
            exit(0);
        }
    }
    let token = std::env::var("GH_TOKEN").unwrap_or_default();
    match (a(0), a(1)) {
        ("auth", "status") => {
            if has("hosts.yml") || !token.is_empty() {
                println!("Logged in to {} account tester", a(3));
                exit(0);
            }
            eprintln!("You are not logged into any GitHub hosts. To log in, run: gh auth login");
            exit(1);
        }
        ("auth", "git-credential") => exit(1),
        ("auth", "login") => {
            let mut stdin = String::new();
            let _ = std::io::stdin().read_to_string(&mut stdin);
            let token = stdin.trim_end_matches('\n');
            if !token.is_empty() {
                let d = dir.expect("GH_CONFIG_DIR");
                std::fs::create_dir_all(&d).unwrap();
                std::fs::write(d.join("hosts.yml"), format!("hosts: {}\n", a(4))).unwrap();
                exit(0);
            }
            eprintln!("no token on stdin");
            exit(1);
        }
        // The PR open_pr asks for; the argv in argv.log is what a test checks. A `pr-empty` file in the config dir
        // makes it succeed without printing the URL.
        ("pr", "create") => {
            if !has("pr-empty") {
                println!("https://example.test/pr/1");
            }
            exit(0);
        }
        // A label named `missing` is one the remote does not have; any other is added.
        ("pr", "edit") => {
            if args
                .windows(2)
                .any(|w| w[0] == "--add-label" && w[1] == "missing")
            {
                eprintln!("could not add label: 'missing' not found");
                exit(1);
            }
            exit(0);
        }
        // The merge watcher's query: replies with `pr-list.json` from the config dir (no PR when it is missing), and
        // fails like a GitHub outage when a `pr-fail` file is there. `pr-list-<repo>.json` and `pr-fail-<repo>`,
        // keyed by the last part of `-R`, do the same for that repo only and win over the shared ones. A `pr-hang`
        // file makes it never answer (30 s); with `pr-hold` it writes `asked` and waits for a `go` file first.
        ("pr", "list") => {
            if has("pr-hang") {
                std::thread::sleep(std::time::Duration::from_secs(30));
            }
            if let Some(d) = dir.as_ref().filter(|_| has("pr-hold")) {
                std::fs::write(d.join("asked"), "").unwrap();
                for _ in 0..400 {
                    if has("go") {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
            let repo = args
                .windows(2)
                .filter(|w| w[0] == "-R")
                .last()
                .map(|w| w[1].rsplit('/').next().unwrap_or("").to_string())
                .unwrap_or_default();
            if has("pr-fail") || (!repo.is_empty() && has(&format!("pr-fail-{repo}"))) {
                eprintln!("HTTP 502: Bad Gateway (https://api.github.com/graphql)");
                exit(1);
            }
            let d = dir.unwrap_or_default();
            let own = d.join(format!("pr-list-{repo}.json"));
            let shared = d.join("pr-list.json");
            if !repo.is_empty() && own.exists() {
                print!("{}", std::fs::read_to_string(own).unwrap());
            } else if shared.exists() {
                print!("{}", std::fs::read_to_string(shared).unwrap());
            } else {
                println!("[]");
            }
            exit(0);
        }
        _ => {
            eprintln!("fake gh: unsupported: {}", args.join(" "));
            exit(2);
        }
    }
}
