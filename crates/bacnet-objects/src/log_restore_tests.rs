//! Restoring or seeding a log's buffer and Total_Record_Count (#1537).
use super::*;

const MAX: u64 = u32::MAX as u64;

fn samples(hours: std::ops::Range<u8>) -> Vec<BACnetLogRecord> {
    hours.map(|hour| ordinary(hour, u64::from(hour))).collect()
}

/// What a refused restore must leave as it was.
fn state(object: &Family) -> (VecDeque<BACnetLogRecord>, Vec<u64>, u64) {
    (object.records(), object.identities(), object.total())
}

#[test]
fn restored_records_number_back_from_the_count_across_the_wrap() {
    for kind in FamilyKind::ALL {
        let mut log = kind.object(5);
        log.restore(10, samples(1..4)).unwrap();
        assert_eq!(log.identities(), vec![8, 9, 10], "{kind:?}");
        assert_eq!(log.records(), VecDeque::from(samples(1..4)), "{kind:?}");

        // A count below the number of records has wrapped (Clause 12.25.16).
        log.restore(2, samples(1..5)).unwrap();
        assert_eq!(log.identities(), vec![MAX - 1, MAX, 1, 2], "{kind:?}");
        assert_eq!(log.total(), 2, "{kind:?}");
        assert_eq!(
            log.read(PropertyIdentifier::RECORD_COUNT),
            PropertyValue::Unsigned(4),
            "{kind:?}"
        );
        log.add_record(ordinary(9, 9)).unwrap();
        assert_eq!(log.identities(), vec![MAX - 1, MAX, 1, 2, 3], "{kind:?}");
        assert_eq!(log.total(), 3, "{kind:?}");
    }
}

#[test]
fn a_seed_near_the_wrap_numbers_new_records_across_it() {
    for kind in FamilyKind::ALL {
        let mut log = kind.object(5);
        log.add_record(ordinary(1, 1)).unwrap();
        log.restore(u32::MAX - 1, Vec::new()).unwrap();
        assert!(log.records().is_empty(), "{kind:?}");
        assert_eq!(log.total(), MAX - 1, "{kind:?}");
        for hour in 2..5 {
            log.add_record(ordinary(hour, u64::from(hour))).unwrap();
        }
        assert_eq!(log.identities(), vec![MAX, 1, 2], "{kind:?}");
        assert_eq!(log.total(), 2, "{kind:?}");
    }
}

#[test]
fn inconsistent_restores_are_refused_and_change_nothing() {
    for kind in FamilyKind::ALL {
        let mut log = kind.object(3);
        log.bind_clock(TestClock::valid());
        log.add_record(ordinary(1, 1)).unwrap();
        log.write(
            PropertyIdentifier::STOP_WHEN_FULL,
            PropertyValue::Boolean(true),
        )
        .unwrap();
        let before = state(&log);
        for (total, records) in [
            // More records than Buffer_Size.
            (10, samples(1..5)),
            // Records that no count has numbered.
            (0, samples(1..2)),
            // A full buffer under Stop_When_Full while Enable is TRUE.
            (3, samples(1..4)),
        ] {
            let error = log.restore(total, records).unwrap_err();
            assert!(matches!(error, Error::OutOfRange(_)), "{kind:?} {error:?}");
            assert_eq!(state(&log), before, "{kind:?} {total}");
        }

        // One short of full is a state the log reaches itself.
        log.restore(2, samples(1..3)).unwrap();
        assert_eq!(log.identities(), vec![1, 2], "{kind:?}");
        // With Enable FALSE, a stopped log may be full.
        log.write(
            PropertyIdentifier::LOG_ENABLE,
            PropertyValue::Boolean(false),
        )
        .unwrap();
        log.restore(3, samples(1..4)).unwrap();
        assert_eq!(log.identities(), vec![1, 2, 3], "{kind:?}");
    }
}

#[test]
fn a_zero_capacity_log_restores_only_its_count() {
    for kind in FamilyKind::ALL {
        let mut log = kind.object(0);
        log.restore(5, Vec::new()).unwrap();
        assert_eq!(log.total(), 5, "{kind:?}");
        assert!(matches!(
            log.restore(6, samples(1..2)).unwrap_err(),
            Error::OutOfRange(_)
        ));
        assert_eq!(log.total(), 5, "{kind:?}");
        log.restore(0, Vec::new()).unwrap();
        assert_eq!(log.total(), 0, "{kind:?}");
    }
}

#[test]
fn a_record_that_would_not_encode_is_refused() {
    let mut log = TrendLogObject::new(1, "TL-1", 5).unwrap();
    log.add_record(ordinary(1, 1)).unwrap();
    let records = log.records().clone();
    // An any-value that opens a context tag and never closes it.
    let open = BACnetLogRecord {
        log_datum: LogDatum::AnyValue(vec![0x3E, 0x19, 0x05]),
        ..ordinary(2, 2)
    };
    log.restore_log_buffer(7, [ordinary(3, 3), open])
        .unwrap_err();
    assert_eq!(log.records(), &records);
    assert_eq!(log.total_record_count(), 1);

    // The same member value in a Trend Log Multiple record. The Event Log's
    // case is in its own tests, which can build a notification.
    let mut multiple_log = TrendLogMultipleObject::new(1, "TLM-1", 5).unwrap();
    multiple_log.add_record(multiple(ordinary(1, 1))).unwrap();
    let records = multiple_log.records().clone();
    let open = BACnetLogMultipleRecord {
        log_data: LogData::Values(vec![LogValue::AnyValue(vec![0x3E, 0x19, 0x05])]),
        ..multiple(ordinary(2, 2))
    };
    multiple_log
        .restore_log_buffer(7, [multiple(ordinary(3, 3)), open])
        .unwrap_err();
    assert_eq!(multiple_log.records(), &records);
    assert_eq!(multiple_log.total_record_count(), 1);
}

#[test]
fn only_a_record_fills_a_stop_when_full_buffer() {
    // A zero-capacity buffer holds nothing, so even under Stop_When_Full
    // with Enable TRUE its count alone restores. The lifecycle can't reach
    // that pair of settings at capacity 0, so the buffer is asked directly.
    let mut buffer = crate::log_buffer::LogRecordBuffer::<BACnetLogRecord>::new(0);
    buffer.restore(5, VecDeque::new(), true).unwrap();
    assert_eq!(buffer.total_record_count(), 5);
    let mut one = crate::log_buffer::LogRecordBuffer::<BACnetLogRecord>::new(1);
    assert!(matches!(
        one.restore(5, VecDeque::from(samples(1..2)), true),
        Err(Error::OutOfRange(_))
    ));
    one.restore(5, VecDeque::new(), true).unwrap();
}

/// The record a family's LOG_INTERRUPTED status is stamped with.
fn at() -> (Date, Time) {
    (valid_frame().local_date, valid_frame().local_time)
}

fn status_of(log: &Family) -> LogDatum {
    log.records().back().unwrap().log_datum.clone()
}

#[test]
fn an_interruption_after_a_restore_is_numbered_past_it() {
    for kind in FamilyKind::ALL {
        let mut log = kind.object(3);
        log.restore(u32::MAX, samples(1..3)).unwrap();
        let (date, time) = at();
        log.interrupt(date, time).unwrap();
        assert_eq!(log.identities(), vec![MAX - 1, MAX, 1], "{kind:?}");
        assert_eq!(log.total(), 1, "{kind:?}");
        assert_eq!(
            status_of(&log),
            LogDatum::LogStatus(LogStatus::LOG_INTERRUPTED),
            "{kind:?}"
        );
        let last = log.records().back().unwrap().clone();
        assert_eq!((last.date, last.time), at(), "{kind:?}");
        // A full buffer gives up its oldest record for it.
        log.interrupt(date, time).unwrap();
        assert_eq!(log.identities(), vec![MAX, 1, 2], "{kind:?}");
    }
}

#[test]
fn an_interruption_carries_log_disabled_while_collection_is_off() {
    for kind in FamilyKind::ALL {
        let (date, time) = at();
        // Enable FALSE.
        let mut off = kind.object(5);
        off.bind_clock(TestClock::valid());
        off.write(
            PropertyIdentifier::LOG_ENABLE,
            PropertyValue::Boolean(false),
        )
        .unwrap();
        off.interrupt(date, time).unwrap();
        assert_eq!(
            status_of(&off),
            LogDatum::LogStatus(LogStatus::LOG_INTERRUPTED | LOG_DISABLED),
            "{kind:?}"
        );

        // Filling a Stop_When_Full buffer stops the log, as its own status
        // records do.
        let mut filling = kind.object(3);
        filling
            .write(
                PropertyIdentifier::STOP_WHEN_FULL,
                PropertyValue::Boolean(true),
            )
            .unwrap();
        filling.restore(2, samples(1..3)).unwrap();
        filling.interrupt(date, time).unwrap();
        assert_eq!(
            status_of(&filling),
            LogDatum::LogStatus(LogStatus::LOG_INTERRUPTED | LOG_DISABLED),
            "{kind:?}"
        );
        assert!(!filling.enabled(), "{kind:?}");
        assert_eq!(filling.records().len(), 3, "{kind:?}");

        // Room left: collection stays on and the flag stays clear.
        let mut room = kind.object(3);
        room.write(
            PropertyIdentifier::STOP_WHEN_FULL,
            PropertyValue::Boolean(true),
        )
        .unwrap();
        room.interrupt(date, time).unwrap();
        assert_eq!(
            status_of(&room),
            LogDatum::LogStatus(LogStatus::LOG_INTERRUPTED),
            "{kind:?}"
        );
        assert!(room.enabled(), "{kind:?}");
    }
}

#[test]
fn an_interruption_needs_an_actual_moment() {
    let (date, time) = at();
    let wrong_weekday = Date {
        day_of_week: date.day_of_week % 7 + 1,
        ..date
    };
    let unspecified = Time {
        second: Time::UNSPECIFIED,
        ..time
    };
    for kind in FamilyKind::ALL {
        let mut log = kind.object(3);
        log.restore(9, samples(1..3)).unwrap();
        let before = state(&log);
        for (date, time) in [(wrong_weekday, time), (date, unspecified)] {
            assert!(
                matches!(log.interrupt(date, time), Err(Error::OutOfRange(_))),
                "{kind:?}"
            );
            assert_eq!(state(&log), before, "{kind:?}");
        }
    }
}

#[test]
fn buffer_ready_counts_from_the_restored_count() {
    for kind in FamilyKind::ALL {
        let mut log = kind.object(10);
        log.write(
            PropertyIdentifier::NOTIFICATION_THRESHOLD,
            PropertyValue::Unsigned(2),
        )
        .unwrap();
        log.restore(1_000, samples(1..4)).unwrap();
        // The jump from 0 to 1,000 is no collection: nothing is due.
        assert!(
            log.object_mut().evaluate_intrinsic_reporting().is_none(),
            "{kind:?}"
        );
        assert_eq!(
            log.read(PropertyIdentifier::LAST_NOTIFY_RECORD),
            PropertyValue::Unsigned(1_000),
            "{kind:?}"
        );
        assert_eq!(
            log.read(PropertyIdentifier::RECORDS_SINCE_NOTIFICATION),
            PropertyValue::Unsigned(0),
            "{kind:?}"
        );
        log.add_record(ordinary(5, 5)).unwrap();
        assert!(log.object_mut().evaluate_intrinsic_reporting().is_none());
        log.add_record(ordinary(6, 6)).unwrap();
        assert!(
            log.object_mut().evaluate_intrinsic_reporting().is_some(),
            "{kind:?}"
        );
    }
}
