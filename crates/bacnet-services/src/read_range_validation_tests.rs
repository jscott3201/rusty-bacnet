use super::*;
use bacnet_types::enums::ObjectType;

fn request(range: Option<RangeSpec>) -> ReadRangeRequest {
    ReadRangeRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range,
    }
}

#[test]
fn invalid_typed_range_preserves_existing_output() {
    let invalid = request(Some(RangeSpec::ByPosition {
        reference_index: 0,
        count: 0,
    }));
    let mut buf = BytesMut::from(&b"sentinel"[..]);
    assert!(invalid.encode(&mut buf).is_err());
    assert_eq!(&buf[..], b"sentinel");
}

#[test]
fn typed_boundaries_validate_before_output_and_keep_zero_references() {
    let mut invalid = Vec::new();
    for property in [
        PropertyIdentifier::ALL,
        PropertyIdentifier::REQUIRED,
        PropertyIdentifier::OPTIONAL,
    ] {
        let mut r = request(None);
        r.property_identifier = property;
        invalid.push(r);
    }
    let mut r = request(None);
    r.property_array_index = Some(0);
    invalid.push(r);
    for count in [i32::MIN, -32769, 0, 32768, i32::MAX] {
        invalid.push(request(Some(RangeSpec::ByPosition {
            reference_index: 0,
            count,
        })));
        invalid.push(request(Some(RangeSpec::BySequenceNumber {
            reference_seq: 0,
            count,
        })));
    }
    let date = Date {
        year: 126,
        month: 2,
        day: 30,
        day_of_week: 1,
    };
    let time = Time {
        hour: 23,
        minute: 59,
        second: 59,
        hundredths: 99,
    };
    for field in 0..8 {
        let bad_values: &[u8] = match field {
            0 => &[255],
            1 => &[0, 13, 14, 255],
            2 => &[0, 32, 33, 34, 255],
            3 => &[0, 8, 255],
            4 => &[24, 255],
            5 | 6 => &[60, 255],
            _ => &[100, 255],
        };
        for &value in bad_values {
            let (mut d, mut t) = (date, time);
            match field {
                0 => d.year = value,
                1 => d.month = value,
                2 => d.day = value,
                3 => d.day_of_week = value,
                4 => t.hour = value,
                5 => t.minute = value,
                6 => t.second = value,
                _ => t.hundredths = value,
            }
            invalid.push(request(Some(RangeSpec::ByTime {
                reference_time: (d, t),
                count: 1,
            })));
        }
    }
    for r in invalid {
        let mut buf = BytesMut::from(&b"prefix"[..]);
        assert!(r.encode(&mut buf).is_err(), "{r:?}");
        assert_eq!(&buf[..], b"prefix");
    }
    for count in [-32768, -1, 1, 32767] {
        for range in [
            RangeSpec::ByPosition {
                reference_index: 0,
                count,
            },
            RangeSpec::BySequenceNumber {
                reference_seq: 0,
                count,
            },
            RangeSpec::ByTime {
                reference_time: (date, time),
                count,
            },
        ] {
            let r = request(Some(range));
            let mut buf = BytesMut::new();
            r.encode(&mut buf).unwrap();
            assert_eq!(ReadRangeRequest::decode(&buf).unwrap(), r);
        }
    }
    // Independent standard-tag vector: TL1, LOG_BUFFER, ByPosition(reference0,count1).
    let mut buf = BytesMut::new();
    request(Some(RangeSpec::ByPosition {
        reference_index: 0,
        count: 1,
    }))
    .encode(&mut buf)
    .unwrap();
    assert_eq!(
        &buf[..],
        &[0x0c, 0x05, 0, 0, 1, 0x19, 0x83, 0x3e, 0x21, 0, 0x31, 1, 0x3f]
    );
}

#[test]
fn maximum_typed_request_fits_minimum_unsegmented_apdu() {
    let mut r = request(Some(RangeSpec::ByTime {
        reference_time: (
            Date {
                year: 254,
                month: 12,
                day: 31,
                day_of_week: 7,
            },
            Time {
                hour: 23,
                minute: 59,
                second: 59,
                hundredths: 99,
            },
        ),
        count: i32::from(i16::MIN),
    }));
    r.property_identifier = PropertyIdentifier::from_raw(u32::MAX);
    r.property_array_index = Some(u32::MAX);
    let mut buf = BytesMut::new();
    r.encode(&mut buf).unwrap();
    assert_eq!(buf.len() + 4, 34); // four-byte unsegmented confirmed APDU header
    assert!(buf.len() + 4 <= 50);
}
