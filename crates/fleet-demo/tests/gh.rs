//! fleet-demo-gh answers as demo/bin/gh does, so the smoke test runs the same world on Windows, which cannot run
//! the script as a program.
use std::process::Command;

fn gh(args: &[&str]) -> (Option<i32>, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_fleet-demo-gh"))
        .args(args)
        .output()
        .unwrap();
    (out.status.code(), String::from_utf8(out.stdout).unwrap())
}

#[test]
fn answers_as_the_demo_script() {
    assert_eq!(
        gh(&["--version"]),
        (Some(0), "gh version 2.81.0 (demo)\n".into())
    );
    assert_eq!(
        gh(&["auth", "status"]),
        (Some(0), "Logged in to github.com account baker\n".into())
    );
    assert_eq!(gh(&["auth", "token"]), (Some(0), "ghp_demo\n".into()));
    assert_eq!(
        gh(&["auth", "git-credential", "get"]),
        (Some(1), String::new())
    );
    assert_eq!(gh(&["pr", "list"]), (Some(2), String::new()));
    assert_eq!(gh(&[]), (Some(2), String::new()));
}
