use fleet_client::shortcuts::{self, Places, ShortcutState, Which};
use std::path::{Path, PathBuf};

const BOTH: Which = Which {
    menu: true,
    desktop: true,
};

struct Fx {
    _tmp: tempfile::TempDir,
    places: Places,
    exe: PathBuf,
}

/// Temp places and a stand-in exe; nothing here touches the real Start menu or Desktop.
fn fx() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let exe = tmp.path().join("app").join(exe_name());
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"x").unwrap();
    let places = Places {
        menu: tmp.path().join("menu"),
        desktop: tmp.path().join("desktop"),
    };
    Fx {
        _tmp: tmp,
        places,
        exe,
    }
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "fleet-gui.exe"
    } else {
        "fleet-gui"
    }
}

fn names(dir: &Path) -> Vec<String> {
    match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => vec![],
    }
}

#[test]
fn add_then_state_is_current() {
    let f = fx();
    assert_eq!(
        shortcuts::state(&f.places, &f.exe, shortcuts::EGUI),
        ShortcutState::None
    );
    shortcuts::add(&f.places, BOTH, &f.exe).unwrap();
    assert_eq!(
        shortcuts::state(&f.places, &f.exe, shortcuts::EGUI),
        ShortcutState::Current {
            menu: true,
            desktop: true
        }
    );
    shortcuts::remove(
        &f.places,
        Which {
            menu: false,
            desktop: true,
        },
        shortcuts::EGUI,
    )
    .unwrap();
    assert_eq!(
        shortcuts::state(&f.places, &f.exe, shortcuts::EGUI),
        ShortcutState::Current {
            menu: true,
            desktop: false
        }
    );
}

#[test]
fn remove_leaves_nothing() {
    let f = fx();
    shortcuts::add(&f.places, BOTH, &f.exe).unwrap();
    shortcuts::remove(&f.places, BOTH, shortcuts::EGUI).unwrap();
    assert!(names(&f.places.menu).is_empty() && names(&f.places.desktop).is_empty());
    assert_eq!(
        shortcuts::state(&f.places, &f.exe, shortcuts::EGUI),
        ShortcutState::None
    );
    // Removing what is not there is not an error.
    shortcuts::remove(&f.places, BOTH, shortcuts::EGUI).unwrap();
}

#[test]
fn a_moved_folder_makes_them_stale() {
    let f = fx();
    shortcuts::add(&f.places, BOTH, &f.exe).unwrap();
    let moved = f.exe.with_file_name(format!("moved-{}", exe_name()));
    std::fs::write(&moved, b"x").unwrap();
    assert_eq!(
        shortcuts::state(&f.places, &moved, shortcuts::EGUI),
        ShortcutState::Stale
    );
    // Adding again for the new exe makes them current.
    shortcuts::add(
        &f.places,
        shortcuts::existing(&f.places, shortcuts::EGUI),
        &moved,
    )
    .unwrap();
    assert_eq!(
        shortcuts::state(&f.places, &moved, shortcuts::EGUI),
        ShortcutState::Current {
            menu: true,
            desktop: true
        }
    );
}

#[cfg(unix)]
#[test]
fn the_desktop_file_runs_this_exe() {
    use std::os::unix::fs::PermissionsExt;
    let f = fx();
    // A path that needs quoting.
    let spaced = f.exe.with_file_name("my app");
    std::fs::create_dir_all(&spaced).unwrap();
    let exe = spaced.join("fleet-gui");
    std::fs::write(&exe, b"x").unwrap();
    std::fs::write(spaced.join("fleet-manager.svg"), b"<svg/>").unwrap();
    shortcuts::add(&f.places, BOTH, &exe).unwrap();
    let file = f.places.desktop.join(shortcuts::EGUI.file);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains(&format!("Exec=\"{}\"\n", exe.display())),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "Icon={}\n",
            spaced.join("fleet-manager.svg").display()
        )),
        "{text}"
    );
    // Until egui retires, the Tauri app is "Fleet Manager" and egui is named apart from it.
    assert!(text.contains("Name=Fleet Manager (egui)\n"), "{text}");
    assert!(
        text.contains("StartupWMClass=fleet-manager-egui\n"),
        "{text}"
    );
    assert!(!text.contains("@EXEC@"));
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(
        shortcuts::state(&f.places, &exe, shortcuts::EGUI),
        ShortcutState::Current {
            menu: true,
            desktop: true
        }
    );
}

#[cfg(unix)]
#[test]
fn without_an_svg_the_icon_is_the_theme_name() {
    let f = fx();
    shortcuts::add(&f.places, BOTH, &f.exe).unwrap();
    let text = std::fs::read_to_string(f.places.menu.join(shortcuts::EGUI.file)).unwrap();
    assert!(text.contains("Icon=fleet-manager\n"), "{text}");
}

#[cfg(windows)]
#[test]
fn the_lnk_targets_this_exe() {
    let f = fx();
    shortcuts::add(&f.places, BOTH, &f.exe).unwrap();
    let lnk = std::fs::read(f.places.menu.join("Fleet Manager.lnk")).unwrap();
    // A shell link header, then the exe as UTF-16 (the shortcut's icon, so also what it starts).
    assert_eq!(&lnk[..4], &[0x4c, 0, 0, 0]);
    let exe: Vec<u8> = f
        .exe
        .to_string_lossy()
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    assert!(lnk.windows(exe.len()).any(|w| w == exe.as_slice()));
}

#[cfg(windows)]
#[test]
fn a_shortcut_under_the_old_name_is_recognised_and_cleaned_up() {
    let f = fx();
    let old = |dir: &Path| dir.join("fleet-manager.lnk");
    std::fs::create_dir_all(&f.places.menu).unwrap();
    std::fs::write(old(&f.places.menu), b"old").unwrap();
    // Seen, and stale, so Update rewrites it.
    assert_eq!(shortcuts::existing(&f.places, shortcuts::EGUI).menu, true);
    assert_eq!(
        shortcuts::state(&f.places, &f.exe, shortcuts::EGUI),
        ShortcutState::Stale
    );
    shortcuts::add(
        &f.places,
        shortcuts::existing(&f.places, shortcuts::EGUI),
        &f.exe,
    )
    .unwrap();
    assert_eq!(names(&f.places.menu), vec!["Fleet Manager.lnk".to_string()]);
    // Remove takes the old name too.
    std::fs::write(old(&f.places.menu), b"old").unwrap();
    shortcuts::remove(&f.places, BOTH, shortcuts::EGUI).unwrap();
    assert!(names(&f.places.menu).is_empty());
}

#[cfg(unix)]
#[test]
fn the_tauri_app_gets_its_own_menu_entry() {
    let f = fx();
    let menu = Which {
        menu: true,
        desktop: false,
    };
    shortcuts::add(&f.places, menu, &f.exe).unwrap();
    shortcuts::add_for(&f.places, menu, &f.exe, shortcuts::TAURI).unwrap();
    let mut files = names(&f.places.menu);
    files.sort();
    // egui's entry stays as it was, beside the Tauri app's, which has the app's own name (#411).
    assert_eq!(
        files,
        ["fleet-manager-egui.desktop", "fleet-manager.desktop"]
    );
    let egui = std::fs::read_to_string(f.places.menu.join("fleet-manager-egui.desktop")).unwrap();
    assert!(
        egui.contains("StartupWMClass=fleet-manager-egui\n"),
        "{egui}"
    );
    let tauri = std::fs::read_to_string(f.places.menu.join("fleet-manager.desktop")).unwrap();
    assert!(tauri.contains("Name=Fleet Manager\n"), "{tauri}");
    assert!(tauri.contains("StartupWMClass=fleet-manager\n"), "{tauri}");
    assert!(
        tauri.contains(&format!("Exec=\"{}\"\n", f.exe.display())),
        "{tauri}"
    );
}

const MENU: Which = Which {
    menu: true,
    desktop: false,
};

#[test]
fn a_tauri_removal_leaves_the_egui_entry() {
    let f = fx();
    shortcuts::add(&f.places, MENU, &f.exe).unwrap();
    shortcuts::add_for(&f.places, MENU, &f.exe, shortcuts::TAURI).unwrap();
    shortcuts::remove(&f.places, MENU, shortcuts::TAURI).unwrap();
    assert_eq!(names(&f.places.menu), [shortcuts::EGUI.file]);
}

#[test]
fn state_and_existing_read_the_apps_own_entry() {
    let f = fx();
    shortcuts::add(&f.places, MENU, &f.exe).unwrap();
    assert_eq!(
        shortcuts::existing(&f.places, shortcuts::TAURI),
        Which::default()
    );
    assert_eq!(
        shortcuts::state(&f.places, &f.exe, shortcuts::TAURI),
        ShortcutState::None
    );
    shortcuts::add_for(&f.places, MENU, &f.exe, shortcuts::TAURI).unwrap();
    assert_eq!(
        shortcuts::state(&f.places, &f.exe, shortcuts::TAURI),
        ShortcutState::Current {
            menu: true,
            desktop: false
        }
    );
}

#[cfg(unix)]
#[test]
fn a_tauri_entry_under_its_old_name_is_recognised_and_cleaned_up() {
    let f = fx();
    let old = f.places.menu.join("fleet-manager-tauri.desktop");
    std::fs::create_dir_all(&f.places.menu).unwrap();
    std::fs::write(&old, "[Desktop Entry]\nExec=\"/elsewhere\"\n").unwrap();
    // Seen, and stale, so Update rewrites it under the new name.
    assert_eq!(shortcuts::existing(&f.places, shortcuts::TAURI), MENU);
    assert_eq!(
        shortcuts::state(&f.places, &f.exe, shortcuts::TAURI),
        ShortcutState::Stale
    );
    shortcuts::add_for(
        &f.places,
        shortcuts::existing(&f.places, shortcuts::TAURI),
        &f.exe,
        shortcuts::TAURI,
    )
    .unwrap();
    assert_eq!(names(&f.places.menu), [shortcuts::TAURI.file]);
    // Remove takes the old name too.
    std::fs::write(&old, "old").unwrap();
    shortcuts::remove(&f.places, MENU, shortcuts::TAURI).unwrap();
    assert!(names(&f.places.menu).is_empty());
}
