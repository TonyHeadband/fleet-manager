use fleet_daemon::session::resolve::{shim_target, ResolveError};

const EXE: &str = "node_modules/@anthropic-ai/claude-code/bin/claude.exe";

#[test]
fn an_npm_shim_resolves_to_the_exe_beside_it_even_under_a_folder_with_spaces() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("Program Files").join("nodejs");
    let exe = dir.join(EXE);
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, "").unwrap();
    std::fs::write(dir.join("claude.cmd"), "").unwrap();
    assert_eq!(shim_target(&dir.join("claude.cmd")).unwrap(), exe);
    assert_eq!(shim_target(&dir.join("claude.ps1")).unwrap(), exe);
}

#[test]
fn a_shim_without_the_exe_says_where_it_looked() {
    let tmp = tempfile::tempdir().unwrap();
    let shim = tmp.path().join("claude.cmd");
    std::fs::write(&shim, "").unwrap();
    let e = shim_target(&shim).unwrap_err();
    assert!(
        matches!(&e, ResolveError::ShimWithoutExe(p) if p == &shim),
        "{e:?}"
    );
    assert_eq!(
        e.to_string(),
        format!(
            "found the claude npm shim at {} but not claude.exe next to it",
            shim.display()
        )
    );
}

#[test]
fn anything_else_runs_as_found() {
    let tmp = tempfile::tempdir().unwrap();
    let exe = tmp.path().join("claude.exe");
    std::fs::write(&exe, "").unwrap();
    assert_eq!(shim_target(&exe).unwrap(), exe);
    let plain = tmp.path().join("claude");
    assert_eq!(shim_target(&plain).unwrap(), plain);
}

#[test]
fn a_missing_program_is_not_found() {
    let e = fleet_daemon::session::resolve::program("no-such-claude-xyz").unwrap_err();
    assert!(matches!(e, ResolveError::NotFound));
    assert_eq!(
        e.to_string(),
        "claude not found: install Claude Code and make sure `claude` runs in a terminal (or set claude_command in config.toml)"
    );
}
