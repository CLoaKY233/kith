//! The API against a fake daemon on a real socket: what the page sends, what reaches the
//! daemon, and what the page gets back.

#![cfg(unix)]
#![expect(
    clippy::panic_in_result_fn,
    reason = "assertions fail the test; ? keeps the setup short"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request as Http, StatusCode, header};
use http_body_util::BodyExt as _;
use kith_identity::InviteToken;
use kith_ipc::{
    BlockedInfo, DnsRecord, Entry, EntryKind, ErrorCode, ImportedInfo, InviteInfo, IpcServer,
    LinkInfo, LinkReport, NetworkSummary, PathKind, PeerInfo, PendingJoin, PolicyInfo, RecordInfo,
    RelayInfo, Request as Ipc, Response as Reply, Status,
};
use kith_types::{NetworkId, NodeId, Timestamp};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tower::ServiceExt as _;

use crate::api::codes::{check_code, key_fpr};
use crate::osauth::{Gate, OsAuthError};
use crate::{Config, Source, Token, build};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const PORT: u16 = 41680;
const ORIGIN: &str = "http://127.0.0.1:41680";
const NET: NetworkId = NetworkId::from_bytes([0xab; 32]);
const ME: NodeId = NodeId::from_bytes([1; 32]);
const NAS: NodeId = NodeId::from_bytes([2; 32]);
const PHONE: NodeId = NodeId::from_bytes([3; 32]);
const PIXEL: NodeId = NodeId::from_bytes([4; 32]);
const TV: NodeId = NodeId::from_bytes([5; 32]);
const OLD: NodeId = NodeId::from_bytes([6; 32]);
const IMPORTED: NetworkId = NetworkId::from_bytes([0xef; 32]);

fn peer(node: NodeId, name: &str, address: &str, online: bool) -> PeerInfo {
    PeerInfo {
        node,
        name: name.to_owned(),
        address: address.to_owned(),
        path: online.then_some(PathKind::Direct),
        path_reason: None,
        last_seen_secs: Some(1),
        endpoint: None,
        rx_bytes: 0,
        tx_bytes: 0,
        online,
        display_name: name.to_owned(),
        gateway: (name == "phone").then(|| "laptop".to_owned()),
        path_mtu: None,
    }
}

fn record(name: &str, record: DnsRecord, device: bool) -> RecordInfo {
    RecordInfo {
        name: format!("{name}.home.kith.internal"),
        record,
        device,
    }
}

fn link(peer: NodeId, peer_name: &str, bytes_per_sec: u64) -> LinkInfo {
    LinkInfo {
        peer,
        peer_name: peer_name.to_owned(),
        path: PathKind::Direct,
        relay: None,
        rtt_ms: Some(12),
        bytes_per_sec,
    }
}

/// A daemon's state, from which the fake answers.
#[derive(Debug, Clone)]
struct World {
    admin: bool,
    pending: Vec<NodeId>,
    journal: Vec<Entry>,
    /// Invites the fake made, shared by every connection.
    invites: Arc<Mutex<Vec<InviteInfo>>>,
}

impl World {
    fn new(admin: bool) -> Self {
        Self {
            admin,
            pending: vec![PIXEL],
            journal: Vec::new(),
            invites: Arc::default(),
        }
    }

    fn invite(&self) -> Reply {
        let Ok(token) = InviteToken::generate(
            NET,
            ME,
            Vec::new(),
            Timestamp::from_millis(9_000_000),
            "home".into(),
        ) else {
            return Self::refuse(ErrorCode::Internal, "no randomness");
        };
        let Ok(link) = token.to_link() else {
            return Self::refuse(ErrorCode::Internal, "no link");
        };
        let id = crate::sim::hex(&token.invite_public_key().to_bytes());
        if let Ok(mut invites) = self.invites.lock() {
            invites.push(InviteInfo {
                id,
                expires_at_ms: 9_000_000,
                uses_left: Some(2),
                created_at_ms: 1_000,
            });
        }
        Reply::Invite {
            invite_link: link.to_string(),
        }
    }

    fn revoke(&self, id: &str) -> Reply {
        let Ok(mut invites) = self.invites.lock() else {
            return Self::refuse(ErrorCode::Internal, "poisoned");
        };
        let before = invites.len();
        invites.retain(|invite| invite.id != id);
        if invites.len() == before {
            Self::refuse(ErrorCode::NotFound, "no such invite")
        } else {
            Reply::InviteRevoked
        }
    }

    fn status(&self) -> Status {
        Status {
            node: Some(ME),
            networks: vec![NetworkSummary {
                id: NET,
                name: "home".into(),
                peers_online: 2,
                peers_total: 3,
                relays: 0,
                problems: Vec::new(),
                admin: self.admin,
            }],
            exit: false,
            relay: false,
            exit_node: Some("home/nas".into()),
            listen_port: 41641,
            dns_upstreams: Vec::new(),
            drops: BTreeMap::new(),
            route_conflicts: Vec::new(),
            warnings: Vec::new(),
            imported: vec![ImportedInfo {
                network: IMPORTED,
                name: "office".into(),
                endpoint: Some("198.51.100.1:51820".into()),
                routes: vec!["0.0.0.0/0".into(), "::/0".into()],
                everything: true,
                active: true,
                last_seen_secs: Some(4),
                rx_bytes: 0,
                tx_bytes: 0,
            }],
        }
    }

    fn peers() -> Vec<PeerInfo> {
        vec![
            peer(NAS, "nas", "100.64.0.2", true),
            peer(PHONE, "phone", "100.64.0.7", true),
            peer(TV, "tv", "100.64.0.9", false),
        ]
    }

    fn records() -> Vec<RecordInfo> {
        vec![
            record("laptop", DnsRecord::A([100, 64, 0, 3].into()), true),
            record("phone", DnsRecord::A([100, 64, 0, 7].into()), true),
            record("files", DnsRecord::A([100, 64, 0, 7].into()), false),
            record(
                "pics",
                DnsRecord::Cname("phone.home.kith.internal.".into()),
                false,
            ),
            record("storage", DnsRecord::A([100, 64, 0, 2].into()), false),
        ]
    }

    fn topology() -> Vec<LinkReport> {
        vec![
            LinkReport {
                node: ME,
                name: "laptop".into(),
                age_secs: 0,
                links: vec![link(NAS, "nas", 5_000)],
            },
            LinkReport {
                node: NAS,
                name: "nas".into(),
                age_secs: 4,
                links: vec![link(PHONE, "phone", 2_000_000)],
            },
        ]
    }

    fn refuse(code: ErrorCode, message: &str) -> Reply {
        Reply::Error {
            code,
            message: message.into(),
        }
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        clippy::too_many_lines,
        reason = "the fake answers only what the tests send, one arm each"
    )]
    fn answer(&self, request: &Ipc) -> Reply {
        let admin_only = || Self::refuse(ErrorCode::InvalidArgument, "only an admin can do this");
        match request {
            Ipc::Status => Reply::Status(self.status()),
            Ipc::Peers { .. } => Reply::Peers(Self::peers()),
            Ipc::Records { .. } => Reply::Records(Self::records()),
            Ipc::Topology { .. } => Reply::Topology(Self::topology()),
            Ipc::ListExits { .. } => Reply::Exits(vec![NAS]),
            Ipc::Traffic => Reply::Traffic(Vec::new()),
            Ipc::PendingJoins => Reply::PendingJoins(
                self.pending
                    .iter()
                    .map(|&node| PendingJoin {
                        network: NET,
                        node,
                        name: "pixel-8".into(),
                        from: Some("203.0.113.7".into()),
                        age_secs: 30,
                    })
                    .collect(),
            ),
            Ipc::Blocked { .. } => Reply::Blocked(vec![BlockedInfo {
                network: NET,
                from: PHONE,
                from_name: "phone".into(),
                to: NAS,
                to_name: "nas".into(),
                protocol: "tcp".into(),
                port: Some(22),
                count: 4,
                last_ms: 1,
                rule: String::new(),
            }]),
            Ipc::ApproveJoin { node, .. } | Ipc::DenyJoin { node, .. } => {
                if !self.admin {
                    admin_only()
                } else if !self.pending.contains(node) {
                    Self::refuse(ErrorCode::NotFound, "no pending request")
                } else if matches!(request, Ipc::ApproveJoin { .. }) {
                    Reply::JoinApproved {
                        device: "pixel-8".into(),
                    }
                } else {
                    Reply::JoinRequestDenied
                }
            }
            Ipc::RemoveDevice { node, .. } => {
                if Self::peers().iter().any(|peer| peer.node == *node) {
                    Reply::DeviceRemoved
                } else {
                    Self::refuse(ErrorCode::NotFound, "not in this network")
                }
            }
            Ipc::SetRecord { .. } if self.admin => Reply::RecordSet,
            Ipc::SetRecord { .. } => admin_only(),
            Ipc::DeleteRecord { .. } => Reply::RecordDeleted,
            Ipc::ExportWireGuard { device_name, .. } => Reply::WireGuardConfig {
                device: device_name.clone(),
                endpoint: "192.168.1.20:41641".into(),
                mesh: "[Interface]\nPrivateKey = mesh-key\n".into(),
                everything: None,
            },
            Ipc::AllowBlocked { .. } => Reply::BlockedAllowed,
            Ipc::CreateInvite { .. } | Ipc::RevokeInvite { .. } | Ipc::RenameDevice { .. }
                if !self.admin =>
            {
                admin_only()
            }
            Ipc::CreateInvite { .. } => self.invite(),
            Ipc::ListInvites { .. } => {
                Reply::Invites(self.invites.lock().map(|i| i.clone()).unwrap_or_default())
            }
            Ipc::RevokeInvite { invite, .. } => self.revoke(invite),
            Ipc::RenameDevice { node, .. } => {
                if Self::peers().iter().any(|peer| peer.node == *node) || *node == PIXEL {
                    Reply::DeviceRenamed
                } else {
                    Self::refuse(ErrorCode::NotFound, "not in this network")
                }
            }
            Ipc::GetPolicy { .. } => Reply::Policy(PolicyInfo {
                toml: "[[rules]]\nsources = [\"Any\"]\n".into(),
                rules: Vec::new(),
            }),
            Ipc::SetPolicy { policy_toml, .. } if policy_toml.contains('=') => Reply::PolicySet,
            Ipc::SetPolicy { .. } => Self::refuse(
                ErrorCode::InvalidArgument,
                "policy: TOML parse error at line 1, column 8",
            ),
            Ipc::ListRelays { .. } => Reply::Relays(vec![RelayInfo {
                name: "r".into(),
                address: "relay.example:443".into(),
                node: NAS,
            }]),
            Ipc::AdvertiseRelay { address, .. } => Reply::RelayAdvertised {
                address: address.clone(),
            },
            Ipc::RemoveRelay { name, .. } if name == "r" => Reply::RelayRemoved,
            Ipc::RemoveRelay { .. } => Self::refuse(ErrorCode::NotFound, "relay gone"),
            Ipc::ServerMode { .. } => Reply::ServerModeSet { notes: Vec::new() },
            Ipc::ImportWireGuard { .. } => Reply::Imported { network: IMPORTED },
            Ipc::UseImport { network, .. } if *network == IMPORTED => Reply::ImportUsed,
            Ipc::RemoveImport { network } if *network == IMPORTED => Reply::ImportRemoved,
            Ipc::UseImport { .. } | Ipc::RemoveImport { .. } => {
                Self::refuse(ErrorCode::NotFound, "imported network")
            }
            Ipc::Leave { .. } => Reply::Left,
            Ipc::RequestCode { .. } => Reply::RequestCode {
                code: "kith://request/abc".into(),
            },
            _ => Self::refuse(ErrorCode::Internal, "not faked"),
        }
    }
}

fn socket_path() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("kith-ui-api-{}-{n}.sock", std::process::id()))
}

/// Serves `world` on a fresh socket and reports every request it receives.
fn serve(world: World) -> TestResult<(PathBuf, mpsc::UnboundedReceiver<Ipc>)> {
    let socket = socket_path();
    let server = IpcServer::bind(&socket)?;
    let (seen, received) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok(mut connection) = server.accept().await {
            let (world, seen) = (world.clone(), seen.clone());
            tokio::spawn(async move {
                while let Ok(Some(request)) = connection.next_request().await {
                    if let Ipc::Journal { .. } = request {
                        let next = u64::try_from(world.journal.len()).unwrap_or_default();
                        if connection
                            .respond(Reply::Journaling { next })
                            .await
                            .is_err()
                        {
                            return;
                        }
                        for entry in world.journal {
                            if connection.send_entry(entry).await.is_err() {
                                return;
                            }
                        }
                        return;
                    }
                    let reply = world.answer(&request);
                    if seen.send(request).is_err() || connection.respond(reply).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    Ok((socket, received))
}

/// The router on `world`, signed in, and the requests that reached the daemon.
struct Page {
    app: Router,
    cookie: String,
    received: mpsc::UnboundedReceiver<Ipc>,
}

impl Page {
    async fn open(world: World, gate: Gate) -> TestResult<Self> {
        let (socket, received) = serve(world)?;
        Self::start(Source::Daemon(socket), gate, received).await
    }

    /// The router on the recording the page is built against.
    async fn recorded() -> TestResult<Self> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../kith-ui/web/fixtures/office.json");
        let source = Source::Fixture(Box::new(crate::load_fixture(&path)?));
        Self::start(source, Gate::System, mpsc::unbounded_channel().1).await
    }

    async fn start(
        source: Source,
        gate: Gate,
        received: mpsc::UnboundedReceiver<Ipc>,
    ) -> TestResult<Self> {
        let token = Token::random()?;
        let hex = token.hex();
        let app = build(
            Config {
                source,
                port: PORT,
                token,
                assets: None,
                dev_origins: Vec::new(),
            },
            gate,
        )?;
        let mut page = Self {
            app,
            cookie: String::new(),
            received,
        };
        let started = page
            .app
            .clone()
            .oneshot(page.request("POST", "/api/session", Some(json!({ "token": hex })))?)
            .await?;
        page.cookie = started
            .headers()
            .get(header::SET_COOKIE)
            .ok_or("no cookie")?
            .to_str()?
            .split(';')
            .next()
            .unwrap_or_default()
            .to_owned();
        Ok(page)
    }

    fn request(&self, method: &str, path: &str, body: Option<Value>) -> TestResult<Http<Body>> {
        Ok(Http::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "127.0.0.1:41680")
            .header(header::ORIGIN, ORIGIN)
            .header(header::COOKIE, &self.cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))?)
    }

    async fn call(
        &mut self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> TestResult<(StatusCode, Value)> {
        let response = self
            .app
            .clone()
            .oneshot(self.request(method, path, body)?)
            .await?;
        let status = response.status();
        let bytes = response.into_body().collect().await?.to_bytes();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)?
        };
        Ok((status, value))
    }

    async fn get(&mut self, path: &str) -> TestResult<(StatusCode, Value)> {
        self.call("GET", path, None).await
    }

    async fn post(&mut self, path: &str, body: Value) -> TestResult<(StatusCode, Value)> {
        self.call("POST", path, Some(body)).await
    }

    /// The changes that reached the daemon since the last look; reads are left out.
    fn changes(&mut self) -> Vec<Ipc> {
        let mut changes = Vec::new();
        while let Ok(request) = self.received.try_recv() {
            let read = matches!(
                request,
                Ipc::Status
                    | Ipc::Peers { .. }
                    | Ipc::Records { .. }
                    | Ipc::Topology { .. }
                    | Ipc::Traffic
                    | Ipc::PendingJoins
                    | Ipc::Blocked { .. }
                    | Ipc::ListInvites { .. }
                    | Ipc::GetPolicy { .. }
                    | Ipc::ListRelays { .. }
                    | Ipc::ListExits { .. }
            );
            if !read {
                changes.push(request);
            }
        }
        changes
    }
}

fn net(rest: &str) -> String {
    format!("/api/networks/{NET}{rest}")
}

#[tokio::test(flavor = "current_thread")]
async fn the_snapshot_describes_this_device_and_hides_other_devices_rates_from_members()
-> TestResult {
    let mut page = Page::open(World::new(false), Gate::System).await?;
    let (code, status) = page.get("/api/status").await?;
    assert_eq!(code, StatusCode::OK);
    let membership = &status["memberships"][0];
    assert_eq!(membership["name"], "laptop");
    assert_eq!(membership["address"], "100.64.0.3");
    assert_eq!(membership["role"], "member");
    assert_eq!(membership["key_fpr"], key_fpr(&ME));
    assert_eq!(status["exit_state"], "active");

    let (_, boot) = page.get("/api/boot").await?;
    let links = boot["last_tick"]["links"].as_array().ok_or("no links")?;
    let own = links
        .iter()
        .find(|l| l["from"] == ME.to_string())
        .ok_or("no own link")?;
    assert_eq!(
        (own["rate"].clone(), own["rate_band"].clone()),
        (json!(5000), Value::Null)
    );
    let others = links
        .iter()
        .find(|l| l["from"] == NAS.to_string())
        .ok_or("no nas link")?;
    assert_eq!(
        (others["rate"].clone(), others["rate_band"].clone()),
        (Value::Null, json!("busy"))
    );
    let join = &boot["pending"]["joins"][0];
    assert_eq!(join["check_code"], check_code(&PIXEL));
    assert_eq!(join["seen"]["key_fpr"], key_fpr(&PIXEL));
    assert_eq!(boot["pending"]["blocked"][0]["from_name"], "phone");

    let (_, request) = page.get(&net("/request-code")).await?;
    assert_eq!(request["command"], "kith request kith://request/abc");
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn a_join_request_settled_elsewhere_answers_with_who_won() -> TestResult {
    let mut world = World::new(true);
    world.journal = vec![
        Entry {
            seq: 0,
            at_ms: 1_000,
            kind: EntryKind::JoinRequested {
                network: NET,
                node: OLD,
                name: "old-box".into(),
            },
        },
        Entry {
            seq: 1,
            at_ms: 2_000,
            kind: EntryKind::DeviceJoined {
                network: NET,
                node: NAS,
                name: "nas".into(),
                address: "100.64.0.2".into(),
            },
        },
    ];
    let mut page = Page::open(world, Gate::System).await?;

    let (code, body) = page
        .post(
            &net(&format!("/joins/{PIXEL}/approve")),
            json!({ "name": "pixel-8" }),
        )
        .await?;
    assert_eq!(
        (code, body["state"].clone()),
        (StatusCode::OK, json!("accepted"))
    );
    assert_eq!(
        page.changes(),
        [Ipc::ApproveJoin {
            network: NET,
            node: PIXEL
        }]
    );

    let (code, body) = page
        .post(&net(&format!("/joins/{NAS}/deny")), json!({}))
        .await?;
    assert_eq!(code, StatusCode::CONFLICT);
    assert_eq!(body["conflict"]["resolution"], "approved");
    assert_eq!(body["conflict"]["at_ms"], 2_000);

    let (code, body) = page
        .post(&net(&format!("/joins/{OLD}/approve")), json!({}))
        .await?;
    assert_eq!(code, StatusCode::CONFLICT);
    assert_eq!(body["conflict"]["resolution"], "expired");

    let mut member = Page::open(World::new(false), Gate::System).await?;
    let (code, body) = member
        .post(&net(&format!("/joins/{PIXEL}/approve")), json!({}))
        .await?;
    assert_eq!(
        (code, body["code"].clone()),
        (StatusCode::FORBIDDEN, json!("permission_denied"))
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn a_name_changes_only_from_what_the_page_saw() -> TestResult {
    let mut page = Page::open(World::new(true), Gate::System).await?;
    let edit = |based_on: &str| json!({ "name": "storage", "type": "A", "value": "100.64.0.9", "based_on": based_on });

    let (code, body) = page.post(&net("/records"), edit("100.64.0.5")).await?;
    assert_eq!(code, StatusCode::CONFLICT);
    assert_eq!(body["conflict"]["op"], "100.64.0.2");
    assert_eq!(page.changes(), []);

    let (code, _) = page.post(&net("/records"), edit("100.64.0.2")).await?;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(
        page.changes(),
        [Ipc::SetRecord {
            network: NET,
            name: "storage".into(),
            record: DnsRecord::A([100, 64, 0, 9].into()),
        }]
    );

    let (code, _) = page
        .post(
            &net("/records"),
            json!({ "name": " Storage.home.kith.internal. ", "type": "A", "value": "100.64.0.9", "based_on": "100.64.0.2" }),
        )
        .await?;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = page
        .post(
            &net("/records"),
            json!({ "name": "web.home.kith.internal", "type": "TXT", "value": "hi", "based_on": null }),
        )
        .await?;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(
        page.changes(),
        [
            Ipc::SetRecord {
                network: NET,
                name: "storage".into(),
                record: DnsRecord::A([100, 64, 0, 9].into()),
            },
            Ipc::SetRecord {
                network: NET,
                name: "web".into(),
                record: DnsRecord::Txt("hi".into()),
            },
        ]
    );

    let (code, _) = page
        .post(
            &net("/records/STORAGE.home.kith.internal/delete"),
            json!({}),
        )
        .await?;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(
        page.changes(),
        [Ipc::DeleteRecord {
            network: NET,
            name: "storage".into()
        }]
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn removing_a_lost_device_drops_its_names_and_a_lost_race_is_a_conflict() -> TestResult {
    let mut world = World::new(true);
    world.journal = vec![Entry {
        seq: 0,
        at_ms: 3_000,
        kind: EntryKind::DeviceRemoved {
            network: NET,
            node: PIXEL,
            name: "pixel-8".into(),
        },
    }];
    let mut page = Page::open(world, Gate::System).await?;

    let (code, _) = page
        .post(
            &net(&format!("/devices/{PHONE}/remove")),
            json!({ "lost": true }),
        )
        .await?;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(
        page.changes(),
        [
            Ipc::RemoveDevice {
                network: NET,
                node: PHONE
            },
            Ipc::DeleteRecord {
                network: NET,
                name: "files".into()
            },
            Ipc::DeleteRecord {
                network: NET,
                name: "pics".into()
            },
        ]
    );

    let (code, body) = page
        .post(
            &net(&format!("/devices/{PIXEL}/remove")),
            json!({ "lost": false }),
        )
        .await?;
    assert_eq!(code, StatusCode::CONFLICT);
    assert_eq!(body["conflict"]["at_ms"], 3_000);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn a_device_is_renamed_only_from_the_name_the_page_saw() -> TestResult {
    let mut page = Page::open(World::new(true), Gate::System).await?;
    let rename = net(&format!("/devices/{TV}/rename"));
    let (code, body) = page
        .post(
            &rename,
            json!({ "name": "telly", "based_on": "television" }),
        )
        .await?;
    assert_eq!(code, StatusCode::CONFLICT);
    assert_eq!(body["conflict"]["op"], "tv");
    assert_eq!(page.changes(), []);

    let (code, _) = page
        .post(&rename, json!({ "name": "telly", "based_on": "tv" }))
        .await?;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(
        page.changes(),
        [Ipc::RenameDevice {
            network: NET,
            node: TV,
            name: "telly".into(),
        }]
    );

    let (code, _) = page
        .post(
            &net(&format!("/joins/{PIXEL}/approve")),
            json!({ "name": "lounge" }),
        )
        .await?;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(
        page.changes(),
        [
            Ipc::ApproveJoin {
                network: NET,
                node: PIXEL
            },
            Ipc::RenameDevice {
                network: NET,
                node: PIXEL,
                name: "lounge".into(),
            },
        ]
    );

    let mut member = Page::open(World::new(false), Gate::System).await?;
    let (code, body) = member
        .post(&rename, json!({ "name": "telly", "based_on": null }))
        .await?;
    assert_eq!(
        (code, body["code"].clone()),
        (StatusCode::FORBIDDEN, json!("permission_denied"))
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn tier_three_changes_wait_for_the_os_and_nothing_happens_without_it() -> TestResult {
    let mut refused = Page::open(World::new(true), Gate::Fixed(Err(OsAuthError::Denied))).await?;
    let (code, phone) = refused
        .post(
            &net("/phones"),
            json!({ "name": "phone", "tags": ["phone"] }),
        )
        .await?;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(
        (phone["node"].clone(), phone["gateway"].clone()),
        (json!(PHONE.to_string()), json!("laptop"))
    );
    let config = net(&format!("/phones/{PHONE}/config"));
    let (code, body) = refused
        .post(&config, json!({ "everything": false }))
        .await?;
    assert_eq!(
        (code, body["code"].clone()),
        (StatusCode::FORBIDDEN, json!("os_auth_denied"))
    );
    assert!(!body.to_string().contains("mesh-key"));
    let (code, _) = refused.post(&net("/leave"), json!({})).await?;
    assert_eq!(code, StatusCode::FORBIDDEN);
    assert!(
        !refused
            .changes()
            .iter()
            .any(|change| matches!(change, Ipc::Leave { .. }))
    );

    let mut confirmed = Page::open(World::new(true), Gate::Fixed(Ok(()))).await?;
    confirmed
        .post(&net("/phones"), json!({ "name": "phone", "tags": [] }))
        .await?;
    let (code, body) = confirmed
        .post(&config, json!({ "everything": false }))
        .await?;
    assert_eq!(code, StatusCode::OK);
    assert!(
        body["config"]
            .as_str()
            .is_some_and(|text| text.contains("mesh-key"))
    );
    assert!(
        body["qr_svg"]
            .as_str()
            .is_some_and(|svg| svg.starts_with("<?xml") || svg.contains("<svg"))
    );
    let (code, _) = confirmed
        .post(&config, json!({ "everything": true }))
        .await?;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    let (code, _) = confirmed.post(&net("/leave"), json!({})).await?;
    assert_eq!(code, StatusCode::OK);
    assert!(confirmed.changes().contains(&Ipc::Leave { network: NET }));
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn removing_this_device_is_leaving_and_forgets_what_was_held() -> TestResult {
    let remove_me = net(&format!("/devices/{ME}/remove"));
    let mut refused = Page::open(World::new(true), Gate::Fixed(Err(OsAuthError::Denied))).await?;
    let (code, _) = refused.post(&remove_me, json!({ "lost": false })).await?;
    assert_eq!(code, StatusCode::FORBIDDEN);
    assert_eq!(refused.changes(), []);

    let mut page = Page::open(World::new(true), Gate::Fixed(Ok(()))).await?;
    page.post(&net("/phones"), json!({ "name": "phone", "tags": [] }))
        .await?;
    let (code, _) = page
        .post(
            &net(&format!("/devices/{PHONE}/remove")),
            json!({ "lost": false }),
        )
        .await?;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = page
        .post(
            &net(&format!("/phones/{PHONE}/config")),
            json!({ "everything": false }),
        )
        .await?;
    assert_eq!(code, StatusCode::NOT_FOUND);

    page.changes();
    let (code, _) = page.post(&remove_me, json!({ "lost": true })).await?;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(page.changes(), [Ipc::Leave { network: NET }]);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn invites_are_bounded_before_anything_is_signed() -> TestResult {
    let mut page = Page::open(World::new(true), Gate::System).await?;
    for bad in [
        json!({ "expires_secs": 0, "uses": null }),
        json!({ "expires_secs": u64::MAX, "uses": null }),
        json!({ "expires_secs": 600, "uses": 0 }),
        json!({ "expires_secs": 600, "uses": 70_000 }),
    ] {
        let (code, _) = page.post(&net("/invites"), bad).await?;
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }
    assert_eq!(page.changes(), []);

    let (code, invite) = page
        .post(&net("/invites"), json!({ "expires_secs": 600, "uses": 2 }))
        .await?;
    assert_eq!(code, StatusCode::OK);
    assert!(invite["qr_svg"].is_string());
    let (_, listed) = page
        .get(&format!(
            "/api/networks/{}/invites",
            NET.to_string().to_uppercase()
        ))
        .await?;
    assert_eq!(
        listed,
        json!([{ "id": invite["id"], "expires_at_ms": 9_000_000, "uses_left": 2, "created_at_ms": 1_000 }])
    );
    let (code, _) = page.get("/api/networks/home/invites").await?;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn a_revoked_invite_leaves_the_list() -> TestResult {
    let mut page = Page::open(World::new(true), Gate::System).await?;
    let (_, invite) = page
        .post(&net("/invites"), json!({ "expires_secs": 600, "uses": 2 }))
        .await?;
    let id = invite["id"].as_str().ok_or("no id")?.to_owned();
    page.changes();
    let revoke = net(&format!("/invites/{id}/revoke"));
    let (code, _) = page.post(&revoke, json!({})).await?;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(
        page.changes(),
        [Ipc::RevokeInvite {
            network: NET,
            invite: id
        }]
    );
    let (_, listed) = page.get(&net("/invites")).await?;
    assert_eq!(listed, json!([]));
    let (code, _) = page.post(&revoke, json!({})).await?;
    assert_eq!(code, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn the_policy_reads_and_a_bad_one_says_where_it_broke() -> TestResult {
    let mut page = Page::open(World::new(true), Gate::System).await?;
    let (code, policy) = page.get(&net("/policy")).await?;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(policy["toml"], "[[rules]]\nsources = [\"Any\"]\n");
    assert_eq!(policy["rules"], json!([]));

    let (code, body) = page
        .post(&net("/policy"), json!({ "toml": "[[rules" }))
        .await?;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|m| m.contains("line 1, column 8"))
    );
    let good = "[[rules]]\nsources = [\"Any\"]";
    let (code, _) = page.post(&net("/policy"), json!({ "toml": good })).await?;
    assert_eq!(code, StatusCode::OK);
    let set: Vec<Ipc> = page
        .changes()
        .into_iter()
        .filter(|change| matches!(change, Ipc::SetPolicy { .. }))
        .collect();
    assert_eq!(
        set.last(),
        Some(&Ipc::SetPolicy {
            network: NET,
            policy_toml: good.into()
        })
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn relays_are_listed_advertised_and_removed() -> TestResult {
    let mut page = Page::open(World::new(true), Gate::System).await?;
    let (code, relays) = page.get(&net("/relays")).await?;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(
        relays,
        json!([{ "name": "r", "address": "relay.example:443", "node": NAS.to_string() }])
    );
    let (code, _) = page
        .post(
            &net("/relays"),
            json!({ "name": "edge", "address": "edge.example:443", "node": NAS.to_string() }),
        )
        .await?;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = page.post(&net("/relays/r/remove"), json!({})).await?;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = page.post(&net("/relays/gone/remove"), json!({})).await?;
    assert_eq!(code, StatusCode::NOT_FOUND);
    assert_eq!(
        page.changes(),
        [
            Ipc::AdvertiseRelay {
                network: NET,
                name: "edge".into(),
                address: "edge.example:443".into(),
                node: Some(NAS),
            },
            Ipc::RemoveRelay {
                network: NET,
                name: "r".into(),
            },
            Ipc::RemoveRelay {
                network: NET,
                name: "gone".into(),
            },
        ]
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn server_mode_and_imports_wait_for_the_os() -> TestResult {
    let mut refused = Page::open(World::new(true), Gate::Fixed(Err(OsAuthError::Denied))).await?;
    let (code, server) = refused.get("/api/server").await?;
    assert_eq!(
        (code, server),
        (StatusCode::OK, json!({ "exit": false, "relay": false }))
    );
    let on = json!({ "exit": true, "relay": true });
    let (code, _) = refused.post("/api/server", on.clone()).await?;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let import = json!({ "name": "office", "conf_text": "[Interface]" });
    let (code, _) = refused
        .post("/api/wireguard/import", import.clone())
        .await?;
    assert_eq!(code, StatusCode::FORBIDDEN);
    assert_eq!(refused.changes(), []);

    let mut page = Page::open(World::new(true), Gate::Fixed(Ok(()))).await?;
    let (code, _) = page.post("/api/server", on).await?;
    assert_eq!(code, StatusCode::OK);
    let (code, imported) = page.post("/api/wireguard/import", import).await?;
    assert_eq!(
        (code, imported),
        (StatusCode::OK, json!({ "network": IMPORTED.to_string() }))
    );
    assert_eq!(
        page.changes(),
        [
            Ipc::ServerMode {
                exit: true,
                relay: true
            },
            Ipc::ImportWireGuard {
                name: "office".into(),
                conf_text: "[Interface]".into(),
            },
        ]
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn a_blocked_connection_previews_then_signs_its_one_rule() -> TestResult {
    let mut page = Page::open(World::new(true), Gate::System).await?;
    let id = format!("blocked:{NET}:{PHONE}:{NAS}:tcp:22");
    let allow = format!("/api/blocked/{id}/allow");

    let (code, preview) = page.post(&allow, json!({ "preview": true })).await?;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(preview["rule_sentence"], "phone may reach nas on TCP 22");
    assert_eq!(preview["new_paths"].as_array().map(Vec::len), Some(1));
    assert_eq!(page.changes(), []);

    let (code, _) = page.post(&allow, json!({ "preview": false })).await?;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(
        page.changes(),
        [Ipc::AllowBlocked {
            network: NET,
            from: PHONE,
            to: NAS,
            protocol: "tcp".into(),
            port: Some(22),
        }]
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn taken_names_come_with_a_free_suggestion() -> TestResult {
    let mut page = Page::open(World::new(true), Gate::System).await?;
    let check = async |page: &mut Page, net: &str, name: &str| -> TestResult<Value> {
        Ok(page
            .get(&format!("/api/validate/name?net={net}&name={name}"))
            .await?
            .1)
    };
    let nas = check(&mut page, &NET.to_string(), "NAS").await?;
    assert_eq!(
        nas,
        json!({ "name": "nas", "problem": "taken", "suggestion": "nas-2" })
    );
    let me = check(&mut page, "home", "laptop").await?;
    assert_eq!(me["problem"], "taken");
    let fresh = check(&mut page, "home", "dev%20box").await?;
    assert_eq!(
        fresh,
        json!({ "name": "dev-box", "problem": null, "suggestion": null })
    );
    let network = check(&mut page, "", "home").await?;
    assert_eq!(network["problem"], "taken");
    let bad = check(&mut page, "", "-x").await?;
    assert_eq!(bad["problem"], "hyphen_at_edge");
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn reads_refuse_bad_ids_and_unknown_networks_alike_live_and_recorded() -> TestResult {
    let unknown = NetworkId::from_bytes([0xcd; 32]);
    let mut live = Page::open(World::new(true), Gate::System).await?;
    let mut recorded = Page::recorded().await?;
    for page in [&mut live, &mut recorded] {
        for list in [
            "peers",
            "records",
            "exits",
            "invites",
            "request-code",
            "policy",
            "relays",
        ] {
            let (code, body) = page.get(&format!("/api/networks/not-an-id/{list}")).await?;
            assert_eq!(code, StatusCode::BAD_REQUEST, "{list}");
            assert_eq!(body["code"], "invalid_argument", "{list}");
        }
        let (code, body) = page
            .get(&format!("/api/validate/name?net={unknown}&name=box"))
            .await?;
        assert_eq!(
            (code, &body["code"]),
            (StatusCode::NOT_FOUND, &json!("not_found"))
        );
        let (code, body) = page.get("/api/validate/name?net=").await?;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_argument");
    }
    for list in [
        "peers",
        "records",
        "exits",
        "invites",
        "request-code",
        "policy",
        "relays",
    ] {
        let (code, body) = recorded
            .get(&format!("/api/networks/{unknown}/{list}"))
            .await?;
        assert_eq!(code, StatusCode::NOT_FOUND, "{list}");
        assert_eq!(body["code"], "not_found", "{list}");
    }
    let (code, invites) = live.get(&net("/invites")).await?;
    assert_eq!((code, invites), (StatusCode::OK, json!([])));
    let (_, check) = live
        .get("/api/validate/name?net=home&name=Na%20--%20S")
        .await?;
    assert_eq!(check["name"], "na-s");
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn an_imported_network_is_listed_used_and_removed() -> TestResult {
    let mut refused = Page::open(World::new(true), Gate::Fixed(Err(OsAuthError::Denied))).await?;
    let (code, status) = refused.get("/api/status").await?;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(status["imported"][0]["network"], IMPORTED.to_string());
    assert_eq!(status["imported"][0]["everything"], true);
    let remove = format!("/api/wireguard/{IMPORTED}/remove");
    let (code, _) = refused.post(&remove, json!({})).await?;
    assert_eq!(code, StatusCode::FORBIDDEN);

    let mut page = Page::open(World::new(true), Gate::Fixed(Ok(()))).await?;
    let (code, _) = page
        .post(
            &format!("/api/wireguard/{IMPORTED}/use"),
            json!({ "active": false }),
        )
        .await?;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = page.post(&remove, json!({})).await?;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = page
        .post(&format!("/api/wireguard/{NET}/remove"), json!({}))
        .await?;
    assert_eq!(code, StatusCode::NOT_FOUND);
    assert_eq!(
        page.changes(),
        [
            Ipc::UseImport {
                network: IMPORTED,
                active: false
            },
            Ipc::RemoveImport { network: IMPORTED },
        ]
    );
    assert_eq!(refused.changes(), []);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn the_simulator_lists_an_import_until_it_is_removed() -> TestResult {
    let mut page = Page::recorded().await?;
    let conf = "[Interface]\nPrivateKey = yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=\n\
                Address = 10.64.0.2/32\n\n[Peer]\n\
                PublicKey = xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=\n\
                Endpoint = 198.51.100.1:51820\nAllowedIPs = 0.0.0.0/0, ::/0\n";
    let (code, imported) = page
        .post(
            "/api/wireguard/import",
            json!({ "name": "mullvad", "conf_text": conf }),
        )
        .await?;
    assert_eq!(code, StatusCode::OK);
    let network = imported["network"].as_str().ok_or("no network")?.to_owned();
    let (_, status) = page.get("/api/status").await?;
    let listed = status["imported"]
        .as_array()
        .and_then(|all| all.iter().find(|one| one["network"] == network.as_str()))
        .ok_or("the import is not listed")?;
    assert_eq!(listed["name"], "mullvad");
    assert_eq!(listed["active"], true);
    assert_eq!(listed["everything"], true);
    assert_eq!(listed["endpoint"], "198.51.100.1:51820");
    assert_eq!(status["exit_node"], Value::Null);

    let (code, _) = page
        .post(
            &format!("/api/wireguard/{network}/use"),
            json!({ "active": false }),
        )
        .await?;
    assert_eq!(code, StatusCode::OK);
    let (_, status) = page.get("/api/status").await?;
    let listed = status["imported"]
        .as_array()
        .and_then(|all| all.iter().find(|one| one["network"] == network.as_str()))
        .ok_or("the import is not listed")?;
    assert_eq!(listed["active"], false);
    let (code, _) = page
        .post(&format!("/api/wireguard/{network}/remove"), json!({}))
        .await?;
    assert_eq!(code, StatusCode::OK);
    let (_, status) = page.get("/api/status").await?;
    let names: Vec<&Value> = status["imported"]
        .as_array()
        .ok_or("no imports")?
        .iter()
        .map(|one| &one["name"])
        .collect();
    assert_eq!(names, [&json!("work-vpn")]);
    Ok(())
}
