//! Binary Input (type 3), Binary Output (type 4), and Binary Value (type 5)
//! objects per ASHRAE 135-2020 Clauses 12.6, 12.7, and 12.8.
//!
//! Each holds the state every I/O object shares (`IoCore`), its event
//! detector (CHANGE_OF_STATE on the Input and the Value, COMMAND_FAILURE on
//! the Output), the state texts the three share and its own Present_Value;
//! the Input and the Output add Polarity, the Input and the Value an
//! Alarm_Value, the Output a Feedback_Value, and the Output and the Value a
//! priority array.
//!
//! The Input and the Output also have a physical side (#2580): the Input
//! takes the state its physical input reads and derives Present_Value from
//! it, and the Output gives the state to drive its physical output to,
//! derived from Present_Value. Polarity relates the two (Tables 12-7 and
//! 12-9), and Out_Of_Service decouples them (Clauses 12.6.10 and 12.7.10).
//! Everything else the objects do, their event algorithms, Feedback_Value
//! and COV included, works on the logical Present_Value.

use alloc::borrow::Cow;
#[cfg(test)]
use bacnet_types::enums::Reliability;
use bacnet_types::enums::{ObjectType, Polarity, PropertyIdentifier};
use bacnet_types::error::Error;
#[cfg(test)]
use bacnet_types::primitives::BACnetTimeStamp;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::common;
use crate::event::{ChangeOfStateDetector, CommandFailureDetector};
use crate::io_core::IoCore;
use crate::prelude::*;
use crate::traits::BACnetObject;

mod metadata;

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

/// Alarm_Value of a Binary Input or Value: the detector's one alarm value.
fn read_alarm_value(detector: &ChangeOfStateDetector) -> PropertyValue {
    // Construction and writes keep exactly one value; ACTIVE is the
    // defensive construction default if a future reset empties it.
    PropertyValue::Enumerated(detector.alarm_values.first().copied().unwrap_or(1))
}

/// The detector a new Binary Input or Value holds: CHANGE_OF_STATE, with
/// ACTIVE its alarm value.
fn alarm_value_detector() -> ChangeOfStateDetector {
    ChangeOfStateDetector {
        alarm_values: vec![1],
        ..Default::default()
    }
}

/// A Polarity a Binary Input or Output can take: NORMAL or REVERSE, and
/// VALUE_OUT_OF_RANGE for any other value.
fn checked_polarity(polarity: Polarity) -> Result<Polarity, Error> {
    if polarity == Polarity::NORMAL || polarity == Polarity::REVERSE {
        Ok(polarity)
    } else {
        Err(common::value_out_of_range_error())
    }
}

/// Carry a state across Polarity, either way (Tables 12-7 and 12-9):
/// `true` is ON on the physical side and ACTIVE on the logical one. NORMAL
/// keeps the state and REVERSE inverts it.
fn across_polarity(state: bool, polarity: Polarity) -> bool {
    state != (polarity == Polarity::REVERSE)
}

/// Whether a binary Present_Value is ACTIVE.
fn is_active(present_value: u32) -> bool {
    present_value == bacnet_types::enums::BinaryPV::ACTIVE.to_raw()
}

/// Active_Text and Inactive_Text, the labels of a binary Present_Value's
/// two states: the rows the three share beyond their [`IoCore`].
struct BinaryProperties {
    active_text: String,
    inactive_text: String,
}

impl BinaryProperties {
    /// `Active` and `Inactive`.
    fn new() -> Self {
        Self {
            active_text: "Active".into(),
            inactive_text: "Inactive".into(),
        }
    }

    /// Read Active_Text or Inactive_Text; `None` for any other property.
    fn read(&self, property: PropertyIdentifier) -> Option<PropertyValue> {
        let text = match property {
            PropertyIdentifier::ACTIVE_TEXT => &self.active_text,
            PropertyIdentifier::INACTIVE_TEXT => &self.inactive_text,
            _ => return None,
        };
        Some(PropertyValue::CharacterString(text.clone()))
    }

    /// Take a write of Active_Text or Inactive_Text, any text; `None` for
    /// any other property.
    fn write(
        &mut self,
        property: PropertyIdentifier,
        value: PropertyValue,
    ) -> Option<Result<(), Error>> {
        let text = match property {
            PropertyIdentifier::ACTIVE_TEXT => &mut self.active_text,
            PropertyIdentifier::INACTIVE_TEXT => &mut self.inactive_text,
            _ => return None,
        };
        let PropertyValue::CharacterString(written) = value else {
            return Some(Err(common::invalid_data_type_error()));
        };
        *text = written;
        Some(Ok(()))
    }
}

/// Give `$object`, a binary object holding [`BinaryProperties`] in
/// `binary`, typed setters for its two texts (#2445).
macro_rules! impl_state_texts {
    ($object:ty) => {
        impl $object {
            /// Set Active_Text, the label of an ACTIVE Present_Value. A new
            /// object says `Active`. Like a write of it, this takes any
            /// text.
            pub fn set_active_text(&mut self, text: impl Into<String>) {
                self.binary.active_text = text.into();
            }

            /// Set Inactive_Text, the label of an INACTIVE Present_Value. A
            /// new object says `Inactive`. Like a write of it, this takes
            /// any text.
            pub fn set_inactive_text(&mut self, text: impl Into<String>) {
                self.binary.inactive_text = text.into();
            }
        }
    };
}
use impl_state_texts;

mod input;
mod output;
mod value;
pub use input::*;
pub use output::*;
pub use value::*;

#[cfg(test)]
#[path = "tests/generic_event_properties.rs"]
mod generic_event_properties_tests;
#[cfg(test)]
#[path = "tests/metadata.rs"]
mod metadata_tests;
#[cfg(test)]
#[path = "tests/polarity.rs"]
mod polarity_tests;
#[cfg(all(test, feature = "std"))]
#[path = "tests/saved_files.rs"]
mod saved_files_tests;
