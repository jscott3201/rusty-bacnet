//! An event sent to a Device recipient (#2525): a Notification Class
//! destination that names a Device object, not an address.
//!
//! Each side resolves the Device from what an I-Am told it: the server's
//! binding table records the I-Am it receives, and the core's `Bindings`,
//! which a firmware host backs with a Who-Is/I-Am cache, is given the
//! same address from the same I-Am. With the binding made first, both send
//! the same notification, octet for octet, to the address the I-Am came
//! from. Without it the two differ by design: the server asks for the
//! Device (one Who-Is limited to its instance) and holds the notification
//! for the I-Am; the core, which has no binding wait of its own (design
//! §1, non-goals), counts the notification unresolved and drops it.
use super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::constructed::decode_event_notification;
use bacnet_services::who_is::{IAmRequest, WhoIsRequest};
use bacnet_types::enums::NotifyType;

/// The Device the Notification Class's destination names.
const DEVICE_INSTANCE: u32 = 77;

/// The destination's process identifier.
const PROCESS: u32 = 9;

fn bound_device() -> ObjectIdentifier {
    oid(ObjectType::DEVICE, DEVICE_INSTANCE)
}

/// Notification Class 1's one destination: Device 77, open all day, every
/// transition, unconfirmed.
fn device_recipient() -> BACnetDestination {
    let all_day = Time {
        hour: 23,
        minute: 59,
        second: 59,
        hundredths: 99,
    };
    BACnetDestination {
        valid_days: DaysOfWeek::all(),
        from_time: at(0, 0),
        to_time: all_day,
        recipient: BACnetRecipient::Device(bound_device()),
        process_identifier: PROCESS,
        issue_confirmed_notifications: false,
        transitions: EventTransitionBits::all(),
    }
}

/// An Analog Value alarming over 80 with no Time_Delay, reporting to the
/// Device recipient.
fn to_a_device(identity: &bacnet_objects::DeviceIdentity) -> ObjectDatabase {
    database_with(identity, alarm_value(0, vec![device_recipient()]))
}

/// The service parameters of an event notification's APDU.
pub(super) fn parameters(apdu: &[u8]) -> bacnet_types::constructed::EventNotificationRequest {
    match decode_apdu(Bytes::copy_from_slice(apdu)).unwrap() {
        Apdu::UnconfirmedRequest(request) => {
            decode_event_notification(&request.service_request).unwrap()
        }
        Apdu::ConfirmedRequest(request) => {
            decode_event_notification(&request.service_request).unwrap()
        }
        other => panic!("not an event notification: {other:?}"),
    }
}

impl EventParity {
    /// Device 77's I-Am, from a source of its own, to the server, and to
    /// the core's bindings as its host's cache records it, at `millis`
    /// after the start; the address it came from, and what the server sent
    /// at once.
    async fn i_am(&mut self, millis: u64) -> (BACnetAddress, Vec<test_transport::SentFrame>) {
        let at = self.instant(millis);
        self.sleep_until(at).await;
        let mut body = BytesMut::new();
        IAmRequest {
            object_identifier: bound_device(),
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NO_SEGMENTATION,
            vendor_id: 5,
        }
        .encode(&mut body);
        let frames = self
            .parity
            .deliver(&unconfirmed(UnconfirmedServiceChoice::I_AM, body))
            .await;
        let from = BACnetAddress {
            network_number: 0,
            mac_address: self.parity.source(),
        };
        self.bindings.0.push((bound_device(), from.clone()));
        (from, frames)
    }
}

/// The notification goes to the address the I-Am came from, from both,
/// octet for octet, and again for the transition back; a notification of a
/// transition made before the I-Am would not.
#[tokio::test(start_paused = true)]
async fn an_event_to_a_bound_device_recipient_matches() {
    let mut both = EventParity::start(to_a_device).await;
    let (from, frames) = both.i_am(100).await;
    assert!(frames.is_empty(), "an I-Am draws no reply: {frames:?}");
    assert_eq!(both.value("over the limit", 300, 90.0).await, 1);
    let sent = std::mem::take(&mut both.compared);
    assert_eq!(sent.len(), 1);
    let (mac, priority, apdu) = &sent[0];
    assert_eq!(
        mac.as_slice(),
        from.mac_address.as_slice(),
        "to the I-Am's source"
    );
    assert_eq!(*priority, NetworkPriority::CriticalEquipment);
    let notification = parameters(apdu);
    assert_eq!(notification.process_identifier, PROCESS);
    assert_eq!(notification.to_state, EventState::HIGH_LIMIT);
    assert_eq!(notification.notify_type, NotifyType::ALARM);
    assert_eq!(both.parity.reporter.counters().unresolved, 0);
    // Back to normal after Time_Delay_Normal: the binding is still there.
    assert_eq!(both.value("back under", 4_500, 50.0).await, 0);
    assert_eq!(both.run_to("back to normal", 5_550).await, 1);
    let back = std::mem::take(&mut both.compared);
    assert_eq!(parameters(&back[0].2).to_state, EventState::NORMAL);
    assert_eq!(back[0].0.as_slice(), from.mac_address.as_slice());
}

/// With no binding yet the sides differ, as pinned here: the server sends
/// one Who-Is for the Device and holds the notification until the I-Am,
/// then sends it; the core drops it, counted, and a later transition goes
/// out from both once the binding exists.
#[tokio::test(start_paused = true)]
async fn before_the_binding_the_server_asks_and_waits_and_the_core_drops() {
    let mut both = EventParity::start(to_a_device).await;
    let now = both.instant(300);
    both.sleep_until(now).await;
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, &PropertyValue::Real(90.0)).unwrap();
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: av(),
        property_identifier: P::PRESENT_VALUE,
        property_array_index: None,
        property_value: encoded.to_vec(),
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    let service = ConfirmedServiceChoice::WRITE_PROPERTY;
    let apdu = confirmed(1, service, &body, 1476);
    let mut frames = both.parity.deliver(&apdu).await;
    let source = both.parity.source();
    let at = frames
        .iter()
        .position(|frame| frame.mac.as_slice() == source.as_slice())
        .expect("the server's reply");
    let server_reply = frames.remove(at).decode_npdu().payload;
    let peer = SubscriberEndpoint::new(&source, None);
    let request = Inbound::new(apdu, 1476).with_source(both.parity.origin());
    let mut out = BytesMut::new();
    let parity = &mut both.parity;
    let handled = parity
        .reporter
        .handle(&mut parity.db, &peer, request, now, &mut out);
    assert_eq!(handled.outcome, Outcome::Reply);
    assert_eq!(server_reply, out.freeze(), "the reply, server then core");
    // The server: one broadcast, the Who-Is limited to Device 77, and no
    // notification.
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert!(frames[0].broadcast, "the Who-Is is a broadcast");
    let Apdu::UnconfirmedRequest(who_is) = decode_apdu(frames[0].decode_npdu().payload).unwrap()
    else {
        panic!("not an unconfirmed request")
    };
    assert_eq!(who_is.service_choice, UnconfirmedServiceChoice::WHO_IS);
    let who_is = WhoIsRequest::decode(&who_is.service_request).unwrap();
    let range = who_is.range.expect("limited to one instance");
    assert_eq!(
        (range.low(), range.high()),
        (DEVICE_INSTANCE, DEVICE_INSTANCE)
    );
    // The core: nothing to send, the notification counted unresolved.
    both.settle(now);
    assert!(both.core_sent(now).is_empty());
    assert_eq!(both.parity.reporter.counters().unresolved, 1);
    // The I-Am: the server sends what it held; the core has nothing to.
    let (from, frames) = both.i_am(1_000).await;
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert_eq!(frames[0].mac.as_slice(), from.mac_address.as_slice());
    let held = parameters(&frames[0].decode_npdu().payload);
    assert_eq!(held.to_state, EventState::HIGH_LIMIT);
    assert_eq!(held.process_identifier, PROCESS);
    both.settle(both.instant(1_000));
    assert!(both.core_sent(both.instant(1_000)).is_empty());
    // Bound on both, the next transition goes out alike.
    assert_eq!(both.value("back under", 4_500, 50.0).await, 0);
    assert_eq!(both.run_to("back to normal", 5_550).await, 1);
}
