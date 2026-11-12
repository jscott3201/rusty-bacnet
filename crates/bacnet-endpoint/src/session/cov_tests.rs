//! COV reporting on an endpoint session (#2490), on paused time over the
//! in-memory link.
//!
//! A bare node on the link plays the subscriber and sends each request to
//! an endpoint and to a `bacnet-server` serving the same objects, with the
//! same Device identity: the two must answer and notify with the same
//! octets, from the first notification of a subscription through a change,
//! a renewal, a cancellation and the lifetime running out, and must list
//! the same Active_COV_Subscriptions. The opt-in, which leaves today's
//! refusal in place when it is off, and the Device's declarations are
//! checked here too; confirmed notifications, the bounds and stopping are
//! in `confirmed`.

use super::*;
use crate::bip::BipEndpointBuilder;
use crate::session::bus_port::BusPort;
use crate::session::{EndpointSession, SessionRole};
use bacnet_encoding::apdu::{ConfirmedRequest, encode_apdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_server::server::BACnetServer;
use bacnet_services::cov::SubscribeCOVRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::MacAddr;
use bacnet_types::enums::{EngineeringUnits, ObjectType, ServiceSupported};
use bytes::Bytes;
use std::net::Ipv4Addr;
use std::time::Duration;
use tokio::sync::mpsc;

#[path = "cov_confirmed_tests.rs"]
mod confirmed;

const DEVICE: u32 = 4321;
const SUBSCRIBE_COV: ConfirmedServiceChoice = ConfirmedServiceChoice::SUBSCRIBE_COV;
const READ_PROPERTY: ConfirmedServiceChoice = ConfirmedServiceChoice::READ_PROPERTY;

fn ai1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

fn lsp1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap()
}

fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, DEVICE).unwrap()
}

/// The Device, AI-1 at 20.0 and LSP-1, built alike for every host.
fn fixture() -> ObjectDatabase {
    let identity = DeviceIdentity::new(DEVICE, 0x105).unwrap();
    let mut input = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    input.set_present_value(20.0);
    let point = bacnet_objects::life_safety::LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    crate::build_database_with_extra(&identity, vec![Box::new(input), Box::new(point)]).unwrap()
}

/// A running endpoint over [`fixture`] on the in-memory link, with COV
/// under `cov` when it is given.
async fn endpoint(role: SessionRole, cov: Option<ReportConfig>) -> EndpointSession<BusPort> {
    let mut session = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(role)
        .database(fixture())
        .build_session_over(BusPort::new())
        .unwrap();
    if let Some(config) = cov {
        session = session.with_cov(config);
    }
    session.start().await.unwrap();
    session
}

/// The endpoint's MAC on the link.
fn mac_of(session: &EndpointSession<BusPort>) -> MacAddr {
    let port = session.bip_local_address().unwrap().port();
    MacAddr::from_slice(&bacnet_transport::bvll::encode_bip_mac(
        [127, 0, 0, 1],
        port,
    ))
}

/// A `bacnet-server` over [`fixture`] on the in-memory link.
async fn server() -> BACnetServer<BusPort> {
    BACnetServer::generic_builder()
        .device(DeviceIdentity::new(DEVICE, 0x105).unwrap())
        .transport(BusPort::new())
        .database(fixture())
        .build()
        .await
        .unwrap()
}

/// SubscribeCOV's parameters for AI-1: a cancellation without `confirmed`
/// and `lifetime`.
fn subscribe(process: u32, confirmed: Option<bool>, lifetime: Option<u32>) -> Vec<u8> {
    subscribe_to(ai1(), process, confirmed, lifetime)
}

/// SubscribeCOV's parameters for `object`.
fn subscribe_to(
    object: ObjectIdentifier,
    process: u32,
    confirmed: Option<bool>,
    lifetime: Option<u32>,
) -> Vec<u8> {
    let mut body = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: process,
        monitored_object_identifier: object,
        issue_confirmed_notifications: confirmed,
        lifetime,
    }
    .encode(&mut body)
    .unwrap();
    body.to_vec()
}

/// ReadProperty of the Device's `property`.
fn read_device(property: PropertyIdentifier) -> Vec<u8> {
    let mut body = BytesMut::new();
    bacnet_services::read_property::ReadPropertyRequest {
        object_identifier: device(),
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut body);
    body.to_vec()
}

/// The confirmed request `service` with `body` under `invoke_id`.
fn confirmed_apdu(invoke_id: u8, service: ConfirmedServiceChoice, body: &[u8]) -> Bytes {
    let mut apdu = BytesMut::new();
    let request = Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: service,
        service_request: Bytes::copy_from_slice(body),
    });
    encode_apdu(&mut apdu, &request).unwrap();
    apdu.freeze()
}

/// A bare node on the link: it sends APDUs and reads what comes back.
struct Node {
    port: BusPort,
    rx: mpsc::Receiver<ReceivedNpdu>,
}

impl Node {
    async fn new() -> Self {
        let mut port = BusPort::new();
        let rx = port.start().await.unwrap();
        Self { port, rx }
    }

    fn mac(&self) -> &[u8] {
        self.port.local_mac()
    }

    async fn send(&self, to: &[u8], apdu: &[u8]) {
        self.send_from(to, apdu, None).await;
    }

    /// Send `apdu` to `to` as a router passing it on from `source`, when
    /// given.
    async fn send_from(&self, to: &[u8], apdu: &[u8], source: Option<&NpduAddress>) {
        let mut npdu = BytesMut::new();
        let npdu_header = Npdu {
            expecting_reply: apdu[0] >> 4 == 0,
            source: source.cloned(),
            payload: Bytes::copy_from_slice(apdu),
            ..Npdu::default()
        };
        encode_npdu(&mut npdu, &npdu_header).unwrap();
        self.port.send_unicast(&npdu, to).await.unwrap();
    }

    /// Send `service` with `body` to `to` under `invoke_id`.
    async fn request(
        &self,
        to: &[u8],
        invoke_id: u8,
        service: ConfirmedServiceChoice,
        body: &[u8],
    ) {
        self.send(to, &confirmed_apdu(invoke_id, service, body))
            .await;
    }

    /// The next APDU and who sent it, if one comes within `within`.
    async fn next_within(&mut self, within: Duration) -> Option<(MacAddr, Bytes)> {
        let (source, npdu) = self.next_npdu(within).await?;
        Some((source, npdu.payload))
    }

    /// The next NPDU and who sent it, if one comes within `within`.
    async fn next_npdu(&mut self, within: Duration) -> Option<(MacAddr, Npdu)> {
        let received = tokio::time::timeout(within, self.rx.recv()).await.ok()??;
        Some((received.source_mac, decode_npdu(received.npdu).unwrap()))
    }

    /// Every APDU from `from` until the link has been quiet for 100 ms.
    /// Nothing else may arrive meanwhile.
    async fn quiet(&mut self, from: &[u8]) -> Vec<Bytes> {
        let mut apdus = Vec::new();
        while let Some((source, apdu)) = self.next_within(Duration::from_millis(100)).await {
            assert_eq!(source.as_slice(), from, "an APDU from another node");
            apdus.push(apdu);
        }
        apdus
    }
}

/// The subscriber and the two hosts it talks to.
struct Pair {
    node: Node,
    server: BACnetServer<BusPort>,
    endpoint: EndpointSession<BusPort>,
    invoke_id: u8,
}

impl Pair {
    async fn start() -> Self {
        Self {
            node: Node::new().await,
            server: server().await,
            endpoint: endpoint(SessionRole::ServerOnly, Some(ReportConfig::default())).await,
            invoke_id: 0,
        }
    }

    /// Send `service` with `body` to the server, then the endpoint, under
    /// one invoke ID: what each sent back, which must match octet for
    /// octet.
    async fn both(
        &mut self,
        what: &str,
        service: ConfirmedServiceChoice,
        body: &[u8],
    ) -> Vec<Bytes> {
        self.invoke_id = self.invoke_id.wrapping_add(1);
        let server = MacAddr::from_slice(self.server.local_mac());
        let endpoint = mac_of(&self.endpoint);
        self.node
            .request(&server, self.invoke_id, service, body)
            .await;
        let from_server = self.node.quiet(&server).await;
        self.node
            .request(&endpoint, self.invoke_id, service, body)
            .await;
        let from_endpoint = self.node.quiet(&endpoint).await;
        assert_eq!(from_endpoint, from_server, "{what}: endpoint, then server");
        from_endpoint
    }

    /// The application gives AI-1 `value` on both hosts: the notifications
    /// that follow, which must match.
    async fn change(&mut self, what: &str, value: f32) -> Vec<Bytes> {
        let value = PropertyValue::Real(value);
        let server = MacAddr::from_slice(self.server.local_mac());
        let endpoint = mac_of(&self.endpoint);
        self.server
            .set_present_value_local(&ai1(), value.clone())
            .await
            .unwrap();
        let from_server = self.node.quiet(&server).await;
        self.endpoint
            .set_present_value_local(&ai1(), value)
            .await
            .unwrap();
        let from_endpoint = self.node.quiet(&endpoint).await;
        assert_eq!(from_endpoint, from_server, "{what}: endpoint, then server");
        from_endpoint
    }

    async fn stop(mut self) {
        self.endpoint.stop().await.unwrap();
        self.server.stop().await.unwrap();
    }
}

/// An UnconfirmedCOVNotification's process, Time_Remaining and
/// Present_Value.
fn notified(apdu: &[u8]) -> (u32, u32, PropertyValue) {
    let Ok(Apdu::UnconfirmedRequest(request)) = decode_apdu(Bytes::copy_from_slice(apdu)) else {
        panic!("an unconfirmed request: {apdu:02X?}");
    };
    assert_eq!(
        request.service_choice,
        bacnet_types::enums::UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION
    );
    let notification =
        bacnet_services::cov::COVNotificationRequest::decode(&request.service_request).unwrap();
    let value = &notification.list_of_values[0];
    assert_eq!(value.property_identifier, PropertyIdentifier::PRESENT_VALUE);
    let (value, _) =
        bacnet_encoding::primitives::decode_application_value(&value.value, 0).unwrap();
    (
        notification.subscriber_process_identifier,
        notification.time_remaining,
        value,
    )
}

/// A SimpleACK of SubscribeCOV under `invoke_id`.
fn acked(invoke_id: u8) -> Bytes {
    Bytes::from(vec![0x20, invoke_id, SUBSCRIBE_COV.to_raw()])
}

/// Off, the opt-in leaves SubscribeCOV refused exactly as before (a Reject,
/// UNRECOGNIZED_SERVICE), the Device declares no COV and has no
/// Active_COV_Subscriptions. On, SubscribeCOVProperty is still refused.
#[tokio::test(start_paused = true)]
async fn cov_is_off_until_the_session_opts_in() {
    let mut node = Node::new().await;
    let mut off = endpoint(SessionRole::ServerOnly, None).await;
    let to = mac_of(&off);
    node.request(&to, 1, SUBSCRIBE_COV, &subscribe(1, Some(false), Some(60)))
        .await;
    assert_eq!(node.quiet(&to).await, [Bytes::from_static(&[0x60, 1, 9])]);
    node.request(
        &to,
        2,
        READ_PROPERTY,
        &read_device(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS),
    )
    .await;
    // Error: PROPERTY (2), UNKNOWN_PROPERTY (32).
    assert_eq!(
        node.quiet(&to).await,
        [Bytes::from_static(&[0x50, 2, 12, 0x91, 2, 0x91, 32])]
    );
    assert!(
        !off.executed_services()
            .contains(&ServiceSupported::SUBSCRIBE_COV)
    );
    off.stop().await.unwrap();

    let mut on = endpoint(SessionRole::ServerOnly, Some(ReportConfig::default())).await;
    let to = mac_of(&on);
    let property = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY;
    node.request(&to, 3, property, &subscribe(1, Some(false), Some(60)))
        .await;
    assert_eq!(node.quiet(&to).await, [Bytes::from_static(&[0x60, 3, 9])]);
    on.stop().await.unwrap();
}

/// On, the Device declares SubscribeCOV and lists Active_COV_Subscriptions,
/// but not Active_COV_Multiple_Subscriptions, whose service it doesn't run
/// (Table 12-13, notes 9 and 18).
#[tokio::test(start_paused = true)]
async fn the_device_declares_subscribe_cov_and_its_list() {
    let mut node = Node::new().await;
    let mut session = endpoint(SessionRole::ServerOnly, Some(ReportConfig::default())).await;
    let to = mac_of(&session);
    let read = |property| {
        let request = bacnet_services::read_property::ReadPropertyRequest {
            object_identifier: device(),
            property_identifier: property,
            property_array_index: None,
        };
        let mut body = BytesMut::new();
        request.encode(&mut body);
        body
    };
    node.request(
        &to,
        1,
        READ_PROPERTY,
        &read(PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED),
    )
    .await;
    let ack = node.quiet(&to).await.remove(0);
    let Ok(Apdu::ComplexAck(ack)) = decode_apdu(ack) else {
        panic!("a ComplexACK")
    };
    let ack = bacnet_services::read_property::ReadPropertyACK::decode(&ack.service_ack).unwrap();
    let (value, _) =
        bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0).unwrap();
    let PropertyValue::BitString { data, .. } = value else {
        panic!("a bit string")
    };
    let bit = |service: ServiceSupported| {
        let bit = usize::from(service.to_raw());
        data[bit / 8] & (0x80 >> (bit % 8)) != 0
    };
    assert!(bit(ServiceSupported::READ_PROPERTY));
    assert!(bit(ServiceSupported::SUBSCRIBE_COV));
    assert!(!bit(ServiceSupported::SUBSCRIBE_COV_PROPERTY));
    assert!(!bit(ServiceSupported::WRITE_PROPERTY));

    let listed = |property: PropertyIdentifier| {
        let raw = property.to_raw();
        let mut encoded = BytesMut::new();
        bacnet_encoding::primitives::encode_app_enumerated(&mut encoded, raw);
        encoded.to_vec()
    };
    node.request(
        &to,
        2,
        READ_PROPERTY,
        &read(PropertyIdentifier::PROPERTY_LIST),
    )
    .await;
    let list = node.quiet(&to).await.remove(0);
    let contains = |needle: &[u8]| list.windows(needle.len()).any(|window| window == needle);
    assert!(contains(&listed(
        PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS
    )));
    assert!(!contains(&listed(
        PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS
    )));
    node.request(
        &to,
        3,
        READ_PROPERTY,
        &read_device(PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS),
    )
    .await;
    assert_eq!(
        node.quiet(&to).await,
        [Bytes::from_static(&[0x50, 3, 12, 0x91, 2, 0x91, 32])]
    );
    session.stop().await.unwrap();
}

/// A subscription's first notification and a change match the server's
/// octet for octet, as do the replies and the Device's list; the reply
/// goes first.
#[tokio::test(start_paused = true)]
async fn notifications_match_the_server() {
    let mut pair = Pair::start().await;
    let list = read_device(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS);
    let empty = pair.both("the empty list", READ_PROPERTY, &list).await;
    assert_eq!(empty.len(), 1);

    let sent = pair
        .both(
            "subscribe",
            SUBSCRIBE_COV,
            &subscribe(7, Some(false), Some(300)),
        )
        .await;
    assert_eq!(sent.len(), 2, "the reply, then the first notification");
    assert_eq!(sent[0], acked(pair.invoke_id));
    assert_eq!(notified(&sent[1]), (7, 300, PropertyValue::Real(20.0)));
    pair.both("the list", READ_PROPERTY, &list).await;

    let sent = pair.change("a change", 25.0).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(notified(&sent[0]), (7, 300, PropertyValue::Real(25.0)));
    // AI-1's COV_Increment is 0: any change of value reports.
    let sent = pair.change("a smaller change", 25.5).await;
    assert_eq!(notified(&sent[0]).2, PropertyValue::Real(25.5));
    assert!(pair.change("no change", 25.5).await.is_empty());
    pair.stop().await;
}

/// A renewal restarts the lifetime and reports again; a cancellation ends
/// the subscription, and one that matches nothing succeeds; a lifetime
/// runs out on the session's clock. Each step as the server takes it.
#[tokio::test(start_paused = true)]
async fn renew_cancel_and_expire_as_the_server_does() {
    let mut pair = Pair::start().await;
    let list = read_device(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS);
    pair.both(
        "subscribe",
        SUBSCRIBE_COV,
        &subscribe(7, Some(false), Some(30)),
    )
    .await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    pair.both("the list after 10 s", READ_PROPERTY, &list).await;
    let sent = pair
        .both("renew", SUBSCRIBE_COV, &subscribe(7, Some(false), Some(60)))
        .await;
    assert_eq!(notified(&sent[1]), (7, 60, PropertyValue::Real(20.0)));

    let sent = pair
        .both("cancel", SUBSCRIBE_COV, &subscribe(7, None, None))
        .await;
    assert_eq!(sent, [acked(pair.invoke_id)]);
    pair.both("the list once cancelled", READ_PROPERTY, &list)
        .await;
    assert!(pair.change("cancelled", 21.0).await.is_empty());
    let sent = pair
        .both("cancel nothing", SUBSCRIBE_COV, &subscribe(8, None, None))
        .await;
    assert_eq!(sent, [acked(pair.invoke_id)]);

    pair.both(
        "subscribe again",
        SUBSCRIBE_COV,
        &subscribe(7, Some(false), Some(30)),
    )
    .await;
    tokio::time::sleep(Duration::from_secs(29)).await;
    let sent = pair.change("a second before it lapses", 22.0).await;
    assert_eq!(notified(&sent[0]), (7, 1, PropertyValue::Real(22.0)));
    tokio::time::sleep(Duration::from_secs(2)).await;
    // Purged at its lifetime's end, before anything looks it up.
    let host = pair.endpoint.cov.host().unwrap();
    assert!(
        host.lock().reporter.table().is_empty(),
        "purged on the clock"
    );
    let lapsed = pair
        .both("the list once lapsed", READ_PROPERTY, &list)
        .await;
    assert_eq!(lapsed, empty_list(pair.invoke_id));
    assert!(pair.change("lapsed", 23.0).await.is_empty());
    pair.stop().await;
}

/// The ComplexACK of a read of an empty Active_COV_Subscriptions.
fn empty_list(invoke_id: u8) -> [Bytes; 1] {
    let mut ack = vec![0x30, invoke_id, READ_PROPERTY.to_raw(), 0x0C];
    ack.extend_from_slice(&device().encode());
    ack.extend_from_slice(&[0x19, 0x98, 0x3E, 0x3F]);
    [Bytes::from(ack)]
}

/// With COV on, a Device write still answers alone: a written Description
/// sends nothing beyond its reply, a subscription to AI-1 notwithstanding.
#[tokio::test(start_paused = true)]
async fn a_device_write_with_cov_on_answers_alone() {
    let mut node = Node::new().await;
    let mut session = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::ServerOnly)
        .database(fixture())
        .build_session_over(BusPort::new())
        .unwrap()
        .with_device_writes(Arc::new(|_| true))
        .with_cov(ReportConfig::default());
    session.start().await.unwrap();
    let to = mac_of(&session);
    node.request(&to, 1, SUBSCRIBE_COV, &subscribe(7, Some(false), Some(60)))
        .await;
    assert_eq!(node.quiet(&to).await.len(), 2);
    let mut body = BytesMut::new();
    let mut value = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut value, "written").unwrap();
    bacnet_services::write_property::WritePropertyRequest {
        object_identifier: device(),
        property_identifier: PropertyIdentifier::DESCRIPTION,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    let write = ConfirmedServiceChoice::WRITE_PROPERTY;
    node.request(&to, 2, write, &body).await;
    assert_eq!(
        node.quiet(&to).await,
        [Bytes::from(vec![0x20, 2, write.to_raw()])]
    );
    session.stop().await.unwrap();
}

/// While a SubscribeCOV's reply is held, its first notification waits:
/// nothing leaves until `replied`, however the task is woken meanwhile.
#[tokio::test(start_paused = true)]
async fn the_first_notification_waits_for_its_reply() {
    let mut node = Node::new().await;
    let mut session = endpoint(SessionRole::ServerOnly, Some(ReportConfig::default())).await;
    let host = Arc::clone(session.cov.host().unwrap());
    let received = ReceivedApdu {
        router_response: None,
        direct_response: None,
        apdu: confirmed_apdu(1, SUBSCRIBE_COV, &subscribe(7, Some(false), Some(300))),
        source_mac: MacAddr::from_slice(node.mac()),
        source_network: None,
        ingress_network: None,
        link_layer_group: false,
        is_group: false,
        global_broadcast: false,
        data_attributes: Vec::new(),
        provenance: bacnet_transport::port::TransportProvenance::unverified(),
        reply_tx: None,
    };
    let database = Arc::clone(session.database.as_ref().unwrap());
    let reply = host
        .subscribe(&mut *database.write().await, &received)
        .unwrap();
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &reply).unwrap();
    assert_eq!(encoded.freeze(), acked(1));
    assert!(host.pop().is_none(), "held behind its reply");
    host.wake.notify_one();
    assert!(node.next_within(Duration::from_millis(50)).await.is_none());
    host.replied();
    let (_, apdu) = node.next_within(Duration::from_millis(50)).await.unwrap();
    assert_eq!(notified(&apdu), (7, 300, PropertyValue::Real(20.0)));
    session.stop().await.unwrap();
}

/// Life Safety values are captured before a local change, so a change that
/// moves Status_Flags reports even when the subscriber's last delivered
/// values match: taking LSP-1 out of service and back before the task sends
/// leaves the subscriber where the server's last notification does, in
/// service, not with the first change's stale flags.
#[tokio::test(start_paused = true)]
async fn a_life_safety_change_back_ends_where_the_server_does() {
    use bacnet_device_core::write::local::LocalCommandSource;
    let mut pair = Pair::start().await;
    let body = subscribe_to(lsp1(), 9, Some(false), Some(300));
    assert_eq!(
        pair.both("subscribe LSP-1", SUBSCRIBE_COV, &body)
            .await
            .len(),
        2
    );
    let server = MacAddr::from_slice(pair.server.local_mac());
    let endpoint = mac_of(&pair.endpoint);
    let source = LocalCommandSource::ServerDevice;
    let oos = PropertyIdentifier::OUT_OF_SERVICE;
    for on in [true, false] {
        let value = PropertyValue::Boolean(on);
        pair.server
            .write_local(&lsp1(), oos, None, value, None, source)
            .await
            .unwrap();
    }
    let from_server = pair.node.quiet(&server).await;
    // Back to back: nothing between them lets the task send.
    for on in [true, false] {
        let value = PropertyValue::Boolean(on);
        pair.endpoint
            .write_local(&lsp1(), oos, None, value, None, source)
            .await
            .unwrap();
    }
    let from_endpoint = pair.node.quiet(&endpoint).await;
    assert!(!from_server.is_empty());
    assert_eq!(
        from_endpoint.last(),
        from_server.last(),
        "endpoint, then server"
    );
    pair.stop().await;
}

/// A subscriber behind a router: its confirmed notification goes through
/// the router to its routed address, and its Ack, which the router passes
/// back, ends the report and gives the invoke ID back.
#[tokio::test(start_paused = true)]
async fn a_routed_subscriber_is_notified_and_answered() {
    let mut router = Node::new().await;
    let mut session = endpoint(SessionRole::ServerOnly, Some(ReportConfig::default())).await;
    let to = mac_of(&session);
    let behind = NpduAddress {
        network: 7,
        mac_address: MacAddr::from_slice(&[0x42]),
    };
    let request = confirmed_apdu(1, SUBSCRIBE_COV, &subscribe(7, Some(true), Some(300)));
    router.send_from(&to, &request, Some(&behind)).await;
    let (_, reply) = router.next_npdu(Duration::from_millis(100)).await.unwrap();
    assert_eq!(reply.payload, acked(1));
    let (_, notification) = router.next_npdu(Duration::from_millis(100)).await.unwrap();
    assert_eq!(notification.destination, Some(behind.clone()), "routed");
    let Ok(Apdu::ConfirmedRequest(sent)) = decode_apdu(notification.payload) else {
        panic!("a confirmed notification")
    };
    assert_eq!(session.active_leases(), 1);
    let ack = [0x20, sent.invoke_id, sent.service_choice.to_raw()];
    router.send_from(&to, &ack, Some(&behind)).await;
    let ended = async || session.active_leases() == 0;
    crate::session::eventually::eventually(Duration::from_secs(1), "the Ack ends it", ended).await;
    session.stop().await.unwrap();
}
