use super::*;
use bacnet_types::constructed::{BACnetLandingDoorStatus, LandingDoor};
use bacnet_types::enums::DoorStatus;

fn status(doors: &[(u8, DoorStatus)]) -> BACnetLandingDoorStatus {
    BACnetLandingDoorStatus {
        landing_doors: doors
            .iter()
            .map(|&(floor_number, door_status)| LandingDoor {
                floor_number,
                door_status,
            })
            .collect(),
    }
}

fn encode(value: &BACnetLandingDoorStatus) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_landing_door_status(&mut buf, value);
    buf.to_vec()
}

/// Independent golden vectors: landing-doors is opening/closing tag 0 around
/// floor-number [0] / door-status [1] pairs.
fn golden_vectors() -> Vec<(BACnetLandingDoorStatus, Vec<u8>)> {
    vec![
        // No landing doors: the empty frame.
        (status(&[]), vec![0x0E, 0x0F]),
        // Floor 3, CLOSED (0).
        (
            status(&[(3, DoorStatus::CLOSED)]),
            vec![0x0E, 0x09, 0x03, 0x19, 0x00, 0x0F],
        ),
        // Floor 1 SAFETY_LOCKED (8), floor 255 NONE (5), floor 0 proprietary
        // 1024 in two octets.
        (
            status(&[
                (1, DoorStatus::SAFETY_LOCKED),
                (255, DoorStatus::NONE),
                (0, DoorStatus::from_raw(1024)),
            ]),
            vec![
                0x0E, 0x09, 0x01, 0x19, 0x08, 0x09, 0xFF, 0x19, 0x05, 0x09, 0x00, 0x1A, 0x04, 0x00,
                0x0F,
            ],
        ),
    ]
}

#[test]
fn landing_door_status_golden_vectors_encode_and_decode() {
    for (value, wire) in golden_vectors() {
        assert_eq!(encode(&value), wire, "{value:?}");
        assert_eq!(
            decode_landing_door_status(&wire, 0).unwrap(),
            (value, wire.len())
        );
    }
}

#[test]
fn landing_door_status_decodes_at_an_offset_and_stops_at_its_closing_tag() {
    let mut data = vec![0xAA, 0xBB];
    let value = status(&[(7, DoorStatus::OPENED)]);
    data.extend(encode(&value));
    let end = data.len();
    // A second element follows, as in a Landing_Door_Status array.
    data.extend([0x0E, 0x0F]);
    assert_eq!(decode_landing_door_status(&data, 2).unwrap(), (value, end));
    assert_eq!(
        decode_landing_door_status(&data, end).unwrap(),
        (status(&[]), data.len())
    );
}

#[test]
fn landing_door_status_keeps_reserved_and_proprietary_door_statuses() {
    for raw in [10u32, 1023, 65_535, 70_000, u32::MAX] {
        let value = status(&[(2, DoorStatus::from_raw(raw))]);
        let wire = encode(&value);
        assert_eq!(
            decode_landing_door_status(&wire, 0).unwrap().0,
            value,
            "{raw}"
        );
    }
}

#[test]
fn landing_door_status_rejects_malformed_values() {
    for (wire, context) in [
        (&[][..], "empty input"),
        (&[0x09, 0x03, 0x19, 0x00], "no frame"),
        (&[0x1E, 0x1F], "frame on the wrong tag"),
        (&[0x0E], "no closing tag"),
        (
            &[0x0E, 0x09, 0x03, 0x19, 0x00],
            "truncated before the closing tag",
        ),
        (
            &[0x0E, 0x19, 0x00, 0x0F],
            "door-status without floor-number",
        ),
        (
            &[0x0E, 0x09, 0x03, 0x0F],
            "floor-number without door-status",
        ),
        (
            &[0x0E, 0x19, 0x00, 0x09, 0x03, 0x0F],
            "members out of order",
        ),
        (&[0x0E, 0x08, 0x19, 0x00, 0x0F], "empty floor-number"),
        (&[0x0E, 0x09, 0x03, 0x18, 0x0F], "empty door-status"),
        (
            &[0x0E, 0x09, 0x03, 0x1A, 0x00, 0x0F],
            "door-status overruns the data",
        ),
        (&[0x0E, 0x91, 0x00, 0x0F], "application-tagged member"),
    ] {
        let error = decode_landing_door_status(wire, 0).unwrap_err();
        assert!(
            !matches!(error, Error::Range(_)),
            "{context}: {wire:02X?} is malformed, not out of range: {error:?}"
        );
    }
}

#[test]
fn landing_door_status_reports_oversized_members_as_range_errors() {
    // Well-formed members whose values don't fit their types are a range
    // error, distinct from a malformed encoding. Each case names the member
    // the error must report.
    let cases: &[(&str, &[u8], &str)] = &[
        (
            "floor 256",
            &[0x0E, 0x0A, 0x01, 0x00, 0x19, 0x00, 0x0F],
            "floor-number",
        ),
        (
            "door-status 2^32",
            &[
                0x0E, 0x09, 0x03, 0x1D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00, 0x0F,
            ],
            "door-status",
        ),
        (
            "floor 256 on the second landing door",
            &[
                0x0E, 0x09, 0x01, 0x19, 0x00, 0x0A, 0x01, 0x00, 0x19, 0x00, 0x0F,
            ],
            "floor-number",
        ),
        (
            "floor 300 before door-status 2^32",
            &[
                0x0E, 0x0A, 0x01, 0x2C, 0x1D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00, 0x0F,
            ],
            "floor-number",
        ),
    ];
    for &(context, wire, member) in cases {
        match decode_landing_door_status(wire, 0) {
            Err(Error::Range(refused)) => {
                let message = refused.label;
                assert!(message.contains(member), "{context}: {message}")
            }
            other => panic!("{context}: expected a range error, got {other:?}"),
        }
    }
    // A malformed member anywhere takes precedence over an oversized one.
    for (context, wire) in [
        (
            "floor 256, then a truncated frame",
            &[0x0E, 0x0A, 0x01, 0x00, 0x19, 0x00][..],
        ),
        (
            "floor 256, then a door without door-status",
            &[0x0E, 0x0A, 0x01, 0x00, 0x19, 0x00, 0x09, 0x01, 0x0F],
        ),
    ] {
        let error = decode_landing_door_status(wire, 0).unwrap_err();
        assert!(
            !matches!(error, Error::Range(_)),
            "{context}: expected a malformed-value error, got {error:?}"
        );
    }
}

#[test]
fn landing_door_status_rejects_more_than_the_item_limit() {
    let mut wire = vec![0x0E];
    for _ in 0..=MAX_FRAMED_ITEMS {
        wire.extend([0x09, 0x01, 0x19, 0x00]);
    }
    wire.push(0x0F);
    assert!(decode_landing_door_status(&wire, 0).is_err());
}

#[test]
fn members_cut_short_are_a_short_buffer() {
    let mut framed = 0;
    for (_, wire) in golden_vectors() {
        framed += super::assert_members_cut_short("BACnetLandingDoorStatus", &wire, |data| {
            decode_landing_door_status(data, 0)
        });
    }
    assert!(framed > 0);
}
