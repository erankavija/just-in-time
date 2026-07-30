//! Embedded web UI assets compiled into the binary.
//!
//! When the `embed-web` feature is enabled and `web/dist/` exists at build
//! time, this module bundles those static files so `jit-server` can serve the
//! web UI without needing the files on disk at runtime.

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;
use std::path::Path;

#[derive(Embed)]
#[folder = "../../web/dist/"]
struct WebAssets;

/// Returns `true` if any web assets were embedded at compile time.
pub fn has_embedded_assets() -> bool {
    WebAssets::iter().next().is_some()
}

fn decode_percent_encoded_path(path: &str) -> Option<Vec<u8>> {
    fn hex_value(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Some(decoded)
}

fn decoded_route(path: &str) -> Option<String> {
    String::from_utf8(decode_percent_encoded_path(path)?).ok()
}

fn is_forbidden_route(path: &str) -> bool {
    let Some(decoded) = decoded_route(path) else {
        return true;
    };
    decoded == "api"
        || decoded.starts_with("api/")
        || decoded.contains('\\')
        || decoded
            .split('/')
            .any(|segment| matches!(segment, "." | ".."))
}

fn spa_fallback_path(path: &str) -> Option<&'static str> {
    let decoded = decoded_route(path)?;
    (!is_forbidden_route(path) && Path::new(&decoded).extension().is_none()).then_some("index.html")
}

fn asset_metadata(path: &str) -> (mime_guess::Mime, &'static str) {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache = if path == "index.html" {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    (mime, cache)
}

/// Serves exact embedded assets and the SPA entry point for extensionless client routes.
///
/// Unknown API routes, traversal-shaped paths, and missing file-like assets remain `404`.
/// The SPA entry point is never cached; exact non-index assets retain immutable caching.
pub async fn embedded_fallback(uri: axum::http::Uri) -> Response {
    let requested = uri.path().trim_start_matches('/');
    if is_forbidden_route(requested) {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }

    let direct = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };
    let path = if WebAssets::get(direct).is_some() {
        direct
    } else if let Some(fallback) = spa_fallback_path(requested) {
        fallback
    } else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };

    match WebAssets::get(path) {
        Some(file) => {
            let (mime, cache) = asset_metadata(path);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spa_fallback_path_returns_index_for_root_and_client_routes() {
        assert_eq!(spa_fallback_path(""), Some("index.html"));
        assert_eq!(spa_fallback_path("issues/abc123"), Some("index.html"));
        assert_eq!(
            spa_fallback_path("projects/current/board"),
            Some("index.html")
        );
    }

    #[test]
    fn test_spa_fallback_path_rejects_api_and_missing_asset_routes() {
        assert_eq!(spa_fallback_path("api"), None);
        assert_eq!(spa_fallback_path("api/missing"), None);
        assert_eq!(spa_fallback_path("assets/missing.js"), None);
        assert_eq!(spa_fallback_path("missing.css"), None);
    }

    #[test]
    fn test_spa_fallback_path_rejects_traversal_shaped_routes() {
        for path in [
            "../secret",
            "docs/../../secret",
            "%2e%2e/secret",
            "%2E%2E%2fsecret",
            "docs/%2e%2e%5csecret",
        ] {
            assert_eq!(spa_fallback_path(path), None, "accepted {path:?}");
        }
    }

    #[test]
    fn test_asset_metadata_keeps_index_uncached_and_assets_immutable() {
        let (index_mime, index_cache) = asset_metadata("index.html");
        assert_eq!(index_mime.essence_str(), "text/html");
        assert_eq!(index_cache, "no-cache");

        let (asset_mime, asset_cache) = asset_metadata("assets/app.css");
        assert_eq!(asset_mime.essence_str(), "text/css");
        assert_eq!(asset_cache, "public, max-age=31536000, immutable");
    }

    #[tokio::test]
    async fn test_embedded_fallback_returns_plain_uncached_404_for_rejected_routes() {
        for path in ["/api/missing", "/assets/missing.js", "/%2e%2e/secret"] {
            let response = embedded_fallback(path.parse().expect("valid test URI")).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "route {path}");
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE),
                Some(&header::HeaderValue::from_static(
                    "text/plain; charset=utf-8"
                )),
                "route {path}"
            );
            assert!(
                response.headers().get(header::CACHE_CONTROL).is_none(),
                "route {path}"
            );
        }
    }
}
