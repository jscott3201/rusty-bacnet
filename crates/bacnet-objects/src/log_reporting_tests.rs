//! BUFFER_READY on the three log objects (#1347, Clause 13.3.7).
use super::*;
use crate::clock::{ClockFrame, ClockReader};
use crate::event::commit_test_proposal;
use crate::event_log::EventLogObject;
use crate::traits::BACnetObject;
use crate::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, EventLogDatum, LogData,
    LogDatum,
};
use bacnet_types::enums::ObjectType;
use bacnet_types::error::RemoteError;
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
        log.bind_clock_internal(Some(Arc::new(FixedClock)));
    }
    logs
}

/// Add one ordinary record of the log's own family.
fn add(log: &mut dyn BACnetObject) {
    match log.object_identifier().object_type() {
        ObjectType::TREND_LOG => log.add_trend_record(BACnetLogRecord {
            date: DATE,
            time: TIME,
            log_datum: LogDatum::UnsignedValue(1),
            status_flags: None,
        }),
        ObjectType::EVENT_LOG => log.add_event_log_record(BACnetEventLogRecord {
            date: DATE,
            time: TIME,
            log_datum: EventLogDatum::TimeChange(1.0),
        }),
        _ => log.add_trend_multiple_record(BACnetLogMultipleRecord {
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
#[test]
fn every_threshold_of_records_proposes_one_report_on_each_log() {
    for mut log in logs() {
        let log = log.as_mut();
        let kind = log.object_identifier().object_type();
        write(log, P::NOTIFICATION_THRESHOLD, PropertyValue::Unsigned(3));
        for round in 1..=2u32 {
            for _ in 0..2 {
                add(log);
                assert_eq!(log.tick_intrinsic_reporting(), None, "{kind:?}");
            }
            add(log);
            let outcome = log.tick_intrinsic_reporting().expect("a report is due");
            assert_eq!(log.evaluate_intrinsic_reporting(), Some(outcome.clone()));
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
                log.buffer_ready_report_internal(),
                Some(BufferReadyReport {
                    previous_notification: 3 * (round - 1),
                    current_notification: 3 * round,
                }),
                "{kind:?}"
            );
            assert_eq!(unsigned(log, P::LAST_NOTIFY_RECORD), u64::from(3 * round));
            assert_eq!(unsigned(log, P::RECORDS_SINCE_NOTIFICATION), 0);
            assert_eq!(log.tick_intrinsic_reporting(), None, "{kind:?}");
            assert_eq!(
                log.read_property(P::EVENT_STATE, None).unwrap(),
                PropertyValue::Enumerated(EventState::NORMAL.to_raw())
            );
        }
        let summary = log.enrollment_summary_capability_internal().unwrap();
        assert_eq!(summary.event_type, EventType::BUFFER_READY);
        assert_eq!(
            summary.last_transition,
            Some(crate::event::EventTransition::ToNormal)
        );
    }
}

#[test]
fn no_threshold_or_detection_off_proposes_nothing() {
    for mut log in logs() {
        let log = log.as_mut();
        for _ in 0..5 {
            add(log);
        }
        // Notification_Threshold starts at zero (pThreshold 0: no transition).
        assert_eq!(unsigned(log, P::NOTIFICATION_THRESHOLD), 0);
        assert_eq!(log.tick_intrinsic_reporting(), None);
        write(
            log,
            P::EVENT_DETECTION_ENABLE,
            PropertyValue::Boolean(false),
        );
        write(log, P::NOTIFICATION_THRESHOLD, PropertyValue::Unsigned(1));
        assert_eq!(log.tick_intrinsic_reporting(), None);
        // Turned on again, the algorithm counts from where the log stands.
        write(log, P::EVENT_DETECTION_ENABLE, PropertyValue::Boolean(true));
        assert_eq!(unsigned(log, P::LAST_NOTIFY_RECORD), 5);
        assert_eq!(unsigned(log, P::RECORDS_SINCE_NOTIFICATION), 0);
        assert_eq!(log.tick_intrinsic_reporting(), None);
        add(log);
        assert!(log.tick_intrinsic_reporting().is_some());
    }
}

/// A purge restarts Records_Since_Notification at its BUFFER_PURGED record,
/// but the algorithm still counts from Last_Notify_Record.
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
        assert_eq!(log.tick_intrinsic_reporting(), None);
        add(log);
        assert!(log.tick_intrinsic_reporting().is_some());
    }
}

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
        let outcome = log.tick_intrinsic_reporting().unwrap();
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
