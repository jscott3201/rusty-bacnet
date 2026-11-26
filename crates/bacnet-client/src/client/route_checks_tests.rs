//! A routed device's router (#2823): an I-Am through another station is a
//! cue for the client's own check, never a change of where requests go.
//! Every test runs on the shared test transport and paused time, at the
//! client's default 6 s APDU timeout, which each step of a check waits.

use std::sync::{Arc, Mutex};

use bacnet_encoding::apdu::{Apdu, SimpleAck, UnconfirmedRequest, encode_apdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, encode_npdu};
use bacnet_npdu::NetworkMessageType;
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_transport::test_transport::{SendLog, SentFrame, TestTransport};
use bacnet_types::MacAddr;
use bacnet_types::enums::{
    ConfirmedServiceChoice, ObjectType, PropertyIdentifier, Segmentation, UnconfirmedServiceChoice,
};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::{Bytes, BytesMut};
use tokio::sync::broadcast::error::TryRecvError;
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};

use super::{BACnetClient, ClientConfig, DeviceEventKind, Station};

pub(super) const NETWORK: u16 = 100;
/// The router the device's first I-Am came through.
pub(super) const ROUTER: [u8; 1] = [2];
/// Another station on this link: a forger, or a router that took over.
pub(super) const OTHER: [u8; 1] = [9];
pub(super) const STATION: [u8; 1] = [3];
pub(super) const INSTANCE: u32 = 7;
/// The client's default APDU timeout: each step of a check waits this long.
pub(super) const WAIT: Duration = Duration::from_secs(6);

/// What answers the client's checks.
#[derive(Default)]
pub(super) struct Script {
    /// The devices behind [`ROUTER`] answer a Who-Is sent to them through
    /// it, but those in `quiet`.
    pub(super) device_answers: bool,
    pub(super) quiet: Vec<Vec<u8>>,
    /// The station at [`OTHER`] answers a Who-Is sent through it, naming
    /// the station asked as one it routes to.
    pub(super) forger_answers: bool,
    /// The routers that answer the client's router query.
    pub(super) routers: Vec<Vec<u8>>,
}

impl Script {
    /// The frames that answer `frame`, each with its link source.
    fn replies(&self, frame: &SentFrame) -> Vec<(Vec<u8>, Bytes)> {
        match classify(frame) {
            Sent::Probe {
                via,
                network,
                station,
            } if via == ROUTER && self.device_answers && !self.quiet.contains(&station) => {
                vec![(via, i_am_on(network, &station, INSTANCE))]
            }
            Sent::Probe {
                via,
                network,
                station,
            } if via == OTHER && self.forger_answers => {
                vec![(via, i_am_on(network, &station, INSTANCE))]
            }
            Sent::RouterQuery(network) => self
                .routers
                .iter()
                .map(|router| (router.clone(), router_announcement(&[network])))
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// A frame the client sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Sent {
    /// A Who-Is through `via` to `station` on `network`.
    Probe {
        via: Vec<u8>,
        network: u16,
        station: Vec<u8>,
    },
    /// The client's broadcast router query, for a network.
    RouterQuery(u16),
    /// A confirmed request through `via`.
    Request {
        via: Vec<u8>,
        invoke_id: u8,
    },
    Other,
}

pub(super) fn classify(frame: &SentFrame) -> Sent {
    let npdu = frame.decode_npdu();
    if npdu.message_type == Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK) {
        return match npdu.payload.as_ref() {
            [high, low] if frame.broadcast => Sent::RouterQuery(u16::from_be_bytes([*high, *low])),
            _ => Sent::Other,
        };
    }
    if npdu.message_type.is_some() {
        return Sent::Other;
    }
    match (frame.apdu(), npdu.destination) {
        (Apdu::UnconfirmedRequest(request), Some(destination))
            if request.service_choice == UnconfirmedServiceChoice::WHO_IS =>
        {
            Sent::Probe {
                via: frame.mac.to_vec(),
                network: destination.network,
                station: destination.mac_address.to_vec(),
            }
        }
        (Apdu::ConfirmedRequest(request), _) => Sent::Request {
            via: frame.mac.to_vec(),
            invoke_id: request.invoke_id,
        },
        _ => Sent::Other,
    }
}

pub(super) fn npdu(source: Option<(u16, &[u8])>, apdu: Apdu) -> Bytes {
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

/// A routed I-Am from `station` on [`NETWORK`] naming `instance`.
pub(super) fn i_am(station: &[u8], instance: u32) -> Bytes {
    i_am_on(NETWORK, station, instance)
}

/// A routed I-Am from `station` on `network` naming `instance`.
pub(super) fn i_am_on(network: u16, station: &[u8], instance: u32) -> Bytes {
    let mut service_request = BytesMut::new();
    IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::SEGMENTED_BOTH,
        vendor_id: 42,
    }
    .encode(&mut service_request);
    npdu(
        Some((network, station)),
        Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: service_request.freeze(),
        }),
    )
}

/// A vendor's own network message, which reaches a layer's control queue
/// and no report.
pub(super) fn proprietary_message() -> Bytes {
    let mut buf = BytesMut::new();
    encode_npdu(
        &mut buf,
        &Npdu {
            message_type: Some(NetworkMessageType::from_raw(0x80)),
            vendor_id: Some(999),
            ..Npdu::default()
        },
    )
    .unwrap();
    buf.freeze()
}

/// A router's announcement (I-Am-Router-To-Network) listing `networks`, sent on this link.
pub(super) fn router_announcement(networks: &[u16]) -> Bytes {
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

pub(super) fn received(source: &[u8], npdu: Bytes) -> ReceivedNpdu {
    ReceivedNpdu {
        direct_response: None,
        npdu,
        source_mac: MacAddr::from_slice(source),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

/// A client on network-less link 1, the script answering its checks.
pub(super) struct Site {
    pub(super) client: Arc<BACnetClient<TestTransport>>,
    inbound: mpsc::Sender<ReceivedNpdu>,
    pub(super) sent: SendLog,
    /// What answers the checks, which a test may change as it goes.
    pub(super) script: Arc<Mutex<Script>>,
}

impl Site {
    /// The client, and the device's first I-Am, through [`ROUTER`].
    pub(super) async fn new(script: Script) -> Self {
        let site = Self::empty(script).await;
        site.hear(&ROUTER, i_am(&STATION, INSTANCE)).await;
        site
    }

    /// The client, with an empty table.
    pub(super) async fn empty(script: Script) -> Self {
        let (inbound, rx) = mpsc::channel(4096);
        let script = Arc::new(Mutex::new(script));
        let (hook_script, hook_inbound) = (Arc::clone(&script), inbound.clone());
        let builder = TestTransport::builder()
            .local_mac(&[1])
            .inbound(rx)
            .on_send(move |frame| {
                for (from, npdu) in hook_script.lock().unwrap().replies(&frame) {
                    hook_inbound.try_send(received(&from, npdu)).unwrap();
                }
                async { Ok(()) }
            });
        let sent = builder.sent();
        let client = BACnetClient::start(ClientConfig::default(), builder.build())
            .await
            .unwrap();
        Self {
            client: Arc::new(client),
            inbound,
            sent,
            script,
        }
    }

    /// Hear `npdu` from `source` on this link, and let the client take it.
    pub(super) async fn hear(&self, source: &[u8], npdu: Bytes) {
        self.inbound.send(received(source, npdu)).await.unwrap();
        sleep(Duration::from_millis(1)).await;
    }

    pub(super) fn router(&self) -> Vec<u8> {
        self.client
            .device_table
            .lock()
            .get(NETWORK, &STATION)
            .unwrap()
            .mac_address
            .to_vec()
    }

    pub(super) fn sent(&self) -> Vec<Sent> {
        self.sent.frames().iter().map(classify).collect()
    }

    pub(super) fn count(&self, wanted: impl Fn(&Sent) -> bool) -> usize {
        self.sent().iter().filter(|sent| wanted(sent)).count()
    }

    pub(super) fn probes(&self) -> usize {
        self.count(|sent| matches!(sent, Sent::Probe { .. }))
    }

    pub(super) fn router_queries(&self) -> usize {
        self.count(|sent| *sent == Sent::RouterQuery(NETWORK))
    }

    /// A WriteProperty to the device by its address, answered through the
    /// station it went to: returns that station.
    pub(super) async fn write(&self) -> Vec<u8> {
        let before = self.sent.len();
        let client = Arc::clone(&self.client);
        let task = tokio::spawn(async move {
            let object = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap();
            client
                .write_property(
                    &Station::remote(NETWORK, &STATION),
                    object,
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    vec![0x44, 0x42, 0x28, 0x00, 0x00],
                    Some(8),
                )
                .await
        });
        let mut found = None;
        for _ in 0..100 {
            sleep(Duration::from_millis(1)).await;
            found = self.sent.frames()[before..]
                .iter()
                .find_map(|frame| match classify(frame) {
                    Sent::Request { via, invoke_id } => Some((via, invoke_id)),
                    _ => None,
                });
            if found.is_some() {
                break;
            }
        }
        let (via, invoke_id) = found.expect("the write went out");
        let ack = Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        });
        self.hear(&via, npdu(Some((NETWORK, &STATION)), ack)).await;
        task.await.unwrap().unwrap();
        via
    }

    pub(super) async fn stop(self) {
        let Ok(mut client) = Arc::try_unwrap(self.client) else {
            panic!("a task kept the client");
        };
        client.stop().await.unwrap();
    }
}

/// The issue's case: an I-Am naming the routed device through another
/// station changes nothing on its row, and the next write still goes
/// through the router the row names. Nothing is ever sent to the other
/// station, and no device notice reports the claim.
#[tokio::test(start_paused = true)]
async fn an_i_am_through_another_station_does_not_move_a_write() {
    let site = Site::new(Script {
        device_answers: true,
        ..Script::default()
    })
    .await;
    let mut events = site.client.device_events();
    let before = site.client.get_device_at(NETWORK, &STATION).await.unwrap();
    // The same device, and another instance at its address.
    site.hear(&OTHER, i_am(&STATION, INSTANCE)).await;
    site.hear(&OTHER, i_am(&STATION, INSTANCE + 1)).await;
    let row = site.client.get_device_at(NETWORK, &STATION).await.unwrap();
    assert_eq!(row.mac_address.as_slice(), ROUTER);
    assert_eq!(row.object_identifier, before.object_identifier);
    assert_eq!(site.write().await, ROUTER);
    sleep(3 * WAIT).await;
    assert_eq!(site.write().await, ROUTER);
    assert!(
        site.sent
            .frames()
            .iter()
            .all(|frame| frame.mac.as_slice() != OTHER)
    );
    // The probe's answer came through the router: an update. The claims
    // themselves made none.
    while let Ok(event) = events.try_recv() {
        assert_eq!(event.device.mac_address.as_slice(), ROUTER, "{event:?}");
    }
    site.stop().await;
}

/// The check asks the device through the router its row names; the answer
/// through that router keeps the path, and no router query follows.
#[tokio::test(start_paused = true)]
async fn an_answer_through_the_router_keeps_the_path() {
    let site = Site::new(Script {
        device_answers: true,
        routers: vec![OTHER.to_vec()],
        ..Script::default()
    })
    .await;
    site.hear(&OTHER, i_am(&STATION, INSTANCE)).await;
    sleep(3 * WAIT).await;
    assert_eq!(
        site.sent(),
        [Sent::Probe {
            via: ROUTER.to_vec(),
            network: NETWORK,
            station: STATION.to_vec()
        }]
    );
    assert_eq!(site.router(), ROUTER);
    site.stop().await;
}

/// The legitimate failover: the old router is gone and another took over.
/// The device's I-Am through the new one cues the check; the Who-Is through
/// the old router draws nothing and the new router alone answers the router
/// query: a first strike, at 12 s. The second check, a minute later, finds
/// the same, and at 84 s the row moves, reported as an update. Until then,
/// requests keep going through the old router.
#[tokio::test(start_paused = true)]
async fn a_router_that_is_gone_is_relearned_by_asking() {
    const NEW: [u8; 1] = [8];
    let site = Site::new(Script {
        device_answers: false,
        routers: vec![NEW.to_vec()],
        ..Script::default()
    })
    .await;
    let mut events = site.client.device_events();
    site.hear(&NEW, i_am(&STATION, INSTANCE)).await;
    assert_eq!(site.router(), ROUTER);
    sleep(2 * WAIT).await;
    assert_eq!(site.router(), ROUTER, "moved on the first strike");
    sleep(crate::discovery::STRIKE_GAP + WAIT).await;
    assert_eq!(
        site.router(),
        ROUTER,
        "moved before the second router query's answers were in"
    );
    sleep(WAIT).await;
    assert_eq!(site.router(), NEW);
    let check = [
        Sent::Probe {
            via: ROUTER.to_vec(),
            network: NETWORK,
            station: STATION.to_vec(),
        },
        Sent::RouterQuery(NETWORK),
    ];
    assert_eq!(site.sent(), [check.clone(), check].concat());
    let event = events.try_recv().unwrap();
    assert_eq!(event.kind, DeviceEventKind::Updated);
    assert_eq!(event.device.mac_address.as_slice(), NEW);
    assert_eq!(site.write().await, NEW);
    site.stop().await;
}

/// The device is quiet but the old router still answers for the network:
/// the path stays, whoever else answers. Several other routers, or none,
/// keep it too.
#[tokio::test(start_paused = true)]
async fn the_path_stays_unless_one_other_router_alone_answers() {
    for routers in [
        vec![ROUTER.to_vec(), OTHER.to_vec()],
        vec![OTHER.to_vec(), vec![8]],
        Vec::new(),
    ] {
        let site = Site::new(Script {
            device_answers: false,
            routers: routers.clone(),
            ..Script::default()
        })
        .await;
        site.hear(&OTHER, i_am(&STATION, INSTANCE)).await;
        sleep(3 * WAIT).await;
        assert_eq!(site.router(), ROUTER, "answered by {routers:?}");
        assert_eq!((site.probes(), site.router_queries()), (1, 1));
        assert_eq!(site.write().await, ROUTER);
        site.stop().await;
    }
}

/// A router announcement the client didn't ask for moves nothing, however
/// often it comes.
#[tokio::test(start_paused = true)]
async fn an_unasked_router_announcement_moves_nothing() {
    let site = Site::new(Script::default()).await;
    for _ in 0..3 {
        site.hear(&OTHER, router_announcement(&[NETWORK])).await;
    }
    assert_eq!(site.write().await, ROUTER);
    assert_eq!((site.probes(), site.router_queries()), (0, 0));
    site.stop().await;
}

/// First sight: a device the table doesn't hold yet gets its row through
/// the station its I-Am came through, with no check (option A; confirming a
/// device before a first write is #2824).
#[tokio::test(start_paused = true)]
async fn first_sight_makes_the_row_through_its_station() {
    let site = Site::empty(Script::default()).await;
    site.hear(&OTHER, i_am(&STATION, INSTANCE)).await;
    assert_eq!(site.router(), OTHER);
    assert_eq!(site.write().await, OTHER);
    sleep(3 * WAIT).await;
    assert_eq!((site.probes(), site.router_queries()), (0, 0));
    site.stop().await;
}

/// An hour of claims, one every 5 seconds (720), costs one check per hold:
/// at once, then after 1, 2, 4 and 8 minutes, then every 10 (the hold
/// stops doubling at the 10 minutes a silent row takes to age out), so 9
/// checks in the hour, each one Who-Is the device answers through its
/// router.
#[tokio::test(start_paused = true)]
async fn a_storm_of_claims_costs_one_check_per_hold() {
    let site = Site::new(Script {
        device_answers: true,
        ..Script::default()
    })
    .await;
    for _ in 0..720 {
        site.hear(&OTHER, i_am(&STATION, INSTANCE)).await;
        sleep(Duration::from_secs(5) - Duration::from_millis(1)).await;
    }
    assert_eq!((site.probes(), site.router_queries()), (9, 0));
    assert_eq!(site.router(), ROUTER);
    site.stop().await;
}

/// With the device quiet, each check is the Who-Is and a router query the
/// router answers, 12 seconds in all, and the hold counts from its end:
/// checks at 0, 72, 204 and 456 seconds in the row's ten minutes.
#[tokio::test(start_paused = true)]
async fn a_storm_over_a_quiet_device_costs_two_frames_per_hold() {
    let site = Site::new(Script {
        device_answers: false,
        routers: vec![ROUTER.to_vec()],
        ..Script::default()
    })
    .await;
    for _ in 0..120 {
        site.hear(&OTHER, i_am(&STATION, INSTANCE)).await;
        sleep(Duration::from_secs(5) - Duration::from_millis(1)).await;
    }
    assert_eq!((site.probes(), site.router_queries()), (4, 4));
    assert_eq!(site.router(), ROUTER);
    site.stop().await;
}

/// A site whose routers never answer a router query (older ones): the row
/// keeps its router while it lives. A router that is really gone leaves
/// its rows silent, so they age out (ten minutes silent, taken at the next
/// five-minute purge, here at 900 s), and the device's next I-Am through
/// the new router makes the row again, as first sight. A claim the client
/// can't check ends the same way once the device has been quiet that long:
/// first sight trusts the frame (#2824).
#[tokio::test(start_paused = true)]
async fn without_router_answers_a_gone_router_ages_out() {
    const NEW: [u8; 1] = [8];
    let site = Site::new(Script::default()).await;
    for _ in 0..14 {
        site.hear(&NEW, i_am(&STATION, INSTANCE)).await;
        sleep(Duration::from_secs(60) - Duration::from_millis(1)).await;
    }
    let placed = site.client.resolve_device(INSTANCE).await.unwrap();
    assert_eq!(placed, Station::routed(&ROUTER, NETWORK, &STATION));
    sleep(Duration::from_secs(61)).await;
    assert!(site.client.get_device_at(NETWORK, &STATION).await.is_none());
    site.hear(&NEW, i_am(&STATION, INSTANCE)).await;
    assert_eq!(site.router(), NEW);
    assert_eq!(site.write().await, NEW);
    site.stop().await;
}

/// A burst of claims for every device on a network, one each, is one check
/// of the network's path: one Who-Is to one of them.
#[tokio::test(start_paused = true)]
async fn a_burst_over_one_network_is_one_check() {
    let site = Site::empty(Script {
        device_answers: true,
        ..Script::default()
    })
    .await;
    for station in 10..60u8 {
        site.hear(&ROUTER, i_am(&[station], u32::from(station)))
            .await;
    }
    for station in 10..60u8 {
        site.hear(&OTHER, i_am(&[station], u32::from(station)))
            .await;
    }
    sleep(crate::discovery::FIRST_HOLD - Duration::from_secs(1)).await;
    assert_eq!((site.probes(), site.router_queries()), (1, 0));
    site.stop().await;
}

/// A claim that is never checked yet is no device notice either, and a
/// device notice for the router's own refresh still goes out.
#[tokio::test(start_paused = true)]
async fn a_held_claim_sends_no_notice() {
    let site = Site::new(Script::default()).await;
    let mut events = site.client.device_events();
    site.hear(&OTHER, i_am(&STATION, INSTANCE + 1)).await;
    assert!(matches!(events.try_recv(), Err(TryRecvError::Empty)));
    site.hear(&ROUTER, i_am(&STATION, INSTANCE)).await;
    assert_eq!(events.try_recv().unwrap().kind, DeviceEventKind::Updated);
    site.stop().await;
}
