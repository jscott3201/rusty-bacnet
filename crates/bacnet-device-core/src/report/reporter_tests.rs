//! Unit tests of the [`Reporter`]: a request's notifications through the
//! outbox, confirmed delivery with its retries, room and invoke IDs running
//! out, and the Device's list through the overlay. The server parity tests
//! (`bacnet-server`, `device_core_parity_tests/cov`) hold the notifications
//! themselves to the server's.

use super::*;
use crate::report::{OutboxLimits, SERVICES};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::DeviceIdentity;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device_identity::build_database_with_extra;
use bacnet_services::cov::{COVNotificationRequest, SubscribeCOVRequest};
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_time::Duration;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier as P};
use bacnet_types::primitives::PropertyValue;

const T0: Instant = Instant::from_secs(1_000);

fn ai() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

fn peer() -> SubscriberEndpoint {
    SubscriberEndpoint::new(&[0x0A, 0, 0, 5, 0xBA, 0xC0], None)
}

/// Invoke IDs from 0 up, at most `limit` taken at once.
#[derive(Default)]
struct Ids {
    taken: Vec<(BACnetAddress, u8)>,
    next: u8,
    limit: usize,
}

impl InvokeIds for Ids {
    fn take(&mut self, to: &BACnetAddress) -> Option<u8> {
        if self.taken.len() >= self.limit {
            return None;
        }
        let id = self.next;
        self.next = self.next.wrapping_add(1);
        self.taken.push((to.clone(), id));
        Some(id)
    }
    fn release(&mut self, to: &BACnetAddress, id: u8) {
        let at = self
            .taken
            .iter()
            .position(|taken| taken.0 == *to && taken.1 == id)
            .expect("released once, as taken");
        self.taken.swap_remove(at);
    }
}

struct NoBindings;

impl Bindings for NoBindings {
    fn address_of(&self, _: ObjectIdentifier) -> Option<BACnetAddress> {
        None
    }
}

struct Device {
    reporter: Reporter,
    db: ObjectDatabase,
    ids: Ids,
    invoke_id: u8,
}

impl Device {
    fn new(outbox: OutboxLimits) -> Self {
        let identity = DeviceIdentity::new(9, 0x105)
            .unwrap()
            .with_services(SERVICES);
        let mut sensor = AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap();
        sensor.set_present_value(10.0);
        let db = build_database_with_extra(&identity, vec![Box::new(sensor)]).unwrap();
        let responder = Responder::new(identity, crate::responder::Limits::default()).unwrap();
        let config = ReportConfig::default()
            .with_outbox(outbox)
            .with_confirmed(FlightTiming::new(Duration::from_secs(3), 1));
        Self {
            reporter: Reporter::new(responder, config).unwrap(),
            db,
            ids: Ids {
                limit: 4,
                ..Ids::default()
            },
            invoke_id: 0,
        }
    }

    /// Hand `service` with `body` to the reporter; its reply.
    fn request(&mut self, service: ConfirmedServiceChoice, body: &[u8], now: Instant) -> Apdu {
        self.invoke_id += 1;
        let mut apdu = BytesMut::new();
        encode_apdu(
            &mut apdu,
            &Apdu::ConfirmedRequest(ConfirmedRequest {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: 1476,
                invoke_id: self.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: service,
                service_request: Bytes::copy_from_slice(body),
            }),
        )
        .unwrap();
        let mut out = BytesMut::new();
        let request = Inbound::new(apdu.freeze(), 1476);
        let written = self
            .reporter
            .handle(&mut self.db, &peer(), request, now, &mut out);
        assert_eq!(written.outcome, Outcome::Reply);
        decode_apdu(out.freeze()).unwrap()
    }

    fn subscribe(&mut self, process: u32, confirmed: bool, now: Instant) {
        let mut body = BytesMut::new();
        SubscribeCOVRequest {
            subscriber_process_identifier: process,
            monitored_object_identifier: ai(),
            issue_confirmed_notifications: Some(confirmed),
            lifetime: Some(300),
        }
        .encode(&mut body)
        .unwrap();
        let reply = self.request(ConfirmedServiceChoice::SUBSCRIBE_COV, &body, now);
        assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    }

    fn write(&mut self, property: P, value: PropertyValue, now: Instant) {
        let mut encoded = BytesMut::new();
        encode_property_value(&mut encoded, &value).unwrap();
        let mut body = BytesMut::new();
        WritePropertyRequest {
            object_identifier: ai(),
            property_identifier: property,
            property_array_index: None,
            property_value: encoded.to_vec(),
            priority: None,
        }
        .encode(&mut body)
        .unwrap();
        let reply = self.request(ConfirmedServiceChoice::WRITE_PROPERTY, &body, now);
        assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    }

    /// The next notification and its APDU.
    fn pop(&mut self, now: Instant) -> Option<(Notification, Apdu)> {
        let mut out = BytesMut::new();
        let notification = self
            .reporter
            .pop(&NoBindings, &mut self.ids, now, &mut out)?;
        Some((notification, decode_apdu(out.freeze()).unwrap()))
    }

    /// The Present_Value a notification carries.
    fn reported_pv(apdu: &Apdu) -> PropertyValue {
        let params = match apdu {
            Apdu::UnconfirmedRequest(request) => &request.service_request,
            Apdu::ConfirmedRequest(request) => &request.service_request,
            other => panic!("not a notification: {other:?}"),
        };
        let values = COVNotificationRequest::decode(params)
            .unwrap()
            .list_of_values;
        let (value, _) =
            bacnet_encoding::primitives::decode_application_value(&values[0].value, 0).unwrap();
        value
    }

    fn ack(&mut self, invoke_id: u8, now: Instant) -> bool {
        let mut apdu = BytesMut::new();
        encode_apdu(
            &mut apdu,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            }),
        )
        .unwrap();
        self.reporter
            .reply(&peer_address(&peer()), &apdu, &mut self.ids, now)
    }

    fn deadline(&mut self) {
        let now = self.reporter.next_deadline().expect("something due");
        self.reporter.on_deadline(&self.db, &mut self.ids, now);
    }
}

#[test]
fn a_subscription_and_a_write_report_through_the_outbox() {
    let mut device = Device::new(OutboxLimits::default());
    device.subscribe(1, false, T0);
    let (initial, apdu) = device.pop(T0).expect("the first notification");
    assert_eq!(initial.invoke_id, None);
    assert_eq!(initial.to, Destination::Subscriber(peer()));
    assert_eq!(Device::reported_pv(&apdu), PropertyValue::Real(10.0));
    assert!(device.pop(T0).is_none());
    device
        .reporter
        .delivered(initial.token, Delivery::Sent, &mut device.ids, T0);

    // Out of service changes Status_Flags; the value then follows the write.
    device.write(P::OUT_OF_SERVICE, PropertyValue::Boolean(true), T0);
    let (flags, _) = device.pop(T0).expect("Status_Flags changed");
    device
        .reporter
        .delivered(flags.token, Delivery::Sent, &mut device.ids, T0);
    device.write(P::PRESENT_VALUE, PropertyValue::Real(12.5), T0);
    let (_, apdu) = device.pop(T0).expect("the value changed");
    assert_eq!(Device::reported_pv(&apdu), PropertyValue::Real(12.5));
    let counters = device.reporter.counters();
    assert_eq!(counters.cov.notifications_unconfirmed, 3);
    assert_eq!(counters.cov.subscriptions_active, 1);
}

#[test]
fn a_confirmed_notification_is_acknowledged_resent_and_given_up() {
    let mut device = Device::new(OutboxLimits::default());
    device.subscribe(1, true, T0);
    let (initial, _) = device.pop(T0).unwrap();
    let id = initial.invoke_id.expect("confirmed");
    assert!(!device.ack(id + 1, T0), "not its invoke ID");
    assert!(device.ack(id, T0));
    assert!(device.ids.taken.is_empty(), "the invoke ID went back");
    assert!(!device.ack(id, T0), "answered once");

    // Unanswered: one resend under the same invoke ID, then it fails.
    device.write(P::OUT_OF_SERVICE, PropertyValue::Boolean(true), T0);
    device.deadline();
    let (sent, _) = device.pop(T0).unwrap();
    let id = sent.invoke_id.unwrap();
    assert_eq!(
        device.reporter.next_deadline(),
        Some(T0 + Duration::from_secs(3))
    );
    device.deadline();
    let (again, _) = device.pop(T0 + Duration::from_secs(3)).unwrap();
    assert_eq!((again.token, again.invoke_id), (sent.token, Some(id)));
    assert_eq!(device.reporter.counters().resent, 1);
    device.deadline();
    assert_eq!(device.reporter.counters().failed, 1);
    assert!(device.ids.taken.is_empty());
    // The hold-off: a change meanwhile isn't reported.
    let later = T0 + Duration::from_secs(7);
    device.write(P::PRESENT_VALUE, PropertyValue::Real(30.0), later);
    assert!(device.pop(later).is_none());
}

#[test]
fn a_change_held_by_an_outstanding_report_follows_its_ack() {
    let mut device = Device::new(OutboxLimits::default());
    device.subscribe(1, true, T0);
    let (initial, _) = device.pop(T0).unwrap();
    device.write(P::OUT_OF_SERVICE, PropertyValue::Boolean(true), T0);
    assert!(device.pop(T0).is_none(), "held behind the first");
    assert!(device.ack(initial.invoke_id.unwrap(), T0));
    assert_eq!(device.reporter.next_deadline(), Some(T0), "a look is owed");
    device.deadline();
    assert!(device.pop(T0).is_some(), "the held change");
}

#[test]
fn a_report_without_room_is_owed_until_the_outbox_empties() {
    let mut device = Device::new(OutboxLimits {
        entries: 1,
        octets: 256,
    });
    device.subscribe(1, false, T0);
    device.subscribe(2, false, T0);
    assert_eq!(device.reporter.counters().owed, 1);
    assert!(
        device.reporter.next_deadline().is_none_or(|due| due > T0),
        "nothing to do while the outbox is full"
    );
    let (first, _) = device.pop(T0).unwrap();
    device
        .reporter
        .delivered(first.token, Delivery::Sent, &mut device.ids, T0);
    device.deadline();
    let (second, apdu) = device.pop(T0).expect("the owed one");
    assert_ne!(first.token, second.token);
    assert_eq!(Device::reported_pv(&apdu), PropertyValue::Real(10.0));
}

#[test]
fn a_confirmed_report_with_no_invoke_id_ends_unsent_and_is_owed() {
    let mut device = Device::new(OutboxLimits::default());
    device.ids.limit = 0;
    device.subscribe(1, true, T0);
    assert!(device.pop(T0).is_none());
    assert_eq!(device.reporter.counters().cov.notifications_sent, 0);
    device.ids.limit = 1;
    device.deadline();
    assert!(device.pop(T0).unwrap().0.invoke_id.is_some());
}

#[test]
fn reads_answer_the_device_list_from_the_table() {
    let mut device = Device::new(OutboxLimits::default());
    let read = |device: &mut Device| {
        let mut body = BytesMut::new();
        ReadPropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap(),
            property_identifier: P::ACTIVE_COV_SUBSCRIPTIONS,
            property_array_index: None,
        }
        .encode(&mut body);
        match device.request(ConfirmedServiceChoice::READ_PROPERTY, &body, T0) {
            Apdu::ComplexAck(ack) => ack.service_ack.len(),
            other => panic!("{other:?}"),
        }
    };
    let empty = read(&mut device);
    device.subscribe(1, false, T0);
    assert!(read(&mut device) > empty, "the subscription is listed");
}
