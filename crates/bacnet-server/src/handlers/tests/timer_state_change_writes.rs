//! State_Change_Values over WriteProperty (#2206): the whole array, one
//! element, a no-value element and an unsupported datatype, as the octets a
//! client sends, through the server's write decode and the Timer.

use super::lighting_required_rows::{assert_refused, db_with};
use super::*;
use PropertyIdentifier as P;
use bacnet_objects::timer::TimerObject;
use bacnet_types::constructed::BACnetTimerStateChangeValue as V;

fn timer() -> (ObjectDatabase, ObjectIdentifier) {
    let mut object = TimerObject::new(3, "TMR-3").unwrap();
    object
        .set_state_change(core::array::from_fn(|_| V::NoValue), vec![], 16)
        .unwrap();
    db_with(Box::new(object))
}

fn write(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    index: Option<u32>,
    octets: &[u8],
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: P::STATE_CHANGE_VALUES,
        property_array_index: index,
        property_value: octets.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    sourced_wp(db, &request).map(|_| ())
}

fn read(db: &ObjectDatabase, oid: ObjectIdentifier, index: Option<u32>) -> Vec<u8> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: P::STATE_CHANGE_VALUES,
        property_array_index: index,
    }
    .encode(&mut request);
    let mut response = BytesMut::new();
    handle_read_property(db, &request, &mut response).unwrap();
    ReadPropertyACK::decode(&response).unwrap().property_value
}

/// One element of each alternative, and an application NULL and REAL.
const WHOLE: &[u8] = &[
    0x08, // [0] no-value
    0x44, 0x42, 0x28, 0x00, 0x00, // REAL 42.0
    0x2E, 0xA4, 126, 11, 1, 7, 0xB4, 8, 30, 0, 0, 0x2F, // [2] datetime
    0x3E, 0x09, 0x0A, 0x3F, // [3] lighting command STOP
    0x1E, 0x21, 0x05, 0x1F, // [1] constructed: Unsigned 5
    0x00, // NULL
    0x44, 0x42, 0x28, 0x00, 0x00, // REAL 42.0
];

#[test]
fn state_change_values_round_trip_whole_and_by_element_over_the_wire() {
    let (mut db, oid) = timer();
    write(&mut db, oid, None, WHOLE).unwrap();
    assert_eq!(read(&db, oid, None), WHOLE);
    assert_eq!(read(&db, oid, Some(3)), &WHOLE[6..18]);
    write(&mut db, oid, Some(2), &[0x08]).unwrap();
    assert_eq!(read(&db, oid, Some(2)), [0x08]);
    write(&mut db, oid, Some(1), &[0x3E, 0x09, 0x0A, 0x3F]).unwrap();
    assert_eq!(read(&db, oid, Some(1)), [0x3E, 0x09, 0x0A, 0x3F]);
    assert_eq!(read(&db, oid, Some(0)), [0x21, 7]);
}

#[test]
fn state_change_values_refusals_over_the_wire() {
    let (mut db, oid) = timer();
    for (index, octets, code) in [
        (
            Some(1),
            &[0x4E, 0x00, 0x4F][..],
            ErrorCode::DATATYPE_NOT_SUPPORTED,
        ),
        (Some(1), &[0x1E, 0x1F][..], ErrorCode::INVALID_DATA_ENCODING),
        (
            None,
            &WHOLE[..WHOLE.len() - 5],
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (Some(8), &[0x08][..], ErrorCode::INVALID_ARRAY_INDEX),
    ] {
        assert_refused(write(&mut db, oid, index, octets), code);
    }
    assert_eq!(read(&db, oid, Some(1)), [0x08]);
}
