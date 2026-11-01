//! Who may use the page: the one-time link token, the session cookie, the `Host` and
//! `Origin` checks, and the security headers on every answer.
//!
//! `kith ui` opens `/#t=<token>`. The fragment never reaches the server or its logs; the
//! page posts it to `/api/session`, which burns it and answers with a cookie holding a
//! separate secret. A token is good once, for 2 minutes.

use core::fmt;
use core::fmt::Write as _;
use core::time::Duration;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Json;
use axum::extract::{Request, State};
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use subtle::ConstantTimeEq as _;
use tokio::time::Instant;

use crate::Shared;
use crate::api::ApiError;
use crate::wire::{ApiErrorCode, SessionIn};

/// The session cookie's name, before the port. Browsers share cookies across the ports
/// of a host, so each server names its own and two never sign each other out.
const COOKIE_PREFIX: &str = "kith_ui_";
/// How long a link token stays good.
const TOKEN_LIFETIME: Duration = Duration::from_secs(120);
/// What every answer allows the page to load.
const CSP: &str = "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; \
script-src 'self'; connect-src 'self'; frame-ancestors 'none'";

/// A 32-byte random secret: the link token or a session secret.
#[derive(Clone)]
pub struct Token([u8; 32]);

impl Token {
    /// A fresh token from the operating system's random source.
    pub fn random() -> io::Result<Self> {
        let mut bytes = [0; 32];
        kith_crypto::fill_random(&mut bytes)
            .map_err(|_| io::Error::other("no random numbers from the operating system"))?;
        Ok(Self(bytes))
    }

    /// Lowercase hex, as it appears in the link and the cookie.
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
        bool::from(self.hex().as_bytes().ct_eq(candidate.as_bytes()))
    }
}

/// Never prints the secret.
impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// The link token, until it is used or lapses, and the sessions it started.
#[derive(Debug)]
pub(crate) struct Sessions {
    link: Mutex<Link>,
    secrets: Mutex<Vec<Token>>,
    /// The cookie's name on this server's port.
    cookie: String,
}

#[derive(Debug)]
enum Link {
    Fresh { token: Token, issued: Instant },
    Used,
}

impl Sessions {
    /// Sessions for the server on `port`, started by `token` within 2 minutes from now.
    pub(crate) fn new(token: Token, port: u16) -> Self {
        Self {
            link: Mutex::new(Link::Fresh {
                token,
                issued: Instant::now(),
            }),
            secrets: Mutex::new(Vec::new()),
            cookie: format!("{COOKIE_PREFIX}{port}"),
        }
    }

    /// Burns the link token and starts a session, or says why not. A wrong token burns
    /// nothing, so a stray request cannot lock the person out.
    fn start(&self, candidate: &str, now: Instant) -> Result<Token, ApiErrorCode> {
        let mut link = self.link.lock().unwrap_or_else(PoisonError::into_inner);
        let Link::Fresh { token, issued } = &*link else {
            return Err(ApiErrorCode::SessionUsed);
        };
        if !token.matches(candidate) {
            return Err(ApiErrorCode::Unauthorized);
        }
        if now.saturating_duration_since(*issued) > TOKEN_LIFETIME {
            *link = Link::Used;
            return Err(ApiErrorCode::SessionExpired);
        }
        let secret = Token::random().map_err(|_| ApiErrorCode::Internal)?;
        *link = Link::Used;
        self.secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(secret.clone());
        Ok(secret)
    }

    /// Whether the request carries a live session cookie.
    pub(crate) fn admits(&self, headers: &HeaderMap) -> bool {
        let secrets = self.secrets.lock().unwrap_or_else(PoisonError::into_inner);
        headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .filter(|(name, _)| *name == self.cookie)
            .any(|(_, candidate)| secrets.iter().any(|secret| secret.matches(candidate)))
    }

    /// The `Set-Cookie` value for a session secret: never readable by scripts, never sent
    /// with a request another site starts.
    fn set_cookie(&self, secret: &Token) -> String {
        format!(
            "{}={}; HttpOnly; SameSite=Strict; Path=/",
            self.cookie,
            secret.hex()
        )
    }
}

/// `POST /api/session`.
pub(crate) async fn start(
    State(shared): State<Arc<Shared>>,
    Json(input): Json<SessionIn>,
) -> Result<Response, ApiError> {
    let sessions = &shared.sessions;
    let secret = sessions
        .start(&input.token, Instant::now())
        .map_err(|code| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                code,
                "the link token is not valid",
            )
        })?;
    let cookie = HeaderValue::from_str(&sessions.set_cookie(&secret)).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiErrorCode::Internal,
            "the session cookie could not be written",
        )
    })?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(response)
}

/// Whether `value` names this server, as `http://127.0.0.1:<port>` style text without the
/// scheme when `scheme` is false.
fn is_local(value: &str, port: u16, scheme: bool) -> bool {
    let rest = if scheme {
        match value.strip_prefix("http://") {
            Some(rest) => rest,
            None => return false,
        }
    } else {
        value
    };
    rest == format!("127.0.0.1:{port}") || rest == format!("localhost:{port}")
}

/// Whether a request without `Origin` was made by a page of this server.
///
/// Browsers leave `Origin` off same-origin GETs such as the page's `EventSource`, but they
/// send `Sec-Fetch-Site`, which says where the request came from. Without either header
/// the caller is not a browser, so no other site can be behind it.
fn same_origin_fetch(headers: &HeaderMap) -> bool {
    headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        .is_none_or(|site| site == "same-origin")
}

/// Why `request` may not go further, if it may not.
fn refusal(shared: &Shared, request: &Request) -> Option<ApiError> {
    let headers = request.headers();
    let host_ok = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|host| is_local(host, shared.port, false));
    if !host_ok {
        return Some(ApiError::new(
            StatusCode::FORBIDDEN,
            ApiErrorCode::Forbidden,
            "wrong Host header",
        ));
    }
    let path = request.uri().path();
    let writes = !matches!(*request.method(), Method::GET | Method::HEAD);
    let from_page = match headers.get(header::ORIGIN) {
        Some(origin) => origin.to_str().is_ok_and(|origin| {
            is_local(origin, shared.port, true)
                || shared.dev_origins.iter().any(|dev| dev == origin)
        }),
        None => !writes && same_origin_fetch(headers),
    };
    if (writes || path == "/api/events") && !from_page {
        return Some(ApiError::new(
            StatusCode::FORBIDDEN,
            ApiErrorCode::Forbidden,
            "requests must come from the kith page",
        ));
    }
    let open = path == "/api/session" || !path.starts_with("/api/");
    if !open && !shared.sessions.admits(headers) {
        return Some(ApiError::new(
            StatusCode::UNAUTHORIZED,
            ApiErrorCode::Unauthorized,
            "no session; run kith ui",
        ));
    }
    None
}

/// Refuses requests that do not come from this machine's page, then those without a
/// session, and adds the security headers to every answer, refusals included. Runs before
/// every route.
pub(crate) async fn guard(
    State(shared): State<Arc<Shared>>,
    request: Request,
    next: Next,
) -> Response {
    let api = request.uri().path().starts_with("/api/");
    let mut response = match refusal(&shared, &request) {
        Some(refused) => refused.into_response(),
        None => next.run(request).await,
    };
    secure(response.headers_mut(), api);
    response
}

/// The headers every answer carries. API answers are never cached unless they say so.
fn secure(headers: &mut HeaderMap, api: bool) {
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    if api && !headers.contains_key(header::CACHE_CONTROL) {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
}

#[cfg(test)]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions fail the test; ? keeps the setup short"
)]
mod tests {
    use super::*;

    fn cookies(value: &str) -> io::Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(value).map_err(io::Error::other)?,
        );
        Ok(headers)
    }

    #[test]
    fn a_link_token_starts_one_session_only() -> io::Result<()> {
        let token = Token::random()?;
        let hex = token.hex();
        let sessions = Sessions::new(token, 41680);
        let now = Instant::now();
        assert_eq!(
            sessions.start(&"0".repeat(64), now).err(),
            Some(ApiErrorCode::Unauthorized)
        );
        let secret = sessions
            .start(&hex, now)
            .map_err(|_| io::Error::other("refused"))?;
        assert_ne!(secret.hex(), hex, "the cookie holds a separate secret");
        assert_eq!(
            sessions.start(&hex, now).err(),
            Some(ApiErrorCode::SessionUsed)
        );
        let set = sessions.set_cookie(&secret);
        assert!(set.starts_with("kith_ui_41680=") && set.contains("; HttpOnly; SameSite=Strict"));
        let ours = format!("kith_ui_41680={}", secret.hex());
        assert!(sessions.admits(&cookies(&ours)?));
        assert!(sessions.admits(&cookies(&format!("kith_ui_9=stale; {ours}"))?));
        assert!(!sessions.admits(&cookies(&format!("kith_ui_9={}", secret.hex()))?));
        assert!(!sessions.admits(&cookies("kith_ui_41680=00")?));
        assert!(!sessions.admits(&HeaderMap::new()));
        Ok(())
    }

    #[test]
    fn a_link_token_lapses_after_two_minutes() -> io::Result<()> {
        let token = Token::random()?;
        let hex = token.hex();
        let sessions = Sessions::new(token, 41680);
        let later = Instant::now() + TOKEN_LIFETIME + Duration::from_secs(1);
        assert_eq!(
            sessions.start(&hex, later).err(),
            Some(ApiErrorCode::SessionExpired)
        );
        assert_eq!(
            sessions.start(&hex, Instant::now()).err(),
            Some(ApiErrorCode::SessionUsed),
            "a lapsed token is burned"
        );
        Ok(())
    }

    #[test]
    fn origin_less_requests_count_only_from_this_site_or_outside_a_browser() {
        let site = |value: &'static str| {
            let mut headers = HeaderMap::new();
            headers.insert("sec-fetch-site", HeaderValue::from_static(value));
            same_origin_fetch(&headers)
        };
        assert!(site("same-origin"));
        assert!(!site("same-site"));
        assert!(!site("cross-site"));
        assert!(!site("none"));
        assert!(same_origin_fetch(&HeaderMap::new()));
    }

    #[test]
    fn only_loopback_names_on_this_port_are_local() {
        assert!(is_local("http://127.0.0.1:41680", 41680, true));
        assert!(is_local("localhost:41680", 41680, false));
        assert!(!is_local("http://127.0.0.1:41681", 41680, true));
        assert!(!is_local("http://evil.test", 41680, true));
        assert!(!is_local("127.0.0.1:41680", 41680, true));
    }
}
