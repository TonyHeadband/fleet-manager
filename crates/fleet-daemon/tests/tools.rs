use fleet_api::{Os, ToolId};
use fleet_daemon::tools::{at_least, detect_manager, fix, parse_version, Manager};

#[test]
fn windows_uses_winget_only_when_present() {
    assert_eq!(detect_manager(Os::Windows, |_| true), Some(Manager::Winget));
    assert_eq!(detect_manager(Os::Windows, |_| false), None);
}

#[test]
fn linux_picks_the_first_manager_found() {
    assert_eq!(
        detect_manager(Os::Linux, |p| p == "dnf"),
        Some(Manager::Dnf)
    );
    assert_eq!(
        detect_manager(Os::Linux, |p| p == "apt-get" || p == "dnf"),
        Some(Manager::Apt)
    );
    assert_eq!(detect_manager(Os::Linux, |_| false), None);
}

#[test]
fn no_manager_means_link_only_except_claude_on_linux() {
    let gh = fix(ToolId::Gh, Os::Linux, None);
    assert_eq!(gh.install, None);
    assert_eq!(gh.link, "https://cli.github.com/");
    let claude = fix(ToolId::Claude, Os::Linux, None);
    assert!(claude.install.unwrap().contains("claude.ai/install.sh"));
}

#[test]
fn versions_are_parsed_from_each_tool() {
    assert_eq!(
        parse_version(ToolId::Git, "git version 2.43.0\n"),
        Some("2.43.0".into())
    );
    assert_eq!(
        parse_version(ToolId::Git, "git version 2.43.0.windows.1\n"),
        Some("2.43.0".into())
    );
    assert_eq!(
        parse_version(ToolId::Claude, "2.0.14 (Claude Code)\n"),
        Some("2.0.14".into())
    );
    assert_eq!(
        parse_version(
            ToolId::Gh,
            "gh version 2.40.1 (2023-12-13)\nhttps://github.com/cli/cli/releases/tag/v2.40.1\n"
        ),
        Some("2.40.1".into())
    );
    assert_eq!(parse_version(ToolId::Git, "garbage"), None);
    assert_eq!(parse_version(ToolId::Gh, "garbage"), None);
}

#[test]
fn gh_2_39_is_too_old_and_2_40_is_enough() {
    assert!(!at_least("2.39.9", "2.40"));
    assert!(at_least("2.40", "2.40"));
    assert!(at_least("2.100.0", "2.40"));
}

mod path {
    use fleet_daemon::tools::merged_path;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    fn join(parts: &[&str]) -> OsString {
        std::env::join_paths(parts).unwrap()
    }

    fn split(p: &OsString) -> Vec<PathBuf> {
        std::env::split_paths(p).collect()
    }

    #[test]
    fn a_directory_added_after_start_is_found() {
        let merged = merged_path(
            Some(join(&["A", "B"])),
            Some(join(&["A", "C"])),
            Path::new("home"),
        );
        let parts = split(&merged);
        let pos = |s: &str| parts.iter().position(|p| p == Path::new(s));
        assert_eq!(parts.iter().filter(|p| *p == Path::new("A")).count(), 1);
        assert!(pos("C").unwrap() < pos("B").unwrap());
    }

    #[test]
    fn no_fresh_path_keeps_the_current_one() {
        let merged = merged_path(Some(join(&["A", "B"])), None, Path::new("home"));
        let parts = split(&merged);
        assert!(parts.contains(&PathBuf::from("A")));
        assert!(parts.contains(&PathBuf::from("B")));
    }

    #[cfg(unix)]
    #[test]
    fn local_bin_is_always_on_it() {
        let home = Path::new("/home/someone");
        let merged = merged_path(Some(join(&["/usr/bin"])), None, home);
        assert!(split(&merged).contains(&home.join(".local/bin")));
    }

    #[test]
    #[ignore = "reads the real machine PATH"]
    fn smoke_system_path() {
        use fleet_daemon::tools::{PathSource, SystemPath};
        println!("{:?}", SystemPath.user_path());
    }
}

mod service {
    use fleet_api::{Os, ToolError, ToolId, ToolList, ToolStatus};
    use fleet_daemon::tools::{PathSource, Tools};
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    /// A fresh PATH the test sets, and can change between checks.
    struct FakePath(Mutex<Option<OsString>>);

    impl PathSource for FakePath {
        fn user_path(&self) -> Option<OsString> {
            self.0.lock().unwrap().clone()
        }
    }

    /// Puts a copy of a fake CLI in `dir` under `name`, as an installer would.
    fn install(fake: &str, dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let to = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        std::fs::copy(fake, to).unwrap();
    }

    fn install_gh(dir: &Path) {
        install(env!("CARGO_BIN_EXE_fake-gh"), dir, "gh");
    }

    /// The fake gh answers `--version` like git when it is named `git`.
    fn install_git(dir: &Path) {
        install(env!("CARGO_BIN_EXE_fake-gh"), dir, "git");
    }

    fn install_claude(dir: &Path) {
        install(env!("CARGO_BIN_EXE_fake-claude"), dir, "claude");
    }

    /// Tools whose fresh PATH is `dir` only: the daemon's own PATH (which may hold the real CLIs) is left out.
    fn tools(dir: &Path, env: &[(&str, &str)]) -> (Tools, Arc<FakePath>) {
        let source = Arc::new(FakePath(Mutex::new(Some(dir.as_os_str().to_owned()))));
        let t = Tools::new(
            Os::current(),
            source.clone(),
            dir.parent().unwrap().to_path_buf(),
            "gh".into(),
            vec!["claude".into()],
        )
        .with_inherited_path(None)
        .with_env(
            env.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        );
        (t, source)
    }

    fn row(list: &ToolList, id: ToolId) -> &ToolStatus {
        list.tools.iter().find(|t| t.tool == id).unwrap()
    }

    fn bin(t: &tempfile::TempDir) -> PathBuf {
        t.path().join("bin")
    }

    #[tokio::test]
    async fn a_tool_on_the_fresh_path_is_found() {
        let t = tempfile::tempdir().unwrap();
        install_gh(&bin(&t));
        install_git(&bin(&t));
        install_claude(&bin(&t));
        let (tools, _) = tools(&bin(&t), &[]);
        let list = tools.check().await;
        let gh = row(&list, ToolId::Gh);
        assert_eq!(gh.version.as_deref(), Some("2.45.0"), "{gh:?}");
        assert_eq!(gh.error, None);
        assert_eq!(row(&list, ToolId::Git).version.as_deref(), Some("2.43.0"));
        assert_eq!(
            row(&list, ToolId::Claude).version.as_deref(),
            Some("2.0.14")
        );
        assert_eq!(tools.last(), Some(list));
        assert_eq!(
            tools.require(ToolId::Gh).await.unwrap(),
            bin(&t).join(format!("gh{}", std::env::consts::EXE_SUFFIX))
        );
    }

    #[tokio::test]
    async fn a_missing_tool_is_not_installed_with_its_fix() {
        let t = tempfile::tempdir().unwrap();
        install_gh(&bin(&t));
        let (tools, _) = tools(&bin(&t), &[]);
        let list = tools.check().await;
        match &row(&list, ToolId::Claude).error {
            Some(ToolError::NotInstalled { tool, fix }) => {
                assert_eq!(*tool, ToolId::Claude);
                assert!(fix
                    .install
                    .as_deref()
                    .unwrap()
                    .contains("claude.ai/install"));
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            tools.require(ToolId::Claude).await,
            Err(ToolError::NotInstalled {
                tool: ToolId::Claude,
                ..
            })
        ));
        assert!(list.missing_required().any(|s| s.tool == ToolId::Claude));
    }

    #[tokio::test]
    async fn an_old_gh_is_too_old() {
        let t = tempfile::tempdir().unwrap();
        install_gh(&bin(&t));
        let (tools, _) = tools(
            &bin(&t),
            &[("FAKE_GH_VERSION", "gh version 2.30.0 (2023-06-01)")],
        );
        let list = tools.check().await;
        match &row(&list, ToolId::Gh).error {
            Some(ToolError::TooOld { found, needed, .. }) => {
                assert_eq!(found, "2.30.0");
                assert_eq!(needed, "2.40");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(row(&list, ToolId::Gh).version.as_deref(), Some("2.30.0"));
    }

    #[tokio::test]
    async fn a_tool_installed_after_the_first_check_is_found_on_recheck() {
        let t = tempfile::tempdir().unwrap();
        let first = t.path().join("first");
        std::fs::create_dir_all(&first).unwrap();
        let (tools, source) = tools(&first, &[]);
        let list = tools.check().await;
        assert!(matches!(
            row(&list, ToolId::Gh).error,
            Some(ToolError::NotInstalled { .. })
        ));
        // The installer adds a new folder to the user's PATH; the daemon was started before.
        let later = t.path().join("later");
        install_gh(&later);
        *source.0.lock().unwrap() = Some(std::env::join_paths([&first, &later]).unwrap());
        let list = tools.check().await;
        let gh = row(&list, ToolId::Gh);
        assert_eq!(gh.error, None, "{gh:?}");
        assert_eq!(gh.version.as_deref(), Some("2.45.0"));
        assert!(std::env::split_paths(&tools.path()).any(|p| p == later));
    }

    #[tokio::test]
    async fn an_explicit_gh_command_path_is_used_as_is() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(bin(&t)).unwrap();
        let source = Arc::new(FakePath(Mutex::new(Some(bin(&t).into_os_string()))));
        let tools = Tools::new(
            Os::current(),
            source,
            t.path().to_path_buf(),
            env!("CARGO_BIN_EXE_fake-gh").into(),
            vec![env!("CARGO_BIN_EXE_fake-claude").into()],
        )
        .with_inherited_path(None);
        let list = tools.check().await;
        assert_eq!(row(&list, ToolId::Gh).error, None);
        assert_eq!(row(&list, ToolId::Claude).error, None);
        assert_eq!(
            tools.require(ToolId::Claude).await.unwrap(),
            PathBuf::from(env!("CARGO_BIN_EXE_fake-claude"))
        );
    }
}
