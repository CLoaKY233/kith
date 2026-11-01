//! Drives the HTTP API against a fake daemon on a Unix socket.

#![cfg(unix)]

#[cfg(test)]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions fail the test; ? keeps the setup short"
)]
mod tests {
    use core::net::SocketAddr;
    use core::time::Duration;
    use std::path::PathBuf;

    use kith_ipc::{
        DnsRecord, Entry, EntryKind, IpcServer, NetworkSummary, Request, Response, Status,
    };
    use kith_types::{NetworkId, NodeId};
    use kith_ui::{Server, Token};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpStream;
    use tokio::sync::mpsc;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    const NET: NetworkId = NetworkId::from_bytes([0xab; 32]);
    const NODE: NodeId = NodeId::from_bytes([0xcd; 32]);

    fn status() -> Status {
        Status {
            node: Some(NODE),
            networks: vec![NetworkSummary {
                id: NET,
                name: "home".into(),
                peers_online: 1,
                peers_total: 2,
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

    /// A daemon that answers what the test needs and reports every action it receives.
    fn fake_daemon(socket: &PathBuf) -> TestResult<mpsc::UnboundedReceiver<Request>> {
        let server = IpcServer::bind(socket)?;
        let (actions, received) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok(mut connection) = server.accept().await {
                let actions = actions.clone();
                tokio::spawn(async move {
                    while let Ok(Some(request)) = connection.next_request().await {
                        #[expect(
                            clippy::wildcard_enum_match_arm,
                            reason = "the fake answers only what the test sends"
                        )]
                        let response = match &request {
                            Request::Status => Response::Status(status()),
                            Request::Journal { since } => {
                                assert_eq!(*since, Some(7), "Last-Event-ID 6 resumes at 7");
                                connection
                                    .respond(Response::Journaling { next: 8 })
                                    .await
                                    .unwrap();
                                let entry = Entry {
                                    seq: 7,
                                    at_ms: 1_700_000_000_000,
                                    kind: EntryKind::PeerOffline {
                                        network: NET,
                                        node: NODE,
                                        name: "laptop".into(),
                                    },
                                };
                                connection.send_entry(entry).await.unwrap();
                                tokio::time::sleep(Duration::from_secs(30)).await;
                                return;
                            }
                            Request::SetRecord { .. } => Response::RecordSet,
                            _ => Response::Error {
                                code: kith_ipc::ErrorCode::Internal,
                                message: "not faked".into(),
                            },
                        };
                        actions.send(request).unwrap();
                        connection.respond(response).await.unwrap();
                    }
                });
            }
        });
        Ok(received)
    }

    struct Reply {
        status: u16,
        head: String,
        body: String,
    }

    async fn send(address: SocketAddr, request: &str) -> TestResult<Reply> {
        let mut stream = TcpStream::connect(address).await?;
        stream.write_all(request.as_bytes()).await?;
        let mut raw = String::new();
        stream.read_to_string(&mut raw).await?;
        let (head, body) = raw.split_once("\r\n\r\n").ok_or("no header end")?;
        let status = head.split(' ').nth(1).ok_or("no status")?.parse()?;
        Ok(Reply {
            status,
            head: head.to_owned(),
            body: body.to_owned(),
        })
    }

    fn get(path: &str, cookie: &str) -> String {
        format!("GET {path} HTTP/1.1\r\nHost: x\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n")
    }

    fn post(path: &str, cookie: &str, origin: Option<&str>, json: &str) -> String {
        let origin = origin
            .map(|o| format!("Origin: {o}\r\n"))
            .unwrap_or_default();
        format!(
            "POST {path} HTTP/1.1\r\nHost: x\r\nCookie: {cookie}\r\n{origin}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
            json.len()
        )
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_session_reads_status_sets_a_record_and_streams_the_journal() -> TestResult {
        let socket = std::env::temp_dir().join(format!("kith-ui-{}.sock", std::process::id()));
        drop(std::fs::remove_file(&socket));
        let mut actions = fake_daemon(&socket)?;
        let token = Token::random()?;
        let hex = token.hex();
        let server = Server::new(kith_ui::bind(0).await?, socket.clone(), token)?;
        let address = server.local_addr()?;
        let origin = format!("http://{address}");
        tokio::spawn(server.run());

        let anonymous = send(address, &get("/api/status", "other=1")).await?;
        assert_eq!(anonymous.status, 401);
        let wrong = send(address, &get("/?token=00", "")).await?;
        assert_eq!(wrong.status, 401);

        let login = send(address, &get(&format!("/?token={hex}"), "")).await?;
        assert_eq!(login.status, 303);
        let set_cookie = login
            .head
            .lines()
            .find_map(|line| line.strip_prefix("set-cookie: "))
            .ok_or("no cookie")?;
        assert!(set_cookie.contains("HttpOnly") && set_cookie.contains("SameSite=Strict"));
        let cookie = set_cookie.split(';').next().ok_or("empty cookie")?;

        let page = send(address, &get("/", cookie)).await?;
        assert_eq!(page.status, 200);
        assert!(page.body.contains("<html"));

        let status = send(address, &get("/api/status", cookie)).await?;
        assert_eq!(status.status, 200);
        let status: serde_json::Value = serde_json::from_str(&status.body)?;
        assert_eq!(status["node"], NODE.to_string());
        assert_eq!(status["networks"][0]["id"], NET.to_string());
        assert_eq!(status["admin"][NET.to_string()], true);
        assert_eq!(actions.recv().await, Some(Request::Status));

        let records = format!("/api/networks/{NET}/records");
        let record = r#"{"name":"nas","type":"a","value":"100.64.0.2"}"#;
        let forbidden = send(address, &post(&records, cookie, None, record)).await?;
        assert_eq!(forbidden.status, 403);
        let foreign = send(
            address,
            &post(&records, cookie, Some("http://evil.test"), record),
        )
        .await?;
        assert_eq!(foreign.status, 403);
        let set = send(address, &post(&records, cookie, Some(&origin), record)).await?;
        assert_eq!((set.status, set.body.as_str()), (200, "{}"));
        assert_eq!(
            actions.recv().await,
            Some(Request::SetRecord {
                network: NET,
                name: "nas".into(),
                record: DnsRecord::A([100, 64, 0, 2].into()),
            })
        );

        let mut events = TcpStream::connect(address).await?;
        events
            .write_all(
                format!("GET /api/events HTTP/1.1\r\nHost: x\r\nCookie: {cookie}\r\nLast-Event-ID: 6\r\n\r\n")
                    .as_bytes(),
            )
            .await?;
        let mut seen = String::new();
        let mut buffer = [0; 4096];
        tokio::time::timeout(Duration::from_secs(5), async {
            while !seen.contains("\n\n") || !seen.contains("data: ") {
                let read = events.read(&mut buffer).await?;
                if read == 0 {
                    break;
                }
                seen.push_str(&String::from_utf8_lossy(&buffer[..read]));
            }
            Ok::<_, std::io::Error>(())
        })
        .await??;
        assert!(seen.contains("text/event-stream"), "{seen}");
        assert!(seen.contains("id: 7\nevent: peer_offline\n"), "{seen}");
        let data = seen
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .ok_or("no data line")?;
        let entry: serde_json::Value = serde_json::from_str(data)?;
        assert_eq!(entry["kind"], "peer_offline");
        assert_eq!(entry["name"], "laptop");
        assert_eq!(entry["node"], NODE.to_string());

        drop(std::fs::remove_file(&socket));
        Ok(())
    }
}
