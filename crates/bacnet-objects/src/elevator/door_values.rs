//! Element values of the Lift's per-door arrays (Clause 12.59, Table 12-77):
//! how each one is checked, decoded from a write and encoded for a read.
//!
//! An element arrives as one Enumerated for Car_Door_Status and
//! Car_Door_Command, one Unsigned for Making_Car_Call, and one
//! `ApplicationData` holding a framed value for Landing_Door_Status,
//! Assigned_Landing_Calls and Registered_Car_Call. Error pairings follow
//! Clause 15.9.1.3: a value of the wrong shape is INVALID_DATA_TYPE, a frame
//! that doesn't decode as exactly one value of the element's type is
//! INVALID_DATA_ENCODING, and a well-formed value outside the element's range
//! is VALUE_OUT_OF_RANGE. That covers a floor number above 255, a door status
//! outside BACnetDoorStatus, a direction outside BACnetLiftCarDirection, and a
//! door command other than NONE, OPEN or CLOSE: BACnetLiftCarDoorCommand has
//! no proprietary range (Clause 23.1, Table 23-1 doesn't list it).

use bacnet_encoding::constructed::{
    decode_assigned_landing_calls, decode_landing_door_status, decode_lift_car_call_list,
    encode_assigned_landing_calls, encode_landing_door_status, encode_lift_car_call_list,
};
use bacnet_types::constructed::{
    BACnetAssignedLandingCalls, BACnetLandingDoorStatus, BACnetLiftCarCallList,
};
use bacnet_types::enums::{DoorStatus, LiftCarDoorCommand};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use super::{checked, direction_in_range, named_or_proprietary};
use crate::common;

/// Whether `status` is in BACnetDoorStatus: a named value, or the
/// proprietary range 1024..=65535 (Clause 23.1).
pub(super) fn door_status_in_range(status: &DoorStatus) -> bool {
    named_or_proprietary(DoorStatus::ALL_NAMED, *status, status.to_raw())
}

/// Whether every landing door of `car_door` has a status in BACnetDoorStatus.
pub(super) fn landing_doors_in_range(car_door: &BACnetLandingDoorStatus) -> bool {
    car_door
        .landing_doors
        .iter()
        .all(|landing| door_status_in_range(&landing.door_status))
}

/// Whether every call of `car_door` asks for a direction in
/// BACnetLiftCarDirection.
pub(super) fn landing_calls_in_range(car_door: &BACnetAssignedLandingCalls) -> bool {
    car_door
        .landing_calls
        .iter()
        .all(|call| direction_in_range(&call.direction))
}

/// Whether `command` is one of the three named door commands.
pub(super) fn door_command_in_range(command: &LiftCarDoorCommand) -> bool {
    LiftCarDoorCommand::ALL_NAMED
        .iter()
        .any(|&(_, named)| named == *command)
}

/// Any value counts as in range: the type itself bounds every member, as an
/// Unsigned8 floor number does.
pub(super) fn always_in_range<T>(_: &T) -> bool {
    true
}

/// Decode one Car_Door_Status element of a write.
pub(super) fn decode_car_door(value: PropertyValue) -> Result<DoorStatus, Error> {
    let PropertyValue::Enumerated(raw) = value else {
        return Err(common::invalid_data_type_error());
    };
    checked(DoorStatus::from_raw(raw), door_status_in_range)
}

/// Decode one Car_Door_Command element of a write.
pub(super) fn decode_door_command(value: PropertyValue) -> Result<LiftCarDoorCommand, Error> {
    let PropertyValue::Enumerated(raw) = value else {
        return Err(common::invalid_data_type_error());
    };
    checked(LiftCarDoorCommand::from_raw(raw), door_command_in_range)
}

/// Decode one Making_Car_Call element of a write: an Unsigned8 floor number,
/// where 0 means no car call.
pub(super) fn decode_car_call(value: PropertyValue) -> Result<u8, Error> {
    let PropertyValue::Unsigned(floor) = value else {
        return Err(common::invalid_data_type_error());
    };
    u8::try_from(floor).map_err(|_| common::value_out_of_range_error())
}

/// Decode one Landing_Door_Status element of a write.
pub(super) fn decode_landing_doors(value: PropertyValue) -> Result<BACnetLandingDoorStatus, Error> {
    checked(
        decode_frame(value, decode_landing_door_status)?,
        landing_doors_in_range,
    )
}

/// Decode one Assigned_Landing_Calls element of a write.
pub(super) fn decode_landing_calls(
    value: PropertyValue,
) -> Result<BACnetAssignedLandingCalls, Error> {
    checked(
        decode_frame(value, decode_assigned_landing_calls)?,
        landing_calls_in_range,
    )
}

/// Decode one Registered_Car_Call element of a write.
pub(super) fn decode_car_calls(value: PropertyValue) -> Result<BACnetLiftCarCallList, Error> {
    decode_frame(value, decode_lift_car_call_list)
}

/// The wire value of one framed element, as `encode` writes it.
pub(super) fn encode_frame<T>(element: &T, encode: fn(&mut BytesMut, &T)) -> PropertyValue {
    let mut encoded = BytesMut::new();
    encode(&mut encoded, element);
    PropertyValue::ApplicationData(encoded.to_vec())
}

/// The wire value of one Landing_Door_Status element.
pub(super) fn encode_landing_doors(element: &BACnetLandingDoorStatus) -> PropertyValue {
    encode_frame(element, encode_landing_door_status)
}

/// The wire value of one Assigned_Landing_Calls element.
pub(super) fn encode_landing_calls(element: &BACnetAssignedLandingCalls) -> PropertyValue {
    encode_frame(element, encode_assigned_landing_calls)
}

/// The wire value of one Registered_Car_Call element.
pub(super) fn encode_car_calls(element: &BACnetLiftCarCallList) -> PropertyValue {
    encode_frame(element, encode_lift_car_call_list)
}

/// A codec's decoder: the value at an offset, and the offset just past it.
type FrameDecoder<T> = fn(&[u8], usize) -> Result<(T, usize), Error>;

/// Decode exactly one framed value from an `ApplicationData` element.
fn decode_frame<T>(value: PropertyValue, decode: FrameDecoder<T>) -> Result<T, Error> {
    let PropertyValue::ApplicationData(bytes) = value else {
        return Err(common::invalid_data_type_error());
    };
    // The codecs report a well-formed but oversized member as a static
    // range error; every other failure is a malformed encoding.
    let (element, consumed) = decode(&bytes, 0).map_err(|error| match error {
        Error::Range(_) => common::value_out_of_range_error(),
        _ => common::invalid_data_encoding_error(),
    })?;
    if consumed != bytes.len() {
        return Err(common::invalid_data_encoding_error());
    }
    Ok(element)
}
