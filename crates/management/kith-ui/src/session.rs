//! The session token, the cookie and origin checks, and dispatch to the page or the API.

use core::fmt;
use core::fmt::Write as _;
use std::io;

use bytes::Bytes;
use http_body_util::{BodyExt as _, Full};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};

use crate::{Body, State, api, assets};

const COOKIE: &str = "kith_ui";

const SIGNED_OUT: &str = "<!doctype html><meta charset=\"utf-8\"><title>kith</title>\
<p>This page needs a session. Run <code>kith ui</code> to open it.</p>";

/// The random secret a browser must present, as the `kith_ui` cookie.
pub struct Token([u8; 32]);

impl Token {
    /// A fresh token from the operating system's random source.
    pub fn random() -> io::Result<Self> {
        let mut bytes = [0; 32];
        kith_crypto::fill_random(&mut bytes)
            .map_err(|_| io::Error::other("no random numbers from the operating system"))?;
        Ok(Self(bytes))
    }

    /// Lowercase hex, as it appears in the URL and the cookie.
    pub fn hex(&self) -> String {
        self.0
            .iter()
            .fold(String::with_capacity(64), |mut text, byte| {
                let _infallible = write!(text, "{byte:02x}");
                text
            })
    }

    /// Compares in constant time, so the token cannot be guessed byte by byte.
    fn matches(&self, candidate: &str) -> bool {
        let expected = self.hex();
        expected.len() == candidate.len()
            && expected
                .bytes()
                .zip(candidate.bytes())
                .fold(0, |diff, (a, b)| diff | (a ^ b))
                == 0
    }
}

/// Never prints the secret.
impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

pub(crate) fn full(
    status: StatusCode,
    content_type: &str,
    body: impl Into<Bytes>,
) -> Response<Body> {
    let mut response = Response::new(Full::new(body.into()).boxed());
    *response.status_mut() = status;
    if let Ok(value) = HeaderValue::from_str(content_type) {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| pair.trim().strip_prefix(COOKIE)?.strip_prefix('='))
}

pub(crate) fn query_param<'a>(query: Option<&'a str>, name: &str) -> Option<&'a str> {
    query?
        .split('&')
        .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
}

/// Whether a `POST` comes from the page itself: `Origin`, or else `Referer`, names this server.
fn same_origin(headers: &HeaderMap, origin: &str) -> bool {
    if let Some(value) = headers.get(header::ORIGIN) {
        return value.as_bytes() == origin.as_bytes();
    }
    headers
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
        .and_then(|referer| referer.strip_prefix(origin))
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

fn start_session(state: &State, token: &str) -> Response<Body> {
    if !state.token.matches(token) {
        return full(
            StatusCode::UNAUTHORIZED,
            "text/html; charset=utf-8",
            SIGNED_OUT,
        );
    }
    let mut response = full(StatusCode::SEE_OTHER, "text/plain", "");
    let headers = response.headers_mut();
    headers.insert(header::LOCATION, HeaderValue::from_static("/"));
    if let Ok(value) = HeaderValue::from_str(&format!(
        "{COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/"
    )) {
        headers.insert(header::SET_COOKIE, value);
    }
    response
}

pub(crate) async fn handle(state: &State, request: Request<Incoming>) -> Response<Body> {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().map(str::to_owned);
    if request.method() == Method::GET
        && path == "/"
        && let Some(token) = query_param(query.as_deref(), "token")
    {
        return start_session(state, token);
    }
    if !cookie(request.headers()).is_some_and(|token| state.token.matches(token)) {
        return full(
            StatusCode::UNAUTHORIZED,
            "text/html; charset=utf-8",
            SIGNED_OUT,
        );
    }
    if request.method() == Method::POST && !same_origin(request.headers(), &state.origin) {
        return api::error(
            StatusCode::FORBIDDEN,
            "requests must come from the kith page",
        );
    }
    match path.strip_prefix("/api/") {
        Some(route) => api::handle(state, route, query.as_deref(), request).await,
        None if request.method() == Method::GET => assets::serve(&path),
        None => api::error(StatusCode::NOT_FOUND, "no such page"),
    }
}
