//! APDU encoding and decoding per ASHRAE 135-2020 Clause 20.1.
//!
//! Covers all eight PDU types:
//! - [`ConfirmedRequest`] (Clause 20.1.2)
//! - [`UnconfirmedRequest`] (Clause 20.1.3)
//! - [`SimpleAck`] (Clause 20.1.4)
//! - [`ComplexAck`] (Clause 20.1.5)
//! - [`SegmentAck`] (Clause 20.1.6)
//! - [`ErrorPdu`] (Clause 20.1.7)
//! - [`RejectPdu`] (Clause 20.1.8)
//! - [`AbortPdu`] (Clause 20.1.9)

use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, ErrorClass, ErrorCode, PduType, RejectReason,
    UnconfirmedServiceChoice,
};
use bacnet_types::error::Error;
use bytes::{BufMut, Bytes, BytesMut};

use crate::primitives;
use crate::tags;

// ---------------------------------------------------------------------------
// Max-segments encoding
// ---------------------------------------------------------------------------

/// Decoded max-segments values indexed by the 3-bit field (0-7).
/// `None` means unspecified (0).
const MAX_SEGMENTS_DECODE: [Option<u8>; 8] = [
    None,      // 0 = unspecified
    Some(2),   // 1
    Some(4),   // 2
    Some(8),   // 3
    Some(16),  // 4
    Some(32),  // 5
    Some(64),  // 6
    Some(255), // 7 = >64 segments accepted
];

/// Encode a max-segments value to a 3-bit field.
fn encode_max_segments(value: Option<u8>) -> u8 {
    match value {
        None => 0,
        Some(2) => 1,
        Some(4) => 2,
        Some(8) => 3,
        Some(16) => 4,
        Some(32) => 5,
        Some(64) => 6,
        Some(_) => 7, // >64
    }
}

/// Decode a 3-bit max-segments field.
fn decode_max_segments(value: u8) -> Option<u8> {
    MAX_SEGMENTS_DECODE[(value & 0x07) as usize]
}

// ---------------------------------------------------------------------------
// Max-APDU-length encoding
// ---------------------------------------------------------------------------

/// Decoded max-APDU-length values indexed by the 4-bit field.
const MAX_APDU_DECODE: [u16; 6] = [50, 128, 206, 480, 1024, 1476];

/// Return true when `value` is one of the BACnet max-APDU-length encodings
/// defined by ASHRAE 135-2020 Clause 20.1.2.5.
pub fn is_valid_max_apdu_length(value: u16) -> bool {
    matches!(value, 50 | 128 | 206 | 480 | 1024 | 1476)
}

/// Validate a locally configured max-APDU-length value.
pub fn validate_max_apdu_length(value: u16) -> Result<(), Error> {
    if is_valid_max_apdu_length(value) {
        Ok(())
    } else {
        Err(Error::Encoding(format!(
            "invalid max-APDU-length {value}; expected one of 50, 128, 206, 480, 1024, 1476"
        )))
    }
}

/// Encode a max-APDU-length to a 4-bit field.
fn encode_max_apdu(value: u16) -> Result<u8, Error> {
    validate_max_apdu_length(value)?;
    match value {
        50 => Ok(0),
        128 => Ok(1),
        206 => Ok(2),
        480 => Ok(3),
        1024 => Ok(4),
        1476 => Ok(5),
        _ => unreachable!("validated max-APDU-length"),
    }
}

/// Decode a 4-bit max-APDU-length field.
fn decode_max_apdu(value: u8) -> Result<u16, Error> {
    let idx = (value & 0x0F) as usize;
    if idx < MAX_APDU_DECODE.len() {
        Ok(MAX_APDU_DECODE[idx])
    } else {
        Err(Error::decoding(
            1,
            format!("reserved max-APDU-length field value {idx}"),
        ))
    }
}

// ---------------------------------------------------------------------------
// PDU structs
// ---------------------------------------------------------------------------

/// Confirmed-Request PDU (Clause 20.1.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedRequest {
    pub segmented: bool,
    pub more_follows: bool,
    pub segmented_response_accepted: bool,
    pub max_segments: Option<u8>,
    pub max_apdu_length: u16,
    pub invoke_id: u8,
    pub sequence_number: Option<u8>,
    pub proposed_window_size: Option<u8>,
    pub service_choice: ConfirmedServiceChoice,
    pub service_request: Bytes,
}

/// Unconfirmed-Request PDU (Clause 20.1.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnconfirmedRequest {
    pub service_choice: UnconfirmedServiceChoice,
    pub service_request: Bytes,
}

/// SimpleACK PDU (Clause 20.1.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleAck {
    pub invoke_id: u8,
    pub service_choice: ConfirmedServiceChoice,
}

/// ComplexACK PDU (Clause 20.1.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplexAck {
    pub segmented: bool,
    pub more_follows: bool,
    pub invoke_id: u8,
    pub sequence_number: Option<u8>,
    pub proposed_window_size: Option<u8>,
    pub service_choice: ConfirmedServiceChoice,
    pub service_ack: Bytes,
}

/// SegmentACK PDU (Clause 20.1.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentAck {
    pub negative_ack: bool,
    pub sent_by_server: bool,
    pub invoke_id: u8,
    pub sequence_number: u8,
    pub actual_window_size: u8,
}

/// Error PDU (Clause 20.1.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorPdu {
    pub invoke_id: u8,
    pub service_choice: ConfirmedServiceChoice,
    pub error_class: ErrorClass,
    pub error_code: ErrorCode,
    pub error_data: Bytes,
}

/// Reject PDU (Clause 20.1.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectPdu {
    pub invoke_id: u8,
    pub reject_reason: RejectReason,
}

/// Abort PDU (Clause 20.1.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbortPdu {
    pub sent_by_server: bool,
    pub invoke_id: u8,
    pub abort_reason: AbortReason,
}

/// Sum type for all APDU PDU types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Apdu {
    ConfirmedRequest(ConfirmedRequest),
    UnconfirmedRequest(UnconfirmedRequest),
    SimpleAck(SimpleAck),
    ComplexAck(ComplexAck),
    SegmentAck(SegmentAck),
    Error(ErrorPdu),
    Reject(RejectPdu),
    Abort(AbortPdu),
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Encode an APDU to wire format.
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
        Apdu::Error(pdu) => {
            encode_error(buf, pdu);
            Ok(())
        }
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

fn encode_confirmed_request(buf: &mut BytesMut, pdu: &ConfirmedRequest) -> Result<(), Error> {
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

    let byte1 =
        (encode_max_segments(pdu.max_segments) << 4) | encode_max_apdu(pdu.max_apdu_length)?;
    buf.put_u8(byte1);

    buf.put_u8(pdu.invoke_id);

    if pdu.segmented {
        buf.put_u8(pdu.sequence_number.unwrap_or(0));
        buf.put_u8(valid_window_size(
            "ConfirmedRequest proposed-window-size",
            pdu.proposed_window_size.unwrap_or(1),
        )?);
    }

    buf.put_u8(pdu.service_choice.to_raw());
    buf.put_slice(&pdu.service_request);
    Ok(())
}

fn encode_unconfirmed_request(buf: &mut BytesMut, pdu: &UnconfirmedRequest) {
    buf.put_u8(PduType::UNCONFIRMED_REQUEST.to_raw() << 4);
    buf.put_u8(pdu.service_choice.to_raw());
    buf.put_slice(&pdu.service_request);
}

fn encode_simple_ack(buf: &mut BytesMut, pdu: &SimpleAck) {
    buf.put_u8(PduType::SIMPLE_ACK.to_raw() << 4);
    buf.put_u8(pdu.invoke_id);
    buf.put_u8(pdu.service_choice.to_raw());
}

fn encode_complex_ack(buf: &mut BytesMut, pdu: &ComplexAck) -> Result<(), Error> {
    let mut byte0 = PduType::COMPLEX_ACK.to_raw() << 4;
    if pdu.segmented {
        byte0 |= 0x08;
    }
    if pdu.more_follows {
        byte0 |= 0x04;
    }
    buf.put_u8(byte0);

    buf.put_u8(pdu.invoke_id);

    if pdu.segmented {
        buf.put_u8(pdu.sequence_number.unwrap_or(0));
        buf.put_u8(valid_window_size(
            "ComplexAck proposed-window-size",
            pdu.proposed_window_size.unwrap_or(1),
        )?);
    }

    buf.put_u8(pdu.service_choice.to_raw());
    buf.put_slice(&pdu.service_ack);
    Ok(())
}

fn encode_segment_ack(buf: &mut BytesMut, pdu: &SegmentAck) -> Result<(), Error> {
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
    buf.put_u8(valid_window_size(
        "SegmentACK actual-window-size",
        pdu.actual_window_size,
    )?);
    Ok(())
}

fn valid_window_size(field: &str, value: u8) -> Result<u8, Error> {
    if (1..=127).contains(&value) {
        Ok(value)
    } else {
        Err(Error::Encoding(format!(
            "{field} {value} outside BACnet range 1..=127"
        )))
    }
}

fn encode_error(buf: &mut BytesMut, pdu: &ErrorPdu) {
    buf.put_u8(PduType::ERROR.to_raw() << 4);
    buf.put_u8(pdu.invoke_id);
    buf.put_u8(pdu.service_choice.to_raw());
    primitives::encode_app_enumerated(buf, pdu.error_class.to_raw() as u32);
    primitives::encode_app_enumerated(buf, pdu.error_code.to_raw() as u32);
    if !pdu.error_data.is_empty() {
        buf.put_slice(&pdu.error_data);
    }
}

fn encode_reject(buf: &mut BytesMut, pdu: &RejectPdu) {
    buf.put_u8(PduType::REJECT.to_raw() << 4);
    buf.put_u8(pdu.invoke_id);
    buf.put_u8(pdu.reject_reason.to_raw());
}

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
        Err(Error::decoding(
            0,
            format!("unknown PDU type nibble: {:#x}", pdu_type_raw),
        ))
    }
}

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

fn decode_simple_ack(data: Bytes) -> Result<SimpleAck, Error> {
    if data.len() < 3 {
        return Err(Error::buffer_too_short(3, data.len()));
    }

    Ok(SimpleAck {
        invoke_id: data[1],
        service_choice: ConfirmedServiceChoice::from_raw(data[2]),
    })
}

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

fn decode_segment_ack(data: Bytes) -> Result<SegmentAck, Error> {
    if data.len() < 4 {
        return Err(Error::buffer_too_short(4, data.len()));
    }

    let byte0 = data[0];
    let raw_window = data[3];
    if raw_window == 0 || raw_window > 127 {
        return Err(Error::decoding(
            3,
            format!("SegmentACK actual-window-size {raw_window} outside range 1..=127"),
        ));
    }
    Ok(SegmentAck {
        negative_ack: byte0 & 0x02 != 0,
        sent_by_server: byte0 & 0x01 != 0,
        invoke_id: data[1],
        sequence_number: data[2],
        actual_window_size: raw_window,
    })
}

fn decode_error(data: Bytes) -> Result<ErrorPdu, Error> {
    if data.len() < 5 {
        return Err(Error::buffer_too_short(5, data.len()));
    }

    let invoke_id = data[1];
    let service_choice = ConfirmedServiceChoice::from_raw(data[2]);

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
        return Err(Error::decoding(
            tag_end,
            "ErrorPDU truncated at error class",
        ));
    }
    let error_class_raw = primitives::decode_unsigned(&data[tag_end..class_end])? as u16;
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
        return Err(Error::decoding(tag_end, "ErrorPDU truncated at error code"));
    }
    let error_code_raw = primitives::decode_unsigned(&data[tag_end..code_end])? as u16;
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

fn decode_reject(data: Bytes) -> Result<RejectPdu, Error> {
    if data.len() < 3 {
        return Err(Error::buffer_too_short(3, data.len()));
    }

    Ok(RejectPdu {
        invoke_id: data[1],
        reject_reason: RejectReason::from_raw(data[2]),
    })
}

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

#[cfg(test)]
mod tests;
