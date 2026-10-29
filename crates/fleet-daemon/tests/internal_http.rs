mod common;

use common::net::*;
use common::*;
use fleet_api::{InternalBranches, InternalFile, InternalFiles, InternalHistory};
use fleet_daemon::http::{bind_loopback, router, AppState};
use std::net::SocketAddr;
use std::path::Path;
use std::process::Command;

async fn start() -> (SocketAddr, Fx) {
    let fx = fixture().await;
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(fx.svc.clone(), fx.store.clone(), fx.sessions.clone(), addr);
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    (addr, fx)
}

fn commit(dir: &Path, name: &str, content: &[u8]) {
    let p = dir.join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, content).unwrap();
    for args in [
        vec!["add", name],
        vec!["commit", "-q", "-m", &format!("add {name}")],
    ] {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=T", "-c", "user.email=t@localhost"])
            .args(["-c", "commit.gpgsign=false"])
            .args(&args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// A product whose internal repo holds `notes/a.md` and `b.txt` on main.
async fn seeded() -> (SocketAddr, Fx) {
    let (addr, fx) = start().await;
    let dir = fx.svc.internal().ensure(PRODUCT).await.unwrap();
    commit(&dir, "notes/a.md", b"alpha");
    commit(&dir, "b.txt", b"beta");
    (addr, fx)
}

fn url(tail: &str) -> String {
    format!("/api/products/{PRODUCT}/internal/{tail}")
}

async fn get<T: serde::de::DeserializeOwned>(addr: SocketAddr, tail: &str) -> T {
    let (s, v) = json_of(addr, "GET", &url(tail), None).await;
    assert_eq!(s, 200, "{v}");
    serde_json::from_value(v).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_branches_files_and_reads_a_file() {
    let (addr, _fx) = seeded().await;
    let b: InternalBranches = get(addr, "branches").await;
    assert_eq!(b.branches, ["main"]);
    let f: InternalFiles = get(addr, "files").await;
    assert_eq!(f.branch, "main");
    assert_eq!(
        f.files,
        [
            "README.md",
            "b.txt",
            "docs/README.md",
            "notes/a.md",
            "plans/README.md",
            "scripts/README.md",
            "specs/README.md",
            "tasks/README.md"
        ]
    );
    let r: InternalFile = get(addr, "file?path=notes%2Fa.md").await;
    assert_eq!(
        (r.branch.as_str(), r.path.as_str(), r.text.as_str()),
        ("main", "notes/a.md", "alpha")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn history_of_one_path() {
    let (addr, _fx) = seeded().await;
    let all: InternalHistory = get(addr, "history").await;
    assert_eq!(all.commits.len(), 3);
    assert_eq!(all.commits[0].subject, "add b.txt");
    let one: InternalHistory = get(addr, "history?path=notes%2Fa.md").await;
    assert_eq!(one.commits.len(), 1);
    assert_eq!(one.commits[0].subject, "add notes/a.md");
    let capped: InternalHistory = get(addr, "history?limit=1").await;
    assert_eq!(capped.commits.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_product_is_404() {
    let (addr, _fx) = seeded().await;
    for tail in ["branches", "files", "file?path=b.txt", "history"] {
        let (s, v) = json_of(
            addr,
            "GET",
            &format!("/api/products/nope/internal/{tail}"),
            None,
        )
        .await;
        assert_eq!(s, 404, "{tail}: {v}");
        assert!(v.to_string().contains("nope"), "{v}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_branch_and_path_are_404() {
    let (addr, _fx) = seeded().await;
    for tail in [
        "files?branch=ghost",
        "file?branch=ghost&path=b.txt",
        "history?branch=ghost",
        "file?path=missing.txt",
    ] {
        let (s, v) = json_of(addr, "GET", &url(tail), None).await;
        assert_eq!(s, 404, "{tail}: {v}");
    }
    let (_, v) = json_of(addr, "GET", &url("files?branch=ghost"), None).await;
    assert!(v.to_string().contains("ghost"), "{v}");
}

#[tokio::test(flavor = "multi_thread")]
async fn dotdot_path_is_400() {
    let (addr, _fx) = seeded().await;
    for p in ["..%2Fx", "%2Fetc%2Fpasswd", "notes%2F..%2F..%2Fx"] {
        let (s, v) = json_of(addr, "GET", &url(&format!("file?path={p}")), None).await;
        assert_eq!(s, 400, "{p}: {v}");
    }
    let (s, _) = json_of(addr, "GET", &url("history?path=..%2Fx"), None).await;
    assert_eq!(s, 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn large_file_is_400() {
    let (addr, fx) = seeded().await;
    let dir = fx.svc.internal().repo_dir(PRODUCT);
    commit(&dir, "big.txt", &vec![b'x'; 1_048_577]);
    commit(&dir, "bin.dat", &[0xff, 0xfe, 0x00]);
    let (s, v) = json_of(addr, "GET", &url("file?path=big.txt"), None).await;
    assert_eq!(s, 400, "{v}");
    assert!(v.to_string().contains("1 MiB"), "{v}");
    let (s, v) = json_of(addr, "GET", &url("file?path=bin.dat"), None).await;
    assert_eq!(s, 400, "{v}");
}

#[tokio::test(flavor = "multi_thread")]
async fn product_without_a_repo_lists_nothing() {
    let (addr, fx) = start().await;
    let b: InternalBranches = get(addr, "branches").await;
    assert!(b.branches.is_empty());
    for tail in ["files", "file?path=b.txt", "history"] {
        let (s, v) = json_of(addr, "GET", &url(tail), None).await;
        assert_eq!(s, 404, "{tail}: {v}");
    }
    // A GET never creates the repo.
    assert!(!fx.svc.internal().repo_dir(PRODUCT).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn branches_report_remote_state() {
    let (addr, fx) = seeded().await;
    let b: InternalBranches = get(addr, "branches").await;
    assert_eq!(b.remote.url, None);

    let remotes = tempfile::tempdir().unwrap();
    let bare = remotes.path().join("origin.git");
    git(
        remotes.path(),
        &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()],
    );
    let url = format!("file://{}", bare.display());
    fx.svc
        .internal()
        .connect(PRODUCT, &url, &[], 1)
        .await
        .unwrap();

    let b: InternalBranches = get(addr, "branches").await;

    assert_eq!(b.remote.url.as_deref(), Some(url.as_str()));
    assert_eq!(b.remote.unpushed, 0);
    assert_eq!(b.remote.error, None);
}
