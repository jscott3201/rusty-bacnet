//! The tail every change runs (#2524): a request's write evaluates
//! intrinsic reporting as the host's change does, and a change's event
//! notifications go ahead of its COV report (#889).

use super::*;
use bacnet_encoding::constructed::decode_event_notification;
use bacnet_types::enums::EventState;

/// The next notification is an event notification to `state`, and the one
/// after it the COV report of the same change, carrying `value`.
fn assert_event_then_cov(device: &mut Device, state: EventState, value: f32, now: Instant) {
    let (event, apdu) = device.pop(now).expect("the event notification");
    assert_eq!(event.service, NotifyService::Event, "{apdu:?}");
    let params = match &apdu {
        Apdu::UnconfirmedRequest(request) => &request.service_request,
        other => panic!("not an unconfirmed notification: {other:?}"),
    };
    let notification = decode_event_notification(params).unwrap();
    assert_eq!(notification.to_state, state);
    let (cov, apdu) = device.pop(now).expect("the COV report after it");
    assert_eq!(cov.service, NotifyService::Cov);
    assert_eq!(Device::reported_pv(&apdu), PropertyValue::Real(value));
    assert!(device.pop(now).is_none());
}

/// A device sweeping every ten seconds, whose AI-3 has no Time_Delay and
/// one unconfirmed COV subscriber, its initial report sent.
fn subscribed() -> Device {
    let mut device = device(
        OutboxLimits::default(),
        0,
        vec![address_destination(false)],
        10,
    );
    device.subscribe_to(alarm(), 1, false, ms(10));
    let (initial, _) = device.pop(ms(10)).expect("the initial report");
    assert_eq!(initial.service, NotifyService::Cov);
    device
}

#[test]
fn a_write_request_evaluates_intrinsic_reporting_at_once() {
    let mut device = subscribed();
    // High_Limit under the reading: the write commits the transition, not
    // the sweep ten seconds on.
    device.write_to(alarm(), P::HIGH_LIMIT, PropertyValue::Real(5.0), ms(100));
    assert_eq!(device.reporter.next_deadline(), Some(ms(10_000)));
    assert_event_then_cov(&mut device, EventState::HIGH_LIMIT, 10.0, ms(100));
}

#[test]
fn a_host_change_queues_its_event_notification_ahead_of_its_cov_report() {
    let mut device = subscribed();
    device.raise(alarm(), ms(100));
    assert_event_then_cov(&mut device, EventState::HIGH_LIMIT, 90.0, ms(100));
}

/// Timestamped COV-multiple rows of a write and the event-state transition
/// it causes (#2535): one row for the two, through a request as through the
/// host's own change.
#[cfg(feature = "cov-multiple")]
mod timed {
    use super::*;
    use bacnet_objects::clock::{ClockFrame, ClockReader};
    use bacnet_services::cov_multiple::{
        COVNotificationMultipleRequest, COVReference, COVSubscriptionSpecification,
        SubscribeCOVPropertyMultipleRequest,
    };
    use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
    use bacnet_types::constructed::PropertyReference;
    use bacnet_types::primitives::{Date, Time};

    const OOS: u8 = 0x10;
    const FAULT: u8 = 0x40;
    const IN_ALARM: u8 = 0x80;

    /// A Device clock a hundredth later at each read, so two captures never
    /// share a time and a report collapses no row into a later one.
    #[derive(Default)]
    struct Ticking(core::sync::atomic::AtomicU8);

    impl ClockReader for Ticking {
        fn read_clock(&self) -> Option<ClockFrame> {
            let tick = self.0.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            Some(ClockFrame {
                local_date: Date {
                    year: 126,
                    month: 9,
                    day: 29,
                    day_of_week: 2,
                },
                local_time: Time {
                    hour: 14,
                    minute: 0,
                    second: 10 + tick / 100,
                    hundredths: tick % 100,
                },
                utc_offset: 0,
                daylight_savings_status: false,
            })
        }
    }

    /// One row: a change of AI-3's Present_Value and the Status_Flags it
    /// carries, as their octets.
    type Row = (Vec<u8>, Vec<u8>);

    /// The event-path device with AI-3's Status_Flags and Present_Value
    /// timestamped in one context, Max_Notification_Delay 0, its initial
    /// report sent.
    fn timestamped() -> Device {
        let mut device = device(
            OutboxLimits::default(),
            0,
            vec![address_destination(false)],
            10,
        );
        let clock = std::sync::Arc::new(Ticking::default());
        device.db.set_clock_reader(Some(clock));
        let reference = |property| COVReference {
            monitored_property: PropertyReference {
                property_identifier: property,
                property_array_index: None,
            },
            cov_increment: None,
            timestamped: true,
        };
        let mut body = BytesMut::new();
        SubscribeCOVPropertyMultipleRequest {
            subscriber_process_identifier: 1,
            issue_confirmed_notifications: false,
            lifetime: Some(300),
            max_notification_delay: Some(0),
            list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
                monitored_object_identifier: alarm(),
                list_of_cov_references: vec![
                    reference(P::STATUS_FLAGS),
                    reference(P::PRESENT_VALUE),
                ],
            }],
        }
        .encode(&mut body)
        .unwrap();
        let service = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE;
        let reply = device.request(service, &body, ms(1));
        assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
        values(&mut device, ms(1));
        device
    }

    /// The values of the COV-multiple reports the device sends now, each
    /// sent, every one with a Time_Of_Change. Event notifications go too.
    fn values(device: &mut Device, now: Instant) -> Vec<(P, Vec<u8>)> {
        let mut values = Vec::new();
        while let Some((notification, apdu)) = device.pop(now) {
            if notification.service == NotifyService::CovMultiple {
                let Apdu::UnconfirmedRequest(request) = &apdu else {
                    panic!("an unconfirmed report: {apdu:?}");
                };
                let report = COVNotificationMultipleRequest::decode(&request.service_request);
                for item in report.unwrap().list_of_cov_notifications {
                    for value in item.list_of_values {
                        let timed = value.time_of_change.is_some();
                        assert!(timed, "{:?}", value.property_identifier);
                        values.push((value.property_identifier, value.value));
                    }
                }
            }
            device
                .reporter
                .delivered(notification.token, Delivery::Sent, &mut device.ids, now);
        }
        values
    }

    /// The rows the device reports now. With a clock that moves at every
    /// capture each row goes out whole, Present_Value then Status_Flags.
    fn rows(device: &mut Device, now: Instant) -> Vec<Row> {
        let values = values(device, now);
        assert_eq!(values.len() % 2, 0, "{values:?}");
        values
            .chunks(2)
            .map(|pair| {
                assert_eq!([pair[0].0, pair[1].0], [P::PRESENT_VALUE, P::STATUS_FLAGS]);
                (pair[0].1.clone(), pair[1].1.clone())
            })
            .collect()
    }

    /// The row of Present_Value `value` with Status_Flags `bits`.
    fn row(value: f32, bits: u8) -> Row {
        let mut real = vec![0x44];
        real.extend_from_slice(&value.to_be_bytes());
        (real, vec![0x82, 0x04, bits])
    }

    /// One WritePropertyMultiple of AI-3's `writes`, in that order.
    fn write_multiple(device: &mut Device, writes: &[(P, PropertyValue)], now: Instant) {
        let list_of_properties = writes
            .iter()
            .map(|(property, value)| {
                let mut encoded = BytesMut::new();
                encode_property_value(&mut encoded, value).unwrap();
                bacnet_services::common::BACnetPropertyValue {
                    property_identifier: *property,
                    property_array_index: None,
                    value: encoded.to_vec(),
                    priority: None,
                }
            })
            .collect();
        let mut body = BytesMut::new();
        WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: alarm(),
                list_of_properties,
            }],
        }
        .encode(&mut body)
        .unwrap();
        let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
        let reply = device.request(service, &body, now);
        assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    }

    /// AI-3 out of service through a request, so a peer may write its
    /// Reliability and Present_Value.
    fn out_of_service(device: &mut Device) {
        let out_of_service = PropertyValue::Boolean(true);
        device.write_to(alarm(), P::OUT_OF_SERVICE, out_of_service, ms(50));
        assert_eq!(rows(device, ms(50)), [row(10.0, OOS)]);
    }

    /// A fault a request's Reliability write causes, and its clear, are a
    /// row each: no row in fault but still normal, or back to no fault but
    /// still in fault.
    #[test]
    fn a_requests_fault_and_its_clear_are_a_row_each() {
        let mut device = timestamped();
        out_of_service(&mut device);
        let reliability = |code| PropertyValue::Enumerated(code);
        device.write_to(alarm(), P::RELIABILITY, reliability(1), ms(100));
        let fault = IN_ALARM | FAULT | OOS;
        assert_eq!(rows(&mut device, ms(100)), [row(10.0, fault)]);
        device.write_to(alarm(), P::RELIABILITY, reliability(0), ms(200));
        assert_eq!(rows(&mut device, ms(200)), [row(10.0, OOS)]);
    }

    #[test]
    fn a_requests_value_over_the_limit_is_one_row() {
        let mut device = timestamped();
        out_of_service(&mut device);
        let value = PropertyValue::Real(90.0);
        device.write_to(alarm(), P::PRESENT_VALUE, value, ms(100));
        let in_alarm = IN_ALARM | OOS;
        assert_eq!(rows(&mut device, ms(100)), [row(90.0, in_alarm)]);
    }

    /// Two writes of Present_Value in one request keep a row each (#856).
    #[test]
    fn two_writes_in_one_request_keep_a_row_each() {
        let mut device = timestamped();
        out_of_service(&mut device);
        let writes = [
            (P::PRESENT_VALUE, PropertyValue::Real(90.0)),
            (P::PRESENT_VALUE, PropertyValue::Real(50.0)),
        ];
        write_multiple(&mut device, &writes, ms(100));
        let rows = rows(&mut device, ms(100));
        assert_eq!(rows, [row(90.0, OOS), row(50.0, OOS)], "no transition");
    }

    /// The transition the second of two writes causes is folded into that
    /// write's row: `(50, normal)` then `(90, in alarm)`, no `(90, normal)`.
    #[test]
    fn a_transition_is_folded_into_the_row_of_the_write_that_caused_it() {
        let mut device = timestamped();
        out_of_service(&mut device);
        let writes = [
            (P::PRESENT_VALUE, PropertyValue::Real(50.0)),
            (P::PRESENT_VALUE, PropertyValue::Real(90.0)),
        ];
        write_multiple(&mut device, &writes, ms(100));
        let rows = rows(&mut device, ms(100));
        assert_eq!(rows, [row(50.0, OOS), row(90.0, IN_ALARM | OOS)]);
    }

    /// A later attempt that changes nothing a reference conveys queues no
    /// row: the transition it causes is a row of its own, after the earlier
    /// attempt's, which it doesn't replace.
    #[test]
    fn a_transition_from_a_later_attempt_keeps_the_earlier_row() {
        let mut device = timestamped();
        out_of_service(&mut device);
        let writes = [
            (P::PRESENT_VALUE, PropertyValue::Real(50.0)),
            (P::HIGH_LIMIT, PropertyValue::Real(40.0)),
        ];
        write_multiple(&mut device, &writes, ms(100));
        let rows = rows(&mut device, ms(100));
        assert_eq!(rows, [row(50.0, OOS), row(50.0, IN_ALARM | OOS)]);
    }

    /// The host's own change is evaluated before it is captured: one row.
    #[test]
    fn a_host_change_and_its_transition_are_one_row() {
        let mut device = timestamped();
        device.raise(alarm(), ms(100));
        assert_eq!(rows(&mut device, ms(100)), [row(90.0, IN_ALARM)]);
    }
}
