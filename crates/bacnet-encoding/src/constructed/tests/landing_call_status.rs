use super::*;
use bacnet_types::constructed::{BACnetLandingCallStatus, LandingCallCommand};
use bacnet_types::enums::LiftCarDirection;

fn call(
    floor_number: u8,
    command: LandingCallCommand,
    text: Option<&str>,
) -> BACnetLandingCallStatus {
    BACnetLandingCallStatus {
        floor_number,
        command,
        floor_text: text.map(str::to_owned),
    }
}

/// The codec's error for a well-formed value with an oversized member. It
/// is a local range error, never a Protocol error that would read as a
/// remote device's Error-PDU.
fn is_out_of_range(error: &Error) -> bool {
    matches!(error, Error::Range(_))
}

fn encode(value: &BACnetLandingCallStatus) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_landing_call_status(&mut buf, value).unwrap();
    buf.to_vec()
}

/// Independent golden vectors: each member is a primitive context tag, the
/// command alternatives carry their own tags, and there is no frame.
fn golden_vectors() -> Vec<(BACnetLandingCallStatus, Vec<u8>)> {
    vec![
        // floor [0] = 5, direction [1] = UP (3).
        (
            call(5, LandingCallCommand::Direction(LiftCarDirection::UP), None),
            vec![0x09, 0x05, 0x19, 0x03],
        ),
        // floor [0] = 12, destination [2] = 20, floor-text [3] = UTF-8 "L".
        (
            call(12, LandingCallCommand::Destination(20), Some("L")),
            vec![0x09, 0x0C, 0x29, 0x14, 0x3A, 0x00, 0x4C],
        ),
        // floor [0] = 0, proprietary direction 1024 in two octets.
        (
            call(
                0,
                LandingCallCommand::Direction(LiftCarDirection::from_raw(1024)),
                None,
            ),
            vec![0x09, 0x00, 0x1A, 0x04, 0x00],
        ),
        // floor [0] = 255, direction DOWN (4), a floor-text over four octets
        // takes the extended length form.
        (
            call(
                255,
                LandingCallCommand::Direction(LiftCarDirection::DOWN),
                Some("Lobby"),
            ),
            vec![
                0x09, 0xFF, 0x19, 0x04, 0x3D, 0x06, 0x00, b'L', b'o', b'b', b'b', b'y',
            ],
        ),
    ]
}

#[test]
fn landing_call_status_golden_vectors_encode_and_round_trip() {
    for (value, expected) in golden_vectors() {
        let encoded = encode(&value);
        assert_eq!(encoded, expected, "{value:?}");
        let (decoded, end) = decode_landing_call_status(&encoded, 0).unwrap();
        assert_eq!(decoded, value);
        assert_eq!(end, encoded.len());
    }
}

#[test]
fn landing_call_status_decodes_at_an_offset_and_stops_before_the_next_element() {
    let mut data = vec![0xAA];
    data.extend([0x09, 0x05, 0x19, 0x03]);
    data.extend([0x09, 0x06, 0x29, 0x01]);
    let (first, end) = decode_landing_call_status(&data, 1).unwrap();
    assert_eq!(
        first,
        call(5, LandingCallCommand::Direction(LiftCarDirection::UP), None)
    );
    assert_eq!(end, 5);
    let (second, end) = decode_landing_call_status(&data, end).unwrap();
    assert_eq!(second, call(6, LandingCallCommand::Destination(1), None));
    assert_eq!(end, data.len());
}

#[test]
fn landing_call_status_list_round_trips_and_concatenates_elements() {
    let values: Vec<_> = golden_vectors()
        .into_iter()
        .map(|(value, _)| value)
        .collect();
    let mut encoded = BytesMut::new();
    encode_landing_call_status_list(&mut encoded, &values).unwrap();
    let expected: Vec<u8> = golden_vectors()
        .into_iter()
        .flat_map(|(_, bytes)| bytes)
        .collect();
    assert_eq!(encoded.to_vec(), expected);
    assert_eq!(decode_landing_call_status_list(&encoded).unwrap(), values);

    let mut empty = BytesMut::new();
    encode_landing_call_status_list(&mut empty, &[]).unwrap();
    assert!(empty.is_empty());
    assert!(decode_landing_call_status_list(&[]).unwrap().is_empty());
}

#[test]
fn landing_call_status_rejects_malformed_members() {
    // An empty input is an empty list, but never a single value.
    assert!(decode_landing_call_status(&[], 0).is_err());
    let cases: &[(&str, &[u8])] = &[
        ("floor-number only", &[0x09, 0x05]),
        ("command before floor-number", &[0x29, 0x14, 0x09, 0x05]),
        ("application-tagged floor", &[0x21, 0x05, 0x19, 0x03]),
        ("opening tag for floor", &[0x0E, 0x09, 0x05, 0x0F]),
        ("truncated floor", &[0x09]),
        ("empty floor content", &[0x08, 0x19, 0x03]),
        ("empty direction content", &[0x09, 0x01, 0x18]),
        ("unknown command tag", &[0x09, 0x01, 0x49, 0x01]),
        ("truncated direction", &[0x09, 0x01, 0x1A, 0x04]),
        (
            "floor-text without charset",
            &[0x09, 0x01, 0x19, 0x03, 0x38],
        ),
        (
            "truncated floor-text",
            &[0x09, 0x01, 0x19, 0x03, 0x3A, 0x00],
        ),
    ];
    for (what, data) in cases {
        let error = decode_landing_call_status(data, 0).expect_err(what);
        assert!(!is_out_of_range(&error), "{what} is malformed: {error:?}");
        let error = decode_landing_call_status_list(data).expect_err(what);
        assert!(!is_out_of_range(&error), "{what} is malformed: {error:?}");
    }
}

#[test]
fn landing_call_status_reports_oversized_members_as_range_errors() {
    // Well-formed members whose values don't fit their types are a range
    // error, distinct from a malformed encoding.
    // Each case names the member the error must report.
    let cases: &[(&str, &[u8], &str)] = &[
        ("floor 256", &[0x0A, 0x01, 0x00, 0x19, 0x03], "floor-number"),
        ("floor 300", &[0x0A, 0x01, 0x2C, 0x19, 0x03], "floor-number"),
        (
            "floor 300 with floor-text",
            &[0x0A, 0x01, 0x2C, 0x19, 0x03, 0x3A, 0x00, 0x4C],
            "floor-number",
        ),
        (
            "floor 300 and destination 256",
            &[0x0A, 0x01, 0x2C, 0x2A, 0x01, 0x00],
            "floor-number",
        ),
        (
            "destination 256",
            &[0x09, 0x01, 0x2A, 0x01, 0x00],
            "destination",
        ),
        (
            "direction 2^32",
            &[0x09, 0x01, 0x1D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00],
            "direction",
        ),
        (
            "direction wider than 64 bits",
            &[
                0x09, 0x01, 0x1D, 0x09, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ],
            "direction",
        ),
    ];
    for (what, data, member) in cases {
        for error in [
            decode_landing_call_status(data, 0).expect_err(what),
            decode_landing_call_status_list(data).expect_err(what),
        ] {
            assert!(is_out_of_range(&error), "{what}: {error:?}");
            assert!(error.to_string().contains(member), "{what}: {error}");
        }
    }
    // A list fails on its first oversized element.
    let error =
        decode_landing_call_status_list(&[0x09, 0x01, 0x19, 0x03, 0x0A, 0x01, 0x2C, 0x19, 0x03])
            .unwrap_err();
    assert!(is_out_of_range(&error), "{error:?}");

    // A malformed member anywhere in the value wins over an oversized one.
    for data in [
        &[0x0A, 0x01, 0x2C][..],
        &[0x0A, 0x01, 0x2C, 0x49, 0x01][..],
        &[0x0A, 0x01, 0x2C, 0x19, 0x03, 0x3A, 0x00][..],
    ] {
        let error = decode_landing_call_status(data, 0).unwrap_err();
        assert!(!is_out_of_range(&error), "{data:02X?}: {error:?}");
    }

    // The limits themselves decode, including through leading zero octets.
    for (data, expected) in [
        (
            &[0x0A, 0x00, 0xFF, 0x2A, 0x00, 0xFF][..],
            call(255, LandingCallCommand::Destination(255), None),
        ),
        (
            &[0x09, 0x01, 0x1C, 0xFF, 0xFF, 0xFF, 0xFF][..],
            call(
                1,
                LandingCallCommand::Direction(LiftCarDirection::from_raw(u32::MAX)),
                None,
            ),
        ),
        (
            &[
                0x09, 0x01, 0x1D, 0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03,
            ][..],
            call(1, LandingCallCommand::Direction(LiftCarDirection::UP), None),
        ),
    ] {
        let (decoded, end) = decode_landing_call_status(data, 0).unwrap();
        assert_eq!(decoded, expected);
        assert_eq!(end, data.len());
    }
}

#[test]
fn landing_call_status_leaves_trailing_members_to_the_caller() {
    // A second command member, or a tag past floor-text, is not part of the
    // value: the single decoder stops before it and the list decoder rejects
    // it as an element that lacks floor-number [0].
    for (data, end) in [
        (&[0x09, 0x05, 0x19, 0x03, 0x29, 0x14][..], 4),
        (
            &[0x09, 0x05, 0x19, 0x03, 0x3A, 0x00, 0x4C, 0x49, 0x01][..],
            7,
        ),
    ] {
        let (_, consumed) = decode_landing_call_status(data, 0).unwrap();
        assert_eq!(consumed, end);
        assert!(decode_landing_call_status_list(data).is_err());
    }
}

#[test]
fn members_cut_short_are_a_short_buffer() {
    for (_, wire) in golden_vectors() {
        super::assert_members_cut_short("BACnetLandingCallStatus", &wire, |data| {
            decode_landing_call_status(data, 0)
        });
    }
}
