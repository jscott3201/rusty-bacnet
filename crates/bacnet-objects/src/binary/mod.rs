//! Binary Input (type 3), Binary Output (type 4), and Binary Value (type 5)
//! objects per ASHRAE 135-2020 Clauses 12.6, 12.7, and 12.8.

use alloc::borrow::Cow;
use bacnet_types::enums::{ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
#[cfg(test)]
use bacnet_types::primitives::BACnetTimeStamp;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use crate::common::{
    self, read_common_properties, read_generic_event_properties, write_generic_event_properties,
};
use crate::event::{ChangeOfStateDetector, CommandFailureDetector, history::EventHistory};
use crate::traits::BACnetObject;

/// Take a write of Alarm_Value on a Binary Input or Value: INACTIVE or
/// ACTIVE, which becomes the detector's one alarm value. A staged write is
/// checked by the same code (`write_config::reporting`).
pub(crate) fn write_alarm_value(
    detector: &mut ChangeOfStateDetector,
    value: PropertyValue,
) -> Result<(), Error> {
    let PropertyValue::Enumerated(state) = value else {
        return Err(common::invalid_data_type_error());
    };
    if state > 1 {
        return Err(common::value_out_of_range_error());
    }
    detector.alarm_values = vec![state];
    Ok(())
}

mod input;
mod output;
mod value;
pub use input::*;
pub use output::*;
pub use value::*;

#[cfg(test)]
#[path = "tests/generic_event_properties.rs"]
mod generic_event_properties_tests;
