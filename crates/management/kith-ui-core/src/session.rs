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

/// The cookie that carries the session secret.
pub(crate) const COOKIE: &str = "kith_ui";
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
}

#[derive(Debug)]
enum Link {
    Fresh { token: Token, issued: Instant },
    Used,
}

impl Sessions {
    pub(crate) fn new(token: Token) -> Self {
        Self {
            link: Mutex::new(Link::Fresh {
                token,
                issued: Instant::now(),
            }),
            secrets: Mutex::new(Vec::new()),
        }
    }

    /// Burns the link token and starts a session, or says why not.
    fn start(&self, candidate: &str) -> Result<Token, ApiErrorCode> {
        let mut link = self.link.lock().unwrap_or_else(PoisonError::into_inner);
        let Link::Fresh { token, issued } = &*link else {
            return Err(ApiErrorCode::SessionUsed);
        };
        if !token.matches(candidate) {
            return Err(ApiErrorCode::Unauthorized);
        }
        if issued.elapsed() > TOKEN_LIFETIME {
            *link = Link::Used;
            return Err(ApiErrorCode::SessionExpired);
        }
        *link = Link::Used;
        let secret = Token::random().map_err(|_| ApiErrorCode::Internal)?;
        self.secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(secret.clone());
        Ok(secret)
    }

    /// Whether the request carries a live session cookie.
    pub(crate) fn admits(&self, headers: &HeaderMap) -> bool {
        let Some(candidate) = cookie(headers) else {
            return false;
        };
        self.secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|secret| secret.matches(candidate))
    }
}

fn cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| pair.trim().strip_prefix(COOKIE)?.strip_prefix('='))
}

/// `POST /api/session`.
pub(crate) async fn start(
    State(shared): State<Arc<Shared>>,
    Json(input): Json<SessionIn>,
) -> Result<Response, ApiError> {
    let secret = shared.sessions.start(&input.token).map_err(|code| {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            code,
            "the link token is not valid",
        )
    })?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    if let Ok(value) = HeaderValue::from_str(&format!(
        "{COOKIE}={}; HttpOnly; SameSite=Strict; Path=/",
        secret.hex()
    )) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
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

/// Refuses requests that do not come from this machine's page, then those without a
/// session. Runs before every route.
pub(crate) async fn guard(
    State(shared): State<Arc<Shared>>,
    request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();
    let host_ok = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|host| is_local(host, shared.port, false));
    if !host_ok {
        return ApiError::new(
            StatusCode::FORBIDDEN,
            ApiErrorCode::Forbidden,
            "wrong Host header",
        )
        .into_response();
    }
    let path = request.uri().path();
    let needs_origin = request.method() == Method::POST || path == "/api/events";
    let origin_ok = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| {
            is_local(origin, shared.port, true) || shared.dev_origins.iter().any(|o| o == origin)
        });
    let same_site_get = request.method() == Method::GET
        && headers.get(header::ORIGIN).is_none()
        && path == "/api/events";
    if needs_origin && !origin_ok && !same_site_get {
        return ApiError::new(
            StatusCode::FORBIDDEN,
            ApiErrorCode::Forbidden,
            "requests must come from the kith page",
        )
        .into_response();
    }
    let open = path == "/api/session" || !path.starts_with("/api/");
    if !open && !shared.sessions.admits(headers) {
        return ApiError::new(
            StatusCode::UNAUTHORIZED,
            ApiErrorCode::Unauthorized,
            "no session; run kith ui",
        )
        .into_response();
    }
    let mut response = next.run(request).await;
    secure(response.headers_mut());
    response
}

/// The headers every answer carries.
fn secure(headers: &mut HeaderMap) {
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
}

#[cfg(test)]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions fail the test; ? keeps the setup short"
)]
mod tests {
    use super::*;

    #[test]
    fn a_link_token_starts_one_session_only() -> io::Result<()> {
        let token = Token::random()?;
        let hex = token.hex();
        let sessions = Sessions::new(token);
        assert_eq!(
            sessions.start(&"0".repeat(64)).err(),
            Some(ApiErrorCode::Unauthorized)
        );
        let secret = sessions
            .start(&hex)
            .map_err(|_| io::Error::other("refused"))?;
        assert_ne!(secret.hex(), hex, "the cookie holds a separate secret");
        assert_eq!(sessions.start(&hex).err(), Some(ApiErrorCode::SessionUsed));
        let mut headers = HeaderMap::new();
        let cookie = HeaderValue::from_str(&format!("{COOKIE}={}", secret.hex()))
            .map_err(io::Error::other)?;
        headers.insert(header::COOKIE, cookie);
        assert!(sessions.admits(&headers));
        Ok(())
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
