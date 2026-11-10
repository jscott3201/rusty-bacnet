//! The [`Reporter`] running SubscribeCOVPropertyMultiple: its reports
//! through the outbox in both forms, a report in several parts, a part that
//! is not sent, the send turn, the Max_Notification_Delay deadline, a
//! confirmed report acknowledged and refused, and a cancellation with a
//! report queued. The server parity tests hold the notifications
//! themselves to the server's.
use std::sync::{Arc, Mutex};

use super::*;
use crate::report::NotifyService;
use bacnet_encoding::apdu::ErrorPdu;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_services::cov_multiple::{
    COVNotificationMultipleRequest, COVReference, COVSubscriptionSpecification,
    SubscribeCOVPropertyMultipleRequest,
};
use bacnet_types::constructed::PropertyReference;
use bacnet_types::enums::{ErrorClass, ErrorCode, UnconfirmedServiceChoice};
use bacnet_types::primitives::{Date, Time};

/// Max_Notification_Delay of every context here, in seconds.
const DELAY: u64 = 5;
const MULTIPLE: ConfirmedServiceChoice = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE;

/// A Device clock set by hand.
struct Clock(Mutex<ClockFrame>);

impl ClockReader for Clock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(*self.0.lock().unwrap())
    }
}

impl Clock {
    fn set(&self, second: u8) {
        self.0.lock().unwrap().local_time.second = second;
    }
}

/// A device whose database reads a hand-set clock, at second 1.
fn device() -> (Device, Arc<Clock>) {
    let clock = Arc::new(Clock(Mutex::new(ClockFrame {
        local_date: Date {
            year: 126,
            month: 9,
            day: 29,
            day_of_week: 2,
        },
        local_time: Time {
            hour: 14,
            minute: 0,
            second: 1,
            hundredths: 0,
        },
        utc_offset: 0,
        daylight_savings_status: false,
    })));
    let mut device = Device::new(OutboxLimits::default());
    device
        .db
        .set_clock_reader(Some(Arc::clone(&clock) as Arc<dyn ClockReader>));
    (device, clock)
}

impl Device {
    /// SubscribeCOVPropertyMultiple from `process` for Present_Value of
    /// each of `objects`, for 300 s, or a cancellation of the context when
    /// `objects` is empty; the reply. The subscriber accepts `max_apdu`
    /// octets.
    fn subscribe_multiple(
        &mut self,
        process: u32,
        confirmed: bool,
        objects: &[(ObjectIdentifier, bool)],
        max_apdu: u16,
        now: Instant,
    ) -> Apdu {
        let cancel = objects.is_empty();
        let mut body = BytesMut::new();
        SubscribeCOVPropertyMultipleRequest {
            subscriber_process_identifier: process,
            issue_confirmed_notifications: confirmed,
            lifetime: (!cancel).then_some(300),
            max_notification_delay: (!cancel).then_some(DELAY as u32),
            list_of_cov_subscription_specifications: objects
                .iter()
                .map(|&(object, timestamped)| COVSubscriptionSpecification {
                    monitored_object_identifier: object,
                    list_of_cov_references: vec![COVReference {
                        monitored_property: PropertyReference {
                            property_identifier: P::PRESENT_VALUE,
                            property_array_index: None,
                        },
                        cov_increment: None,
                        timestamped,
                    }],
                })
                .collect(),
        }
        .encode(&mut body)
        .unwrap();
        self.invoke_id += 1;
        let mut apdu = BytesMut::new();
        encode_apdu(
            &mut apdu,
            &Apdu::ConfirmedRequest(ConfirmedRequest {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: max_apdu,
                invoke_id: self.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: MULTIPLE,
                service_request: body.freeze(),
            }),
        )
        .unwrap();
        let mut out = BytesMut::new();
        let request = Inbound::new(apdu.freeze(), 1476);
        self.reporter
            .handle(&mut self.db, &peer(), request, now, &mut out);
        decode_apdu(out.freeze()).unwrap()
    }

    /// The next notification, a COV-multiple one, with its rows.
    fn pop_multiple(&mut self, now: Instant) -> Option<(Notification, Vec<Row>)> {
        let (notification, apdu) = self.pop(now)?;
        assert_eq!(notification.service, NotifyService::CovMultiple);
        let (confirmed, params) = match &apdu {
            Apdu::UnconfirmedRequest(request) => {
                let service = UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE;
                assert_eq!(request.service_choice, service);
                (false, &request.service_request)
            }
            Apdu::ConfirmedRequest(request) => {
                let service = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE;
                assert_eq!(request.service_choice, service);
                (true, &request.service_request)
            }
            other => panic!("not a notification: {other:?}"),
        };
        assert_eq!(notification.invoke_id.is_some(), confirmed);
        let request = COVNotificationMultipleRequest::decode(params).unwrap();
        Some((notification, rows(&request)))
    }

    fn sent(&mut self, notification: &Notification, now: Instant) {
        self.reporter
            .delivered(notification.token, Delivery::Sent, &mut self.ids, now);
    }

    /// Pop every queued notification as sent; their rows.
    fn drain(&mut self, now: Instant) -> Vec<Vec<Row>> {
        let mut all = Vec::new();
        while let Some((notification, rows)) = self.pop_multiple(now) {
            self.sent(&notification, now);
            all.push(rows);
        }
        all
    }

    /// The host changes `object`'s Present_Value itself and reports it.
    fn sample(&mut self, object: ObjectIdentifier, value: f32, now: Instant) {
        let sensor = self.db.get_mut(&object).unwrap();
        sensor
            .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
            .unwrap();
        sensor
            .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(value), None)
            .unwrap();
        self.reporter.changed(&self.db, &[object], now);
    }

    /// Answer the confirmed COV-multiple notification under `invoke_id`.
    fn answer(&mut self, invoke_id: u8, refuse: bool, now: Instant) -> bool {
        let service_choice = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE;
        let pdu = if refuse {
            Apdu::Error(ErrorPdu {
                invoke_id,
                service_choice,
                error_class: ErrorClass::SERVICES,
                error_code: ErrorCode::SERVICE_REQUEST_DENIED,
                error_data: Bytes::new(),
            })
        } else {
            Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice,
            })
        };
        let mut apdu = BytesMut::new();
        encode_apdu(&mut apdu, &pdu).unwrap();
        self.reporter
            .reply(&peer_address(&peer()), &apdu, &mut self.ids, now)
    }
}

/// One Present_Value row: the Analog Input's instance, the REAL, and the
/// Device second of its Time_Of_Change.
type Row = (u32, f32, Option<u8>);

fn rows(notification: &COVNotificationMultipleRequest) -> Vec<Row> {
    let mut rows = Vec::new();
    for item in &notification.list_of_cov_notifications {
        for value in &item.list_of_values {
            if value.property_identifier == P::PRESENT_VALUE {
                let octets: [u8; 4] = value.value[1..5].try_into().unwrap();
                rows.push((
                    item.monitored_object_identifier.instance_number(),
                    f32::from_be_bytes(octets),
                    value.time_of_change.map(|time| time.second),
                ));
            }
        }
    }
    rows
}

fn secs(seconds: u64) -> Instant {
    T0.saturating_add(Duration::from_secs(seconds))
}

/// When the subscriptions made at [`T0`] expire: the one deadline of a
/// reporter with nothing pending.
fn lifetime_end() -> Option<Instant> {
    Some(secs(300))
}

#[test]
fn an_untimestamped_context_reports_through_the_outbox() {
    let (mut device, _) = device();
    let reply = device.subscribe_multiple(1, false, &[(ai(), false), (ai2(), false)], 1476, T0);
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert_eq!(device.reporter.table().len(), 2);
    // The initial report, in one notification.
    assert_eq!(device.drain(T0), [[(1, 10.0, None), (2, 0.0, None)]]);

    device.sample(ai(), 11.0, secs(1));
    assert_eq!(device.drain(secs(1)), [[(1, 11.0, None)]]);
    // Delivered: the same value reports nothing again.
    device.reporter.changed(&device.db, &[ai()], secs(2));
    assert!(device.pop(secs(2)).is_none());
    assert_eq!(device.reporter.next_deadline(), lifetime_end());
    let counters = device.reporter.counters();
    assert_eq!(counters.cov.notifications_unconfirmed, 2);
}

#[test]
fn a_timestamped_context_reports_each_change_with_its_time() {
    let (mut device, clock) = device();
    device.subscribe_multiple(1, false, &[(ai(), true)], 1476, T0);
    assert_eq!(device.drain(T0), [[(1, 10.0, Some(1))]]);

    // A change through a write request, captured as it commits; then one
    // the host made itself, captured when it reports it.
    clock.set(10);
    device.write(P::OUT_OF_SERVICE, PropertyValue::Boolean(true), secs(1));
    clock.set(11);
    device.write(P::PRESENT_VALUE, PropertyValue::Real(12.5), secs(1));
    clock.set(12);
    device.sample(ai(), 13.0, secs(2));
    // The first report holds the context's turn while it is queued, so the
    // later changes wait and go out together once it has been sent.
    let (first, sent) = device.pop_multiple(secs(2)).expect("the first report");
    assert_eq!(sent, [(1, 10.0, Some(10))]);
    assert!(device.pop(secs(2)).is_none(), "one report at a time");
    device.sent(&first, secs(2));
    device.deadline();
    assert_eq!(
        device.drain(secs(2)),
        [[(1, 12.5, Some(11)), (1, 13.0, Some(12))]]
    );
    assert_eq!(device.reporter.next_deadline(), lifetime_end());
}

#[test]
fn contexts_that_report_at_once_are_queued_in_key_order() {
    let (mut device, _) = device();
    for process in [3, 1, 2] {
        device.subscribe_multiple(process, false, &[(ai(), false)], 1476, T0);
    }
    device.drain(T0);
    device.sample(ai(), 11.0, secs(1));
    let mut order = Vec::new();
    while let Some((notification, apdu)) = device.pop(secs(1)) {
        let Apdu::UnconfirmedRequest(request) = apdu else {
            panic!("unconfirmed contexts");
        };
        let request = COVNotificationMultipleRequest::decode(&request.service_request).unwrap();
        order.push(request.subscriber_process_identifier);
        device.sent(&notification, secs(1));
    }
    assert_eq!(order, [1, 2, 3]);
}

#[test]
fn a_part_that_is_not_sent_stops_its_report_and_leaves_the_rest_owed() {
    // A 50-octet subscriber takes one object's item per notification.
    let (mut device, _) = device();
    device.subscribe_multiple(1, false, &[(ai(), false), (ai2(), false)], 50, T0);
    let (first, sent) = device.pop_multiple(T0).expect("the first part");
    assert_eq!(sent, [(1, 10.0, None)]);
    device.sent(&first, T0);
    // The second part fails at second 1: nothing more is handed out, and
    // its reference is owed from then.
    let (second, sent) = device.pop_multiple(T0).expect("the second part");
    assert_eq!(sent, [(2, 0.0, None)]);
    device
        .reporter
        .delivered(second.token, Delivery::NotSent, &mut device.ids, secs(1));
    assert!(device.pop(secs(1)).is_none());
    assert_eq!(device.reporter.next_deadline(), Some(secs(1 + DELAY)));

    // At the deadline the owed reference is reported, and only it.
    device.deadline();
    assert_eq!(device.drain(secs(1 + DELAY)), [[(2, 0.0, None)]]);
    assert_eq!(device.reporter.next_deadline(), lifetime_end());
}

#[test]
fn a_first_part_that_is_not_sent_takes_the_queued_rest_with_it() {
    // To a 50-octet subscriber a timestamped change goes out one value per
    // notification: four parts for the two initial changes.
    let (mut device, _) = device();
    device.subscribe_multiple(1, false, &[(ai(), true), (ai2(), true)], 50, T0);
    let (first, _) = device.pop_multiple(T0).expect("the first part");
    device
        .reporter
        .delivered(first.token, Delivery::NotSent, &mut device.ids, T0);
    assert!(device.pop(T0).is_none(), "the other parts left the outbox");
    // Nothing went, so both changes wait for their delay, from admission.
    assert_eq!(device.reporter.next_deadline(), Some(secs(DELAY)));
    device.deadline();
    let parts = device.drain(secs(DELAY));
    assert_eq!(parts.len(), 4);
    assert_eq!(parts.concat(), [(1, 10.0, Some(1)), (2, 0.0, Some(1))]);
    assert_eq!(device.reporter.next_deadline(), lifetime_end());
}

#[test]
fn a_confirmed_report_is_retired_by_its_ack() {
    let (mut device, clock) = device();
    device.subscribe_multiple(1, true, &[(ai(), true)], 1476, T0);
    let (initial, sent) = device.pop_multiple(T0).expect("the initial report");
    assert_eq!(sent, [(1, 10.0, Some(1))]);
    let invoke_id = initial.invoke_id.expect("confirmed");

    // A change while the report is out waits for its Ack.
    clock.set(10);
    device.sample(ai(), 11.0, secs(1));
    assert!(device.pop(secs(1)).is_none(), "one outstanding report");
    assert!(device.answer(invoke_id, false, secs(2)));
    device.deadline();
    let (next, sent) = device.pop_multiple(secs(2)).expect("the held change");
    assert_eq!(sent, [(1, 11.0, Some(10))], "and not the initial one again");
    assert!(device.answer(next.invoke_id.unwrap(), false, secs(3)));

    // Both claims were retired: nothing comes back when the reports'
    // deadlines pass.
    device.deadline();
    device
        .reporter
        .on_deadline(&device.db, &mut device.ids, secs(60));
    assert!(device.pop(secs(60)).is_none());
    assert_eq!(device.reporter.next_deadline(), lifetime_end());
    assert!(device.ids.taken.is_empty());
}

#[test]
fn a_refused_confirmed_report_goes_out_again_after_its_hold_off() {
    let (mut device, clock) = device();
    device.subscribe_multiple(1, true, &[(ai(), true)], 1476, T0);
    let (initial, _) = device.pop_multiple(T0).expect("the initial report");
    assert!(device.answer(initial.invoke_id.unwrap(), false, T0));
    device.deadline();

    clock.set(10);
    device.sample(ai(), 11.0, secs(1));
    let (report, sent) = device.pop_multiple(secs(1)).expect("the change");
    assert_eq!(sent, [(1, 11.0, Some(10))]);
    // The subscriber refuses it at second 2. The change is pending again,
    // and overdue by its delay at second 6, yet the context is held off
    // for one retry cycle: 6 s with this timing.
    assert!(device.answer(report.invoke_id.unwrap(), true, secs(2)));
    assert_eq!(device.reporter.counters().failed, 1);
    assert_eq!(device.reporter.next_deadline(), Some(secs(8)));
    device
        .reporter
        .on_deadline(&device.db, &mut device.ids, secs(7));
    assert!(device.pop(secs(7)).is_none(), "still held off");

    // At its end the first look hands the context to one follow-up, which
    // carries the change again, with its own time.
    device.deadline();
    let (again, sent) = device.pop_multiple(secs(8)).expect("the follow-up");
    assert_eq!(sent, [(1, 11.0, Some(10))]);
    assert!(device.answer(again.invoke_id.unwrap(), false, secs(9)));
    device.deadline();
    assert_eq!(device.reporter.next_deadline(), lifetime_end());
}

#[test]
fn a_cancellation_drops_the_queued_report_and_its_pending_changes() {
    let (mut device, clock) = device();
    device.subscribe_multiple(1, false, &[(ai(), true), (ai2(), true)], 1476, T0);
    device.drain(T0);
    clock.set(10);
    device.sample(ai(), 11.0, secs(1));
    clock.set(11);
    device.sample(ai2(), 12.0, secs(2));
    // One report is queued, and one change waits behind its turn.
    assert!(device.reporter.next_deadline() < lifetime_end());

    let reply = device.subscribe_multiple(1, false, &[], 1476, secs(3));
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert!(device.reporter.table().is_empty());
    assert!(device.pop(secs(3)).is_none(), "the queued report went");
    // Nothing is pending, owed or due any more: what is left is the
    // table's bound from the lifetimes it held.
    assert_eq!(device.reporter.next_deadline(), lifetime_end());

    // The same context subscribes again and reports afresh: its turn was
    // not left held.
    clock.set(20);
    device.subscribe_multiple(1, false, &[(ai(), true)], 1476, secs(4));
    assert_eq!(device.drain(secs(4)), [[(1, 11.0, Some(20))]]);
}

#[test]
fn a_refused_reference_is_named_and_those_before_it_report() {
    let (mut device, _) = device();
    let missing = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();
    let reply = device.subscribe_multiple(1, false, &[(ai(), false), (missing, false)], 1476, T0);
    match reply {
        Apdu::Error(error) => {
            assert_eq!(error.service_choice, MULTIPLE);
            assert_eq!(error.error_code, ErrorCode::UNKNOWN_OBJECT);
            assert!(!error.error_data.is_empty(), "the reference that failed");
        }
        other => panic!("{other:?}"),
    }
    // The reference accepted before it is kept, and reported.
    assert_eq!(device.reporter.table().len(), 1);
    assert_eq!(device.drain(T0), [[(1, 10.0, None)]]);

    // A request that does not decode is rejected and keeps nothing more.
    let reply = device.request(MULTIPLE, &[0xFF], T0);
    assert!(matches!(reply, Apdu::Reject(_)), "{reply:?}");
    assert_eq!(device.reporter.table().len(), 1);
    assert!(device.pop(T0).is_none());
}

#[test]
fn a_renewal_that_fences_a_report_takes_its_changes_back() {
    let (mut device, clock) = device();
    device.subscribe_multiple(1, true, &[(ai(), true)], 1476, T0);
    let (initial, _) = device.pop_multiple(T0).expect("the initial report");
    assert!(device.answer(initial.invoke_id.unwrap(), false, T0));
    device.deadline();
    clock.set(10);
    device.sample(ai(), 11.0, secs(1));
    let (out, sent) = device.pop_multiple(secs(1)).expect("the change");
    assert_eq!(sent, [(1, 11.0, Some(10))]);

    // The subscriber lists the reference again while that report is out:
    // the report is fenced, and its change goes back to the reference at
    // once, not at the report's deadline, so the renewal's own report
    // carries it ahead of the value captured at the renewal.
    clock.set(20);
    device.subscribe_multiple(1, true, &[(ai(), true)], 1476, secs(2));
    let (renewed, sent) = device.pop_multiple(secs(2)).expect("the renewal's report");
    assert_eq!(sent.first(), Some(&(1, 11.0, Some(10))), "{sent:?}");
    assert_eq!(sent.last(), Some(&(1, 11.0, Some(20))), "{sent:?}");
    // The fenced report's Ack answers nothing any more.
    assert!(!device.answer(out.invoke_id.unwrap(), false, secs(3)));
    assert!(device.answer(renewed.invoke_id.unwrap(), false, secs(3)));
    device.deadline();
    assert!(device.pop(secs(3)).is_none());
    assert!(device.ids.taken.is_empty());
}

#[test]
fn a_deleted_objects_pending_changes_go_with_its_references() {
    let (mut device, clock) = device();
    device.subscribe_multiple(1, false, &[(ai(), true)], 1476, T0);
    device.drain(T0);
    clock.set(10);
    device.sample(ai(), 11.0, secs(1));
    let (report, _) = device.pop_multiple(secs(1)).expect("the change");
    device
        .reporter
        .delivered(report.token, Delivery::NotSent, &mut device.ids, secs(1));
    assert_eq!(device.reporter.next_deadline(), Some(secs(1 + DELAY)));

    // The histories follow the table: with the object's references gone,
    // no change of it waits for a deadline.
    device.reporter.remove_for_object(ai(), &mut device.ids);
    assert!(device.reporter.table().is_empty());
    assert_eq!(device.reporter.next_deadline(), lifetime_end());
    device
        .reporter
        .on_deadline(&device.db, &mut device.ids, secs(1 + DELAY));
    assert!(device.pop(secs(1 + DELAY)).is_none());
}
