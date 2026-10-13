//! ReadRange of a log whose Total_Record_Count was seeded or restored near
//! the Unsigned32 wrap (#1537): records number on across 2^32 - 1 to 1, and
//! By Sequence Number and By Time page across it.
use super::*;

const MAX: u64 = u32::MAX as u64;

/// A log of `family` restored with the samples `values`, the newest
/// numbered `total`, then given the samples `added`.
fn restored_log(family: LogFamily, total: u32, values: &[u64], added: &[u64]) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    match family {
        LogFamily::Trend => {
            let mut log = TrendLogObject::new(1, "TL-1", 10).unwrap();
            log.restore_log_buffer(total, values.iter().copied().map(record))
                .unwrap();
            for &value in added {
                log.add_record(record(value)).unwrap();
            }
            db.add(Box::new(log)).unwrap();
        }
        LogFamily::Event => {
            let mut log = EventLogObject::new(1, "EL-1", 10).unwrap();
            log.restore_log_buffer(total, values.iter().copied().map(event_record))
                .unwrap();
            for &value in added {
                log.add_record(event_record(value)).unwrap();
            }
            db.add(Box::new(log)).unwrap();
        }
        LogFamily::TrendMultiple => {
            let mut log = TrendLogMultipleObject::new(1, "TLM-1", 10).unwrap();
            log.restore_log_buffer(total, values.iter().copied().map(multiple_record))
                .unwrap();
            for &value in added {
                log.add_record(multiple_record(value)).unwrap();
            }
            db.add(Box::new(log)).unwrap();
        }
    }
    db
}

fn oid(family: LogFamily) -> ObjectIdentifier {
    let object_type = match family {
        LogFamily::Trend => ObjectType::TREND_LOG,
        LogFamily::Event => ObjectType::EVENT_LOG,
        LogFamily::TrendMultiple => ObjectType::TREND_LOG_MULTIPLE,
    };
    ObjectIdentifier::new(object_type, 1).unwrap()
}

fn read(db: &ObjectDatabase, family: LogFamily, range: RangeSpec) -> ReadRangeAck {
    call(db, oid(family), PropertyIdentifier::LOG_BUFFER, Some(range)).unwrap()
}

fn by_sequence(reference_seq: u64, count: i32) -> RangeSpec {
    RangeSpec::BySequenceNumber {
        reference_seq,
        count,
    }
}

fn items(family: LogFamily, values: &[u64]) -> Vec<PropertyValue> {
    values
        .iter()
        .map(|&value| projected(family, value))
        .collect()
}

const FAMILIES: [LogFamily; 3] = [LogFamily::Trend, LogFamily::Event, LogFamily::TrendMultiple];

#[test]
fn a_count_seeded_near_the_wrap_pages_across_it_by_sequence() {
    for family in FAMILIES {
        // Seeded at 2^32 - 3, the samples 1 to 5 are numbered 2^32 - 2,
        // 2^32 - 1, 1, 2 and 3.
        let db = restored_log(family, u32::MAX - 2, &[], &[1, 2, 3, 4, 5]);
        let ack = read(&db, family, by_sequence(MAX, 3));
        assert_ack(
            &ack,
            &items(family, &[2, 3, 4]),
            (false, false, false),
            Some(MAX),
        );
        let back = read(&db, family, by_sequence(2, -3));
        assert_ack(
            &back,
            &items(family, &[2, 3, 4]),
            (false, false, false),
            Some(MAX),
        );
        let all = read(&db, family, by_sequence(MAX - 1, 10));
        assert_ack(
            &all,
            &items(family, &[1, 2, 3, 4, 5]),
            (true, true, false),
            Some(MAX - 1),
        );
        // A reader continuing from the ACK's last record goes on from 1.
        let next = read(&db, family, by_sequence(1, 2));
        assert_ack(
            &next,
            &items(family, &[3, 4]),
            (false, false, false),
            Some(1),
        );
        let by_time = read(
            &db,
            family,
            RangeSpec::ByTime {
                reference_time: (DATE, time(2)),
                count: 2,
            },
        );
        assert_ack(
            &by_time,
            &items(family, &[3, 4]),
            (false, false, false),
            Some(1),
        );
    }
}

#[test]
fn restored_records_that_straddle_the_wrap_page_by_sequence() {
    for family in FAMILIES {
        // A count of 1 below three records: they were numbered 2^32 - 2,
        // 2^32 - 1 and 1, and the next is 2.
        let db = restored_log(family, 1, &[1, 2, 3], &[4]);
        let ack = read(&db, family, by_sequence(MAX, -2));
        assert_ack(
            &ack,
            &items(family, &[1, 2]),
            (true, false, false),
            Some(MAX - 1),
        );
        let tail = read(&db, family, by_sequence(1, 5));
        assert_ack(
            &tail,
            &items(family, &[3, 4]),
            (false, true, false),
            Some(1),
        );
        let gone = read(&db, family, by_sequence(MAX - 2, 1));
        assert_ack(&gone, &[], (false, false, false), None);
    }
}
