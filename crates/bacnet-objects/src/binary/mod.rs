//! Binary Input (type 3), Binary Output (type 4), and Binary Value (type 5)
//! objects per ASHRAE 135-2020 Clauses 12.6, 12.7, and 12.8.

use alloc::borrow::Cow;
use bacnet_types::enums::{
    ErrorClass, ErrorCode, EventState, ObjectType, PropertyIdentifier, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue, StatusFlags};

use crate::common::{
    self, read_common_properties, read_generic_event_properties, write_generic_event_properties,
};
use crate::event::{ChangeOfStateDetector, CommandFailureDetector, history::EventHistory};
use crate::traits::BACnetObject;

mod input;
mod output;
mod value;
pub use input::*;
pub use output::*;
pub use value::*;

#[cfg(test)]
#[path = "tests/generic_event_properties.rs"]
mod generic_event_properties_tests;
