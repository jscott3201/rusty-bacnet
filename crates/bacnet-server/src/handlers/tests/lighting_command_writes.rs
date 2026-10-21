//! Lighting Output's Lighting_Command over WriteProperty and ReadProperty
//! (#1263, Clause 12.54 and Table 12-67): the BACnetLightingCommand SEQUENCE
//! is taken and served as written, other datatypes and broken encodings are
//! refused, and each operation's fields are range-checked.
//!
//! Field octets: the operation is context tag 0 (`09` with one octet), the
//! target level, ramp rate and step increment REALs tags 1 to 3 (`1C`, `2C`,
//! `3C`), the fade time and priority Unsigneds tags 4 and 5 (`49`/`4A`,
//! `59`).

use super::lighting_required_rows::{assert_refused, db_with, read_wire, write_wire};
use super::*;
use bacnet_objects::lighting::LightingOutputObject;

const LC: PropertyIdentifier = PropertyIdentifier::LIGHTING_COMMAND;

/// WriteProperty of Lighting_Command carrying `value` as its octets.
fn write_raw(db: &mut ObjectDatabase, oid: ObjectIdentifier, value: &[u8]) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: LC,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    sourced_wp(db, &request).map(|_| ())
}

fn lighting_output() -> (ObjectDatabase, ObjectIdentifier) {
    db_with(Box::new(LightingOutputObject::new(1, "LO-1").unwrap()))
}

#[test]
fn lighting_command_reads_none_until_written_then_what_was_written() {
    let (mut db, oid) = lighting_output();
    // Operation NONE and no other field.
    assert_eq!(read_wire(&db, oid, LC), [0x09, 0x00]);
    let commands: [&[u8]; 12] = [
        // FADE_TO 50.0 % over 2,000 ms at priority 8.
        &[
            0x09, 0x01, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x4A, 0x07, 0xD0, 0x59, 0x08,
        ],
        // RAMP_TO 25.0 % at 10.0 %/s.
        &[
            0x09, 0x02, 0x1C, 0x41, 0xC8, 0x00, 0x00, 0x2C, 0x41, 0x20, 0x00, 0x00,
        ],
        // STEP_UP by 5.0 %, then STEP_DOWN, STEP_ON and STEP_OFF bare.
        &[0x09, 0x03, 0x3C, 0x40, 0xA0, 0x00, 0x00],
        &[0x09, 0x04],
        &[0x09, 0x05],
        &[0x09, 0x06],
        // WARN at priority 1, WARN_OFF, WARN_RELINQUISH, STOP at 16.
        &[0x09, 0x07, 0x59, 0x01],
        &[0x09, 0x08],
        &[0x09, 0x09],
        &[0x09, 0x0A, 0x59, 0x10],
        // The first and last proprietary operations, 256 and 65,535.
        &[0x0A, 0x01, 0x00],
        &[0x0A, 0xFF, 0xFF],
    ];
    for command in commands {
        write_raw(&mut db, oid, command).unwrap();
        assert_eq!(read_wire(&db, oid, LC), command, "{command:02X?}");
    }
}

#[test]
fn lighting_command_refuses_other_datatypes_and_broken_encodings() {
    let (mut db, oid) = lighting_output();
    // An OCTET STRING, the form this property once took, is not a lighting
    // command, nor is any other application-tagged value.
    for value in [
        PropertyValue::OctetString(vec![0x09, 0x01]),
        PropertyValue::Real(50.0),
        PropertyValue::Enumerated(1),
    ] {
        assert_refused(
            write_wire(&mut db, oid, LC, value, None),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    // Lighting_Command isn't commandable and has no NULL in its datatype, so
    // a NULL succeeds and leaves it as it is (#1396).
    write_wire(&mut db, oid, LC, PropertyValue::Null, None).unwrap();
    assert_eq!(read_wire(&db, oid, LC), [0x09, 0x00]);
    // A target level without the operation before it.
    assert_refused(
        write_raw(&mut db, oid, &[0x1C, 0x42, 0x48, 0x00, 0x00]),
        ErrorCode::INVALID_DATA_TYPE,
    );
    let broken: [&[u8]; 7] = [
        // Fields out of order: ramp rate [2] before target level [1].
        &[
            0x09, 0x02, 0x2C, 0x41, 0x20, 0x00, 0x00, 0x1C, 0x42, 0x48, 0x00, 0x00,
        ],
        // A target level of three octets.
        &[0x09, 0x01, 0x1B, 0x42, 0x48, 0x00],
        // An undefined field [6], and an application-tagged one.
        &[0x09, 0x07, 0x69, 0x01],
        &[0x09, 0x01, 0x44, 0x42, 0x48, 0x00, 0x00],
        // An operation with no content octets, and STOP in five octets that
        // open with zeros.
        &[0x08],
        &[0x0D, 0x05, 0x00, 0x00, 0x00, 0x00, 0x0A],
        // A priority too wide for an Unsigned8 with an application NULL after
        // it: the broken encoding outranks the oversized field. (A lone 0xFF
        // there would break the request's own [3] frame instead.)
        &[0x09, 0x07, 0x5A, 0x01, 0x00, 0x00],
    ];
    for value in broken {
        assert_refused(
            write_raw(&mut db, oid, value),
            ErrorCode::INVALID_DATA_ENCODING,
        );
    }
    assert_eq!(read_wire(&db, oid, LC), [0x09, 0x00]);
}

#[test]
fn lighting_command_checks_the_fields_each_operation_uses() {
    let (mut db, oid) = lighting_output();
    let out_of_range: [&[u8]; 18] = [
        // NONE, reserved operations 11 and 255, and 65,536.
        &[0x09, 0x00],
        &[0x09, 0x0B],
        &[0x09, 0xFF],
        &[0x0B, 0x01, 0x00, 0x00],
        // FADE_TO and RAMP_TO without their target level.
        &[0x09, 0x01, 0x59, 0x08],
        &[0x09, 0x02],
        // FADE_TO to 100.5 %, -1.0 % and NaN.
        &[0x09, 0x01, 0x1C, 0x42, 0xC9, 0x00, 0x00],
        &[0x09, 0x01, 0x1C, 0xBF, 0x80, 0x00, 0x00],
        &[0x09, 0x01, 0x1C, 0x7F, 0xC0, 0x00, 0x00],
        // FADE_TO over 99 ms and over 86,400,001 ms.
        &[0x09, 0x01, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x49, 0x63],
        &[
            0x09, 0x01, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x4C, 0x05, 0x26, 0x5C, 0x01,
        ],
        // RAMP_TO 50.0 % at 0.0 %/s.
        &[
            0x09, 0x02, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x2C, 0x00, 0x00, 0x00, 0x00,
        ],
        // STEP_UP by 100.5 %.
        &[0x09, 0x03, 0x3C, 0x42, 0xC9, 0x00, 0x00],
        // WARN at priorities 0, 17 and 256.
        &[0x09, 0x07, 0x59, 0x00],
        &[0x09, 0x07, 0x59, 0x11],
        &[0x09, 0x07, 0x5A, 0x01, 0x00],
        // Proprietary operations 256 at priority 0 and 65,535 at 17.
        &[0x0A, 0x01, 0x00, 0x59, 0x00],
        &[0x0A, 0xFF, 0xFF, 0x59, 0x11],
    ];
    for value in out_of_range {
        assert_refused(
            write_raw(&mut db, oid, value),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_eq!(read_wire(&db, oid, LC), [0x09, 0x00]);
    let accepted: [&[u8]; 6] = [
        // The range ends: FADE_TO 0.0 % over 100 ms at priority 1, FADE_TO
        // 100.0 % over 86,400,000 ms at priority 16, RAMP_TO at 0.1 %/s and
        // STEP_DOWN by 100.0 %.
        &[
            0x09, 0x01, 0x1C, 0x00, 0x00, 0x00, 0x00, 0x49, 0x64, 0x59, 0x01,
        ],
        &[
            0x09, 0x01, 0x1C, 0x42, 0xC8, 0x00, 0x00, 0x4C, 0x05, 0x26, 0x5C, 0x00, 0x59, 0x10,
        ],
        &[
            0x09, 0x02, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x2C, 0x3D, 0xCC, 0xCC, 0xCD,
        ],
        &[0x09, 0x04, 0x3C, 0x42, 0xC8, 0x00, 0x00],
        // Fields an operation doesn't use are kept but not checked: FADE_TO
        // with a ramp rate of 0.0 and a step increment of 500.0, STEP_UP with
        // a target level of 150.0 and a fade time of 5 ms.
        &[
            0x09, 0x01, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x2C, 0x00, 0x00, 0x00, 0x00, 0x3C, 0x43,
            0xFA, 0x00, 0x00,
        ],
        &[0x09, 0x03, 0x1C, 0x43, 0x16, 0x00, 0x00, 0x49, 0x05],
    ];
    for value in accepted {
        write_raw(&mut db, oid, value).unwrap();
        assert_eq!(read_wire(&db, oid, LC), value, "{value:02X?}");
    }
}
