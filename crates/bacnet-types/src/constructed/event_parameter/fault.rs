//! `BACnetFaultParameter` encode/decode (ASHRAE 135-2020 Clause 21).
//!
//! Split out of `mod.rs` to keep every file under the 700-LOC cap.

use super::*;
use crate::enums::FaultType;

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
    /// Return the fault algorithm selected by this parameter alternative.
    pub const fn fault_type(&self) -> FaultType {
        use crate::constructed::FaultParameters as F;
        match self {
            F::FaultNone => FaultType::NONE,
            F::FaultCharacterString { .. } => FaultType::FAULT_CHARACTERSTRING,
            F::FaultExtended { .. } => FaultType::FAULT_EXTENDED,
            F::FaultLifeSafety { .. } => FaultType::FAULT_LIFE_SAFETY,
            F::FaultState { .. } => FaultType::FAULT_STATE,
            F::FaultStatusFlags { .. } => FaultType::FAULT_STATUS_FLAGS,
            F::FaultOutOfRange { .. } => FaultType::FAULT_OUT_OF_RANGE,
            F::FaultListed { .. } => FaultType::FAULT_LISTED,
        }
    }

    /// Encode this fault-parameter set as a flat [`PropertyValue::List`].
    ///
    /// The first element is the variant tag as [`PropertyValue::Unsigned`],
    /// followed by the variant's fields in declaration order. This is the
    /// LEGACY pre-framing layout (see #154); the wire form is the full ASN.1
    /// framing from
    /// `bacnet_encoding::constructed::encode_fault_parameters`. See
    /// [`Self::decode_property_value`] for the inverse.
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
