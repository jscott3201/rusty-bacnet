//! By-Time reads of a Trend Log, whose buffer bisects its timestamps while
//! they run in order (#1536): the edges of a run of equal timestamps, a
//! buffer stamped all alike, and a clock set back right at the anchor,
//! which the log walks instead.
use super::*;
use bacnet_objects::log_buffer::TimestampOrder;

/// A Trend Log holding samples 1, 2, ... stamped at `hours` in turn.
fn stamped_log(hours: &[u8]) -> (ObjectDatabase, ObjectIdentifier, TimestampOrder) {
    let mut log = TrendLogObject::new(1, "TL-1", 10).unwrap();
    for (sample, &hour) in hours.iter().enumerate() {
        log.add_record(BACnetLogRecord {
            time: time(hour),
            ..record(sample as u64 + 1)
        })
        .unwrap();
    }
    let order = log.log_buffer_internal().unwrap().timestamp_order();
    let oid = log.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(log)).unwrap();
    (db, oid, order)
}

/// The samples `values` as a Trend Log's ReadRange items, each stamped at
/// its hour in `hours`.
fn samples(hours: &[u8], values: &[u64]) -> Vec<PropertyValue> {
    values
        .iter()
        .map(|&value| {
            let mut framed = BytesMut::new();
            let record = BACnetLogRecord {
                time: time(hours[value as usize - 1]),
                ..record(value)
            };
            bacnet_encoding::constructed::encode_log_record(&record, &mut framed).unwrap();
            PropertyValue::ApplicationData(framed.to_vec())
        })
        .collect()
}

fn by_time(db: &ObjectDatabase, oid: ObjectIdentifier, hour: u8, count: i32) -> ReadRangeAck {
    call(
        db,
        oid,
        PropertyIdentifier::LOG_BUFFER,
        Some(RangeSpec::ByTime {
            reference_time: (DATE, time(hour)),
            count,
        }),
    )
    .unwrap()
}

#[test]
fn a_run_of_equal_timestamps_is_skipped_from_either_side() {
    let hours = [1, 2, 2, 2, 3];
    let (db, oid, order) = stamped_log(&hours);
    assert_eq!(order, TimestampOrder::Ascending);
    // After 2:00 is the record stamped 3:00; before it, the one at 1:00.
    let after = by_time(&db, oid, 2, 2);
    assert_ack(
        &after,
        &samples(&hours, &[5]),
        (false, true, false),
        Some(5),
    );
    let before = by_time(&db, oid, 2, -2);
    assert_ack(
        &before,
        &samples(&hours, &[1]),
        (true, false, false),
        Some(1),
    );
    // From the far side of the run, it is read whole.
    let into = by_time(&db, oid, 1, 3);
    assert_ack(
        &into,
        &samples(&hours, &[2, 3, 4]),
        (false, false, false),
        Some(2),
    );
    let back_into = by_time(&db, oid, 3, -3);
    assert_ack(
        &back_into,
        &samples(&hours, &[2, 3, 4]),
        (false, false, false),
        Some(2),
    );
}

#[test]
fn a_buffer_stamped_all_alike_has_nothing_on_either_side_of_its_time() {
    let hours = [2, 2, 2, 2];
    let (db, oid, order) = stamped_log(&hours);
    assert_eq!(order, TimestampOrder::Ascending);
    for count in [1, -1, 4, -4] {
        assert_ack(
            &by_time(&db, oid, 2, count),
            &[],
            (false, false, false),
            None,
        );
    }
    let after_earlier = by_time(&db, oid, 1, 2);
    assert_ack(
        &after_earlier,
        &samples(&hours, &[1, 2]),
        (true, false, false),
        Some(1),
    );
    let before_later = by_time(&db, oid, 3, -2);
    assert_ack(
        &before_later,
        &samples(&hours, &[3, 4]),
        (false, true, false),
        Some(3),
    );
}

#[test]
fn a_clock_set_back_at_the_anchor_is_walked_not_bisected() {
    // The clock goes back from 3:00 to 2:00 right after the first record
    // stamped after 2:00, which a bisection would step over.
    let hours = [1, 3, 2, 4];
    let (db, oid, order) = stamped_log(&hours);
    assert_eq!(order, TimestampOrder::Unordered);
    let after = by_time(&db, oid, 2, 2);
    assert_ack(
        &after,
        &samples(&hours, &[2, 3]),
        (false, false, false),
        Some(2),
    );
    let before = by_time(&db, oid, 3, -2);
    assert_ack(
        &before,
        &samples(&hours, &[2, 3]),
        (false, false, false),
        Some(2),
    );
}
