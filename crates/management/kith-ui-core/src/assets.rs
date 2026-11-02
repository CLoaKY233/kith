//! The page's files and the SPA routes that serve `index.html`.
//!
//! `index.html` carries two markers that are filled per request:
//! `<!--kith:boot-->` becomes the boot snapshot as `<script type="application/json"
//! id="boot">`, and `<!--kith:signed-out-->` becomes the signed-out page when there is no
//! session, so that page works without JavaScript.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::Shared;
use crate::api::ApiError;
use crate::ipc::snapshot;
use crate::wire::ApiErrorCode;

/// One embedded file.
#[derive(Debug, Clone, Copy)]
pub struct Asset {
    /// Path under the site root, without a leading slash, such as `assets/index-3f2a.js`.
    pub path: &'static str,
    /// The file.
    pub body: &'static [u8],
    /// The file compressed with brotli, when that made it smaller.
    pub brotli: Option<&'static [u8]>,
}

/// The signed-out page, shown when the page has no session. Same words as `copy.ts`.
const SIGNED_OUT: &str = "<main class=\"signed-out\"><h1>This page needs a fresh link.</h1>\
<p>For safety, kith only opens for the person at this computer. Run this in a terminal:</p>\
<pre><code>kith ui</code></pre><p>The link it opens works once.</p></main>";

/// Whether `path` is one of the page's routes (design spec §4.1).
fn is_page(path: &str) -> bool {
    let segments: Vec<&str> = path.trim_end_matches('/').split('/').skip(1).collect();
    match segments.as_slice() {
        [] | ["" | "welcome" | "this"] => true,
        ["n", net]
        | [
            "n",
            net,
            "devices" | "activity" | "names" | "exits" | "access" | "settings",
        ] => !net.is_empty(),
        ["n", net, "devices", device] => !net.is_empty() && !device.is_empty(),
        _ => false,
    }
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, extension)| extension) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("webp") => "image/webp",
        Some("webmanifest") => "application/manifest+json",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "Not found").into_response()
}

fn file(asset: &Asset, accepts_brotli: bool) -> Response {
    let (body, brotli) = match asset.brotli {
        Some(compressed) if accepts_brotli => (compressed, true),
        Some(_) | None => (asset.body, false),
    };
    let mut response = (StatusCode::OK, body).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(asset.path)),
    );
    if brotli {
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("br"));
    }
    headers.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    let cache = if asset.path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

/// JSON safe inside `<script>`: no `<` can close the element.
fn script_safe(json: &str) -> String {
    json.replace('<', "\\u003c")
}

async fn page(shared: &Shared, index: &Asset, signed_in: bool) -> Response {
    let html = String::from_utf8_lossy(index.body);
    let boot = if signed_in {
        match snapshot(shared).await {
            Ok(boot) => serde_json::to_string(&boot).unwrap_or_else(|_| "null".to_owned()),
            Err(_) => "null".to_owned(),
        }
    } else {
        "null".to_owned()
    };
    let html = html
        .replace(
            "<!--kith:boot-->",
            &format!(
                "<script type=\"application/json\" id=\"boot\">{}</script>",
                script_safe(&boot)
            ),
        )
        .replace(
            "<!--kith:signed-out-->",
            if signed_in { "" } else { SIGNED_OUT },
        );
    let mut response = (StatusCode::OK, html).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Whether `Accept-Encoding` allows brotli: `br` listed without `q=0`. Codings and
/// parameter names are case-insensitive (RFC 9110 §8.4.1, §5.6.6).
fn accepts_brotli(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|part| {
            let mut params = part.split(';').map(str::trim);
            params
                .next()
                .is_some_and(|coding| coding.eq_ignore_ascii_case("br"))
                && params.all(|param| {
                    param
                        .split_once('=')
                        .filter(|(name, _)| name.trim().eq_ignore_ascii_case("q"))
                        .and_then(|(_, q)| q.trim().parse::<f32>().ok())
                        .is_none_or(|q| q > 0.0)
                })
        })
}

/// Everything not routed: the page's routes and files. Nothing else exists.
pub(crate) async fn serve(State(shared): State<Arc<Shared>>, request: Request) -> Response {
    let path = request.uri().path();
    if path == "/api" || path.starts_with("/api/") {
        return ApiError::new(
            StatusCode::NOT_FOUND,
            ApiErrorCode::NotFound,
            "no such endpoint",
        )
        .into_response();
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return not_found();
    }
    let Some(assets) = shared.assets else {
        return not_found();
    };
    if is_page(path) {
        let signed_in = shared.sessions.admits(request.headers());
        return match assets.iter().find(|asset| asset.path == "index.html") {
            Some(index) => page(&shared, index, signed_in).await,
            None => not_found(),
        };
    }
    let name = path.trim_start_matches('/');
    match assets
        .iter()
        .find(|asset| asset.path == name && asset.path != "index.html")
    {
        Some(asset) => file(asset, accepts_brotli(request.headers())),
        None => not_found(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_spec_routes_are_pages() {
        for page in [
            "/",
            "/welcome",
            "/this",
            "/n/home",
            "/n/home/devices",
            "/n/home/devices/nas",
            "/n/home/activity",
            "/n/home/names",
            "/n/home/exits",
            "/n/home/access",
            "/n/home/settings",
        ] {
            assert!(is_page(page), "{page}");
        }
        for other in [
            "/n",
            "/n/home/inbox",
            "/admin",
            "/n/home/devices/nas/x",
            "/n//devices",
            "/assets",
            "/index.html",
        ] {
            assert!(!is_page(other), "{other}");
        }
    }

    #[test]
    fn brotli_only_when_the_browser_takes_it() {
        let accepts = |value: &'static str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::ACCEPT_ENCODING, HeaderValue::from_static(value));
            accepts_brotli(&headers)
        };
        assert!(accepts("gzip, deflate, br, zstd"));
        assert!(accepts("br;q=0.5"));
        assert!(!accepts("br;q=0"));
        assert!(accepts("BR"));
        assert!(!accepts("br; Q=0.0"));
        assert!(!accepts("gzip, brotli"));
        assert!(!accepts_brotli(&HeaderMap::new()));
    }

    #[test]
    fn boot_json_cannot_close_its_script() {
        assert_eq!(
            script_safe(r#"{"a":"</script>"}"#),
            "{\"a\":\"\\u003c/script>\"}"
        );
    }

    #[test]
    fn only_hashed_files_are_cached_for_good() {
        let hashed = Asset {
            path: "assets/index-3f2a.js",
            body: b"plain",
            brotli: Some(b"br"),
        };
        let packed = file(&hashed, true);
        assert_eq!(packed.headers()[header::CONTENT_ENCODING], "br");
        assert_eq!(packed.headers()[header::VARY], "accept-encoding");
        assert_eq!(
            packed.headers()[header::CACHE_CONTROL],
            "public, max-age=31536000, immutable"
        );
        assert!(
            file(&hashed, false)
                .headers()
                .get(header::CONTENT_ENCODING)
                .is_none()
        );
        let icon = Asset {
            path: "favicon.svg",
            body: b"<svg/>",
            brotli: None,
        };
        let icon = file(&icon, true);
        assert_eq!(icon.headers()[header::CACHE_CONTROL], "no-cache");
        assert_eq!(icon.headers()[header::CONTENT_TYPE], "image/svg+xml");
        assert!(icon.headers().get(header::CONTENT_ENCODING).is_none());
    }
}
