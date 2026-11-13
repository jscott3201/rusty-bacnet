//! Unit tests of the [`Reporter`]: a request's notifications through the
//! outbox, confirmed delivery with its retries, room and invoke IDs running
//! out, the Device's list through the overlay, and a deleted object's
//! reports. The server parity tests (`bacnet-server`,
//! `device_core_parity_tests/cov`) hold the notifications themselves to
//! the server's.

use super::*;
use crate::report::{Bindings, Destination, Notification, OutboxLimits, SERVICES, peer_address};
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

#[path = "reporter_change_back_tests.rs"]
mod change_back;
#[cfg(feature = "event-reporting")]
#[path = "reporter_events_tests.rs"]
mod events;
#[path = "reporter_local_tests.rs"]
mod local;
#[cfg(feature = "cov-multiple")]
#[path = "reporter_multiple_tests.rs"]
mod multiple;
#[path = "reporter_owed_tests.rs"]
mod owed;
#[path = "reporter_remove_tests.rs"]
mod remove;

const T0: Instant = Instant::from_secs(1_000);

fn ai() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

fn ai2() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap()
}

fn peer() -> SubscriberEndpoint {
    SubscriberEndpoint::new(&[0x0A, 0, 0, 5, 0xBA, 0xC0], None)
}

/// Invoke IDs from 0 up, at most `limit` taken at once, noting the service
/// each was asked for.
#[derive(Default)]
struct Ids {
    taken: Vec<(BACnetAddress, u8)>,
    next: u8,
    limit: usize,
    services: Vec<ConfirmedServiceChoice>,
}

impl InvokeIds for Ids {
    fn take(&mut self, to: &BACnetAddress, service: ConfirmedServiceChoice) -> Option<u8> {
        self.services.push(service);
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
        Self::with_policy(outbox, crate::cov::CovPolicy::default())
    }

    fn with_policy(outbox: OutboxLimits, policy: crate::cov::CovPolicy) -> Self {
        let identity = DeviceIdentity::new(9, 0x105)
            .unwrap()
            .with_services(SERVICES);
        let mut sensor = AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap();
        sensor.set_present_value(10.0);
        let second = AnalogInputObject::new(2, "AI-2", EngineeringUnits::NO_UNITS).unwrap();
        let sensors: Vec<Box<dyn bacnet_objects::traits::BACnetObject>> =
            vec![Box::new(sensor), Box::new(second)];
        let db = build_database_with_extra(&identity, sensors).unwrap();
        let responder = Responder::new(identity, crate::responder::Limits::default()).unwrap();
        let config = ReportConfig::new(policy)
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
        self.subscribe_to(ai(), process, confirmed, now);
    }

    fn subscribe_to(
        &mut self,
        object: ObjectIdentifier,
        process: u32,
        confirmed: bool,
        now: Instant,
    ) {
        let mut body = BytesMut::new();
        SubscribeCOVRequest {
            subscriber_process_identifier: process,
            monitored_object_identifier: object,
            issue_confirmed_notifications: Some(confirmed),
            lifetime: Some(300),
        }
        .encode(&mut body)
        .unwrap();
        let reply = self.request(ConfirmedServiceChoice::SUBSCRIBE_COV, &body, now);
        assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    }

    fn write(&mut self, property: P, value: PropertyValue, now: Instant) {
        self.write_to(ai(), property, value, now);
    }

    fn write_to(
        &mut self,
        object: ObjectIdentifier,
        property: P,
        value: PropertyValue,
        now: Instant,
    ) {
        let mut encoded = BytesMut::new();
        encode_property_value(&mut encoded, &value).unwrap();
        let mut body = BytesMut::new();
        WritePropertyRequest {
            object_identifier: object,
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
        self.reporter.on_deadline(&mut self.db, &mut self.ids, now);
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
fn a_report_without_room_is_owed_until_room_frees() {
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

/// The host lends a confirmed COV report its invoke ID for the service it
/// goes out as (#2303).
#[test]
fn a_confirmed_report_names_its_service_to_the_invoke_ids() {
    let mut device = Device::new(OutboxLimits::default());
    device.subscribe(1, true, T0);
    let (initial, _) = device.pop(T0).unwrap();
    assert!(initial.invoke_id.is_some());
    assert_eq!(
        device.ids.services,
        [ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION]
    );
}

#[test]
fn a_confirmed_report_with_no_invoke_id_ends_unsent_and_is_owed() {
    let mut device = Device::new(OutboxLimits::default());
    device.ids.limit = 0;
    device.subscribe(1, true, T0);
    assert!(device.pop(T0).is_none());
    assert_eq!(device.reporter.counters().cov.notifications_sent, 0);
    // Owed for an ID, it isn't due again until one comes back (#2512).
    assert!(device.reporter.next_deadline().is_none_or(|due| due > T0));
    device.ids.limit = 1;
    device.reporter.invoke_ids_released(1);
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

/// The Reporter runs no SubscribeCOVPropertyMultiple (#2320): it rejects
/// the request as an unrecognized service, and subscribes nothing.
// With `cov-multiple` the reporter serves it: `reporter_multiple_tests`.
#[cfg(not(feature = "cov-multiple"))]
#[test]
fn subscribe_cov_property_multiple_is_rejected() {
    use bacnet_services::cov_multiple::{
        COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
    };
    use bacnet_types::constructed::PropertyReference;
    use bacnet_types::enums::RejectReason;

    let mut device = Device::new(OutboxLimits::default());
    let mut body = BytesMut::new();
    SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: false,
        lifetime: Some(300),
        max_notification_delay: Some(10),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: ai(),
            list_of_cov_references: vec![COVReference {
                monitored_property: PropertyReference {
                    property_identifier: P::PRESENT_VALUE,
                    property_array_index: None,
                },
                cov_increment: None,
                timestamped: false,
            }],
        }],
    }
    .encode(&mut body)
    .unwrap();
    let service = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE;
    match device.request(service, &body, T0) {
        Apdu::Reject(reject) => {
            assert_eq!(reject.reject_reason, RejectReason::UNRECOGNIZED_SERVICE);
        }
        other => panic!("{other:?}"),
    }
    assert!(device.reporter.table().is_empty());
    assert!(device.pop(T0).is_none());
}

const fn at(millis: u64) -> Instant {
    Instant::from_secs(1_000).saturating_add(Duration::from_millis(millis))
}

/// The process a notification reports to, and the Present_Value it carries.
fn reported(apdu: &Apdu) -> (u32, PropertyValue) {
    let params = match apdu {
        Apdu::UnconfirmedRequest(request) => &request.service_request,
        Apdu::ConfirmedRequest(request) => &request.service_request,
        other => panic!("not a notification: {other:?}"),
    };
    let process = COVNotificationRequest::decode(params)
        .unwrap()
        .subscriber_process_identifier;
    (process, Device::reported_pv(apdu))
}

/// Timing 3 s with one retry: a report's cutoff is 7.5 s after the table
/// began it, its deadline 9 s.
#[test]
fn a_report_popped_late_is_cut_off_before_its_flight_ends() {
    let mut device = Device::new(OutboxLimits::default());
    device.subscribe(1, true, at(0));
    // The link is busy: the first attempt goes out 4 s after evaluation.
    let (sent, _) = device.pop(at(4_000)).unwrap();
    let id = sent.invoke_id.unwrap();
    assert_eq!(device.reporter.next_deadline(), Some(at(7_000)));
    device.deadline();
    let (again, _) = device.pop(at(7_000)).unwrap();
    assert_eq!(again.invoke_id, Some(id));
    assert_eq!(
        device.reporter.next_deadline(),
        Some(at(7_500)),
        "the cutoff, before the attempt's 10 s and the table's 9 s"
    );
    device.deadline();
    assert_eq!(device.reporter.counters().failed, 1);
    assert!(device.ids.taken.is_empty(), "its invoke ID went back");
    // An answer inside the last attempt's window but past the cutoff ends
    // nothing, and nothing goes out again: the subscription holds off.
    assert!(!device.ack(id, at(9_500)));
    assert!(device.pop(at(9_600)).is_none());
    assert_eq!(device.reporter.counters().cov.notifications_sent, 1);
}

#[test]
fn an_answer_before_the_cutoff_commits_a_late_report() {
    let mut device = Device::new(OutboxLimits::default());
    device.subscribe(1, true, at(0));
    let (sent, _) = device.pop(at(4_000)).unwrap();
    device.deadline();
    device.pop(at(7_000)).unwrap();
    assert!(device.ack(sent.invoke_id.unwrap(), at(7_200)));
    let baseline = device
        .reporter
        .table()
        .entries()
        .next()
        .unwrap()
        .last_notified_observation
        .clone();
    assert!(baseline.is_some(), "the Ack committed what it carried");
    assert_eq!(device.reporter.counters().failed, 0);
}

#[test]
fn a_report_popped_past_its_cutoff_fails_unsent() {
    let mut device = Device::new(OutboxLimits::default());
    device.subscribe(1, true, at(0));
    assert!(device.pop(at(8_000)).is_none());
    let counters = device.reporter.counters();
    assert_eq!((counters.failed, counters.cov.notifications_sent), (1, 0));
    assert!(device.ids.taken.is_empty());
}

/// One subscription changes twice for each notification the link sends;
/// the other's one change still goes out, and the hot one never holds more
/// than one queued entry, carrying its latest value.
#[test]
fn a_hot_subscription_holds_one_entry_and_others_still_report() {
    let mut device = Device::new(OutboxLimits {
        entries: 2,
        octets: 256,
    });
    device.subscribe_to(ai(), 1, false, at(0));
    device.subscribe_to(ai2(), 2, false, at(0));
    for object in [ai(), ai2()] {
        device.write_to(
            object,
            P::OUT_OF_SERVICE,
            PropertyValue::Boolean(true),
            at(0),
        );
    }
    while let Some((sent, _)) = device.pop(at(0)) {
        device
            .reporter
            .delivered(sent.token, Delivery::Sent, &mut device.ids, at(0));
    }
    let hot = device
        .reporter
        .table()
        .entries()
        .find(|entry| entry.subscriber_process_identifier == 1)
        .unwrap()
        .key()
        .clone();
    let mut other = None;
    let mut last_hot = None;
    for tick in 1..=8u64 {
        let now = at(tick * 1_000);
        for step in 0..2u64 {
            let value = (100 + tick * 2 + step) as f32;
            device.write_to(ai(), P::PRESENT_VALUE, PropertyValue::Real(value), now);
            assert!(device.reporter.outbox.live_for(&hot) <= 1);
        }
        if tick == 2 {
            device.write_to(ai2(), P::PRESENT_VALUE, PropertyValue::Real(50.0), now);
        }
        if device
            .reporter
            .next_deadline()
            .is_some_and(|due| due <= now)
        {
            device
                .reporter
                .on_deadline(&mut device.db, &mut device.ids, now);
        }
        // The link sends one notification a tick.
        if let Some((sent, apdu)) = device.pop(now) {
            device
                .reporter
                .delivered(sent.token, Delivery::Sent, &mut device.ids, now);
            match reported(&apdu) {
                (2, value) => other = Some((tick, value)),
                (_, value) => last_hot = Some(value),
            }
        }
    }
    assert_eq!(other, Some((3, PropertyValue::Real(50.0))), "its turn came");
    assert_eq!(
        last_hot,
        Some(PropertyValue::Real(117.0)),
        "the hot one's latest value at its turn"
    );
}

/// Room for one notification and three subscriptions owed: each popped
/// entry frees room for one, so a deadline looks at the oldest owed
/// reference only, and a reference refused again meanwhile isn't counted
/// again.
#[test]
fn a_freed_entry_looks_at_one_owed_reference() {
    let mut device = Device::new(OutboxLimits {
        entries: 1,
        octets: 256,
    });
    device.write(P::OUT_OF_SERVICE, PropertyValue::Boolean(true), T0);
    for process in 1..=4 {
        device.subscribe(process, false, T0);
    }
    assert_eq!(device.reporter.counters().owed, 3);
    // 1's queued report takes the change; 2, 3 and 4 are refused again.
    device.write(P::PRESENT_VALUE, PropertyValue::Real(20.0), T0);
    assert_eq!(device.reporter.counters().owed, 3, "owed once each");
    let mut order = Vec::new();
    while let Some((sent, apdu)) = device.pop(T0) {
        order.push(reported(&apdu).0);
        device
            .reporter
            .delivered(sent.token, Delivery::Sent, &mut device.ids, T0);
        let looked = device.reporter.looked;
        if device.reporter.next_deadline() == Some(T0) {
            device.deadline();
        }
        assert!(
            device.reporter.looked - looked <= 1,
            "one free entry, one look"
        );
    }
    assert_eq!(order, [1, 2, 3, 4]);
    assert_eq!(device.reporter.counters().owed, 3);
    assert!(device.reporter.owed.is_empty());
}

/// Four subscribers to one object, room for one notification, and the
/// value changing at every send: each takes its turn, the oldest owed
/// first. In key order the two lowest would share every freed entry.
#[test]
fn owed_references_take_freed_room_oldest_first() {
    let mut device = Device::new(OutboxLimits {
        entries: 1,
        octets: 256,
    });
    device.write(P::OUT_OF_SERVICE, PropertyValue::Boolean(true), at(0));
    for process in [1, 4, 3, 2] {
        device.subscribe(process, false, at(0));
    }
    let mut order = Vec::new();
    for tick in 1..=8u64 {
        let now = at(tick * 1_000);
        let (sent, apdu) = device.pop(now).expect("one queued each tick");
        device
            .reporter
            .delivered(sent.token, Delivery::Sent, &mut device.ids, now);
        order.push(reported(&apdu).0);
        if device
            .reporter
            .next_deadline()
            .is_some_and(|due| due <= now)
        {
            device
                .reporter
                .on_deadline(&mut device.db, &mut device.ids, now);
        }
        let value = PropertyValue::Real(100.0 + tick as f32);
        device.write(P::PRESENT_VALUE, value, now);
    }
    assert_eq!(order, [1, 4, 3, 2, 1, 4, 3, 2]);
}
