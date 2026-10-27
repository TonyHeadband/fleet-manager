use std::path::Path;
use std::process::Command;

#[test]
fn agent_without_a_matching_scenario_says_why_on_the_pane_and_fails() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("PROMPT.md"), "Mow the lawn\n").unwrap();
    let settings = dir.path().join("settings.json");
    std::fs::write(
        &settings,
        r#"{"hooks":{"PreToolUse":[{"hooks":[{"type":"http","url":"http://127.0.0.1:9/hook"}]}]}}"#,
    )
    .unwrap();
    let shot = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../demo/shots/pie");

    // Lingers 3 s on failure so tmux shows the reason before closing the pane.
    let out = Command::new(env!("CARGO_BIN_EXE_fleet-demo-agent"))
        .current_dir(dir.path())
        .env("FLEET_TOKEN", "t")
        .arg("--shot")
        .arg(&shot)
        .arg("--settings")
        .arg(&settings)
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("matches the prompt \"Mow the lawn\""),
        "{stdout}"
    );
}
