//! The children's PATH is process-wide, so these tests have a test binary of their own.
use fleet_fs::child;
use std::ffi::OsStr;

#[test]
fn a_set_path_reaches_background_commands_and_lookups() {
    let dir = tempfile::tempdir().unwrap();
    let name = "fleet-child-path-probe";
    let exe = dir
        .path()
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&exe, b"").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert_eq!(child::which(name), None);

    child::set_path(dir.path().as_os_str().to_owned());
    assert_eq!(child::which(name), Some(exe));
    let cmd = child::background("git");
    let path = cmd
        .get_envs()
        .find(|(k, _)| *k == OsStr::new("PATH"))
        .and_then(|(_, v)| v);
    assert_eq!(path, Some(dir.path().as_os_str()));
}
