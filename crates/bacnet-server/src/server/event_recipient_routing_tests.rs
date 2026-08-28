//! How a `Recipient_List` entry becomes a network destination.
//!
//! Clause 21's `BACnetAddress` carries two independent fields — `network-number`
//! ("A value of 0 indicates the local network") and `mac-address` ("A string of
//! length 0 indicates a broadcast") — so a recipient names one of four
//! destinations, not one unicast with edge cases. These tests pin each one, plus
//! the two that cannot be resolved at all.
//!
//! Split from `event_notifications_tests.rs`, which is at the 700-LOC cap.

use super::event_notifications_tests::local_broadcast_destination;
use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event::{EventStateChange, EventTransition};
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{BACnetAddress, BACnetDestination, BACnetRecipient};
use bacnet_types::enums::{EventState, EventType};
use bytes::Bytes;
use std::borrow::Cow;
use std::sync::{Arc as StdArc, Mutex as StdMutex};
use tokio::sync::mpsc;

/// Records broadcasts and unicasts separately, keeping each unicast's target
/// MAC. The MAC matters: a recipient on a remote network that is delivered as a
/// local unicast reaches whichever device happens to hold that MAC on this link,
/// which is the failure this module exists to catch.
#[derive(Clone, Default)]
struct RoutingTransport {
    broadcasts: StdArc<StdMutex<Vec<Bytes>>>,
    unicasts: StdArc<StdMutex<Vec<(Vec<u8>, Bytes)>>>,
}

impl TransportPort for RoutingTransport {
    async fn start(
        &mut self,
    ) -> Result<mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error> {
        let (_tx, rx) = mpsc::channel(1);
        Ok(rx)
    }
    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.unicasts
            .lock()
            .unwrap()
            .push((mac.to_vec(), Bytes::copy_from_slice(npdu)));
        Ok(())
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.broadcasts
            .lock()
            .unwrap()
            .push(Bytes::copy_from_slice(npdu));
        Ok(())
    }
    fn local_mac(&self) -> &[u8] {
        &[127, 0, 0, 1, 0xBA, 0xC0]
    }
    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        mac == LITERAL_BROADCAST_MAC
    }
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

pub(super) fn address_recipient(network_number: u16, mac: &[u8]) -> BACnetRecipient {
    BACnetRecipient::Address(BACnetAddress {
        network_number,
        mac_address: MacAddr::from_slice(mac),
    })
}

/// Fire one TO_OFFNORMAL transition at an AnalogInput whose Notification Class
/// holds exactly `destinations`, and return what reached the transport.
async fn distribute_to(
    destinations: Vec<BACnetDestination>,
) -> (Vec<Bytes>, Vec<(Vec<u8>, Bytes)>) {
    distribute_with_priority([255, 255, 255], destinations).await
}

/// [`distribute_to`] with the Notification Class's per-transition Priority
/// array set, for tests that pin the Table 13-6 NPDU-priority projection.
pub(super) async fn distribute_with_priority(
    priority: [u8; 3],
    destinations: Vec<BACnetDestination>,
) -> (Vec<Bytes>, Vec<(Vec<u8>, Bytes)>) {
    distribute_with_clock_mode(priority, destinations, true).await
}

async fn distribute_with_clock_mode(
    priority: [u8; 3],
    destinations: Vec<BACnetDestination>,
    clocked: bool,
) -> (Vec<Bytes>, Vec<(Vec<u8>, Bytes)>) {
    let mut db = if clocked {
        clocked_test_database()
    } else {
        ObjectDatabase::new()
    };
    let mut nc = NotificationClass::new(0, "NC-0").unwrap();
    nc.priority = priority;
    for destination in destinations {
        nc.add_destination(destination);
    }
    db.add(Box::new(nc)).unwrap();
    distribute_from_database(db).await
}

async fn distribute_from_database(mut db: ObjectDatabase) -> (Vec<Bytes>, Vec<(Vec<u8>, Bytes)>) {
    let transport = RoutingTransport::default();
    let broadcasts = StdArc::clone(&transport.broadcasts);
    let unicasts = StdArc::clone(&transport.unicasts);
    let network = Arc::new(NetworkLayer::new(transport));
    let comm_state = Arc::new(AtomicU8::new(0));
    let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));

    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 1,
            name: "Dev".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let mut ai = AnalogInputObject::new(1, "AI-1", 0).unwrap();
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
    BACnetServer::<RoutingTransport>::build_and_send_event_notification(
        &db,
        &network,
        &comm_state,
        &server_tsm,
        &NotificationTransactions::new(),
        &oid,
        (
            EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            EventType::OUT_OF_RANGE,
        ),
        1000,
    )
    .await;

    // The confirmed path spawns its send, so a test that sampled the transport
    // straight after the await would pass whether or not anything was sent.
    // Yield until the spawned task has reached its first send.
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }

    let broadcasts = broadcasts.lock().unwrap().clone();
    let unicasts = unicasts.lock().unwrap().clone();
    (broadcasts, unicasts)
}

enum TestRecipientList {
    Unavailable,
    Invalid,
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
                TestRecipientList::Unavailable => Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
                }),
                TestRecipientList::Invalid => Ok(PropertyValue::ApplicationData(vec![0x5E])),
            },
            _ => Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            }),
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::RECIPIENT_LIST,
        ])
    }
}

async fn distribute_non_matched_case(case: &str) -> (Vec<Bytes>, Vec<(Vec<u8>, Bytes)>) {
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
        "empty-list" => db
            .add(Box::new(NotificationClass::new(0, "NC-0").unwrap()))
            .unwrap(),
        "no-eligible-destination" => {
            let mut nc = NotificationClass::new(0, "NC-0").unwrap();
            let mut destination = destination_for(address_recipient(0, &[]), false);
            destination.transitions = EventTransition::ToNormal.bit_mask();
            nc.add_destination(destination);
            db.add(Box::new(nc)).unwrap();
        }
        _ => unreachable!("test case is fixed"),
    }
    distribute_from_database(db).await
}

#[tokio::test]
async fn every_non_matched_lookup_outcome_emits_no_frame() {
    for case in [
        "missing-class",
        "list-unavailable",
        "list-invalid",
        "empty-list",
        "no-eligible-destination",
    ] {
        let (broadcasts, unicasts) = distribute_non_matched_case(case).await;
        assert!(broadcasts.is_empty(), "{case} must not widen to broadcast");
        assert!(unicasts.is_empty(), "{case} must not emit a unicast");
    }
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
/// to ship exactly one entry, "with the Recipient set to a local broadcast".
/// Clause 21 spells a local broadcast as network 0 with a zero-length MAC.
///
/// This previously reached `send_apdu` with an empty MAC, which BACnet/IP
/// rejects ("BIP MAC must be 6 bytes, got 0"), so the one configuration the
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
/// Clause 6.3: "DNET shall specify the network number of the remote network and
/// DLEN shall be set to zero."
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
/// network that happens to be numbered 65535. Clause 6.3: "A global broadcast,
/// indicated by a DNET of X'FFFF', is sent to all networks through all routers."
///
/// It needs its own send: `NetworkLayer::broadcast_to_network` rejects 0xFFFF
/// ("reserved for global broadcasts; use broadcast_global_apdu instead"), so
/// routing it as an ordinary remote broadcast turns the notification into a
/// logged send error and delivers nothing.
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
/// 6.5.3's form for when "the address of the router is initially unknown"
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

    assert!(
        broadcasts.is_empty(),
        "a targeted device recipient must not be widened to a broadcast"
    );
    assert!(unicasts.is_empty());
}

/// Clause 6.3: "Of the BACnet APDUs, only the BACnet-Unconfirmed-Request-PDU
/// may be transmitted using a multicast or broadcast network layer address."
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
/// 13.2.5 distributes "to the notification-clients specified by the
/// Recipient_List input". Broadcasting instead invented a destination the
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
    assert_eq!(broadcasts.len(), 1, "the broadcast recipient still gets it");
}
