//! The desktop finds the window's icon through the `.desktop` file named after its app id (#411): the packages'
//! entry, the menu shortcut and the window must all agree on it.
use fleet_client::shortcuts::TAURI;

const DIR: &str = env!("CARGO_MANIFEST_DIR");

fn read(name: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(DIR).join(name)).unwrap()
}

#[cfg(target_os = "linux")]
#[test]
fn the_packages_entry_is_named_after_the_app_id() {
    let linux: serde_json::Value = serde_json::from_str(&read("tauri.linux.conf.json")).unwrap();
    // The bundler names the entry `<productName>.desktop`.
    let product = linux["productName"].as_str().unwrap();
    assert_eq!(format!("{product}.desktop"), TAURI.file);
    assert_eq!(product, TAURI.wm_class);
    let template = linux["bundle"]["linux"]["deb"]["desktopTemplate"]
        .as_str()
        .unwrap();
    assert_eq!(
        linux["bundle"]["linux"]["rpm"]["desktopTemplate"].as_str(),
        Some(template)
    );
    let entry = read(template);
    assert!(
        entry.contains(&format!("\nStartupWMClass={}\n", TAURI.wm_class)),
        "{entry}"
    );
    // productName is only the file's name here: what the menu shows stays the app's name.
    assert!(entry.contains("\nName=Fleet Manager\n"), "{entry}");
}
