//! `BACnetFaultParameter` full ASN.1 framing.
//!
//! Production (ASHRAE 135-2020 Clause 21), modeled by [`FaultParameters`]:
//!
//! ```text
//! BACnetFaultParameter ::= CHOICE {
//!     none                 [0] NULL,
//!     fault-characterstring [1] SEQUENCE { list-of-fault-values [0] SEQUENCE OF CharacterString },
//!     fault-extended       [2] SEQUENCE { vendor-id [0] Unsigned16,
//!                                         extended-fault-type [1] Unsigned,
//!                                         parameters [2] SEQUENCE OF CHOICE { ... } },
//!     fault-life-safety    [3] SEQUENCE { list-of-fault-values [0] SEQUENCE OF BACnetLifeSafetyState,
//!                                         mode-property-reference [1] BACnetDeviceObjectPropertyReference },
//!     fault-state          [4] SEQUENCE { list-of-fault-values [0] SEQUENCE OF BACnetPropertyStates },
//!     fault-status-flags   [5] SEQUENCE { status-flags-reference [0] BACnetDeviceObjectPropertyReference },
//!     fault-out-of-range   [6] SEQUENCE { min-normal-value [0] CHOICE { real REAL, unsigned Unsigned,
//!                                                                         double Double, integer INTEGER },
//!                                         max-normal-value [1] CHOICE { ... same ... } },
//!     fault-listed         [7] SEQUENCE { fault-list-reference [0] BACnetDeviceObjectPropertyReference } }
//! ```
//!
//! The min/max inner CHOICE alternatives are untagged — discovered by their
//! APPLICATION tag (REAL=4, Unsigned=2, Double=5, INTEGER=3). The Rust type
//! stores `f64`, so encode always selects the `Double` alternative; decode
//! accepts all four.

use bacnet_types::constructed::FaultParameters;
use bacnet_types::error::Error;
use bytes::BytesMut;

use crate::primitives;
use crate::tags::{self, TagClass};

use super::{
    decode_app_character_string, decode_app_enumerated, decode_ctx_unsigned, decode_dopr_body,
    decode_property_state, encode_dopr_body, encode_property_state, expect_closing, expect_opening,
    MAX_FRAMED_ITEMS,
};

/// Encode a [`FaultParameters`] as its full CHOICE framing.
pub fn encode_fault_parameters(buf: &mut BytesMut, value: &FaultParameters) -> Result<(), Error> {
    use FaultParameters as F;
    match value {
        F::FaultNone => {
            // none [0] NULL — primitive context tag, no contents.
            tags::encode_tag(buf, 0, TagClass::Context, 0);
        }
        F::FaultCharacterString { fault_values } => {
            tags::encode_opening_tag(buf, 1);
            tags::encode_opening_tag(buf, 0);
            for s in fault_values {
                primitives::encode_app_character_string(buf, s)?;
            }
            tags::encode_closing_tag(buf, 0);
            tags::encode_closing_tag(buf, 1);
        }
        F::FaultExtended {
            vendor_id,
            extended_fault_type,
            parameters,
        } => {
            tags::encode_opening_tag(buf, 2);
            primitives::encode_ctx_unsigned(buf, 0, *vendor_id as u64);
            primitives::encode_ctx_unsigned(buf, 1, *extended_fault_type as u64);
            tags::encode_opening_tag(buf, 2);
            buf.extend_from_slice(parameters);
            tags::encode_closing_tag(buf, 2);
            tags::encode_closing_tag(buf, 2);
        }
        F::FaultLifeSafety {
            fault_values,
            mode_for_reference,
        } => {
            tags::encode_opening_tag(buf, 3);
            tags::encode_opening_tag(buf, 0);
            for v in fault_values {
                primitives::encode_app_enumerated(buf, *v);
            }
            tags::encode_closing_tag(buf, 0);
            // mode-property-reference [1] BACnetDeviceObjectPropertyReference
            tags::encode_opening_tag(buf, 1);
            encode_dopr_body(buf, mode_for_reference);
            tags::encode_closing_tag(buf, 1);
            tags::encode_closing_tag(buf, 3);
        }
        F::FaultState { fault_values } => {
            tags::encode_opening_tag(buf, 4);
            tags::encode_opening_tag(buf, 0);
            for state in fault_values {
                encode_property_state(buf, state);
            }
            tags::encode_closing_tag(buf, 0);
            tags::encode_closing_tag(buf, 4);
        }
        F::FaultStatusFlags { reference } => {
            tags::encode_opening_tag(buf, 5);
            tags::encode_opening_tag(buf, 0);
            encode_dopr_body(buf, reference);
            tags::encode_closing_tag(buf, 0);
            tags::encode_closing_tag(buf, 5);
        }
        F::FaultOutOfRange {
            min_normal,
            max_normal,
        } => {
            tags::encode_opening_tag(buf, 6);
            // min/max-normal-value [n] CHOICE — explicitly tagged around the
            // application-tagged alternative; the model stores f64, so the
            // `double` alternative encodes it losslessly.
            tags::encode_opening_tag(buf, 0);
            primitives::encode_app_double(buf, *min_normal);
            tags::encode_closing_tag(buf, 0);
            tags::encode_opening_tag(buf, 1);
            primitives::encode_app_double(buf, *max_normal);
            tags::encode_closing_tag(buf, 1);
            tags::encode_closing_tag(buf, 6);
        }
        F::FaultListed { reference } => {
            tags::encode_opening_tag(buf, 7);
            tags::encode_opening_tag(buf, 0);
            encode_dopr_body(buf, reference);
            tags::encode_closing_tag(buf, 0);
            tags::encode_closing_tag(buf, 7);
        }
    }
    Ok(())
}

/// Decode one application-tagged REAL/Unsigned/Double/INTEGER as f64 — the
/// `fault-out-of-range` min/max inner CHOICE alternatives.
fn decode_fault_normal_value(data: &[u8], offset: usize) -> Result<(f64, usize), Error> {
    let (t, pos) = tags::decode_tag(data, offset)?;
    if t.class != TagClass::Application || t.is_opening || t.is_closing {
        return Err(Error::decoding(
            offset,
            "fault-out-of-range: min/max-normal-value must be application-tagged",
        ));
    }
    let what = "fault-out-of-range min/max-normal-value";
    if t.number == tags::app_tag::BOOLEAN {
        return Err(Error::decoding(
            offset,
            format!("{what}: BOOLEAN is not a valid alternative"),
        ));
    }
    let end = pos
        .checked_add(t.length as usize)
        .ok_or_else(|| Error::decoding(pos, format!("{what}: length overflow")))?;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    let content = &data[pos..end];
    let value = match t.number {
        tags::app_tag::REAL => primitives::decode_real(content)? as f64,
        tags::app_tag::UNSIGNED => primitives::decode_unsigned(content)? as f64,
        tags::app_tag::DOUBLE => primitives::decode_double(content)?,
        tags::app_tag::SIGNED => primitives::decode_signed(content)? as f64,
        other => {
            return Err(Error::decoding(
                offset,
                format!("{what}: unexpected application tag {other}"),
            ))
        }
    };
    Ok((value, end))
}

/// Decode one framed [`FaultParameters`] CHOICE alternative at `offset`.
///
/// Returns the value and the offset past its encoding.
pub fn decode_fault_parameters(
    data: &[u8],
    offset: usize,
) -> Result<(FaultParameters, usize), Error> {
    use FaultParameters as F;
    let (tag, pos) = tags::decode_tag(data, offset)?;
    let what = "BACnetFaultParameter";

    // none [0] NULL — primitive context tag, no contents.
    if tag.is_context(0) {
        if tag.length != 0 {
            return Err(Error::decoding(
                offset,
                "BACnetFaultParameter: none [0] NULL must have no contents",
            ));
        }
        return Ok((F::FaultNone, pos));
    }
    if !tag.is_opening {
        return Err(Error::decoding(
            offset,
            "BACnetFaultParameter: expected an opening context tag or [0] NULL",
        ));
    }

    let value = match tag.number {
        1 => {
            let mut pos = expect_opening(data, pos, 0, what)?;
            let mut fault_values = Vec::new();
            loop {
                let (peek, _) = tags::decode_tag(data, pos)?;
                if peek.is_closing_tag(0) {
                    break;
                }
                if fault_values.len() >= MAX_FRAMED_ITEMS {
                    return Err(Error::decoding(
                        pos,
                        "fault-characterstring: list-of-fault-values exceeds limit",
                    ));
                }
                let (s, p) = decode_app_character_string(data, pos, what)?;
                fault_values.push(s);
                pos = p;
            }
            pos = expect_closing(data, pos, 0, what)?;
            pos = expect_closing(data, pos, 1, what)?;
            (F::FaultCharacterString { fault_values }, pos)
        }
        2 => {
            let mut pos = pos;
            let (raw, p) = decode_ctx_unsigned(data, pos, 0, what)?;
            let vendor_id = u16::try_from(raw)
                .map_err(|_| Error::decoding(pos, "fault-extended: vendor-id exceeds u16"))?;
            pos = p;
            let (raw, p) = decode_ctx_unsigned(data, pos, 1, what)?;
            let extended_fault_type = u32::try_from(raw).map_err(|_| {
                Error::decoding(pos, "fault-extended: extended-fault-type exceeds u32")
            })?;
            pos = p;
            pos = expect_opening(data, pos, 2, what)?;
            // parameters [2] — vendor-defined content preserved verbatim via
            // the raw scanner (not guaranteed to be well-formed TLVs).
            let (raw_params, p) = tags::extract_raw_context(data, pos, 2)?;
            pos = p;
            pos = expect_closing(data, pos, 2, what)?;
            (
                F::FaultExtended {
                    vendor_id,
                    extended_fault_type,
                    parameters: raw_params.to_vec(),
                },
                pos,
            )
        }
        3 => {
            let mut pos = expect_opening(data, pos, 0, what)?;
            let mut fault_values = Vec::new();
            loop {
                let (peek, _) = tags::decode_tag(data, pos)?;
                if peek.is_closing_tag(0) {
                    break;
                }
                if fault_values.len() >= MAX_FRAMED_ITEMS {
                    return Err(Error::decoding(
                        pos,
                        "fault-life-safety: list-of-fault-values exceeds limit",
                    ));
                }
                let (v, p) = decode_app_enumerated(data, pos, what)?;
                fault_values.push(v);
                pos = p;
            }
            pos = expect_closing(data, pos, 0, what)?;
            pos = expect_opening(data, pos, 1, what)?;
            let (mode_for_reference, p) = decode_dopr_body(data, pos, what)?;
            pos = p;
            pos = expect_closing(data, pos, 1, what)?;
            pos = expect_closing(data, pos, 3, what)?;
            (
                F::FaultLifeSafety {
                    fault_values,
                    mode_for_reference,
                },
                pos,
            )
        }
        4 => {
            let mut pos = expect_opening(data, pos, 0, what)?;
            let mut fault_values = Vec::new();
            loop {
                let (peek, _) = tags::decode_tag(data, pos)?;
                if peek.is_closing_tag(0) {
                    break;
                }
                if fault_values.len() >= MAX_FRAMED_ITEMS {
                    return Err(Error::decoding(
                        pos,
                        "fault-state: list-of-fault-values exceeds limit",
                    ));
                }
                let (state, p) = decode_property_state(data, pos)?;
                fault_values.push(state);
                pos = p;
            }
            pos = expect_closing(data, pos, 0, what)?;
            pos = expect_closing(data, pos, 4, what)?;
            (F::FaultState { fault_values }, pos)
        }
        5 => {
            let mut pos = expect_opening(data, pos, 0, what)?;
            let (reference, p) = decode_dopr_body(data, pos, what)?;
            pos = p;
            pos = expect_closing(data, pos, 0, what)?;
            pos = expect_closing(data, pos, 5, what)?;
            (F::FaultStatusFlags { reference }, pos)
        }
        6 => {
            let mut pos = expect_opening(data, pos, 0, what)?;
            let (min_normal, p) = decode_fault_normal_value(data, pos)?;
            pos = p;
            pos = expect_closing(data, pos, 0, what)?;
            pos = expect_opening(data, pos, 1, what)?;
            let (max_normal, p) = decode_fault_normal_value(data, pos)?;
            pos = p;
            pos = expect_closing(data, pos, 1, what)?;
            pos = expect_closing(data, pos, 6, what)?;
            (
                F::FaultOutOfRange {
                    min_normal,
                    max_normal,
                },
                pos,
            )
        }
        7 => {
            let mut pos = expect_opening(data, pos, 0, what)?;
            let (reference, p) = decode_dopr_body(data, pos, what)?;
            pos = p;
            pos = expect_closing(data, pos, 0, what)?;
            pos = expect_closing(data, pos, 7, what)?;
            (F::FaultListed { reference }, pos)
        }
        n => {
            return Err(Error::decoding(
                offset,
                format!("BACnetFaultParameter: unknown CHOICE tag [{n}]"),
            ));
        }
    };
    Ok(value)
}
