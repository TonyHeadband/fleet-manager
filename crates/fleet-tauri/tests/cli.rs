#[test]
fn the_binary_prints_its_version() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fleet-gui-tauri"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("fleet-gui-tauri {}", env!("CARGO_PKG_VERSION"))
    );
}
