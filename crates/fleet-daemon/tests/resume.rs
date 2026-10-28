mod common;
use common::*;
use fleet_api::{BlockedReason, RepoRequest, Status, StopCause};
use fleet_daemon::ingest::ingest;
use serde_json::json;
use std::path::PathBuf;

#[tokio::test]
async fn a_spawn_passes_a_session_id_and_stores_it() {
    let fx = fixture().await;
    let a = spawn_ready(&fx).await;
    let id = fx
        .store
        .claude_session(&a.id)
        .await
        .unwrap()
        .expect("stored");
    assert!(uuid::Uuid::parse_str(&id).is_ok(), "{id}");
    let s = fx.sessions.screen_text(&a.session).unwrap();
    assert!(
        s.contains("arg=--session-id") && s.contains(&format!("arg={id}")),
        "{s}"
    );
}

#[tokio::test]
async fn a_restart_resumes_working_agents_with_the_same_id_and_a_prompt() {
    let fx = fixture().await;
    let a = spawn_ready(&fx).await;
    set_status(&fx, &a.id, Status::Working).await;
    let id = fx.store.claude_session(&a.id).await.unwrap().unwrap();
    let (svc, sessions) = restart(&fx).await;
    svc.resume_interrupted().await.unwrap();
    let s = wait_screen_in(&sessions, &a.session, |s| s.contains("ready")).await;
    assert!(
        s.contains("arg=--resume") && s.contains(&format!("arg={id}")),
        "{s}"
    );
    assert!(
        s.contains(
            "arg=The fleet daemon restarted and interrupted you. Continue where you left off."
        ),
        "{s}"
    );
    assert!(!s.contains("arg=--session-id"), "{s}");
    let a = svc.agent(&a.id).await.unwrap();
    assert_eq!(a.status, Status::Working);
    assert_eq!(a.activity, "resumed after daemon restart");
    sessions.shutdown_all();
}

#[tokio::test]
async fn a_session_start_after_clear_moves_the_stored_id_and_a_restart_resumes_that_one() {
    let fx = fixture().await;
    let a = spawn_ready(&fx).await;
    let token = fx.store.agent_token(&a.id).await.unwrap().unwrap();
    let new_id = uuid::Uuid::new_v4().to_string();
    let start = json!({"hook_event_name": "SessionStart", "source": "clear", "session_id": new_id});
    ingest(&fx.store, &a.id, Some(&token), start, 2_000)
        .await
        .unwrap();
    assert_eq!(
        fx.store.claude_session(&a.id).await.unwrap().as_deref(),
        Some(new_id.as_str())
    );
    // Never a value that could pass for a flag.
    let flag = json!({"hook_event_name": "SessionStart", "source": "resume", "session_id": "--x"});
    ingest(&fx.store, &a.id, Some(&token), flag, 2_001)
        .await
        .unwrap();
    assert_eq!(
        fx.store.claude_session(&a.id).await.unwrap().as_deref(),
        Some(new_id.as_str())
    );
    set_status(&fx, &a.id, Status::Working).await;
    let (svc, sessions) = restart(&fx).await;
    svc.resume_interrupted().await.unwrap();
    let s = wait_screen_in(&sessions, &a.session, |s| s.contains("ready")).await;
    assert!(
        s.contains("arg=--resume") && s.contains(&format!("arg={new_id}")),
        "{s}"
    );
    sessions.shutdown_all();
}

#[tokio::test]
async fn an_agent_interrupted_before_its_first_prompt_starts_afresh() {
    let fx = fixture().await;
    let queued = spawn_ready(&fx).await;
    let at_trust = spawn_ready(&fx).await;
    fx.store
        .update_agent(&at_trust.id, |x| {
            x.status = Status::NeedsYou;
            x.blocked_reason = Some(BlockedReason::Startup);
            x.activity = "waiting at startup prompt".into();
            x.status_since = 1;
        })
        .await
        .unwrap();
    let old = |id| {
        let store = fx.store.clone();
        async move { store.claude_session(id).await.unwrap().unwrap() }
    };
    let (old_queued, old_trust) = (old(&queued.id).await, old(&at_trust.id).await);
    let (svc, sessions) = restart(&fx).await;
    svc.resume_interrupted().await.unwrap();
    for (a, old) in [(&queued, old_queued), (&at_trust, old_trust)] {
        let s = wait_screen_in(&sessions, &a.session, |s| s.contains("ready")).await;
        let new = fx.store.claude_session(&a.id).await.unwrap().unwrap();
        assert_ne!(new, old);
        assert!(
            s.contains("arg=--session-id") && s.contains(&format!("arg={new}")),
            "{s}"
        );
        assert!(
            s.contains("arg=Your task is in "),
            "the prompt pointer: {s}"
        );
        assert!(!s.contains("arg=--resume"), "{s}");
        let after = svc.agent(&a.id).await.unwrap();
        assert_eq!(
            (after.status, after.blocked_reason, after.activity.as_str()),
            (Status::Queued, None, "starting")
        );
        assert!(after.status_since > 1);
    }
    sessions.shutdown_all();
}

#[tokio::test]
async fn a_blocked_agent_resumes_working_with_its_request_cleared() {
    let fx = fixture().await;
    let a = spawn_ready(&fx).await;
    fx.store
        .update_agent(&a.id, |x| {
            x.status = Status::NeedsYou;
            x.blocked_reason = Some(BlockedReason::Permission);
        })
        .await
        .unwrap();
    let (svc, sessions) = restart(&fx).await;
    svc.resume_interrupted().await.unwrap();
    let s = wait_screen_in(&sessions, &a.session, |s| s.contains("ready")).await;
    assert!(s.contains("arg=The fleet daemon restarted"), "{s}");
    let a = svc.agent(&a.id).await.unwrap();
    assert_eq!(
        (a.status, a.blocked_reason, a.activity.as_str()),
        (Status::Working, None, "resumed after daemon restart")
    );
    sessions.shutdown_all();
}

#[tokio::test]
async fn a_finished_agent_resumes_without_a_prompt_and_stays_in_review() {
    let fx = fixture().await;
    let a = spawn_ready(&fx).await;
    set_status(&fx, &a.id, Status::Review).await;
    let (svc, sessions) = restart(&fx).await;
    svc.resume_interrupted().await.unwrap();
    let s = wait_screen_in(&sessions, &a.session, |s| s.contains("ready")).await;
    assert!(!s.contains("arg=The fleet daemon restarted"), "{s}");
    assert_eq!(svc.agent(&a.id).await.unwrap().status, Status::Review);
    sessions.shutdown_all();
}

#[tokio::test]
async fn a_paused_agent_resumes_without_a_prompt_and_stays_paused() {
    let fx = fixture().await;
    let a = spawn_ready(&fx).await;
    fx.store
        .update_agent(&a.id, |x| {
            x.status = Status::Working;
            x.paused = true;
        })
        .await
        .unwrap();
    let (svc, sessions) = restart(&fx).await;
    svc.resume_interrupted().await.unwrap();
    let s = wait_screen_in(&sessions, &a.session, |s| s.contains("ready")).await;
    assert!(!s.contains("arg=The fleet daemon restarted"), "{s}");
    let a = svc.agent(&a.id).await.unwrap();
    assert_eq!(
        (a.status, a.paused, a.activity.as_str()),
        (Status::Working, true, "resumed after daemon restart")
    );
    sessions.shutdown_all();
}

#[tokio::test]
async fn an_agent_without_a_stored_id_is_stopped() {
    let fx = fixture().await;
    let a = spawn_ready(&fx).await;
    set_status(&fx, &a.id, Status::Working).await;
    fx.store.clear_claude_session_for_test(&a.id).await;
    let (svc, sessions) = restart(&fx).await;
    svc.resume_interrupted().await.unwrap();
    let after = svc.agent(&a.id).await.unwrap();
    assert_eq!(after.status, Status::Stopped);
    assert_eq!(after.stop_cause, Some(StopCause::Interrupted));
    assert_eq!(after.activity, "stopped by a daemon restart");
    assert!(!sessions.alive(&a.session));
}

#[tokio::test]
async fn a_resume_that_exits_at_once_is_stopped_with_the_reason_and_not_retried() {
    let fx = fixture_config(|_| {}).await;
    let a = spawn_ready(&fx).await;
    set_status(&fx, &a.id, Status::Working).await;
    let (svc, sessions) = restart_with_env(&fx, &[("FAKE_CLAUDE_EXIT", "1")]).await;
    svc.resume_interrupted().await.unwrap();
    let a = wait_agent(&svc, &a.id, |a| a.status == Status::Stopped).await;
    assert_eq!(a.stop_cause, Some(StopCause::Interrupted));
    assert!(
        a.activity.starts_with("could not resume: "),
        "{}",
        a.activity
    );
    assert!(
        a.activity.contains("ready"),
        "the last screen line is quoted: {}",
        a.activity
    );
    assert!(!sessions.alive(&a.session));
}

#[tokio::test]
async fn a_deleted_task_folder_stops_that_agent_and_the_others_still_resume() {
    let fx = fixture().await;
    let gone = spawn_ready(&fx).await;
    let kept = spawn_ready(&fx).await;
    let (svc, sessions) = restart(&fx).await;
    let workdir = PathBuf::from(&gone.workdir);
    when_released(&workdir, || std::fs::remove_dir_all(&workdir));
    svc.resume_interrupted().await.unwrap();
    let g = svc.agent(&gone.id).await.unwrap();
    assert_eq!(g.status, Status::Stopped);
    assert!(
        g.activity.starts_with("could not resume: "),
        "{}",
        g.activity
    );
    wait_screen_in(&sessions, &kept.session, |s| s.contains("ready")).await;
    sessions.shutdown_all();
}

#[tokio::test]
async fn a_reconcile_after_shutdown_began_leaves_the_agents_to_resume() {
    let fx = fixture().await;
    let a = spawn_ready(&fx).await;
    set_status(&fx, &a.id, Status::Working).await;
    fx.svc.begin_shutdown();
    fx.sessions.shutdown_all();
    fx.svc.reconcile().await.unwrap();
    assert_ne!(agent(&fx, &a.id).await.status, Status::Stopped);
    let (svc, sessions) = restart(&fx).await;
    svc.resume_interrupted().await.unwrap();
    wait_screen_in(&sessions, &a.session, |s| s.contains("arg=--resume")).await;
    sessions.shutdown_all();
}

#[tokio::test]
async fn a_breakdown_awaiting_its_decision_stays_blocked_after_a_resume() {
    let fx = fixture().await;
    let a = spawn_ready(&fx).await;
    fx.store
        .update_agent(&a.id, |x| {
            x.status = Status::NeedsYou;
            x.blocked_reason = Some(BlockedReason::Breakdown);
        })
        .await
        .unwrap();
    let (svc, sessions) = restart(&fx).await;
    svc.resume_interrupted().await.unwrap();
    wait_screen_in(&sessions, &a.session, |s| s.contains("ready")).await;
    let a = svc.agent(&a.id).await.unwrap();
    assert_eq!(
        (a.status, a.blocked_reason, a.activity.as_str()),
        (
            Status::NeedsYou,
            Some(BlockedReason::Breakdown),
            "resumed after daemon restart"
        )
    );
    sessions.shutdown_all();
}

#[tokio::test]
async fn the_resumed_sessions_start_hook_keeps_the_status_and_the_note() {
    let fx = fixture().await;
    let a = spawn_ready(&fx).await;
    set_status(&fx, &a.id, Status::Review).await;
    let (svc, sessions) = restart(&fx).await;
    svc.resume_interrupted().await.unwrap();
    wait_screen_in(&sessions, &a.session, |s| s.contains("ready")).await;
    let token = fx.store.agent_token(&a.id).await.unwrap().unwrap();
    let start = |source: &str| json!({"hook_event_name": "SessionStart", "source": source});
    ingest(&fx.store, &a.id, Some(&token), start("resume"), 2_000)
        .await
        .unwrap();
    let after = svc.agent(&a.id).await.unwrap();
    assert_eq!(
        (after.status, after.activity.as_str()),
        (Status::Review, "resumed after daemon restart")
    );
    ingest(&fx.store, &a.id, Some(&token), start("startup"), 2_001)
        .await
        .unwrap();
    let after = svc.agent(&a.id).await.unwrap();
    assert_eq!(
        (after.status, after.activity.as_str()),
        (Status::Working, "starting")
    );
    sessions.shutdown_all();
}

#[tokio::test]
async fn recover_resumes_before_it_reconciles() {
    let fx = fixture().await;
    let a = spawn_ready(&fx).await;
    set_status(&fx, &a.id, Status::Working).await;
    let (svc, sessions) = restart(&fx).await;
    // A reconcile first would find no session and record a crash.
    svc.recover().await.unwrap();
    let after = svc.agent(&a.id).await.unwrap();
    assert_eq!(
        (after.status, after.activity.as_str()),
        (Status::Working, "resumed after daemon restart")
    );
    assert!(sessions.alive(&a.session));
    sessions.shutdown_all();
}

#[tokio::test]
async fn a_resume_pass_after_shutdown_began_starts_nothing() {
    let fx = fixture().await;
    let a = spawn_ready(&fx).await;
    let (svc, sessions) = restart(&fx).await;
    let before = svc.agent(&a.id).await.unwrap();
    svc.begin_shutdown();
    svc.resume_interrupted().await.unwrap();
    assert!(sessions.list().is_empty());
    assert_eq!(svc.agent(&a.id).await.unwrap(), before);
}

#[tokio::test]
async fn recover_ends_an_interrupted_repo_request_before_it_resumes() {
    let fx = fixture().await;
    let a = spawn_ready(&fx).await;
    let request = || RepoRequest {
        repo: "tools".into(),
        why: "w".into(),
    };
    fx.store
        .update_agent(&a.id, |x| {
            x.status = Status::NeedsYou;
            x.blocked_reason = Some(BlockedReason::RepoAccess);
            x.repo_request = Some(request());
        })
        .await
        .unwrap();
    let (svc, sessions) = restart(&fx).await;
    svc.recover().await.unwrap();
    let s = wait_screen_in(&sessions, &a.session, |s| s.contains("ready")).await;
    assert!(s.contains("arg=The fleet daemon restarted"), "{s}");
    let after = svc.agent(&a.id).await.unwrap();
    // "resumed", not "repo request interrupted": the clear ran first, so it cannot reach a resumed agent.
    assert_eq!(
        (
            after.status,
            after.blocked_reason,
            after.repo_request,
            after.activity.as_str()
        ),
        (Status::Working, None, None, "resumed after daemon restart")
    );
    // A request the resumed agent makes now is live; nothing after the resume ends it.
    fx.store
        .update_agent(&a.id, |x| {
            x.status = Status::NeedsYou;
            x.blocked_reason = Some(BlockedReason::RepoAccess);
            x.repo_request = Some(request());
        })
        .await
        .unwrap();
    svc.reconcile().await.unwrap();
    assert_eq!(
        svc.agent(&a.id).await.unwrap().repo_request,
        Some(request())
    );
    sessions.shutdown_all();
}

#[tokio::test]
async fn a_store_error_clearing_requests_still_resumes_and_reconciles() {
    let fx = fixture().await;
    let a = spawn_ready(&fx).await;
    set_status(&fx, &a.id, Status::Working).await;
    let (svc, sessions) = restart(&fx).await;
    // The first snapshot is clear_interrupted_requests'.
    fx.store.fail_snapshots_for_test(1);
    let _ = svc.recover().await;
    let after = svc.agent(&a.id).await.unwrap();
    assert_eq!(
        (after.status, after.activity.as_str()),
        (Status::Working, "resumed after daemon restart")
    );
    assert!(sessions.alive(&a.session));
    sessions.shutdown_all();
}

#[tokio::test]
async fn a_resume_points_hooks_and_mcp_at_the_current_address() {
    let fx = fixture().await;
    let a = spawn_ready(&fx).await;
    set_status(&fx, &a.id, Status::Working).await;
    let dir = fx.svc.config().state_dir.join("agents").join(&a.id.0);
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap();
    let old = fx.svc.options().base_url.clone();
    assert!(read("settings.json").contains(&old));
    let settings_before: serde_json::Value = serde_json::from_str(&read("settings.json")).unwrap();
    let (svc, sessions) = restart_with(&fx, |o| o.base_url = "http://127.0.0.1:59999".into()).await;
    svc.resume_interrupted().await.unwrap();
    wait_screen_in(&sessions, &a.session, |s| s.contains("ready")).await;
    for f in ["settings.json", "mcp.json"] {
        let text = read(f);
        assert!(!text.contains(&old), "{f}: {text}");
        assert!(
            text.contains("http://127.0.0.1:59999/internal/"),
            "{f}: {text}"
        );
    }
    let settings_after: serde_json::Value = serde_json::from_str(&read("settings.json")).unwrap();
    assert_eq!(
        settings_after["permissions"],
        settings_before["permissions"]
    );
    sessions.shutdown_all();
}
