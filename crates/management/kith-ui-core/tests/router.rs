//! Drives the router in process: the session, the guards, the page and one change that
//! reaches a fake daemon.

#![cfg(unix)]

#[cfg(test)]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions fail the test; ? keeps the setup short"
)]
mod tests {
    use std::path::{Path, PathBuf};

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, Response, StatusCode, header};
    use http_body_util::BodyExt as _;
    use kith_ipc::{
        DnsRecord, IpcServer, NetworkSummary, Request as Ipc, Response as Answer, Status,
    };
    use kith_types::NetworkId;
    use kith_ui_core::{Asset, Config, Source, Token, load_fixture, router};
    use serde_json::{Value, json};
    use tokio::sync::mpsc;
    use tower::ServiceExt as _;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    const PORT: u16 = 41680;
    const ORIGIN: &str = "http://127.0.0.1:41680";
    const NET: NetworkId = NetworkId::from_bytes([0xab; 32]);
    static ASSETS: &[Asset] = &[
        Asset {
            path: "index.html",
            body: b"<head><!--kith:boot--></head><body><!--kith:signed-out--><div id=\"app\"></div></body>",
            brotli: None,
        },
        Asset {
            path: "assets/app-1a2b.js",
            body: b"export {}",
            brotli: Some(b"compressed"),
        },
    ];

    fn office() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../kith-ui/web/fixtures/office.json")
    }

    fn app(source: Source) -> TestResult<(Router, String)> {
        let token = Token::random()?;
        let hex = token.hex();
        let app = router(Config {
            source,
            port: PORT,
            token,
            assets: Some(ASSETS),
            dev_origins: Vec::new(),
        })?;
        Ok((app, hex))
    }

    fn get(path: &str) -> axum::http::request::Builder {
        Request::get(path).header(header::HOST, "127.0.0.1:41680")
    }

    fn post(path: &str, origin: &str, body: &Value) -> TestResult<Request<Body>> {
        Ok(Request::post(path)
            .header(header::HOST, "127.0.0.1:41680")
            .header(header::ORIGIN, origin)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))?)
    }

    async fn text(response: Response<Body>) -> TestResult<String> {
        let bytes = response.into_body().collect().await?.to_bytes();
        Ok(String::from_utf8(bytes.to_vec())?)
    }

    async fn sign_in(app: &Router, token: &str) -> TestResult<String> {
        let response = app
            .clone()
            .oneshot(post("/api/session", ORIGIN, &json!({ "token": token }))?)
            .await?;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .ok_or("no cookie")?
            .to_str()?
            .to_owned();
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
        Ok(cookie.split(';').next().ok_or("empty cookie")?.to_owned())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_one_time_link_opens_the_fixture_page() -> TestResult {
        let (app, token) = app(Source::Fixture(Box::new(load_fixture(&office())?)))?;

        let foreign_host = Request::get("/api/status")
            .header(header::HOST, "evil.test")
            .body(Body::empty())?;
        let response = app.clone().oneshot(foreign_host).await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let response = app
            .clone()
            .oneshot(get("/api/status").body(Body::empty())?)
            .await?;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let signed_out = text(app.clone().oneshot(get("/").body(Body::empty())?).await?).await?;
        assert!(signed_out.contains("This page needs a fresh link."));
        assert!(signed_out.contains(r#"<script type="application/json" id="boot">null</script>"#));

        let foreign = post(
            "/api/session",
            "http://evil.test",
            &json!({ "token": token }),
        )?;
        assert_eq!(
            app.clone().oneshot(foreign).await?.status(),
            StatusCode::FORBIDDEN
        );
        let cookie = sign_in(&app, &token).await?;
        let reused = app
            .clone()
            .oneshot(post("/api/session", ORIGIN, &json!({ "token": token }))?)
            .await?;
        assert_eq!(reused.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            serde_json::from_str::<Value>(&text(reused).await?)?["code"],
            "session_used"
        );

        let page = app
            .clone()
            .oneshot(
                get("/n/home/devices")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())?,
            )
            .await?;
        assert!(page.headers().contains_key(header::CONTENT_SECURITY_POLICY));
        let page = text(page).await?;
        let boot = page
            .split(r#"id="boot">"#)
            .nth(1)
            .and_then(|rest| rest.split("</script>").next())
            .ok_or("no boot script")?;
        let boot: Value = serde_json::from_str(boot)?;
        assert_eq!(boot["boot"], "9f2c1a7e5b3d4c60");
        assert_eq!(boot["status"]["memberships"][0]["name"], "laptop");
        assert!(!page.contains("fresh link"));

        let missing = app
            .clone()
            .oneshot(
                get("/n/home/inbox")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);

        let script = app
            .clone()
            .oneshot(
                get("/assets/app-1a2b.js")
                    .header(header::ACCEPT_ENCODING, "gzip, br")
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(script.headers()[header::CONTENT_ENCODING], "br");
        assert!(
            script.headers()[header::CACHE_CONTROL]
                .to_str()?
                .contains("immutable")
        );
        Ok(())
    }

    fn status() -> Status {
        Status {
            node: None,
            networks: vec![NetworkSummary {
                id: NET,
                name: "home".into(),
                peers_online: 0,
                peers_total: 0,
                relays: 0,
                problems: Vec::new(),
                admin: true,
            }],
            exit: false,
            relay: false,
            exit_node: None,
            listen_port: 41641,
            dns_upstreams: Vec::new(),
            drops: std::collections::BTreeMap::default(),
            route_conflicts: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// A daemon that answers the server's follower, status and record changes, and reports
    /// the changes it received.
    fn fake_daemon(socket: &Path) -> TestResult<mpsc::UnboundedReceiver<Ipc>> {
        let server = IpcServer::bind(socket)?;
        let (seen, received) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok(mut connection) = server.accept().await {
                let seen = seen.clone();
                tokio::spawn(async move {
                    while let Ok(Some(request)) = connection.next_request().await {
                        #[expect(
                            clippy::wildcard_enum_match_arm,
                            reason = "the fake answers only what the test sends"
                        )]
                        let (response, change) = match &request {
                            Ipc::Journal { .. } => (Answer::Journaling { next: 0 }, false),
                            Ipc::Traffic => (Answer::Traffic(Vec::new()), false),
                            Ipc::Topology { .. } => (Answer::Topology(Vec::new()), false),
                            Ipc::Status => (Answer::Status(status()), false),
                            Ipc::SetRecord { .. } => (Answer::RecordSet, true),
                            _ => (
                                Answer::Error {
                                    code: kith_ipc::ErrorCode::Internal,
                                    message: "not faked".into(),
                                },
                                true,
                            ),
                        };
                        if (change && seen.send(request).is_err())
                            || connection.respond(response).await.is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });
        Ok(received)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_change_reaches_the_daemon_only_from_the_page() -> TestResult {
        let socket = std::env::temp_dir().join(format!("kith-ui-core-{}.sock", std::process::id()));
        drop(std::fs::remove_file(&socket));
        let mut received = fake_daemon(&socket)?;
        let (app, token) = app(Source::Daemon(socket.clone()))?;
        let cookie = sign_in(&app, &token).await?;

        let status = app
            .clone()
            .oneshot(
                get("/api/status")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(status.status(), StatusCode::OK);
        let status: Value = serde_json::from_str(&text(status).await?)?;
        assert_eq!(status["networks"][0]["id"], NET.to_string());
        assert_eq!(status["memberships"][0]["role"], "admin");

        let path = format!("/api/networks/{NET}/records");
        let record = json!({ "name": "nas", "type": "A", "value": "100.64.0.2", "based_on": null });
        let mut foreign = post(&path, "http://evil.test", &record)?;
        foreign
            .headers_mut()
            .insert(header::COOKIE, cookie.parse()?);
        assert_eq!(
            app.clone().oneshot(foreign).await?.status(),
            StatusCode::FORBIDDEN
        );

        let mut set = post(&path, ORIGIN, &record)?;
        set.headers_mut().insert(header::COOKIE, cookie.parse()?);
        let set = app.clone().oneshot(set).await?;
        assert_eq!(set.status(), StatusCode::OK);
        assert_eq!(
            serde_json::from_str::<Value>(&text(set).await?)?["state"],
            "accepted"
        );
        assert_eq!(
            received.recv().await,
            Some(Ipc::SetRecord {
                network: NET,
                name: "nas".into(),
                record: DnsRecord::A([100, 64, 0, 2].into()),
            })
        );
        drop(std::fs::remove_file(&socket));
        Ok(())
    }
}
