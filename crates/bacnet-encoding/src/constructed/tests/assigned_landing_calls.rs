use super::*;
use bacnet_types::constructed::{AssignedLandingCall, BACnetAssignedLandingCalls};
use bacnet_types::enums::LiftCarDirection;

fn calls(entries: &[(u8, LiftCarDirection)]) -> BACnetAssignedLandingCalls {
    BACnetAssignedLandingCalls {
        landing_calls: entries
            .iter()
            .map(|&(floor_number, direction)| AssignedLandingCall {
                floor_number,
                direction,
            })
            .collect(),
    }
}

fn encode(value: &BACnetAssignedLandingCalls) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_assigned_landing_calls(&mut buf, value);
    buf.to_vec()
}

/// Independent golden vectors: landing-calls is opening/closing tag 0 around
/// floor-number [0] / direction [1] pairs.
fn golden_vectors() -> Vec<(BACnetAssignedLandingCalls, Vec<u8>)> {
    vec![
        // No assigned calls: the empty frame.
        (calls(&[]), vec![0x0E, 0x0F]),
        // Floor 4, UP (3).
        (
            calls(&[(4, LiftCarDirection::UP)]),
            vec![0x0E, 0x09, 0x04, 0x19, 0x03, 0x0F],
        ),
        // Floor 1 DOWN (4), floor 255 UP_AND_DOWN (5), floor 0 proprietary
        // 1024 in two octets.
        (
            calls(&[
                (1, LiftCarDirection::DOWN),
                (255, LiftCarDirection::UP_AND_DOWN),
                (0, LiftCarDirection::from_raw(1024)),
            ]),
            vec![
                0x0E, 0x09, 0x01, 0x19, 0x04, 0x09, 0xFF, 0x19, 0x05, 0x09, 0x00, 0x1A, 0x04, 0x00,
                0x0F,
            ],
        ),
    ]
}

#[test]
fn assigned_landing_calls_golden_vectors_encode_and_decode() {
    for (value, wire) in golden_vectors() {
        assert_eq!(encode(&value), wire, "{value:?}");
        assert_eq!(
            decode_assigned_landing_calls(&wire, 0).unwrap(),
            (value, wire.len())
        );
    }
}

#[test]
fn assigned_landing_calls_decode_at_an_offset_and_stop_at_the_closing_tag() {
    let mut data = vec![0xAA];
    let value = calls(&[(9, LiftCarDirection::DOWN)]);
    data.extend(encode(&value));
    let end = data.len();
    // A second element follows, as in an Assigned_Landing_Calls array.
    data.extend([0x0E, 0x0F]);
    assert_eq!(
        decode_assigned_landing_calls(&data, 1).unwrap(),
        (value, end)
    );
    assert_eq!(
        decode_assigned_landing_calls(&data, end).unwrap(),
        (calls(&[]), data.len())
    );
}

#[test]
fn assigned_landing_calls_keep_reserved_and_proprietary_directions() {
    for raw in [6u32, 1023, 65_535, 70_000, u32::MAX] {
        let value = calls(&[(2, LiftCarDirection::from_raw(raw))]);
        let wire = encode(&value);
        assert_eq!(
            decode_assigned_landing_calls(&wire, 0).unwrap().0,
            value,
            "{raw}"
        );
    }
}

#[test]
fn assigned_landing_calls_reject_malformed_values() {
    for (wire, context) in [
        (&[][..], "empty input"),
        (&[0x09, 0x04, 0x19, 0x03], "no frame"),
        (&[0x1E, 0x1F], "frame on the wrong tag"),
        (&[0x0E], "no closing tag"),
        (
            &[0x0E, 0x09, 0x04, 0x19, 0x03],
            "truncated before the closing tag",
        ),
        (&[0x0E, 0x19, 0x03, 0x0F], "direction without floor-number"),
        (&[0x0E, 0x09, 0x04, 0x0F], "floor-number without direction"),
        (
            &[0x0E, 0x19, 0x03, 0x09, 0x04, 0x0F],
            "members out of order",
        ),
        (&[0x0E, 0x08, 0x19, 0x03, 0x0F], "empty floor-number"),
        (&[0x0E, 0x09, 0x04, 0x18, 0x0F], "empty direction"),
        (
            &[0x0E, 0x09, 0x04, 0x1A, 0x00, 0x0F],
            "direction overruns the data",
        ),
        (&[0x0E, 0x21, 0x04, 0x0F], "application-tagged member"),
    ] {
        let error = decode_assigned_landing_calls(wire, 0).unwrap_err();
        assert!(
            !matches!(error, Error::Range(_)),
            "{context}: {wire:02X?} is malformed, not out of range: {error:?}"
        );
    }
}

#[test]
fn assigned_landing_calls_report_oversized_members_as_range_errors() {
    // Each case names the member the range error must report.
    let cases: &[(&str, &[u8], &str)] = &[
        (
            "floor 256",
            &[0x0E, 0x0A, 0x01, 0x00, 0x19, 0x03, 0x0F],
            "floor-number",
        ),
        (
            "direction 2^32",
            &[
                0x0E, 0x09, 0x04, 0x1D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00, 0x0F,
            ],
            "direction",
        ),
        (
            "floor 256 on the second call",
            &[
                0x0E, 0x09, 0x01, 0x19, 0x03, 0x0A, 0x01, 0x00, 0x19, 0x04, 0x0F,
            ],
            "floor-number",
        ),
    ];
    for &(context, wire, member) in cases {
        match decode_assigned_landing_calls(wire, 0) {
            Err(Error::Range(refused)) => {
                let message = refused.label;
                assert!(message.contains(member), "{context}: {message}");
                assert!(message.contains("assigned landing calls"), "{context}");
            }
            other => panic!("{context}: expected a range error, got {other:?}"),
        }
    }
    // A malformed member anywhere takes precedence over an oversized one.
    let error =
        decode_assigned_landing_calls(&[0x0E, 0x0A, 0x01, 0x00, 0x19, 0x03, 0x09, 0x01, 0x0F], 0)
            .unwrap_err();
    assert!(!matches!(error, Error::Range(_)), "{error:?}");
}

#[test]
fn assigned_landing_calls_reject_more_than_the_item_limit() {
    let mut wire = vec![0x0E];
    for _ in 0..=MAX_FRAMED_ITEMS {
        wire.extend([0x09, 0x01, 0x19, 0x03]);
    }
    wire.push(0x0F);
    assert!(matches!(
        decode_assigned_landing_calls(&wire, 0),
        Err(Error::Decoding(_))
    ));
}

#[test]
fn members_cut_short_are_a_short_buffer() {
    let mut framed = 0;
    for (_, wire) in golden_vectors() {
        framed += super::assert_members_cut_short("BACnetAssignedLandingCalls", &wire, |data| {
            decode_assigned_landing_calls(data, 0)
        });
    }
    assert!(framed > 0);
}
