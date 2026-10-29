use super::*;
use bacnet_encoding::constructed::{decode_log_multiple_record, decode_log_record};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{BACnetLogMultipleRecord, LogData, LogDatum, LogValue};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::{Date, StatusFlags, Time};
use bytes::BytesMut;

fn record(hour: u8, value: f32, status_flags: Option<StatusFlags>) -> BACnetLogRecord {
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

fn multiple_record(hour: u8, values: Vec<LogValue>) -> BACnetLogMultipleRecord {
    let single = record(hour, 0.0, None);
    BACnetLogMultipleRecord {
        date: single.date,
        time: single.time,
        log_data: LogData::Values(values),
    }
}

/// Each resident record as ReadRange serves it.
fn served(object: &dyn BACnetObject) -> Vec<Vec<u8>> {
    let records = object.log_buffer_internal().unwrap();
    (0..records.record_count())
        .map(|index| {
            let mut buf = BytesMut::new();
            records.encode_record(index, &mut buf);
            buf.to_vec()
        })
        .collect()
}

/// The timestamp field of [`record`] at `hour`.
fn timestamp(hour: u8) -> Vec<u8> {
    vec![
        0x0E, 0xA4, 0x7E, 0x08, 0x1F, 0x01, 0xB4, hour, 0x00, 0x00, 0x00, 0x0F,
    ]
}

/// Clauses 12.25.14 and 12.30.19 open the buffer to ReadRange only (#1237).
#[test]
fn trend_logs_refuse_read_property_of_log_buffer() {
    let mut trend = TrendLogObject::new(1, "TL-1", 2).unwrap();
    trend.add_record(record(1, 10.0, None)).unwrap();
    let mut multiple = TrendLogMultipleObject::new(1, "TLM-1", 2).unwrap();
    multiple
        .add_record(multiple_record(1, vec![LogValue::RealValue(10.0)]))
        .unwrap();
    for object in [&trend as &dyn BACnetObject, &multiple] {
        match object.read_property(PropertyIdentifier::LOG_BUFFER, None) {
            Err(Error::Remote(RemoteError::Error { class, code })) => {
                assert_eq!(class, ErrorClass::PROPERTY);
                assert_eq!(code, ErrorCode::READ_ACCESS_DENIED);
            }
            other => panic!("expected READ_ACCESS_DENIED, got {other:?}"),
        }
        assert_eq!(object.log_buffer_internal().unwrap().record_count(), 1);
    }
}

/// A Trend Log record carries status-flags [2] after its datum only when it
/// recorded them (#1233).
#[test]
fn trend_log_serves_status_flags_only_when_recorded() {
    let mut trend = TrendLogObject::new(1, "TL-1", 2).unwrap();
    trend
        .add_record(record(1, 10.0, Some(StatusFlags::FAULT)))
        .unwrap();
    trend.add_record(record(1, 20.0, None)).unwrap();

    let identities = trend.log_record_identities_internal().unwrap();
    assert_eq!(identities[0].sequence_number(), 1);
    assert_eq!(identities[1].sequence_number(), 2);
    assert_eq!(identities[0].date(), identities[1].date());
    assert_eq!(identities[0].time(), identities[1].time());
    assert_eq!(trend.records()[0].status_flags, Some(StatusFlags::FAULT));

    let with_status = [
        timestamp(1),
        vec![0x1E, 0x2C, 0x41, 0x20, 0x00, 0x00, 0x1F, 0x2A, 0x04, 0x40],
    ]
    .concat();
    let without_status = [timestamp(1), vec![0x1E, 0x2C, 0x41, 0xA0, 0x00, 0x00, 0x1F]].concat();
    assert_eq!(served(&trend), vec![with_status, without_status]);
}

#[test]
fn trend_log_keeps_log_status_and_record_status_flags_as_distinct_bitstrings() {
    let mut trend = TrendLogObject::new(1, "TL-1", 1).unwrap();
    let mut status = record(1, 0.0, Some(StatusFlags::FAULT));
    status.log_datum = LogDatum::LogStatus(LogStatus::LOG_DISABLED | LogStatus::LOG_INTERRUPTED);
    trend.add_record(status.clone()).unwrap();

    let served = served(&trend);
    assert_eq!(
        served,
        vec![
            [
                timestamp(1),
                // log-status [0] inside log-datum [1], then status-flags [2].
                vec![0x1E, 0x0A, 0x05, 0xA0, 0x1F, 0x2A, 0x04, 0x40],
            ]
            .concat()
        ]
    );
    assert_eq!(
        decode_log_record(&served[0], 0).unwrap(),
        (status, served[0].len())
    );
}

#[test]
fn trend_multiple_serves_each_record_framed() {
    let mut trend = TrendLogMultipleObject::new(1, "TLM-1", 2).unwrap();
    let sample = multiple_record(
        1,
        vec![
            LogValue::RealValue(10.0),
            LogValue::Failure {
                error_class: ErrorClass::PROPERTY,
                error_code: ErrorCode::UNKNOWN_PROPERTY,
            },
        ],
    );
    trend.add_record(sample.clone()).unwrap();

    let served = served(&trend);
    assert_eq!(served.len(), 1);
    assert_eq!(
        decode_log_multiple_record(&served[0], 0).unwrap(),
        (sample.clone(), served[0].len())
    );
    assert_eq!(trend.records()[0], sample);
}

#[test]
fn trend_family_identity_raw_and_served_views_stay_fifo_aligned() {
    let mut trend = TrendLogObject::new(1, "TL-1", 2).unwrap();
    let mut multiple = TrendLogMultipleObject::new(1, "TLM-1", 2).unwrap();
    for hour in 1..=3 {
        trend.add_record(record(hour, hour as f32, None)).unwrap();
        multiple
            .add_record(multiple_record(
                hour,
                vec![LogValue::RealValue(hour as f32)],
            ))
            .unwrap();
    }

    for object in [&trend as &dyn BACnetObject, &multiple as &dyn BACnetObject] {
        let identities = object.log_record_identities_internal().unwrap();
        let served = served(object);
        assert_eq!(identities.len(), 2);
        assert_eq!(identities.len(), served.len());
        assert_eq!(identities[0].sequence_number(), 2);
        assert_eq!(identities[1].sequence_number(), 3);
        for (identity, wire) in identities.iter().zip(served) {
            let (date, time) = if object.object_identifier().object_type() == ObjectType::TREND_LOG
            {
                let record = decode_log_record(&wire, 0).unwrap().0;
                (record.date, record.time)
            } else {
                let record = decode_log_multiple_record(&wire, 0).unwrap().0;
                (record.date, record.time)
            };
            assert_eq!(date, identity.date());
            assert_eq!(time, identity.time());
        }
    }
}

/// A record that would not encode is refused when it is added, through the
/// object's own method or the poller's hook, and the log keeps serving the
/// records it already holds.
#[test]
fn trend_logs_refuse_unencodable_records_at_add() {
    let stray_close = vec![0x21, 0x01, 0x0F];
    let bad_padding = LogValue::BitstringValue {
        unused_bits: 3,
        data: Vec::new(),
    };

    let mut trend = TrendLogObject::new(1, "TL-1", 4).unwrap();
    trend.add_record(record(1, 10.0, None)).unwrap();
    let mut bad = record(2, 0.0, None);
    bad.log_datum = LogDatum::AnyValue(stray_close.clone());
    assert!(trend.add_record(bad.clone()).is_err());
    bad.log_datum = bad_padding.clone().into();
    assert!(trend.add_trend_record(bad).is_err());
    trend.add_record(record(3, 30.0, None)).unwrap();

    let mut multiple = TrendLogMultipleObject::new(1, "TLM-1", 4).unwrap();
    multiple
        .add_record(multiple_record(1, vec![LogValue::RealValue(10.0)]))
        .unwrap();
    assert!(
        multiple
            .add_record(multiple_record(
                2,
                vec![LogValue::RealValue(1.0), LogValue::AnyValue(stray_close)],
            ))
            .is_err()
    );
    assert!(
        multiple
            .add_trend_multiple_record(multiple_record(2, vec![bad_padding]))
            .is_err()
    );
    multiple
        .add_record(multiple_record(3, vec![LogValue::RealValue(30.0)]))
        .unwrap();

    for object in [&trend as &dyn BACnetObject, &multiple] {
        assert_eq!(
            object
                .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
                .unwrap(),
            PropertyValue::Unsigned(2)
        );
        let served = served(object);
        assert_eq!(served.len(), 2);
        // The timestamp's Time octets start at offset 7: the hour.
        assert_eq!(served[0][7], 1);
        assert_eq!(served[1][7], 3);
    }
}
