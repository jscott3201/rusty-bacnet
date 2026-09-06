use super::*;
use bacnet_types::constructed::LogDatum;
use bacnet_types::primitives::{Date, Time};

fn record(hour: u8, value: f32, status_flags: Option<u8>) -> BACnetLogRecord {
    BACnetLogRecord {
        date: Date {
            year: 126,
            month: 8,
            day: 31,
            day_of_week: 1,
        },
        time: Time {
            hour,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        log_datum: LogDatum::RealValue(value),
        status_flags,
    }
}

fn projected_records(object: &dyn BACnetObject) -> Vec<PropertyValue> {
    match object
        .read_property(PropertyIdentifier::LOG_BUFFER, None)
        .unwrap()
    {
        PropertyValue::List(records) => records,
        other => panic!("expected projected log list, got {other:?}"),
    }
}

#[test]
fn trend_log_projects_only_present_status_flags_as_fourth_field() {
    let mut trend = TrendLogObject::new(1, "TL-1", 2).unwrap();
    trend.add_record(record(1, 10.0, Some(0b0100)));
    trend.add_record(record(1, 20.0, None));

    let identities = trend.log_record_identities_internal().unwrap();
    assert_eq!(identities[0].sequence_number(), 1);
    assert_eq!(identities[1].sequence_number(), 2);
    assert_eq!(identities[0].date(), identities[1].date());
    assert_eq!(identities[0].time(), identities[1].time());
    assert_eq!(trend.records()[0].status_flags, Some(0b0100));

    let projected = projected_records(&trend);
    let PropertyValue::List(with_status) = &projected[0] else {
        panic!("expected projected record fields");
    };
    assert_eq!(with_status.len(), 4);
    assert_eq!(with_status[2], PropertyValue::Real(10.0));
    assert_eq!(
        with_status[3],
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0b0100_0000],
        }
    );
    let PropertyValue::List(without_status) = &projected[1] else {
        panic!("expected projected record fields");
    };
    assert_eq!(without_status.len(), 3);
    assert_eq!(without_status[2], PropertyValue::Real(20.0));
}

#[test]
fn trend_log_keeps_log_status_and_record_status_flags_as_distinct_bitstrings() {
    let mut trend = TrendLogObject::new(1, "TL-1", 1).unwrap();
    let mut status = record(1, 0.0, Some(0b0100));
    status.log_datum = LogDatum::LogStatus(0b101);
    trend.add_record(status);

    let projected = projected_records(&trend);
    let PropertyValue::List(fields) = &projected[0] else {
        panic!("expected projected record fields");
    };
    assert_eq!(fields.len(), 4);
    assert_eq!(
        fields[2],
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1010_0000],
        }
    );
    assert_eq!(
        fields[3],
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0b0100_0000],
        }
    );
}

#[test]
fn trend_multiple_retains_raw_flags_but_projects_three_fields() {
    let mut trend = TrendLogMultipleObject::new(1, "TLM-1", 1).unwrap();
    trend.add_record(record(1, 10.0, Some(0b0100)));

    assert_eq!(trend.records()[0].status_flags, Some(0b0100));
    let projected = projected_records(&trend);
    let PropertyValue::List(fields) = &projected[0] else {
        panic!("expected projected record fields");
    };
    assert_eq!(fields.len(), 3);
    assert_eq!(fields[2], PropertyValue::Real(10.0));
}

#[test]
fn trend_family_identity_raw_and_projection_views_stay_fifo_aligned() {
    let mut trend = TrendLogObject::new(1, "TL-1", 2).unwrap();
    let mut multiple = TrendLogMultipleObject::new(1, "TLM-1", 2).unwrap();
    for hour in 1..=3 {
        trend.add_record(record(hour, hour as f32, None));
        multiple.add_record(record(hour, hour as f32, Some(0b0001)));
    }

    for object in [&trend as &dyn BACnetObject, &multiple as &dyn BACnetObject] {
        let identities = object.log_record_identities_internal().unwrap();
        let projected = projected_records(object);
        assert_eq!(identities.len(), 2);
        assert_eq!(identities.len(), projected.len());
        assert_eq!(identities[0].sequence_number(), 2);
        assert_eq!(identities[1].sequence_number(), 3);
        for (identity, wire) in identities.iter().zip(projected) {
            let PropertyValue::List(fields) = wire else {
                panic!("expected projected record fields");
            };
            assert_eq!(fields[0], PropertyValue::Date(identity.date()));
            assert_eq!(fields[1], PropertyValue::Time(identity.time()));
            assert_eq!(fields.len(), 3);
        }
    }
}
