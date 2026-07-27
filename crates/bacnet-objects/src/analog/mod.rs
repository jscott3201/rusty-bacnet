//! Analog Input (type 0), Analog Output (type 1), and Analog Value (type 2) objects.
//!
//! Per ASHRAE 135-2020 Clauses 12.1 (AI), 12.2 (AO), and 12.3 (AV).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::event::OutOfRangeDetector;
use crate::traits::BACnetObject;

mod input;
mod output;
mod value;
pub use input::*;
pub use output::*;
pub use value::*;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
