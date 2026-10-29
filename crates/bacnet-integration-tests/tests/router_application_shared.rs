//! A `BACnetServer` and a `BACnetClient` sharing a `BACnetRouter`'s own
//! application entity (#2069), through `RouterApplicationPorts`, and the
//! ports following a router port's NPDU limit (#2070), on in-process
//! loopback links. The site:
//!
//! - network 1 holds a `BACnetClient` as station 7;
//! - network 2 holds a `BACnetServer` as station 5;
//! - the router hosts a server of its own, device 100, and a client.
//!
//! Each client reads the other side's server, a segmented reply each way,
//! each finds the other side's server by Who-Is, and each server's
//! confirmed COV notification is acknowledged once, which shows each
//! answer reached the stack that asked.

use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_network::router::{
    BACnetRouter, RouterApplicationPort, RouterApplicationPorts, RouterOptions, RouterPort,
};
use bacnet_npdu::NpduLimits;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier, Segmentation};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use tokio::sync::{mpsc, watch};
use tokio::time::sleep;

/// The router-hosted server's device instance.
const HOSTED: u32 = 100;
/// The server on network 2.
const REMOTE: u32 = 4321;
const STATION: u8 = 5;
const REMOTE_CLIENT: u8 = 7;
/// The router's MAC on network 1.
const ROUTER_ON_1: u8 = 1;
/// The clients' advertised APDU limit: a whole Device object needs several
/// segments.
const SMALL_APDU: u16 = 128;

/// A loopback link end that reports what it sends, and whose NPDU limit
/// the test sets.
struct Link {
    link: LoopbackTransport,
    sent: mpsc::UnboundedSender<Vec<u8>>,
    limits: watch::Sender<NpduLimits>,
}

impl Link {
    fn new(link: LoopbackTransport) -> (Self, mpsc::UnboundedReceiver<Vec<u8>>) {
        let (sent, rx) = mpsc::unbounded_channel();
        let limits = watch::Sender::new(NpduLimits::STANDARD);
        (Self { link, sent, limits }, rx)
    }
}

impl TransportPort for Link {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.link.start().await
    }

    async fn stop(&mut self) -> Result<(), Error> {
        self.link.stop().await
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let _ = self.sent.send(npdu.to_vec());
        self.link.send_unicast(npdu, mac).await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        let _ = self.sent.send(npdu.to_vec());
        self.link.send_broadcast(npdu).await
    }

    fn local_mac(&self) -> &[u8] {
        self.link.local_mac()
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        self.link.local_receive_apdu_capacity()
    }

    fn max_npdu(&self) -> NpduLimits {
        *self.limits.borrow()
    }

    fn max_npdu_changes(&self) -> Option<watch::Receiver<NpduLimits>> {
        Some(self.limits.subscribe())
    }
}

struct Site {
    router: BACnetRouter,
    hosted: BACnetServer<RouterApplicationPort>,
    client: BACnetClient<RouterApplicationPort>,
    remote_server: BACnetServer<LoopbackTransport>,
    remote_client: BACnetClient<LoopbackTransport>,
    /// What the router sends on network 1, toward the remote client.
    to_client: mpsc::UnboundedReceiver<Vec<u8>>,
    /// What the router sends on network 2, toward the remote server.
    to_station: mpsc::UnboundedReceiver<Vec<u8>>,
    /// Network 2's NPDU limit, as its link end reports it.
    station_limits: watch::Sender<NpduLimits>,
}

async fn site() -> Site {
    site_with(SMALL_APDU).await
}

/// The site, with the router's client advertising `client_max_apdu`.
async fn site_with(client_max_apdu: u16) -> Site {
    let (near, far_client) = LoopbackTransport::pair(vec![ROUTER_ON_1], vec![REMOTE_CLIENT]);
    let (far, station) = LoopbackTransport::pair(vec![2], vec![STATION]);
    let (near, to_client) = Link::new(near);
    let (far, to_station) = Link::new(far);
    let station_limits = far.limits.clone();
    let started = BACnetRouter::start(
        vec![
            RouterPort {
                transport: near,
                network_number: 1,
            },
            RouterPort {
                transport: far,
                network_number: 2,
            },
        ],
        RouterOptions::new().track_admission(),
    )
    .await
    .unwrap();
    let (router, RouterApplicationPorts { client, server, .. }, _) =
        started.into_application_ports();
    let hosted = serve(server, HOSTED).await;
    let client = BACnetClient::generic_builder()
        .transport(client)
        .max_apdu_length(client_max_apdu)
        .build()
        .await
        .unwrap();
    let remote_server = serve(station, REMOTE).await;
    let remote_client = BACnetClient::generic_builder()
        .transport(far_client)
        .max_apdu_length(SMALL_APDU)
        .build()
        .await
        .unwrap();
    let mut site = Site {
        router,
        hosted,
        client,
        remote_server,
        remote_client,
        to_client,
        to_station,
        station_limits,
    };
    // Let the router's start-up announcements go by.
    sleep(Duration::from_millis(100)).await;
    while site.to_client.try_recv().is_ok() {}
    while site.to_station.try_recv().is_ok() {}
    site
}

impl Site {
    async fn stop(mut self) {
        self.client.stop().await.unwrap();
        self.remote_client.stop().await.unwrap();
        self.hosted.stop().await.unwrap();
        self.remote_server.stop().await.unwrap();
        self.router.stop().await.unwrap();
    }
}

async fn serve<T: TransportPort + 'static>(transport: T, instance: u32) -> BACnetServer<T> {
    let identity = bacnet_objects::DeviceIdentity::new(instance, 0)
        .unwrap()
        .with_segmentation(Segmentation::BOTH);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: identity.clone().with_name(format!("Device {instance}")),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogInputObject::new(3, format!("Sensor {instance}"), EngineeringUnits::PERCENT).unwrap(),
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

fn sensor() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap()
}

fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

fn name(encoded: &[u8]) -> PropertyValue {
    bacnet_encoding::primitives::decode_application_value(encoded, 0)
        .unwrap()
        .0
}

fn all_of(instance: u32) -> Vec<ReadAccessSpecification> {
    vec![ReadAccessSpecification {
        object_identifier: device(instance),
        list_of_property_references: vec![PropertyReference {
            property_identifier: PropertyIdentifier::ALL,
            property_array_index: None,
        }],
    }]
}

/// The router's client reads the server on network 2, and the client on
/// network 1 reads the router's server at the router's MAC there.
#[tokio::test(start_paused = true)]
async fn each_client_reads_the_other_sides_server() {
    let site = site().await;
    let remote = site
        .client
        .read_property_routed(
            &[0xEE],
            2,
            &[STATION],
            sensor(),
            PropertyIdentifier::OBJECT_NAME,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        name(&remote.property_value),
        PropertyValue::CharacterString(format!("Sensor {REMOTE}"))
    );
    let hosted = site
        .remote_client
        .read_property(
            &[ROUTER_ON_1],
            sensor(),
            PropertyIdentifier::OBJECT_NAME,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        name(&hosted.property_value),
        PropertyValue::CharacterString(format!("Sensor {HOSTED}"))
    );
    // Both at once.
    let (remote, hosted) = tokio::join!(
        site.client.read_property_routed(
            &[0xEE],
            2,
            &[STATION],
            device(REMOTE),
            PropertyIdentifier::OBJECT_NAME,
            None,
        ),
        site.remote_client.read_property(
            &[ROUTER_ON_1],
            device(HOSTED),
            PropertyIdentifier::OBJECT_NAME,
            None,
        )
    );
    assert_eq!(
        name(&remote.unwrap().property_value),
        PropertyValue::CharacterString(format!("Device {REMOTE}"))
    );
    assert_eq!(
        name(&hosted.unwrap().property_value),
        PropertyValue::CharacterString(format!("Device {HOSTED}"))
    );
    site.stop().await;
}

/// A whole Device object, segmented each way: the remote server's reply to
/// the router's client, acknowledged by the client's TSM, and the hosted
/// server's reply to the remote client, whose Segment-ACKs reach the
/// hosted server.
#[tokio::test(start_paused = true)]
async fn a_segmented_reply_each_way() {
    let mut site = site().await;
    site.client.who_is(None).await.unwrap();
    found(&site.client, REMOTE).await;
    while site.to_station.try_recv().is_ok() {}
    let ack = site
        .client
        .read_property_multiple_from_device(REMOTE, all_of(REMOTE))
        .await
        .unwrap();
    assert!(
        !ack.list_of_read_access_results[0]
            .list_of_results
            .is_empty()
    );
    let sent: Vec<Vec<u8>> = std::iter::from_fn(|| site.to_station.try_recv().ok()).collect();
    let segment_acks = sent.iter().filter(|npdu| npdu[2] >> 4 == 4).count();
    assert!(segment_acks >= 2, "{segment_acks} Segment-ACKs in {sent:?}");

    while site.to_client.try_recv().is_ok() {}
    let ack = site
        .remote_client
        .read_property_multiple(&[ROUTER_ON_1], all_of(HOSTED))
        .await
        .unwrap();
    assert!(
        !ack.list_of_read_access_results[0]
            .list_of_results
            .is_empty()
    );
    // The hosted server's segments, Complex-ACKs with more to follow.
    let sent: Vec<Vec<u8>> = std::iter::from_fn(|| site.to_client.try_recv().ok()).collect();
    let segments = sent
        .iter()
        .filter(|npdu| npdu[2] >> 4 == 3 && npdu[2] & 0x08 != 0)
        .count();
    assert!(segments >= 3, "{segments} segments in {sent:?}");
    site.stop().await;
}

/// Who-Is from each client finds the server on the other side, while the
/// router hosts its own.
#[tokio::test(start_paused = true)]
async fn who_is_finds_servers_on_both_sides() {
    let site = site().await;
    site.client.who_is(None).await.unwrap();
    let remote = found(&site.client, REMOTE).await;
    assert_eq!(remote.source_network, Some(2));
    assert_eq!(remote.source_address.as_deref(), Some(&[STATION][..]));

    site.remote_client.who_is(None).await.unwrap();
    let hosted = found(&site.remote_client, HOSTED).await;
    assert_eq!(hosted.mac_address.as_slice(), [ROUTER_ON_1]);
    assert_eq!(hosted.source_network, None, "the router's own, not routed");
    let ack = site
        .remote_client
        .read_property_from_device(HOSTED, sensor(), PropertyIdentifier::OBJECT_NAME, None)
        .await
        .unwrap();
    assert_eq!(
        name(&ack.property_value),
        PropertyValue::CharacterString(format!("Sensor {HOSTED}"))
    );
    site.stop().await;
}

/// Each server's confirmed COV notification reaches the other side's
/// client, and its acknowledgement comes back to the server that sent it:
/// neither server sends it again.
#[tokio::test(start_paused = true)]
async fn confirmed_notifications_are_acknowledged_to_their_sender() {
    let site = site().await;
    // The hosted server notifies the remote client; the Simple-ACK must
    // reach the hosted server, not the router's client.
    let mut heard = site.remote_client.cov_notifications();
    site.remote_client
        .subscribe_cov(&[ROUTER_ON_1], 1, sensor(), true, Some(300))
        .await
        .unwrap();
    // The router's client hears the remote server's notification, a
    // Confirmed-Request, and answers it through the saved plan.
    site.client.who_is(None).await.unwrap();
    found(&site.client, REMOTE).await;
    let mut routed = site.client.cov_notifications();
    site.client
        .subscribe_cov_to_device(REMOTE, 2, sensor(), true, Some(300))
        .await
        .unwrap();

    // Long past every retry either server would make.
    sleep(Duration::from_secs(60)).await;
    let count = |rx: &mut tokio::sync::broadcast::Receiver<_>| {
        std::iter::from_fn(|| rx.try_recv().ok()).count()
    };
    assert_eq!(count(&mut heard), 1, "the hosted server's, once");
    assert_eq!(count(&mut routed), 1, "the remote server's, once");
    site.stop().await;
}

/// Network 2's link falls to 501 octets after the ports exist: both ports
/// report it and publish it, and the router's client builds its next
/// request for it.
#[tokio::test(start_paused = true)]
async fn the_ports_follow_a_router_ports_limit() {
    let mut site = site_with(1476).await;
    let mut changes = site
        .client
        .transport()
        .max_npdu_changes()
        .expect("published");
    assert_eq!(*changes.borrow_and_update(), NpduLimits::STANDARD);
    assert_eq!(site.client.transport_max_apdu_length(), 1476);
    let read = async |site: &Site| {
        site.client
            .read_property_routed(
                &[0xEE],
                2,
                &[STATION],
                sensor(),
                PropertyIdentifier::OBJECT_NAME,
                None,
            )
            .await
            .unwrap();
    };
    read(&site).await;
    let before = site.to_station.try_recv().expect("the request");

    site.station_limits.send_replace(NpduLimits::both(501));
    tokio::time::timeout(Duration::from_secs(1), changes.changed())
        .await
        .expect("a change")
        .unwrap();
    assert_eq!(*changes.borrow(), NpduLimits::both(501));
    assert_eq!(site.client.transport().max_npdu(), NpduLimits::both(501));
    assert_eq!(site.hosted_transport_max_npdu(), NpduLimits::both(501));
    assert_eq!(site.client.transport_max_apdu_length(), 480);
    // The receive capacity is a declaration, fixed when the port was made.
    assert_eq!(site.client.transport().local_receive_apdu_capacity(), 1476);

    while site.to_station.try_recv().is_ok() {}
    read(&site).await;
    let after = site.to_station.try_recv().expect("the request");
    // A Confirmed-Request; its second octet's low four bits give the
    // largest APDU the client accepts: smaller now, at most 480 (code 3).
    assert_eq!(before[2] >> 4, 0);
    assert_eq!(after[2] >> 4, 0);
    let (was, now) = (before[3] & 0x0F, after[3] & 0x0F);
    assert!(now < was && now <= 3, "{was} then {now}");
    site.stop().await;
}

/// A ConfirmedEventNotification to the router reaches both stacks: the
/// hosted server's Notification Forwarder forwards it, the router's client
/// hears it, and the sender gets exactly one answer, the server's.
#[tokio::test(start_paused = true)]
async fn an_event_notification_reaches_both_and_is_answered_once() {
    use bacnet_encoding::constructed::{
        encode_event_notification, encode_event_notification_subscription_list,
    };
    use bacnet_objects::notification_forwarder::NotificationForwarderObject;
    use bacnet_services::alarm_event::EventNotificationRequest;
    use bacnet_types::MacAddr;
    use bacnet_types::constructed::{
        BACnetAddress, BACnetEventNotificationSubscription, BACnetRecipient,
    };
    use bacnet_types::enums::{ConfirmedServiceChoice, EventState, EventType, NotifyType};
    use bacnet_types::primitives::BACnetTimeStamp;

    let mut site = site().await;
    let forwarder = ObjectIdentifier::new(ObjectType::NOTIFICATION_FORWARDER, 1).unwrap();
    site.hosted
        .database()
        .write()
        .await
        .add(Box::new(
            NotificationForwarderObject::new(1, "Forwarder").unwrap(),
        ))
        .unwrap();
    // Forward process 5 to the remote client, station 7 on network 1.
    site.remote_client
        .write_property(
            &[ROUTER_ON_1],
            forwarder,
            PropertyIdentifier::PROCESS_IDENTIFIER_FILTER,
            None,
            vec![0x21, 0x05],
            None,
        )
        .await
        .unwrap();
    let mut subscriptions = bytes::BytesMut::new();
    encode_event_notification_subscription_list(
        &mut subscriptions,
        &[BACnetEventNotificationSubscription {
            recipient: BACnetRecipient::Address(BACnetAddress {
                network_number: 1,
                mac_address: MacAddr::from_slice(&[REMOTE_CLIENT]),
            }),
            process_identifier: 77,
            issue_confirmed_notifications: false,
            time_remaining: 300,
        }],
    )
    .unwrap();
    site.remote_client
        .write_property(
            &[ROUTER_ON_1],
            forwarder,
            PropertyIdentifier::SUBSCRIBED_RECIPIENTS,
            None,
            subscriptions.to_vec(),
            None,
        )
        .await
        .unwrap();

    let mut forwarded = site.remote_client.event_notifications();
    let mut heard = site.client.event_notifications();
    while site.to_client.try_recv().is_ok() {}
    let mut request = bytes::BytesMut::new();
    encode_event_notification(
        &EventNotificationRequest {
            process_identifier: 5,
            initiating_device_identifier: device(900),
            event_object_identifier: sensor(),
            timestamp: BACnetTimeStamp::SequenceNumber(31),
            notification_class: 3,
            priority: 90,
            event_type: EventType::CHANGE_OF_STATE,
            message_text: Some("door open".into()),
            notify_type: NotifyType::EVENT,
            ack_required: false,
            from_state: EventState::NORMAL,
            to_state: EventState::OFFNORMAL,
            event_values: None,
        },
        &mut request,
    )
    .unwrap();
    let ack = site
        .remote_client
        .confirmed_request(
            &[ROUTER_ON_1],
            ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
            &request,
        )
        .await
        .unwrap();
    assert!(ack.is_empty(), "a Simple-ACK");

    let copy = tokio::time::timeout(Duration::from_secs(5), forwarded.recv())
        .await
        .expect("the forwarder's copy")
        .unwrap();
    assert_eq!(copy.notification.process_identifier, 77);
    let own = tokio::time::timeout(Duration::from_secs(5), heard.recv())
        .await
        .expect("the client hears it")
        .unwrap();
    assert_eq!(own.notification.process_identifier, 5);
    sleep(Duration::from_secs(30)).await;
    // On network 1: one Simple-ACK for ConfirmedEventNotification (service
    // 2), whoever else wrote there.
    let answers = std::iter::from_fn(|| site.to_client.try_recv().ok())
        .filter(|npdu| {
            npdu.get(2..)
                .is_some_and(|apdu| apdu[0] >> 4 == 2 && apdu.get(2) == Some(&2))
        })
        .count();
    assert_eq!(answers, 1);
    site.stop().await;
}

impl Site {
    fn hosted_transport_max_npdu(&self) -> NpduLimits {
        self.hosted.transport().expect("running").max_npdu()
    }
}

/// Wait for a Who-Is answer from `instance`.
async fn found<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    instance: u32,
) -> bacnet_client::discovery::DiscoveredDevice {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(found) = client.get_device(instance).await {
                break found;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the I-Am arrives")
}
