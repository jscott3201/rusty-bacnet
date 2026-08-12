//! `BACnetEventParameter` full ASN.1 framing.
//!
//! Production (ASHRAE 135-2020 Clause 21), alternatives modeled by
//! [`BACnetEventParameter`]:
//!
//! ```text
//! BACnetEventParameter ::= CHOICE {
//!     change-of-bitstring [0]  SEQUENCE { time-delay [0] Unsigned,
//!                                         bitmask [1] BIT STRING,
//!                                         list-of-bitstring-values [2] SEQUENCE OF BIT STRING },
//!     change-of-state     [1]  SEQUENCE { time-delay [0] Unsigned,
//!                                         list-of-values [1] SEQUENCE OF BACnetPropertyStates },
//!     change-of-value     [2]  SEQUENCE { time-delay [0] Unsigned,
//!                                         cov-criteria [1] CHOICE { bitmask [0] BIT STRING,
//!                                                     referenced-property-increment [1] REAL } },
//!     floating-limit      [4]  SEQUENCE { time-delay [0] Unsigned,
//!                                         setpoint-reference [1] BACnetDeviceObjectPropertyReference,
//!                                         low-diff-limit [2] REAL, high-diff-limit [3] REAL,
//!                                         deadband [4] REAL },
//!     out-of-range        [5]  SEQUENCE { time-delay [0] Unsigned,
//!                                         low-limit [1] REAL, high-limit [2] REAL,
//!                                         deadband [3] REAL },
//!     /* [6] omitted: proprietary parallel of the notification CHOICE [6] */
//!     /* [7] deprecated */    /* [12] reserved */  /* [19] omitted: change-of-reliability */
//!     extended            [9]  SEQUENCE { vendor-id [0] Unsigned16,
//!                                         extended-event-type [1] Unsigned,
//!                                         parameters [2] SEQUENCE OF CHOICE { ... } },
//!     none                [20] NULL,
//!     ... }
//! ```
//!
//! Alternatives with no Rust model (`command-failure [3]`,
//! `change-of-life-safety [8]`, `buffer-ready [10]`, `unsigned-range [11]`,
//! `access-event [13]`, `*-out-of-range [14..=16]`, `change-of-characterstring
//! [17]`, `change-of-status-flags [18]`, `none [20] NULL`,
//! `change-of-discrete-value [21]`, `change-of-timer [22]`, vendor tags) are
//! preserved byte-for-byte through [`BACnetEventParameter::Opaque`]. The
//! omitted/deprecated/reserved slots 6, 7, 12 and 19 are rejected on decode.

use bacnet_types::constructed::{BACnetEventParameter, ChangeOfValueCriteria};
use bacnet_types::error::Error;
use bytes::BytesMut;

use crate::primitives;
use crate::tags;

use super::{
    decode_app_bit_string, decode_ctx_bit_string, decode_ctx_real, decode_ctx_unsigned,
    decode_dopr_body, decode_property_state, encode_dopr_body, encode_property_state,
    expect_closing, expect_opening, MAX_FRAMED_ITEMS,
};

/// Context tags the Clause 21 production omits, deprecates, or reserves.
/// Decoding one means the peer is speaking something other than the 135-2020
/// production, so these are hard errors rather than Opaque shadows.
const RESERVED_EVENT_TAGS: [u8; 4] = [6, 7, 12, 19];

/// Encode a [`BACnetEventParameter`] as its full CHOICE framing.
pub fn encode_event_parameter(buf: &mut BytesMut, value: &BACnetEventParameter) {
    match value {
        BACnetEventParameter::ChangeOfBitstring {
            time_delay,
            bitmask,
            list_of_values,
        } => {
            tags::encode_opening_tag(buf, 0);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            primitives::encode_ctx_bit_string(buf, 1, bitmask.0, &bitmask.1);
            tags::encode_opening_tag(buf, 2);
            for (unused_bits, data) in list_of_values {
                primitives::encode_app_bit_string(buf, *unused_bits, data);
            }
            tags::encode_closing_tag(buf, 2);
            tags::encode_closing_tag(buf, 0);
        }
        BACnetEventParameter::ChangeOfState {
            time_delay,
            list_of_values,
        } => {
            tags::encode_opening_tag(buf, 1);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            tags::encode_opening_tag(buf, 1);
            for state in list_of_values {
                encode_property_state(buf, state);
            }
            tags::encode_closing_tag(buf, 1);
            tags::encode_closing_tag(buf, 1);
        }
        BACnetEventParameter::ChangeOfValue {
            time_delay,
            criteria,
        } => {
            tags::encode_opening_tag(buf, 2);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            // cov-criteria [1] CHOICE — a tagged CHOICE is always explicitly
            // tagged: opening/closing [1] around the alternative's own tag.
            tags::encode_opening_tag(buf, 1);
            match criteria {
                ChangeOfValueCriteria::Bitmask { unused_bits, data } => {
                    primitives::encode_ctx_bit_string(buf, 0, *unused_bits, data);
                }
                ChangeOfValueCriteria::ReferencedPropertyIncrement(increment) => {
                    primitives::encode_ctx_real(buf, 1, *increment);
                }
            }
            tags::encode_closing_tag(buf, 1);
            tags::encode_closing_tag(buf, 2);
        }
        BACnetEventParameter::FloatingLimit {
            time_delay,
            setpoint_reference,
            low_diff_limit,
            high_diff_limit,
            deadband,
        } => {
            tags::encode_opening_tag(buf, 4);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            // setpoint-reference [1] BACnetDeviceObjectPropertyReference (SEQUENCE)
            tags::encode_opening_tag(buf, 1);
            encode_dopr_body(buf, setpoint_reference);
            tags::encode_closing_tag(buf, 1);
            primitives::encode_ctx_real(buf, 2, *low_diff_limit);
            primitives::encode_ctx_real(buf, 3, *high_diff_limit);
            primitives::encode_ctx_real(buf, 4, *deadband);
            tags::encode_closing_tag(buf, 4);
        }
        BACnetEventParameter::OutOfRange {
            time_delay,
            low_limit,
            high_limit,
            deadband,
        } => {
            tags::encode_opening_tag(buf, 5);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            primitives::encode_ctx_real(buf, 1, *low_limit);
            primitives::encode_ctx_real(buf, 2, *high_limit);
            primitives::encode_ctx_real(buf, 3, *deadband);
            tags::encode_closing_tag(buf, 5);
        }
        BACnetEventParameter::Extended {
            vendor_id,
            extended_event_type,
            parameters,
        } => {
            tags::encode_opening_tag(buf, 9);
            primitives::encode_ctx_unsigned(buf, 0, *vendor_id as u64);
            primitives::encode_ctx_unsigned(buf, 1, *extended_event_type as u64);
            // parameters [2] SEQUENCE OF CHOICE — held as raw pre-encoded items.
            tags::encode_opening_tag(buf, 2);
            buf.extend_from_slice(parameters);
            tags::encode_closing_tag(buf, 2);
            tags::encode_closing_tag(buf, 9);
        }
        BACnetEventParameter::Opaque { tag, data } => {
            // Preserved alternative: its captured bytes are the complete
            // SEQUENCE body, re-emitted under its own opening/closing pair.
            // (A `none [20] NULL` captured as a zero-length primitive tag
            // decodes to `Opaque { tag: 20, data: [] }` and re-encodes in the
            // constructed form — same value, different but self-consistent
            // tag form.)
            tags::encode_opening_tag(buf, *tag);
            buf.extend_from_slice(data);
            tags::encode_closing_tag(buf, *tag);
        }
    }
}

/// Decode one framed [`BACnetEventParameter`] CHOICE alternative at `offset`.
///
/// Returns the value and the offset past its encoding. Unmodeled (but valid)
/// alternatives are preserved as [`BACnetEventParameter::Opaque`]; the
/// omitted/deprecated/reserved tags 6, 7, 12, 19 are rejected.
pub fn decode_event_parameter(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetEventParameter, usize), Error> {
    use BACnetEventParameter as EP;
    let (tag, pos) = tags::decode_tag(data, offset)?;
    let what = "BACnetEventParameter";

    if RESERVED_EVENT_TAGS.contains(&tag.number) {
        return Err(Error::decoding(
            offset,
            format!(
                "BACnetEventParameter: context tag [{}] is omitted/deprecated/reserved in Clause 21",
                tag.number
            ),
        ));
    }

    // Primitive alternatives: only none [20] NULL (zero-length). Any other
    // primitive ctx tag is preserved as opaque contents.
    if !tag.is_opening && !tag.is_closing {
        let end = pos
            .checked_add(tag.length as usize)
            .ok_or_else(|| Error::decoding(pos, "BACnetEventParameter: length overflow"))?;
        if end > data.len() {
            return Err(Error::buffer_too_short(end, data.len()));
        }
        return Ok((
            EP::Opaque {
                tag: tag.number,
                data: data[pos..end].to_vec(),
            },
            end,
        ));
    }

    if !tag.is_opening {
        return Err(Error::decoding(
            offset,
            "BACnetEventParameter: unexpected closing tag",
        ));
    }

    let value = match tag.number {
        0 => {
            let mut pos = pos;
            let (raw, p) = decode_ctx_unsigned(data, pos, 0, what)?;
            let time_delay = u32::try_from(raw)
                .map_err(|_| Error::decoding(pos, "change-of-bitstring: time-delay exceeds u32"))?;
            pos = p;
            let (bitmask, p) = decode_ctx_bit_string(data, pos, 1, what)?;
            pos = p;
            pos = expect_opening(data, pos, 2, what)?;
            let mut list_of_values = Vec::new();
            loop {
                let (peek, _) = tags::decode_tag(data, pos)?;
                if peek.is_closing_tag(2) {
                    break;
                }
                if list_of_values.len() >= MAX_FRAMED_ITEMS {
                    return Err(Error::decoding(
                        pos,
                        "change-of-bitstring: list-of-bitstring-values exceeds limit",
                    ));
                }
                let (bits, p) = decode_app_bit_string(data, pos, what)?;
                list_of_values.push(bits);
                pos = p;
            }
            pos = expect_closing(data, pos, 2, what)?;
            pos = expect_closing(data, pos, 0, what)?;
            (
                EP::ChangeOfBitstring {
                    time_delay,
                    bitmask,
                    list_of_values,
                },
                pos,
            )
        }
        1 => {
            let mut pos = pos;
            let (raw, p) = decode_ctx_unsigned(data, pos, 0, what)?;
            let time_delay = u32::try_from(raw)
                .map_err(|_| Error::decoding(pos, "change-of-state: time-delay exceeds u32"))?;
            pos = p;
            pos = expect_opening(data, pos, 1, what)?;
            let mut list_of_values = Vec::new();
            loop {
                let (peek, _) = tags::decode_tag(data, pos)?;
                if peek.is_closing_tag(1) {
                    break;
                }
                if list_of_values.len() >= MAX_FRAMED_ITEMS {
                    return Err(Error::decoding(
                        pos,
                        "change-of-state: list-of-values exceeds limit",
                    ));
                }
                let (state, p) = decode_property_state(data, pos)?;
                list_of_values.push(state);
                pos = p;
            }
            pos = expect_closing(data, pos, 1, what)?;
            pos = expect_closing(data, pos, 1, what)?;
            (
                EP::ChangeOfState {
                    time_delay,
                    list_of_values,
                },
                pos,
            )
        }
        2 => {
            let mut pos = pos;
            let (raw, p) = decode_ctx_unsigned(data, pos, 0, what)?;
            let time_delay = u32::try_from(raw)
                .map_err(|_| Error::decoding(pos, "change-of-value: time-delay exceeds u32"))?;
            pos = p;
            pos = expect_opening(data, pos, 1, what)?;
            let (inner, _) = tags::decode_tag(data, pos)?;
            let criteria = if inner.is_context(0) {
                let (bits, p) = decode_ctx_bit_string(data, pos, 0, what)?;
                pos = p;
                ChangeOfValueCriteria::Bitmask {
                    unused_bits: bits.0,
                    data: bits.1,
                }
            } else if inner.is_context(1) {
                let (increment, p) = decode_ctx_real(data, pos, 1, what)?;
                pos = p;
                ChangeOfValueCriteria::ReferencedPropertyIncrement(increment)
            } else {
                return Err(Error::decoding(
                    pos,
                    format!(
                        "change-of-value: unexpected cov-criteria tag [{}]",
                        inner.number
                    ),
                ));
            };
            pos = expect_closing(data, pos, 1, what)?;
            pos = expect_closing(data, pos, 2, what)?;
            (
                EP::ChangeOfValue {
                    time_delay,
                    criteria,
                },
                pos,
            )
        }
        4 => {
            let mut pos = pos;
            let (raw, p) = decode_ctx_unsigned(data, pos, 0, what)?;
            let time_delay = u32::try_from(raw)
                .map_err(|_| Error::decoding(pos, "floating-limit: time-delay exceeds u32"))?;
            pos = p;
            pos = expect_opening(data, pos, 1, what)?;
            let (setpoint_reference, p) = decode_dopr_body(data, pos, what)?;
            pos = p;
            pos = expect_closing(data, pos, 1, what)?;
            let (low_diff_limit, p) = decode_ctx_real(data, pos, 2, what)?;
            pos = p;
            let (high_diff_limit, p) = decode_ctx_real(data, pos, 3, what)?;
            pos = p;
            let (deadband, p) = decode_ctx_real(data, pos, 4, what)?;
            pos = p;
            pos = expect_closing(data, pos, 4, what)?;
            (
                EP::FloatingLimit {
                    time_delay,
                    setpoint_reference,
                    low_diff_limit,
                    high_diff_limit,
                    deadband,
                },
                pos,
            )
        }
        5 => {
            let mut pos = pos;
            let (raw, p) = decode_ctx_unsigned(data, pos, 0, what)?;
            let time_delay = u32::try_from(raw)
                .map_err(|_| Error::decoding(pos, "out-of-range: time-delay exceeds u32"))?;
            pos = p;
            let (low_limit, p) = decode_ctx_real(data, pos, 1, what)?;
            pos = p;
            let (high_limit, p) = decode_ctx_real(data, pos, 2, what)?;
            pos = p;
            let (deadband, p) = decode_ctx_real(data, pos, 3, what)?;
            pos = p;
            pos = expect_closing(data, pos, 5, what)?;
            (
                EP::OutOfRange {
                    time_delay,
                    low_limit,
                    high_limit,
                    deadband,
                },
                pos,
            )
        }
        9 => {
            let mut pos = pos;
            let (raw, p) = decode_ctx_unsigned(data, pos, 0, what)?;
            let vendor_id = u16::try_from(raw)
                .map_err(|_| Error::decoding(pos, "extended: vendor-id exceeds u16"))?;
            pos = p;
            let (raw, p) = decode_ctx_unsigned(data, pos, 1, what)?;
            let extended_event_type = u32::try_from(raw)
                .map_err(|_| Error::decoding(pos, "extended: extended-event-type exceeds u32"))?;
            pos = p;
            pos = expect_opening(data, pos, 2, what)?;
            // parameters [2] SEQUENCE OF CHOICE — preserved as raw bytes;
            // vendor-defined content is not guaranteed to be well-formed
            // TLVs, so skip it with the raw scanner rather than tag-parsing.
            let (raw_params, p) = tags::extract_raw_context(data, pos, 2)?;
            pos = p;
            pos = expect_closing(data, pos, 9, what)?;
            (
                EP::Extended {
                    vendor_id,
                    extended_event_type,
                    parameters: raw_params.to_vec(),
                },
                pos,
            )
        }
        n => {
            // Unmodeled constructed alternative: preserve the SEQUENCE body
            // verbatim and consume through the matching closing tag. The body
            // may be vendor bytes that are not well-formed TLVs (and for
            // legacy Opaque payloads, actively are not), so use the raw
            // scanner rather than tag-parsing.
            let (body, p) = tags::extract_raw_context(data, pos, n)?;
            (
                EP::Opaque {
                    tag: n,
                    data: body.to_vec(),
                },
                p,
            )
        }
    };
    Ok(value)
}
