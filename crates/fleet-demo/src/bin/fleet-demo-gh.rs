//! Stands in for gh as demo/bin/gh does: a version, logged in, and a token that fleet-demo-forge takes as baker's. A
//! program, not a script, so it runs on Windows too (scripts/tauri-smoke.sh).
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, line) = match args.as_slice() {
        ["--version", ..] => (0, Some("gh version 2.81.0 (demo)")),
        ["auth", "status", ..] => (0, Some("Logged in to github.com account baker")),
        ["auth", "token", ..] => (0, Some("ghp_demo")),
        ["auth", "git-credential", ..] => (1, None),
        _ => (2, None),
    };
    if let Some(line) = line {
        println!("{line}");
    }
    ExitCode::from(code)
}
