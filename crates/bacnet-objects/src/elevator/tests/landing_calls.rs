//! Elevator Group Landing_Call_Control and Landing_Calls carry
//! BACnetLandingCallStatus values (Clause 12.58, Table 12-76; #980).

use super::super::*;
use super::{assert_invalid_data_type, assert_value_out_of_range};
use bacnet_encoding::constructed::decode_landing_call_status;
use bacnet_types::constructed::{BACnetLandingCallStatus, LandingCallCommand};
use bacnet_types::enums::{ErrorClass, ErrorCode, LiftCarDirection};

const LCC: PropertyIdentifier = PropertyIdentifier::LANDING_CALL_CONTROL;

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

fn direction(raw: u32) -> LandingCallCommand {
    LandingCallCommand::Direction(LiftCarDirection::from_raw(raw))
}

fn app(bytes: &[u8]) -> PropertyValue {
    PropertyValue::ApplicationData(bytes.to_vec())
}

/// floor `[0]` 7, then a direction `[1]` holding `raw` in the fewest octets.
fn direction_bytes(raw: u32) -> Vec<u8> {
    let octets: Vec<u8> = raw
        .to_be_bytes()
        .into_iter()
        .skip_while(|&octet| octet == 0)
        .collect();
    let octets = if octets.is_empty() { vec![0] } else { octets };
    let mut bytes = vec![0x09, 0x07, 0x18 | octets.len() as u8];
    bytes.extend(octets);
    bytes
}

fn assert_invalid_data_encoding(result: Result<(), Error>, context: &str) {
    match result.expect_err(&format!("{context}: write must be refused")) {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::PROPERTY, "{context}");
            assert_eq!(code, ErrorCode::INVALID_DATA_ENCODING, "{context}");
        }
        other => panic!("{context}: expected PROPERTY/INVALID_DATA_ENCODING, got {other:?}"),
    }
}

fn read_control(group: &ElevatorGroupObject) -> BACnetLandingCallStatus {
    let PropertyValue::ApplicationData(bytes) = group.read_property(LCC, None).unwrap() else {
        panic!("Landing_Call_Control must read as encoded BACnetLandingCallStatus");
    };
    let (status, end) = decode_landing_call_status(&bytes, 0).unwrap();
    assert_eq!(end, bytes.len());
    assert_eq!(&status, group.landing_call_control());
    status
}

#[test]
fn landing_call_control_starts_as_the_unknown_placeholder() {
    let group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    assert_eq!(
        group.read_property(LCC, None).unwrap(),
        app(&[0x09, 0x00, 0x19, 0x00])
    );
    assert_eq!(
        read_control(&group),
        call(
            0,
            LandingCallCommand::Direction(LiftCarDirection::UNKNOWN),
            None
        )
    );
}

#[test]
fn landing_call_control_accepts_whole_and_split_member_shapes() {
    let mut group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    let cases = [
        (
            vec![0x09, 0x05, 0x19, 0x03],
            call(5, LandingCallCommand::Direction(LiftCarDirection::UP), None),
        ),
        (
            vec![0x09, 0x0C, 0x29, 0x14, 0x3A, 0x00, 0x4C],
            call(12, LandingCallCommand::Destination(20), Some("L")),
        ),
    ];
    for (bytes, expected) in cases {
        group.write_property(LCC, None, app(&bytes), None).unwrap();
        assert_eq!(read_control(&group), expected);
        assert_eq!(group.read_property(LCC, None).unwrap(), app(&bytes));
    }
    // The service decoder hands over one ApplicationData per context-tagged
    // member; they are rejoined before decoding.
    let split = PropertyValue::List(vec![
        app(&[0x09, 0x03]),
        app(&[0x19, 0x04]),
        app(&[0x3D, 0x06, 0x00, b'L', b'o', b'b', b'b', b'y']),
    ]);
    group.write_property(LCC, None, split, None).unwrap();
    assert_eq!(
        read_control(&group),
        call(
            3,
            LandingCallCommand::Direction(LiftCarDirection::DOWN),
            Some("Lobby")
        )
    );
}

#[test]
fn landing_call_control_accepts_named_and_proprietary_directions() {
    let mut group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    let named = LiftCarDirection::ALL_NAMED
        .iter()
        .map(|&(_, value)| value.to_raw());
    for raw in named.chain([1024, 40_000, 65_535]) {
        group
            .write_property(LCC, None, app(&direction_bytes(raw)), None)
            .unwrap_or_else(|error| panic!("direction {raw} must be accepted: {error:?}"));
        assert_eq!(read_control(&group), call(7, direction(raw), None));
    }
}

#[test]
fn landing_call_control_rejects_reserved_and_oversized_directions_atomically() {
    let mut group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    group
        .write_property(LCC, None, app(&[0x09, 0x05, 0x19, 0x03]), None)
        .unwrap();
    let before = group.read_property(LCC, None).unwrap();
    for raw in [6, 512, 1023, 65_536, u32::MAX] {
        assert_value_out_of_range(
            group.write_property(LCC, None, app(&direction_bytes(raw)), None),
            &format!("direction {raw}"),
        );
        assert_eq!(group.read_property(LCC, None).unwrap(), before);
    }
}

#[test]
fn landing_call_control_refuses_oversized_unsigned_members_as_out_of_range() {
    // Clause 15.9.1.3 separates a well-formed value outside the property's
    // range from a malformed encoding: every case below is a well-formed
    // BACnetLandingCallStatus whose member doesn't fit its type.
    let mut group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    group
        .write_property(LCC, None, app(&[0x09, 0x05, 0x19, 0x03]), None)
        .unwrap();
    let before = group.read_property(LCC, None).unwrap();
    let cases: &[(&str, &[u8])] = &[
        ("floor 256", &[0x0A, 0x01, 0x00, 0x19, 0x03]),
        ("floor 300", &[0x0A, 0x01, 0x2C, 0x19, 0x03]),
        ("destination 256", &[0x09, 0x01, 0x2A, 0x01, 0x00]),
        (
            "direction 2^32",
            &[0x09, 0x01, 0x1D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00],
        ),
    ];
    for (what, bytes) in cases {
        assert_value_out_of_range(group.write_property(LCC, None, app(bytes), None), what);
        assert_eq!(group.read_property(LCC, None).unwrap(), before, "{what}");
    }
    // The same calls split into one ApplicationData per member, as the
    // service decoder delivers them.
    let split = PropertyValue::List(vec![app(&[0x0A, 0x01, 0x2C]), app(&[0x19, 0x03])]);
    assert_value_out_of_range(
        group.write_property(LCC, None, split, None),
        "split floor 300",
    );
    assert_eq!(group.read_property(LCC, None).unwrap(), before);
}

#[test]
fn landing_call_control_rejects_wrong_datatypes_atomically() {
    let mut group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    let before = group.read_property(LCC, None).unwrap();
    for value in [
        PropertyValue::Enumerated(1),
        PropertyValue::Unsigned(1),
        PropertyValue::Null,
        PropertyValue::CharacterString("up".into()),
        PropertyValue::OctetString(vec![0x09, 0x05, 0x19, 0x03]),
        PropertyValue::List(vec![]),
        PropertyValue::List(vec![app(&[0x09, 0x05]), PropertyValue::Enumerated(3)]),
    ] {
        assert_invalid_data_type(
            group.write_property(LCC, None, value.clone(), None),
            &format!("{value:?}"),
        );
        assert_eq!(group.read_property(LCC, None).unwrap(), before);
    }
}

#[test]
fn landing_call_control_rejects_malformed_encodings_atomically() {
    let mut group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    let before = group.read_property(LCC, None).unwrap();
    let cases: &[(&str, &[u8])] = &[
        ("floor-number only", &[0x09, 0x05]),
        ("command first", &[0x19, 0x03, 0x09, 0x05]),
        ("oversized floor without a command", &[0x0A, 0x01, 0x2C]),
        (
            "both command alternatives",
            &[0x09, 0x05, 0x19, 0x03, 0x29, 0x14],
        ),
        (
            "unknown trailing tag",
            &[0x09, 0x05, 0x19, 0x03, 0x49, 0x01],
        ),
        (
            "truncated floor-text",
            &[0x09, 0x05, 0x19, 0x03, 0x3A, 0x00],
        ),
        ("application-tagged members", &[0x21, 0x05, 0x91, 0x03]),
    ];
    for (what, bytes) in cases {
        assert_invalid_data_encoding(group.write_property(LCC, None, app(bytes), None), what);
        assert_eq!(group.read_property(LCC, None).unwrap(), before, "{what}");
    }
}

#[test]
fn landing_calls_serve_the_application_list_and_refuse_reserved_directions() {
    let mut group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    let calls = vec![
        call(5, LandingCallCommand::Direction(LiftCarDirection::UP), None),
        call(12, LandingCallCommand::Destination(20), Some("L")),
    ];
    group.set_landing_calls(calls.clone()).unwrap();
    assert_eq!(group.landing_calls(), calls.as_slice());
    assert_eq!(
        group
            .read_property(PropertyIdentifier::LANDING_CALLS, None)
            .unwrap(),
        PropertyValue::List(vec![
            app(&[0x09, 0x05, 0x19, 0x03]),
            app(&[0x09, 0x0C, 0x29, 0x14, 0x3A, 0x00, 0x4C]),
        ])
    );

    // A Landing_Call_Control write leaves the application's list alone.
    group
        .write_property(LCC, None, app(&[0x09, 0x02, 0x19, 0x04]), None)
        .unwrap();
    assert_eq!(group.landing_calls(), calls.as_slice());

    let refused = vec![
        call(1, LandingCallCommand::Destination(2), None),
        call(3, direction(6), None),
    ];
    assert_value_out_of_range(group.set_landing_calls(refused), "reserved direction");
    assert_eq!(group.landing_calls(), calls.as_slice());

    group.set_landing_calls(Vec::new()).unwrap();
    assert_eq!(
        group
            .read_property(PropertyIdentifier::LANDING_CALLS, None)
            .unwrap(),
        PropertyValue::List(vec![])
    );
}
