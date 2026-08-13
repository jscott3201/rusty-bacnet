//! Binary Input (type 3), Binary Output (type 4), and Binary Value (type 5)
//! objects per ASHRAE 135-2020 Clauses 12.6, 12.7, and 12.8.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{
    self, read_common_properties, read_generic_event_properties, write_generic_event_properties,
};
use crate::event::{history::EventHistory, ChangeOfStateDetector, CommandFailureDetector};
use crate::rollback::impl_intrinsic_write_rollback;
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
