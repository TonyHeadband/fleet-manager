use fleet_client::update::{latest, newer, UpdateState};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// Answers every request with `status` and `body`, and keeps each request's head for the test to read.
fn canned_github(status: &'static str, body: String) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://{}/repos/o/r/releases/latest",
        listener.local_addr().unwrap()
    );
    let heads = Arc::new(Mutex::new(vec![]));
    let seen = heads.clone();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            seen.lock().unwrap().push(head);
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (url, heads)
}

fn release_json(tag: &str) -> String {
    serde_json::json!({
        "tag_name": tag,
        "body": "Faster boards.",
        "html_url": "https://github.com/o/r/releases/tag/v0.2.0",
        "draft": false,
        "prerelease": false,
    })
    .to_string()
}

#[test]
fn newer_compares_versions() {
    assert!(newer("0.1.0", "0.2.0"));
    assert!(newer("0.1.9", "0.1.10"));
    assert!(newer("0.9.0", "1.0.0"));
    assert!(!newer("0.2.0", "0.2.0"));
    assert!(!newer("0.2.0", "0.1.0"));
    assert!(!newer("0.1.0", "0.2.0-rc1"), "a pre-release is never newer");
    assert!(!newer("0.1.0", "not a version"));
}

#[test]
fn the_tag_loses_its_v() {
    let (url, heads) = canned_github("200 OK", release_json("v0.2.0"));
    let release = latest(&url).unwrap();
    assert_eq!(release.version, "0.2.0");
    assert_eq!(release.notes, "Faster boards.");
    assert_eq!(release.page, "https://github.com/o/r/releases/tag/v0.2.0");
    let head = heads.lock().unwrap()[0].to_ascii_lowercase();
    assert!(
        head.contains(&format!(
            "user-agent: fleet-manager/{}",
            env!("CARGO_PKG_VERSION")
        )),
        "GitHub refuses requests without a User-Agent: {head}"
    );
}

#[test]
fn a_rate_limited_answer_is_an_error_not_a_panic() {
    let body = serde_json::json!({"message": "API rate limit exceeded"}).to_string();
    let (url, _) = canned_github("403 Forbidden", body);
    let e = latest(&url).unwrap_err();
    assert!(e.contains("403"), "{e}");
}

#[test]
fn garbage_json_is_an_error() {
    let (url, _) = canned_github("200 OK", "<html>not json</html>".into());
    assert!(latest(&url).is_err());
    let (url, _) = canned_github("200 OK", serde_json::json!({"name": "x"}).to_string());
    assert!(
        latest(&url).is_err(),
        "a release without a tag is no release"
    );
}

#[test]
fn nothing_listening_is_an_error() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    assert!(latest(&format!("http://127.0.0.1:{port}/latest")).is_err());
}

#[test]
fn a_tag_that_is_no_version_is_a_failed_check() {
    for tag in ["nightly", "vv0.2.0", "v0.2"] {
        let (url, _) = canned_github("200 OK", release_json(tag));
        let answer = latest(&url);
        assert!(answer.is_err(), "{tag}: {answer:?}");
        assert_eq!(
            UpdateState::from_answer("0.1.0", &answer),
            UpdateState::Failed,
            "{tag}"
        );
    }
    let nightly = fleet_client::update::Release {
        version: "nightly".into(),
        notes: String::new(),
        page: String::new(),
    };
    assert_eq!(
        UpdateState::from_answer("0.1.0", &Ok(nightly)),
        UpdateState::Failed
    );
}

#[test]
fn a_release_without_notes_has_empty_notes() {
    for body in [serde_json::Value::Null, serde_json::json!("")] {
        let json = serde_json::json!({
            "tag_name": "v0.2.0",
            "body": body,
            "html_url": "https://github.com/o/r/releases/tag/v0.2.0",
        })
        .to_string();
        let (url, _) = canned_github("200 OK", json);
        let release = latest(&url).unwrap();
        assert_eq!(release.notes, "");
        assert!(matches!(
            UpdateState::from_answer("0.1.0", &Ok(release)),
            UpdateState::Available(_)
        ));
    }
}
