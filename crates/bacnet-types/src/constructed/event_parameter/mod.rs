//! `BACnetEventParameter` — structured `Event_Parameters` CHOICE
//! (ASHRAE 135-2020 Clause 13.5).
//!
//! See the [`BACnetEventParameter`] type for details.

#[cfg(not(feature = "std"))]
use alloc::{format, string::String, vec, vec::Vec};

use crate::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetExtendedPropertyState, BACnetPropertyStates,
    BACnetProprietaryPropertyState,
};
use crate::error::Error;
use crate::primitives::PropertyValue;

// ---------------------------------------------------------------------------
// Algorithm tag constants
// ---------------------------------------------------------------------------

/// Algorithm tag constants for the [`BACnetEventParameter`] CHOICE.
///
/// These mirror the context-tag numbers in the ASHRAE 135-2020
/// `BACnetEventParameter ::= CHOICE` definition (Clause 13.5) and are used as
/// the leading element of the flat [`PropertyValue::List`] encoding so that an
/// enrollment's parameters survive a complete property round trip without
/// requiring ASN.1 context-tagged framing.
pub mod event_parameter_tag {
    /// `change-of-bitstring [0]`.
    pub const CHANGE_OF_BITSTRING: u8 = 0;
    /// `change-of-state [1]`.
    pub const CHANGE_OF_STATE: u8 = 1;
    /// `change-of-value [2]`.
    pub const CHANGE_OF_VALUE: u8 = 2;
    /// `command-failure [3]` (not modeled as a structured variant).
    pub const COMMAND_FAILURE: u8 = 3;
    /// `floating-limit [4]`.
    pub const FLOATING_LIMIT: u8 = 4;
    /// `out-of-range [5]`.
    pub const OUT_OF_RANGE: u8 = 5;
    /// `extended [9]`.
    pub const EXTENDED: u8 = 9;
}

/// The `cov-criteria [1] CHOICE` nested inside `change-of-value`.
///
/// Per Clause 13.5 the change-of-value algorithm monitors either a bitmask or
/// a referenced-property increment; this preserves which alternative is in
/// use along with its payload.
#[derive(Debug, Clone, PartialEq)]
pub enum ChangeOfValueCriteria {
    /// `bitmask [0] BIT STRING` — report when the masked bits change.
    Bitmask {
        /// Number of unused bits in the last byte of `data`.
        unused_bits: u8,
        /// The bitmask bytes.
        data: Vec<u8>,
    },
    /// `referenced-property-increment [1] REAL` — report on magnitude change.
    ReferencedPropertyIncrement(f32),
}

/// Structured `Event_Parameters` for an EventEnrollment object.
///
/// Models the evaluated algorithm alternatives of the `BACnetEventParameter`
/// `CHOICE` (ASHRAE 135-2020 Clause 13.5) that this library can evaluate.
/// Unknown or vendor-defined alternatives are preserved verbatim via
/// [`BACnetEventParameter::Opaque`] so that values written by a remote client
/// are never silently discarded.
///
/// The wire form is the full ASN.1 CHOICE framing of the Clause 21
/// production, produced by
/// `bacnet_encoding::constructed::encode_event_parameter` /
/// `decode_event_parameter` and carried in
/// [`PropertyValue::ApplicationData`]. The [`PropertyValue::List`] form here
/// (leading algorithm tag as [`PropertyValue::Unsigned`]; see
/// [`BACnetEventParameter::encode`] and [`BACnetEventParameter::decode`]) is
/// the LEGACY pre-framing layout, kept for compatibility with values written
/// before the framing migration (#154).
#[derive(Debug, Clone, PartialEq)]
pub enum BACnetEventParameter {
    /// `change-of-bitstring [0]`: report when masked bits match a value.
    ChangeOfBitstring {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `bitmask [1] BIT STRING` — bits of interest.
        bitmask: (u8, Vec<u8>),
        /// `list-of-bitstring-values [2] SEQUENCE OF BIT STRING` — alarm values.
        list_of_values: Vec<(u8, Vec<u8>)>,
    },
    /// `change-of-state [1]`: report when the monitored value matches a
    /// configured [`BACnetPropertyStates`] value.
    ChangeOfState {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `list-of-values [1] SEQUENCE OF BACnetPropertyStates` — alarm values.
        list_of_values: Vec<BACnetPropertyStates>,
    },
    /// `change-of-value [2]`: report on a bitmask or increment change.
    ChangeOfValue {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `cov-criteria [1] CHOICE`.
        criteria: ChangeOfValueCriteria,
    },
    /// `floating-limit [4]`: report when the value leaves a band around a
    /// setpoint reference.
    FloatingLimit {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `setpoint-reference [1] BACnetDeviceObjectPropertyReference`.
        setpoint_reference: BACnetDeviceObjectPropertyReference,
        /// `low-diff-limit [2] REAL`.
        low_diff_limit: f32,
        /// `high-diff-limit [3] REAL`.
        high_diff_limit: f32,
        /// `deadband [4] REAL`.
        deadband: f32,
    },
    /// `out-of-range [5]`: report when the value leaves a fixed band.
    OutOfRange {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `low-limit [1] REAL`.
        low_limit: f32,
        /// `high-limit [2] REAL`.
        high_limit: f32,
        /// `deadband [3] REAL`.
        deadband: f32,
    },
    /// `extended [9]`: vendor-defined algorithm.
    Extended {
        /// `vendor-id [0] Unsigned16`.
        vendor_id: u16,
        /// `extended-event-type [1] Unsigned`.
        extended_event_type: u32,
        /// Encoded `parameters [2] SEQUENCE OF CHOICE` items, without the
        /// enclosing tag pair.
        parameters: Vec<u8>,
    },
    /// Preserves an unmodeled constructed alternative or `none [20] NULL`.
    ///
    /// For constructed alternatives, `data` is the complete SEQUENCE body.
    /// `Opaque { tag: 20, data: [] }` represents the primitive `none` value.
    /// The framed encoder rejects modeled and reserved tags in this variant.
    Opaque {
        /// The unrecognized algorithm tag.
        tag: u8,
        /// The raw parameter bytes.
        data: Vec<u8>,
    },
}

impl BACnetEventParameter {
    /// Return the algorithm tag for this alternative.
    pub fn tag(&self) -> u8 {
        match self {
            Self::ChangeOfBitstring { .. } => event_parameter_tag::CHANGE_OF_BITSTRING,
            Self::ChangeOfState { .. } => event_parameter_tag::CHANGE_OF_STATE,
            Self::ChangeOfValue { .. } => event_parameter_tag::CHANGE_OF_VALUE,
            Self::FloatingLimit { .. } => event_parameter_tag::FLOATING_LIMIT,
            Self::OutOfRange { .. } => event_parameter_tag::OUT_OF_RANGE,
            Self::Extended { .. } => event_parameter_tag::EXTENDED,
            Self::Opaque { tag, .. } => *tag,
        }
    }

    /// Encode this parameter set as a flat [`PropertyValue::List`].
    ///
    /// The first element is the algorithm tag as [`PropertyValue::Unsigned`],
    /// followed by the variant's fields in declaration order. This is the
    /// LEGACY pre-framing layout (see #154); the wire form is the full ASN.1
    /// framing from `bacnet_encoding::constructed::encode_event_parameter`.
    /// See [`Self::decode`] for the inverse.
    pub fn encode(&self) -> PropertyValue {
        let tag = PropertyValue::Unsigned(self.tag() as u64);
        match self {
            Self::ChangeOfBitstring {
                time_delay,
                bitmask,
                list_of_values,
            } => {
                let mut items = vec![tag, PropertyValue::Unsigned(*time_delay as u64)];
                items.push(bitstring_pv(bitmask.clone()));
                items.push(PropertyValue::List(
                    list_of_values
                        .iter()
                        .map(|bs| bitstring_pv(bs.clone()))
                        .collect(),
                ));
                PropertyValue::List(items)
            }
            Self::ChangeOfState {
                time_delay,
                list_of_values,
            } => PropertyValue::List(vec![
                tag,
                PropertyValue::Unsigned(*time_delay as u64),
                PropertyValue::List(list_of_values.iter().map(property_state_pv).collect()),
            ]),
            Self::ChangeOfValue {
                time_delay,
                criteria,
            } => {
                let crit = match criteria {
                    ChangeOfValueCriteria::Bitmask { unused_bits, data } => {
                        PropertyValue::BitString {
                            unused_bits: *unused_bits,
                            data: data.clone(),
                        }
                    }
                    ChangeOfValueCriteria::ReferencedPropertyIncrement(inc) => {
                        PropertyValue::Real(*inc)
                    }
                };
                PropertyValue::List(vec![tag, PropertyValue::Unsigned(*time_delay as u64), crit])
            }
            Self::FloatingLimit {
                time_delay,
                setpoint_reference,
                low_diff_limit,
                high_diff_limit,
                deadband,
            } => PropertyValue::List(vec![
                tag,
                PropertyValue::Unsigned(*time_delay as u64),
                device_object_property_reference_pv(setpoint_reference),
                PropertyValue::Real(*low_diff_limit),
                PropertyValue::Real(*high_diff_limit),
                PropertyValue::Real(*deadband),
            ]),
            Self::OutOfRange {
                time_delay,
                low_limit,
                high_limit,
                deadband,
            } => PropertyValue::List(vec![
                tag,
                PropertyValue::Unsigned(*time_delay as u64),
                PropertyValue::Real(*low_limit),
                PropertyValue::Real(*high_limit),
                PropertyValue::Real(*deadband),
            ]),
            Self::Extended {
                vendor_id,
                extended_event_type,
                parameters,
            } => PropertyValue::List(vec![
                tag,
                PropertyValue::Unsigned(*vendor_id as u64),
                PropertyValue::Unsigned(*extended_event_type as u64),
                PropertyValue::OctetString(parameters.clone()),
            ]),
            Self::Opaque { tag: _, data } => {
                PropertyValue::List(vec![tag, PropertyValue::OctetString(data.clone())])
            }
        }
    }

    /// Decode a [`PropertyValue`] previously produced by [`Self::encode`].
    ///
    /// Returns `Ok` with the reconstructed parameter set, or a decoding
    /// [`Error`] when the value is not a `List`, is empty, or has an
    /// unrecognized/inconsistent shape. `Opaque` values round-trip unchanged.
    pub fn decode(value: &PropertyValue) -> Result<Self, Error> {
        let PropertyValue::List(items) = value else {
            return Err(Error::decoding(0, "Event_Parameters is not a List"));
        };
        let Some((tag_pv, rest)) = items.split_first() else {
            return Err(Error::decoding(0, "Event_Parameters list is empty"));
        };
        let PropertyValue::Unsigned(tag) = tag_pv else {
            return Err(Error::decoding(0, "Event_Parameters tag is not Unsigned"));
        };
        let tag = u8::try_from(*tag)
            .map_err(|_| Error::decoding(0, "Event_Parameters tag exceeds 255"))?;
        let mut idx = 0;
        let parameters = match tag {
            event_parameter_tag::OUT_OF_RANGE => {
                let time_delay = take_u32(rest, &mut idx)?;
                let low_limit = take_real(rest, &mut idx)?;
                let high_limit = take_real(rest, &mut idx)?;
                let deadband = take_real(rest, &mut idx)?;
                Self::OutOfRange {
                    time_delay,
                    low_limit,
                    high_limit,
                    deadband,
                }
            }
            event_parameter_tag::FLOATING_LIMIT => {
                let time_delay = take_u32(rest, &mut idx)?;
                let setpoint_reference = take_dopr(rest, &mut idx)?;
                let low_diff_limit = take_real(rest, &mut idx)?;
                let high_diff_limit = take_real(rest, &mut idx)?;
                let deadband = take_real(rest, &mut idx)?;
                Self::FloatingLimit {
                    time_delay,
                    setpoint_reference,
                    low_diff_limit,
                    high_diff_limit,
                    deadband,
                }
            }
            event_parameter_tag::CHANGE_OF_STATE => {
                let time_delay = take_u32(rest, &mut idx)?;
                let list_of_values = take_property_state_list(rest, &mut idx)?;
                Self::ChangeOfState {
                    time_delay,
                    list_of_values,
                }
            }
            event_parameter_tag::CHANGE_OF_BITSTRING => {
                let time_delay = take_u32(rest, &mut idx)?;
                let bitmask = take_bitstring(rest, &mut idx)?;
                let list_of_values = take_bitstring_list(rest, &mut idx)?;
                Self::ChangeOfBitstring {
                    time_delay,
                    bitmask,
                    list_of_values,
                }
            }
            event_parameter_tag::CHANGE_OF_VALUE => {
                let time_delay = take_u32(rest, &mut idx)?;
                let criteria = match take_pv(rest, &mut idx) {
                    Some(PropertyValue::BitString { unused_bits, data }) => {
                        ChangeOfValueCriteria::Bitmask {
                            unused_bits: *unused_bits,
                            data: data.clone(),
                        }
                    }
                    Some(PropertyValue::Real(inc)) => {
                        ChangeOfValueCriteria::ReferencedPropertyIncrement(*inc)
                    }
                    _ => return Err(Error::decoding(idx, "invalid cov-criteria")),
                };
                Self::ChangeOfValue {
                    time_delay,
                    criteria,
                }
            }
            event_parameter_tag::EXTENDED => {
                let vendor_id = take_u16(rest, &mut idx)?;
                let extended_event_type = take_u32(rest, &mut idx)?;
                let parameters = match take_pv(rest, &mut idx) {
                    Some(PropertyValue::OctetString(bytes)) => bytes.clone(),
                    _ => return Err(Error::decoding(idx, "extended parameters not octets")),
                };
                Self::Extended {
                    vendor_id,
                    extended_event_type,
                    parameters,
                }
            }
            tag => {
                let data = match take_pv(rest, &mut idx) {
                    Some(PropertyValue::OctetString(bytes)) => bytes.clone(),
                    Some(_) => return Err(Error::decoding(idx, "opaque payload not octets")),
                    None => Vec::new(),
                };
                Self::Opaque { tag, data }
            }
        };
        if idx != rest.len() {
            return Err(Error::decoding(idx, "Event_Parameters has trailing values"));
        }
        Ok(parameters)
    }
}

mod codec;
mod fault;
use codec::*;

#[cfg(test)]
mod tests;
