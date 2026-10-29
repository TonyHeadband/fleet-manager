//! Alone in its binary: it sets the process env, which every other test in the same process would see.
use fleet_daemon::internal::Internal;

/// A daemon started where a commit identity is set in the env (from an agent's shell, under the `open_pr` gate) still
/// signs its own commits as itself: git prefers `GIT_AUTHOR_*` and `GIT_COMMITTER_*` to `user.name`.
#[tokio::test]
async fn the_daemons_own_commits_keep_its_identity_whatever_the_env() {
    for (k, v) in [
        ("GIT_AUTHOR_NAME", "Someone"),
        ("GIT_AUTHOR_EMAIL", "someone@example.com"),
        ("GIT_COMMITTER_NAME", "Someone"),
        ("GIT_COMMITTER_EMAIL", "someone@example.com"),
    ] {
        std::env::set_var(k, v);
    }
    let tmp = tempfile::tempdir().unwrap();
    let i = Internal::new(tmp.path().join("internal"));
    i.ensure("p").await.unwrap();
    let log = i.history("p", "main", None, 10).await.unwrap();
    assert_eq!(log[0].author, "Fleet Manager");
}
