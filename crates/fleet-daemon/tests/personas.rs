mod common;

use common::*;
use fleet_api::{
    Agent, AgentId, EventKind, GithubAuth, NewPersona, PersonaUpdate, ProjectState, Status,
    StopCause,
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
        fx.projects_root.clone(),
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
        "id": id, "task": "t", "workdir": format!("/w/{id}"), "tmux_session": "s",
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
async fn check_marks_ready_and_adopts() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Home")).await.unwrap();
    fx.store.insert_agent(&orphan("a1"), "t").await.unwrap();
    let mut rx = fx.store.subscribe();
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
    assert_eq!(p.projects, 1);
    let snap = fx.store.snapshot().await.unwrap();
    assert_eq!(snap.agents[0].persona, "home");
    assert_eq!(
        fx.store
            .project(&AgentId("a1".into()))
            .await
            .unwrap()
            .unwrap()
            .persona,
        "home"
    );
    // A live client sees the card leave and come back with its persona.
    assert!(matches!(
        rx.recv().await.unwrap().kind,
        EventKind::AgentRemoved { .. }
    ));
    match rx.recv().await.unwrap().kind {
        EventKind::AgentCreated { agent } => assert_eq!(agent.persona, "home"),
        other => panic!("{other:?}"),
    }
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

/// A persona with one stopped project.
async fn with_stopped_project(fx: &Fx, ps: &Personas) -> fleet_api::Persona {
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
async fn delete_refuses_with_projects() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = with_stopped_project(&fx, &ps).await;
    assert!(matches!(
        ps.delete(&p.id).await,
        Err(PersonaError::HasProjects(1))
    ));
    fx.store
        .archive_project(&AgentId("a1".into()), |p| p.state = ProjectState::Archived)
        .await
        .unwrap();
    ps.delete(&p.id).await.unwrap();
    assert!(!Path::new(&p.claude_dir).exists());
    assert!(ps.list().await.unwrap().personas.is_empty());
}

#[tokio::test]
async fn a_persona_deleted_mid_check_adopts_nothing() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    fx.store.insert_agent(&orphan("a1"), "t").await.unwrap();
    // What check() does once the CLIs answer, for a persona that was deleted while they ran.
    assert_eq!(fx.store.ready_and_adopt("gone", &[]).await.unwrap(), None);
    let snap = fx.store.snapshot().await.unwrap();
    assert_eq!(snap.agents[0].persona, "");
    assert_eq!(
        fx.store
            .project(&AgentId("a1".into()))
            .await
            .unwrap()
            .unwrap()
            .persona,
        ""
    );
    // And through the service: a vanished persona is NotFound, not ready.
    let p = ps.create(new_persona("Home")).await.unwrap();
    log_in(&p, true, true);
    ps.delete(&p.id).await.unwrap();
    assert!(matches!(
        ps.check(&p.id).await,
        Err(PersonaError::NotFound(_))
    ));
}

#[tokio::test]
async fn check_all_gives_orphans_to_the_oldest_persona() {
    let fx = fixture_without_persona().await;
    let ps = std::sync::Arc::new(personas(&fx));
    // "Zed" sorts after "Amy" by id, so only created_at can put it first.
    let zed = ps.create(new_persona("Zed")).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let amy = ps.create(new_persona("Amy")).await.unwrap();
    log_in(&zed, true, true);
    log_in(&amy, true, true);
    fx.store.insert_agent(&orphan("a1"), "t").await.unwrap();
    ps.check_all().await.unwrap();
    assert_eq!(fx.store.snapshot().await.unwrap().agents[0].persona, "zed");
    assert!(ps.get("amy").await.unwrap().ready && ps.get("zed").await.unwrap().ready);
}

async fn owner_of(fx: &Fx, location: &Path) -> Option<String> {
    fx.store
        .repo_by_location(&location.to_string_lossy())
        .await
        .unwrap()
        .and_then(|r| r.persona)
}

#[tokio::test]
async fn a_path_repo_goes_to_the_persona_of_its_origin_host() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let work = make_repo(&fx._tmp.path().join("work"));
    git(
        &work,
        &["remote", "add", "origin", "https://ghe.example.invalid/a/b.git"],
    );
    fx.store
        .seed_repos(&[fleet_daemon::config::Repo {
            name: "work".into(),
            path: work.clone(),
        }])
        .await
        .unwrap();
    let home = ps.create(new_persona("Home")).await.unwrap();
    log_in(&home, true, true);
    assert!(ps.check(&home.id).await.unwrap().ready);
    // `core` has no origin, so it needs no login and goes with the first persona.
    assert_eq!(owner_of(&fx, &fx.repo).await.as_deref(), Some("home"));
    assert_eq!(owner_of(&fx, &work).await, None);
    let acme = ps
        .create(on_host("Acme", "ghe.example.invalid"))
        .await
        .unwrap();
    log_in(&acme, true, true);
    assert!(ps.check(&acme.id).await.unwrap().ready);
    assert_eq!(owner_of(&fx, &work).await.as_deref(), Some("acme"));
}

#[tokio::test]
async fn delete_forgets_its_repos_and_clones() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let home = ps.create(new_persona("Home")).await.unwrap();
    log_in(&home, true, true);
    assert!(ps.check(&home.id).await.unwrap().ready);
    assert_eq!(owner_of(&fx, &fx.repo).await.as_deref(), Some("home"));
    let clones = fx.state_dir.join("repos").join("home");
    std::fs::create_dir_all(clones.join("x")).unwrap();
    ps.delete(&home.id).await.unwrap();
    assert!(!clones.exists());
    assert_eq!(owner_of(&fx, &fx.repo).await, None);

    // The same name gives the same id, and nothing of the old persona comes with it.
    let again = ps.create(new_persona("Home")).await.unwrap();
    assert_eq!(again.id, "home");
    assert!(fx.store.repos_for("home").await.unwrap().is_empty());

    // The path is free for another persona.
    let other = ps.create(new_persona("Other")).await.unwrap();
    log_in(&other, true, true);
    assert!(ps.check(&other.id).await.unwrap().ready);
    let row = fx.store.persona("other").await.unwrap().unwrap();
    let env = persona_env(&row, &fx._tmp.path().join("personas"), &fake_gh());
    fx.repos
        .resolve(&fx.repo.to_string_lossy(), &env)
        .await
        .unwrap();
    assert_eq!(owner_of(&fx, &fx.repo).await.as_deref(), Some("other"));
}

#[tokio::test]
async fn delete_keeps_a_clone_another_repo_lives_in() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = ps.create(new_persona("Legacy")).await.unwrap();
    // A clone from before personas, named like the persona's id and owned by someone else.
    let old = fx.state_dir.join("repos").join("legacy");
    std::fs::create_dir_all(&old).unwrap();
    fx.store
        .insert_repo(&fleet_daemon::store::RepoRow {
            name: "legacy".into(),
            source: fleet_api::RepoSource::Url,
            location: "https://github.com/o/legacy".into(),
            base: old.clone(),
            persona: Some("someone".into()),
        })
        .await
        .unwrap();
    ps.delete(&p.id).await.unwrap();
    assert!(old.exists());
}

#[tokio::test]
async fn delete_keeps_the_clones_of_archived_projects() {
    let fx = fixture_without_persona().await;
    let ps = personas(&fx);
    let p = with_stopped_project(&fx, &ps).await;
    // A base clone of the persona's, holding the branch an archive kept.
    let clone = make_repo(&fx.state_dir.join("repos").join(&p.id).join("app"));
    git(&clone, &["branch", "fleet/kept-work"]);
    fx.store
        .archive_project(&AgentId("a1".into()), |p| p.state = ProjectState::Archived)
        .await
        .unwrap();
    ps.delete(&p.id).await.unwrap();
    assert!(!Path::new(&p.claude_dir).exists());
    assert!(fx.store.repos_for(&p.id).await.unwrap().is_empty());
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
        fx.projects_root.clone(),
        fx.state_dir.clone(),
    );
    let p = ps.create(new_persona("Home")).await.unwrap();
    log_in(&p, true, true);
    let p = ps.check(&p.id).await.unwrap();
    assert!(p.claude.as_ref().unwrap().ok, "{:?}", p.claude);
}
