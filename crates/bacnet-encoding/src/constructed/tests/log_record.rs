use super::*;
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::primitives::{Date, StatusFlags, Time};

fn record(log_datum: LogDatum, status_flags: Option<StatusFlags>) -> BACnetLogRecord {
    BACnetLogRecord {
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
        log_datum,
        status_flags,
    }
}

fn encoded(record: &BACnetLogRecord) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_log_record(record, &mut buf).unwrap();
    buf.to_vec()
}

const TIMESTAMP: [u8; 12] = [
    0x0E, 0xA4, 0x7E, 0x08, 0x1F, 0x01, 0xB4, 0x0E, 0x19, 0x24, 0x2F, 0x0F,
];

/// Each kind of Trend Log record, with the bytes that follow its timestamp.
fn kinds() -> Vec<(LogDatum, Option<StatusFlags>, Vec<u8>)> {
    vec![
        // real-value [2], then status-flags [2] with OVERRIDDEN (bit 2) set.
        (
            LogDatum::RealValue(72.5),
            Some(StatusFlags::OVERRIDDEN),
            vec![0x1E, 0x2C, 0x42, 0x91, 0x00, 0x00, 0x1F, 0x2A, 0x04, 0x20],
        ),
        // failure [8]: PROPERTY / UNKNOWN_PROPERTY.
        (
            LogDatum::Failure {
                error_class: ErrorClass::PROPERTY,
                error_code: ErrorCode::UNKNOWN_PROPERTY,
            },
            None,
            vec![0x1E, 0x8E, 0x91, 0x02, 0x91, 0x20, 0x8F, 0x1F],
        ),
        // log-status [0]: buffer-purged, bit 1.
        (
            LogDatum::LogStatus(LogStatus::BUFFER_PURGED),
            None,
            vec![0x1E, 0x0A, 0x05, 0x40, 0x1F],
        ),
        // time-change [9].
        (
            LogDatum::TimeChange(-1.5),
            None,
            vec![0x1E, 0x9C, 0xBF, 0xC0, 0x00, 0x00, 0x1F],
        ),
        // any-value [10] around an application CharacterString "Hi".
        (
            LogDatum::AnyValue(vec![0x73, 0x00, 0x48, 0x69]),
            None,
            vec![0x1E, 0xAE, 0x73, 0x00, 0x48, 0x69, 0xAF, 0x1F],
        ),
    ]
}

#[test]
fn log_record_kinds_have_exact_bytes_and_round_trip() {
    for (log_datum, status_flags, tail) in kinds() {
        let value = record(log_datum, status_flags);
        let bytes = encoded(&value);
        assert_eq!(bytes, [&TIMESTAMP[..], &tail].concat(), "{value:?}");
        assert_eq!(decode_log_record(&bytes, 0).unwrap(), (value, bytes.len()));
    }
}

/// Bit 0 of a BIT STRING is the top bit of its first octet (Clause 20.2.10):
/// log-disabled and in-alarm are the bit-0 flags of their strings.
#[test]
fn log_record_bit_strings_put_bit0_in_the_top_bit() {
    for (status, octet) in [
        (LogStatus::LOG_DISABLED, 0x80),
        (LogStatus::LOG_DISABLED | LogStatus::BUFFER_PURGED, 0xC0),
        (LogStatus::LOG_INTERRUPTED, 0x20),
    ] {
        let value = record(LogDatum::LogStatus(status), None);
        let bytes = encoded(&value);
        assert_eq!(&bytes[12..], &[0x1E, 0x0A, 0x05, octet, 0x1F], "{status}");
        assert_eq!(decode_log_record(&bytes, 0).unwrap().0, value);
    }
    for (flags, octet) in [
        (StatusFlags::IN_ALARM, 0x80),
        (StatusFlags::FAULT, 0x40),
        (StatusFlags::OUT_OF_SERVICE, 0x10),
    ] {
        let value = record(LogDatum::NullValue, Some(flags));
        let bytes = encoded(&value);
        assert_eq!(&bytes[15..], &[0x2A, 0x04, octet], "{flags}");
        assert_eq!(decode_log_record(&bytes, 0).unwrap().0, value);
    }
}

#[test]
fn log_record_every_plain_datum_has_its_own_tag() {
    for (log_datum, tail) in [
        (LogDatum::BooleanValue(true), vec![0x19, 0x01]),
        (LogDatum::EnumValue(3), vec![0x39, 0x03]),
        (LogDatum::UnsignedValue(7), vec![0x49, 0x07]),
        (LogDatum::SignedValue(-2), vec![0x59, 0xFE]),
        // Integers past 32 bits keep their full width, in the fewest octets.
        (
            LogDatum::SignedValue(-(1 << 39)),
            vec![0x5D, 0x05, 0x80, 0x00, 0x00, 0x00, 0x00],
        ),
        (
            LogDatum::SignedValue(i64::MAX),
            vec![0x5D, 0x08, 0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        ),
        (
            LogDatum::EnumValue(1 << 32),
            vec![0x3D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00],
        ),
        (
            LogDatum::BitstringValue {
                unused_bits: 4,
                data: vec![0b1010_0000],
            },
            vec![0x6A, 0x04, 0xA0],
        ),
        (LogDatum::NullValue, vec![0x78]),
        // A Double and a whole two-element array as any-values: the value's
        // own encoding, context tags and all.
        (
            LogDatum::AnyValue(vec![0x55, 0x08, 0x3F, 0xF0, 0, 0, 0, 0, 0, 0]),
            vec![0xAE, 0x55, 0x08, 0x3F, 0xF0, 0, 0, 0, 0, 0, 0, 0xAF],
        ),
        (
            LogDatum::AnyValue(vec![0x0E, 0x09, 0x01, 0x0F, 0x21, 0x02]),
            vec![0xAE, 0x0E, 0x09, 0x01, 0x0F, 0x21, 0x02, 0xAF],
        ),
        (LogDatum::AnyValue(Vec::new()), vec![0xAE, 0xAF]),
    ] {
        let value = record(log_datum, None);
        let bytes = encoded(&value);
        assert_eq!(bytes[12], 0x1E);
        assert_eq!(&bytes[13..bytes.len() - 1], &tail[..], "{value:?}");
        assert_eq!(bytes[bytes.len() - 1], 0x1F);
        assert_eq!(decode_log_record(&bytes, 0).unwrap(), (value, bytes.len()));
    }
}

/// A peer's log need not hold integers to 32 bits (Clause 21), so a
/// five-octet INTEGER, even one with a redundant sign octet, decodes.
#[test]
fn log_record_from_a_peer_keeps_a_five_octet_integer() {
    for (contents, value) in [
        (&[0xFF, 0x7F, 0xFF, 0xFF, 0xFF][..], -(1i64 << 31) - 1),
        (&[0x01, 0x00, 0x00, 0x00, 0x00][..], 1 << 32),
        (&[0x00, 0x00, 0x00, 0x00, 0x2A][..], 42),
    ] {
        let mut bytes = TIMESTAMP.to_vec();
        bytes.extend([0x1E, 0x5D, 0x05]);
        bytes.extend(contents);
        bytes.push(0x1F);
        assert_eq!(
            decode_log_record(&bytes, 0).unwrap(),
            (record(LogDatum::SignedValue(value), None), bytes.len())
        );
    }
}

#[test]
fn consecutive_log_records_decode_by_returned_offset() {
    // The first record's status flags must not swallow the next record, and
    // a record without them must stop at its datum.
    let first = record(
        LogDatum::UnsignedValue(1),
        Some(StatusFlags::IN_ALARM | StatusFlags::OUT_OF_SERVICE),
    );
    let second = record(LogDatum::NullValue, None);
    let third = record(
        LogDatum::LogStatus(LogStatus::LOG_DISABLED),
        Some(StatusFlags::empty()),
    );
    let mut bytes = encoded(&first);
    bytes.extend(encoded(&second));
    bytes.extend(encoded(&third));
    let (decoded, next) = decode_log_record(&bytes, 0).unwrap();
    assert_eq!(decoded, first);
    let (decoded, next) = decode_log_record(&bytes, next).unwrap();
    assert_eq!(decoded, second);
    assert_eq!(
        decode_log_record(&bytes, next).unwrap(),
        (third, bytes.len())
    );
}

#[test]
fn log_record_rejects_unencodable_values_without_writing() {
    for log_datum in [
        LogDatum::BitstringValue {
            unused_bits: 8,
            data: vec![0],
        },
        LogDatum::BitstringValue {
            unused_bits: 1,
            data: Vec::new(),
        },
        // A value cut short, an opening tag left open, and a stray closing tag.
        LogDatum::AnyValue(vec![0x44, 0x00]),
        LogDatum::AnyValue(vec![0x0E, 0x21, 0x01]),
        LogDatum::AnyValue(vec![0x21, 0x01, 0x0F]),
    ] {
        let mut buf = BytesMut::from(&b"kept"[..]);
        assert!(encode_log_record(&record(log_datum, None), &mut buf).is_err());
        assert_eq!(&buf[..], b"kept");
    }
    // Bits a flags value retains past its defined ones are never sent.
    let value = record(
        LogDatum::LogStatus(LogStatus::from_bits_retain(0b1001)),
        Some(StatusFlags::from_bits_retain(0b1_1000)),
    );
    assert_eq!(
        &encoded(&value)[12..],
        &[0x1E, 0x0A, 0x05, 0x80, 0x1F, 0x2A, 0x04, 0x80]
    );
}

#[test]
fn log_record_decoder_rejects_malformed_records() {
    let good = encoded(&record(LogDatum::NullValue, None));
    let mut cases: Vec<Vec<u8>> = vec![
        good[..good.len() - 1].to_vec(),
        // log-datum [1] holding an unknown alternative, then two alternatives.
        [&TIMESTAMP[..], &[0x1E, 0xB9, 0x01, 0x1F]].concat(),
        [&TIMESTAMP[..], &[0x1E, 0x78, 0x78, 0x1F]].concat(),
        // A null-value with contents, and a two-octet boolean.
        [&TIMESTAMP[..], &[0x1E, 0x79, 0x00, 0x1F]].concat(),
        [&TIMESTAMP[..], &[0x1E, 0x1A, 0x00, 0x01, 0x1F]].concat(),
        // log-status that isn't a three-bit BitString.
        [&TIMESTAMP[..], &[0x1E, 0x0A, 0x04, 0x60, 0x1F]].concat(),
        // status-flags that aren't a four-bit BitString.
        [&TIMESTAMP[..], &[0x1E, 0x78, 0x1F, 0x2A, 0x05, 0x20]].concat(),
        // An INTEGER of nine octets.
        [
            &TIMESTAMP[..],
            &[0x1E, 0x5D, 0x09, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0x1F],
        ]
        .concat(),
        // A failure with a third member.
        [
            &TIMESTAMP[..],
            &[0x1E, 0x8E, 0x91, 0x02, 0x91, 0x20, 0x91, 0x00, 0x8F, 0x1F],
        ]
        .concat(),
    ];
    // A timestamp missing its Time.
    cases.push(vec![
        0x0E, 0xA4, 0x7E, 0x08, 0x1F, 0x01, 0x0F, 0x1E, 0x78, 0x1F,
    ]);
    for bytes in cases {
        assert!(decode_log_record(&bytes, 0).is_err(), "{bytes:02X?}");
    }
}

#[test]
fn members_cut_short_are_a_short_buffer() {
    for (log_datum, status_flags, _) in kinds() {
        let octets = encoded(&record(log_datum, status_flags));
        let framed = assert_members_cut_short("BACnetLogRecord", &octets, |data| {
            decode_log_record(data, 0)
        });
        assert!(framed > 0);
    }
}

/// `bytes` with its one run of `from` replaced by `to`.
fn spliced(bytes: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let at = bytes
        .windows(from.len())
        .position(|window| window == from)
        .unwrap();
    [&bytes[..at], to, &bytes[at + from.len()..]].concat()
}

#[test]
fn a_failure_wider_than_sixteen_bits_is_refused() {
    // Clause 21 holds an error class and code to 16 bits, so a failure
    // carrying a wider one is refused, as the Error-PDU decoder refuses it
    // (#1814); 65535 is the widest kept.
    let bytes = encoded(&record(
        LogDatum::Failure {
            error_class: ErrorClass::PROPERTY,
            error_code: ErrorCode::UNKNOWN_PROPERTY,
        },
        None,
    ));
    for (from, to, what) in [
        (
            &[0x91, 0x02][..],
            &[0x93, 0x01, 0x00, 0x00][..],
            "error class",
        ),
        (
            &[0x91, 0x20][..],
            &[0x93, 0x01, 0x00, 0x00][..],
            "error code",
        ),
    ] {
        let error = decode_log_record(&spliced(&bytes, from, to), 0).unwrap_err();
        let text = error.to_string();
        assert!(
            text.contains(what) && text.contains("exceeds u16"),
            "{text}"
        );
    }
    let widest = spliced(&bytes, &[0x91, 0x02], &[0x92, 0xFF, 0xFF]);
    assert_eq!(
        decode_log_record(&widest, 0).unwrap().0.log_datum,
        LogDatum::Failure {
            error_class: ErrorClass::from_raw(u16::MAX),
            error_code: ErrorCode::UNKNOWN_PROPERTY,
        }
    );
}
