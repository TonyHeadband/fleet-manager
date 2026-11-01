//! An in-memory GitHub REST API on 127.0.0.1, for tests of the forge client.
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Recorded {
    pub method: String,
    pub path_and_query: String,
    /// The raw `Authorization` header value.
    pub auth: String,
    pub accept: String,
    pub api_version: String,
    pub user_agent: String,
    pub body: Value,
}

#[derive(Default)]
struct State_ {
    requests: Vec<Recorded>,
    prs: HashMap<(String, String), Vec<Value>>,
    fail: Option<(u16, String)>,
    fail_on: Option<(String, String, u16, String)>,
    delay: Option<Duration>,
    next_number: u64,
}

pub struct FakeForge {
    addr: std::net::SocketAddr,
    state: Arc<Mutex<State_>>,
}

impl FakeForge {
    pub fn start() -> FakeForge {
        let state = Arc::new(Mutex::new(State_ {
            next_number: 100,
            ..Default::default()
        }));
        let app = Router::new().fallback(handle).with_state(state.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        // Its own runtime on its own thread, so blocking calls from the test thread can reach it.
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                tx.send(l.local_addr().unwrap()).unwrap();
                axum::serve(l, app).await.unwrap();
            });
        });
        FakeForge {
            addr: rx.recv().unwrap(),
            state,
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.state.lock().unwrap().requests.clone()
    }

    pub fn set_prs(&self, owner: &str, repo: &str, prs: Vec<Value>) {
        self.state
            .lock()
            .unwrap()
            .prs
            .insert((owner.into(), repo.into()), prs);
    }

    /// The next request answers `{ "message": message }` with this status.
    pub fn fail_next(&self, status: u16, message: &str) {
        self.state.lock().unwrap().fail = Some((status, message.into()));
    }

    /// The next `method` request whose path contains `path_part` answers `{ "message": message }` with this status.
    pub fn fail_next_on(&self, method: &str, path_part: &str, status: u16, message: &str) {
        self.state.lock().unwrap().fail_on =
            Some((method.into(), path_part.into(), status, message.into()));
    }

    /// The next request waits this long before answering.
    pub fn delay_next(&self, d: Duration) {
        self.state.lock().unwrap().delay = Some(d);
    }
}

fn header(h: &HeaderMap, name: &str) -> String {
    h.get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

async fn handle(
    State(state): State<Arc<Mutex<State_>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let path = uri.path().to_string();
    let (fail, delay) = {
        let mut s = state.lock().unwrap();
        s.requests.push(Recorded {
            method: method.to_string(),
            path_and_query: uri
                .path_and_query()
                .map(|p| p.to_string())
                .unwrap_or_default(),
            auth: header(&headers, "authorization"),
            accept: header(&headers, "accept"),
            api_version: header(&headers, "x-github-api-version"),
            user_agent: header(&headers, "user-agent"),
            body: body.clone(),
        });
        let matched = s
            .fail_on
            .as_ref()
            .is_some_and(|(m, part, ..)| m == method.as_str() && path.contains(part.as_str()));
        let fail = if matched {
            s.fail_on
                .take()
                .map(|(_, _, status, message)| (status, message))
        } else {
            s.fail.take()
        };
        (fail, s.delay.take())
    };
    if let Some(d) = delay {
        tokio::time::sleep(d).await;
    }
    if let Some((status, message)) = fail {
        let code = StatusCode::from_u16(status).unwrap();
        return (code, axum::Json(json!({ "message": message }))).into_response();
    }
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let mut s = state.lock().unwrap();
    match (method.as_str(), parts.as_slice()) {
        ("GET", ["user"]) => axum::Json(json!({"login": "fake-user"})).into_response(),
        ("GET", ["repos", o, r, "pulls"]) => {
            let query = uri.query().unwrap_or_default();
            let open_only = query.split('&').any(|p| p == "state=open");
            let prs: Vec<Value> = s
                .prs
                .get(&(o.to_string(), r.to_string()))
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|p| !open_only || p["state"] == "open")
                .collect();
            axum::Json(prs).into_response()
        }
        ("POST", ["repos", o, r, "pulls"]) => {
            s.next_number += 1;
            let n = s.next_number;
            let pr = json!({
                "html_url": format!("https://github.com/{o}/{r}/pull/{n}"),
                "number": n,
                "state": "open",
                "merged_at": null,
            });
            s.prs
                .entry((o.to_string(), r.to_string()))
                .or_default()
                .push(pr.clone());
            (StatusCode::CREATED, axum::Json(pr)).into_response()
        }
        ("PATCH", ["repos", _, _, "pulls", _]) => axum::Json(json!({})).into_response(),
        ("POST", ["repos", _, _, "issues", _, "labels"]) => axum::Json(json!([])).into_response(),
        _ => (
            StatusCode::NOT_FOUND,
            axum::Json(json!({"message": "Not Found"})),
        )
            .into_response(),
    }
}
