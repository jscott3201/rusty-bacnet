//! Who answers a What-Is-Network-Number on a segment (#2597), over an
//! in-memory link every node shares, on paused time.
//!
//! A router answers at once. A node that is not one (a client, a server)
//! holds its answer to a broadcast question for a random wait and skips it
//! when another answer is heard first (Clause 6.4.19), so one question
//! draws one answer where each node used to send its own. A question sent
//! to one node is answered by that node, at once.

use std::sync::{Arc, Mutex};

use bacnet_client::client::BACnetClient;
use bacnet_network::network_number::{ANSWER_DELAY_MAX, ANSWER_DELAY_MIN};
use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_transport::test_transport::{SentFrame, TestTransport};
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};

const QUERY: &[u8] = &[1, 0x80, 0x12];
/// The segment's network number.
const NETWORK: u16 = 7;
const ROUTER: u8 = 1;
/// The station the test speaks as; no node has this MAC.
const ASKER: u8 = 99;
/// A step on the paused clock that gives every node's tasks a turn.
const TURN: Duration = Duration::from_millis(5);

fn number_is(number: u16, flag: u8) -> Vec<u8> {
    let [high, low] = number.to_be_bytes();
    vec![1, 0x80, 0x13, high, low, flag]
}

/// A Network-Number-Is seen on the segment.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Answer {
    at: Instant,
    from: u8,
    number: u16,
    flag: u8,
}

/// One link: a broadcast reaches every other port, a unicast the port with
/// that MAC, and each Network-Number-Is broadcast is noted with its time.
#[derive(Default)]
struct Segment {
    ports: Mutex<Vec<(u8, mpsc::Sender<ReceivedNpdu>)>>,
    answers: Mutex<Vec<Answer>>,
}

impl Segment {
    /// A port at `mac`, which takes the Number controls when `numbers` (a
    /// router's port doesn't: the router answers for itself).
    fn port(self: &Arc<Self>, mac: u8, numbers: bool) -> TestTransport {
        let (tx, rx) = mpsc::channel(64);
        self.ports.lock().unwrap().push((mac, tx));
        let segment = Arc::clone(self);
        let mut port = TestTransport::builder()
            .local_mac(&[mac])
            .inbound(rx)
            .on_send(move |frame| {
                segment.carry(mac, &frame);
                async { Ok(()) }
            });
        if numbers {
            port = port.number_controls();
        }
        port.build()
    }

    fn carry(&self, from: u8, frame: &SentFrame) {
        if frame.broadcast && frame.npdu.len() == 6 && frame.npdu[..3] == [1, 0x80, 0x13] {
            self.answers.lock().unwrap().push(Answer {
                at: Instant::now(),
                from,
                number: u16::from_be_bytes([frame.npdu[3], frame.npdu[4]]),
                flag: frame.npdu[5],
            });
        }
        for (mac, port) in self.ports.lock().unwrap().iter() {
            if *mac == from || !(frame.broadcast || frame.mac.as_slice() == [*mac]) {
                continue;
            }
            port.try_send(ReceivedNpdu {
                npdu: frame.npdu.clone(),
                source_mac: MacAddr::from_slice(&[from]),
                link_layer_group: frame.broadcast,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                direct_response: None,
                reply_tx: None,
            })
            .expect("a port's queue has room");
        }
    }

    /// Send `npdu` from the asker's station, to `to` or as a broadcast, and
    /// give the nodes a turn.
    async fn say(&self, to: Option<u8>, npdu: &[u8]) {
        let frame = SentFrame {
            npdu: Bytes::copy_from_slice(npdu),
            mac: to.map_or_else(MacAddr::new, |mac| MacAddr::from_slice(&[mac])),
            broadcast: to.is_none(),
            attributes: Vec::new(),
        };
        self.carry(ASKER, &frame);
        tokio::time::sleep(TURN).await;
    }

    /// The answers noted since the last look.
    fn answers(&self) -> Vec<Answer> {
        std::mem::take(&mut *self.answers.lock().unwrap())
    }
}

async fn server(port: TestTransport, instance: u32) -> BACnetServer<TestTransport> {
    let identity = bacnet_objects::DeviceIdentity::new(instance, 0).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: identity.clone(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    BACnetServer::generic_builder()
        .device(identity)
        .database(db)
        .transport(port)
        .build()
        .await
        .unwrap()
}

async fn client(port: TestTransport) -> BACnetClient<TestTransport> {
    BACnetClient::generic_builder()
        .transport(port)
        .build()
        .await
        .unwrap()
}

/// The nodes that are not routers: three servers at MACs 10 to 12 and two
/// clients at 20 and 21.
struct Nodes {
    servers: Vec<BACnetServer<TestTransport>>,
    clients: Vec<BACnetClient<TestTransport>>,
}

const NODES: [u8; 5] = [10, 11, 12, 20, 21];

impl Nodes {
    async fn start(segment: &Arc<Segment>) -> Self {
        let mut servers = Vec::new();
        for mac in &NODES[..3] {
            servers.push(server(segment.port(*mac, true), 1000 + u32::from(*mac)).await);
        }
        let mut clients = Vec::new();
        for mac in &NODES[3..] {
            clients.push(client(segment.port(*mac, true)).await);
        }
        Self { servers, clients }
    }

    async fn stop(mut self) {
        for server in &mut self.servers {
            server.stop().await.unwrap();
        }
        for client in &mut self.clients {
            client.stop().await.unwrap();
        }
    }
}

/// Ask each node alone: it answers at once with the learned number, and no
/// other node does. This is also how the tests know every node holds the
/// number before the broadcast question goes.
async fn each_answers_when_asked_alone(segment: &Segment) {
    for mac in NODES {
        let asked = Instant::now();
        segment.say(Some(mac), QUERY).await;
        let answers = segment.answers();
        assert_eq!(answers.len(), 1, "node {mac} alone: {answers:?}");
        assert_eq!(
            (answers[0].from, answers[0].number, answers[0].flag),
            (mac, NETWORK, 0)
        );
        assert!(answers[0].at - asked < TURN, "at once");
    }
}

/// With a router on the segment, a broadcast question gets the router's
/// answer and no other: every node hears it inside its own wait.
#[tokio::test(start_paused = true)]
async fn a_router_and_five_nodes_give_one_answer() {
    let segment = Arc::new(Segment::default());
    let nodes = Nodes::start(&segment).await;
    // The router's start-up announcement teaches the nodes the number.
    let far = TestTransport::builder().local_mac(&[2]).build();
    let ports = vec![
        RouterPort {
            transport: segment.port(ROUTER, false),
            network_number: NETWORK,
        },
        RouterPort {
            transport: far,
            network_number: 8,
        },
    ];
    let StartedRouter { mut router, .. } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let announced = segment.answers();
    assert_eq!(announced.len(), 1, "{announced:?}");
    assert_eq!(
        (announced[0].from, announced[0].number, announced[0].flag),
        (ROUTER, NETWORK, 1)
    );
    each_answers_when_asked_alone(&segment).await;

    let asked = Instant::now();
    segment.say(None, QUERY).await;
    tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
    let answers = segment.answers();
    assert_eq!(answers.len(), 1, "one answer, the router's: {answers:?}");
    assert_eq!(
        (answers[0].from, answers[0].number, answers[0].flag),
        (ROUTER, NETWORK, 1)
    );
    assert!(answers[0].at - asked < TURN, "the router answers at once");

    nodes.stop().await;
    router.stop().await.unwrap();
}

/// With no router, the nodes that know the number answer for the segment:
/// the first whose wait ends, and no node that heard it. (Nodes whose
/// waits end in the same instant would each answer; none answers later.)
#[tokio::test(start_paused = true)]
async fn without_a_router_the_first_node_answers_for_all() {
    let segment = Arc::new(Segment::default());
    let nodes = Nodes::start(&segment).await;
    // Nothing is known yet, so a question gets no answer at all.
    segment.say(None, QUERY).await;
    tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
    assert_eq!(segment.answers(), []);

    // A station announces the number, as configured there; the nodes learn
    // it, and none answers an announcement.
    segment.say(None, &number_is(NETWORK, 1)).await;
    let announced = segment.answers();
    assert_eq!(announced.len(), 1, "{announced:?}");
    assert_eq!(announced[0].from, ASKER);
    each_answers_when_asked_alone(&segment).await;

    let asked = Instant::now();
    segment.say(None, QUERY).await;
    tokio::time::sleep(ANSWER_DELAY_MAX * 2).await;
    let answers = segment.answers();
    let first = answers.first().expect("some node answers");
    let waited = first.at - asked;
    assert!(
        (ANSWER_DELAY_MIN..=ANSWER_DELAY_MAX).contains(&waited),
        "inside the window: {waited:?}"
    );
    for answer in &answers {
        assert_eq!((answer.number, answer.flag), (NETWORK, 0), "{answer:?}");
        assert_eq!(answer.at, first.at, "none after the first: {answers:?}");
    }
    assert!(
        answers.len() < NODES.len(),
        "not one from each: {answers:?}"
    );

    nodes.stop().await;
}
