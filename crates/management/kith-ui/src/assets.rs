//! The page's files, compiled in from `web/` by the build script.

use hyper::{Response, StatusCode};

use crate::{Body, api, session};

/// Every file under `web/`, by its path relative to `web/`.
const FILES: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/web.rs"));

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
        _ => "application/octet-stream",
    }
}

/// `GET /` is `index.html`; `GET /assets/<file>` is `web/assets/<file>`. Nothing else.
pub(crate) fn serve(path: &str) -> Response<Body> {
    let name = if path == "/" {
        Some("index.html")
    } else {
        path.strip_prefix('/')
            .filter(|name| name.starts_with("assets/"))
    };
    match name.and_then(|name| FILES.iter().find(|(file, _)| *file == name)) {
        Some((file, bytes)) => session::full(StatusCode::OK, content_type(file), *bytes),
        None => api::error(StatusCode::NOT_FOUND, "no such page"),
    }
}
