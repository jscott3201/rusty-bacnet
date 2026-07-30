//! Multi-State Input (type 13), Multi-State Output (type 14), and
//! Multi-State Value (type 19) objects per ASHRAE 135-2020 Clauses 12.18,
//! 12.19, and 12.20.

use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{
    self, read_common_properties, read_generic_event_properties, write_generic_event_properties,
};
use crate::event::ChangeOfStateDetector;
use crate::traits::BACnetObject;

/// Resource cap consistent with bounded server tables such as
/// `MAX_COV_SUBSCRIPTIONS`. Recipient_List has the same pre-existing unbounded
/// growth gap, which is outside issue #228.
pub(crate) const MAX_ALARM_VALUES: usize = 1024;

fn decode_alarm_values_write(
    array_index: Option<u32>,
    value: PropertyValue,
) -> Result<Vec<u32>, Error> {
    if array_index.is_some() {
        return Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::PROPERTY_IS_NOT_AN_ARRAY.to_raw() as u32,
        });
    }
    let PropertyValue::List(values) = value else {
        return Err(common::invalid_data_type_error());
    };
    if values.len() > MAX_ALARM_VALUES {
        return Err(Error::Protocol {
            class: ErrorClass::RESOURCES.to_raw() as u32,
            code: ErrorCode::NO_SPACE_TO_WRITE_PROPERTY.to_raw() as u32,
        });
    }
    values
        .into_iter()
        .map(|value| match value {
            PropertyValue::Unsigned(value) => common::u64_to_u32(value),
            _ => Err(common::invalid_data_type_error()),
        })
        .collect()
}

/// Reject a `number_of_states` of zero.
///
/// Every multi-state object starts `PRESENT_VALUE` (and, for Output/Value, the
/// relinquish default) at 1, and accepts writes only in `1..=number_of_states`.
/// With zero states that initial value is out of range with no attainable value,
/// so the constructors centralize this guard here.
fn require_nonzero_states(number_of_states: u32) -> Result<(), Error> {
    if number_of_states == 0 {
        return Err(Error::OutOfRange(
            "number_of_states must be at least 1".into(),
        ));
    }
    Ok(())
}

mod input;
mod output;
mod value;
pub use input::*;
pub use output::*;
pub use value::*;
#[cfg(test)]
mod tests;
