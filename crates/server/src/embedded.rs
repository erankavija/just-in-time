//! Embedded web UI assets compiled into the binary.
//!
//! When the `embed-web` feature is enabled and `web/dist/` exists at build
//! time, this module bundles those static files so `jit-server` can serve the
//! web UI without needing the files on disk at runtime.

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../../web/dist/"]
struct WebAssets;

/// Returns `true` if any web assets were embedded at compile time.
pub fn has_embedded_assets() -> bool {
    WebAssets::iter().next().is_some()
}

/// Axum fallback handler that serves files from the embedded assets.
pub async fn embedded_fallback(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Serve the requested file, falling back to index.html for bare paths.
    let path = if path.is_empty() { "index.html" } else { path };

    match WebAssets::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let cache = if path == "index.html" {
                "no-cache"
            } else {
                "public, max-age=31536000, immutable"
            };
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, mime.as_ref().to_owned()),
                    (header::CACHE_CONTROL, cache.to_owned()),
                ],
                file.data,
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}
