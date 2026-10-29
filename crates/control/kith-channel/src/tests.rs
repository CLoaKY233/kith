use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kith_crypto::SigningKey;
use kith_ledger::{JoinProof, Ledger, NetworkParams, OpBody};
use kith_sync::SyncSession;
use kith_types::{NetworkId, NodeId, OpId, Timestamp};
use proptest::prelude::*;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::time::{sleep, timeout};
use tokio_rustls::TlsAcceptor;

use crate::channel::ALPN;
use crate::message::{FrameReader, Message};
use crate::{Channel, ChannelClient, ChannelError, ChannelHandler, ChannelServer, Presence};

const NOW: u64 = 1_000_000;
const HOUR: u64 = 3_600_000;

type Gossip = (NetworkId, NodeId, Vec<(NodeId, Vec<SocketAddr>)>);
type Seen = (NetworkId, NodeId, Vec<(NodeId, Presence)>);

fn key(seed: u8) -> SigningKey {
    SigningKey::from_seed(&[seed; 32])
}

struct Node {
    key: SigningKey,
    ledgers: Mutex<BTreeMap<NetworkId, Ledger>>,
    gossip: Mutex<Vec<Gossip>>,
    presence: Mutex<Vec<Seen>>,
    joins: AtomicUsize,
}

impl Node {
    fn new(seed: u8, ledgers: impl IntoIterator<Item = Ledger>) -> Arc<Self> {
        Arc::new(Self {
            key: key(seed),
            ledgers: Mutex::new(
                ledgers
                    .into_iter()
                    .map(|ledger| (ledger.network_id(), ledger))
                    .collect(),
            ),
            gossip: Mutex::new(Vec::new()),
            presence: Mutex::new(Vec::new()),
            joins: AtomicUsize::new(0),
        })
    }

    fn id(&self) -> NodeId {
        self.key.node_id()
    }

    fn op_ids(&self, network: NetworkId) -> Vec<OpId> {
        self.with_ledger(network, |ledger| ledger.op_ids().copied().collect())
            .unwrap()
    }

    fn act(&self, network: NetworkId, body: OpBody) {
        self.with_ledger(network, |ledger| act(ledger, &self.key, body))
            .unwrap();
    }

    fn gossip(&self) -> Vec<Gossip> {
        self.gossip.lock().unwrap().clone()
    }
}

impl ChannelHandler for Node {
    fn networks(&self) -> Vec<NetworkId> {
        self.ledgers.lock().unwrap().keys().copied().collect()
    }

    fn with_ledger<R>(&self, network: NetworkId, f: impl FnOnce(&mut Ledger) -> R) -> Option<R> {
        self.ledgers.lock().unwrap().get_mut(&network).map(f)
    }

    fn handle_join(&self, network: NetworkId, proof: JoinProof) -> Result<(), String> {
        self.joins.fetch_add(1, Ordering::Relaxed);
        let mut ledgers = self.ledgers.lock().map_err(|_| "poisoned")?;
        let ledger = ledgers.get_mut(&network).ok_or("unknown network")?;
        let invite = ledger
            .view()
            .invites()
            .get(&proof.invite_key.to_bytes())
            .ok_or("unknown invite")?;
        if invite.expires < Timestamp::from_millis(NOW) {
            return Err("invite expired".to_owned());
        }
        let op = ledger
            .draft(self.id(), OpBody::Join(proof), Timestamp::from_millis(NOW))
            .sign(&self.key)
            .map_err(|error| error.to_string())?;
        ledger.insert(op).map_err(|error| error.to_string())?;
        Ok(())
    }

    fn on_endpoints(
        &self,
        network: NetworkId,
        from: NodeId,
        entries: Vec<(NodeId, Vec<SocketAddr>)>,
    ) {
        self.gossip.lock().unwrap().push((network, from, entries));
    }

    fn on_presence(&self, network: NetworkId, from: NodeId, entries: Vec<(NodeId, Presence)>) {
        self.presence.lock().unwrap().push((network, from, entries));
    }

    fn on_ledger_changed(&self, _network: NetworkId) {}
}

fn found() -> Ledger {
    found_by(1, "home", &[key(2).node_id()])
}

fn found_by(founder: u8, name: &str, admins: &[NodeId]) -> Ledger {
    let params = NetworkParams {
        ipv4: "100.64.0.0/10".parse().unwrap(),
        ipv6: "fd7a:115c:a1e0::/48".parse().unwrap(),
    };
    let now = Timestamp::from_millis(NOW);
    Ledger::create(&key(founder), name, "laptop", params, admins, 1, now).unwrap()
}

fn act(ledger: &mut Ledger, author: &SigningKey, body: OpBody) {
    let op = ledger
        .draft(author.node_id(), body, Timestamp::from_millis(NOW))
        .sign(author)
        .unwrap();
    ledger.insert(op).unwrap();
}

fn add_device(seed: u8) -> OpBody {
    OpBody::AddDevice {
        device: key(seed).node_id(),
        name: format!("device-{seed}"),
        tags: Vec::new(),
    }
}

fn create_invite(seed: u8, expires: u64) -> OpBody {
    OpBody::CreateInvite {
        invite_key: key(seed).verifying_key(),
        expires: Timestamp::from_millis(expires),
        max_uses: 1,
        tags: Vec::new(),
    }
}

fn record(name: &str) -> OpBody {
    OpBody::SetRecord {
        key: name.to_owned(),
        value: Some(b"value".to_vec()),
    }
}

async fn serve(node: &Arc<Node>) -> (Channel<Node>, ChannelServer) {
    let channel = Channel::new(&node.key, Arc::clone(node)).unwrap();
    let server = ChannelServer::bind(&channel, (Ipv4Addr::LOCALHOST, 0).into())
        .await
        .unwrap();
    (channel, server)
}

async fn eventually(what: &str, condition: impl Fn() -> bool) {
    let waited = timeout(Duration::from_secs(10), async {
        while !condition() {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "timed out waiting until {what}");
}

async fn exchange(
    channel: &Channel<Node>,
    addr: SocketAddr,
    server: NodeId,
    messages: &[Message],
) -> Vec<Message> {
    let mut stream = channel.dial(addr, server).await.unwrap();
    let bytes: Vec<u8> = messages
        .iter()
        .flat_map(|message| message.to_frame().unwrap())
        .collect();
    stream.write_all(&bytes).await.ok();
    stream.flush().await.ok();
    let mut frames = FrameReader::new();
    let mut received = Vec::new();
    let read = Duration::from_secs(5);
    while let Ok(message) = timeout(read, frames.next(&mut stream)).await.unwrap() {
        received.push(message);
    }
    received
}

#[tokio::test]
async fn device_with_valid_proof_joins_with_the_sponsors_whole_ledger() {
    let mut ledger = found();
    act(&mut ledger, &key(1), create_invite(50, NOW + HOUR));
    act(&mut ledger, &key(1), create_invite(51, NOW - 1));
    act(&mut ledger, &key(1), add_device(3));
    act(&mut ledger, &key(1), record("nas"));
    let network = ledger.network_id();
    let sponsor = Node::new(1, [ledger]);
    let (_sponsor_channel, server) = serve(&sponsor).await;
    let device = Node::new(9, []);
    let channel = Channel::new(&device.key, Arc::clone(&device)).unwrap();
    let addr = server.local_addr();
    let before = sponsor.op_ids(network);

    let expired = JoinProof::create(network, &key(51), &device.key, "phone").unwrap();
    let refused = ChannelClient::join(&channel, addr, sponsor.id(), network, expired).await;
    assert!(
        matches!(&refused, Err(ChannelError::JoinRejected(reason)) if reason == "invite expired"),
        "{refused:?}"
    );
    assert_eq!(sponsor.op_ids(network), before);

    let proof = JoinProof::create(network, &key(50), &device.key, "phone").unwrap();
    let joined = ChannelClient::join(&channel, addr, sponsor.id(), network, proof.clone())
        .await
        .unwrap();
    assert!(joined.view().is_member(&device.id()));
    let joined_ids: Vec<OpId> = joined.op_ids().copied().collect();
    assert_eq!(joined_ids, sponsor.op_ids(network));
    assert_eq!(joined_ids.len(), before.len() + 1);

    let rejoined = ChannelClient::join(&channel, addr, sponsor.id(), network, proof)
        .await
        .unwrap();
    assert_eq!(rejoined.op_ids().copied().collect::<Vec<_>>(), joined_ids);
    assert_eq!(sponsor.joins.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn divergent_ledgers_converge_and_changes_spread_through_a_shared_peer() {
    let mut base = found();
    act(&mut base, &key(1), add_device(2));
    act(&mut base, &key(1), add_device(3));
    let network = base.network_id();
    let mut ledger_one = base.clone();
    let mut ledger_two = base.clone();
    for index in 0..40 {
        act(&mut ledger_one, &key(1), record(&format!("one-{index}")));
        act(&mut ledger_two, &key(2), record(&format!("two-{index}")));
    }
    let hub = Node::new(1, [ledger_one]);
    let author = Node::new(2, [ledger_two]);
    let observer = Node::new(3, [base]);
    let (hub_channel, server) = serve(&hub).await;
    let addr = server.local_addr();
    let observer_channel = Channel::new(&observer.key, Arc::clone(&observer)).unwrap();
    let _observer_client = ChannelClient::connect(&observer_channel, addr, hub.id())
        .await
        .unwrap();
    eventually("the observer catches up with the hub", || {
        observer.op_ids(network) == hub.op_ids(network)
    })
    .await;

    let author_channel = Channel::new(&author.key, Arc::clone(&author)).unwrap();
    let _author_client = ChannelClient::connect(&author_channel, addr, hub.id())
        .await
        .unwrap();
    let converged = |len: usize| {
        let ids = hub.op_ids(network);
        ids.len() == len && ids == author.op_ids(network) && ids == observer.op_ids(network)
    };
    eventually("all three hold all 83 ops", || converged(83)).await;

    author.act(network, record("late-two"));
    author_channel.notify_changed(network);
    eventually("the author's change reaches everyone", || converged(84)).await;
    hub.act(network, record("late-one"));
    hub_channel.notify_changed(network);
    eventually("the hub's change reaches everyone", || converged(85)).await;
}

#[tokio::test]
async fn gossip_reaches_members_including_ones_that_connect_later() {
    let mut ledger = found();
    act(&mut ledger, &key(1), add_device(2));
    let network = ledger.network_id();
    let server_node = Node::new(1, [ledger.clone()]);
    let client_node = Node::new(2, [ledger]);
    let (server_channel, server) = serve(&server_node).await;
    let from_server = vec![(server_node.id(), vec!["192.0.2.1:41641".parse().unwrap()])];
    server_channel
        .send_endpoints(network, from_server.clone())
        .unwrap();
    let phone = Presence {
        online: true,
        last_seen: 1_700_000_000,
    };
    let present = vec![(key(9).node_id(), phone)];
    server_channel
        .send_presence(
            network,
            vec![(
                key(9).node_id(),
                Presence {
                    online: false,
                    last_seen: 0,
                },
            )],
        )
        .unwrap();
    server_channel
        .send_presence(network, present.clone())
        .unwrap();

    let client_channel = Channel::new(&client_node.key, Arc::clone(&client_node)).unwrap();
    let _client = ChannelClient::connect(&client_channel, server.local_addr(), server_node.id())
        .await
        .unwrap();
    let expected = vec![(network, server_node.id(), from_server)];
    eventually(
        "gossip from before the connection reaches the client",
        || client_node.gossip() == expected,
    )
    .await;
    let seen = vec![(network, server_node.id(), present)];
    eventually(
        "only the latest presence reaches a member that connects later",
        || *client_node.presence.lock().unwrap() == seen,
    )
    .await;

    let from_client = vec![
        (
            client_node.id(),
            vec!["[2001:db8::2]:41641".parse().unwrap()],
        ),
        (key(3).node_id(), Vec::new()),
    ];
    client_channel
        .send_endpoints(network, from_client.clone())
        .unwrap();
    let expected = vec![(network, client_node.id(), from_client)];
    eventually("the client's gossip reaches the server", || {
        server_node.gossip() == expected
    })
    .await;
}

#[tokio::test]
async fn non_member_can_neither_sync_nor_gossip() {
    let mut ledger = found();
    act(&mut ledger, &key(1), create_invite(50, NOW + HOUR));
    let network = ledger.network_id();
    let member = Node::new(1, [ledger.clone()]);
    let (member_channel, server) = serve(&member).await;
    let addr = server.local_addr();
    let outsider = Node::new(9, [ledger.clone()]);
    let channel = Channel::new(&outsider.key, Arc::clone(&outsider)).unwrap();

    let hello = Message::Hello {
        networks: vec![network],
    };
    let sync = Message::Sync {
        network,
        bytes: SyncSession::new(true, network)
            .poll_transmit(&ledger)
            .unwrap(),
    };
    let gossip = Message::Endpoints {
        network,
        entries: vec![(outsider.id(), vec!["192.0.2.9:41641".parse().unwrap()])],
    };
    let unknown_invite = Message::JoinRequest {
        network,
        proof: Box::new(JoinProof::create(network, &key(77), &outsider.key, "laptop").unwrap()),
    };
    let someone_elses = Message::JoinRequest {
        network,
        proof: Box::new(JoinProof::create(network, &key(50), &key(8), "phone").unwrap()),
    };
    let before = member.op_ids(network);
    let attempts = [
        (vec![sync.clone()], vec![]),
        (vec![gossip.clone()], vec![]),
        (vec![hello, sync.clone(), gossip.clone()], vec![]),
        (
            vec![unknown_invite.clone(), unknown_invite, sync, gossip],
            vec![Message::rejection("unknown invite")],
        ),
        (
            vec![someone_elses],
            vec![Message::rejection("the join proof names another device")],
        ),
    ];
    for (sent, expected) in attempts {
        let received = exchange(&channel, addr, member.id(), &sent).await;
        assert_eq!(received, expected, "after sending {sent:?}");
    }
    assert_eq!(member.joins.load(Ordering::Relaxed), 1);

    let mut client = ChannelClient::connect(&channel, addr, member.id())
        .await
        .unwrap();
    timeout(Duration::from_secs(5), client.closed())
        .await
        .unwrap();

    assert_eq!(member.op_ids(network), before);
    assert_eq!(member.gossip(), Vec::new());
    assert_eq!(member_channel.peers(), Vec::new());
}

#[tokio::test]
async fn member_of_one_network_cannot_pull_another_networks_ledger() {
    let mut shared = found();
    act(&mut shared, &key(1), add_device(2));
    let private = found_by(2, "private", &[]);
    let (shared_id, private_id) = (shared.network_id(), private.network_id());
    let dialer = Node::new(2, [shared, private.clone()]);
    let channel = Channel::new(&dialer.key, Arc::clone(&dialer)).unwrap();

    let acceptor = TlsAcceptor::from(kith_tls::server_config(&key(1), &[ALPN]).unwrap());
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut stream = acceptor.accept(tcp).await.unwrap();
        let mut frames = FrameReader::new();
        let hello = frames.next(&mut stream).await.unwrap();
        let pull = [
            Message::Hello {
                networks: vec![shared_id, private_id],
            },
            Message::Sync {
                network: private_id,
                bytes: SyncSession::new(true, private_id)
                    .poll_transmit(&private)
                    .unwrap(),
            },
        ];
        for message in &pull {
            stream
                .write_all(&message.to_frame().unwrap())
                .await
                .unwrap();
        }
        stream.flush().await.unwrap();
        let mut received = Vec::new();
        while let Ok(message) = frames.next(&mut stream).await {
            received.push(message);
        }
        (hello, received)
    });
    let _client = ChannelClient::connect(&channel, addr, key(1).node_id())
        .await
        .unwrap();
    let (hello, received) = timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        hello,
        Message::Hello {
            networks: vec![shared_id]
        }
    );
    assert_eq!(received, Vec::new());
}

#[test]
fn golden_bytes() {
    let hello = Message::Hello {
        networks: vec![NetworkId::from_bytes([1; 32])],
    };
    let mut expected = vec![38, b'k', b't', 0x61, 1, 0, 1];
    expected.extend([1; 32]);
    assert_eq!(hello.to_frame().unwrap(), expected);

    let endpoints = Message::Endpoints {
        network: NetworkId::from_bytes([2; 32]),
        entries: vec![(
            NodeId::from_bytes([3; 32]),
            vec!["1.2.3.4:41641".parse().unwrap(), "[::1]:5".parse().unwrap()],
        )],
    };
    let mut expected = vec![97, b'k', b't', 0x61, 1, 5];
    expected.extend([2; 32]);
    expected.push(1);
    expected.extend([3; 32]);
    expected.extend([2, 0, 1, 2, 3, 4, 0xa9, 0xc5, 0x02, 1]);
    expected.extend([0; 15]);
    expected.extend([1, 5]);
    assert_eq!(endpoints.to_frame().unwrap(), expected);
}

proptest! {
    #[test]
    fn arbitrary_bodies_never_panic(body in proptest::collection::vec(any::<u8>(), 0..512)) {
        let mut payload = vec![b'k', b't', 0x61, 1];
        payload.extend(body);
        let _ignored = Message::decode(&payload);
    }
}
