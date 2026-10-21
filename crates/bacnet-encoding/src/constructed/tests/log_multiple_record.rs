use super::*;
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{BACnetLogMultipleRecord, LogData, LogValue};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::primitives::{Date, Time};

fn record(log_data: LogData) -> BACnetLogMultipleRecord {
    BACnetLogMultipleRecord {
        date: Date {
            year: 126,
            month: 8,
            day: 31,
            day_of_week: 1,
        },
        time: Time {
            hour: 14,
            minute: 25,
            second: 36,
            hundredths: 47,
        },
        log_data,
    }
}

fn encoded(record: &BACnetLogMultipleRecord) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_log_multiple_record(record, &mut buf).unwrap();
    buf.to_vec()
}

const TIMESTAMP: [u8; 12] = [
    0x0E, 0xA4, 0x7E, 0x08, 0x1F, 0x01, 0xB4, 0x0E, 0x19, 0x24, 0x2F, 0x0F,
];

#[test]
fn log_multiple_record_values_have_exact_bytes_and_round_trip() {
    let value = record(LogData::Values(vec![
        LogValue::RealValue(72.5),
        LogValue::Failure {
            error_class: ErrorClass::PROPERTY,
            error_code: ErrorCode::UNKNOWN_PROPERTY,
        },
        LogValue::NullValue,
        LogValue::UnsignedValue(7),
        LogValue::BooleanValue(true),
    ]));
    let bytes = encoded(&value);
    let mut expected = TIMESTAMP.to_vec();
    expected.extend([
        0x1E, 0x1E, // log-data [1], then its member list [1]
        0x1C, 0x42, 0x91, 0x00, 0x00, // real-value [1]
        0x7E, 0x91, 0x02, 0x91, 0x20, 0x7F, // failure [7]: PROPERTY / UNKNOWN_PROPERTY
        0x68, // null-value [6]
        0x39, 0x07, // unsigned-value [3]
        0x09, 0x01, // boolean-value [0]
        0x1F, 0x1F,
    ]);
    assert_eq!(bytes, expected);
    assert_eq!(
        decode_log_multiple_record(&bytes, 0).unwrap(),
        (value, bytes.len())
    );
}

#[test]
fn log_multiple_record_every_alternative_round_trips() {
    for log_data in [
        LogData::LogStatus(LogStatus::LOG_DISABLED | LogStatus::LOG_INTERRUPTED),
        LogData::TimeChange(-1.5),
        LogData::Values(Vec::new()),
        LogData::Values(vec![
            LogValue::EnumValue(65_535),
            LogValue::SignedValue(-70_000),
            LogValue::SignedValue(i64::MIN),
            LogValue::EnumValue(u64::MAX),
            LogValue::BitstringValue {
                unused_bits: 4,
                data: vec![0b1010_0000],
            },
            LogValue::BitstringValue {
                unused_bits: 0,
                data: Vec::new(),
            },
            LogValue::BooleanValue(false),
            LogValue::AnyValue(vec![0xA4, 0x7E, 0x08, 0x1F, 0x01, 0x10]),
            // A value with context tags of its own, such as one element of
            // a BACnetTimeStamp array.
            LogValue::AnyValue(vec![
                0x2E, 0xA4, 0x7E, 0x08, 0x1F, 0x01, 0xB4, 0, 0, 0, 0, 0x2F,
            ]),
        ]),
    ] {
        let value = record(log_data);
        let bytes = encoded(&value);
        assert_eq!(&bytes[..12], &TIMESTAMP);
        assert_eq!(
            decode_log_multiple_record(&bytes, 0).unwrap(),
            (value.clone(), bytes.len()),
            "{value:?}"
        );
    }
    assert_eq!(
        &encoded(&record(LogData::LogStatus(
            LogStatus::LOG_DISABLED | LogStatus::BUFFER_PURGED
        )))[12..],
        &[0x1E, 0x0A, 0x05, 0xC0, 0x1F]
    );
    assert_eq!(
        &encoded(&record(LogData::TimeChange(0.0)))[12..],
        &[0x1E, 0x2C, 0x00, 0x00, 0x00, 0x00, 0x1F]
    );
}

/// A peer's member values need not stay within 32 bits (Clause 21), so
/// five-octet INTEGER and ENUMERATED entries decode whole.
#[test]
fn log_multiple_record_from_a_peer_keeps_wide_integers() {
    let mut bytes = TIMESTAMP.to_vec();
    bytes.extend([
        0x1E, 0x1E, // log-data [1], then its member list [1]
        0x4D, 0x05, 0xFF, 0x7F, 0xFF, 0xFF, 0xFF, // integer-value [4]
        0x2D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00, // enumerated-value [2]
        0x1F, 0x1F,
    ]);
    let expected = record(LogData::Values(vec![
        LogValue::SignedValue(-(1 << 31) - 1),
        LogValue::EnumValue(1 << 32),
    ]));
    assert_eq!(
        decode_log_multiple_record(&bytes, 0).unwrap(),
        (expected.clone(), bytes.len())
    );
    assert_eq!(encoded(&expected), bytes);
}

#[test]
fn consecutive_log_multiple_records_decode_by_returned_offset() {
    let first = record(LogData::Values(vec![LogValue::UnsignedValue(1)]));
    let second = record(LogData::LogStatus(LogStatus::BUFFER_PURGED));
    let mut bytes = encoded(&first);
    bytes.extend(encoded(&second));
    let (decoded, next) = decode_log_multiple_record(&bytes, 0).unwrap();
    assert_eq!(decoded, first);
    assert_eq!(
        decode_log_multiple_record(&bytes, next).unwrap(),
        (second, bytes.len())
    );
}

#[test]
fn log_multiple_record_rejects_unencodable_values_without_writing() {
    for log_data in [
        LogData::Values(vec![LogValue::BitstringValue {
            unused_bits: 8,
            data: vec![0],
        }]),
        LogData::Values(vec![LogValue::BitstringValue {
            unused_bits: 1,
            data: Vec::new(),
        }]),
        // A value cut short, an opening tag left open, and a stray closing
        // tag.
        LogData::Values(vec![LogValue::AnyValue(vec![0x44, 0x00])]),
        LogData::Values(vec![LogValue::AnyValue(vec![0x0E, 0x21, 0x01])]),
        LogData::Values(vec![LogValue::AnyValue(vec![0x21, 0x01, 0x0F])]),
    ] {
        let mut buf = BytesMut::from(&b"kept"[..]);
        assert!(encode_log_multiple_record(&record(log_data), &mut buf).is_err());
        assert_eq!(&buf[..], b"kept");
    }
}

#[test]
fn log_multiple_record_decoder_rejects_malformed_records() {
    let good = encoded(&record(LogData::Values(vec![LogValue::NullValue])));
    let mut cases: Vec<Vec<u8>> = vec![
        good[..good.len() - 1].to_vec(),
        // log-data [1] holding an unknown alternative.
        [&TIMESTAMP[..], &[0x1E, 0x39, 0x01, 0x1F]].concat(),
        // A member list entry with an unknown tag.
        [&TIMESTAMP[..], &[0x1E, 0x1E, 0x99, 0x00, 0x1F, 0x1F]].concat(),
        // A null-value with contents, and a two-octet boolean.
        [&TIMESTAMP[..], &[0x1E, 0x1E, 0x69, 0x00, 0x1F, 0x1F]].concat(),
        [&TIMESTAMP[..], &[0x1E, 0x1E, 0x0A, 0x00, 0x01, 0x1F, 0x1F]].concat(),
        // log-status that isn't a three-bit BitString.
        [&TIMESTAMP[..], &[0x1E, 0x0A, 0x04, 0x60, 0x1F]].concat(),
    ];
    // A timestamp missing its Time.
    cases.push(vec![
        0x0E, 0xA4, 0x7E, 0x08, 0x1F, 0x01, 0x0F, 0x1E, 0x2C, 0, 0, 0, 0, 0x1F,
    ]);
    for bytes in cases {
        assert!(
            decode_log_multiple_record(&bytes, 0).is_err(),
            "{bytes:02X?}"
        );
    }
}

#[test]
fn members_cut_short_are_a_short_buffer() {
    for log_data in [
        LogData::LogStatus(LogStatus::LOG_DISABLED),
        LogData::TimeChange(-1.5),
        LogData::Values(vec![
            LogValue::RealValue(72.5),
            LogValue::Failure {
                error_class: ErrorClass::PROPERTY,
                error_code: ErrorCode::UNKNOWN_PROPERTY,
            },
            LogValue::UnsignedValue(7),
            LogValue::AnyValue(vec![
                0x2E, 0xA4, 0x7E, 0x08, 0x1F, 0x01, 0xB4, 0, 0, 0, 0, 0x2F,
            ]),
        ]),
    ] {
        let octets = encoded(&record(log_data));
        let framed = assert_members_cut_short("BACnetLogMultipleRecord", &octets, |data| {
            decode_log_multiple_record(data, 0)
        });
        assert!(framed > 0);
    }
}

#[test]
fn a_member_failure_wider_than_sixteen_bits_is_refused() {
    // As a Trend Log record's failure is (#1814): Clause 21 holds both to
    // 16 bits.
    let record = BACnetLogMultipleRecord {
        date: Date {
            year: 126,
            month: 8,
            day: 31,
            day_of_week: 1,
        },
        time: Time {
            hour: 14,
            minute: 25,
            second: 36,
            hundredths: 47,
        },
        log_data: LogData::Values(vec![LogValue::Failure {
            error_class: ErrorClass::PROPERTY,
            error_code: ErrorCode::UNKNOWN_PROPERTY,
        }]),
    };
    let mut buf = BytesMut::new();
    encode_log_multiple_record(&record, &mut buf).unwrap();
    let at = buf.windows(2).position(|w| w == [0x91, 0x20]).unwrap();
    let wide = [&buf[..at], &[0x93, 0x01, 0x00, 0x00][..], &buf[at + 2..]].concat();
    let text = decode_log_multiple_record(&wide, 0)
        .unwrap_err()
        .to_string();
    assert!(
        text.contains("error code") && text.contains("exceeds u16"),
        "{text}"
    );
    assert_eq!(decode_log_multiple_record(&buf, 0).unwrap().0, record);
}
