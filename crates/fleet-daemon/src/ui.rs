//! Serves the web UI that `build.rs` embedded in the binary. Nothing is fetched at runtime.
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rust_embed::RustEmbed;
use serde_json::json;

#[derive(RustEmbed)]
#[folder = "$OUT_DIR/web"]
struct Assets;

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response()
}

fn file(path: &str) -> Option<Response> {
    let f = Assets::get(path)?;
    // Vite hashes the names under /assets/, so those never change; everything else must revalidate.
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    Some(
        (
            [
                (header::CONTENT_TYPE, f.metadata.mimetype().to_string()),
                (header::CACHE_CONTROL, cache.to_string()),
            ],
            f.data.into_owned(),
        )
            .into_response(),
    )
}

pub async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") || path.starts_with("internal/") {
        return not_found();
    }
    let wanted = if path.is_empty() { "index.html" } else { path };
    if let Some(r) = file(wanted) {
        return r;
    }
    // A missing file with an extension is a broken link, not a client-side route.
    if wanted.starts_with("assets/") || wanted.rsplit('/').next().is_some_and(|n| n.contains('.')) {
        return not_found();
    }
    file("index.html").unwrap_or_else(not_found)
}
