//! The page's files and the SPA routes that serve `index.html`.
//!
//! `index.html` carries two markers that are filled per request:
//! `<!--kith:boot-->` becomes the boot snapshot as `<script type="application/json"
//! id="boot">`, and `<!--kith:signed-out-->` becomes the signed-out page when there is no
//! session, so that page works without JavaScript.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::{self, HeaderValue};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::Shared;
use crate::api::status::snapshot;

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
            "devices" | "activity" | "names" | "exits" | "settings",
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
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
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

/// Everything that is not `/api/`: the page's routes and files. Nothing else exists.
pub(crate) async fn serve(State(shared): State<Arc<Shared>>, request: Request) -> Response {
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return not_found();
    }
    let Some(assets) = shared.assets else {
        return not_found();
    };
    let path = request.uri().path();
    if is_page(path) {
        let signed_in = shared.sessions.admits(request.headers());
        return match assets.iter().find(|asset| asset.path == "index.html") {
            Some(index) => page(&shared, index, signed_in).await,
            None => not_found(),
        };
    }
    let accepts_brotli = request
        .headers()
        .get(header::ACCEPT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|part| part.trim().starts_with("br")));
    let name = path.trim_start_matches('/');
    match assets
        .iter()
        .find(|asset| asset.path == name && asset.path != "index.html")
    {
        Some(asset) => file(asset, accepts_brotli),
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
            "/n/home/settings",
        ] {
            assert!(is_page(page), "{page}");
        }
        for other in [
            "/n",
            "/n/home/inbox",
            "/admin",
            "/n/home/devices/nas/x",
            "/assets",
        ] {
            assert!(!is_page(other), "{other}");
        }
    }

    #[test]
    fn boot_json_cannot_close_its_script() {
        assert_eq!(
            script_safe(r#"{"a":"</script>"}"#),
            "{\"a\":\"\\u003c/script>\"}"
        );
    }
}
