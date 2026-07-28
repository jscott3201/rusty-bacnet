//! Binary Input (type 3), Binary Output (type 4), and Binary Value (type 5)
//! objects per ASHRAE 135-2020 Clauses 12.4, 12.5, 12.6.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{
    self, read_common_properties, read_generic_event_properties, write_generic_event_properties,
};
use crate::event::{ChangeOfStateDetector, CommandFailureDetector};
use crate::traits::BACnetObject;

mod input;
mod output;
mod value;
pub use input::*;
pub use output::*;
pub use value::*;
