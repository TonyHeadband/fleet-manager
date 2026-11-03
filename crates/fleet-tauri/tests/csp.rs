//! The webview's Content-Security-Policy (#400): the page reaches only its own files, the app's IPC and the daemon,
//! which is always on this machine (the config refuses a `listen` that is not loopback).
use serde_json::Value;

fn csp() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    let conf: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    conf["app"]["security"]["csp"].clone()
}

/// A directive's sources, written as one string or as a list.
fn sources(csp: &Value, directive: &str) -> Vec<String> {
    match &csp[directive] {
        Value::String(s) => s.split_whitespace().map(String::from).collect(),
        Value::Array(a) => a.iter().map(|s| s.as_str().unwrap().to_string()).collect(),
        other => panic!("{directive}: {other}"),
    }
}

#[test]
fn the_page_reaches_only_its_files_the_ipc_and_the_loopback_daemon() {
    let csp = csp();
    assert!(csp.is_object(), "app.security.csp is unset: {csp}");
    assert_eq!(sources(&csp, "default-src"), ["'self'"]);
    let connect = sources(&csp, "connect-src");
    for needed in [
        "ipc:",
        "http://ipc.localhost",
        // The IPC under WebView2's https scheme (tauri.windows.conf.json).
        "https://ipc.localhost",
        "http://127.0.0.1:*",
        "http://localhost:*",
    ] {
        assert!(connect.iter().any(|s| s == needed), "{needed}: {connect:?}");
    }
    assert!(
        connect.iter().all(|s| s == "'self'"
            || s == "ipc:"
            || s == "http://ipc.localhost"
            || s == "https://ipc.localhost"
            || s.starts_with("http://127.")
            || s.starts_with("http://localhost:")
            || s.starts_with("http://[::1]:")),
        "{connect:?}"
    );
    let script = sources(&csp, "script-src");
    assert!(
        !script.iter().any(|s| s.starts_with("'unsafe")),
        "{script:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn the_linux_config_keeps_the_policy() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.linux.conf.json");
    let linux: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(linux["app"]["security"].is_null(), "{linux}");
}

/// On Windows the app is served as `https://tauri.localhost`, the origin the daemon lets in there; plain http, which
/// any local web page could have, is not. The overlay replaces the window list whole, so it keeps the main window.
#[test]
fn the_windows_config_serves_the_app_over_https_and_keeps_the_rest() {
    let read = |name: &str| -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    };
    let (base, windows) = (read("tauri.conf.json"), read("tauri.windows.conf.json"));
    let main = &windows["app"]["windows"][0];
    assert_eq!(main["useHttpsScheme"], true, "{windows}");
    let mut without = main.clone();
    without.as_object_mut().unwrap().remove("useHttpsScheme");
    assert_eq!(without, base["app"]["windows"][0]);
    assert_eq!(windows["app"]["windows"].as_array().unwrap().len(), 1);
    assert!(windows["app"]["security"].is_null(), "{windows}");
}
