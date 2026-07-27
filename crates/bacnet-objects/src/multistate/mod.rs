//! Multi-State Input (type 13), Multi-State Output (type 14), and
//! Multi-State Value (type 19) objects per ASHRAE 135-2020 Clauses 12.20-12.22.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::event::ChangeOfStateDetector;
use crate::traits::BACnetObject;

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
