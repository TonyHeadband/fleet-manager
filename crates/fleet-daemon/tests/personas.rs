mod common;

use common::*;
use fleet_api::{
    Agent, AgentId, GithubAuth, NewPersona, PersonaUpdate, Status, StopCause, TaskState, ToolError,
    ToolId,
};
use fleet_daemon::personas::{
    check_command, normalize_host, persona_env, persona_id, PersonaError, Personas,
};
use fleet_daemon::store::PersonaRow;
use std::path::Path;

fn new_persona(name: &str) -> NewPersona {
    on_host(name, "github.com")
}

fn on_host(name: &str, host: &str) -> NewPersona {
    NewPersona {
        name: name.into(),
        host: host.into(),
        commit_name: "Tester".into(),
        commit_email: "tester@example.com".into(),
        github_auth: GithubAuth::Gh,
    }
}

fn personas(fx: &Fx) -> Personas {
    personas_with(fx, fake_claude(), fake_gh())
}

fn personas_with(fx: &Fx, claude: Vec<String>, gh: String) -> Personas {
    Personas::new(
        fx.store.clone(),
        fx._tmp.path().join("personas"),
        claude,
        gh,
        fx.tasks_root.clone(),
        fx.state_dir.clone(),
        fx.svc.options().tools.clone(),
        fx.svc.options().forge.clone(),
        fx.svc.options().daemon_exe.clone(),
    )
}

fn log_in(p: &fleet_api::Persona, claude: bool, gh: bool) {
    if claude {
        std::fs::write(Path::new(&p.claude_dir).join(".fake-login"), "").unwrap();
    }
    if gh {
        store_token(p);
    }
}

fn orphan(id: &str) -> Agent {
    agent_fixture_with_id(id)
}

fn agent_fixture_with_id(id: &str) -> Agent {
    let mut a: Agent = serde_json::from_value(serde_json::json!({
        "id": id, "goal": "t", "workdir": format!("/w/{id}"), "session": "s",
        "workspaces": [{"repo": "core", "branch": "b", "worktree": "/w/core", "base": "abc", "has_changes": false}],
        "status": "working", "activity": "", "status_since": 1, "has_changes": false, "paused": false,
        "owner": "o", "blocked_reason": null
    }))
    .unwrap();
    a.id = AgentId(id.into());
    a
}

#[tokio::test]
async fn ids_from_loose_names() {
    assert_eq!(persona_id("Acme Corp"), "acme-corp");
    assert_eq!(persona_id("  "), "persona");
    assert_eq!(persona_id("--A  b__c--"), "a-b-c");
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    assert_eq!(
        ps.create(new_persona("Acme Corp")).await.unwrap().id,
        "acme-corp"
    );
    assert_eq!(
        ps.create(new_persona("Acme Corp")).await.unwrap().id,
        "acme-corp-2"
    );
    assert!(matches!(
        ps.create(new_persona("  ")).await,
        Err(PersonaError::BadName)
    ));
}

#[test]
fn hosts_are_normalised() {
    assert_eq!(normalize_host("https://GitHub.com/").unwrap(), "github.com");
    assert_eq!(
        normalize_host("ghe.example.invalid:443").unwrap(),
        "ghe.example.invalid"
    );
    assert_eq!(normalize_host("github.com.").unwrap(), "github.com");
    assert_eq!(
        normalize_host("www.github.com/a/b?x#y").unwrap(),
        "github.com"
    );
    for bad in ["a b", "", "tok@github.com", "https://"] {
        assert!(
            matches!(normalize_host(bad), Err(PersonaError::BadHost(_))),
            "{bad:?}"
        );
    }
}

#[tokio::test]
async fn create_makes_private_folders() {
    let fx = fixture_without_persona().await;
    let p = personas(&fx).create(new_persona("Home")).await.unwrap();
    assert!(!p.ready);
    for d in [&p.claude_dir, &p.gh_dir] {
        let meta = std::fs::metadata(d).unwrap();
        assert!(meta.is_dir());
        // Owner-only is a Unix file mode; Windows has no such bits.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(meta.permissions().mode() & 0o777, 0o700, "{d}");
        }
    }
}

#[tokio::test]
async fn check_marks_ready() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Home")).await.unwrap();
    // An agent with no persona stays without one: nothing is handed over any more.
    fx.store.insert_agent(&orphan("a1"), "t").await.unwrap();
    log_in(&p, true, true);
    let p = ps.check(&p.id).await.unwrap();
    assert!(p.ready);
    assert_eq!(
        p.github.as_ref().unwrap().account.as_deref(),
        Some("fake-user")
    );
    assert_eq!(
        p.claude.as_ref().unwrap().account.as_deref(),
        Some("tester@example.com")
    );
    assert_eq!(p.tasks, 0);
    assert_eq!(fx.store.snapshot().await.unwrap().agents[0].persona, "");
}

#[tokio::test]
async fn check_reports_which_login_is_missing() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Home")).await.unwrap();
    log_in(&p, false, true);
    let p = ps.check(&p.id).await.unwrap();
    let claude = p.claude.unwrap();
    assert!(!claude.ok);
    assert_eq!(claude.detail.as_deref(), Some("not logged in"));
    assert!(p.github.unwrap().ok);
    assert!(!p.ready);
}

#[tokio::test]
async fn ready_env_rejects_draft_and_broken() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Home")).await.unwrap();
    assert!(matches!(
        ps.ready_env(&p.id).await,
        Err(PersonaError::NotReady(_))
    ));
    assert!(matches!(
        ps.ready_env("nope").await,
        Err(PersonaError::NotFound(_))
    ));
    log_in(&p, true, true);
    assert!(ps.check(&p.id).await.unwrap().ready);
    let env = ps.ready_env(&p.id).await.unwrap();
    assert_eq!(env.host, "github.com");
    // A token problem is refused as itself, not as a gh CLI that is not logged in.
    fx.forge.refuse_token("ghp_fake");
    let refused = ps.ready_env(&p.id).await.unwrap_err();
    assert_eq!(refused.to_string(), "the token was refused");
    std::fs::remove_file(token_file(&p)).unwrap();
    let missing = ps.ready_env(&p.id).await.unwrap_err();
    assert!(matches!(missing, PersonaError::NoToken), "{missing:?}");
    assert_eq!(
        missing.to_string(),
        "no GitHub token: set one in the persona"
    );
    // The list still offers a login for it.
    let github = ps.get(&p.id).await.unwrap().github.unwrap();
    assert!(matches!(
        github.tool,
        Some(ToolError::NotLoggedIn {
            tool: ToolId::Gh,
            ..
        })
    ));
}

#[tokio::test]
async fn a_persona_with_a_token_needs_no_gh() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Home")).await.unwrap();
    log_in(&p, true, true);
    assert!(ps.check(&p.id).await.unwrap().ready);
    let without = personas_with(&fx, fake_claude(), "fleet-no-such-gh".into());
    let github = without.check(&p.id).await.unwrap().github.unwrap();
    assert!(github.ok, "{github:?}");
    without.ready_env(&p.id).await.unwrap();
    without.gh_env(&p.id).await.unwrap();
}

#[test]
fn env_has_no_token_and_clears_helpers() {
    let row = PersonaRow {
        id: "home".into(),
        name: "Home".into(),
        host: "github.com".into(),
        commit_name: "T".into(),
        commit_email: "t@e".into(),
        github_auth: GithubAuth::Gh,
        ready: true,
        created_at: 0,
    };
    let env = persona_env(&row, Path::new("/p"), Path::new("/bin/fleet-daemon"));
    assert!(env.vars.iter().all(|(k, _)| !k.contains("TOKEN")));
    let get = |k: &str| {
        env.vars
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(get("GIT_CONFIG_KEY_0"), Some("credential.helper"));
    assert_eq!(get("GIT_CONFIG_VALUE_0"), Some(""));
    assert_eq!(
        get("GIT_CONFIG_KEY_1"),
        Some("credential.https://github.com.helper")
    );
    assert_eq!(
        get("GIT_CONFIG_VALUE_1"),
        Some("!/bin/fleet-daemon git-credential /p/home/forge/token")
    );
    assert!(!get("GIT_CONFIG_VALUE_1")
        .unwrap()
        .contains("auth git-credential"));
    // git runs the helper through `sh -c`.
    let spaced = persona_env(
        &row,
        Path::new("/p q"),
        Path::new("/opt/my tools/it's/daemon"),
    );
    assert!(spaced.vars.contains(&(
        "GIT_CONFIG_VALUE_1".into(),
        r"!'/opt/my tools/it'\''s/daemon' git-credential '/p q/home/forge/token'".into()
    )));
    let under = |leaf: &str| Path::new("/p").join("home").join(leaf);
    assert_eq!(
        get("CLAUDE_CONFIG_DIR").map(Path::new),
        Some(under("claude").as_path())
    );
    assert_eq!(
        get("GH_CONFIG_DIR").map(Path::new),
        Some(under("gh").as_path())
    );
    assert_eq!(get("GIT_TERMINAL_PROMPT"), Some("0"));
    assert_eq!(get("GIT_ASKPASS"), Some(""));
    assert_eq!(get("GIT_ALLOW_PROTOCOL"), Some("https:file"));
    assert_eq!(
        get("GIT_CONFIG_GLOBAL").map(Path::new),
        Some(under("gitconfig").as_path())
    );
    assert_eq!(get("GIT_CONFIG_NOSYSTEM"), Some("1"));
    assert_eq!(env.vars.len(), 17);
}

#[test]
fn git_asks_the_helper_for_the_personas_host() {
    let tmp = tempfile::tempdir().unwrap();
    let personas = tmp.path().join("my personas");
    let row = PersonaRow {
        id: "home".into(),
        name: "Home".into(),
        host: "github.com".into(),
        commit_name: "T".into(),
        commit_email: "t@e".into(),
        github_auth: GithubAuth::Gh,
        ready: true,
        created_at: 0,
    };
    let token = fleet_daemon::forge::token::token_path(&personas, "home");
    std::fs::create_dir_all(token.parent().unwrap()).unwrap();
    std::fs::write(&token, "ghp_secret\n").unwrap();
    let env = persona_env(
        &row,
        &personas,
        Path::new(env!("CARGO_BIN_EXE_fleet-daemon")),
    );
    let fill = |host: &str| {
        use std::io::Write;
        let mut cmd = std::process::Command::new("git");
        cmd.args(["credential", "fill"])
            .env("HOME", tmp.path())
            .envs(env.vars.iter().map(|(k, v)| (k, v)))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let mut child = cmd.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(format!("protocol=https\nhost={host}\n\n").as_bytes())
            .unwrap();
        String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
    };
    assert!(fill("github.com").contains("password=ghp_secret\n"));
    assert!(!fill("example.com").contains("password="));
}

#[test]
fn the_owners_git_config_is_invisible_to_a_persona() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(home.join(".config/git")).unwrap();
    std::fs::write(home.join(".gitconfig"), "[user]\n\tpass = hunter2\n").unwrap();
    std::fs::write(home.join(".config/git/config"), "[user]\n\txdg = hunter3\n").unwrap();
    let row = PersonaRow {
        id: "home".into(),
        name: "Home".into(),
        host: "github.com".into(),
        commit_name: "T".into(),
        commit_email: "t@e".into(),
        github_auth: GithubAuth::Gh,
        ready: true,
        created_at: 0,
    };
    let env = persona_env(
        &row,
        &tmp.path().join("personas"),
        Path::new("/bin/fleet-daemon"),
    );
    let user_keys = |isolated: bool| {
        let mut cmd = std::process::Command::new("git");
        cmd.args(["config", "--get-regexp", r"user\."])
            .env("HOME", &home)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("GIT_CONFIG_GLOBAL")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        if isolated {
            // The persona's file does not exist: git must treat it as empty.
            cmd.envs(env.vars.iter().map(|(k, v)| (k, v)));
        }
        String::from_utf8_lossy(&cmd.output().unwrap().stdout).into_owned()
    };
    let control = user_keys(false);
    assert!(control.contains("user.pass"), "{control}");
    let seen = user_keys(true);
    assert!(!seen.contains("hunter"), "{seen}");
}

#[tokio::test]
async fn an_ambient_token_never_counts_as_a_login() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Home")).await.unwrap();
    // The fake gh, like the real one, takes GH_TOKEN as a login; only this test sets it, and every check strips it.
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            std::env::remove_var("GH_TOKEN");
            std::env::remove_var("ANTHROPIC_API_KEY");
        }
    }
    let _restore = Restore;
    std::env::set_var("GH_TOKEN", "ambient");
    std::env::set_var("ANTHROPIC_API_KEY", "ambient");
    let p = ps.check(&p.id).await.unwrap();
    assert!(!p.github.unwrap().ok);
    assert!(!p.ready);
    let path = std::env::var_os("PATH").unwrap_or_default();
    let cmd = check_command(&fake_gh(), &["x"], "GH_CONFIG_DIR", Path::new("/d"), &path).unwrap();
    let removed: Vec<_> = cmd
        .as_std()
        .get_envs()
        .filter(|(_, v)| v.is_none())
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect();
    for k in [
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
    ] {
        assert!(removed.iter().any(|r| r == k), "{k}");
    }
}

#[tokio::test]
async fn update_keeps_id_and_host_and_missing_is_not_found() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Home")).await.unwrap();
    let up = PersonaUpdate {
        name: Some("Renamed".into()),
        commit_name: None,
        commit_email: Some("n@e".into()),
        github_auth: Some(GithubAuth::Pat),
        nicknames: None,
    };
    let u = ps.update(&p.id, up.clone()).await.unwrap();
    assert_eq!(
        (u.id.as_str(), u.name.as_str(), u.host.as_str()),
        ("home", "Renamed", "github.com")
    );
    assert_eq!(u.commit_email, "n@e");
    assert_eq!(u.github_auth, GithubAuth::Pat);
    assert!(matches!(
        ps.update("nope", up).await,
        Err(PersonaError::NotFound(_))
    ));
    assert!(matches!(
        ps.delete("nope").await,
        Err(PersonaError::NotFound(_))
    ));
    assert!(matches!(
        ps.check("nope").await,
        Err(PersonaError::NotFound(_))
    ));
}

#[tokio::test]
async fn nicknames_are_kept_in_the_persona_folder() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Home")).await.unwrap();
    assert!(p.nicknames.is_empty());
    let nicknames = |pairs: &[(&str, &str)]| PersonaUpdate {
        name: None,
        commit_name: None,
        commit_email: None,
        github_auth: None,
        nicknames: Some(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ),
    };
    let u = ps
        .update(&p.id, nicknames(&[("rust", " Ferris "), ("docs", "  ")]))
        .await
        .unwrap();
    assert_eq!(
        u.nicknames,
        [("rust".to_string(), "Ferris".to_string())].into()
    );
    let listed = ps.list().await.unwrap().personas.remove(0);
    assert_eq!(listed.nicknames, u.nicknames);
    let file = std::path::Path::new(&p.claude_dir)
        .parent()
        .unwrap()
        .join("nicknames.json");
    assert!(file.is_file());

    let err = ps
        .update(&p.id, nicknames(&[("rust", "two\nlines")]))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, PersonaError::BadNickname(s) if s == "rust"),
        "{err:?}"
    );
    assert_eq!(ps.list().await.unwrap().personas[0].nicknames, u.nicknames);

    // A hand edit gone wrong reads as none, and clearing them all removes the file.
    std::fs::write(&file, "not json").unwrap();
    assert!(ps.list().await.unwrap().personas[0].nicknames.is_empty());
    ps.update(&p.id, nicknames(&[])).await.unwrap();
    assert!(!file.exists());
}

/// A persona with one stopped task.
async fn with_stopped_task(fx: &Fx, ps: &Personas) -> fleet_api::Persona {
    let p = ps.create(new_persona("Home")).await.unwrap();
    let mut a = agent_fixture_with_id("a1");
    a.persona = p.id.clone();
    fx.store.insert_agent(&a, "t").await.unwrap();
    fx.store
        .update_agent(&a.id, |a| {
            a.status = Status::Stopped;
            a.stop_cause = Some(StopCause::Exited);
        })
        .await
        .unwrap();
    p
}

#[tokio::test]
async fn delete_refuses_with_tasks() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = with_stopped_task(&fx, &ps).await;
    assert!(matches!(
        ps.delete(&p.id).await,
        Err(PersonaError::HasTasks(1))
    ));
    fx.store
        .archive_task(&AgentId("a1".into()), |p| p.state = TaskState::Archived)
        .await
        .unwrap();
    ps.delete(&p.id).await.unwrap();
    assert!(!Path::new(&p.claude_dir).exists());
    assert!(ps.list().await.unwrap().personas.is_empty());
}

#[tokio::test]
async fn a_persona_deleted_mid_check_is_not_found() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    // What check() does once the CLIs answer, for a persona that was deleted while they ran.
    assert!(!fx.store.set_persona_ready("gone").await.unwrap());
    let p = ps.create(new_persona("Home")).await.unwrap();
    log_in(&p, true, true);
    ps.delete(&p.id).await.unwrap();
    assert!(matches!(
        ps.check(&p.id).await,
        Err(PersonaError::NotFound(_))
    ));
}

/// An npm install puts only a `claude.cmd` shim on PATH, and Windows cannot start a process from a `.cmd`.
#[tokio::test]
async fn the_claude_check_follows_an_npm_shim_to_its_exe() {
    let fx = fixture_without_persona().await;
    let npm = fx._tmp.path().join("npm");
    let bin = npm.join("node_modules/@anthropic-ai/claude-code/bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::copy(&fake_claude()[0], bin.join("claude.exe")).unwrap();
    let shim = npm.join("claude.cmd");
    std::fs::write(&shim, "").unwrap();
    let ps = Personas::new(
        fx.store.clone(),
        fx._tmp.path().join("personas"),
        vec![shim.display().to_string()],
        fake_gh(),
        fx.tasks_root.clone(),
        fx.state_dir.clone(),
        fx.svc.options().tools.clone(),
        fx.svc.options().forge.clone(),
        fx.svc.options().daemon_exe.clone(),
    );
    let p = ps.create(new_persona("Npm")).await.unwrap();
    log_in(&p, true, true);
    let checked = ps.check(&p.id).await.unwrap();
    assert!(checked.ready, "{:?}", checked.claude);
}

#[tokio::test]
async fn check_all_makes_every_logged_in_persona_ready() {
    let fx = fixture_without_persona().await;
    let ps = std::sync::Arc::new(personas(&fx));
    let zed = ps.create(new_persona("Zed")).await.unwrap();
    let amy = ps.create(new_persona("Amy")).await.unwrap();
    let out = ps.create(new_persona("Out")).await.unwrap();
    log_in(&zed, true, true);
    log_in(&amy, true, true);
    log_in(&out, false, true);
    ps.check_all().await.unwrap();
    assert!(ps.get("amy").await.unwrap().ready && ps.get("zed").await.unwrap().ready);
    assert!(!ps.get("out").await.unwrap().ready);
}

#[tokio::test]
async fn delete_removes_its_clones_and_leaves_the_products_repos() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let home = ps.create(new_persona("Home")).await.unwrap();
    log_in(&home, true, true);
    assert!(ps.check(&home.id).await.unwrap().ready);
    let clones = fx.state_dir.join("repos").join("home");
    std::fs::create_dir_all(clones.join(PRODUCT).join("x")).unwrap();
    ps.delete(&home.id).await.unwrap();
    assert!(!clones.exists());
    // Repos are the product's, whoever used them.
    assert!(fx.store.repo(PRODUCT, "core").await.unwrap().is_some());

    // The same name gives the same id; another persona starts from the same repo.
    let again = ps.create(new_persona("Home")).await.unwrap();
    assert_eq!(again.id, "home");
    let other = ps.create(new_persona("Other")).await.unwrap();
    log_in(&other, true, true);
    assert!(ps.check(&other.id).await.unwrap().ready);
    let row = fx.store.persona("other").await.unwrap().unwrap();
    let env = persona_env(
        &row,
        &fx._tmp.path().join("personas"),
        Path::new(env!("CARGO_BIN_EXE_fleet-daemon")),
    );
    fx.repos
        .resolve(PRODUCT, &pick(&fx, "core"), &env)
        .await
        .unwrap();
}

#[tokio::test]
async fn delete_keeps_the_clones_of_archived_tasks() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = with_stopped_task(&fx, &ps).await;
    // A base clone of the persona's, holding the branch an archive kept.
    let clone = make_repo(&fx.state_dir.join("repos").join(&p.id).join("app"));
    git(&clone, &["branch", "fleet/kept-work"]);
    fx.store
        .archive_task(&AgentId("a1".into()), |p| p.state = TaskState::Archived)
        .await
        .unwrap();
    ps.delete(&p.id).await.unwrap();
    assert!(!Path::new(&p.claude_dir).exists());
    assert!(clone.exists());
    assert!(git(&clone, &["branch", "--list", "fleet/kept-work"]).contains("fleet/kept-work"));
}

#[tokio::test]
async fn the_claude_check_runs_the_whole_command() {
    let fx = fixture_without_persona().await;
    // A wrapper: `sh -c 'exec "$0" "$@"' <claude> auth status`. Only its first element would run `sh auth status`.
    let sh = fleet_fs::posix_shell().expect("a POSIX shell");
    let mut claude = vec![
        sh.to_string_lossy().into_owned(),
        "-c".into(),
        r#"exec "$0" "$@""#.into(),
    ];
    claude.extend(fake_claude());
    let ps = Personas::new(
        fx.store.clone(),
        fx._tmp.path().join("personas"),
        claude,
        fake_gh(),
        fx.tasks_root.clone(),
        fx.state_dir.clone(),
        fx.svc.options().tools.clone(),
        fx.svc.options().forge.clone(),
        fx.svc.options().daemon_exe.clone(),
    );
    let p = ps.create(new_persona("Home")).await.unwrap();
    log_in(&p, true, true);
    let p = ps.check(&p.id).await.unwrap();
    assert!(p.claude.as_ref().unwrap().ok, "{:?}", p.claude);
}

/// A database as version 6 left it, with persona `p`, so the next open runs the fresh start.
async fn v6_database(path: &Path) {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    // One connection: another would prepare 006's ALTER against the schema before 005's.
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    for sql in [
        include_str!("../migrations/001_init.sql"),
        include_str!("../migrations/002_repos.sql"),
        include_str!("../migrations/003_projects.sql"),
        include_str!("../migrations/004_personas.sql"),
        include_str!("../migrations/005_fleet.sql"),
        include_str!("../migrations/006_workflow.sql"),
        "PRAGMA user_version = 6",
        "INSERT INTO personas (id, name, host, commit_name, commit_email, github_auth, ready, created_at) VALUES ('p', 'p', 'github.com', 'T', 't@e', 'gh', 1, 1)",
    ] {
        sqlx::raw_sql(sql).execute(&pool).await.unwrap();
    }
    pool.close().await;
}

/// The fresh start drops every task record, so "no archived record" no longer means "no branch worth keeping": a
/// clone from before products (`repos/<persona>/<repo>`) or from before personas (`repos/<id>` itself) stays.
#[tokio::test]
async fn delete_keeps_clones_from_before_products() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("fleet.db");
    v6_database(&db).await;
    let store = std::sync::Arc::new(fleet_daemon::store::Store::open(&db).await.unwrap());
    assert!(store.was_reset());
    store
        .insert_product(&fleet_daemon::store::ProductRow {
            id: "app".into(),
            name: "App".into(),
            default_persona: None,
            internal_url: None,
            created_at: 1,
        })
        .await
        .unwrap();
    let state = tmp.path().join("state");
    let ps = Personas::new(
        store.clone(),
        tmp.path().join("personas"),
        fake_claude(),
        fake_gh(),
        tmp.path().join("tasks"),
        state.clone(),
        fleet_daemon::tools::Tools::new(
            fleet_api::Os::current(),
            std::sync::Arc::new(fleet_daemon::tools::CurrentPath),
            tmp.path().to_path_buf(),
            fake_gh(),
            fake_claude(),
        ),
        std::sync::Arc::new(fleet_daemon::forge::ForgeConfig::default()),
        std::path::PathBuf::from(env!("CARGO_BIN_EXE_fleet-daemon")),
    );
    let old = make_repo(&state.join("repos/p/core"));
    git(&old, &["branch", "fleet/kept-work"]);
    let current = make_repo(&state.join("repos/p/app/web"));
    ps.delete("p").await.unwrap();
    assert!(
        old.join(".git").is_dir(),
        "the pre-product clone was deleted"
    );
    assert!(git(&old, &["branch", "--list", "fleet/kept-work"]).contains("fleet/kept-work"));
    assert!(!current.exists(), "the clone of a product stayed");

    // A clone from before products whose repo is named like a product is a repo, not a product folder: it stays.
    let named = ps.create(new_persona("Named")).await.unwrap();
    let same = make_repo(&state.join("repos").join(&named.id).join("app"));
    git(&same, &["branch", "fleet/kept-work"]);
    ps.delete(&named.id).await.unwrap();
    assert!(
        same.join(".git").is_dir(),
        "the old clone named like a product was deleted"
    );
    assert!(git(&same, &["branch", "--list", "fleet/kept-work"]).contains("fleet/kept-work"));

    // A clone from before personas named like the persona's id is the whole folder: nothing in it goes.
    let legacy = ps.create(new_persona("Legacy")).await.unwrap();
    let dir = make_repo(&state.join("repos").join(&legacy.id));
    std::fs::create_dir_all(dir.join("app")).unwrap();
    ps.delete(&legacy.id).await.unwrap();
    assert!(dir.join(".git").is_dir() && dir.join("app").is_dir());
}

/// A login terminal runs the CLIs where the daemon found them, so it needs no PATH of its own.
#[tokio::test]
async fn the_list_says_where_the_daemon_found_each_cli() {
    let fx = fixture_without_persona().await;
    let list = personas(&fx).list().await.unwrap();
    assert_eq!(list.claude_path.as_deref(), Some(fake_claude()[0].as_str()));
    assert_eq!(list.gh_path.as_deref(), Some(fake_gh().as_str()));
    let missing = personas_with(
        &fx,
        vec!["fleet-no-such-claude".into()],
        "fleet-no-such-gh".into(),
    );
    let list = missing.list().await.unwrap();
    assert_eq!(
        (list.claude_path.as_ref(), list.gh_path.as_ref()),
        (None, None)
    );
    assert_eq!(
        list.claude_login_command(),
        vec!["fleet-no-such-claude".to_string()]
    );
    assert_eq!(list.gh_login_program(), "fleet-no-such-gh");
}

/// A CLI that is there but will not start is a tool problem (logging in cannot help); one that answers what the
/// check cannot read is a broken login.
#[tokio::test]
async fn a_cli_that_cannot_start_is_broken_and_an_unreadable_answer_is_a_broken_login() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Home")).await.unwrap();
    log_in(&p, true, true);
    assert!(ps.check(&p.id).await.unwrap().ready);

    let junk = fx
        ._tmp
        .path()
        .join(format!("junk{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&junk, "not a program").unwrap();
    // gh is only read for a token, so one that cannot start costs a persona with a stored token nothing.
    let cannot_start = personas_with(&fx, fake_claude(), junk.to_string_lossy().into_owned());
    assert!(cannot_start.check(&p.id).await.unwrap().github.unwrap().ok);
    cannot_start.gh_env(&p.id).await.unwrap();

    // gh standing in for claude: `auth status` prints no JSON.
    let unreadable = personas_with(&fx, vec![fake_gh()], fake_gh());
    let claude = unreadable.check(&p.id).await.unwrap().claude.unwrap();
    assert!(!claude.ok);
    assert_eq!(claude.tool, None, "{claude:?}");
    let refused = unreadable.ready_env(&p.id).await;
    assert!(
        matches!(
            &refused,
            Err(PersonaError::LoginBroken {
                which: "Claude",
                ..
            })
        ),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_token_is_stored_trimmed_and_private() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Home")).await.unwrap();
    ps.set_token(&p.id, "  ghp_x\n").await.unwrap();
    let file = token_file(&p);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "ghp_x");
    assert_eq!(ps.token(&p.id).await.unwrap(), "ghp_x");
    assert_eq!(fx.forge.requests()[0].auth, "Bearer ghp_x");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&file), 0o600);
        assert_eq!(mode(file.parent().unwrap()), 0o700);
    }
}

#[tokio::test]
async fn an_empty_token_is_refused() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Home")).await.unwrap();
    let err = ps.set_token(&p.id, " \n").await.unwrap_err();
    assert_eq!(err.to_string(), "no token given");
    assert!(fx.forge.requests().is_empty());
}

#[tokio::test]
async fn a_token_the_forge_refuses_is_not_stored() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Home")).await.unwrap();
    fx.forge.fail_next(401, "Bad credentials");
    let err = ps.set_token(&p.id, "ghp_bad").await.unwrap_err();
    assert_eq!(err.to_string(), "the token was refused");
    assert!(!token_file(&p).exists());
}

#[tokio::test]
async fn an_existing_gh_login_is_migrated_once() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Home")).await.unwrap();
    std::fs::write(Path::new(&p.gh_dir).join("token"), "ghp_from_gh\n").unwrap();
    assert_eq!(ps.token(&p.id).await.unwrap(), "ghp_from_gh");
    assert_eq!(ps.token(&p.id).await.unwrap(), "ghp_from_gh");
    assert_eq!(
        std::fs::read_to_string(token_file(&p)).unwrap(),
        "ghp_from_gh"
    );
    let log = std::fs::read_to_string(Path::new(&p.gh_dir).join("argv.log")).unwrap();
    assert_eq!(
        log.lines().filter(|l| l.starts_with("auth token")).count(),
        1,
        "{log}"
    );
}

#[tokio::test]
async fn a_persona_without_gh_and_without_a_token_is_logged_out_not_missing_a_tool() {
    let fx = fixture_without_persona().await;
    let without = personas_with(&fx, fake_claude(), "fleet-no-such-gh".into());
    let p = without.create(new_persona("Home")).await.unwrap();
    let github = without.check(&p.id).await.unwrap().github.unwrap();
    assert!(!github.ok);
    assert_eq!(
        github.detail.as_deref(),
        Some("no GitHub token: set one in the persona")
    );
    assert!(matches!(github.tool, Some(ToolError::NotLoggedIn { .. })));
}

#[tokio::test]
async fn a_revoked_token_shows_logged_out_with_the_reason() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Home")).await.unwrap();
    log_in(&p, true, true);
    fx.forge.fail_next(401, "Bad credentials");
    let github = ps.check(&p.id).await.unwrap().github.unwrap();
    assert!(!github.ok);
    assert_eq!(github.detail.as_deref(), Some("the token was refused"));
}

#[tokio::test]
async fn the_login_check_is_a_recorded_call_without_the_token() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Home")).await.unwrap();
    log_in(&p, true, true);
    let github = ps.check(&p.id).await.unwrap().github.unwrap();
    assert_eq!(github.account.as_deref(), Some("fake-user"));
    let calls = fx.store.forge_calls(50).await.unwrap();
    let call = calls.iter().find(|c| c.op == "whoami").expect("a row");
    assert_eq!(
        (call.persona.as_str(), call.repo.as_str()),
        (p.id.as_str(), "github.com//")
    );
    assert!(calls.iter().all(|c| !format!("{c:?}").contains("ghp_fake")));
}

/// For a persona that logs in through gh, the stored token caches gh's: a new gh login replaces it.
#[tokio::test]
async fn a_gh_relogin_is_picked_up() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Home")).await.unwrap();
    log_in(&p, true, false);
    let gh_token = Path::new(&p.gh_dir).join("token");
    std::fs::write(&gh_token, "ghp_one\n").unwrap();
    assert!(ps.check(&p.id).await.unwrap().ready);
    assert_eq!(ps.token(&p.id).await.unwrap(), "ghp_one");

    // Logged in again: gh now holds another token, and the old one is revoked.
    fx.forge.refuse_token("ghp_one");
    std::fs::write(&gh_token, "ghp_two\n").unwrap();
    let github = ps.check(&p.id).await.unwrap().github.unwrap();
    assert!(github.ok, "{github:?}");
    assert_eq!(ps.token(&p.id).await.unwrap(), "ghp_two");
    ps.ready_env(&p.id).await.unwrap();
    let last = fx.forge.requests().pop().unwrap();
    assert_eq!(last.auth, "Bearer ghp_two");
}

/// A token gh now holds that the forge refuses does not replace a stored one it accepts.
#[tokio::test]
async fn a_refused_gh_token_keeps_the_stored_one() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Home")).await.unwrap();
    log_in(&p, true, true);
    std::fs::write(Path::new(&p.gh_dir).join("token"), "ghp_stale").unwrap();
    fx.forge.refuse_token("ghp_stale");
    let github = ps.check(&p.id).await.unwrap().github.unwrap();
    assert!(github.ok, "{github:?}");
    assert_eq!(ps.token(&p.id).await.unwrap(), "ghp_fake");
}

/// A PAT persona's token is the one set for it: gh's is never read once there is one.
#[tokio::test]
async fn a_pat_persona_is_not_overwritten_by_gh() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let mut req = new_persona("Home");
    req.github_auth = GithubAuth::Pat;
    let p = ps.create(req).await.unwrap();
    log_in(&p, true, false);
    ps.set_token(&p.id, "ghp_pat").await.unwrap();
    std::fs::write(Path::new(&p.gh_dir).join("token"), "ghp_gh").unwrap();
    assert!(ps.check(&p.id).await.unwrap().github.unwrap().ok);
    fx.forge.refuse_token("ghp_pat");
    let github = ps.check(&p.id).await.unwrap().github.unwrap();
    assert_eq!(github.detail.as_deref(), Some("the token was refused"));
    assert_eq!(ps.token(&p.id).await.unwrap(), "ghp_pat");
    let log = std::fs::read_to_string(Path::new(&p.gh_dir).join("argv.log")).unwrap_or_default();
    assert!(!log.lines().any(|l| l.starts_with("auth token")), "{log}");
}

#[test]
fn a_migrated_token_never_replaces_one_that_is_there() {
    use fleet_daemon::forge::token::{read_token, write_token, write_token_if_absent};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("forge/token");
    assert!(write_token_if_absent(&path, "from_gh").unwrap());
    write_token(&path, "set_by_user").unwrap();
    assert!(!write_token_if_absent(&path, "from_gh").unwrap());
    assert_eq!(read_token(&path).as_deref(), Some("set_by_user"));
    let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().collect();
    assert_eq!(leftovers.len(), 1, "temp files were left behind");
}
