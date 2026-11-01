//! Drives the router in process against the fixture and a fake daemon: the one-time link,
//! the `Host` and `Origin` guards, the security headers, and the event stream resuming,
//! reporting gaps and following the daemon through restarts.

#![cfg(unix)]

#[cfg(test)]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions fail the test; ? keeps the setup short"
)]
mod tests {
    use core::time::Duration;
    use std::path::{Path, PathBuf};

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, Response, StatusCode, header};
    use http_body_util::BodyExt as _;
    use kith_ipc::{
        Entry, EntryKind, IpcServer, NetworkSummary, Request as Ipc, Response as Answer, Status,
    };
    use kith_types::NetworkId;
    use kith_ui_core::{Asset, Config, Source, Token, load_fixture, router};
    use serde_json::{Value, json};
    use tokio::sync::broadcast;
    use tokio::task::{JoinHandle, JoinSet};
    use tower::ServiceExt as _;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    const HOST: &str = "127.0.0.1:41680";
    const ORIGIN: &str = "http://127.0.0.1:41680";
    const VITE: &str = "http://localhost:5173";
    const CSP: &str = "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; \
script-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; \
form-action 'self'; frame-ancestors 'none'";
    /// The fixture's boot id and last journal seq.
    const FIXTURE_BOOT: &str = "9f2c1a7e5b3d4c60";
    const FIXTURE_SEQ: u64 = 230;
    /// Longer than a tick plus a reconnect to the daemon.
    const WITHIN: Duration = Duration::from_secs(5);
    static ASSETS: &[Asset] = &[Asset {
        path: "index.html",
        body: b"<head><!--kith:boot--></head><body><!--kith:signed-out--></body>",
        brotli: None,
    }];

    fn fixture() -> TestResult<Source> {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../kith-ui/web/fixtures/office.json");
        Ok(Source::Fixture(Box::new(load_fixture(&path)?)))
    }

    fn app(source: Source) -> TestResult<(Router, String)> {
        let token = Token::random()?;
        let hex = token.hex();
        let app = router(Config {
            source,
            port: 41680,
            token,
            assets: Some(ASSETS),
            dev_origins: vec![VITE.to_owned()],
        })?;
        Ok((app, hex))
    }

    fn get(path: &str) -> axum::http::request::Builder {
        Request::get(path).header(header::HOST, HOST)
    }

    fn post(path: &str, origin: Option<&str>, body: &Value) -> TestResult<Request<Body>> {
        let mut request = Request::post(path)
            .header(header::HOST, HOST)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        Ok(request.body(Body::from(body.to_string()))?)
    }

    async fn send(app: &Router, request: Request<Body>) -> TestResult<Response<Body>> {
        Ok(app.clone().oneshot(request).await?)
    }

    async fn json_body(response: Response<Body>) -> TestResult<Value> {
        let bytes = response.into_body().collect().await?.to_bytes();
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Trades the link token for the session cookie, as `name=secret`.
    async fn sign_in(app: &Router, token: &str) -> TestResult<String> {
        let response = send(
            app,
            post("/api/session", Some(ORIGIN), &json!({ "token": token }))?,
        )
        .await?;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .ok_or("no cookie")?
            .to_str()?;
        assert!(cookie.starts_with("kith_ui_41680="), "{cookie}");
        assert!(
            cookie.ends_with("; HttpOnly; SameSite=Strict; Path=/"),
            "{cookie}"
        );
        Ok(cookie.split(';').next().ok_or("empty cookie")?.to_owned())
    }

    /// One server-sent event.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Sse {
        name: String,
        id: Option<String>,
        data: String,
    }

    /// Reads events off a streaming response body.
    struct Reader {
        body: Body,
        buffer: String,
    }

    impl Reader {
        async fn open(app: &Router, cookie: &str, last: &str) -> TestResult<Self> {
            let response = send(
                app,
                get(&format!("/api/events?last={last}"))
                    .header(header::COOKIE, cookie)
                    .header("sec-fetch-site", "same-origin")
                    .body(Body::empty())?,
            )
            .await?;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "text/event-stream"
            );
            Ok(Self {
                body: response.into_body(),
                buffer: String::new(),
            })
        }

        /// The next event, or an error when none arrives in [`WITHIN`].
        async fn next(&mut self) -> TestResult<Sse> {
            loop {
                if let Some((block, rest)) = self.buffer.split_once("\n\n") {
                    let mut event = Sse {
                        name: "message".to_owned(),
                        id: None,
                        data: String::new(),
                    };
                    for line in block.lines() {
                        match line.split_once(':') {
                            Some(("event", value)) => event.name = value.trim().to_owned(),
                            Some(("id", value)) => event.id = Some(value.trim().to_owned()),
                            Some(("data", value)) => event.data.push_str(value.trim()),
                            _ => {}
                        }
                    }
                    self.buffer = rest.to_owned();
                    return Ok(event);
                }
                let frame = tokio::time::timeout(WITHIN, self.body.frame())
                    .await?
                    .ok_or("the stream ended")??;
                if let Ok(data) = frame.into_data() {
                    self.buffer.push_str(core::str::from_utf8(&data)?);
                }
            }
        }

        /// The next event that is not a tick.
        async fn next_change(&mut self) -> TestResult<Sse> {
            loop {
                let event = self.next().await?;
                if event.name != "tick" {
                    return Ok(event);
                }
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_link_starts_one_session_and_nothing_else_does() -> TestResult {
        let (app, token) = app(fixture()?)?;
        let status = || get("/api/status").body(Body::empty());
        assert_eq!(
            send(&app, status()?).await?.status(),
            StatusCode::UNAUTHORIZED
        );

        let wrong = send(
            &app,
            post(
                "/api/session",
                Some(ORIGIN),
                &json!({ "token": "0".repeat(64) }),
            )?,
        )
        .await?;
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(json_body(wrong).await?["code"], "unauthorized");
        let no_origin = send(
            &app,
            post("/api/session", None, &json!({ "token": token }))?,
        )
        .await?;
        assert_eq!(no_origin.status(), StatusCode::FORBIDDEN);

        let cookie = sign_in(&app, &token).await?;
        let again = send(
            &app,
            post("/api/session", Some(ORIGIN), &json!({ "token": token }))?,
        )
        .await?;
        assert_eq!(again.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(json_body(again).await?["code"], "session_used");

        let signed_in = get("/api/status")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())?;
        assert_eq!(send(&app, signed_in).await?.status(), StatusCode::OK);
        let forged = get("/api/status")
            .header(header::COOKIE, format!("kith_ui_41680={}", "1".repeat(64)))
            .body(Body::empty())?;
        assert_eq!(send(&app, forged).await?.status(), StatusCode::UNAUTHORIZED);
        let other_port = get("/api/status")
            .header(header::COOKIE, cookie.replace("41680", "41681"))
            .body(Body::empty())?;
        assert_eq!(
            send(&app, other_port).await?.status(),
            StatusCode::UNAUTHORIZED
        );
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn every_request_must_name_this_machine() -> TestResult {
        let (app, token) = app(fixture()?)?;
        let cookie = sign_in(&app, &token).await?;
        for host in [
            "evil.test",
            "127.0.0.1:41681",
            "localhost",
            "127.0.0.1.evil.test:41680",
        ] {
            for path in ["/", "/api/status", "/assets/x.js"] {
                let request = Request::get(path)
                    .header(header::HOST, host)
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())?;
                let response = send(&app, request).await?;
                assert_eq!(response.status(), StatusCode::FORBIDDEN, "{host}{path}");
                assert_eq!(json_body(response).await?["code"], "forbidden");
            }
        }
        let no_host = Request::get("/api/status")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())?;
        assert_eq!(send(&app, no_host).await?.status(), StatusCode::FORBIDDEN);
        let localhost = Request::get("/api/status")
            .header(header::HOST, "localhost:41680")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())?;
        assert_eq!(send(&app, localhost).await?.status(), StatusCode::OK);
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn changes_and_the_stream_come_only_from_the_page() -> TestResult {
        let (app, token) = app(fixture()?)?;
        let cookie = sign_in(&app, &token).await?;
        let change = |origin| -> TestResult<Request<Body>> {
            let mut request = post(
                "/api/exit",
                origin,
                &json!({ "network": null, "node": null }),
            )?;
            request
                .headers_mut()
                .insert(header::COOKIE, cookie.parse()?);
            Ok(request)
        };
        for origin in [
            None,
            Some("http://evil.test"),
            Some("null"),
            Some("http://127.0.0.1:41681"),
        ] {
            let response = send(&app, change(origin)?).await?;
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{origin:?}");
        }
        for origin in [ORIGIN, "http://localhost:41680", VITE] {
            let response = send(&app, change(Some(origin))?).await?;
            assert_ne!(response.status(), StatusCode::FORBIDDEN, "{origin}");
        }

        let stream = |origin: Option<&str>, site: Option<&str>| -> TestResult<Request<Body>> {
            let mut request = get("/api/events").header(header::COOKIE, &cookie);
            if let Some(origin) = origin {
                request = request.header(header::ORIGIN, origin);
            }
            if let Some(site) = site {
                request = request.header("sec-fetch-site", site);
            }
            Ok(request.body(Body::empty())?)
        };
        for (origin, site) in [
            (Some("http://evil.test"), Some("cross-site")),
            (None, Some("cross-site")),
            (None, Some("same-site")),
        ] {
            let response = send(&app, stream(origin, site)?).await?;
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "{origin:?} {site:?}"
            );
        }
        for (origin, site) in [
            (None, Some("same-origin")),
            (Some(ORIGIN), Some("same-origin")),
            (None, None),
        ] {
            let response = send(&app, stream(origin, site)?).await?;
            assert_eq!(response.status(), StatusCode::OK, "{origin:?} {site:?}");
        }
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn every_answer_carries_the_security_headers() -> TestResult {
        let (app, token) = app(fixture()?)?;
        let cookie = sign_in(&app, &token).await?;
        let requests = [
            (get("/").body(Body::empty())?, StatusCode::OK),
            (
                get("/n/home")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())?,
                StatusCode::OK,
            ),
            (
                get("/api/status")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())?,
                StatusCode::OK,
            ),
            (
                get("/api/status").body(Body::empty())?,
                StatusCode::UNAUTHORIZED,
            ),
            (
                Request::get("/")
                    .header(header::HOST, "evil.test")
                    .body(Body::empty())?,
                StatusCode::FORBIDDEN,
            ),
            (
                get("/n/home/inbox").body(Body::empty())?,
                StatusCode::NOT_FOUND,
            ),
            (
                get("/api/nothing")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())?,
                StatusCode::NOT_FOUND,
            ),
        ];
        for (request, expected) in requests {
            let path = request.uri().path().to_owned();
            let response = send(&app, request).await?;
            assert_eq!(response.status(), expected, "{path}");
            let headers = response.headers();
            assert_eq!(headers[header::CONTENT_SECURITY_POLICY], CSP, "{path}");
            assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff", "{path}");
            assert_eq!(headers[header::REFERRER_POLICY], "no-referrer", "{path}");
            assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY", "{path}");
            if path.starts_with("/api/") {
                assert_eq!(headers[header::CACHE_CONTROL], "no-store", "{path}");
            }
        }
        let missing = get("/api/nothing")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())?;
        assert_eq!(
            json_body(send(&app, missing).await?).await?["code"],
            "not_found"
        );
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_page_resumes_from_its_last_entry() -> TestResult {
        let (app, token) = app(fixture()?)?;
        let cookie = sign_in(&app, &token).await?;
        let page = get("/n/home")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())?;
        let page = send(&app, page)
            .await?
            .into_body()
            .collect()
            .await?
            .to_bytes();
        let page = core::str::from_utf8(&page)?;
        let boot = page
            .split(r#"id="boot">"#)
            .nth(1)
            .and_then(|rest| rest.split("</script>").next())
            .ok_or("no boot script")?;
        let boot: Value = serde_json::from_str(boot)?;
        assert_eq!(boot["boot"], FIXTURE_BOOT);
        let seq = boot["seq"].as_u64().ok_or("no seq")?;
        assert!(seq >= FIXTURE_SEQ);
        assert!(boot["last_tick"]["links"].is_array());
        let tail: Vec<usize> = boot["traffic_tail"]
            .as_array()
            .ok_or("no traffic")?
            .iter()
            .filter_map(|series| series["samples"].as_array().map(Vec::len))
            .collect();
        assert_eq!(tail.iter().max(), Some(&150), "five minutes per device");

        let mut from_snapshot =
            Reader::open(&app, &cookie, &format!("{FIXTURE_BOOT}:{seq}")).await?;
        let (mut tick, mut replayed) = (None, None);
        while tick.is_none() || replayed.is_none() {
            let event = from_snapshot.next().await?;
            match event.name.as_str() {
                "tick" => tick = tick.or(Some(event)),
                _ => replayed = replayed.or(Some(event)),
            }
        }
        let tick = tick.ok_or("no tick")?;
        assert_eq!(tick.id, None, "a tick is never replayed");
        let tick: Value = serde_json::from_str(&tick.data)?;
        assert!(tick["at_ms"].is_u64() && tick["traffic"].is_array() && tick["links"].is_array());
        let replayed = replayed.ok_or("no entry")?;
        assert_eq!(replayed.name, "entry");
        assert_eq!(replayed.id, Some(format!("{FIXTURE_BOOT}:{}", seq + 1)));

        let mut behind = Reader::open(&app, &cookie, &format!("{FIXTURE_BOOT}:225")).await?;
        for expected in 226..=FIXTURE_SEQ {
            let entry = behind.next_change().await?;
            assert_eq!(entry.name, "entry");
            assert_eq!(entry.id, Some(format!("{FIXTURE_BOOT}:{expected}")));
            assert_eq!(serde_json::from_str::<Value>(&entry.data)?["seq"], expected);
        }

        let header_wins = get(&format!("/api/events?last={FIXTURE_BOOT}:200"))
            .header(header::COOKIE, &cookie)
            .header("last-event-id", format!("{FIXTURE_BOOT}:229"))
            .body(Body::empty())?;
        let mut resumed = Reader {
            body: send(&app, header_wins).await?.into_body(),
            buffer: String::new(),
        };
        assert_eq!(
            resumed.next_change().await?.id,
            Some(format!("{FIXTURE_BOOT}:230"))
        );

        let mut stale = Reader::open(&app, &cookie, "0123456789abcdef:12").await?;
        let reset = stale.next().await?;
        assert_eq!(reset.name, "reset");
        assert_eq!(
            serde_json::from_str::<Value>(&reset.data)?["boot"],
            FIXTURE_BOOT
        );
        Ok(())
    }

    fn status() -> Status {
        Status {
            node: None,
            networks: vec![NetworkSummary {
                id: NetworkId::from_bytes([0xab; 32]),
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

    fn entry(seq: u64, run: u64) -> Entry {
        Entry {
            seq,
            at_ms: run * 1_000_000 + seq,
            kind: EntryKind::Log {
                level: "info".into(),
                target: "kith".into(),
                message: format!("run {run} entry {seq}"),
            },
        }
    }

    /// A daemon on `socket` whose journal holds `journal`; entries sent on the returned
    /// channel reach subscribers live. Aborting the task closes every connection.
    fn daemon(
        socket: &Path,
        journal: Vec<Entry>,
    ) -> TestResult<(JoinHandle<()>, broadcast::Sender<Entry>)> {
        let server = IpcServer::bind(socket)?;
        let (live, _) = broadcast::channel(16);
        let feed = live.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            while let Ok(mut connection) = server.accept().await {
                let journal = journal.clone();
                let mut fresh = feed.subscribe();
                connections.spawn(async move {
                    while let Ok(Some(request)) = connection.next_request().await {
                        #[expect(
                            clippy::wildcard_enum_match_arm,
                            reason = "the fake answers only what the server asks"
                        )]
                        let response = match request {
                            Ipc::Journal { since } => {
                                let next = journal.last().map_or(0, |entry| entry.seq + 1);
                                if connection
                                    .respond(Answer::Journaling { next })
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                                let from = since.unwrap_or(0);
                                for entry in journal.iter().filter(|entry| entry.seq >= from) {
                                    if connection.send_entry(entry.clone()).await.is_err() {
                                        return;
                                    }
                                }
                                while let Ok(entry) = fresh.recv().await {
                                    if connection.send_entry(entry).await.is_err() {
                                        return;
                                    }
                                }
                                return;
                            }
                            Ipc::Traffic => Answer::Traffic(Vec::new()),
                            Ipc::Status => Answer::Status(status()),
                            Ipc::Topology { .. } => Answer::Topology(Vec::new()),
                            _ => Answer::Error {
                                code: kith_ipc::ErrorCode::Internal,
                                message: "not faked".into(),
                            },
                        };
                        if connection.respond(response).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        Ok((task, live))
    }

    fn socket(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("kith-ui-{name}-{}.sock", std::process::id()))
    }

    /// The boot id the server gives the daemon's run, learned from a `reset`.
    async fn current_boot(app: &Router, cookie: &str) -> TestResult<String> {
        let mut reader = Reader::open(app, cookie, "unknown:0").await?;
        let reset = reader.next_change().await?;
        assert_eq!(reset.name, "reset");
        Ok(serde_json::from_str::<Value>(&reset.data)?["boot"]
            .as_str()
            .ok_or("no boot")?
            .to_owned())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_page_behind_what_is_kept_is_told_of_the_gap() -> TestResult {
        let socket = socket("gap");
        let (task, _live) = daemon(&socket, (100..105).map(|seq| entry(seq, 1)).collect())?;
        let (app, token) = app(Source::Daemon(socket.clone()))?;
        let cookie = sign_in(&app, &token).await?;
        let boot = current_boot(&app, &cookie).await?;

        let mut reader = Reader::open(&app, &cookie, &format!("{boot}:50")).await?;
        let gap = reader.next_change().await?;
        assert_eq!(gap.name, "gap");
        assert_eq!(gap.id, None);
        assert_eq!(
            serde_json::from_str::<Value>(&gap.data)?,
            json!({ "missed_from": 51, "missed_to": 99 })
        );
        for expected in 100..105 {
            assert_eq!(
                reader.next_change().await?.id,
                Some(format!("{boot}:{expected}"))
            );
        }
        task.abort();
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_stream_follows_the_daemon_through_reconnects_and_restarts() -> TestResult {
        let socket = socket("restart");
        let first: Vec<Entry> = (0..10).map(|seq| entry(seq, 1)).collect();
        let (task, _live) = daemon(&socket, first.clone())?;
        let (app, token) = app(Source::Daemon(socket.clone()))?;
        let cookie = sign_in(&app, &token).await?;
        let boot = current_boot(&app, &cookie).await?;
        let mut reader = Reader::open(&app, &cookie, &format!("{boot}:9")).await?;

        task.abort();
        assert_eq!(reader.next_change().await?.name, "daemon_down");
        let longer: Vec<Entry> = (0..12).map(|seq| entry(seq, 1)).collect();
        let (task, live) = daemon(&socket, longer)?;
        assert_eq!(reader.next_change().await?.name, "daemon_up");
        for expected in [10, 11] {
            assert_eq!(
                reader.next_change().await?.id,
                Some(format!("{boot}:{expected}"))
            );
        }

        task.abort();
        assert_eq!(reader.next_change().await?.name, "daemon_down");
        let (task, live_after) = daemon(&socket, (0..3).map(|seq| entry(seq, 2)).collect())?;
        drop(live);
        assert_eq!(reader.next_change().await?.name, "daemon_up");
        let reset = reader.next_change().await?;
        assert_eq!(reset.name, "reset");
        let reset: Value = serde_json::from_str(&reset.data)?;
        let restarted = reset["boot"].as_str().ok_or("no boot")?.to_owned();
        assert_ne!(restarted, boot);
        assert_eq!(reset["started_at_ms"], 2_000_000);

        let mut fresh = Reader::open(&app, &cookie, &format!("{restarted}:2")).await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        live_after.send(entry(3, 2))?;
        assert_eq!(
            reader.next_change().await?.id,
            Some(format!("{restarted}:3"))
        );
        assert_eq!(
            fresh.next_change().await?.id,
            Some(format!("{restarted}:3"))
        );
        task.abort();
        drop(std::fs::remove_file(&socket));
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_stream_opened_without_a_daemon_says_it_is_down() -> TestResult {
        let (app, token) = app(Source::Daemon(socket("absent")))?;
        let cookie = sign_in(&app, &token).await?;
        let mut reader = Reader::open(&app, &cookie, "x:0").await?;
        let first = reader.next().await?;
        let second = reader.next().await?;
        assert_eq!(
            [first.name.as_str(), second.name.as_str()],
            ["reset", "daemon_down"]
        );
        Ok(())
    }
}
