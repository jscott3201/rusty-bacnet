//! The element of a Timer object's State_Change_Values (Clause 12.57.20).

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use super::BACnetLightingCommand;
use crate::primitives::{Date, PropertyValue, Time};

/// `BACnetTimerStateChangeValue` (Clause 21): what a Timer writes to its
/// referenced properties when one of its state transitions happens.
///
/// On the wire it is an untagged CHOICE. Each primitive alternative is an
/// application-tagged value; `no-value` is a context-tagged NULL `[0]`; the
/// constructed value, a BACnetDateTime and a BACnetLightingCommand each sit
/// between an opening and a closing context tag, `[1]`, `[2]` and `[3]`. The
/// codec is `bacnet_encoding::constructed::{encode_timer_state_change_value,
/// decode_timer_state_change_value}`.
#[derive(Debug, Clone, PartialEq)]
pub enum BACnetTimerStateChangeValue {
    /// `[0]`: the transition writes nothing.
    NoValue,
    /// One application-tagged primitive, a NULL (a relinquish) included. A
    /// value that isn't primitive ([`PropertyValue::is_primitive`]) has no
    /// encoding here.
    Primitive(PropertyValue),
    /// `[1]`: a value of some other datatype, kept as the encoded octets
    /// inside the frame.
    Constructed(Vec<u8>),
    /// `[2]`: a BACnetDateTime.
    DateTime(Date, Time),
    /// `[3]`: a BACnetLightingCommand.
    LightingCommand(BACnetLightingCommand),
}

impl BACnetTimerStateChangeValue {
    /// The datatype the value writes, for comparing two values: an
    /// application tag number for a primitive (a NULL being 0), or 16 plus
    /// the context tag of the other alternatives. `None` for `NoValue`, which
    /// writes nothing.
    pub fn datatype(&self) -> Option<u8> {
        Some(match self {
            Self::NoValue => return None,
            Self::Primitive(value) => match value {
                PropertyValue::Null => 0,
                PropertyValue::Boolean(_) => 1,
                PropertyValue::Unsigned(_) => 2,
                PropertyValue::Signed(_) => 3,
                PropertyValue::Real(_) => 4,
                PropertyValue::Double(_) => 5,
                PropertyValue::OctetString(_) => 6,
                PropertyValue::CharacterString(_) => 7,
                PropertyValue::BitString { .. } => 8,
                PropertyValue::Enumerated(_) => 9,
                PropertyValue::Date(_) => 10,
                PropertyValue::Time(_) => 11,
                PropertyValue::ObjectIdentifier(_) => 12,
                PropertyValue::List(_) | PropertyValue::ApplicationData(_) => 15,
            },
            Self::Constructed(_) => 17,
            Self::DateTime(..) => 18,
            Self::LightingCommand(_) => 19,
        })
    }
}
