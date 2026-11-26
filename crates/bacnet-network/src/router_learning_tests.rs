//! The router a node's own requests go through, learned and replaced only
//! through its own router query (#2831), over the shared test transport
//! with paused time.

use super::*;
use crate::router_checks::{FIRST_HOLD, LAST_HOLD, STRIKE_GAP};
use bacnet_encoding::apdu::{Apdu, UnconfirmedRequest, encode_apdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, encode_npdu};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_transport::test_transport::{SendLog, SentFrame, TestTransport};
use bacnet_types::enums::UnconfirmedServiceChoice;
use bytes::{Bytes, BytesMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::sleep;

const DNET: u16 = 100;
const OLD: [u8; 1] = [2];
const NEW: [u8; 1] = [8];
const STATION: [u8; 1] = [3];
const WAIT: Duration = Duration::from_secs(6);
/// Past the end of a check of a known router: two steps of [`WAIT`].
const CHECK: Duration = Duration::from_secs(12);
const TICK: Duration = Duration::from_millis(1);

/// What answers the learner.
#[derive(Default)]
struct Script {
    /// The station answers a Who-Is sent to it through [`OLD`].
    device_answers: bool,
    /// The routers that answer the router query.
    routers: Vec<Vec<u8>>,
}

#[derive(Debug, PartialEq, Eq)]
enum Sent {
    Probe { via: Vec<u8>, station: Vec<u8> },
    RouterQuery(u16),
    Other,
}

fn classify(frame: &SentFrame) -> Sent {
    let npdu = frame.decode_npdu();
    if npdu.message_type == Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK) {
        return match npdu.payload.as_ref() {
            [high, low] if frame.broadcast => Sent::RouterQuery(u16::from_be_bytes([*high, *low])),
            _ => Sent::Other,
        };
    }
    match (frame.apdu(), npdu.destination) {
        (Apdu::UnconfirmedRequest(request), Some(destination))
            if request.service_choice == UnconfirmedServiceChoice::WHO_IS
                && !frame.broadcast
                && destination.network == DNET =>
        {
            Sent::Probe {
                via: frame.mac.to_vec(),
                station: destination.mac_address.to_vec(),
            }
        }
        _ => Sent::Other,
    }
}

fn npdu(source: Option<(u16, &[u8])>, apdu: Apdu) -> Bytes {
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &apdu).unwrap();
    let mut buf = BytesMut::new();
    encode_npdu(
        &mut buf,
        &Npdu {
            source: source.map(|(network, mac)| NpduAddress {
                network,
                mac_address: MacAddr::from_slice(mac),
            }),
            payload: apdu_buf.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    buf.freeze()
}

/// A routed I-Am from [`STATION`] on [`DNET`] (its body is not read).
fn i_am() -> Bytes {
    npdu(
        Some((DNET, &STATION)),
        Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: Bytes::new(),
        }),
    )
}

fn announcement(networks: &[u16]) -> Bytes {
    let payload: Vec<u8> = networks.iter().flat_map(|n| n.to_be_bytes()).collect();
    let mut buf = BytesMut::new();
    encode_npdu(
        &mut buf,
        &Npdu {
            message_type: Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK),
            payload: Bytes::from(payload),
            ..Npdu::default()
        },
    )
    .unwrap();
    buf.freeze()
}

fn received(source: &[u8], npdu: Bytes, broadcast: bool) -> ReceivedNpdu {
    ReceivedNpdu {
        direct_response: None,
        npdu,
        source_mac: MacAddr::from_slice(source),
        link_layer_group: broadcast,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

impl Script {
    fn replies(&self, frame: &SentFrame) -> Vec<(Vec<u8>, Bytes)> {
        match classify(frame) {
            Sent::Probe { via, .. } if via == OLD && self.device_answers => {
                vec![(via, i_am())]
            }
            Sent::RouterQuery(network) => self
                .routers
                .iter()
                .map(|router| (router.clone(), announcement(&[network])))
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// A node with a learner on link 1, as an owner runs one: the frames it
/// receives from another network refresh the router that carried them
/// ([`RouterLearning::heard`]), its announcements reach the learner, and
/// the script answers what it sends.
struct Site {
    learning: Arc<RouterLearning>,
    sent: SendLog,
    inbound: mpsc::Sender<ReceivedNpdu>,
    script: Arc<std::sync::Mutex<Script>>,
    may_initiate: Arc<AtomicBool>,
    task: JoinHandle<()>,
}

impl Site {
    async fn new(script: Script) -> Self {
        Self::with_reports(script, true).await
    }

    /// `counted`: whether the reports know what the control queue dropped.
    async fn with_reports(script: Script, counted: bool) -> Self {
        let (inbound, rx) = mpsc::channel(4096);
        let script = Arc::new(std::sync::Mutex::new(script));
        let (hook_script, hook_inbound) = (Arc::clone(&script), inbound.clone());
        let builder = TestTransport::builder()
            .local_mac(&[1])
            .inbound(rx)
            .on_send(move |frame| {
                for (from, npdu) in hook_script.locked().replies(&frame) {
                    let broadcast = frame.broadcast;
                    hook_inbound
                        .try_send(received(&from, npdu, broadcast))
                        .unwrap();
                }
                async { Ok(()) }
            });
        let sent = builder.sent();
        let mut network = NetworkLayer::new(builder.build());
        let (mut controls, drops) = network
            .enable_network_control_receiver_counting_drops()
            .unwrap();
        let mut apdus = network.start().await.unwrap();
        let network = Arc::new(network);
        let reports = RouterReports::new(counted.then_some(drops));
        let learning = Arc::new(RouterLearning::new());
        let may_initiate = Arc::new(AtomicBool::new(true));
        let task = {
            let (learning, may) = (Arc::clone(&learning), Arc::clone(&may_initiate));
            tokio::spawn(async move {
                let pump = async {
                    loop {
                        tokio::select! {
                            Some(apdu) = apdus.recv() => {
                                if let Some(source) = &apdu.source_network {
                                    learning.heard(source.network, &apdu.source_mac);
                                }
                            }
                            Some(control) = controls.recv() => reports.offer(&control),
                        }
                    }
                };
                let run = learning.run(&network, &reports, || may.load(Ordering::Relaxed), WAIT);
                tokio::join!(pump, run);
            })
        };
        Self {
            learning,
            sent,
            inbound,
            script,
            may_initiate,
            task,
        }
    }

    /// A site whose learner knows [`OLD`] for [`DNET`], learned from its
    /// own query; the send log starts empty.
    async fn learned() -> Self {
        let site = Self::new(Script {
            routers: vec![OLD.to_vec()],
            ..Script::default()
        })
        .await;
        site.learning.unrouted(DNET);
        sleep(WAIT + TICK).await;
        assert_eq!(site.router(), Some(MacAddr::from_slice(&OLD)));
        site.script.locked().routers.clear();
        site.sent.clear();
        site
    }

    async fn hear(&self, source: &[u8], npdu: Bytes) {
        self.inbound
            .send(received(source, npdu, true))
            .await
            .unwrap();
        sleep(TICK).await;
    }

    fn router(&self) -> Option<MacAddr> {
        self.learning.router(DNET)
    }

    fn sent(&self) -> Vec<Sent> {
        self.sent.frames().iter().map(classify).collect()
    }

    fn queries(&self) -> usize {
        self.sent()
            .iter()
            .filter(|sent| matches!(sent, Sent::RouterQuery(DNET)))
            .count()
    }

    fn probes(&self) -> usize {
        self.sent()
            .iter()
            .filter(|sent| matches!(sent, Sent::Probe { .. }))
            .count()
    }

    /// The request through [`OLD`] to [`STATION`] went unanswered.
    fn silent(&self) {
        self.learning.silent(DNET, &OLD, &STATION);
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test(start_paused = true)]
async fn one_router_alone_answering_the_query_is_learned() {
    let site = Site::new(Script {
        routers: vec![OLD.to_vec()],
        ..Script::default()
    })
    .await;
    assert_eq!(site.router(), None);
    site.learning.unrouted(DNET);
    sleep(TICK).await;
    // Asked at once, by broadcast.
    assert_eq!(site.sent(), [Sent::RouterQuery(DNET)]);
    assert_eq!(site.router(), None, "decided only at the end of the wait");
    sleep(WAIT).await;
    assert_eq!(site.router(), Some(MacAddr::from_slice(&OLD)));
    assert_eq!(site.queries(), 1);
}

#[tokio::test(start_paused = true)]
async fn several_routers_teach_nothing_and_the_query_is_paced() {
    let site = Site::new(Script {
        routers: vec![OLD.to_vec(), NEW.to_vec()],
        ..Script::default()
    })
    .await;
    site.learning.unrouted(DNET);
    sleep(WAIT + TICK).await;
    assert_eq!(site.router(), None);
    // Requests keep going by broadcast, and each says so; the next query
    // waits out the hold.
    for _ in 0..10 {
        site.learning.unrouted(DNET);
        sleep(Duration::from_secs(5)).await;
    }
    assert_eq!(site.queries(), 1);
    sleep(FIRST_HOLD).await;
    site.learning.unrouted(DNET);
    sleep(TICK).await;
    assert_eq!(site.queries(), 2);
    // One router left: the next query learns it.
    site.script.locked().routers = vec![NEW.to_vec()];
    sleep(2 * FIRST_HOLD).await;
    site.learning.unrouted(DNET);
    sleep(WAIT + TICK).await;
    assert_eq!(site.queries(), 3);
    assert_eq!(site.router(), Some(MacAddr::from_slice(&NEW)));
}

#[tokio::test(start_paused = true)]
async fn answers_that_may_have_been_lost_teach_nothing() {
    let site = Site::with_reports(
        Script {
            routers: vec![OLD.to_vec()],
            ..Script::default()
        },
        false,
    )
    .await;
    site.learning.unrouted(DNET);
    sleep(WAIT + TICK).await;
    assert_eq!(site.queries(), 1);
    assert_eq!(site.router(), None);
}

#[tokio::test(start_paused = true)]
async fn an_announcement_nobody_asked_for_teaches_nothing() {
    let site = Site::new(Script::default()).await;
    site.learning.unrouted(DNET);
    sleep(WAIT + TICK).await;
    for _ in 0..100 {
        site.hear(&NEW, announcement(&[DNET])).await;
    }
    assert_eq!(site.router(), None);
    // A frame from the network through a station teaches nothing either.
    site.hear(&NEW, i_am()).await;
    assert_eq!(site.router(), None);
    assert_eq!(site.queries(), 1);
}

#[tokio::test(start_paused = true)]
async fn silence_forgets_nothing_while_the_device_answers_through_the_router() {
    let site = Site::learned().await;
    site.script.locked().device_answers = true;
    site.silent();
    sleep(CHECK).await;
    // The probe went through the router to the station asked; the device's
    // I-Am through it kept the router, with no query.
    assert_eq!(
        site.sent(),
        [Sent::Probe {
            via: OLD.to_vec(),
            station: STATION.to_vec()
        }]
    );
    assert_eq!(site.router(), Some(MacAddr::from_slice(&OLD)));
}

#[tokio::test(start_paused = true)]
async fn the_old_router_answering_the_query_keeps_it() {
    let site = Site::learned().await;
    site.script.locked().routers = vec![OLD.to_vec(), NEW.to_vec()];
    site.silent();
    sleep(CHECK + TICK).await;
    assert_eq!((site.probes(), site.queries()), (1, 1));
    sleep(STRIKE_GAP + CHECK).await;
    assert_eq!(
        (site.probes(), site.queries()),
        (1, 1),
        "no strike, no second check"
    );
    assert_eq!(site.router(), Some(MacAddr::from_slice(&OLD)));
}

/// One strike taken: [`OLD`] silent, [`NEW`] alone answering the query.
async fn struck() -> Site {
    let site = Site::learned().await;
    site.script.locked().routers = vec![NEW.to_vec()];
    site.silent();
    sleep(CHECK + TICK).await;
    assert_eq!((site.probes(), site.queries()), (1, 1));
    assert_eq!(
        site.router(),
        Some(MacAddr::from_slice(&OLD)),
        "moved on the first strike"
    );
    site
}

#[tokio::test(start_paused = true)]
async fn two_strikes_a_minute_apart_move_the_router() {
    let site = struck().await;
    sleep(STRIKE_GAP).await;
    assert_eq!(site.router(), Some(MacAddr::from_slice(&OLD)));
    sleep(CHECK).await;
    assert_eq!((site.probes(), site.queries()), (2, 2));
    assert_eq!(site.router(), Some(MacAddr::from_slice(&NEW)));
    // Nothing more is sent for the network.
    sleep(LAST_HOLD).await;
    assert_eq!((site.probes(), site.queries()), (2, 2));
}

#[tokio::test(start_paused = true)]
async fn a_frame_through_the_old_router_between_the_strikes_clears_the_strike() {
    let site = struck().await;
    site.hear(&OLD, i_am()).await;
    sleep(STRIKE_GAP + CHECK).await;
    // The second check found the old router working before its query.
    assert_eq!((site.probes(), site.queries()), (2, 1));
    assert_eq!(site.router(), Some(MacAddr::from_slice(&OLD)));
}

#[tokio::test(start_paused = true)]
async fn the_old_router_answering_the_second_query_clears_the_strike() {
    let site = struck().await;
    site.script.locked().routers = vec![OLD.to_vec(), NEW.to_vec()];
    sleep(STRIKE_GAP + CHECK).await;
    assert_eq!((site.probes(), site.queries()), (2, 2));
    // A later check that strikes is a first strike again.
    site.script.locked().routers = vec![NEW.to_vec()];
    sleep(FIRST_HOLD).await;
    site.silent();
    sleep(CHECK + TICK).await;
    assert_eq!(site.queries(), 3);
    assert_eq!(site.router(), Some(MacAddr::from_slice(&OLD)));
}

#[tokio::test(start_paused = true)]
async fn another_router_the_second_time_moves_nothing() {
    let site = struck().await;
    site.script.locked().routers = vec![vec![7]];
    sleep(STRIKE_GAP + CHECK).await;
    assert_eq!((site.probes(), site.queries()), (2, 2));
    assert_eq!(site.router(), Some(MacAddr::from_slice(&OLD)));
}

#[tokio::test(start_paused = true)]
async fn several_or_no_routers_the_second_time_move_nothing() {
    for routers in [vec![], vec![NEW.to_vec(), vec![7]]] {
        let site = struck().await;
        site.script.locked().routers = routers;
        sleep(STRIKE_GAP + CHECK).await;
        assert_eq!((site.probes(), site.queries()), (2, 2));
        assert_eq!(site.router(), Some(MacAddr::from_slice(&OLD)));
    }
}

#[tokio::test(start_paused = true)]
async fn under_dcc_a_known_router_is_not_checked_but_a_query_still_goes() {
    let site = Site::learned().await;
    site.script.locked().routers = vec![NEW.to_vec()];
    site.may_initiate.store(false, Ordering::Relaxed);
    for _ in 0..3 {
        site.silent();
        sleep(STRIKE_GAP + CHECK).await;
    }
    assert_eq!(site.sent.len(), 0, "{:?}", site.sent());
    assert_eq!(site.router(), Some(MacAddr::from_slice(&OLD)));
    // The router query is a network message: it goes for a network with
    // no router known.
    site.learning.unrouted(DNET + 1);
    sleep(TICK).await;
    assert_eq!(site.sent(), [Sent::RouterQuery(DNET + 1)]);
}

/// A flood of announcements naming the network, an hour of one a second
/// from a station on the link, while the known router is silent and each
/// request through it says so: the router moves only by the checks, which
/// the pacing bounds. Here the flooder is the one station answering the
/// query twice, so it is taken after two strikes, the stated residual;
/// what the flood buys is nothing beyond that.
#[tokio::test(start_paused = true)]
async fn an_announcement_flood_is_paced() {
    let site = Site::learned().await;
    for _ in 0..3600 {
        site.hear(&NEW, announcement(&[DNET])).await;
        site.silent();
        sleep(Duration::from_secs(1) - TICK).await;
    }
    // Two checks: a strike, then the move, a minute apart; the flood's
    // own announcements answered no query of the learner's, and once the
    // router moved, silence through NEW is never claimed by this test.
    assert_eq!((site.probes(), site.queries()), (2, 2));
    assert_eq!(site.router(), Some(MacAddr::from_slice(&NEW)));
}

/// The same flood with the known router carrying the device's frames: no
/// check ever finds it silent, so each ends at its Who-Is, and the hold
/// paces them (at once, then after 1, 2, 4 and 8 minutes, then every 10).
#[tokio::test(start_paused = true)]
async fn a_flood_against_a_working_router_moves_nothing() {
    let site = Site::learned().await;
    site.script.locked().device_answers = true;
    for _ in 0..3600 {
        site.hear(&NEW, announcement(&[DNET])).await;
        site.silent();
        sleep(Duration::from_secs(1) - TICK).await;
    }
    assert_eq!(site.queries(), 0);
    assert!(site.probes() <= 9, "{}", site.probes());
    assert_eq!(site.router(), Some(MacAddr::from_slice(&OLD)));
}

/// A learner tracks at most [`MAX_NETWORKS`] networks: one past them is
/// never asked about, and requests to it keep going by broadcast.
#[tokio::test(start_paused = true)]
async fn a_network_past_the_cap_is_never_asked_about() {
    let site = Site::new(Script::default()).await;
    let last = u16::try_from(MAX_NETWORKS).unwrap() + 1;
    for network in 1..=last {
        site.learning.unrouted(network);
    }
    sleep(WAIT * u32::try_from(MAX_NETWORKS).unwrap() + WAIT).await;
    let asked: std::collections::BTreeSet<u16> = site
        .sent()
        .into_iter()
        .filter_map(|sent| match sent {
            Sent::RouterQuery(network) => Some(network),
            _ => None,
        })
        .collect();
    assert_eq!(asked.len(), MAX_NETWORKS);
    assert!(!asked.contains(&last));
    assert_eq!(site.sent.len(), MAX_NETWORKS, "one query each");
}
