//! Present_Value and Reliability of a Credential Data Input while
//! Out_Of_Service is TRUE (Clauses 12.36.4, 12.36.7 and 12.36.8, Table 12-43
//! footnote 1, #1168).
//!
//! Footnote 1 of Table 12-43 marks these two rows, so a client can stand in
//! for the reader by writing them. The object takes WriteProperty and
//! WritePropertyMultiple of either one only while Out_Of_Service is TRUE; in
//! service both are refused with WRITE_ACCESS_DENIED. A refused write changes
//! nothing.
//!
//! A Present_Value write is checked in three steps:
//!
//! - Its bytes have to decode as exactly one BACnetAuthenticationFactor:
//!   format type, format class and value under context tags 0, 1 and 2, with
//!   nothing after them. Any other form is INVALID_DATA_TYPE, the error Load
//!   Control gives a malformed shed level (#1133).
//! - The format type has to be a named BACnetAuthenticationFactorType. That
//!   production is closed, so any other number is VALUE_OUT_OF_RANGE.
//! - Clause 12.36.4 ties a factor to the formats this reader declares, so the
//!   pair of format type and format class has to match a Supported_Formats
//!   element and the Supported_Format_Classes element at the same position.
//!   Two types need no declaration: UNDEFINED (no factor read) and ERROR (a
//!   read the reader can't decode), each with format class 0. Any other pair
//!   is VALUE_OUT_OF_RANGE. `set_present_value` applies the same rule
//!   (`credential_data_input_formats`). The value octets aren't checked
//!   against the Annex P layout of the format.
//!
//! A Reliability write has to be an Enumerated that passes
//! `common::is_reliability_value_valid`: another number is VALUE_OUT_OF_RANGE
//! and another datatype INVALID_DATA_TYPE.
//!
//! Update_Time moves with each Present_Value update (Clause 12.36.11), and
//! Clause 12.36.8 asks the rest of the device to treat a simulated value as a
//! real read. So an accepted Present_Value write stamps Update_Time from the
//! Device clock, as the Pulse Converter stamps a new Count. With no usable
//! clock it takes the object's next sequence number instead (`UpdateClock`),
//! so each simulated read still moves it. Writing the same factor again
//! stamps it again. A Reliability write leaves Update_Time alone.
//!
//! Out of service the three values stop following the reader, so the object
//! keeps the reader's own values to one side:
//!
//! - On the FALSE-to-TRUE edge it puts aside the Present_Value, Update_Time
//!   and Reliability it serves (`common::write_out_of_service_with_restore`).
//! - Meanwhile a read the application reports (`set_present_value`) replaces
//!   the Present_Value and Update_Time put aside, not those served. A
//!   Reliability it works out (`set_reliability_internal`) is refused, as on
//!   the other Reliability carriers.
//! - On the TRUE-to-FALSE edge all three go back to the values put aside, so
//!   the reader's state is served again at once and the simulation is gone.
//!
//! COV: the server compares the values of the Credential Data Input row of
//! Table 13-1 (#1061), Present_Value and Update_Time among them, around every
//! committed write. A simulated Present_Value reports, a simulated Reliability
//! that leaves or returns to NO_FAULT_DETECTED flips the FAULT flag and
//! reports, and the return to service reports when it changes any of them.

use crate::prelude::*;
use bacnet_encoding::constructed::decode_authentication_factor;
use bacnet_types::constructed::BACnetAuthenticationFactor;
use bacnet_types::enums::{PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue};

use super::credential_data_input_formats::{SupportedFormat, is_declared};
use super::{UpdateClock, simulated_reliability};
use crate::common;

/// The values a client can simulate while Out_Of_Service is TRUE, plus the
/// Update_Time that moves with Present_Value.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Reading {
    pub(super) present_value: BACnetAuthenticationFactor,
    pub(super) update_time: BACnetTimeStamp,
    pub(super) reliability: Reliability,
}

impl Reading {
    /// Apply a client's write of Present_Value or Reliability, taken only
    /// while `out_of_service`; `None` for any other property.
    pub(super) fn write(
        &mut self,
        out_of_service: bool,
        formats: &[SupportedFormat],
        clock: &mut UpdateClock,
        property: PropertyIdentifier,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        if property == PropertyIdentifier::PRESENT_VALUE {
            if !out_of_service {
                return Some(Err(common::write_access_denied_error()));
            }
            return Some(checked_factor(value, formats).map(|factor| {
                self.present_value = factor;
                self.update_time = clock.stamp();
            }));
        }
        if property == PropertyIdentifier::RELIABILITY {
            if !out_of_service {
                return Some(Err(common::write_access_denied_error()));
            }
            return Some(simulated_reliability(value).map(|reliability| {
                self.reliability = reliability;
            }));
        }
        None
    }
}

/// Decode a client's Present_Value write and check it against `formats`.
fn checked_factor(
    value: &PropertyValue,
    formats: &[SupportedFormat],
) -> Result<BACnetAuthenticationFactor, Error> {
    let factor = decode_factor(value).ok_or_else(common::invalid_data_type_error)?;
    if !is_declared(&factor, formats) {
        return Err(common::value_out_of_range_error());
    }
    Ok(factor)
}

/// The factor a write carries, or `None` when its bytes aren't one
/// BACnetAuthenticationFactor.
///
/// The server hands a context-tagged value over as `ApplicationData`, one
/// chunk per tag, so a whole factor arrives as a `List` of three chunks. They
/// are joined back into the bytes the client sent before decoding, as
/// `reference::decode_reference_write` does for a framed reference.
fn decode_factor(value: &PropertyValue) -> Option<BACnetAuthenticationFactor> {
    let bytes = match value {
        PropertyValue::ApplicationData(bytes) => bytes.clone(),
        PropertyValue::List(chunks) => {
            let mut bytes = Vec::new();
            for chunk in chunks {
                let PropertyValue::ApplicationData(part) = chunk else {
                    return None;
                };
                bytes.extend_from_slice(part);
            }
            bytes
        }
        _ => return None,
    };
    match decode_authentication_factor(&bytes, 0) {
        Ok((factor, end)) if end == bytes.len() => Some(factor),
        _ => None,
    }
}
