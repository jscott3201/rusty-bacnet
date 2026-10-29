//! How a `Recipient_List` entry becomes a network destination.
//!
//! Clause 21's `BACnetAddress` carries two independent fields — `network-number`
//! (zero selects the local network) and `mac-address` (empty selects
//! broadcast delivery) — so a recipient names one of four
//! destinations, not one unicast with edge cases. These tests pin each one, plus
//! the two that cannot be resolved at all.
//!
//! Split from `event_notifications_tests.rs`, which is at the 700-LOC cap.

use super::event_notifications_tests::local_broadcast_destination;
use super::*;
use crate::server::test_transport::{BIP_LOCAL_MAC, SendLog, TestTransport};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event::{EventStateChange, EventTransition};
use bacnet_objects::notification_class::{MAX_RECIPIENT_LIST_DESTINATIONS, NotificationClass};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{BACnetAddress, BACnetDestination, BACnetRecipient};
use bacnet_types::enums::{EngineeringUnits, EventState, EventType};
use bytes::Bytes;
use std::borrow::Cow;

mod group_routes;
mod post_route_counters;
mod route_skip_counters;
mod suppression_counters;

/// One recorded unicast send: destination MAC and NPDU bytes.
type UnicastFrame = (Vec<u8>, Bytes);

/// Records broadcasts and unicasts, keeping each unicast's target MAC. The MAC
/// matters: a recipient on a remote network that is delivered as a local
/// unicast reaches whichever device happens to hold that MAC on this link,
/// which is the failure this module exists to catch.
fn routing_transport() -> (TestTransport, SendLog) {
    let transport = TestTransport::builder()
        .local_mac(&BIP_LOCAL_MAC)
        .broadcast_mac(LITERAL_BROADCAST_MAC)
        .build();
    let sent = transport.sent();
    (transport, sent)
}

/// This test link's literal broadcast MAC — the data-link spelling of a
/// broadcast (#360), as distinct from the zero-length network-layer form.
pub(super) const LITERAL_BROADCAST_MAC: &[u8] = &[255, 255, 255, 255, 0xBA, 0xC0];

/// A destination carrying `recipient`, active on every day at every time for
/// every transition, so only the address shape under test decides the outcome.
pub(super) fn destination_for(recipient: BACnetRecipient, confirmed: bool) -> BACnetDestination {
    BACnetDestination {
        recipient,
        issue_confirmed_notifications: confirmed,
        ..local_broadcast_destination()
    }
}

/// Whether `frame` is a Who-Is, as a look for an unbound Device recipient
/// sends (#1368).
pub(super) fn is_who_is(frame: &crate::server::test_transport::SentFrame) -> bool {
    matches!(
        frame.apdu(),
        Apdu::UnconfirmedRequest(request)
            if request.service_choice == UnconfirmedServiceChoice::WHO_IS
    )
}

/// Split broadcast NPDUs into the Who-Is requests among them, which looks
/// for unbound Device recipients send (#1368), and the rest.
pub(super) fn split_who_is(broadcasts: Vec<Bytes>) -> (Vec<WhoIsRequest>, Vec<Bytes>) {
    let mut who_is = Vec::new();
    let mut rest = Vec::new();
    for npdu in broadcasts {
        let payload = bacnet_encoding::npdu::decode_npdu(npdu.clone())
            .unwrap()
            .payload;
        match bacnet_encoding::apdu::decode_apdu(payload).unwrap() {
            Apdu::UnconfirmedRequest(request)
                if request.service_choice == UnconfirmedServiceChoice::WHO_IS =>
            {
                who_is.push(WhoIsRequest::decode(&request.service_request).unwrap());
            }
            _ => rest.push(npdu),
        }
    }
    (who_is, rest)
}

pub(super) fn address_recipient(network_number: u16, mac: &[u8]) -> BACnetRecipient {
    BACnetRecipient::Address(BACnetAddress {
        network_number,
        mac_address: MacAddr::from_slice(mac),
    })
}

/// Fire one TO_OFFNORMAL transition at an AnalogInput whose Notification Class
/// holds exactly `destinations`, and return what reached the transport.
async fn distribute_to(destinations: Vec<BACnetDestination>) -> (Vec<Bytes>, Vec<UnicastFrame>) {
    distribute_with_priority([255, 255, 255], destinations).await
}

/// [`distribute_to`] with the Notification Class's per-transition Priority
/// array set, for tests that pin the Table 13-6 NPDU-priority projection.
pub(super) async fn distribute_with_priority(
    priority: [u8; 3],
    destinations: Vec<BACnetDestination>,
) -> (Vec<Bytes>, Vec<UnicastFrame>) {
    distribute_with_clock_mode(priority, destinations, true).await
}

async fn distribute_with_clock_mode(
    priority: [u8; 3],
    destinations: Vec<BACnetDestination>,
    clocked: bool,
) -> (Vec<Bytes>, Vec<UnicastFrame>) {
    let mut db = if clocked {
        clocked_test_database()
    } else {
        ObjectDatabase::new()
    };
    let mut nc = NotificationClass::new(0, "NC-0").unwrap();
    nc.priority = priority;
    for destination in destinations {
        nc.add_destination(destination).unwrap();
    }
    db.add(Box::new(nc)).unwrap();
    distribute_from_database(db).await
}

async fn distribute_from_database(db: ObjectDatabase) -> (Vec<Bytes>, Vec<UnicastFrame>) {
    distribute_from_database_with_bindings(
        db,
        Arc::new(RwLock::new(
            super::device_bindings::DeviceBindingTable::new(),
        )),
    )
    .await
}

pub(super) async fn distribute_from_database_with_bindings(
    db: ObjectDatabase,
    device_bindings: Arc<RwLock<super::device_bindings::DeviceBindingTable>>,
) -> (Vec<Bytes>, Vec<UnicastFrame>) {
    let (broadcasts, unicasts, _) = distribute_counted(db, device_bindings, DccState::Enable).await;
    (broadcasts, unicasts)
}

/// [`distribute_from_database_with_bindings`] under the given DCC state, also
/// returning the undelivered-notification counters the transition moved.
pub(super) async fn distribute_counted(
    db: ObjectDatabase,
    device_bindings: Arc<RwLock<super::device_bindings::DeviceBindingTable>>,
    state: DccState,
) -> (Vec<Bytes>, Vec<UnicastFrame>, EventNotificationCounters) {
    let (transport, _) = routing_transport();
    distribute_counted_on(transport, db, device_bindings, state).await
}

/// [`distribute_counted`] over a caller-built transport.
pub(super) async fn distribute_counted_on(
    transport: TestTransport,
    db: ObjectDatabase,
    device_bindings: Arc<RwLock<super::device_bindings::DeviceBindingTable>>,
    state: DccState,
) -> (Vec<Bytes>, Vec<UnicastFrame>, EventNotificationCounters) {
    let sent = transport.sent();
    let network = Arc::new(NetworkLayer::new(transport));
    distribute_counted_through(&network, &sent, db, device_bindings, state).await
}

/// [`distribute_counted`] through a caller's network, such as a started
/// server's, reading what it sent from `sent`.
pub(super) async fn distribute_counted_through(
    network: &Arc<NetworkLayer<TestTransport>>,
    sent: &SendLog,
    mut db: ObjectDatabase,
    device_bindings: Arc<RwLock<super::device_bindings::DeviceBindingTable>>,
    state: DccState,
) -> (Vec<Bytes>, Vec<UnicastFrame>, EventNotificationCounters) {
    let comm_state = Arc::new(CommState::default());
    comm_state.set_for_test(state);
    let suppressions = Arc::default();
    let learned_routers = Arc::new(Mutex::new(LearnedRouterCache::new()));

    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default().with_name("Dev"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::SQUARE_METERS).unwrap();
    ai.write_property(
        PropertyIdentifier::NOTIFY_TYPE,
        None,
        PropertyValue::Enumerated(NotifyType::ALARM.to_raw()),
        None,
    )
    .unwrap();
    db.add(Box::new(ai)).unwrap();

    let db = Arc::new(RwLock::new(db));
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let notifications = NotificationTransactions::new();
    BACnetServer::<TestTransport>::build_and_send_event_notification_with_bindings(
        &EventDelivery {
            db: &db,
            network,
            comm_state: &comm_state,
            learned_routers: &learned_routers,
            notification_transactions: &notifications,
            device_bindings: &device_bindings,
            suppressions: &suppressions,
            retry_timeout_ms: 1000,
            local_apdu_capacity: 1476,
        },
        &oid,
        (
            EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            EventType::OUT_OF_RANGE,
        ),
    )
    .await;

    // The confirmed path spawns its send, so a test that sampled the transport
    // straight after the await would pass whether or not anything was sent.
    // Yield until the spawned task has reached its first send.
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    // A Device recipient with no binding is looked for with a Who-Is first
    // (#1368). Its device stays silent here: its probe runs out now, and the
    // notification waiting on it is skipped and counted.
    device_bindings.write().await.probes.run_out_for_test();
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    notifications.close();
    while let Some(result) = notifications.join_next().await {
        NotificationTransactions::observe(Some(result));
    }

    let broadcasts = sent
        .broadcasts()
        .into_iter()
        .map(|frame| frame.npdu)
        .collect();
    let unicasts = sent
        .unicasts()
        .into_iter()
        .map(|frame| (frame.mac.to_vec(), frame.npdu))
        .collect();
    (broadcasts, unicasts, suppressions.snapshot())
}

enum TestRecipientList {
    Unavailable,
    Invalid,
    /// This many local-broadcast destinations, process identifiers 1 up.
    Broadcasts(u32),
}

struct TestNotificationClass {
    oid: ObjectIdentifier,
    recipient_list: TestRecipientList,
}

impl TestNotificationClass {
    fn new(recipient_list: TestRecipientList) -> Self {
        Self {
            oid: ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 0).unwrap(),
            recipient_list,
        }
    }
}

impl BACnetObject for TestNotificationClass {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "recipient-lookup-test-class"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => Ok(PropertyValue::Unsigned(0)),
            p if p == PropertyIdentifier::RECIPIENT_LIST => match self.recipient_list {
                TestRecipientList::Unavailable => Err(Error::protocol(
                    ErrorClass::PROPERTY,
                    ErrorCode::UNKNOWN_PROPERTY,
                )),
                TestRecipientList::Invalid => Ok(PropertyValue::ApplicationData(vec![0x5E])),
                TestRecipientList::Broadcasts(count) => {
                    let destinations: Vec<_> = (1..=count)
                        .map(|process_identifier| BACnetDestination {
                            process_identifier,
                            ..destination_for(address_recipient(0, &[]), false)
                        })
                        .collect();
                    let mut list = BytesMut::new();
                    bacnet_encoding::constructed::encode_destination_list(&mut list, &destinations)
                        .unwrap();
                    Ok(PropertyValue::ApplicationData(list.to_vec()))
                }
            },
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            )),
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::RECIPIENT_LIST,
        ])
    }
}

async fn distribute_non_matched_case(case: &str) -> (Vec<Bytes>, Vec<UnicastFrame>) {
    distribute_from_database(non_matched_database(case)).await
}

/// A database whose Notification Class 0 lookup gives the named outcome.
fn non_matched_database(case: &str) -> ObjectDatabase {
    let mut db = clocked_test_database();
    match case {
        "missing-class" => {}
        "list-unavailable" => db
            .add(Box::new(TestNotificationClass::new(
                TestRecipientList::Unavailable,
            )))
            .unwrap(),
        "list-invalid" => db
            .add(Box::new(TestNotificationClass::new(
                TestRecipientList::Invalid,
            )))
            .unwrap(),
        // Only a custom class can serve a list past the cap (#1124).
        "list-past-the-cap" => db
            .add(Box::new(TestNotificationClass::new(
                TestRecipientList::Broadcasts(CAP + 1),
            )))
            .unwrap(),
        "empty-list" => db
            .add(Box::new(NotificationClass::new(0, "NC-0").unwrap()))
            .unwrap(),
        "no-eligible-destination" => {
            let mut nc = NotificationClass::new(0, "NC-0").unwrap();
            let mut destination = destination_for(address_recipient(0, &[]), false);
            destination.transitions = EventTransition::ToNormal.bit_mask();
            nc.add_destination(destination).unwrap();
            db.add(Box::new(nc)).unwrap();
        }
        _ => unreachable!("test case is fixed"),
    }
    db
}

#[tokio::test]
async fn every_non_matched_lookup_outcome_emits_no_frame() {
    for case in [
        "missing-class",
        "list-unavailable",
        "list-invalid",
        "list-past-the-cap",
        "empty-list",
        "no-eligible-destination",
    ] {
        let (broadcasts, unicasts) = distribute_non_matched_case(case).await;
        assert!(broadcasts.is_empty(), "{case} must not widen to broadcast");
        assert!(unicasts.is_empty(), "{case} must not emit a unicast");
    }
}

/// The Recipient_List cap (#1098), which routing applies to every class.
const CAP: u32 = MAX_RECIPIENT_LIST_DESTINATIONS as u32;

#[tokio::test]
async fn custom_class_at_the_cap_reaches_every_destination() {
    // #1124: the routing bound is inclusive. A custom class serving exactly
    // the cap is routed whole; one more destination and none is.
    let mut db = clocked_test_database();
    db.add(Box::new(TestNotificationClass::new(
        TestRecipientList::Broadcasts(CAP),
    )))
    .unwrap();
    let (broadcasts, unicasts) = distribute_from_database(db).await;
    assert_eq!(broadcasts.len(), CAP as usize);
    assert!(unicasts.is_empty());
}

#[tokio::test]
async fn clockless_all_day_recipient_uses_system_utc_filter_fallback() {
    let (broadcasts, unicasts) = distribute_with_clock_mode(
        [255; 3],
        vec![destination_for(address_recipient(0, &[]), false)],
        false,
    )
    .await;

    assert_eq!(broadcasts.len(), 1, "clockless mode preserves delivery");
    assert!(unicasts.is_empty());
}

/// The NPDU destination (DNET/DADR) of a captured frame, or `None` when the
/// destination specifier bit is clear.
pub(super) fn npdu_destination(frame: &Bytes) -> Option<(u16, Vec<u8>)> {
    let npdu = bacnet_encoding::npdu::decode_npdu(frame.clone()).expect("decode NPDU");
    npdu.destination
        .map(|d| (d.network, d.mac_address.to_vec()))
}

/// #185: Clause 12.21 requires a device whose `Recipient_List` is not writable
/// to ship exactly one entry whose Recipient targets the local link broadcast.
/// Clause 21 spells a local broadcast as network 0 with a zero-length MAC.
///
/// This previously reached `send_apdu` with an empty MAC, which BACnet/IP
/// rejects ("B/IP address must be 6 octets, got 0"), so the one configuration the
/// standard prescribes for a whole class of device sent nothing at all.
#[tokio::test]
async fn zero_length_mac_on_local_network_broadcasts() {
    let (broadcasts, unicasts) =
        distribute_to(vec![destination_for(address_recipient(0, &[]), false)]).await;

    assert_eq!(
        broadcasts.len(),
        1,
        "a zero-length MAC on network 0 is a local broadcast request"
    );
    assert!(
        unicasts.is_empty(),
        "a zero-length MAC must never be offered to a unicast send"
    );
    assert_eq!(
        npdu_destination(&broadcasts[0]),
        None,
        "a local broadcast carries no DNET"
    );
}

/// #185/#186: a zero-length MAC on a *remote* network is a remote broadcast.
/// Clause 6.3 encodes that destination with the remote network number in
/// DNET and a zero DLEN.
#[tokio::test]
async fn zero_length_mac_on_remote_network_broadcasts_with_dnet() {
    let (broadcasts, unicasts) =
        distribute_to(vec![destination_for(address_recipient(1000, &[]), false)]).await;

    assert_eq!(broadcasts.len(), 1, "remote broadcast goes out");
    assert!(unicasts.is_empty());
    assert_eq!(
        npdu_destination(&broadcasts[0]),
        Some((1000, vec![])),
        "DNET names the remote network and DLEN is zero"
    );
}

/// Network 65535 with a zero-length MAC is a *global* broadcast, not a remote
/// network that happens to be numbered 65535. Clause 6.3 reserves DNET
/// X'FFFF' for the global form, which routers propagate everywhere.
///
/// It needs its own send: `NetworkLayer::broadcast_to_network` rejects 0xFFFF
/// ("dest_network 0xFFFF is the global broadcast; use broadcast_global_apdu
/// for a global broadcast"), so routing it as an ordinary remote broadcast
/// turns the notification into a logged send error and delivers nothing.
#[tokio::test]
async fn zero_length_mac_on_network_65535_is_a_global_broadcast() {
    let (broadcasts, unicasts) =
        distribute_to(vec![destination_for(address_recipient(65535, &[]), false)]).await;

    assert_eq!(broadcasts.len(), 1, "global broadcast goes out");
    assert!(unicasts.is_empty());
    assert_eq!(
        npdu_destination(&broadcasts[0]),
        Some((0xFFFF, vec![])),
        "DNET is the global broadcast address and DLEN is zero"
    );
}

/// Network 65535 with a unicast MAC is self-contradictory — a broadcast
/// requires DLEN zero — so it is skipped rather than guessed at.
#[tokio::test]
async fn global_broadcast_network_with_a_mac_is_skipped() {
    let mac = [0x0A, 0x00, 0x00, 0x64, 0xBA, 0xC0];
    let (broadcasts, unicasts) =
        distribute_to(vec![destination_for(address_recipient(65535, &mac), false)]).await;

    assert!(broadcasts.is_empty());
    assert!(unicasts.is_empty());
}

/// #186: a unicast MAC on a remote network goes out as a routed NPDU whose
/// DNET/DADR name the recipient, sent with a broadcast link DA — Clause
/// 6.5.3's initial send form before learning the router's address
/// (this non-routing device keeps no router table).
///
/// The unicast assertion still matters most: the pre-#357 behavior discarded
/// `network_number` entirely and delivered the alarm to whichever device held
/// that MAC on the *local* link.
#[tokio::test]
async fn remote_unicast_recipient_routes_via_broadcast_link_da() {
    let mac = [0x0A, 0x00, 0x00, 0x64, 0xBA, 0xC0];
    let (broadcasts, unicasts) =
        distribute_to(vec![destination_for(address_recipient(1000, &mac), false)]).await;

    assert!(
        unicasts.is_empty(),
        "must not deliver a remote recipient to that MAC on the local link"
    );
    assert_eq!(broadcasts.len(), 1, "one routed frame on the broadcast DA");
    assert_eq!(
        npdu_destination(&broadcasts[0]),
        Some((1000, mac.to_vec())),
        "DNET names the remote network and DADR carries the recipient's MAC"
    );
}

/// #360: a confirmed recipient spelled with the data link's *literal*
/// broadcast MAC is the same unsatisfiable ask as the zero-length form —
/// Clause 6.3 permits only unconfirmed PDUs at a broadcast — and must be
/// skipped, not unicast to the broadcast address.
#[tokio::test]
async fn confirmed_recipient_at_literal_broadcast_mac_is_skipped() {
    let (broadcasts, unicasts) = distribute_to(vec![destination_for(
        address_recipient(0, LITERAL_BROADCAST_MAC),
        true,
    )])
    .await;

    assert!(
        unicasts.is_empty(),
        "a literal broadcast MAC must not be offered to a confirmed unicast send"
    );
    assert!(broadcasts.is_empty(), "and must not be broadcast confirmed");
}

/// #360, the deliverable half: an *unconfirmed* recipient at the literal
/// broadcast MAC resolves to the same broadcast route as the zero-length
/// spelling.
#[tokio::test]
async fn unconfirmed_recipient_at_literal_broadcast_mac_broadcasts() {
    let (broadcasts, unicasts) = distribute_to(vec![destination_for(
        address_recipient(0, LITERAL_BROADCAST_MAC),
        false,
    )])
    .await;

    assert_eq!(broadcasts.len(), 1, "delivered via the broadcast path");
    assert!(unicasts.is_empty());
}

/// #125: a device-instance recipient needs a device-to-address binding this
/// device does not maintain, so it is skipped rather than widened to a
/// broadcast that reaches every device on the link.
#[tokio::test]
async fn device_recipient_is_skipped_not_broadcast() {
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap();
    let (broadcasts, unicasts) = distribute_to(vec![destination_for(
        BACnetRecipient::Device(device),
        false,
    )])
    .await;

    // The device is looked for with a Who-Is limited to it (#1368), and its
    // notification goes nowhere when it doesn't answer.
    let (who_is, broadcasts) = split_who_is(broadcasts);
    assert_eq!(
        who_is,
        [crate::server::remote_write_discovery_tests::targeted(99)]
    );
    assert!(
        broadcasts.is_empty(),
        "a targeted device recipient must not be widened to a broadcast"
    );
    assert!(unicasts.is_empty());
}

/// Clause 6.3 reserves network-layer broadcast and multicast addressing for
/// unconfirmed requests: no other APDU type may be sent to such a destination.
///
/// A recipient asking for confirmed notifications at a broadcast address is
/// unsatisfiable — it is skipped rather than broadcast as a ConfirmedRequest
/// (which the previous code did for device recipients) or silently downgraded
/// to unconfirmed (which would drop the acknowledgment it asked for).
#[tokio::test]
async fn confirmed_notification_to_a_broadcast_recipient_is_skipped() {
    let (broadcasts, unicasts) =
        distribute_to(vec![destination_for(address_recipient(0, &[]), true)]).await;

    assert!(
        broadcasts.is_empty(),
        "a ConfirmedRequest may not use a broadcast address"
    );
    assert!(unicasts.is_empty());
}

/// The ordinary case still works: network 0 with a real MAC is a local unicast
/// addressed to exactly that MAC.
#[tokio::test]
async fn local_unicast_recipient_reaches_its_mac() {
    let mac = [127, 0, 0, 1, 0xBA, 0xC1];
    let (broadcasts, unicasts) =
        distribute_to(vec![destination_for(address_recipient(0, &mac), false)]).await;

    assert!(broadcasts.is_empty());
    assert_eq!(unicasts.len(), 1, "one unicast for one recipient");
    assert_eq!(unicasts[0].0, mac, "addressed to the configured MAC");
    assert_eq!(
        npdu_destination(&unicasts[0].1),
        None,
        "a local recipient carries no DNET"
    );
}

/// The confirmed path still delivers to a recipient that can actually receive
/// one. Guards the other side of the Clause 6.3 check: rejecting broadcast
/// destinations must not reject unicast ones.
#[tokio::test]
async fn confirmed_notification_to_a_local_unicast_recipient_is_sent() {
    let mac = [127, 0, 0, 1, 0xBA, 0xC1];
    let (broadcasts, unicasts) =
        distribute_to(vec![destination_for(address_recipient(0, &mac), true)]).await;

    assert!(broadcasts.is_empty());
    assert_eq!(unicasts.len(), 1, "confirmed notification reaches the wire");
    assert_eq!(unicasts[0].0, mac);
}

/// #124: an empty `Recipient_List` names no notification-clients, and Clause
/// 13.2.5 restricts distribution to the notification-clients in
/// Recipient_List. Broadcasting instead invented a destination the
/// configuration never named.
#[tokio::test]
async fn empty_recipient_list_distributes_nothing() {
    let (broadcasts, unicasts) = distribute_to(vec![]).await;

    assert!(
        broadcasts.is_empty(),
        "an empty Recipient_List is not permission to broadcast"
    );
    assert!(unicasts.is_empty());
}

/// Resolution is per-recipient: one undeliverable entry does not suppress the
/// deliverable ones alongside it.
#[tokio::test]
async fn one_unresolvable_recipient_does_not_suppress_the_others() {
    let mac = [127, 0, 0, 1, 0xBA, 0xC1];
    let (broadcasts, unicasts) = distribute_to(vec![
        destination_for(
            BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap()),
            false,
        ),
        destination_for(address_recipient(0, &mac), false),
        destination_for(address_recipient(0, &[]), false),
    ])
    .await;

    assert_eq!(
        unicasts.len(),
        1,
        "the local unicast recipient still gets it"
    );
    assert_eq!(unicasts[0].0, mac);
    // Device 99 is looked for with one Who-Is (#1368).
    let (who_is, broadcasts) = split_who_is(broadcasts);
    assert_eq!(who_is.len(), 1);
    assert_eq!(broadcasts.len(), 1, "the broadcast recipient still gets it");
}
