//! Start menu and Desktop shortcuts to this exe, for the current user only. Windows writes `.lnk` files, Linux
//! `.desktop` files; both keep the same file name as the old installer's entry, so adding replaces it.
use std::io;
use std::path::{Path, PathBuf};

/// Where the two shortcuts live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Places {
    pub menu: PathBuf,
    pub desktop: PathBuf,
}

/// Which of the two places a call is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Which {
    pub menu: bool,
    pub desktop: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutState {
    None,
    /// Every shortcut that exists points at this exe.
    Current {
        menu: bool,
        desktop: bool,
    },
    /// A shortcut exists but points elsewhere (the folder was moved).
    Stale,
}

#[cfg(windows)]
const FILE: &str = "fleet-manager.lnk";
#[cfg(not(windows))]
const FILE: &str = "fleet-manager.desktop";

/// The current user's Start menu (Linux: app menu) and Desktop folders.
pub fn places() -> Places {
    Places {
        menu: menu_dir(),
        desktop: desktop_dir(),
    }
}

/// Writes the shortcuts asked for, replacing ones already there.
pub fn add(places: &Places, which: Which, exe: &Path) -> io::Result<()> {
    for dir in chosen(places, which) {
        std::fs::create_dir_all(dir)?;
        write(&dir.join(FILE), exe)?;
    }
    Ok(())
}

/// Deletes the shortcuts asked for; one that is not there is fine.
pub fn remove(places: &Places, which: Which) -> io::Result<()> {
    for dir in chosen(places, which) {
        match std::fs::remove_file(dir.join(FILE)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// Which shortcuts exist, whatever they point at.
pub fn existing(places: &Places) -> Which {
    Which {
        menu: places.menu.join(FILE).is_file(),
        desktop: places.desktop.join(FILE).is_file(),
    }
}

pub fn state(places: &Places, exe: &Path) -> ShortcutState {
    let found = existing(places);
    if !found.menu && !found.desktop {
        return ShortcutState::None;
    }
    let points_here = |dir: &Path| target(&dir.join(FILE)).is_some_and(|t| same_path(&t, exe));
    let stale = (found.menu && !points_here(&places.menu))
        || (found.desktop && !points_here(&places.desktop));
    if stale {
        ShortcutState::Stale
    } else {
        ShortcutState::Current {
            menu: found.menu,
            desktop: found.desktop,
        }
    }
}

fn chosen(places: &Places, which: Which) -> impl Iterator<Item = &Path> {
    [
        (which.menu, places.menu.as_path()),
        (which.desktop, places.desktop.as_path()),
    ]
    .into_iter()
    .filter_map(|(on, dir)| on.then_some(dir))
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

#[cfg(unix)]
fn write(file: &Path, exe: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let icon = exe.with_file_name("fleet-manager.svg");
    let icon = if icon.is_file() {
        icon.to_string_lossy().into_owned()
    } else {
        "fleet-manager".to_string()
    };
    let body: String = include_str!("../../../packaging/fleet-manager.desktop.in")
        .lines()
        .map(|line| match line {
            l if l.starts_with("Icon=") => format!("Icon={icon}\n"),
            l => format!("{}\n", l.replace("@EXEC@", &quote_exec(exe))),
        })
        .collect();
    std::fs::write(file, body)?;
    // Some desktops only show a launcher on the Desktop when it is executable.
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755))
}

/// The exe as one `Exec=` argument: quoted, with the characters the Desktop Entry spec escapes in a quoted string.
#[cfg(unix)]
fn quote_exec(exe: &Path) -> String {
    let mut out = String::from("\"");
    for c in exe.to_string_lossy().chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// What `Exec=` runs, with the quoting `quote_exec` adds taken off.
#[cfg(unix)]
fn target(file: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(file).ok()?;
    let value = text.lines().find_map(|l| l.strip_prefix("Exec="))?;
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        out.push(if c == '\\' { chars.next()? } else { c });
    }
    Some(PathBuf::from(out))
}

#[cfg(unix)]
fn menu_dir() -> PathBuf {
    home().join(".local/share/applications")
}

#[cfg(unix)]
fn desktop_dir() -> PathBuf {
    let home = home();
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    std::fs::read_to_string(config.join("user-dirs.dirs"))
        .ok()
        .and_then(|text| desktop_from_user_dirs(&text, &home))
        .unwrap_or_else(|| home.join("Desktop"))
}

/// `XDG_DESKTOP_DIR="$HOME/Desktop"` from `user-dirs.dirs`.
#[cfg(unix)]
fn desktop_from_user_dirs(text: &str, home: &Path) -> Option<PathBuf> {
    let value = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("XDG_DESKTOP_DIR="))?
        .trim()
        .trim_matches('"');
    let path = match value.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(value),
    };
    path.is_absolute().then_some(path)
}

#[cfg(unix)]
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

#[cfg(windows)]
fn write(file: &Path, exe: &Path) -> io::Result<()> {
    let mut link = mslnk::ShellLink::new(exe).map_err(io::Error::other)?;
    // The exe's own embedded icon; reading it back also tells which exe the shortcut starts.
    link.set_icon_location(Some(exe.to_string_lossy().into_owned()));
    link.create_lnk(file).map_err(io::Error::other)
}

/// The icon location of the `.lnk` (see `write`), read from its string data (MS-SHLLINK 2.1 to 2.4).
#[cfg(windows)]
fn target(file: &Path) -> Option<PathBuf> {
    let data = std::fs::read(file).ok()?;
    let u16_at =
        |at: usize| Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?) as usize);
    let flags = u32::from_le_bytes(data.get(20..24)?.try_into().ok()?);
    let mut at = 0x4c;
    if flags & 0x1 != 0 {
        at += 2 + u16_at(at)?;
    }
    if flags & 0x2 != 0 {
        at += u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?) as usize;
    }
    let unicode = flags & 0x80 != 0;
    // Name, relative path, working dir, arguments, icon location: each present one is a counted string.
    for bit in [0x4, 0x8, 0x10, 0x20, 0x40] {
        if flags & bit == 0 {
            continue;
        }
        let count = u16_at(at)?;
        let len = if unicode { count * 2 } else { count };
        let bytes = data.get(at + 2..at + 2 + len)?;
        at += 2 + len;
        if bit == 0x40 {
            let text = if unicode {
                let units: Vec<u16> = bytes
                    .chunks(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                String::from_utf16(&units).ok()?
            } else {
                bytes.iter().map(|&b| b as char).collect()
            };
            return Some(PathBuf::from(text));
        }
    }
    None
}

#[cfg(windows)]
fn menu_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
        .join(r"Microsoft\Windows\Start Menu\Programs")
}

#[cfg(windows)]
fn desktop_dir() -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Desktop, SHGetKnownFolderPath};
    let mut raw: *mut u16 = std::ptr::null_mut();
    // SAFETY: on success `raw` is a NUL-terminated string the shell allocated, freed below after the copy.
    let found = unsafe {
        let hr = SHGetKnownFolderPath(&FOLDERID_Desktop, 0, std::ptr::null_mut(), &mut raw);
        let path = (hr >= 0 && !raw.is_null()).then(|| {
            let len = (0..).take_while(|&i| *raw.add(i) != 0).count();
            std::ffi::OsString::from_wide(std::slice::from_raw_parts(raw, len))
        });
        CoTaskMemFree(raw.cast());
        path
    };
    match found {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join("Desktop"),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn user_dirs_names_the_desktop() {
        let home = Path::new("/home/a");
        let text = "XDG_DOCUMENTS_DIR=\"$HOME/Docs\"\nXDG_DESKTOP_DIR=\"$HOME/Schreibtisch\"\n";
        assert_eq!(
            desktop_from_user_dirs(text, home),
            Some(PathBuf::from("/home/a/Schreibtisch"))
        );
        assert_eq!(
            desktop_from_user_dirs("XDG_DESKTOP_DIR=\"/mnt/d\"", home),
            Some("/mnt/d".into())
        );
        assert_eq!(desktop_from_user_dirs("", home), None);
    }
}
