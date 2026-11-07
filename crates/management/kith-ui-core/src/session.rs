//! Who may use the page: the one-time link token, passkey sessions, the `Host` and
//! `Origin` checks, and the security headers on every answer.
//!
//! Locally, `kith ui` opens `/#t=<token>`. The fragment never reaches the server or its
//! logs; the page posts it to `/api/session`, which burns it and answers with a cookie
//! holding a separate secret. A token is good once, for 2 minutes.
//!
//! A person who signs in with a passkey gets a session in the Console's database instead,
//! good for 12 hours after its last use. In public mode that is the only way in.

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
use crate::accounts::acting_as;
use crate::api::ApiError;
use crate::console::{Mode, Principal, person_session, require, signed_in};
use crate::wire::{ApiErrorCode, PersonRole, SessionIn};

/// The session cookie's name, before the port. Browsers share cookies across the ports
/// of a host, so each server names its own and two never sign each other out.
const COOKIE_PREFIX: &str = "kith_ui_";
/// The public Console's cookie: `__Host-` makes browsers insist on `Secure`, `Path=/` and
/// no `Domain`, so no other host under the domain can set or read it.
const PUBLIC_COOKIE: &str = "__Host-kith";
/// How long a link token stays good.
const TOKEN_LIFETIME: Duration = Duration::from_secs(120);
/// What every answer allows the page to load. `base-uri` and `form-action` do not fall
/// back to `default-src`, so they are named: an injected `<base>` or form must not send
/// the page's requests or fields elsewhere.
const CSP: &str = "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; \
script-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; \
form-action 'self'; frame-ancestors 'none'";
/// HTTPS only, for a year, on the public Console.
const HSTS: &str = "max-age=31536000";
/// API paths a request reaches without a session: they start one.
const OPEN: [&str; 4] = [
    "/api/setup/",
    "/api/login/",
    "/api/register/",
    "/api/recover/",
];
/// Changes a signed-in member may ask for; their handlers check roles themselves. Every
/// other change needs an owner or an admin.
const SELF_SERVICE: [&str; 8] = [
    "/api/passkeys/",
    "/api/logout",
    "/api/device-codes",
    "/api/authority/",
    "/api/people",
    "/api/setup/",
    "/api/login/",
    "/api/register/",
];

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

/// The request's caller, which the guard puts in every request's extensions.
#[derive(Debug, Clone)]
pub(crate) struct Caller(pub(crate) Option<Principal>);

/// The link token, until it is used or lapses, and the sessions it started.
#[derive(Debug)]
pub(crate) struct Sessions {
    link: Mutex<Link>,
    secrets: Mutex<Vec<Token>>,
    /// The cookie's name on this server.
    cookie: String,
    /// Whether the cookie is `Secure`: on the public Console.
    secure: bool,
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
            secure: false,
        }
    }

    /// Sessions for the public Console: passkeys only, no link.
    pub(crate) fn public() -> Self {
        Self {
            link: Mutex::new(Link::Used),
            secrets: Mutex::new(Vec::new()),
            cookie: PUBLIC_COOKIE.to_owned(),
            secure: true,
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

    /// This server's session cookie values in the request.
    fn values<'a>(&'a self, headers: &'a HeaderMap) -> impl Iterator<Item = &'a str> + 'a {
        headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .filter(|(name, _)| *name == self.cookie)
            .map(|(_, value)| value)
    }

    /// Whether the request carries a session the local link started.
    pub(crate) fn admits(&self, headers: &HeaderMap) -> bool {
        let secrets = self.secrets.lock().unwrap_or_else(PoisonError::into_inner);
        self.values(headers)
            .any(|candidate| secrets.iter().any(|secret| secret.matches(candidate)))
    }

    /// Who the request is from: the local link's session, a person's, or nobody.
    pub(crate) fn principal(&self, shared: &Shared, headers: &HeaderMap) -> Option<Principal> {
        if self.admits(headers) {
            return Some(Principal::Local);
        }
        self.values(headers)
            .filter(|value| value.len() == 64)
            .find_map(|secret| person_session(shared, secret))
    }

    /// The `Set-Cookie` value for a session secret: never readable by scripts. Locally
    /// never sent with a request another site starts; publicly also sent when a link
    /// opens the page, and only over HTTPS.
    pub(crate) fn set_cookie_value(&self, secret: &str) -> String {
        if self.secure {
            format!(
                "{}={secret}; HttpOnly; Secure; SameSite=Lax; Path=/",
                self.cookie
            )
        } else {
            format!(
                "{}={secret}; HttpOnly; SameSite=Strict; Path=/",
                self.cookie
            )
        }
    }

    /// The `Set-Cookie` value that removes the cookie.
    pub(crate) fn clear_cookie(&self) -> String {
        let secure = if self.secure { " Secure;" } else { "" };
        format!(
            "{}=; HttpOnly;{secure} SameSite=Strict; Path=/; Max-Age=0",
            self.cookie
        )
    }

    /// The session cookie's secrets in the request, to sign out.
    pub(crate) fn secrets_in<'a>(
        &'a self,
        headers: &'a HeaderMap,
    ) -> impl Iterator<Item = &'a str> + 'a {
        self.values(headers)
    }
}

/// `POST /api/session`.
pub(crate) async fn start(
    State(shared): State<Arc<Shared>>,
    Json(input): Json<SessionIn>,
) -> Result<Response, ApiError> {
    if shared.mode != Mode::Local {
        return Err(ApiError::not_found("sign in with your passkey"));
    }
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
    let cookie =
        HeaderValue::from_str(&sessions.set_cookie_value(&secret.hex())).map_err(|_| {
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

/// What the guard checks a request against.
#[derive(Debug, Clone, Copy)]
struct Rules<'a> {
    mode: &'a Mode,
    port: u16,
    dev_origins: &'a [String],
}

impl Rules<'_> {
    /// Whether `host`, a `Host` header, names this server.
    fn host(&self, host: &str) -> bool {
        match self.mode {
            Mode::Local => is_local(host, self.port, false),
            Mode::Public { domain } => {
                let host = host.strip_suffix(":443").unwrap_or(host);
                host.eq_ignore_ascii_case(domain)
            }
        }
    }

    /// Whether `origin`, an `Origin` header, is this server's page.
    fn origin(&self, origin: &str) -> bool {
        match self.mode {
            Mode::Local => {
                is_local(origin, self.port, true)
                    || self.dev_origins.iter().any(|dev| dev == origin)
            }
            Mode::Public { domain } => origin
                .strip_prefix("https://")
                .is_some_and(|rest| rest.eq_ignore_ascii_case(domain)),
        }
    }
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

/// Whether `path` is an API path that needs no session.
fn open(mode: &Mode, path: &str) -> bool {
    (path == "/api/session" && *mode == Mode::Local)
        || OPEN.iter().any(|prefix| path.starts_with(prefix))
}

/// Why a request may not go further, if it may not.
///
/// Every other server on `127.0.0.1` is the same site as this one, so a page it serves
/// gets our cookie attached to its requests, `SameSite=Strict` or not. Every API request
/// therefore has to show it came from this server's own page, reads included: they reach
/// the daemon too.
fn refuse(
    rules: &Rules<'_>,
    signed_in: bool,
    method: &Method,
    path: &str,
    headers: &HeaderMap,
) -> Option<ApiError> {
    let host_ok = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|host| rules.host(host));
    if !host_ok {
        return Some(ApiError::new(
            StatusCode::FORBIDDEN,
            ApiErrorCode::Forbidden,
            "wrong Host header",
        ));
    }
    let api = path.starts_with("/api/");
    let writes = !matches!(*method, Method::GET | Method::HEAD);
    let from_page = match headers.get(header::ORIGIN) {
        Some(origin) => origin.to_str().is_ok_and(|origin| rules.origin(origin)),
        None => !writes && same_origin_fetch(headers),
    };
    if (writes || api) && !from_page {
        return Some(ApiError::new(
            StatusCode::FORBIDDEN,
            ApiErrorCode::Forbidden,
            "requests must come from the kith page",
        ));
    }
    // The public signed-out page asks `/api/status` which mode it is in.
    let public_status = path == "/api/status" && !writes && *rules.mode != Mode::Local;
    if api && !open(rules.mode, path) && !public_status && !signed_in {
        let hint = match rules.mode {
            Mode::Local => "no session; run kith ui",
            Mode::Public { .. } => "sign in with your passkey",
        };
        return Some(ApiError::new(
            StatusCode::UNAUTHORIZED,
            ApiErrorCode::Unauthorized,
            hint,
        ));
    }
    None
}

/// `/api/networks/:net/devices/:node/settings`: a member may change their own devices; the
/// handler checks whose it is.
fn is_device_settings(path: &str) -> bool {
    matches!(
        path.split('/').collect::<Vec<_>>().as_slice(),
        ["", "api", "networks", _, "devices", _, "settings"]
    )
}

/// Whether a signed-in person needs to be an owner or admin for this request.
fn needs_admin(principal: Option<&Principal>, method: &Method, path: &str) -> bool {
    matches!(principal, Some(Principal::Person { .. }))
        && !matches!(*method, Method::GET | Method::HEAD)
        && path.starts_with("/api/")
        && !SELF_SERVICE.iter().any(|prefix| path.starts_with(prefix))
        && !is_device_settings(path)
}

/// Refuses requests that do not come from this server's page, then those without a
/// session, then members asking for an admin's change; runs everything else on behalf of
/// the signed-in person, and adds the security headers to every answer, refusals
/// included. Runs before every route.
pub(crate) async fn guard(
    State(shared): State<Arc<Shared>>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_owned();
    let api = path.starts_with("/api/");
    let principal = shared.sessions.principal(&shared, request.headers());
    let rules = Rules {
        mode: &shared.mode,
        port: shared.port,
        dev_origins: &shared.dev_origins,
    };
    let refused = refuse(
        &rules,
        principal.is_some(),
        request.method(),
        &path,
        request.headers(),
    );
    let refused = match refused {
        Some(refused) => Some(refused),
        // Every API request of a signed-in person, reads and the event stream included,
        // checks the ledger still holds them: a removed person's session reads nothing.
        None if api
            && !open(&shared.mode, &path)
            && matches!(principal, Some(Principal::Person { .. })) =>
        {
            match signed_in(&shared, principal.as_ref()).await {
                Ok((_, person)) if needs_admin(principal.as_ref(), request.method(), &path) => {
                    require(&person, PersonRole::Admin).err()
                }
                Ok(_) => None,
                Err(error) => Some(error),
            }
        }
        None => None,
    };
    let mut response = if let Some(refused) = refused {
        refused.into_response()
    } else {
        let actor = principal
            .as_ref()
            .and_then(Principal::account)
            .map(str::to_owned);
        request.extensions_mut().insert(Caller(principal));
        acting_as(actor, next.run(request)).await
    };
    secure(
        response.headers_mut(),
        api,
        matches!(shared.mode, Mode::Public { .. }),
    );
    response
}

/// The headers every answer carries. API answers are never cached unless they say so.
fn secure(headers: &mut HeaderMap, api: bool, public: bool) {
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
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
    if public {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static(HSTS),
        );
    }
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
        let set = sessions.set_cookie_value(&secret.hex());
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
    fn the_public_cookie_is_secure_and_host_only() {
        let set = Sessions::public().set_cookie_value("ab");
        assert_eq!(
            set,
            "__Host-kith=ab; HttpOnly; Secure; SameSite=Lax; Path=/"
        );
        assert!(Sessions::public().clear_cookie().contains("Secure; "));
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
    fn only_this_page_reaches_the_api() -> io::Result<()> {
        let dev = ["http://localhost:5173".to_owned()];
        let rules = Rules {
            mode: &Mode::Local,
            port: 41680,
            dev_origins: &dev,
        };
        let status = |method: Method, path: &str, extra: &[(&str, &str)]| -> io::Result<u16> {
            let mut headers = HeaderMap::new();
            headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:41680"));
            for (name, value) in extra {
                headers.insert(
                    header::HeaderName::from_bytes(name.as_bytes()).map_err(io::Error::other)?,
                    HeaderValue::from_str(value).map_err(io::Error::other)?,
                );
            }
            Ok(refuse(&rules, true, &method, path, &headers)
                .map_or(200, |refused| refused.into_response().status().as_u16()))
        };
        let page = [("sec-fetch-site", "same-origin")];
        assert_eq!(status(Method::GET, "/api/status", &page)?, 200);
        assert_eq!(
            status(Method::GET, "/api/status", &[])?,
            200,
            "not a browser"
        );
        assert_eq!(
            status(
                Method::GET,
                "/api/status",
                &[("host", "kith.evil.test:41680")]
            )?,
            403,
            "DNS rebinding"
        );
        for site in ["same-site", "cross-site", "none"] {
            assert_eq!(
                status(Method::GET, "/api/status", &[("sec-fetch-site", site)])?,
                403,
                "a GET from {site}"
            );
        }
        assert_eq!(
            status(Method::GET, "/", &[("sec-fetch-site", "cross-site")])?,
            200,
            "the page shell opens from a link"
        );
        let post = |origin: &'static str| status(Method::POST, "/api/exit", &[("origin", origin)]);
        assert_eq!(post("http://127.0.0.1:41680")?, 200);
        assert_eq!(post("http://localhost:5173")?, 200);
        assert_eq!(post("http://127.0.0.1:8080")?, 403, "another server here");
        assert_eq!(post("null")?, 403);
        assert_eq!(status(Method::POST, "/api/exit", &page)?, 403, "no Origin");
        let mut bare = HeaderMap::new();
        bare.insert(header::HOST, HeaderValue::from_static("localhost:41680"));
        bare.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://localhost:41680"),
        );
        let without_session = |path: &str| {
            refuse(&rules, false, &Method::POST, path, &bare)
                .map(|refused| refused.into_response().status().as_u16())
        };
        assert_eq!(without_session("/api/session"), None);
        assert_eq!(without_session("/api/login/begin"), None);
        assert_eq!(without_session("/api/exit"), Some(401));
        Ok(())
    }

    #[test]
    fn the_public_console_answers_only_to_its_domain() {
        let mode = Mode::Public {
            domain: "vpn.example.com".to_owned(),
        };
        let rules = Rules {
            mode: &mode,
            port: 443,
            dev_origins: &[],
        };
        assert!(rules.host("vpn.example.com"));
        assert!(rules.host("VPN.example.com:443"));
        assert!(!rules.host("vpn.example.com:8443"));
        assert!(!rules.host("127.0.0.1:443"));
        assert!(rules.origin("https://vpn.example.com"));
        assert!(!rules.origin("http://vpn.example.com"));
        assert!(!rules.origin("https://evil.example.com"));
        assert!(!open(&mode, "/api/session"), "no link sign-in in public");
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
