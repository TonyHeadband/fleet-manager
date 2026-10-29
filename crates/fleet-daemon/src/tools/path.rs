//! The user's real PATH: what a fresh login would see, not what this daemon inherited at start.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Where the freshly computed PATH comes from; tests inject their own.
pub trait PathSource: Send + Sync {
    fn user_path(&self) -> Option<OsString>;
}

/// Reads the registry on Windows, asks the login shell on Linux.
pub struct SystemPath;

impl PathSource for SystemPath {
    fn user_path(&self) -> Option<OsString> {
        #[cfg(windows)]
        {
            windows_path()
        }
        #[cfg(unix)]
        {
            unix_path()
        }
    }
}

/// `fresh` entries first, then the `current` ones not already present, then (Linux) `~/.local/bin`. An entry that
/// cannot be written into a PATH (a `:` on Linux) leaves `current` as it was rather than no PATH at all.
pub fn merged_path(current: Option<OsString>, fresh: Option<OsString>, home: &Path) -> OsString {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut add = |p: PathBuf| {
        if !p.as_os_str().is_empty() && !out.contains(&p) {
            out.push(p);
        }
    };
    for list in [fresh, current.clone()].into_iter().flatten() {
        std::env::split_paths(&list).for_each(&mut add);
    }
    if cfg!(unix) {
        add(home.join(".local/bin"));
    }
    std::env::join_paths(out).unwrap_or_else(|_| current.unwrap_or_default())
}

/// Printed on both sides of `$PATH`, so whatever the login profile prints (a greeting, `fortune`) or a logout
/// script prints after it is not taken for it.
const PATH_MARKER: &str = "__fleet_path__";

/// The PATH in a login shell's output: what is between the last two markers.
pub fn path_after_marker(out: &str) -> Option<OsString> {
    let mut parts = out.rsplitn(3, PATH_MARKER);
    let (_after, path, _before) = (parts.next()?, parts.next()?, parts.next()?);
    (!path.is_empty()).then(|| OsString::from(path))
}

#[cfg(unix)]
fn unix_path() -> Option<OsString> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".into());
    login_shell_path(&shell)
}

/// The PATH `shell` has as a login shell, or `None` when it fails or takes over 5 s.
#[cfg(unix)]
pub fn login_shell_path(shell: &str) -> Option<OsString> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let mut child = Command::new(shell)
        .args([
            "-lc",
            &format!(r#"printf '%s%s%s' '{PATH_MARKER}' "$PATH" '{PATH_MARKER}'"#),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    let mut out = Vec::new();
    child.stdout.take()?.read_to_end(&mut out).ok()?;
    path_after_marker(&String::from_utf8(out).ok()?)
}

#[cfg(windows)]
fn windows_path() -> Option<OsString> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_NOEXPAND,
        RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
    };

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(Some(0))
            .collect()
    }

    fn read(root: HKEY, key: &str) -> Option<OsString> {
        let key = wide(key);
        let name = wide("Path");
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
        let mut bytes: u32 = 0;
        // SAFETY: both strings are NUL-terminated and outlive the call; a null data pointer asks for the size only.
        let rc = unsafe {
            RegGetValueW(
                root,
                key.as_ptr(),
                name.as_ptr(),
                flags,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut bytes,
            )
        };
        if rc != 0 || bytes < 2 {
            return None;
        }
        let mut raw = vec![0u16; bytes as usize / 2 + 1];
        let mut size = (raw.len() * 2) as u32;
        // SAFETY: `raw` holds `size` bytes and the call writes at most that many.
        let rc = unsafe {
            RegGetValueW(
                root,
                key.as_ptr(),
                name.as_ptr(),
                flags,
                std::ptr::null_mut(),
                raw.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if rc != 0 {
            return None;
        }
        raw.truncate(raw.iter().position(|&c| c == 0).unwrap_or(raw.len()));
        raw.push(0);
        // SAFETY: `raw` is NUL-terminated; the first call (null buffer) returns the needed length in chars.
        let need = unsafe { ExpandEnvironmentStringsW(raw.as_ptr(), std::ptr::null_mut(), 0) };
        if need == 0 {
            return Some(OsString::from_wide(&raw[..raw.len() - 1]));
        }
        let mut expanded = vec![0u16; need as usize];
        // SAFETY: `expanded` has `need` chars, which is what the call is told.
        let got = unsafe { ExpandEnvironmentStringsW(raw.as_ptr(), expanded.as_mut_ptr(), need) };
        if got == 0 || got > need {
            return Some(OsString::from_wide(&raw[..raw.len() - 1]));
        }
        expanded.truncate(got as usize);
        if expanded.last() == Some(&0) {
            expanded.pop();
        }
        Some(OsString::from_wide(&expanded))
    }

    let machine = read(
        HKEY_LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
    );
    let user = read(HKEY_CURRENT_USER, "Environment");
    let mut joined = OsString::new();
    for part in [machine, user].into_iter().flatten() {
        if !joined.is_empty() {
            joined.push(";");
        }
        joined.push(part);
    }
    (!joined.is_empty()).then_some(joined)
}
