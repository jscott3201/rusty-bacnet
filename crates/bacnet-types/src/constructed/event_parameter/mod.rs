//! `BACnetEventParameter` — structured `Event_Parameters` CHOICE
//! (ASHRAE 135-2020 Clause 13.5).
//!
//! See the [`BACnetEventParameter`] type for details.

#[cfg(not(feature = "std"))]
use alloc::{format, string::String, vec, vec::Vec};

use crate::constructed::{BACnetDeviceObjectPropertyReference, BACnetPropertyStates};
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
/// are never silently discarded. The type is encoded/decoded as a flat
/// [`PropertyValue::List`] whose first element is the algorithm tag
/// ([`PropertyValue::Unsigned`]); see [`BACnetEventParameter::encode`] and
/// [`BACnetEventParameter::decode`].
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
        /// `parameters [2] SEQUENCE OF CHOICE` — raw, vendor-specific.
        parameters: Vec<u8>,
    },
    /// Catch-all preserving an unknown algorithm tag and its raw bytes.
    ///
    /// This keeps values for algorithms the library does not evaluate (e.g.
    /// `command-failure [3]`, `change-of-life-safety [8]`, `buffer-ready [10]`,
    /// the reserved slots, or genuinely unknown tags) intact across a property
    /// round trip.
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
    /// followed by the variant's fields in declaration order. This shape is
    /// the on-the-wire representation used by an EventEnrollment object's
    /// `Event_Parameters` property; see [`Self::decode`] for the inverse.
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
        let mut idx = 0;
        Ok(match *tag as u8 {
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
        })
    }
}

// ---- encode/decode helpers ----

/// Build a [`PropertyValue::BitString`] from an `(unused_bits, data)` pair.
fn bitstring_pv((unused_bits, data): (u8, Vec<u8>)) -> PropertyValue {
    PropertyValue::BitString { unused_bits, data }
}

/// Build a [`PropertyValue`] for a [`BACnetPropertyStates`] tag.
///
/// `BACnetPropertyStates` is itself a CHOICE; absent full context-tagged
/// framing we carry its discriminant as the low byte and the raw payload as an
/// octet string. This round-trips through the flat-`List` encoding.
pub(super) fn property_state_pv(state: &BACnetPropertyStates) -> PropertyValue {
    let (tag, data) = property_state_parts(state);
    PropertyValue::List(vec![
        PropertyValue::Unsigned(tag as u64),
        PropertyValue::OctetString(data),
    ])
}

/// Extract `(tag, raw data)` from a [`BACnetPropertyStates`].
fn property_state_parts(state: &BACnetPropertyStates) -> (u8, Vec<u8>) {
    match state {
        BACnetPropertyStates::BooleanValue(v) => (0u8, vec![u8::from(*v)]),
        BACnetPropertyStates::BinaryValue(v) => (1, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::EventType(v) => (2, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::Polarity(v) => (3, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::ProgramChange(v) => (4, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::ProgramState(v) => (5, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::ReasonForHalt(v) => (6, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::Reliability(v) => (7, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::State(v) => (8, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::SystemStatus(v) => (9, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::Units(v) => (10, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::UnsignedValue(v) => (11, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LifeSafetyMode(v) => (12, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LifeSafetyState(v) => (13, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::DoorAlarmState(v) => (14, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::Action(v) => (15, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::DoorSecuredStatus(v) => (16, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::DoorStatus(v) => (17, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::DoorValue(v) => (18, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LiftCarDirection(v) => (40, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LiftCarDoorCommand(v) => (42, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::TimerState(v) => (38, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::TimerTransition(v) => (39, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::Other { tag, data } => (*tag, data.clone()),
    }
}

/// Build a [`PropertyValue`] for a [`BACnetDeviceObjectPropertyReference`].
pub(super) fn device_object_property_reference_pv(
    r: &BACnetDeviceObjectPropertyReference,
) -> PropertyValue {
    PropertyValue::List(vec![
        PropertyValue::ObjectIdentifier(r.object_identifier),
        PropertyValue::Unsigned(r.property_identifier as u64),
        match r.property_array_index {
            Some(idx) => PropertyValue::Unsigned(idx as u64),
            None => PropertyValue::Null,
        },
        match r.device_identifier {
            Some(dev) => PropertyValue::ObjectIdentifier(dev),
            None => PropertyValue::Null,
        },
    ])
}

/// Read the next [`PropertyValue`] at `idx`, advancing it.
fn take_pv<'a>(items: &'a [PropertyValue], idx: &mut usize) -> Option<&'a PropertyValue> {
    let v = items.get(*idx)?;
    *idx += 1;
    Some(v)
}

/// Read a `Unsigned` as a `u32`.
fn take_u32(items: &[PropertyValue], idx: &mut usize) -> Result<u32, Error> {
    match items.get(*idx) {
        Some(PropertyValue::Unsigned(v)) => {
            *idx += 1;
            u32::try_from(*v).map_err(|_| Error::decoding(*idx, "Unsigned exceeds u32"))
        }
        _ => Err(Error::decoding(*idx, "expected Unsigned")),
    }
}

/// Read a `Unsigned` as a `u16`.
fn take_u16(items: &[PropertyValue], idx: &mut usize) -> Result<u16, Error> {
    match items.get(*idx) {
        Some(PropertyValue::Unsigned(v)) => {
            *idx += 1;
            u16::try_from(*v).map_err(|_| Error::decoding(*idx, "Unsigned exceeds u16"))
        }
        _ => Err(Error::decoding(*idx, "expected Unsigned")),
    }
}

/// Read a `Real`/`Double`/`Unsigned` as `f32`.
fn take_real(items: &[PropertyValue], idx: &mut usize) -> Result<f32, Error> {
    let v = match items.get(*idx) {
        Some(PropertyValue::Real(v)) => *v,
        Some(PropertyValue::Double(v)) => *v as f32,
        Some(PropertyValue::Unsigned(v)) => *v as f32,
        _ => return Err(Error::decoding(*idx, "expected Real")),
    };
    *idx += 1;
    Ok(v)
}

/// Read a bitstring `(unused_bits, data)` pair.
fn take_bitstring(items: &[PropertyValue], idx: &mut usize) -> Result<(u8, Vec<u8>), Error> {
    match items.get(*idx) {
        Some(PropertyValue::BitString { unused_bits, data }) => {
            *idx += 1;
            Ok((*unused_bits, data.clone()))
        }
        _ => Err(Error::decoding(*idx, "expected BitString")),
    }
}

/// Read a `SEQUENCE OF BIT STRING` as a list of `(unused_bits, data)` pairs.
fn take_bitstring_list(
    items: &[PropertyValue],
    idx: &mut usize,
) -> Result<Vec<(u8, Vec<u8>)>, Error> {
    match items.get(*idx) {
        Some(PropertyValue::List(inner)) => {
            *idx += 1;
            inner
                .iter()
                .map(|v| take_bitstring(core::slice::from_ref(v), &mut 0))
                .collect()
        }
        _ => Err(Error::decoding(*idx, "expected list of bitstrings")),
    }
}

/// Read a [`BACnetDeviceObjectPropertyReference`].
pub(super) fn take_dopr(
    items: &[PropertyValue],
    idx: &mut usize,
) -> Result<BACnetDeviceObjectPropertyReference, Error> {
    let Some(PropertyValue::List(inner)) = items.get(*idx) else {
        return Err(Error::decoding(
            *idx,
            "expected device object property reference",
        ));
    };
    *idx += 1;
    if inner.len() < 4 {
        return Err(Error::decoding(
            *idx,
            "device object property reference too short",
        ));
    }
    let object_identifier = match &inner[0] {
        PropertyValue::ObjectIdentifier(oid) => *oid,
        _ => return Err(Error::decoding(0, "reference object id missing")),
    };
    let property_identifier = match &inner[1] {
        PropertyValue::Unsigned(v) => *v as u32,
        _ => return Err(Error::decoding(1, "reference property id missing")),
    };
    let property_array_index = match &inner[2] {
        PropertyValue::Unsigned(v) => Some(*v as u32),
        PropertyValue::Null => None,
        _ => return Err(Error::decoding(2, "reference array index invalid")),
    };
    let device_identifier = match &inner[3] {
        PropertyValue::ObjectIdentifier(dev) => Some(*dev),
        PropertyValue::Null => None,
        _ => return Err(Error::decoding(3, "reference device id invalid")),
    };
    Ok(BACnetDeviceObjectPropertyReference {
        object_identifier,
        property_identifier,
        property_array_index,
        device_identifier,
    })
}

/// Read a `SEQUENCE OF BACnetPropertyStates`.
pub(super) fn take_property_state_list(
    items: &[PropertyValue],
    idx: &mut usize,
) -> Result<Vec<BACnetPropertyStates>, Error> {
    let Some(PropertyValue::List(inner)) = items.get(*idx) else {
        return Err(Error::decoding(*idx, "expected list of property states"));
    };
    *idx += 1;
    inner.iter().map(property_state_from_pv).collect()
}

/// Reconstruct a [`BACnetPropertyStates`] from its flat-`List` encoding.
pub(super) fn property_state_from_pv(pv: &PropertyValue) -> Result<BACnetPropertyStates, Error> {
    let PropertyValue::List(inner) = pv else {
        return Err(Error::decoding(0, "property state is not a List"));
    };
    let Some((tag_pv, rest)) = inner.split_first() else {
        return Err(Error::decoding(0, "property state list empty"));
    };
    let PropertyValue::Unsigned(tag) = tag_pv else {
        return Err(Error::decoding(0, "property state tag not Unsigned"));
    };
    let data = match rest.first() {
        Some(PropertyValue::OctetString(b)) => b.clone(),
        _ => return Err(Error::decoding(1, "property state data not octets")),
    };
    let read_u32 = |data: &[u8]| -> Result<u32, Error> {
        data.try_into()
            .map(u32::from_le_bytes)
            .map_err(|_| Error::decoding(1, "property state data wrong length"))
    };
    Ok(match *tag as u8 {
        0 => BACnetPropertyStates::BooleanValue(data.first().copied().unwrap_or(0) != 0),
        1 => BACnetPropertyStates::BinaryValue(read_u32(&data)?),
        2 => BACnetPropertyStates::EventType(read_u32(&data)?),
        3 => BACnetPropertyStates::Polarity(read_u32(&data)?),
        4 => BACnetPropertyStates::ProgramChange(read_u32(&data)?),
        5 => BACnetPropertyStates::ProgramState(read_u32(&data)?),
        6 => BACnetPropertyStates::ReasonForHalt(read_u32(&data)?),
        7 => BACnetPropertyStates::Reliability(read_u32(&data)?),
        8 => BACnetPropertyStates::State(read_u32(&data)?),
        9 => BACnetPropertyStates::SystemStatus(read_u32(&data)?),
        10 => BACnetPropertyStates::Units(read_u32(&data)?),
        11 => BACnetPropertyStates::UnsignedValue(read_u32(&data)?),
        12 => BACnetPropertyStates::LifeSafetyMode(read_u32(&data)?),
        13 => BACnetPropertyStates::LifeSafetyState(read_u32(&data)?),
        14 => BACnetPropertyStates::DoorAlarmState(read_u32(&data)?),
        15 => BACnetPropertyStates::Action(read_u32(&data)?),
        16 => BACnetPropertyStates::DoorSecuredStatus(read_u32(&data)?),
        17 => BACnetPropertyStates::DoorStatus(read_u32(&data)?),
        18 => BACnetPropertyStates::DoorValue(read_u32(&data)?),
        40 => BACnetPropertyStates::LiftCarDirection(read_u32(&data)?),
        42 => BACnetPropertyStates::LiftCarDoorCommand(read_u32(&data)?),
        38 => BACnetPropertyStates::TimerState(read_u32(&data)?),
        39 => BACnetPropertyStates::TimerTransition(read_u32(&data)?),
        other => BACnetPropertyStates::Other { tag: other, data },
    })
}

// ---------------------------------------------------------------------------
// FaultParameters structured round trip (Clause 12.12.50 -- Fault_Parameters)
// ---------------------------------------------------------------------------

/// Variant tag carried as the leading element of the flat-`List` encoding.
mod fault_parameter_tag {
    pub const NONE: u8 = 0;
    pub const CHARACTER_STRING: u8 = 1;
    pub const EXTENDED: u8 = 2;
    pub const LIFE_SAFETY: u8 = 3;
    pub const STATE: u8 = 4;
    pub const STATUS_FLAGS: u8 = 5;
    pub const OUT_OF_RANGE: u8 = 6;
    pub const LISTED: u8 = 7;
}

impl crate::constructed::FaultParameters {
    /// Encode this fault-parameter set as a flat [`PropertyValue::List`].
    ///
    /// The first element is the variant tag as [`PropertyValue::Unsigned`],
    /// followed by the variant's fields in declaration order. This is the
    /// on-the-wire representation used by an EventEnrollment object's
    /// `Fault_Parameters` property; see [`Self::decode_property_value`].
    pub fn encode_property_value(&self) -> PropertyValue {
        use crate::constructed::FaultParameters as F;
        match self {
            F::FaultNone => PropertyValue::List(vec![PropertyValue::Unsigned(
                fault_parameter_tag::NONE as u64,
            )]),
            F::FaultCharacterString { fault_values } => PropertyValue::List(vec![
                PropertyValue::Unsigned(fault_parameter_tag::CHARACTER_STRING as u64),
                PropertyValue::List(
                    fault_values
                        .iter()
                        .map(|s| PropertyValue::CharacterString(s.clone()))
                        .collect(),
                ),
            ]),
            F::FaultExtended {
                vendor_id,
                extended_fault_type,
                parameters,
            } => PropertyValue::List(vec![
                PropertyValue::Unsigned(fault_parameter_tag::EXTENDED as u64),
                PropertyValue::Unsigned(*vendor_id as u64),
                PropertyValue::Unsigned(*extended_fault_type as u64),
                PropertyValue::OctetString(parameters.clone()),
            ]),
            F::FaultLifeSafety {
                fault_values,
                mode_for_reference,
            } => PropertyValue::List(vec![
                PropertyValue::Unsigned(fault_parameter_tag::LIFE_SAFETY as u64),
                PropertyValue::List(
                    fault_values
                        .iter()
                        .map(|v| PropertyValue::Unsigned(*v as u64))
                        .collect(),
                ),
                device_object_property_reference_pv(mode_for_reference),
            ]),
            F::FaultState { fault_values } => PropertyValue::List(vec![
                PropertyValue::Unsigned(fault_parameter_tag::STATE as u64),
                PropertyValue::List(fault_values.iter().map(property_state_pv).collect()),
            ]),
            F::FaultStatusFlags { reference } => PropertyValue::List(vec![
                PropertyValue::Unsigned(fault_parameter_tag::STATUS_FLAGS as u64),
                device_object_property_reference_pv(reference),
            ]),
            F::FaultOutOfRange {
                min_normal,
                max_normal,
            } => PropertyValue::List(vec![
                PropertyValue::Unsigned(fault_parameter_tag::OUT_OF_RANGE as u64),
                PropertyValue::Double(*min_normal),
                PropertyValue::Double(*max_normal),
            ]),
            F::FaultListed { reference } => PropertyValue::List(vec![
                PropertyValue::Unsigned(fault_parameter_tag::LISTED as u64),
                device_object_property_reference_pv(reference),
            ]),
        }
    }

    /// Decode a [`PropertyValue`] previously produced by
    /// [`Self::encode_property_value`].
    pub fn decode_property_value(value: &PropertyValue) -> Result<Self, Error> {
        use crate::constructed::FaultParameters as F;
        let PropertyValue::List(items) = value else {
            return Err(Error::decoding(0, "Fault_Parameters is not a List"));
        };
        let Some((tag_pv, rest)) = items.split_first() else {
            return Err(Error::decoding(0, "Fault_Parameters list is empty"));
        };
        let PropertyValue::Unsigned(tag) = tag_pv else {
            return Err(Error::decoding(0, "Fault_Parameters tag is not Unsigned"));
        };
        let mut idx = 0;
        Ok(match *tag as u8 {
            fault_parameter_tag::NONE => F::FaultNone,
            fault_parameter_tag::CHARACTER_STRING => {
                let Some(PropertyValue::List(inner)) = rest.get(idx) else {
                    return Err(Error::decoding(idx, "fault characterstring values missing"));
                };
                idx += 1;
                let fault_values = inner
                    .iter()
                    .map(|v| match v {
                        PropertyValue::CharacterString(s) => Ok(s.clone()),
                        _ => Err(Error::decoding(idx, "fault value not a string")),
                    })
                    .collect::<Result<Vec<String>, Error>>()?;
                F::FaultCharacterString { fault_values }
            }
            fault_parameter_tag::EXTENDED => {
                let vendor_id = take_u16(rest, &mut idx)?;
                let extended_fault_type = take_u32(rest, &mut idx)?;
                let parameters = match take_pv(rest, &mut idx) {
                    Some(PropertyValue::OctetString(b)) => b.clone(),
                    _ => return Err(Error::decoding(idx, "extended fault params not octets")),
                };
                F::FaultExtended {
                    vendor_id,
                    extended_fault_type,
                    parameters,
                }
            }
            fault_parameter_tag::LIFE_SAFETY => {
                let Some(PropertyValue::List(inner)) = rest.get(idx) else {
                    return Err(Error::decoding(idx, "life-safety fault values missing"));
                };
                idx += 1;
                let fault_values = inner
                    .iter()
                    .map(|v| match v {
                        PropertyValue::Unsigned(u) => Ok(*u as u32),
                        _ => Err(Error::decoding(idx, "life-safety value not unsigned")),
                    })
                    .collect::<Result<Vec<u32>, Error>>()?;
                let mode_for_reference = take_dopr(rest, &mut idx)?;
                F::FaultLifeSafety {
                    fault_values,
                    mode_for_reference,
                }
            }
            fault_parameter_tag::STATE => {
                let list_of_values = take_property_state_list(rest, &mut idx)?;
                F::FaultState {
                    fault_values: list_of_values,
                }
            }
            fault_parameter_tag::STATUS_FLAGS => {
                let reference = take_dopr(rest, &mut idx)?;
                F::FaultStatusFlags { reference }
            }
            fault_parameter_tag::OUT_OF_RANGE => {
                let min_normal = match take_pv(rest, &mut idx) {
                    Some(PropertyValue::Double(v)) => *v,
                    Some(PropertyValue::Real(v)) => *v as f64,
                    _ => return Err(Error::decoding(idx, "min_normal not a double")),
                };
                let max_normal = match take_pv(rest, &mut idx) {
                    Some(PropertyValue::Double(v)) => *v,
                    Some(PropertyValue::Real(v)) => *v as f64,
                    _ => return Err(Error::decoding(idx, "max_normal not a double")),
                };
                F::FaultOutOfRange {
                    min_normal,
                    max_normal,
                }
            }
            fault_parameter_tag::LISTED => {
                let reference = take_dopr(rest, &mut idx)?;
                F::FaultListed { reference }
            }
            other => return Err(Error::decoding(0, format!("unknown fault tag {other}"))),
        })
    }
}

#[cfg(test)]
mod tests;
