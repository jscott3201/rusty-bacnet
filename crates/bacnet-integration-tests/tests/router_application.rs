//! A `BACnetClient` on a `BACnetRouter`'s own application entity (#1808),
//! through `RouterApplicationPort`, on in-process loopback links. The site:
//!
//! - network 1 holds a silent station at MAC 7, which the tests name as the
//!   next router to network 9, a network nobody answers on;
//! - network 2 holds a `BACnetServer` as station 5;
//! - network 3 joins this router to a second `BACnetRouter`, which reaches
//!   network 4, where another `BACnetServer` is station 6.
//!
//! The client finds the servers with a global Who-Is, reads them through
//! the routing table, directly attached and behind the second router, takes
//! a segmented reply, runs requests side by side, times out toward network
//! 9, and hears a network the table lacks refused the way
//! `BACnetRouter::originate` refuses it.

use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_network::router::{
    BACnetRouter, OriginateError, OriginateFailure, RouterApplicationPort, RouterOptions,
    RouterPort,
};
use bacnet_network::router_table::PortId;
use bacnet_npdu::Mac;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{
    AbortReason, EngineeringUnits, ObjectType, PropertyIdentifier, Segmentation,
};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use tokio::sync::mpsc;

const INSTANCE: u32 = 4321;
const REMOTE_INSTANCE: u32 = 4322;
const NAME: &str = "Supply Air Temp";
const STATION: u8 = 5;
const REMOTE_STATION: u8 = 6;
const SILENT: u8 = 7;
/// The second router's MAC on network 3.
const NEXT_ROUTER: u8 = 8;
/// This router's MAC on network 3, longer than its others.
const LONG_MAC: [u8; 6] = [10, 0, 0, 3, 0xBA, 0xC0];
/// The client's advertised APDU limit, small enough that a whole Device
/// object needs several segments.
const CLIENT_MAX_APDU: u16 = 128;
const APDU_TIMEOUT_MS: u64 = 1_000;
const RETRIES: u8 = 2;

/// A loopback link end that reports each NPDU it sends.
struct Tap {
    link: LoopbackTransport,
    sent: Option<mpsc::UnboundedSender<Vec<u8>>>,
}

impl Tap {
    fn new(link: LoopbackTransport) -> (Self, mpsc::UnboundedReceiver<Vec<u8>>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Self {
                link,
                sent: Some(tx),
            },
            rx,
        )
    }

    fn untapped(link: LoopbackTransport) -> Self {
        Self { link, sent: None }
    }

    fn note(&self, npdu: &[u8]) {
        if let Some(sent) = &self.sent {
            let _ = sent.send(npdu.to_vec());
        }
    }
}

impl TransportPort for Tap {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.link.start().await
    }

    async fn stop(&mut self) -> Result<(), Error> {
        self.link.stop().await
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.note(npdu);
        self.link.send_unicast(npdu, mac).await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.note(npdu);
        self.link.send_broadcast(npdu).await
    }

    fn local_mac(&self) -> &[u8] {
        self.link.local_mac()
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        self.link.local_receive_apdu_capacity()
    }
}

struct Site {
    router: BACnetRouter,
    next_router: BACnetRouter,
    servers: [BACnetServer<LoopbackTransport>; 2],
    client: BACnetClient<RouterApplicationPort>,
    /// What reaches the silent station on network 1.
    silent: mpsc::Receiver<ReceivedNpdu>,
    /// What this router sends on network 2, toward station 5.
    to_station: mpsc::UnboundedReceiver<Vec<u8>>,
    /// What this router sends on network 3, toward the second router.
    to_next_router: mpsc::UnboundedReceiver<Vec<u8>>,
}

fn port(transport: Tap, network_number: u16) -> RouterPort<Tap> {
    RouterPort {
        transport,
        network_number,
    }
}

async fn site() -> Site {
    let (near, mut silent) = LoopbackTransport::pair(vec![1], vec![SILENT]);
    let (far, station) = LoopbackTransport::pair(vec![2], vec![STATION]);
    let (link, next_near) = LoopbackTransport::pair(LONG_MAC.to_vec(), vec![NEXT_ROUTER]);
    let (next_far, remote_station) = LoopbackTransport::pair(vec![9], vec![REMOTE_STATION]);
    let (far, to_station) = Tap::new(far);
    let (link, to_next_router) = Tap::new(link);
    // This router's local receiver is the tracked one.
    let started = BACnetRouter::start(
        vec![port(Tap::untapped(near), 1), port(far, 2), port(link, 3)],
        RouterOptions::new().track_admission(),
    )
    .await
    .unwrap();
    // Started second, so its announcement of network 4 teaches the first.
    let next_router = BACnetRouter::start(
        vec![
            port(Tap::untapped(next_near), 3),
            port(Tap::untapped(next_far), 4),
        ],
        RouterOptions::new(),
    )
    .await
    .unwrap()
    .router;
    let silent = silent.start().await.unwrap();
    let servers = [
        server(station, INSTANCE).await,
        server(remote_station, REMOTE_INSTANCE).await,
    ];
    let (router, port, _) = started.into_application_port();
    let client = BACnetClient::generic_builder()
        .transport(port)
        .apdu_timeout_ms(APDU_TIMEOUT_MS)
        .apdu_retries(RETRIES)
        .max_apdu_length(CLIENT_MAX_APDU)
        .build()
        .await
        .unwrap();
    let mut site = Site {
        router,
        next_router,
        servers,
        client,
        silent,
        to_station,
        to_next_router,
    };
    // Let the routers' start-up announcements go by.
    tokio::time::sleep(Duration::from_millis(100)).await;
    while site.silent.try_recv().is_ok() {}
    while site.to_station.try_recv().is_ok() {}
    while site.to_next_router.try_recv().is_ok() {}
    site
}

impl Site {
    async fn stop(mut self) {
        self.client.stop().await.unwrap();
        for server in &mut self.servers {
            server.stop().await.unwrap();
        }
        self.router.stop().await.unwrap();
        self.next_router.stop().await.unwrap();
    }

    /// Wait for a Who-Is answer from `instance`.
    async fn found(&self, instance: u32) -> bacnet_client::discovery::DiscoveredDevice {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(found) = self.client.get_device(instance).await {
                    break found;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the I-Am arrives")
    }
}

async fn server(transport: LoopbackTransport, instance: u32) -> BACnetServer<LoopbackTransport> {
    let identity = bacnet_objects::DeviceIdentity::new(instance, 0)
        .unwrap()
        .with_segmentation(Segmentation::BOTH);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: identity.clone().with_name("Air handler"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogInputObject::new(3, NAME, EngineeringUnits::DEGREES_FAHRENHEIT).unwrap(),
    ))
    .unwrap();
    BACnetServer::generic_builder()
        .device(identity)
        .database(db)
        .transport(transport)
        .build()
        .await
        .unwrap()
}

fn supply_air() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap()
}

fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, INSTANCE).unwrap()
}

fn character_string(encoded: &[u8]) -> PropertyValue {
    bacnet_encoding::primitives::decode_application_value(encoded, 0)
        .unwrap()
        .0
}

/// The read the routed tests make: Supply Air Temp's name, from station
/// `station` on `network`. The router MAC is ignored by the port.
async fn read_name(
    client: &BACnetClient<RouterApplicationPort>,
    network: u16,
    station: u8,
) -> Result<PropertyValue, Error> {
    let ack = client
        .read_property_routed(
            &[0xEE],
            network,
            &[station],
            supply_air(),
            PropertyIdentifier::OBJECT_NAME,
            None,
        )
        .await?;
    Ok(character_string(&ack.property_value))
}

#[tokio::test(start_paused = true)]
async fn the_routers_client_reads_a_device_through_its_table() {
    let site = site().await;
    // The longest of the router's MACs is the port's own.
    assert_eq!(site.client.local_mac(), LONG_MAC);
    let name = read_name(&site.client, 2, STATION).await.unwrap();
    assert_eq!(name, PropertyValue::CharacterString(NAME.into()));

    // Side by side: two transactions in flight at once, each matched to
    // its own reply.
    let (first, second) = tokio::join!(
        read_name(&site.client, 2, STATION),
        site.client.read_property_routed(
            &[0xEE],
            2,
            &[STATION],
            device(),
            PropertyIdentifier::OBJECT_NAME,
            None,
        )
    );
    assert_eq!(first.unwrap(), PropertyValue::CharacterString(NAME.into()));
    assert_eq!(
        character_string(&second.unwrap().property_value),
        PropertyValue::CharacterString("Air handler".into())
    );
    assert_eq!(site.router.counters().originated, 3);
    site.stop().await;
}

/// A device on a remote network, behind the second router: the request
/// leaves by network 3 for the next router with DNET 4, and the reply that
/// router relays back, with SNET 4 and SADR 6, completes the transaction.
#[tokio::test(start_paused = true)]
async fn a_device_behind_another_router_is_read_through_it() {
    let mut site = site().await;
    let name = read_name(&site.client, 4, REMOTE_STATION).await.unwrap();
    assert_eq!(name, PropertyValue::CharacterString(NAME.into()));
    let request = site.to_next_router.try_recv().expect("the request");
    assert_eq!(&request[..7], &[1, 0x24, 0, 4, 1, REMOTE_STATION, 255]);
    site.stop().await;
}

/// A global Who-Is leaves by every port; each I-Am comes back with the
/// device's network, which the client keeps.
#[tokio::test(start_paused = true)]
async fn a_device_found_by_who_is_is_read_by_its_instance() {
    let mut site = site().await;
    site.client.who_is(None).await.unwrap();
    let found = site.found(INSTANCE).await;
    assert_eq!(found.source_network, Some(2));
    assert_eq!(found.source_address.as_deref(), Some(&[STATION][..]));
    let remote = site.found(REMOTE_INSTANCE).await;
    assert_eq!(remote.source_network, Some(4));
    assert_eq!(
        remote.source_address.as_deref(),
        Some(&[REMOTE_STATION][..])
    );
    // Network 1's silent station heard the Who-Is too.
    let who_is = next_frame(&mut site.silent).await;
    assert_eq!(&who_is[..6], &[1, 0x20, 0xFF, 0xFF, 0, 255]);

    for instance in [INSTANCE, REMOTE_INSTANCE] {
        let ack = site
            .client
            .read_property_from_device(
                instance,
                supply_air(),
                PropertyIdentifier::OBJECT_NAME,
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            character_string(&ack.property_value),
            PropertyValue::CharacterString(NAME.into())
        );
    }
    site.stop().await;
}

/// The whole Device object doesn't fit the client's 128-octet APDUs: the
/// server segments its reply and the client's TSM acknowledges each
/// segment back through the router.
#[tokio::test(start_paused = true)]
async fn a_segmented_reply_is_reassembled() {
    let mut site = site().await;
    site.client.who_is(None).await.unwrap();
    site.found(INSTANCE).await;
    while site.to_station.try_recv().is_ok() {}
    let ack = site
        .client
        .read_property_multiple_from_device(
            INSTANCE,
            vec![ReadAccessSpecification {
                object_identifier: device(),
                list_of_property_references: vec![PropertyReference {
                    property_identifier: PropertyIdentifier::ALL,
                    property_array_index: None,
                }],
            }],
        )
        .await
        .unwrap();
    let results = &ack.list_of_read_access_results[0].list_of_results;
    let encoded: usize = results
        .iter()
        .map(|result| {
            result
                .property_value
                .as_ref()
                .map_or(0, |value| value.len())
        })
        .sum();
    assert!(
        encoded > usize::from(CLIENT_MAX_APDU),
        "{encoded} octets of values"
    );
    // On network 2: the request, then the client's Segment-ACKs (PDU type
    // 4), each straight to station 5, after the two-octet NPCI.
    let sent: Vec<Vec<u8>> = std::iter::from_fn(|| site.to_station.try_recv().ok()).collect();
    assert_eq!(sent[0][2] >> 4, 0, "the Confirmed-Request first");
    let segment_acks = sent.iter().filter(|npdu| npdu[2] >> 4 == 4).count();
    assert!(segment_acks >= 2, "{segment_acks} Segment-ACKs in {sent:?}");
    assert_eq!(segment_acks, sent.len() - 1, "{sent:?}");
    site.stop().await;
}

/// A route the table holds, to a network where nothing answers: each
/// attempt goes to the next router, and the request times out.
#[tokio::test(start_paused = true)]
async fn a_request_toward_a_silent_network_times_out() {
    let mut site = site().await;
    let now = site.router.now();
    assert!(
        site.router
            .table()
            .add_learned(9, PortId(0), Mac::from_array([SILENT]), now)
    );
    let error = read_name(&site.client, 9, 9).await.unwrap_err();
    assert!(
        matches!(
            error,
            Error::Remote(RemoteError::Abort {
                reason: AbortReason::TSM_TIMEOUT
            })
        ),
        "{error:?}"
    );
    // The first attempt and each retry, to network 9 by station 7.
    let mut attempts = 0;
    while let Ok(frame) = site.silent.try_recv() {
        assert_eq!(&frame.npdu[..6], &[1, 0x24, 0, 9, 1, 9]);
        attempts += 1;
    }
    assert_eq!(attempts, 1 + usize::from(RETRIES));
    site.stop().await;
}

/// A network the table lacks: refused at once as `originate` refuses it,
/// with the search for it started.
#[tokio::test(start_paused = true)]
async fn an_unknown_network_is_refused_as_the_router_refuses_it() {
    let mut site = site().await;
    let error = read_name(&site.client, 77, 9).await.unwrap_err();
    assert_eq!(
        OriginateFailure::from_error(&error),
        Some(OriginateFailure::Refused(OriginateError::NoRoute)),
        "{error:?}"
    );
    // Who-Is-Router-To-Network 77, on network 1 as on every port.
    assert_eq!(
        next_frame(&mut site.silent).await,
        [1, 0x80, 0x00, 0x00, 77]
    );
    site.stop().await;
}

/// The next NPDU to reach the silent station.
async fn next_frame(silent: &mut mpsc::Receiver<ReceivedNpdu>) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(2), silent.recv())
        .await
        .expect("a frame arrives")
        .expect("the link is open")
        .npdu
        .to_vec()
}
