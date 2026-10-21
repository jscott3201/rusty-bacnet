//! APDU encoding and decoding per ASHRAE 135-2020 Clause 20.1.
//!
//! Covers all eight PDU types:
//! - `ConfirmedRequest` (Clause 20.1.2)
//! - `UnconfirmedRequest` (Clause 20.1.3)
//! - `SimpleAck` (Clause 20.1.4)
//! - `ComplexAck` (Clause 20.1.5)
//! - `SegmentAck` (Clause 20.1.6)
//! - `ErrorPdu` (Clause 20.1.7)
//! - `RejectPdu` (Clause 20.1.8)
//! - `AbortPdu` (Clause 20.1.9)
//!
//! Those owned PDUs need `alloc`. Without it, [`header`] reads and writes the
//! header of an Unconfirmed-Request, an unsegmented Confirmed-Request or an
//! unsegmented ComplexACK, borrowing the service parameters in place.

#[cfg(feature = "alloc")]
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, ErrorClass, ErrorCode, PduType, RejectReason,
    UnconfirmedServiceChoice,
};
#[cfg(feature = "alloc")]
use bacnet_types::error::{DecodeError, DecodingKind, Error, RangeError};
#[cfg(feature = "alloc")]
use bytes::{BufMut, Bytes, BytesMut};

#[cfg(feature = "alloc")]
use crate::primitives;
#[cfg(feature = "alloc")]
use crate::tags;

#[cfg(feature = "alloc")]
mod formal_error;
pub mod header;

use header::{MaxApdu, MaxSegments};

// ---------------------------------------------------------------------------
// Max-segments encoding
// ---------------------------------------------------------------------------

/// Validate a configured maximum segment count for a Confirmed-Request header.
///
/// Clause 20.1.2.4 has no encoding for a finite capacity below two segments.
/// To decline segmented responses, set `segmented-response-accepted = false`
/// and use `None` rather than `Some(1)`. Only with `alloc`;
/// [`MaxSegments::from_capacity`] is the allocation-free check.
#[cfg(feature = "alloc")]
pub fn validate_max_segments(value: Option<u8>) -> Result<(), Error> {
    encode_max_segments(value).map(drop)
}

/// Encode a max-segments value to a 3-bit field.
///
/// Finite counts that fall between the Clause 20.1.2.4 rungs are rounded down
/// so the wire header never promises more receive capacity than configured.
#[cfg(feature = "alloc")]
fn encode_max_segments(value: Option<u8>) -> Result<u8, Error> {
    match MaxSegments::from_capacity(value) {
        Some(field) => Ok(field.bits()),
        None => Err(RangeError::new(
            "invalid max-segments-accepted {}; expected 2..=255 or unspecified",
        )
        .with_value(value.unwrap_or(0).into())
        .into()),
    }
}

/// Decode a 3-bit max-segments field.
#[cfg(feature = "alloc")]
fn decode_max_segments(value: u8) -> Option<u8> {
    MaxSegments::from_bits(value).count()
}

/// The segment count a configured `max-segments-accepted` actually promises a peer.
///
/// Clause 20.1.2.4 carries this parameter in three bits, and only B'001'
/// through B'110' name a number (2, 4, 8, 16, 32, 64). B'000' leaves capacity
/// unspecified; B'111' advertises capacity above 64 without an upper bound.
/// Neither tells the peer a limit, so both yield `None`.
///
/// This deliberately round-trips through the wire encoding rather than reading
/// `configured` directly: a value such as `Some(100)` encodes as B'111', so the
/// peer was told only that capacity exceeds 64, not that it is 100. Answering
/// from the configured number would claim a promise that was never sent.
/// Invalid finite capacities below two yield `None`; APDU encoding and client
/// startup reject those values through `validate_max_segments`.
pub fn advertised_max_segments(configured: Option<u8>) -> Option<u8> {
    match MaxSegments::from_capacity(configured)?.count() {
        // B'111' advertises capacity above 64, not a specific upper bound.
        Some(255) => None,
        decoded => decoded,
    }
}

// ---------------------------------------------------------------------------
// Max-APDU-length encoding
// ---------------------------------------------------------------------------

/// MinimumMessageSize: the smallest APDU any BACnet device accepts.
///
/// Clause 20.1.2.5 assigns the lowest max-APDU-length-accepted code, `B'0000'`,
/// to the 50-octet MinimumMessageSize capacity. Clause 12.11.18 sets the same
/// floor for `Max_APDU_Length_Accepted`, as does Clause 5.2.1.2 for a remote
/// peer's receive capacity.
pub const MINIMUM_MESSAGE_SIZE: u16 = 50;

/// Floor a raw local receive capacity to a Confirmed-Request header value.
///
/// Device `Max_APDU_Length_Accepted` and I-Am retain the raw unsigned value.
/// Only the Clause 20.1.2.5 header uses this six-code representation; flooring
/// must not change byte budgets or the Device declaration. Values below
/// MinimumMessageSize are invalid, while larger values saturate at 1476.
/// Only with `alloc`; [`MaxApdu::at_or_below`] is the allocation-free form.
#[cfg(feature = "alloc")]
pub fn max_apdu_header_at_or_below(capacity: u32) -> Result<u16, Error> {
    MaxApdu::at_or_below(capacity)
        .map(MaxApdu::octets)
        .ok_or_else(|| {
            RangeError::new("local APDU capacity {} is below 50")
                .with_value(capacity.into())
                .into()
        })
}

/// Return true when `value` is one of the BACnet max-APDU-length encodings
/// defined by ASHRAE 135-2020 Clause 20.1.2.5.
pub fn is_valid_max_apdu_length(value: u16) -> bool {
    MaxApdu::from_octets(value).is_some()
}

/// Validate an exact Confirmed-Request max-APDU-length header value. Only
/// with `alloc`.
#[cfg(feature = "alloc")]
pub fn validate_max_apdu_length(value: u16) -> Result<(), Error> {
    encode_max_apdu(value).map(drop)
}

/// Encode a max-APDU-length to a 4-bit field.
#[cfg(feature = "alloc")]
fn encode_max_apdu(value: u16) -> Result<u8, Error> {
    match MaxApdu::from_octets(value) {
        Some(field) => Ok(field.bits()),
        None => Err(RangeError::new(
            "invalid max-APDU-length {}; expected one of 50, 128, 206, 480, 1024, 1476",
        )
        .with_value(value.into())
        .into()),
    }
}

/// Decode a 4-bit max-APDU-length field.
#[cfg(feature = "alloc")]
fn decode_max_apdu(value: u8) -> Result<u16, Error> {
    match MaxApdu::from_bits(value) {
        Some(field) => Ok(field.octets()),
        None => Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            1,
            "reserved max-APDU-length field value {}",
        )
        .with_value(u64::from(value & 0x0F))
        .into()),
    }
}

// ---------------------------------------------------------------------------
// PDU structs
// ---------------------------------------------------------------------------

/// Confirmed-Request PDU (Clause 20.1.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct ConfirmedRequest {
    /// Set when this PDU is one segment of a segmented request; adds the sequence and window
    /// fields.
    pub segmented: bool,
    /// Set on every segment except the last one of a segmented request.
    pub more_follows: bool,
    /// Whether the requester can receive the reply as a segmented message.
    pub segmented_response_accepted: bool,
    /// Most segments the requester will accept in a reply; `None` leaves the limit unspecified.
    /// Must be `None` or at least 2; the 3-bit wire field rounds down to 2, 4, 8, 16, 32 or 64, and
    /// anything above 64 encodes as "more than 64".
    pub max_segments: Option<u8>,
    /// Largest APDU, in octets, the requester can receive; must be 50, 128, 206, 480, 1024 or 1476.
    pub max_apdu_length: u16,
    /// Identifier the requester uses to match the reply to this request.
    pub invoke_id: u8,
    /// Segment number within a segmented request; `None` for unsegmented PDUs. `None` on a
    /// segmented PDU encodes as 0.
    pub sequence_number: Option<u8>,
    /// Proposed window: how many segments the sender will transmit per SegmentACK (1..=127);
    /// `None` for unsegmented PDUs. `None` on a segmented PDU encodes as 1.
    pub proposed_window_size: Option<u8>,
    /// Which confirmed service is being requested.
    pub service_choice: ConfirmedServiceChoice,
    /// Raw encoded service request parameters (the bytes after the header).
    pub service_request: Bytes,
}

/// Unconfirmed-Request PDU (Clause 20.1.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct UnconfirmedRequest {
    /// Which unconfirmed service this PDU carries.
    pub service_choice: UnconfirmedServiceChoice,
    /// Raw encoded service request parameters.
    pub service_request: Bytes,
}

/// SimpleACK PDU (Clause 20.1.4).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct SimpleAck {
    /// Invoke ID of the confirmed request being acknowledged.
    pub invoke_id: u8,
    /// Confirmed service being acknowledged.
    pub service_choice: ConfirmedServiceChoice,
}

/// ComplexACK PDU (Clause 20.1.5).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct ComplexAck {
    /// Set when this PDU is one segment of a segmented reply; adds the sequence and window fields.
    pub segmented: bool,
    /// Set on every segment except the last one of a segmented reply.
    pub more_follows: bool,
    /// Invoke ID of the confirmed request this reply answers.
    pub invoke_id: u8,
    /// Segment number within a segmented reply; `None` for unsegmented PDUs. `None` on a
    /// segmented PDU encodes as 0.
    pub sequence_number: Option<u8>,
    /// Proposed window: how many segments the sender will transmit per SegmentACK (1..=127);
    /// `None` for unsegmented PDUs. `None` on a segmented PDU encodes as 1.
    pub proposed_window_size: Option<u8>,
    /// Confirmed service this reply belongs to.
    pub service_choice: ConfirmedServiceChoice,
    /// Raw encoded service acknowledgment parameters.
    pub service_ack: Bytes,
}

/// SegmentACK PDU (Clause 20.1.6).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct SegmentAck {
    /// Set when a segment arrived out of order; the peer resends starting with the segment after
    /// `sequence_number`.
    pub negative_ack: bool,
    /// Set when the SegmentACK comes from the server side (the device answering a request), clear
    /// when sent by the requester.
    pub sent_by_server: bool,
    /// Invoke ID of the segmented transaction being acknowledged.
    pub invoke_id: u8,
    /// Last segment received in order. It and every earlier segment are acknowledged; the peer
    /// continues (or, on a negative ack, resends) from the next one, modulo 256.
    pub sequence_number: u8,
    /// Window size, in segments (1..=127), the receiver will accept from now on.
    pub actual_window_size: u8,
}

/// Error PDU (Clause 20.1.7).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct ErrorPdu {
    /// Invoke ID of the confirmed request that failed.
    pub invoke_id: u8,
    /// Confirmed service that produced the error.
    pub service_choice: ConfirmedServiceChoice,
    /// Error class from the standard Error production.
    pub error_class: ErrorClass,
    /// Error code from the standard Error production.
    pub error_code: ErrorCode,
    /// Service-specific bytes following the error class and code; empty for a plain class/code
    /// error. For a service whose Clause 21 production replaces the plain pair
    /// (WritePropertyMultiple-Error, ChangeList-Error, CreateObject-Error,
    /// SubscribeCOVPropertyMultiple-Error, ConfirmedPrivateTransfer-Error, VTClose-Error) it
    /// holds the whole error body when that decodes in the formal form; a legacy class/code
    /// error from those services keeps only the bytes after the code.
    pub error_data: Bytes,
}

/// Reject PDU (Clause 20.1.8).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct RejectPdu {
    /// Invoke ID of the rejected request.
    pub invoke_id: u8,
    /// Why the request was rejected as malformed or unsupported.
    pub reject_reason: RejectReason,
}

/// Abort PDU (Clause 20.1.9).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct AbortPdu {
    /// Set when the abort comes from the server side of the transaction, clear when sent by the
    /// requester.
    pub sent_by_server: bool,
    /// Invoke ID of the transaction this PDU ends.
    pub invoke_id: u8,
    /// Why the transaction was aborted.
    pub abort_reason: AbortReason,
}

/// Sum type for all APDU PDU types.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub enum Apdu {
    /// Confirmed service request.
    ConfirmedRequest(ConfirmedRequest),
    /// Unconfirmed service request.
    UnconfirmedRequest(UnconfirmedRequest),
    /// Acknowledgment carrying no result data.
    SimpleAck(SimpleAck),
    /// Acknowledgment carrying result data, possibly segmented.
    ComplexAck(ComplexAck),
    /// Flow-control acknowledgment of received segments.
    SegmentAck(SegmentAck),
    /// Service failure reported with an error class and code.
    Error(ErrorPdu),
    /// Request refused before execution.
    Reject(RejectPdu),
    /// Transaction terminated early.
    Abort(AbortPdu),
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Encode an APDU to wire format.
#[cfg(feature = "alloc")]
pub fn encode_apdu(buf: &mut BytesMut, apdu: &Apdu) -> Result<(), Error> {
    match apdu {
        Apdu::ConfirmedRequest(pdu) => encode_confirmed_request(buf, pdu),
        Apdu::UnconfirmedRequest(pdu) => {
            encode_unconfirmed_request(buf, pdu);
            Ok(())
        }
        Apdu::SimpleAck(pdu) => {
            encode_simple_ack(buf, pdu);
            Ok(())
        }
        Apdu::ComplexAck(pdu) => encode_complex_ack(buf, pdu),
        Apdu::SegmentAck(pdu) => encode_segment_ack(buf, pdu),
        Apdu::Error(pdu) => encode_error(buf, pdu),
        Apdu::Reject(pdu) => {
            encode_reject(buf, pdu);
            Ok(())
        }
        Apdu::Abort(pdu) => {
            encode_abort(buf, pdu);
            Ok(())
        }
    }
}

#[cfg(feature = "alloc")]
fn encode_confirmed_request(buf: &mut BytesMut, pdu: &ConfirmedRequest) -> Result<(), Error> {
    // Every refusal comes before the first octet, so a refused APDU leaves
    // `buf` as it was.
    let max_segments = encode_max_segments(pdu.max_segments)?;
    let max_apdu = encode_max_apdu(pdu.max_apdu_length)?;
    let window = pdu
        .segmented
        .then(|| {
            valid_window_size(
                "ConfirmedRequest proposed-window-size {} outside BACnet range 1..=127",
                pdu.proposed_window_size.unwrap_or(1),
            )
        })
        .transpose()?;

    let mut byte0 = PduType::CONFIRMED_REQUEST.to_raw() << 4;
    if pdu.segmented {
        byte0 |= 0x08;
    }
    if pdu.more_follows {
        byte0 |= 0x04;
    }
    if pdu.segmented_response_accepted {
        byte0 |= 0x02;
    }
    buf.put_u8(byte0);

    let byte1 = (max_segments << 4) | max_apdu;
    buf.put_u8(byte1);

    buf.put_u8(pdu.invoke_id);

    if let Some(window) = window {
        buf.put_u8(pdu.sequence_number.unwrap_or(0));
        buf.put_u8(window);
    }

    buf.put_u8(pdu.service_choice.to_raw());
    buf.put_slice(&pdu.service_request);
    Ok(())
}

#[cfg(feature = "alloc")]
fn encode_unconfirmed_request(buf: &mut BytesMut, pdu: &UnconfirmedRequest) {
    buf.put_u8(PduType::UNCONFIRMED_REQUEST.to_raw() << 4);
    buf.put_u8(pdu.service_choice.to_raw());
    buf.put_slice(&pdu.service_request);
}

#[cfg(feature = "alloc")]
fn encode_simple_ack(buf: &mut BytesMut, pdu: &SimpleAck) {
    buf.put_u8(PduType::SIMPLE_ACK.to_raw() << 4);
    buf.put_u8(pdu.invoke_id);
    buf.put_u8(pdu.service_choice.to_raw());
}

#[cfg(feature = "alloc")]
fn encode_complex_ack(buf: &mut BytesMut, pdu: &ComplexAck) -> Result<(), Error> {
    // Refused before the first octet, so `buf` stays as it was.
    let window = pdu
        .segmented
        .then(|| {
            valid_window_size(
                "ComplexAck proposed-window-size {} outside BACnet range 1..=127",
                pdu.proposed_window_size.unwrap_or(1),
            )
        })
        .transpose()?;

    let mut byte0 = PduType::COMPLEX_ACK.to_raw() << 4;
    if pdu.segmented {
        byte0 |= 0x08;
    }
    if pdu.more_follows {
        byte0 |= 0x04;
    }
    buf.put_u8(byte0);

    buf.put_u8(pdu.invoke_id);

    if let Some(window) = window {
        buf.put_u8(pdu.sequence_number.unwrap_or(0));
        buf.put_u8(window);
    }

    buf.put_u8(pdu.service_choice.to_raw());
    buf.put_slice(&pdu.service_ack);
    Ok(())
}

#[cfg(feature = "alloc")]
fn encode_segment_ack(buf: &mut BytesMut, pdu: &SegmentAck) -> Result<(), Error> {
    // Refused before the first octet, so `buf` stays as it was.
    let window = valid_window_size(
        "SegmentACK actual-window-size {} outside BACnet range 1..=127",
        pdu.actual_window_size,
    )?;
    let mut byte0 = PduType::SEGMENT_ACK.to_raw() << 4;
    if pdu.negative_ack {
        byte0 |= 0x02;
    }
    if pdu.sent_by_server {
        byte0 |= 0x01;
    }
    buf.put_u8(byte0);
    buf.put_u8(pdu.invoke_id);
    buf.put_u8(pdu.sequence_number);
    buf.put_u8(window);
    Ok(())
}

/// `value` when it is a window size (1 to 127), or the [`Error::Range`]
/// `label` names it in, with a placeholder for the value.
#[cfg(feature = "alloc")]
fn valid_window_size(label: &'static str, value: u8) -> Result<u8, Error> {
    if (1..=127).contains(&value) {
        Ok(value)
    } else {
        Err(RangeError::new(label).with_value(value.into()).into())
    }
}

#[cfg(feature = "alloc")]
fn encode_error(buf: &mut BytesMut, pdu: &ErrorPdu) -> Result<(), Error> {
    // A legacy generic error may carry arbitrary service data beginning with
    // an opening tag. Only suppress the generic pair for a complete,
    // structurally valid formal body of a service that defines one.
    let formal =
        formal_error::decode_formal_body(pdu.service_choice, &pdu.error_data).unwrap_or(None);
    if let Some((error_class, error_code)) = formal {
        if error_class != pdu.error_class || error_code != pdu.error_code {
            return Err(Error::encode(
                "formal Error body disagrees with ErrorPdu class/code",
            ));
        }
    }

    buf.put_u8(PduType::ERROR.to_raw() << 4);
    buf.put_u8(pdu.invoke_id);
    buf.put_u8(pdu.service_choice.to_raw());
    if formal.is_some() {
        buf.put_slice(&pdu.error_data);
    } else {
        primitives::encode_app_enumerated(buf, u32::from(pdu.error_class.to_raw()));
        primitives::encode_app_enumerated(buf, u32::from(pdu.error_code.to_raw()));
        if !pdu.error_data.is_empty() {
            buf.put_slice(&pdu.error_data);
        }
    }
    Ok(())
}

#[cfg(feature = "alloc")]
fn encode_reject(buf: &mut BytesMut, pdu: &RejectPdu) {
    buf.put_u8(PduType::REJECT.to_raw() << 4);
    buf.put_u8(pdu.invoke_id);
    buf.put_u8(pdu.reject_reason.to_raw());
}

#[cfg(feature = "alloc")]
fn encode_abort(buf: &mut BytesMut, pdu: &AbortPdu) {
    let mut byte0 = PduType::ABORT.to_raw() << 4;
    if pdu.sent_by_server {
        byte0 |= 0x01;
    }
    buf.put_u8(byte0);
    buf.put_u8(pdu.invoke_id);
    buf.put_u8(pdu.abort_reason.to_raw());
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Decode an APDU from raw bytes.
#[cfg(feature = "alloc")]
pub fn decode_apdu(data: Bytes) -> Result<Apdu, Error> {
    if data.is_empty() {
        return Err(Error::decoding(0, "APDU data is empty"));
    }

    let pdu_type_raw = (data[0] >> 4) & 0x0F;
    let pdu_type = PduType::from_raw(pdu_type_raw);

    if pdu_type == PduType::CONFIRMED_REQUEST {
        decode_confirmed_request(data).map(Apdu::ConfirmedRequest)
    } else if pdu_type == PduType::UNCONFIRMED_REQUEST {
        decode_unconfirmed_request(data).map(Apdu::UnconfirmedRequest)
    } else if pdu_type == PduType::SIMPLE_ACK {
        decode_simple_ack(data).map(Apdu::SimpleAck)
    } else if pdu_type == PduType::COMPLEX_ACK {
        decode_complex_ack(data).map(Apdu::ComplexAck)
    } else if pdu_type == PduType::SEGMENT_ACK {
        decode_segment_ack(data).map(Apdu::SegmentAck)
    } else if pdu_type == PduType::ERROR {
        decode_error(data).map(Apdu::Error)
    } else if pdu_type == PduType::REJECT {
        decode_reject(data).map(Apdu::Reject)
    } else if pdu_type == PduType::ABORT {
        decode_abort(data).map(Apdu::Abort)
    } else {
        Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            0,
            "unknown PDU type nibble: {:#x}",
        )
        .with_value(pdu_type_raw.into())
        .into())
    }
}

#[cfg(feature = "alloc")]
fn decode_confirmed_request(data: Bytes) -> Result<ConfirmedRequest, Error> {
    if data.len() < 4 {
        return Err(Error::buffer_too_short(4, data.len()));
    }

    let byte0 = data[0];
    let segmented = byte0 & 0x08 != 0;
    let more_follows = byte0 & 0x04 != 0;
    let segmented_response_accepted = byte0 & 0x02 != 0;

    let byte1 = data[1];
    let max_segments = decode_max_segments((byte1 >> 4) & 0x07);
    let max_apdu_length = decode_max_apdu(byte1 & 0x0F)?;

    let invoke_id = data[2];
    let mut offset = 3;

    let (sequence_number, proposed_window_size) = if segmented {
        if data.len() < 6 {
            return Err(Error::decoding(
                offset,
                "segmented ConfirmedRequest too short for sequence/window fields",
            ));
        }
        let seq = data[offset];
        let win = data[offset + 1];
        offset += 2;
        (Some(seq), Some(win))
    } else {
        (None, None)
    };

    if offset >= data.len() {
        return Err(Error::decoding(
            offset,
            "ConfirmedRequest missing service choice",
        ));
    }
    let service_choice = ConfirmedServiceChoice::from_raw(data[offset]);
    offset += 1;

    let service_request = data.slice(offset..);

    Ok(ConfirmedRequest {
        segmented,
        more_follows,
        segmented_response_accepted,
        max_segments,
        max_apdu_length,
        invoke_id,
        sequence_number,
        proposed_window_size,
        service_choice,
        service_request,
    })
}

#[cfg(feature = "alloc")]
fn decode_unconfirmed_request(data: Bytes) -> Result<UnconfirmedRequest, Error> {
    if data.len() < 2 {
        return Err(Error::buffer_too_short(2, data.len()));
    }

    let service_choice = UnconfirmedServiceChoice::from_raw(data[1]);
    let service_request = data.slice(2..);

    Ok(UnconfirmedRequest {
        service_choice,
        service_request,
    })
}

#[cfg(feature = "alloc")]
fn decode_simple_ack(data: Bytes) -> Result<SimpleAck, Error> {
    if data.len() < 3 {
        return Err(Error::buffer_too_short(3, data.len()));
    }

    Ok(SimpleAck {
        invoke_id: data[1],
        service_choice: ConfirmedServiceChoice::from_raw(data[2]),
    })
}

#[cfg(feature = "alloc")]
fn decode_complex_ack(data: Bytes) -> Result<ComplexAck, Error> {
    if data.len() < 3 {
        return Err(Error::buffer_too_short(3, data.len()));
    }

    let byte0 = data[0];
    let segmented = byte0 & 0x08 != 0;
    let more_follows = byte0 & 0x04 != 0;

    let invoke_id = data[1];
    let mut offset = 2;

    let (sequence_number, proposed_window_size) = if segmented {
        if data.len() < 5 {
            return Err(Error::decoding(
                offset,
                "segmented ComplexAck too short for sequence/window fields",
            ));
        }
        let seq = data[offset];
        let win = data[offset + 1];
        offset += 2;
        (Some(seq), Some(win))
    } else {
        (None, None)
    };

    if offset >= data.len() {
        return Err(Error::decoding(offset, "ComplexAck missing service choice"));
    }
    let service_choice = ConfirmedServiceChoice::from_raw(data[offset]);
    offset += 1;

    let service_ack = data.slice(offset..);

    Ok(ComplexAck {
        segmented,
        more_follows,
        invoke_id,
        sequence_number,
        proposed_window_size,
        service_choice,
        service_ack,
    })
}

#[cfg(feature = "alloc")]
fn decode_segment_ack(data: Bytes) -> Result<SegmentAck, Error> {
    if data.len() < 4 {
        return Err(Error::buffer_too_short(4, data.len()));
    }

    let byte0 = data[0];
    let raw_window = data[3];
    if raw_window == 0 || raw_window > 127 {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            3,
            "SegmentACK actual-window-size {} outside range 1..=127",
        )
        .with_value(raw_window.into())
        .into());
    }
    Ok(SegmentAck {
        negative_ack: byte0 & 0x02 != 0,
        sent_by_server: byte0 & 0x01 != 0,
        invoke_id: data[1],
        sequence_number: data[2],
        actual_window_size: raw_window,
    })
}

#[cfg(feature = "alloc")]
fn decode_error(data: Bytes) -> Result<ErrorPdu, Error> {
    if data.len() < 5 {
        return Err(Error::buffer_too_short(5, data.len()));
    }

    let invoke_id = data[1];
    let service_choice = ConfirmedServiceChoice::from_raw(data[2]);

    if let Some((error_class, error_code)) =
        formal_error::decode_formal_body(service_choice, &data[3..])?
    {
        return Ok(ErrorPdu {
            invoke_id,
            service_choice,
            error_class,
            error_code,
            error_data: data.slice(3..),
        });
    }

    let mut offset = 3;
    let (tag, tag_end) = tags::decode_tag(&data, offset)?;
    if tag.class != tags::TagClass::Application || tag.number != tags::app_tag::ENUMERATED {
        return Err(Error::decoding(
            offset,
            "ErrorPDU error class: expected application-tagged enumerated",
        ));
    }
    let class_end = tag_end
        .checked_add(tag.length as usize)
        .ok_or_else(|| Error::decoding(tag_end, "ErrorPDU error class length overflow"))?;
    if class_end > data.len() {
        return Err(Error::buffer_too_short(class_end, data.len()));
    }
    let error_class_raw = primitives::decode_unsigned(&data[tag_end..class_end])?;
    let error_class_raw = u16::try_from(error_class_raw).map_err(|_| {
        DecodeError::new(
            DecodingKind::OutOfRange,
            tag_end,
            "ErrorPDU error class {} exceeds u16",
        )
        .with_value(error_class_raw)
    })?;
    offset = class_end;

    let (tag, tag_end) = tags::decode_tag(&data, offset)?;
    if tag.class != tags::TagClass::Application || tag.number != tags::app_tag::ENUMERATED {
        return Err(Error::decoding(
            offset,
            "ErrorPDU error code: expected application-tagged enumerated",
        ));
    }
    let code_end = tag_end
        .checked_add(tag.length as usize)
        .ok_or_else(|| Error::decoding(tag_end, "ErrorPDU error code length overflow"))?;
    if code_end > data.len() {
        return Err(Error::buffer_too_short(code_end, data.len()));
    }
    let error_code_raw = primitives::decode_unsigned(&data[tag_end..code_end])?;
    let error_code_raw = u16::try_from(error_code_raw).map_err(|_| {
        DecodeError::new(
            DecodingKind::OutOfRange,
            tag_end,
            "ErrorPDU error code {} exceeds u16",
        )
        .with_value(error_code_raw)
    })?;
    offset = code_end;

    let error_data = if offset < data.len() {
        data.slice(offset..)
    } else {
        Bytes::new()
    };

    Ok(ErrorPdu {
        invoke_id,
        service_choice,
        error_class: ErrorClass::from_raw(error_class_raw),
        error_code: ErrorCode::from_raw(error_code_raw),
        error_data,
    })
}

#[cfg(feature = "alloc")]
fn decode_reject(data: Bytes) -> Result<RejectPdu, Error> {
    if data.len() < 3 {
        return Err(Error::buffer_too_short(3, data.len()));
    }

    Ok(RejectPdu {
        invoke_id: data[1],
        reject_reason: RejectReason::from_raw(data[2]),
    })
}

#[cfg(feature = "alloc")]
fn decode_abort(data: Bytes) -> Result<AbortPdu, Error> {
    if data.len() < 3 {
        return Err(Error::buffer_too_short(3, data.len()));
    }

    let byte0 = data[0];
    Ok(AbortPdu {
        sent_by_server: byte0 & 0x01 != 0,
        invoke_id: data[1],
        abort_reason: AbortReason::from_raw(data[2]),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "alloc"))]
mod change_list_error_tests;
#[cfg(all(test, feature = "alloc"))]
mod encode_refusal_tests;
#[cfg(all(test, feature = "alloc"))]
mod structured_error_tests;
#[cfg(all(test, feature = "alloc"))]
mod tests;
#[cfg(all(test, feature = "alloc"))]
mod wpm_error_tests;
