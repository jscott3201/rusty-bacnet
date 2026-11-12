//! BUFFER_READY on the three log objects (#1347, Clause 13.3.7).
use super::*;
use crate::clock::{ClockFrame, ClockReader};
#[cfg(feature = "event-reporting")]
use crate::event::commit_test_proposal;
#[cfg(feature = "event-reporting")]
use crate::event::test_clock::{test_now, test_tick};
use crate::event_log::EventLogObject;
use crate::traits::BACnetObject;
use crate::trend::{TrendLogMultipleObject, TrendLogObject};
#[cfg(feature = "event-reporting")]
use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, EventLogDatum, LogData,
    LogDatum,
};
#[cfg(feature = "event-reporting")]
use bacnet_types::enums::ObjectType;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;
#[cfg(feature = "event-reporting")]
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::primitives::{Date, Time};
use std::sync::Arc;

struct FixedClock;

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: DATE,
            local_time: TIME,
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

const DATE: Date = Date {
    year: 126,
    month: 10,
    day: 4,
    day_of_week: 7,
};

const TIME: Time = Time {
    hour: 9,
    minute: 30,
    second: 0,
    hundredths: 0,
};

/// One log of each kind, on a valid clock.
fn logs() -> [Box<dyn BACnetObject>; 3] {
    let mut logs: [Box<dyn BACnetObject>; 3] = [
        Box::new(TrendLogObject::new(1, "TL-1", 4).unwrap()),
        Box::new(EventLogObject::new(1, "EL-1", 4).unwrap()),
        Box::new(TrendLogMultipleObject::new(1, "TLM-1", 4).unwrap()),
    ];
    for log in &mut logs {
        log.timed().unwrap().bind_clocks(&crate::traits::Clocks {
            clock: Some(Arc::new(FixedClock)),
            ..crate::traits::Clocks::default()
        });
    }
    logs
}

#[cfg(feature = "event-reporting")]
/// Add one ordinary record of the log's own family.
fn add(log: &mut dyn BACnetObject) {
    match log.object_identifier().object_type() {
        ObjectType::TREND_LOG => log
            .log_buffer_mut()
            .unwrap()
            .add_trend_record(BACnetLogRecord {
                date: DATE,
                time: TIME,
                log_datum: LogDatum::UnsignedValue(1),
                status_flags: None,
            }),
        ObjectType::EVENT_LOG => {
            log.log_buffer_mut()
                .unwrap()
                .add_event_log_record(BACnetEventLogRecord {
                    date: DATE,
                    time: TIME,
                    log_datum: EventLogDatum::TimeChange(1.0),
                })
        }
        _ => log
            .log_buffer_mut()
            .unwrap()
            .add_trend_multiple_record(BACnetLogMultipleRecord {
                date: DATE,
                time: TIME,
                log_data: LogData::Values(Vec::new()),
            }),
    }
    .unwrap();
}

/// The class and code of a protocol error.
fn code(error: Error) -> (ErrorClass, ErrorCode) {
    match error {
        Error::Remote(RemoteError::Error { class, code }) => (class, code),
        other => panic!("not a protocol error: {other:?}"),
    }
}

fn write(log: &mut dyn BACnetObject, property: P, value: PropertyValue) {
    log.write_property(property, None, value, None).unwrap();
}

fn unsigned(log: &dyn BACnetObject, property: P) -> u64 {
    match log.read_property(property, None).unwrap() {
        PropertyValue::Unsigned(value) => value,
        other => panic!("{property:?} read {other:?}"),
    }
}

/// Every Notification_Threshold records make one NORMAL to NORMAL proposal,
/// and committing it reports the previous and current Total_Record_Count.
#[cfg(feature = "event-reporting")]
#[test]
fn every_threshold_of_records_proposes_one_report_on_each_log() {
    for mut log in logs() {
        let log = log.as_mut();
        let kind = log.object_identifier().object_type();
        write(log, P::NOTIFICATION_THRESHOLD, PropertyValue::Unsigned(3));
        for round in 1..=2u32 {
            for _ in 0..2 {
                add(log);
                assert_eq!(
                    log.event_reporting_mut()
                        .expect("lends event reporting")
                        .tick_intrinsic_reporting(test_tick()),
                    None,
                    "{kind:?}"
                );
            }
            add(log);
            let outcome = log
                .event_reporting_mut()
                .expect("lends event reporting")
                .tick_intrinsic_reporting(test_tick())
                .expect("a report is due");
            assert_eq!(
                log.event_reporting_mut()
                    .expect("lends event reporting")
                    .evaluate_intrinsic_reporting(test_now()),
                Some(outcome.clone())
            );
            assert_eq!(
                outcome,
                TransitionOutcome {
                    change: EventStateChange {
                        from: EventState::NORMAL,
                        to: EventState::NORMAL,
                    },
                    event_type: EventType::BUFFER_READY,
                    distribute: true,
                }
            );
            // Proposing changes nothing until the commit.
            assert_eq!(
                unsigned(log, P::LAST_NOTIFY_RECORD),
                u64::from(3 * (round - 1))
            );
            assert_eq!(unsigned(log, P::RECORDS_SINCE_NOTIFICATION), 3);
            commit_test_proposal(log, outcome);
            assert_eq!(
                log.event_reporting().unwrap().buffer_ready_report(),
                Some(BufferReadyReport {
                    previous_notification: 3 * (round - 1),
                    current_notification: 3 * round,
                }),
                "{kind:?}"
            );
            assert_eq!(unsigned(log, P::LAST_NOTIFY_RECORD), u64::from(3 * round));
            assert_eq!(unsigned(log, P::RECORDS_SINCE_NOTIFICATION), 0);
            assert_eq!(
                log.event_reporting_mut()
                    .expect("lends event reporting")
                    .tick_intrinsic_reporting(test_tick()),
                None,
                "{kind:?}"
            );
            assert_eq!(
                log.read_property(P::EVENT_STATE, None).unwrap(),
                PropertyValue::Enumerated(EventState::NORMAL.to_raw())
            );
        }
        let summary = log
            .event_reporting()
            .expect("lends event reporting")
            .enrollment_summary_capability()
            .unwrap();
        assert_eq!(summary.event_type, EventType::BUFFER_READY);
        assert_eq!(
            summary.last_transition,
            Some(crate::event::EventTransition::ToNormal)
        );
    }
}

#[cfg(feature = "event-reporting")]
#[test]
fn no_threshold_or_detection_off_proposes_nothing() {
    for mut log in logs() {
        let log = log.as_mut();
        for _ in 0..5 {
            add(log);
        }
        // Notification_Threshold starts at zero (pThreshold 0: no transition).
        assert_eq!(unsigned(log, P::NOTIFICATION_THRESHOLD), 0);
        assert_eq!(
            log.event_reporting_mut()
                .expect("lends event reporting")
                .tick_intrinsic_reporting(test_tick()),
            None
        );
        write(
            log,
            P::EVENT_DETECTION_ENABLE,
            PropertyValue::Boolean(false),
        );
        write(log, P::NOTIFICATION_THRESHOLD, PropertyValue::Unsigned(1));
        assert_eq!(
            log.event_reporting_mut()
                .expect("lends event reporting")
                .tick_intrinsic_reporting(test_tick()),
            None
        );
        // Turned on again, the algorithm counts from where the log stands.
        write(log, P::EVENT_DETECTION_ENABLE, PropertyValue::Boolean(true));
        assert_eq!(unsigned(log, P::LAST_NOTIFY_RECORD), 5);
        assert_eq!(unsigned(log, P::RECORDS_SINCE_NOTIFICATION), 0);
        assert_eq!(
            log.event_reporting_mut()
                .expect("lends event reporting")
                .tick_intrinsic_reporting(test_tick()),
            None
        );
        add(log);
        assert!(
            log.event_reporting_mut()
                .expect("lends event reporting")
                .tick_intrinsic_reporting(test_tick())
                .is_some()
        );
    }
}

/// A purge restarts Records_Since_Notification at its BUFFER_PURGED record,
/// but the algorithm still counts from Last_Notify_Record.
#[cfg(feature = "event-reporting")]
#[test]
fn a_purge_restarts_records_since_notification_only() {
    for mut log in logs() {
        let log = log.as_mut();
        write(log, P::NOTIFICATION_THRESHOLD, PropertyValue::Unsigned(4));
        add(log);
        add(log);
        write(log, P::RECORD_COUNT, PropertyValue::Unsigned(0));
        assert_eq!(unsigned(log, P::TOTAL_RECORD_COUNT), 3);
        assert_eq!(unsigned(log, P::RECORDS_SINCE_NOTIFICATION), 1);
        assert_eq!(
            log.event_reporting_mut()
                .expect("lends event reporting")
                .tick_intrinsic_reporting(test_tick()),
            None
        );
        add(log);
        assert!(
            log.event_reporting_mut()
                .expect("lends event reporting")
                .tick_intrinsic_reporting(test_tick())
                .is_some()
        );
    }
}

#[cfg(feature = "event-reporting")]
#[test]
fn event_enable_to_normal_decides_distribution_only() {
    for mut log in logs() {
        let log = log.as_mut();
        write(log, P::NOTIFICATION_THRESHOLD, PropertyValue::Unsigned(1));
        write(
            log,
            P::EVENT_ENABLE,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![EventTransitionBits::TO_OFFNORMAL.to_bacnet()],
            },
        );
        add(log);
        let outcome = log
            .event_reporting_mut()
            .expect("lends event reporting")
            .tick_intrinsic_reporting(test_tick())
            .unwrap();
        assert!(!outcome.distribute);
        commit_test_proposal(log, outcome);
        assert_eq!(unsigned(log, P::LAST_NOTIFY_RECORD), 1);
    }
}

#[test]
fn rows_refuse_bad_writes_and_acked_transitions() {
    for mut log in logs() {
        let log = log.as_mut();
        let refused = |log: &mut dyn BACnetObject, property, value| {
            code(log.write_property(property, None, value, None).unwrap_err())
        };
        let acked = log.read_property(P::ACKED_TRANSITIONS, None).unwrap();
        assert_eq!(
            refused(log, P::ACKED_TRANSITIONS, acked),
            code(common::write_access_denied_error())
        );
        assert_eq!(
            refused(
                log,
                P::NOTIFICATION_THRESHOLD,
                PropertyValue::Unsigned(1 << 32)
            ),
            code(common::value_out_of_range_error())
        );
        assert_eq!(
            refused(log, P::NOTIFICATION_CLASS, PropertyValue::Boolean(true)),
            code(common::invalid_data_type_error())
        );
        assert_eq!(
            refused(log, P::NOTIFY_TYPE, PropertyValue::Enumerated(9)),
            code(common::value_out_of_range_error())
        );
        for property in [P::RECORDS_SINCE_NOTIFICATION, P::LAST_NOTIFY_RECORD] {
            assert_eq!(
                refused(log, property, PropertyValue::Unsigned(0)),
                code(common::write_access_denied_error())
            );
        }
        write(
            log,
            P::NOTIFY_TYPE,
            PropertyValue::Enumerated(NotifyType::ALARM.to_raw()),
        );
        write(log, P::NOTIFICATION_CLASS, PropertyValue::Unsigned(7));
        assert_eq!(unsigned(log, P::NOTIFICATION_CLASS), 7);
        assert_eq!(
            log.read_property(P::NOTIFY_TYPE, None).unwrap(),
            PropertyValue::Enumerated(NotifyType::ALARM.to_raw())
        );
    }
}

/// The counts and the threshold run on across Total_Record_Count's wrap from
/// 2^32 - 1 to 1 (condition (b) of the algorithm).
#[test]
fn the_count_runs_on_across_the_wrap() {
    assert_eq!(records_between(0, 3), 3);
    assert_eq!(records_between(u32::MAX, 1), 1);
    assert_eq!(records_between(u32::MAX - 1, 2), 3);
    let mut reporting = BufferReadyReporting::default();
    reporting.set_notification_threshold(3);
    let commit = |reporting: &mut BufferReadyReporting, total| {
        let outcome = reporting.propose(total).expect("due");
        reporting
            .commit(
                EventTransitionCommit {
                    coordinate: outcome.change.transition(),
                    change: outcome.change,
                    ack_required: false,
                    timestamp: BACnetTimeStamp::SequenceNumber(0),
                    message_text: None,
                },
                total,
            )
            .unwrap();
    };
    commit(&mut reporting, u32::MAX - 1);
    assert_eq!(reporting.propose(1), None);
    commit(&mut reporting, 2);
    assert_eq!(
        reporting.report(),
        Some(BufferReadyReport {
            previous_notification: u32::MAX - 1,
            current_notification: 2,
        })
    );
}

#[test]
fn only_a_report_to_normal_commits() {
    let mut reporting = BufferReadyReporting::default();
    let change = EventStateChange {
        from: EventState::NORMAL,
        to: EventState::OFFNORMAL,
    };
    let refused = reporting.commit(
        EventTransitionCommit {
            coordinate: change.transition(),
            change,
            ack_required: false,
            timestamp: BACnetTimeStamp::SequenceNumber(0),
            message_text: None,
        },
        5,
    );
    assert_eq!(refused, Err(EventTransitionCommitError::Unsupported));
    assert_eq!(reporting.event_state(), EventState::NORMAL);
    assert_eq!(reporting.report(), None);
}

/// Commit a report the recipients must acknowledge, stamped `stamp`.
#[cfg(feature = "event-reporting")]
fn commit_acked_report(log: &mut dyn BACnetObject, stamp: &BACnetTimeStamp) {
    add(log);
    let reporting = log.event_reporting_mut().expect("lends event reporting");
    let outcome = reporting
        .tick_intrinsic_reporting(test_tick())
        .expect("a report is due");
    reporting
        .commit_event_transition(EventTransitionCommit {
            coordinate: outcome.change.transition(),
            change: outcome.change,
            ack_required: true,
            timestamp: stamp.clone(),
            message_text: None,
        })
        .unwrap();
}

#[cfg(feature = "event-reporting")]
fn acked(log: &dyn BACnetObject) -> PropertyValue {
    log.read_property(P::ACKED_TRANSITIONS, None).unwrap()
}

#[cfg(feature = "event-reporting")]
fn acknowledge(
    log: &mut dyn BACnetObject,
    state: EventState,
    stamp: &BACnetTimeStamp,
) -> Result<Option<EventStateChange>, Error> {
    log.event_reporting_mut()
        .expect("lends event reporting")
        .acknowledge_alarm(state, stamp)
}

/// A report needing acknowledgment is acknowledged by its own timestamp.
/// With detection off there is nothing to acknowledge, so the log answers
/// NO_ALARM_CONFIGURED and changes nothing, and turning detection back on
/// lifts that refusal (#2183).
#[cfg(feature = "event-reporting")]
#[test]
fn an_acknowledgment_needs_detection_and_the_reports_timestamp() {
    let all_acked = PropertyValue::BitString {
        unused_bits: 5,
        data: vec![EventTransitionBits::all().to_bacnet()],
    };
    let unacked_normal = PropertyValue::BitString {
        unused_bits: 5,
        data: vec![(EventTransitionBits::TO_OFFNORMAL | EventTransitionBits::TO_FAULT).to_bacnet()],
    };
    let stamp = BACnetTimeStamp::SequenceNumber(42);
    let other = BACnetTimeStamp::SequenceNumber(43);
    let invalid_stamp = (ErrorClass::SERVICES, ErrorCode::INVALID_TIMESTAMP);
    for mut log in logs() {
        let log = log.as_mut();
        write(log, P::NOTIFICATION_THRESHOLD, PropertyValue::Unsigned(1));
        commit_acked_report(log, &stamp);
        assert_eq!(acked(log), unacked_normal);

        // Another timestamp, or a state the log never entered, is refused.
        assert_eq!(
            code(acknowledge(log, EventState::NORMAL, &other).unwrap_err()),
            invalid_stamp
        );
        assert_eq!(
            code(acknowledge(log, EventState::OFFNORMAL, &stamp).unwrap_err()),
            invalid_stamp
        );
        assert_eq!(acked(log), unacked_normal);
        assert_eq!(
            acknowledge(log, EventState::NORMAL, &stamp).unwrap(),
            Some(EventStateChange {
                from: EventState::NORMAL,
                to: EventState::NORMAL,
            })
        );
        assert_eq!(acked(log), all_acked);

        // Detection off: the refusal changes nothing.
        commit_acked_report(log, &other);
        write(
            log,
            P::EVENT_DETECTION_ENABLE,
            PropertyValue::Boolean(false),
        );
        assert_eq!(
            code(acknowledge(log, EventState::NORMAL, &other).unwrap_err()),
            (ErrorClass::OBJECT, ErrorCode::NO_ALARM_CONFIGURED)
        );
        assert_eq!(acked(log), all_acked);

        // Back on, the log answers again: the reset history holds no report.
        write(log, P::EVENT_DETECTION_ENABLE, PropertyValue::Boolean(true));
        assert_eq!(
            code(acknowledge(log, EventState::NORMAL, &other).unwrap_err()),
            invalid_stamp
        );
        commit_acked_report(log, &stamp);
        assert!(acknowledge(log, EventState::NORMAL, &stamp).is_ok());
        assert_eq!(acked(log), all_acked);
    }
}

/// Event_Algorithm_Inhibit follows its reference only once one is set.
#[cfg(feature = "event-reporting")]
#[test]
fn the_inhibit_follows_only_a_configured_reference() {
    let binary_value = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap();
    let reference = BACnetObjectPropertyReference::new(binary_value, P::PRESENT_VALUE.to_raw());
    for mut log in logs() {
        let log = log.as_mut();
        let reporting = log.event_reporting_mut().unwrap();
        assert_eq!(reporting.event_algorithm_inhibit_reference(), None);
        assert!(!reporting.follow_event_algorithm_inhibit(true));
        let mut octets = bytes::BytesMut::new();
        bacnet_encoding::constructed::encode_object_property_reference(&mut octets, &reference);
        write(
            log,
            P::EVENT_ALGORITHM_INHIBIT_REF,
            PropertyValue::ApplicationData(octets.to_vec()),
        );
        let reporting = log.event_reporting_mut().unwrap();
        assert_eq!(
            reporting.event_algorithm_inhibit_reference(),
            Some(reference.clone())
        );
        assert!(reporting.follow_event_algorithm_inhibit(true));
        assert!(!reporting.follow_event_algorithm_inhibit(true));
        assert_eq!(
            log.read_property(P::EVENT_ALGORITHM_INHIBIT, None).unwrap(),
            PropertyValue::Boolean(true)
        );
        assert!(
            log.event_reporting_mut()
                .unwrap()
                .follow_event_algorithm_inhibit(false)
        );
    }
}

/// The trend logs' setters for the event rows the network also writes.
#[test]
fn the_trend_logs_set_event_enable_and_notify_type() {
    let enable = EventTransitionBits::TO_OFFNORMAL;
    let expected = PropertyValue::BitString {
        unused_bits: 5,
        data: vec![enable.to_bacnet()],
    };
    let alarm = PropertyValue::Enumerated(NotifyType::ALARM.to_raw());
    let mut trend = TrendLogObject::new(1, "TL-1", 4).unwrap();
    trend.set_event_enable(enable);
    trend.set_notify_type(NotifyType::ALARM);
    let mut multiple = TrendLogMultipleObject::new(1, "TLM-1", 4).unwrap();
    multiple.set_event_enable(enable);
    multiple.set_notify_type(NotifyType::ALARM);
    for log in [&trend as &dyn BACnetObject, &multiple] {
        assert_eq!(log.read_property(P::EVENT_ENABLE, None).unwrap(), expected);
        assert_eq!(log.read_property(P::NOTIFY_TYPE, None).unwrap(), alarm);
    }
}

/// `BUFFER_READY_METADATA` is a `const`, filled in at build time, so a
/// coverage run sees `required` and `permitted` as never called. Calling
/// them here at run time checks they build the table's rows (#2183).
#[test]
fn the_event_rows_are_what_their_builders_make_at_run_time() {
    use crate::property_metadata::PropertyWriteCapability::{Always, ReadOnly};
    use core::hint::black_box;
    assert_eq!(
        required(black_box(P::NOTIFICATION_THRESHOLD), Always),
        BUFFER_READY_METADATA[0]
    );
    assert_eq!(
        permitted(black_box(P::EVENT_MESSAGE_TEXTS), ReadOnly),
        BUFFER_READY_METADATA[8]
    );
}
