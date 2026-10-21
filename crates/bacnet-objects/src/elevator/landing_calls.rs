//! Landing_Call_Control and Landing_Calls of the Elevator Group object
//! (Clause 12.58), both built on `BACnetLandingCallStatus` (Clause 21).
//!
//! A Landing_Call_Control write arrives in one of two shapes: the whole
//! propertyValue as one `ApplicationData`, or, from the service decoder,
//! which splits a payload at context-tag boundaries, a `List` holding one
//! `ApplicationData` per member. The members are rejoined and decoded
//! strictly. Error pairings follow Clause 15.9.1.3, which separates a
//! malformed encoding from a well-formed value outside the property's range:
//! a value that isn't this constructed type at all is INVALID_DATA_TYPE,
//! bytes that don't decode as one complete `BACnetLandingCallStatus` are
//! INVALID_DATA_ENCODING, and a well-formed call whose floor-number or
//! destination exceeds an Unsigned8, or whose direction lies outside
//! BACnetLiftCarDirection, is VALUE_OUT_OF_RANGE.

use bacnet_encoding::constructed::{decode_landing_call_status, encode_landing_call_status};
use bacnet_types::constructed::{BACnetLandingCallStatus, LandingCallCommand};
use bacnet_types::enums::LiftCarDirection;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use crate::common;

/// Landing_Call_Control's value before any call is written: a placeholder of
/// floor 0 with direction UNKNOWN, since the datatype has no empty form.
pub(super) fn initial_landing_call_control() -> BACnetLandingCallStatus {
    BACnetLandingCallStatus {
        floor_number: 0,
        command: LandingCallCommand::Direction(LiftCarDirection::UNKNOWN),
        floor_text: None,
    }
}

/// The wire value of one `BACnetLandingCallStatus`.
pub(super) fn encode(value: &BACnetLandingCallStatus) -> Result<PropertyValue, Error> {
    let mut encoded = BytesMut::new();
    encode_landing_call_status(&mut encoded, value)?;
    Ok(PropertyValue::ApplicationData(encoded.to_vec()))
}

/// Decode and validate a Landing_Call_Control write.
pub(super) fn decode_write(value: PropertyValue) -> Result<BACnetLandingCallStatus, Error> {
    let bytes = match value {
        PropertyValue::ApplicationData(bytes) => bytes,
        PropertyValue::List(members) if !members.is_empty() => {
            let mut bytes = Vec::new();
            for member in members {
                let PropertyValue::ApplicationData(member) = member else {
                    return Err(common::invalid_data_type_error());
                };
                bytes.extend_from_slice(&member);
            }
            bytes
        }
        _ => return Err(common::invalid_data_type_error()),
    };
    // The codec reports a well-formed but oversized member as a static
    // range error; every other failure is a malformed encoding.
    let (status, consumed) =
        decode_landing_call_status(&bytes, 0).map_err(|error| match error {
            Error::Range(_) => common::value_out_of_range_error(),
            _ => common::invalid_data_encoding_error(),
        })?;
    if consumed != bytes.len() {
        return Err(common::invalid_data_encoding_error());
    }
    validate(&status)?;
    Ok(status)
}

/// Refuse a direction outside BACnetLiftCarDirection: its named values, or
/// the proprietary range 1024..=65535 (Clause 23.1). 6..=1023 is reserved.
pub(super) fn validate(status: &BACnetLandingCallStatus) -> Result<(), Error> {
    if let LandingCallCommand::Direction(direction) = status.command {
        super::checked(direction, super::direction_in_range)?;
    }
    Ok(())
}
