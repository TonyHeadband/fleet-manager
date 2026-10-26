mod common;

use common::*;
use fleet_api::{
    Agent, AgentId, GithubAuth, NewPersona, PersonaUpdate, Status, StopCause, TaskState,
};
use fleet_daemon::personas::{
    check_command, normalize_host, persona_env, persona_id, PersonaError, Personas,
};
use fleet_daemon::store::PersonaRow;
use std::os::unix::fs::PermissionsExt;
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
    Personas::new(
        fx.store.clone(),
        fx._tmp.path().join("personas"),
        fake_claude(),
        fake_gh(),
        fx.tasks_root.clone(),
        fx.state_dir.clone(),
    )
}

fn log_in(p: &fleet_api::Persona, claude: bool, gh: bool) {
    if claude {
        std::fs::write(Path::new(&p.claude_dir).join(".fake-login"), "").unwrap();
    }
    if gh {
        std::fs::write(Path::new(&p.gh_dir).join("hosts.yml"), "x").unwrap();
    }
}

fn orphan(id: &str) -> Agent {
    agent_fixture_with_id(id)
}

fn agent_fixture_with_id(id: &str) -> Agent {
    let mut a: Agent = serde_json::from_value(serde_json::json!({
        "id": id, "goal": "t", "workdir": format!("/w/{id}"), "tmux_session": "s",
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
        assert_eq!(meta.permissions().mode() & 0o777, 0o700, "{d}");
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
        Some("tester")
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
    std::fs::remove_file(Path::new(&p.gh_dir).join("hosts.yml")).unwrap();
    match ps.ready_env(&p.id).await {
        Err(PersonaError::LoginBroken { which, persona, .. }) => {
            assert_eq!((which, persona.as_str()), ("GitHub", "Home"));
        }
        other => panic!("{other:?}"),
    }
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
    let env = persona_env(&row, Path::new("/p"), "/bin/gh");
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
        Some("!/bin/gh auth git-credential")
    );
    // git runs the helper through `sh -c`.
    let spaced = persona_env(&row, Path::new("/p"), "/opt/my tools/it's/gh");
    assert!(spaced.vars.contains(&(
        "GIT_CONFIG_VALUE_1".into(),
        r"!'/opt/my tools/it'\''s/gh' auth git-credential".into()
    )));
    assert_eq!(get("CLAUDE_CONFIG_DIR"), Some("/p/home/claude"));
    assert_eq!(get("GH_CONFIG_DIR"), Some("/p/home/gh"));
    assert_eq!(get("GIT_TERMINAL_PROMPT"), Some("0"));
    assert_eq!(get("GIT_ASKPASS"), Some(""));
    assert_eq!(get("GIT_ALLOW_PROTOCOL"), Some("https:file"));
    assert_eq!(env.vars.len(), 15);
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
    let cmd = check_command("gh", &["x"], "GH_CONFIG_DIR", Path::new("/d"));
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
    let env = persona_env(&row, &fx._tmp.path().join("personas"), &fake_gh());
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
    // A wrapper: `env <claude> auth status`. Only its first element would run `env auth status`.
    let mut claude = vec!["env".to_string()];
    claude.extend(fake_claude());
    let ps = Personas::new(
        fx.store.clone(),
        fx._tmp.path().join("personas"),
        claude,
        fake_gh(),
        fx.tasks_root.clone(),
        fx.state_dir.clone(),
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
