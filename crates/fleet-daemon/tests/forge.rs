mod common;

use common::fake_forge::FakeForge;
use fleet_api::PrState;
use fleet_daemon::forge::{api_base, github, Forge, ForgeConfig, ForgeError, NewPr, RepoRef};
use serde_json::json;
use std::time::Duration;

fn client(fake: &FakeForge, timeout: Duration) -> Box<dyn Forge> {
    let cfg = ForgeConfig {
        api_override: Some(fake.base_url()),
    };
    github("github.com", "t0k".into(), &cfg, timeout)
}

fn repo() -> RepoRef {
    RepoRef::from_url("https://github.com/acme/core").unwrap()
}

fn pr(number: u64, state: &str, merged_at: Option<&str>) -> serde_json::Value {
    json!({
        "html_url": format!("https://github.com/acme/core/pull/{number}"),
        "number": number,
        "state": state,
        "merged_at": merged_at,
    })
}

#[test]
fn api_base_is_api_github_com_or_the_hosts_api_v3() {
    assert_eq!(api_base("github.com"), "https://api.github.com");
    assert_eq!(
        api_base("ghe.example.com"),
        "https://ghe.example.com/api/v3"
    );
}

#[test]
fn repo_ref_keeps_the_host() {
    for url in [
        "https://ghe.example.com/acme/core",
        "https://ghe.example.com/acme/core.git",
        "https://ghe.example.com/acme/core/",
        "https://ghe.example.com/acme/core.git/",
    ] {
        let r = RepoRef::from_url(url).unwrap_or_else(|| panic!("{url}"));
        assert_eq!(
            (r.host.as_str(), r.owner.as_str(), r.name.as_str()),
            ("ghe.example.com", "acme", "core"),
            "{url}"
        );
    }
    assert_eq!(RepoRef::from_url("https://github.com/acme"), None);
    assert_eq!(RepoRef::from_url("https://gitlab.example.com/a/b/c"), None);
}

#[test]
fn requests_carry_the_token_as_bearer_and_the_api_headers() {
    let fake = FakeForge::start();
    let login = client(&fake, Duration::from_secs(5)).whoami().unwrap();
    assert_eq!(login, "fake-user");
    let reqs = fake.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].method, "GET");
    assert_eq!(reqs[0].path_and_query, "/user");
    assert_eq!(reqs[0].auth, "Bearer t0k");
    assert_eq!(reqs[0].accept, "application/vnd.github+json");
    assert_eq!(reqs[0].api_version, "2022-11-28");
    assert_eq!(reqs[0].user_agent, "fleet-manager");
}

#[test]
fn a_branch_with_slash_and_hash_is_percent_encoded() {
    let fake = FakeForge::start();
    fake.set_prs("acme", "core", vec![pr(7, "open", None)]);
    let found = client(&fake, Duration::from_secs(5))
        .open_pr_of(&repo(), "feat/a#1")
        .unwrap();
    assert_eq!(found.unwrap().number, 7);
    let q = &fake.requests()[0].path_and_query;
    assert_eq!(
        q,
        "/repos/acme/core/pulls?head=acme%3Afeat%2Fa%231&state=open&per_page=100"
    );
}

#[test]
fn prs_map_merged_open_and_closed() {
    let fake = FakeForge::start();
    fake.set_prs(
        "acme",
        "core",
        vec![
            pr(1, "open", None),
            pr(2, "closed", Some("1970-01-01T00:01:00Z")),
            pr(3, "closed", None),
        ],
    );
    let prs = client(&fake, Duration::from_secs(5))
        .prs_of(&repo(), "feat")
        .unwrap();
    let got: Vec<_> = prs
        .iter()
        .map(|p| (p.number, p.state, p.merged_at))
        .collect();
    assert_eq!(
        got,
        vec![
            (1, PrState::Open, None),
            (2, PrState::Merged, Some(60)),
            (3, PrState::Closed, None),
        ]
    );
    assert_eq!(prs[0].url, "https://github.com/acme/core/pull/1");
    assert!(fake.requests()[0].path_and_query.contains("state=all"));
}

#[test]
fn create_sends_draft_and_returns_the_html_url() {
    let fake = FakeForge::start();
    let made = client(&fake, Duration::from_secs(5))
        .create_pr(
            &repo(),
            &NewPr {
                head: "feat/x",
                base: "main",
                title: "T",
                body: "B",
                draft: true,
            },
        )
        .unwrap();
    assert_eq!(made.state, PrState::Open);
    assert!(made.url.starts_with("https://github.com/acme/core/pull/"));
    let r = &fake.requests()[0];
    assert_eq!(
        (r.method.as_str(), r.path_and_query.as_str()),
        ("POST", "/repos/acme/core/pulls")
    );
    assert_eq!(
        r.body,
        json!({"title": "T", "body": "B", "head": "feat/x", "base": "main", "draft": true})
    );
}

#[test]
fn update_and_labels_hit_their_endpoints() {
    let fake = FakeForge::start();
    let c = client(&fake, Duration::from_secs(5));
    c.update_pr_text(&repo(), 9, "T2", "B2").unwrap();
    c.add_labels(&repo(), 9, &["a".into(), "b".into()]).unwrap();
    let reqs = fake.requests();
    assert_eq!(
        (reqs[0].method.as_str(), reqs[0].path_and_query.as_str()),
        ("PATCH", "/repos/acme/core/pulls/9")
    );
    assert_eq!(reqs[0].body, json!({"title": "T2", "body": "B2"}));
    assert_eq!(
        (reqs[1].method.as_str(), reqs[1].path_and_query.as_str()),
        ("POST", "/repos/acme/core/issues/9/labels")
    );
    assert_eq!(reqs[1].body, json!({"labels": ["a", "b"]}));
}

#[test]
fn a_401_is_unauthorized_and_a_422_carries_githubs_message() {
    let fake = FakeForge::start();
    let c = client(&fake, Duration::from_secs(5));
    fake.fail_next(401, "Bad credentials");
    assert_eq!(c.whoami(), Err(ForgeError::Unauthorized));
    fake.fail_next(422, "Validation Failed");
    match c.whoami() {
        Err(ForgeError::Refused {
            status: 422,
            message,
        }) => {
            assert!(message.contains("Validation Failed"), "{message}")
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_hung_forge_times_out() {
    let fake = FakeForge::start();
    fake.delay_next(Duration::from_secs(2));
    let c = client(&fake, Duration::from_millis(200));
    assert_eq!(c.whoami(), Err(ForgeError::TimedOut));
}
