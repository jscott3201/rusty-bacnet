//! The optional rows that shape an intrinsic reporter's transitions (#1329):
//! Event_Message_Texts_Config and the Event_Algorithm_Inhibit pair.
//!
//! Every object table here that has intrinsic reporting lists the three as
//! rows that only an intrinsic reporter may have (Table 12-2, footnote 5,
//! and the like). Event Enrollment's Table 12-14 lists them as plain
//! optional rows, and its algorithm takes them the same way (#1507); Alert
//! Enrollment's Table 12-61 does too (#1865). Each
//! family keeps them in its
//! [`EventHistory`](super::history::EventHistory), which serves and takes
//! them, so every family reads, writes and acts on them the same way:
//!
//! - **Event_Message_Texts_Config** is a BACnetARRAY\[3\] of CharacterString,
//!   one entry per transition in the order TO_OFFNORMAL, TO_FAULT,
//!   TO_NORMAL. A non-empty entry is the Message Text of that transition:
//!   the shared commit kernel stores it in Event_Message_Texts in place of
//!   the server's own text, and the notification carries it from there. An
//!   empty entry, the starting value, leaves the server's text (an Event
//!   Enrollment has none, so it sends no text). The standard lets a device
//!   give these strings substitution codes of its own (Clause 12.12.15 and
//!   its counterparts); this stack defines none and sends the text exactly
//!   as written, so a `{`, `%` or `$` in it goes out as is (#1507).
//! - **Event_Algorithm_Inhibit_Ref** names the local Boolean or BinaryPV
//!   property Event_Algorithm_Inhibit follows. The datatype has no device
//!   member, so it can only name a property of this device. Unset, it reads
//!   as Binary Value 4194303's Present_Value, the reserved-instance form
//!   other bare references take (#1417), and writing that form clears it.
//!   Clearing it also puts Event_Algorithm_Inhibit back to FALSE, so the
//!   object isn't left inhibited by a reference it no longer has.
//! - **Event_Algorithm_Inhibit** is writable while there is no reference and
//!   Event_Detection_Enable is TRUE; otherwise a write is
//!   WRITE_ACCESS_DENIED. With a reference, the database refreshes it from
//!   the referenced property each time the server evaluates the object
//!   ([`ObjectDatabase::follow_event_algorithm_inhibit`]): on each write of
//!   the object and on the one-second tick, and for an Event Enrollment at
//!   the start of each of its evaluation passes as well. A read returns the
//!   value as of that last refresh, so it can trail the referenced property
//!   by up to about a second. Clause 12.2.31 and its counterparts set no
//!   timing, and the read path deliberately doesn't follow the reference
//!   itself (#1507).
//!
//! A TRUE inhibit stops the event algorithm, not fault detection (Clause
//! 13.2.2.1): the detectors take it after fault precedence, propose no
//! offnormal or normal transition of their own and drop any countdown, and
//! an object already offnormal returns to NORMAL at once (Clause
//! 13.2.2.1.5). Once it is FALSE again, a condition has to last its whole
//! time delay from then before it is reported. A log's BUFFER_READY report
//! waits while it is TRUE.
//!
//! [`ObjectDatabase::follow_event_algorithm_inhibit`]: crate::database::ObjectDatabase::follow_event_algorithm_inhibit

use crate::prelude::*;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::EventTransition;
use crate::common;
use crate::property_metadata::{
    PropertyConformance::Optional, PropertyMetadata,
    PropertyPresenceCondition::IntrinsicReportingOptional, PropertyWriteCapability::Always,
};
use crate::reference::{self, ReferenceFrame};

/// The three rows, in the order the object tables list them. Each is only
/// permitted, not required, of an intrinsic reporter, and each takes writes,
/// Event_Algorithm_Inhibit only while it follows no reference.
///
/// Event_Algorithm_Inhibit's own footnote (6 in Table 12-2) makes it
/// mandatory wherever Event_Algorithm_Inhibit_Ref is present, which here is
/// always. It still keeps the class its intrinsic-reporting footnote gives,
/// as Reliability does on objects that serve Reliability_Evaluation_Inhibit,
/// whose footnote 7 ties Reliability's presence to it the same way.
pub(crate) const REPORTING_OPTION_METADATA: [PropertyMetadata; 3] = [
    PropertyMetadata::new(
        P::EVENT_MESSAGE_TEXTS_CONFIG,
        Optional,
        Some(IntrinsicReportingOptional),
        Always,
    ),
    PropertyMetadata::new(
        P::EVENT_ALGORITHM_INHIBIT_REF,
        Optional,
        Some(IntrinsicReportingOptional),
        Always,
    ),
    PropertyMetadata::new(
        P::EVENT_ALGORITHM_INHIBIT,
        Optional,
        Some(IntrinsicReportingOptional),
        Always,
    ),
];

/// The object type an unset Event_Algorithm_Inhibit_Ref names at the
/// reserved instance: one whose Present_Value is a BinaryPV.
const UNSET_REFERENCE_TYPE: ObjectType = ObjectType::BINARY_VALUE;

/// Event_Message_Texts_Config, Event_Algorithm_Inhibit_Ref and
/// Event_Algorithm_Inhibit of one intrinsic reporter; see the module
/// documentation.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ReportingOptions {
    /// Event_Message_Texts_Config, ordered `[TO_OFFNORMAL, TO_FAULT,
    /// TO_NORMAL]`.
    message_texts_config: [String; 3],
    /// Event_Algorithm_Inhibit_Ref, `None` while unset.
    inhibit_reference: Option<BACnetObjectPropertyReference>,
    /// Event_Algorithm_Inhibit: as written while there is no reference, and
    /// the referenced value as last read while there is one.
    inhibit: bool,
}

impl ReportingOptions {
    /// Whether Event_Algorithm_Inhibit is TRUE.
    pub(crate) fn inhibited(&self) -> bool {
        self.inhibit
    }

    /// The property Event_Algorithm_Inhibit follows, if any.
    pub(crate) fn inhibit_reference(&self) -> Option<&BACnetObjectPropertyReference> {
        self.inhibit_reference.as_ref()
    }

    /// Take the value the database read from the referenced property;
    /// returns whether Event_Algorithm_Inhibit changed. Ignored without a
    /// reference, when the value is the client's to write.
    pub(crate) fn follow(&mut self, inhibit: bool) -> bool {
        if self.inhibit_reference.is_none() || self.inhibit == inhibit {
            return false;
        }
        self.inhibit = inhibit;
        true
    }

    /// The configured Message Text of a transition, if its entry isn't empty.
    pub(crate) fn message_text(&self, coordinate: EventTransition) -> Option<&str> {
        let text = &self.message_texts_config[coordinate.index()];
        (!text.is_empty()).then_some(text.as_str())
    }

    /// Serve one of the three rows, or `None` for any other property.
    pub(crate) fn read(
        &self,
        property: P,
        array_index: Option<u32>,
    ) -> Option<Result<PropertyValue, Error>> {
        Some(match property {
            P::EVENT_MESSAGE_TEXTS_CONFIG => {
                let text = |text: &String| PropertyValue::CharacterString(text.clone());
                match array_index {
                    None => Ok(PropertyValue::List(
                        self.message_texts_config.iter().map(text).collect(),
                    )),
                    Some(0) => Ok(PropertyValue::Unsigned(3)),
                    Some(index @ 1..=3) => Ok(text(&self.message_texts_config[index as usize - 1])),
                    Some(_) => Err(common::invalid_array_index_error()),
                }
            }
            P::EVENT_ALGORITHM_INHIBIT_REF | P::EVENT_ALGORITHM_INHIBIT
                if array_index.is_some() =>
            {
                Err(common::property_is_not_an_array_error())
            }
            P::EVENT_ALGORITHM_INHIBIT_REF => Ok(reference::object_property_reference_value(
                self.inhibit_reference.as_ref(),
                UNSET_REFERENCE_TYPE,
            )),
            P::EVENT_ALGORITHM_INHIBIT => Ok(PropertyValue::Boolean(self.inhibit)),
            _ => return None,
        })
    }

    /// Take a write of one of the three rows, or `None` for any other
    /// property; `detection_enabled` is the object's Event_Detection_Enable.
    ///
    /// Event_Message_Texts_Config takes three CharacterStrings whole
    /// (VALUE_OUT_OF_RANGE for another count) or one by index 1 to 3; its
    /// size, index 0, is WRITE_ACCESS_DENIED: the tables fix it at three, and
    /// Clause 12.1.5.1 keeps such an array from being resized, while
    /// State_Text, which a whole write can resize, takes a count there. The
    /// reference takes the encodings
    /// Loop's references take ([`crate::reference`]); clearing it puts the
    /// inhibit back to FALSE.
    pub(crate) fn write(
        &mut self,
        property: P,
        array_index: Option<u32>,
        value: &PropertyValue,
        detection_enabled: bool,
    ) -> Option<Result<(), Error>> {
        Some(match property {
            P::EVENT_MESSAGE_TEXTS_CONFIG => self.write_texts(array_index, value),
            P::EVENT_ALGORITHM_INHIBIT_REF | P::EVENT_ALGORITHM_INHIBIT
                if array_index.is_some() =>
            {
                Err(common::property_is_not_an_array_error())
            }
            P::EVENT_ALGORITHM_INHIBIT_REF => {
                reference::decode_reference_write(value, ReferenceFrame::Bare).map(|reference| {
                    // Cleared, the reference leaves no inhibit behind.
                    if reference.is_none() && self.inhibit_reference.is_some() {
                        self.inhibit = false;
                    }
                    self.inhibit_reference = reference;
                })
            }
            P::EVENT_ALGORITHM_INHIBIT => {
                let PropertyValue::Boolean(inhibit) = *value else {
                    return Some(Err(common::invalid_data_type_error()));
                };
                if self.inhibit_reference.is_some() || !detection_enabled {
                    Err(common::write_access_denied_error())
                } else {
                    self.inhibit = inhibit;
                    Ok(())
                }
            }
            _ => return None,
        })
    }

    fn write_texts(
        &mut self,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> Result<(), Error> {
        let label = |value: &PropertyValue| match value {
            PropertyValue::CharacterString(text) => Ok(text.clone()),
            _ => Err(common::invalid_data_type_error()),
        };
        match array_index {
            None => {
                // One string alone arrives bare, as a one-element array
                // decodes, and is one short of three.
                let values = match value {
                    PropertyValue::List(values) => values.as_slice(),
                    single => core::slice::from_ref(single),
                };
                let texts = values.iter().map(label).collect::<Result<Vec<_>, _>>()?;
                self.message_texts_config = texts
                    .try_into()
                    .map_err(|_| common::value_out_of_range_error())?;
                Ok(())
            }
            // Always three entries (Clause 12.1.5.1): the size isn't written.
            Some(0) => Err(common::write_access_denied_error()),
            Some(index @ 1..=3) => {
                self.message_texts_config[index as usize - 1] = label(value)?;
                Ok(())
            }
            Some(_) => Err(common::invalid_array_index_error()),
        }
    }
}

#[cfg(test)]
#[path = "options_tests.rs"]
mod tests;
