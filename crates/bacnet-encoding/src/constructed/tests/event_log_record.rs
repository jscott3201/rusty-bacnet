use super::*;
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetDeviceObjectReference, BACnetEventLogRecord, BACnetPropertyValue, ChangeOfValueChoice,
    EventLogDatum, EventNotificationRequest, NotificationParameters, OpaqueChoice,
};
use bacnet_types::enums::{
    AccessEvent, EventState, EventType, NotifyType, PropertyIdentifier, TimerState, TimerTransition,
};
use bacnet_types::primitives::{BACnetTimeStamp, Date, StatusFlags, Time};

const DATE: Date = Date {
    year: 126,
    month: 8,
    day: 31,
    day_of_week: 1,
};

const TIME: Time = Time {
    hour: 14,
    minute: 25,
    second: 36,
    hundredths: 47,
};

fn record(log_datum: EventLogDatum) -> BACnetEventLogRecord {
    BACnetEventLogRecord {
        date: DATE,
        time: TIME,
        log_datum,
    }
}

fn encoded(record: &BACnetEventLogRecord) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_event_log_record(record, &mut buf).unwrap();
    buf.to_vec()
}

const TIMESTAMP: [u8; 12] = [
    0x0E, 0xA4, 0x7E, 0x08, 0x1F, 0x01, 0xB4, 0x0E, 0x19, 0x24, 0x2F, 0x0F,
];

/// The parameters of a short ConfirmedEventNotification request: process 1,
/// Device 1 reporting Analog Input 1 at sequence-number timestamp 5, class 0,
/// priority 100, CHANGE_OF_STATE, ALARM, no ack, NORMAL to OFFNORMAL.
const NOTIFICATION: [u8; 30] = [
    0x09, 0x01, 0x1C, 0x02, 0x00, 0x00, 0x01, 0x2C, 0x00, 0x00, 0x00, 0x01, 0x3E, 0x19, 0x05, 0x3F,
    0x49, 0x00, 0x59, 0x64, 0x69, 0x01, 0x89, 0x00, 0x99, 0x00, 0xA9, 0x00, 0xB9, 0x02,
];

/// The request [`NOTIFICATION`] encodes.
fn notification() -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(5),
        notification_class: 0,
        priority: 100,
        event_type: EventType::CHANGE_OF_STATE,
        message_text: None,
        notify_type: NotifyType::ALARM,
        ack_required: false,
        from_state: EventState::NORMAL,
        to_state: EventState::OFFNORMAL,
        event_values: None,
    }
}

/// [`notification`] reporting `event_type` with `event_values`.
fn notification_with(
    event_type: EventType,
    event_values: NotificationParameters,
) -> EventNotificationRequest {
    EventNotificationRequest {
        event_type,
        event_values: Some(event_values),
        ..notification()
    }
}

/// The record's bytes around a notification: the timestamp, then the datum's
/// `[1]` frame and the notification's own `[1]` frame around `parameters`.
fn framed(parameters: &[u8]) -> Vec<u8> {
    [&TIMESTAMP[..], &[0x1E, 0x1E], parameters, &[0x1F, 0x1F][..]].concat()
}

#[test]
fn event_log_record_kinds_have_exact_bytes_and_round_trip() {
    for (log_datum, tail) in [
        // log-status [0]: log-disabled, bit 0, in the top bit.
        (
            EventLogDatum::LogStatus(LogStatus::LOG_DISABLED),
            vec![0x1E, 0x0A, 0x05, 0x80, 0x1F],
        ),
        // notification [1] around the request's own fields.
        (
            EventLogDatum::Notification(notification()),
            framed(&NOTIFICATION)[TIMESTAMP.len()..].to_vec(),
        ),
        // time-change [2], unknown amount.
        (
            EventLogDatum::TimeChange(0.0),
            vec![0x1E, 0x2C, 0x00, 0x00, 0x00, 0x00, 0x1F],
        ),
    ] {
        let value = record(log_datum);
        let bytes = encoded(&value);
        assert_eq!(bytes, [&TIMESTAMP[..], &tail].concat(), "{value:?}");
        assert_eq!(
            decode_event_log_record(&bytes, 0).unwrap(),
            (value, bytes.len())
        );
    }
}

/// A notification record carries the request exactly as the notification
/// codec writes it, and reads back as the same typed request, whatever its
/// event values.
#[test]
fn event_log_notification_records_round_trip_typed_event_values() {
    let date_time = (DATE, TIME);
    let notifications = [
        notification_with(
            EventType::OUT_OF_RANGE,
            NotificationParameters::OutOfRange {
                exceeding_value: 85.5,
                status_flags: StatusFlags::IN_ALARM,
                deadband: 1.0,
                exceeded_limit: 80.0,
            },
        ),
        notification_with(
            EventType::CHANGE_OF_STATE,
            NotificationParameters::ChangeOfState {
                new_state: BACnetPropertyStates::BooleanValue(true),
                status_flags: StatusFlags::IN_ALARM | StatusFlags::FAULT,
            },
        ),
        notification_with(
            EventType::CHANGE_OF_VALUE,
            NotificationParameters::ChangeOfValue {
                new_value: ChangeOfValueChoice::ChangedBits {
                    unused_bits: 4,
                    data: vec![0xA0],
                },
                status_flags: StatusFlags::empty(),
            },
        ),
        notification_with(
            EventType::BUFFER_READY,
            NotificationParameters::BufferReady {
                buffer_property: BACnetDeviceObjectPropertyReference {
                    object_identifier: ObjectIdentifier::new(ObjectType::TREND_LOG, 2).unwrap(),
                    property_identifier: PropertyIdentifier::LOG_BUFFER.to_raw(),
                    property_array_index: None,
                    device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap()),
                },
                previous_notification: 10,
                current_notification: 20,
            },
        ),
        notification_with(
            EventType::EXTENDED,
            NotificationParameters::ComplexEventType {
                property_values: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x44, 0x42, 0x91, 0x00, 0x00],
                    priority: Some(8),
                }],
            },
        ),
        notification_with(
            EventType::CHANGE_OF_CHARACTERSTRING,
            NotificationParameters::ChangeOfCharacterstring {
                changed_value: "door open".into(),
                status_flags: StatusFlags::IN_ALARM,
                alarm_value: "open".into(),
            },
        ),
        notification_with(
            EventType::ACCESS_EVENT,
            NotificationParameters::AccessEvent {
                access_event: AccessEvent::GRANTED,
                status_flags: StatusFlags::empty(),
                access_event_tag: 3,
                access_event_time: date_time,
                access_credential: BACnetDeviceObjectReference {
                    device_identifier: None,
                    object_identifier: ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, 4)
                        .unwrap(),
                },
                authentication_factor: None,
            },
        ),
        notification_with(
            EventType::CHANGE_OF_TIMER,
            NotificationParameters::ChangeOfTimer {
                new_state: TimerState::RUNNING,
                status_flags: StatusFlags::empty(),
                update_time: date_time,
                last_state_change: Some(TimerTransition::IDLE_TO_RUNNING),
                initial_timeout: Some(60),
                expiration_time: Some(date_time),
            },
        ),
        // An acknowledgment carries a message and no ack-required, from-state
        // or event values.
        EventNotificationRequest {
            message_text: Some("acknowledged".into()),
            notify_type: NotifyType::ACK_NOTIFICATION,
            ..notification()
        },
    ];
    for notification in notifications {
        let mut parameters = BytesMut::new();
        encode_event_notification(&notification, &mut parameters).unwrap();
        let value = record(EventLogDatum::Notification(notification));
        let bytes = encoded(&value);
        assert_eq!(bytes, framed(&parameters), "{value:?}");
        assert_eq!(
            decode_event_log_record(&bytes, 0).unwrap(),
            (value, bytes.len())
        );
    }
}

#[test]
fn consecutive_event_log_records_decode_by_returned_offset() {
    let first = record(EventLogDatum::Notification(notification()));
    let second = record(EventLogDatum::TimeChange(3.25));
    let mut bytes = encoded(&first);
    bytes.extend(encoded(&second));
    let (decoded, next) = decode_event_log_record(&bytes, 0).unwrap();
    assert_eq!(decoded, first);
    assert_eq!(
        decode_event_log_record(&bytes, next).unwrap(),
        (second, bytes.len())
    );
}

#[test]
fn event_log_record_rejects_unencodable_values_without_writing() {
    for notification in [
        // Raw event values with an opening tag left open.
        notification_with(
            EventType::COMMAND_FAILURE,
            NotificationParameters::CommandFailure {
                command_value: vec![0x3E, 0x19, 0x05],
                status_flags: StatusFlags::empty(),
                feedback_value: vec![0x91, 0x00],
            },
        ),
        // A property priority outside 1 to 16.
        notification_with(
            EventType::EXTENDED,
            NotificationParameters::ComplexEventType {
                property_values: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x10],
                    priority: Some(17),
                }],
            },
        ),
        // Bit strings the decoder would refuse: eight unused bits, and
        // unused bits with no octets to hold them.
        notification_with(
            EventType::CHANGE_OF_BITSTRING,
            NotificationParameters::ChangeOfBitstring {
                referenced_bitstring: (8, vec![0xA0]),
                status_flags: StatusFlags::empty(),
            },
        ),
        notification_with(
            EventType::CHANGE_OF_VALUE,
            NotificationParameters::ChangeOfValue {
                new_value: ChangeOfValueChoice::ChangedBits {
                    unused_bits: 3,
                    data: vec![],
                },
                status_flags: StatusFlags::empty(),
            },
        ),
    ] {
        let mut buf = BytesMut::from(&b"kept"[..]);
        let value = record(EventLogDatum::Notification(notification));
        assert!(encode_event_log_record(&value, &mut buf).is_err());
        assert_eq!(&buf[..], b"kept");
    }
}

#[test]
fn event_log_record_decoder_rejects_malformed_records() {
    let good = encoded(&record(EventLogDatum::TimeChange(1.0)));
    let mut cases: Vec<Vec<u8>> = vec![
        good[..good.len() - 1].to_vec(),
        // Two alternatives in one datum.
        [
            &TIMESTAMP[..],
            &[0x1E, 0x0A, 0x05, 0x80, 0x0A, 0x05, 0x80, 0x1F],
        ]
        .concat(),
        // log-status that isn't a three-bit BitString.
        [&TIMESTAMP[..], &[0x1E, 0x0A, 0x04, 0x60, 0x1F]].concat(),
        // A notification holding a stray application tag cut short.
        [&TIMESTAMP[..], &[0x1E, 0x1E, 0x44, 0x00, 0x1F, 0x1F]].concat(),
    ];
    // A timestamp missing its Time.
    cases.push(vec![
        0x0E, 0xA4, 0x7E, 0x08, 0x1F, 0x01, 0x0F, 0x1E, 0x2C, 0, 0, 0, 0, 0x1F,
    ]);
    for bytes in cases {
        assert!(decode_event_log_record(&bytes, 0).is_err(), "{bytes:02X?}");
    }
    // A tag the datum doesn't assign, a Trend Log's null-value [7] say, is
    // kept as received (#2474), not refused.
    let unknown = [&TIMESTAMP[..], &[0x1E, 0x78, 0x1F]].concat();
    let (decoded, end) = decode_event_log_record(&unknown, 0).unwrap();
    assert_eq!(end, unknown.len());
    assert_eq!(
        decoded.log_datum,
        EventLogDatum::Opaque(OpaqueChoice {
            tag: 7,
            constructed: false,
            data: vec![],
        })
    );
}

/// Well-formed tagged fields that don't make up a notification request fail
/// to decode: a process identifier alone, the request cut before its
/// to-state, and the request with a field after its last member.
#[test]
fn event_log_record_decoder_rejects_notifications_that_are_not_requests() {
    let cut = &NOTIFICATION[..NOTIFICATION.len() - 2];
    let mut trailing = NOTIFICATION.to_vec();
    trailing.extend([0xD9, 0x01]);
    for parameters in [&[0x09, 0x01][..], cut, &trailing] {
        let bytes = framed(parameters);
        assert!(decode_event_log_record(&bytes, 0).is_err(), "{bytes:02X?}");
    }
}

/// A message text whose characters don't decode (here one in the DBCS
/// character set) is dropped, as the client drops it from a received
/// notification, and the rest of the record still reads. A text that isn't
/// framed as the one field before the notify type still fails the record.
#[test]
fn event_log_record_drops_a_message_text_it_cannot_read() {
    // The notify type [8] starts at offset 22 of the request.
    let with_text = |text: &[u8]| {
        let mut parameters = NOTIFICATION.to_vec();
        parameters.splice(22..22, text.iter().copied());
        framed(&parameters)
    };
    let bytes = with_text(&[0x7A, 0x01, b'A']);
    assert_eq!(
        decode_event_log_record(&bytes, 0).unwrap(),
        (
            record(EventLogDatum::Notification(notification())),
            bytes.len()
        )
    );
    // Two text fields, and a text with no charset octet.
    for text in [&[0x7A, 0x01, b'A', 0x7A, 0x01, b'A'][..], &[0x78]] {
        let bytes = with_text(text);
        assert!(decode_event_log_record(&bytes, 0).is_err(), "{bytes:02X?}");
    }
}

#[test]
fn members_cut_short_are_a_short_buffer() {
    for log_datum in [
        EventLogDatum::LogStatus(LogStatus::LOG_DISABLED),
        EventLogDatum::Notification(notification()),
        EventLogDatum::TimeChange(0.0),
    ] {
        let octets = encoded(&record(log_datum));
        let framed = assert_members_cut_short("BACnetEventLogRecord", &octets, |data| {
            decode_event_log_record(data, 0)
        });
        assert!(framed > 0);
    }
}

#[test]
fn event_log_record_refuses_an_acknowledgment_with_alarm_only_fields() {
    // #2247: an ALARM's parameters with the notify type turned to
    // ACK_NOTIFICATION keep ackRequired and fromState, which an
    // acknowledgment leaves out; the record refuses them.
    let mut parameters = BytesMut::new();
    encode_event_notification(&notification(), &mut parameters).unwrap();
    let notify_type = parameters
        .windows(2)
        .position(|pair| pair == [0x89, 0x00])
        .expect("[8] notifyType ALARM");
    parameters[notify_type + 1] = 0x02;
    assert!(decode_event_log_record(&framed(&parameters), 0).is_err());
}
