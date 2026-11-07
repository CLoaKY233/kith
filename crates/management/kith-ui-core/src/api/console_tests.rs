//! The Console over HTTP, in public mode, on the recorded network with the in-memory
//! ledger: setup, sign-in, sessions, people and authority prompts signed by software
//! passkeys, device codes, and the `Host` rule.

#![expect(
    clippy::panic_in_result_fn,
    reason = "assertions fail the test; ? keeps the setup short"
)]

use core::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tower::ServiceExt as _;

use crate::osauth::Gate;
use crate::public::ClientAddr;
use crate::webauthn::testing::Authenticator;
use crate::{Config, Mode, Shared, Source, SqliteStore, Token, build, setup_link};

type TestResult<T = ()> = Result<T, Box<dyn core::error::Error>>;

const DOMAIN: &str = "vpn.example.com";
const ORIGIN: &str = "https://vpn.example.com";
const MINUTE: u64 = 60 * 1000;
const HOUR: u64 = 60 * MINUTE;

/// One answer: status, JSON body (`null` when empty) and the cookie it set.
struct Answer {
    status: StatusCode,
    body: Value,
    cookie: Option<String>,
    headers: axum::http::HeaderMap,
}

struct Console {
    app: Router,
    shared: Arc<Shared>,
}

/// A request as the Console's page sends it.
struct Call<'a> {
    method: Method,
    path: &'a str,
    body: Option<Value>,
    cookie: Option<&'a str>,
    host: &'a str,
    origin: Option<&'a str>,
    from: Option<[u8; 4]>,
}

impl<'a> Call<'a> {
    fn get(path: &'a str) -> Self {
        Self {
            method: Method::GET,
            path,
            body: None,
            cookie: None,
            host: DOMAIN,
            origin: None,
            from: None,
        }
    }

    fn post(path: &'a str, body: Value) -> Self {
        Self {
            method: Method::POST,
            body: Some(body),
            origin: Some(ORIGIN),
            ..Self::get(path)
        }
    }

    const fn with(mut self, cookie: &'a str) -> Self {
        self.cookie = Some(cookie);
        self
    }
}

impl Console {
    fn public() -> TestResult<Self> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../kith-ui/web/fixtures/office.json");
        let (app, shared) = build(
            Config {
                source: Source::Fixture(Box::new(crate::load_fixture(&path)?)),
                port: 443,
                token: Token::random()?,
                assets: None,
                dev_origins: Vec::new(),
                mode: Mode::Public {
                    domain: DOMAIN.to_owned(),
                },
                store: Arc::new(SqliteStore::memory()?),
            },
            Gate::System,
            None,
        )?;
        Ok(Self { app, shared })
    }

    async fn send(&self, call: Call<'_>) -> TestResult<Answer> {
        let mut request = Request::builder()
            .method(call.method)
            .uri(call.path)
            .header(header::HOST, call.host);
        if let Some(origin) = call.origin {
            request = request.header(header::ORIGIN, origin);
        }
        if let Some(cookie) = call.cookie {
            request = request.header(header::COOKIE, cookie);
        }
        let mut request = match call.body {
            Some(body) => request
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))?,
            None => request.body(Body::empty())?,
        };
        if let Some(ip) = call.from {
            request
                .extensions_mut()
                .insert(ConnectInfo(ClientAddr(SocketAddr::from((ip, 4000)))));
        }
        let response = self.app.clone().oneshot(request).await?;
        let status = response.status();
        let headers = response.headers().clone();
        let cookie = headers
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::to_owned);
        let bytes = response.into_body().collect().await?.to_bytes();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)?
        };
        Ok(Answer {
            status,
            body,
            cookie,
            headers,
        })
    }

    /// `GET /api/me`'s status with `cookie`.
    async fn me(&self, cookie: &str) -> TestResult<StatusCode> {
        Ok(self.send(Call::get("/api/me").with(cookie)).await?.status)
    }

    /// A fresh setup link's token.
    fn setup_token(&self) -> TestResult<String> {
        let link = setup_link(&*self.shared.store, DOMAIN, self.shared.clock.now_ms())?
            .ok_or("setup was done")?;
        Ok(link
            .rsplit('/')
            .next()
            .ok_or("no token in the link")?
            .to_owned())
    }

    /// Runs setup as the page does: the owner's passkey, their session, the console
    /// authorized, the recovery words. Returns the owner's passkey and cookie.
    async fn set_up(&self) -> TestResult<(Authenticator, String)> {
        let token = self.setup_token()?;
        let owner = Authenticator::new(1, DOMAIN, ORIGIN).ok_or("no key")?;
        let begun = self
            .send(Call::post(
                "/api/setup/begin",
                json!({"token": token, "name": "Ada"}),
            ))
            .await?;
        assert_eq!(begun.status, StatusCode::OK, "{}", begun.body);
        let credential = owner.register(&begun.body["public_key"]);
        let finished = self
            .send(Call::post(
                "/api/setup/finish",
                json!({"token": token, "credential": credential}),
            ))
            .await?;
        assert_eq!(finished.status, StatusCode::OK, "{}", finished.body);
        assert_eq!(finished.body["person"]["role"], "owner");
        assert_eq!(finished.body["authority"]["change"], "authorize_console");
        let cookie = finished.cookie.ok_or("no session cookie")?;
        let signed = owner.assert(&finished.body["authority"]["public_key"]);
        let authorized = self
            .send(Call::post("/api/authority/finish", json!({"credential": signed})).with(&cookie))
            .await?;
        assert_eq!(authorized.status, StatusCode::OK, "{}", authorized.body);
        Ok((owner, cookie))
    }

    /// Signs in with `passkey`; returns the session cookie, or the refusal's status.
    async fn sign_in(&self, passkey: &Authenticator) -> TestResult<Result<String, StatusCode>> {
        let begun = self.send(Call::post("/api/login/begin", json!({}))).await?;
        let answer = passkey.assert(&begun.body["public_key"]);
        let finished = self
            .send(Call::post(
                "/api/login/finish",
                json!({"credential": answer}),
            ))
            .await?;
        Ok(finished.cookie.ok_or(finished.status))
    }

    /// Begins an authority change and signs it with `passkey` as the person `cookie`
    /// belongs to.
    async fn authorize(
        &self,
        cookie: &str,
        passkey: &Authenticator,
        change: Value,
    ) -> TestResult<Answer> {
        let begun = self
            .send(Call::post("/api/authority/begin", json!({"change": change})).with(cookie))
            .await?;
        assert_eq!(begun.status, StatusCode::OK, "{}", begun.body);
        let answer = passkey.assert(&begun.body["public_key"]);
        self.send(Call::post("/api/authority/finish", json!({"credential": answer})).with(cookie))
            .await
    }
}

#[tokio::test(flavor = "current_thread")]
async fn setup_runs_once_and_shows_the_recovery_words_once() -> TestResult {
    let console = Console::public()?;
    let token = console.setup_token()?;
    let path = format!("/api/setup/{token}");
    let info = console.send(Call::get(&path)).await?;
    assert_eq!(info.status, StatusCode::OK);
    assert_eq!(info.body["network"]["name"], "home");

    let (_, cookie) = console.set_up().await?;
    let used = console.send(Call::get(&path)).await?;
    assert_eq!(used.status, StatusCode::GONE);
    assert_eq!(used.body["code"], "session_used");
    let again = console
        .send(Call::post(
            "/api/setup/begin",
            json!({"token": token, "name": "Eve"}),
        ))
        .await?;
    assert_eq!(again.body["code"], "session_used");
    assert!(
        setup_link(&*console.shared.store, DOMAIN, 0)?.is_none(),
        "no new link once there is an owner"
    );

    let words = console
        .send(Call::post("/api/setup/recovery", json!({})).with(&cookie))
        .await?;
    assert_eq!(words.status, StatusCode::OK);
    assert_eq!(words.body["words"].as_array().map(Vec::len), Some(12));
    let twice = console
        .send(Call::post("/api/setup/recovery", json!({})).with(&cookie))
        .await?;
    assert_eq!(twice.status, StatusCode::CONFLICT);

    let status = console.send(Call::get("/api/status").with(&cookie)).await?;
    assert_eq!(status.body["mode"], "public");
    assert_eq!(status.body["me"]["name"], "Ada");
    assert_eq!(status.body["me"]["role"], "owner");
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn a_setup_link_lapses_after_thirty_minutes() -> TestResult {
    let console = Console::public()?;
    let token = console.setup_token()?;
    console.shared.clock.advance(31 * MINUTE);
    let lapsed = console
        .send(Call::get(&format!("/api/setup/{token}")))
        .await?;
    assert_eq!(lapsed.status, StatusCode::GONE);
    assert_eq!(lapsed.body["code"], "session_expired");
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn sessions_slide_for_twelve_hours_and_end_on_sign_out() -> TestResult {
    let console = Console::public()?;
    let (owner, _) = console.set_up().await?;
    let stranger = Authenticator::new(9, DOMAIN, ORIGIN).ok_or("no key")?;
    assert_eq!(
        console.sign_in(&stranger).await?,
        Err(StatusCode::UNAUTHORIZED)
    );
    let elsewhere =
        Authenticator::new(1, "evil.example.com", "https://evil.example.com").ok_or("no key")?;
    assert_eq!(
        console.sign_in(&elsewhere).await?,
        Err(StatusCode::UNAUTHORIZED),
        "an answer for another site"
    );

    let begun = console
        .send(Call::post("/api/login/begin", json!({})))
        .await?;
    let answer = owner.assert(&begun.body["public_key"]);
    let login = Call::post("/api/login/finish", json!({"credential": answer.clone()}));
    let cookie = console.send(login).await?.cookie.ok_or("not signed in")?;
    let replay = Call::post("/api/login/finish", json!({"credential": answer}));
    assert_eq!(
        console.send(replay).await?.status,
        StatusCode::UNAUTHORIZED,
        "a prompt is answered once"
    );

    assert_eq!(console.me(&cookie).await?, StatusCode::OK);
    console.shared.clock.advance(11 * HOUR);
    assert_eq!(console.me(&cookie).await?, StatusCode::OK, "and slides");
    console.shared.clock.advance(11 * HOUR);
    assert_eq!(console.me(&cookie).await?, StatusCode::OK, "still in use");
    console.shared.clock.advance(13 * HOUR);
    assert_eq!(
        console.me(&cookie).await?,
        StatusCode::UNAUTHORIZED,
        "idle too long"
    );

    let fresh = console.sign_in(&owner).await?.map_err(|_| "refused")?;
    let out = console
        .send(Call::post("/api/logout", json!({})).with(&fresh))
        .await?;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert!(out.cookie.is_some_and(|cookie| cookie == "__Host-kith="));
    assert_eq!(console.me(&fresh).await?, StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[expect(
    clippy::too_many_lines,
    reason = "one person's story, from invite to removal"
)]
async fn people_join_change_roles_and_leave_by_passkey() -> TestResult {
    let console = Console::public()?;
    let (owner, ada) = console.set_up().await?;

    let invited = console
        .send(Call::post("/api/people", json!({"name": "Bo", "role": "member"})).with(&ada))
        .await?;
    assert_eq!(invited.status, StatusCode::OK, "{}", invited.body);
    assert!(invited.body["qr_svg"].is_string());
    let link = invited.body["link"].as_str().ok_or("no link")?;
    let token = link.rsplit('/').next().ok_or("no token")?.to_owned();
    assert!(link.starts_with("https://vpn.example.com/register/"));
    let pending = invited.body["person"]["id"]
        .as_str()
        .ok_or("no id")?
        .to_owned();

    let info = console
        .send(Call::get(&format!("/api/register/{token}")))
        .await?;
    assert_eq!(info.body["invited_by"], "Ada");
    let bo = Authenticator::new(2, DOMAIN, ORIGIN).ok_or("no key")?;
    let begun = console
        .send(Call::post("/api/register/begin", json!({"token": token})))
        .await?;
    let registered = console
        .send(Call::post(
            "/api/register/finish",
            json!({"token": token, "credential": bo.register(&begun.body["public_key"])}),
        ))
        .await?;
    assert_eq!(
        registered.body["state"], "registered",
        "{}",
        registered.body
    );
    let again = console
        .send(Call::post("/api/register/begin", json!({"token": token})))
        .await?;
    assert_eq!(again.body["code"], "session_used");
    assert_eq!(
        console.sign_in(&bo).await?,
        Err(StatusCode::UNAUTHORIZED),
        "not in the ledger yet"
    );

    let added = console
        .authorize(
            &ada,
            &owner,
            json!({"kind": "add_admin", "person": pending}),
        )
        .await?;
    assert_eq!(added.status, StatusCode::OK, "{}", added.body);
    let people = console.send(Call::get("/api/people").with(&ada)).await?;
    let bo_view = people
        .body
        .as_array()
        .and_then(|all| all.iter().find(|person| person["name"] == "Bo"))
        .ok_or("Bo is not listed")?
        .clone();
    assert_eq!(bo_view["state"], "active");
    let bo_id = bo_view["id"].as_str().ok_or("no id")?.to_owned();
    let bo_cookie = console.sign_in(&bo).await?.map_err(|_| "Bo was refused")?;

    let as_member = console
        .send(Call::post("/api/people", json!({"name": "X", "role": "member"})).with(&bo_cookie))
        .await?;
    assert_eq!(as_member.body["code"], "permission_denied");
    let status = console.send(Call::get("/api/status").with(&ada)).await?;
    let network = status.body["networks"][0]["id"]
        .as_str()
        .ok_or("no network")?
        .to_owned();
    let admin_change = console
        .send(
            Call::post(
                &format!("/api/networks/{network}/invites"),
                json!({"expires_secs": 60, "uses": 1}),
            )
            .with(&bo_cookie),
        )
        .await?;
    assert_eq!(admin_change.body["code"], "permission_denied");

    let promoted = console
        .authorize(
            &ada,
            &owner,
            json!({"kind": "set_role", "person": bo_id, "role": "admin"}),
        )
        .await?;
    assert_eq!(promoted.status, StatusCode::OK, "{}", promoted.body);

    let begun = console
        .send(
            Call::post(
                "/api/authority/begin",
                json!({"change": {"kind": "remove_person", "person": bo_id}}),
            )
            .with(&ada),
        )
        .await?;
    let forged = console
        .send(
            Call::post(
                "/api/authority/finish",
                json!({"credential": bo.assert(&begun.body["public_key"])}),
            )
            .with(&bo_cookie),
        )
        .await?;
    assert_eq!(
        forged.status,
        StatusCode::BAD_REQUEST,
        "someone else's prompt"
    );
    let begun = console
        .send(
            Call::post(
                "/api/authority/begin",
                json!({"change": {"kind": "remove_person", "person": bo_id}}),
            )
            .with(&ada),
        )
        .await?;
    let removed = console
        .send(
            Call::post(
                "/api/authority/finish",
                json!({"credential": owner.assert(&begun.body["public_key"])}),
            )
            .with(&ada),
        )
        .await?;
    assert_eq!(removed.status, StatusCode::OK, "{}", removed.body);
    let gone = console.send(Call::get("/api/me").with(&bo_cookie)).await?;
    assert_eq!(
        gone.status,
        StatusCode::UNAUTHORIZED,
        "signed out everywhere"
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn an_admin_invite_works_only_once_its_inviter_confirms_it() -> TestResult {
    let console = Console::public()?;
    let (owner, ada) = console.set_up().await?;
    let invited = console
        .send(Call::post("/api/people", json!({"name": "Cy", "role": "admin"})).with(&ada))
        .await?;
    let token = invited.body["link"]
        .as_str()
        .and_then(|link| link.rsplit('/').next())
        .ok_or("no token")?
        .to_owned();
    let person = invited.body["person"]["id"].clone();
    let early = console
        .send(Call::post("/api/register/begin", json!({"token": token})))
        .await?;
    assert_eq!(early.status, StatusCode::FORBIDDEN);
    let confirmed = console
        .authorize(&ada, &owner, json!({"kind": "add_admin", "person": person}))
        .await?;
    assert_eq!(confirmed.status, StatusCode::OK, "{}", confirmed.body);
    let begun = console
        .send(Call::post("/api/register/begin", json!({"token": token})))
        .await?;
    assert_eq!(begun.status, StatusCode::OK, "{}", begun.body);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn a_device_code_hands_out_one_invite_within_ten_minutes() -> TestResult {
    let console = Console::public()?;
    let (_, ada) = console.set_up().await?;
    let made = console
        .send(Call::post("/api/device-codes", json!({})).with(&ada))
        .await?;
    assert_eq!(made.status, StatusCode::OK, "{}", made.body);
    let code = made.body["code"].as_str().ok_or("no code")?.to_owned();
    assert_eq!(code.len(), 6);
    assert_eq!(
        made.body["command"],
        format!("kith join https://vpn.example.com/j/{code}")
    );

    let path = format!("/j/{}", code.to_lowercase());
    let joined = console.send(Call::get(&path)).await?;
    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    assert!(
        joined.body["invite_link"]
            .as_str()
            .is_some_and(|link| link.starts_with("kith://join/"))
    );
    let twice = console.send(Call::get(&path)).await?;
    assert_eq!(twice.body["code"], "session_used");

    let late = console
        .send(Call::post("/api/device-codes", json!({})).with(&ada))
        .await?;
    console.shared.clock.advance(11 * MINUTE);
    let lapsed = console
        .send(Call::get(&format!(
            "/j/{}",
            late.body["code"].as_str().ok_or("no code")?
        )))
        .await?;
    assert_eq!(lapsed.body["code"], "session_expired");

    for _ in 0..10 {
        let guess = Call {
            from: Some([203, 0, 113, 7]),
            ..Call::get("/j/ZZZZZZ")
        };
        assert_eq!(console.send(guess).await?.status, StatusCode::NOT_FOUND);
    }
    let limited = console
        .send(Call {
            from: Some([203, 0, 113, 7]),
            ..Call::get("/j/ZZZZZZ")
        })
        .await?;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(limited.body["code"], "rate_limited");
    let other = console
        .send(Call {
            from: Some([203, 0, 113, 8]),
            ..Call::get("/j/ZZZZZZ")
        })
        .await?;
    assert_eq!(other.status, StatusCode::NOT_FOUND, "another address");
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn the_public_console_answers_only_its_domain_over_https() -> TestResult {
    let console = Console::public()?;
    let status = console.send(Call::get("/api/status")).await?;
    assert_eq!(status.status, StatusCode::OK);
    assert_eq!(status.body, json!({"mode": "public", "me": null}));
    assert_eq!(
        status
            .headers
            .get(header::STRICT_TRANSPORT_SECURITY)
            .and_then(|value| value.to_str().ok()),
        Some("max-age=31536000")
    );
    let rebound = console
        .send(Call {
            host: "evil.example.com",
            ..Call::get("/api/status")
        })
        .await?;
    assert_eq!(rebound.status, StatusCode::FORBIDDEN);
    let local = console
        .send(Call {
            host: "127.0.0.1:443",
            ..Call::get("/j/ABCDEF")
        })
        .await?;
    assert_eq!(local.status, StatusCode::FORBIDDEN);
    let plain = console
        .send(Call {
            origin: Some("http://vpn.example.com"),
            ..Call::post("/api/login/begin", json!({}))
        })
        .await?;
    assert_eq!(plain.status, StatusCode::FORBIDDEN, "not over https");
    let people = console.send(Call::get("/api/people")).await?;
    assert_eq!(people.status, StatusCode::UNAUTHORIZED);
    let link = console
        .send(Call::post("/api/session", json!({"token": "00"})))
        .await?;
    assert_eq!(link.status, StatusCode::UNAUTHORIZED, "no link sign-in");
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn history_keeps_the_journal_the_console_saw() -> TestResult {
    let console = Console::public()?;
    let (_, ada) = console.set_up().await?;
    let mut entries = 0;
    for _ in 0..50 {
        let page = console
            .send(Call::get("/api/history/journal?limit=5").with(&ada))
            .await?;
        entries = page.body["entries"].as_array().map_or(0, Vec::len);
        if entries > 0 {
            assert!(page.body["entries"][0]["entry"]["kind"].is_string());
            break;
        }
        tokio::time::sleep(core::time::Duration::from_millis(20)).await;
    }
    assert!(entries > 0, "the recorded journal was kept");
    Ok(())
}
