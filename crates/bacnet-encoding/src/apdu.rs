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

/// Encode a max-APDU-length to a 4-bit field.
fn encode_max_apdu(value: u16) -> u8 {
    match value {
        50 => 0,
        128 => 1,
        206 => 2,
        480 => 3,
        1024 => 4,
        _ => 5, // 1476 (default)
    }
}

/// Decode a 4-bit max-APDU-length field.
fn decode_max_apdu(value: u8) -> u16 {
    let idx = (value & 0x0F) as usize;
    if idx < MAX_APDU_DECODE.len() {
        MAX_APDU_DECODE[idx]
    } else {
        1476
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
pub fn encode_apdu(buf: &mut BytesMut, apdu: &Apdu) {
    match apdu {
        Apdu::ConfirmedRequest(pdu) => encode_confirmed_request(buf, pdu),
        Apdu::UnconfirmedRequest(pdu) => encode_unconfirmed_request(buf, pdu),
        Apdu::SimpleAck(pdu) => encode_simple_ack(buf, pdu),
        Apdu::ComplexAck(pdu) => encode_complex_ack(buf, pdu),
        Apdu::SegmentAck(pdu) => encode_segment_ack(buf, pdu),
        Apdu::Error(pdu) => encode_error(buf, pdu),
        Apdu::Reject(pdu) => encode_reject(buf, pdu),
        Apdu::Abort(pdu) => encode_abort(buf, pdu),
    }
}

fn encode_confirmed_request(buf: &mut BytesMut, pdu: &ConfirmedRequest) {
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

    let byte1 = (encode_max_segments(pdu.max_segments) << 4) | encode_max_apdu(pdu.max_apdu_length);
    buf.put_u8(byte1);

    buf.put_u8(pdu.invoke_id);

    if pdu.segmented {
        buf.put_u8(pdu.sequence_number.unwrap_or(0));
        buf.put_u8(pdu.proposed_window_size.unwrap_or(1).clamp(1, 127));
    }

    buf.put_u8(pdu.service_choice.to_raw());
    buf.put_slice(&pdu.service_request);
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

fn encode_complex_ack(buf: &mut BytesMut, pdu: &ComplexAck) {
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
        buf.put_u8(pdu.proposed_window_size.unwrap_or(1).clamp(1, 127));
    }

    buf.put_u8(pdu.service_choice.to_raw());
    buf.put_slice(&pdu.service_ack);
}

fn encode_segment_ack(buf: &mut BytesMut, pdu: &SegmentAck) {
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
    buf.put_u8(pdu.actual_window_size.clamp(1, 127));
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
    let max_apdu_length = decode_max_apdu(byte1 & 0x0F);

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
    Ok(SegmentAck {
        negative_ack: byte0 & 0x02 != 0,
        sent_by_server: byte0 & 0x01 != 0,
        invoke_id: data[1],
        sequence_number: data[2],
        actual_window_size: data[3],
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
mod tests {
    use super::*;

    fn encode_to_vec(apdu: &Apdu) -> Vec<u8> {
        let mut buf = BytesMut::with_capacity(64);
        encode_apdu(&mut buf, apdu);
        buf.to_vec()
    }

    // --- Max-segments / max-APDU helpers ---

    #[test]
    fn max_segments_round_trip() {
        assert_eq!(decode_max_segments(encode_max_segments(None)), None);
        assert_eq!(decode_max_segments(encode_max_segments(Some(2))), Some(2));
        assert_eq!(decode_max_segments(encode_max_segments(Some(4))), Some(4));
        assert_eq!(decode_max_segments(encode_max_segments(Some(8))), Some(8));
        assert_eq!(decode_max_segments(encode_max_segments(Some(16))), Some(16));
        assert_eq!(decode_max_segments(encode_max_segments(Some(32))), Some(32));
        assert_eq!(decode_max_segments(encode_max_segments(Some(64))), Some(64));
        assert_eq!(
            decode_max_segments(encode_max_segments(Some(100))),
            Some(255)
        );
    }

    #[test]
    fn max_apdu_round_trip() {
        assert_eq!(decode_max_apdu(encode_max_apdu(50)), 50);
        assert_eq!(decode_max_apdu(encode_max_apdu(128)), 128);
        assert_eq!(decode_max_apdu(encode_max_apdu(206)), 206);
        assert_eq!(decode_max_apdu(encode_max_apdu(480)), 480);
        assert_eq!(decode_max_apdu(encode_max_apdu(1024)), 1024);
        assert_eq!(decode_max_apdu(encode_max_apdu(1476)), 1476);
        // Unknown value defaults to 1476
        assert_eq!(decode_max_apdu(encode_max_apdu(9999)), 1476);
    }

    // --- ConfirmedRequest ---

    #[test]
    fn confirmed_request_non_segmented_round_trip() {
        let pdu = ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: true,
            max_segments: Some(4),
            max_apdu_length: 1476,
            invoke_id: 42,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: Bytes::from_static(&[0x0C, 0x02, 0x00, 0x00, 0x01]),
        };
        let apdu = Apdu::ConfirmedRequest(pdu);
        let encoded = encode_to_vec(&apdu);
        let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
        assert_eq!(apdu, decoded);
    }

    #[test]
    fn confirmed_request_segmented_round_trip() {
        let pdu = ConfirmedRequest {
            segmented: true,
            more_follows: true,
            segmented_response_accepted: true,
            max_segments: Some(64),
            max_apdu_length: 480,
            invoke_id: 7,
            sequence_number: Some(3),
            proposed_window_size: Some(16),
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            service_request: Bytes::from_static(&[0xAA, 0xBB]),
        };
        let apdu = Apdu::ConfirmedRequest(pdu);
        let encoded = encode_to_vec(&apdu);
        let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
        assert_eq!(apdu, decoded);
    }

    #[test]
    fn confirmed_request_wire_format() {
        let pdu = ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 1,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: Bytes::new(),
        };
        let encoded = encode_to_vec(&Apdu::ConfirmedRequest(pdu));
        // byte0: (0<<4) | 0 = 0x00
        // byte1: (0<<4) | 5 = 0x05  (unspecified segments, 1476 apdu)
        // invoke_id: 0x01
        // service_choice: ReadProperty = 12 = 0x0C
        assert_eq!(&encoded[..4], &[0x00, 0x05, 0x01, 0x0C]);
    }

    // --- UnconfirmedRequest ---

    #[test]
    fn unconfirmed_request_round_trip() {
        let pdu = UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::WHO_IS,
            service_request: Bytes::from_static(&[0x01, 0x02, 0x03]),
        };
        let apdu = Apdu::UnconfirmedRequest(pdu);
        let encoded = encode_to_vec(&apdu);
        let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
        assert_eq!(apdu, decoded);
    }

    #[test]
    fn unconfirmed_request_wire_format() {
        let pdu = UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: Bytes::new(),
        };
        let encoded = encode_to_vec(&Apdu::UnconfirmedRequest(pdu));
        // byte0: (1<<4) = 0x10
        // service_choice: IAm = 0
        assert_eq!(encoded, vec![0x10, 0x00]);
    }

    // --- SimpleAck ---

    #[test]
    fn simple_ack_round_trip() {
        let pdu = SimpleAck {
            invoke_id: 99,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        };
        let apdu = Apdu::SimpleAck(pdu);
        let encoded = encode_to_vec(&apdu);
        assert_eq!(encoded.len(), 3);
        let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
        assert_eq!(apdu, decoded);
    }

    #[test]
    fn simple_ack_wire_format() {
        let pdu = SimpleAck {
            invoke_id: 5,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        };
        let encoded = encode_to_vec(&Apdu::SimpleAck(pdu));
        // byte0: (2<<4) = 0x20
        assert_eq!(encoded, vec![0x20, 0x05, 0x0C]);
    }

    // --- ComplexAck ---

    #[test]
    fn complex_ack_non_segmented_round_trip() {
        let pdu = ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: 42,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_ack: Bytes::from_static(&[0xDE, 0xAD]),
        };
        let apdu = Apdu::ComplexAck(pdu);
        let encoded = encode_to_vec(&apdu);
        let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
        assert_eq!(apdu, decoded);
    }

    #[test]
    fn complex_ack_segmented_round_trip() {
        let pdu = ComplexAck {
            segmented: true,
            more_follows: false,
            invoke_id: 10,
            sequence_number: Some(5),
            proposed_window_size: Some(8),
            service_choice: ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
            service_ack: Bytes::from_static(&[0x01]),
        };
        let apdu = Apdu::ComplexAck(pdu);
        let encoded = encode_to_vec(&apdu);
        let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
        assert_eq!(apdu, decoded);
    }

    // --- SegmentAck ---

    #[test]
    fn segment_ack_round_trip() {
        let pdu = SegmentAck {
            negative_ack: true,
            sent_by_server: false,
            invoke_id: 55,
            sequence_number: 12,
            actual_window_size: 4,
        };
        let apdu = Apdu::SegmentAck(pdu);
        let encoded = encode_to_vec(&apdu);
        assert_eq!(encoded.len(), 4);
        let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
        assert_eq!(apdu, decoded);
    }

    #[test]
    fn segment_ack_flags() {
        // Both flags set
        let pdu = SegmentAck {
            negative_ack: true,
            sent_by_server: true,
            invoke_id: 1,
            sequence_number: 0,
            actual_window_size: 1,
        };
        let encoded = encode_to_vec(&Apdu::SegmentAck(pdu));
        // byte0: (4<<4) | 0x02 | 0x01 = 0x43
        assert_eq!(encoded[0], 0x43);
    }

    // --- Error ---

    #[test]
    fn error_round_trip() {
        let pdu = ErrorPdu {
            invoke_id: 10,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            error_class: ErrorClass::PROPERTY,
            error_code: ErrorCode::UNKNOWN_PROPERTY,
            error_data: Bytes::new(),
        };
        let apdu = Apdu::Error(pdu);
        let encoded = encode_to_vec(&apdu);
        let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
        assert_eq!(apdu, decoded);
    }

    #[test]
    fn error_with_trailing_data_round_trip() {
        let pdu = ErrorPdu {
            invoke_id: 20,
            service_choice: ConfirmedServiceChoice::CREATE_OBJECT,
            error_class: ErrorClass::OBJECT,
            error_code: ErrorCode::NO_OBJECTS_OF_SPECIFIED_TYPE,
            error_data: Bytes::from_static(&[0x01, 0x02, 0x03]),
        };
        let apdu = Apdu::Error(pdu);
        let encoded = encode_to_vec(&apdu);
        let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
        assert_eq!(apdu, decoded);
    }

    // --- Reject ---

    #[test]
    fn reject_round_trip() {
        let pdu = RejectPdu {
            invoke_id: 77,
            reject_reason: RejectReason::INVALID_TAG,
        };
        let apdu = Apdu::Reject(pdu);
        let encoded = encode_to_vec(&apdu);
        assert_eq!(encoded.len(), 3);
        let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
        assert_eq!(apdu, decoded);
    }

    // --- Abort ---

    #[test]
    fn abort_round_trip() {
        let pdu = AbortPdu {
            sent_by_server: true,
            invoke_id: 33,
            abort_reason: AbortReason::BUFFER_OVERFLOW,
        };
        let apdu = Apdu::Abort(pdu);
        let encoded = encode_to_vec(&apdu);
        assert_eq!(encoded.len(), 3);
        let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
        assert_eq!(apdu, decoded);
    }

    #[test]
    fn abort_server_flag() {
        let pdu = AbortPdu {
            sent_by_server: true,
            invoke_id: 0,
            abort_reason: AbortReason::OTHER,
        };
        let encoded = encode_to_vec(&Apdu::Abort(pdu));
        // byte0: (7<<4) | 0x01 = 0x71
        assert_eq!(encoded[0], 0x71);

        let pdu = AbortPdu {
            sent_by_server: false,
            invoke_id: 0,
            abort_reason: AbortReason::OTHER,
        };
        let encoded = encode_to_vec(&Apdu::Abort(pdu));
        // byte0: (7<<4) = 0x70
        assert_eq!(encoded[0], 0x70);
    }

    // --- Decode errors ---

    #[test]
    fn decode_empty_data() {
        assert!(decode_apdu(Bytes::new()).is_err());
    }

    #[test]
    fn decode_unknown_pdu_type() {
        // PDU type nibble 0x0F (reserved)
        assert!(decode_apdu(Bytes::from_static(&[0xF0])).is_err());
    }

    #[test]
    fn decode_truncated_confirmed_request() {
        // Only 2 bytes, need at least 4
        assert!(decode_apdu(Bytes::from_static(&[0x00, 0x05])).is_err());
    }

    #[test]
    fn decode_truncated_simple_ack() {
        // Only 2 bytes, need 3
        assert!(decode_apdu(Bytes::from_static(&[0x20, 0x01])).is_err());
    }

    // --- Segmented APDU edge cases ---

    #[test]
    fn decode_truncated_segmented_confirmed_request() {
        // Segmented flag set but not enough bytes for sequence/window
        // byte0: (0<<4) | 0x08 (segmented) = 0x08
        // byte1: max-segments/apdu = 0x05
        // invoke_id: 0x01
        // Missing: sequence_number, window_size, service_choice
        assert!(decode_apdu(Bytes::from_static(&[0x08, 0x05, 0x01])).is_err());
    }

    #[test]
    fn decode_segmented_confirmed_request_missing_service() {
        // Segmented, has sequence/window, but no service choice
        // byte0: 0x08 (segmented), byte1: 0x05, invoke_id: 1, seq: 0, win: 1
        assert!(decode_apdu(Bytes::from_static(&[0x08, 0x05, 0x01, 0x00, 0x01])).is_err());
    }

    #[test]
    fn decode_truncated_segmented_complex_ack() {
        // Segmented ComplexAck but too short for sequence/window
        // byte0: (3<<4) | 0x08 = 0x38
        // invoke_id: 0x01
        // Missing: sequence_number, window_size
        assert!(decode_apdu(Bytes::from_static(&[0x38, 0x01])).is_err());
    }

    #[test]
    fn decode_complex_ack_missing_service_choice() {
        // Non-segmented ComplexAck, only 2 bytes (need 3 minimum)
        assert!(decode_apdu(Bytes::from_static(&[0x30, 0x01])).is_err());
    }

    #[test]
    fn decode_truncated_segment_ack() {
        // SegmentAck needs exactly 4 bytes
        assert!(decode_apdu(Bytes::from_static(&[0x40, 0x01, 0x02])).is_err());
    }

    #[test]
    fn decode_truncated_error_pdu() {
        // Error PDU needs at least 5 bytes (type, invoke, service, error_class tag+value)
        assert!(decode_apdu(Bytes::from_static(&[0x50, 0x01, 0x0C, 0x91])).is_err());
    }

    #[test]
    fn decode_truncated_reject() {
        // Reject needs 3 bytes
        assert!(decode_apdu(Bytes::from_static(&[0x60, 0x01])).is_err());
    }

    #[test]
    fn decode_truncated_abort() {
        // Abort needs 3 bytes
        assert!(decode_apdu(Bytes::from_static(&[0x70, 0x01])).is_err());
    }

    // --- APDU round-trip edge cases ---

    #[test]
    fn confirmed_request_empty_service_data() {
        let pdu = ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 0,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: Bytes::new(),
        };
        let apdu = Apdu::ConfirmedRequest(pdu);
        let encoded = encode_to_vec(&apdu);
        let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
        assert_eq!(apdu, decoded);
    }

    #[test]
    fn confirmed_request_invoke_id_zero() {
        let pdu = ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: true,
            max_segments: Some(64),
            max_apdu_length: 1476,
            invoke_id: 0,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            service_request: Bytes::from_static(&[0xAA]),
        };
        let apdu = Apdu::ConfirmedRequest(pdu);
        let encoded = encode_to_vec(&apdu);
        let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
        assert_eq!(apdu, decoded);
    }

    #[test]
    fn confirmed_request_invoke_id_255() {
        let pdu = ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: true,
            max_segments: None,
            max_apdu_length: 480,
            invoke_id: 255,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: Bytes::from_static(&[0x01]),
        };
        let apdu = Apdu::ConfirmedRequest(pdu);
        let encoded = encode_to_vec(&apdu);
        let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
        assert_eq!(apdu, decoded);
    }

    #[test]
    fn segmented_request_sequence_zero() {
        let pdu = ConfirmedRequest {
            segmented: true,
            more_follows: true,
            segmented_response_accepted: true,
            max_segments: Some(64),
            max_apdu_length: 480,
            invoke_id: 5,
            sequence_number: Some(0),
            proposed_window_size: Some(1),
            service_choice: ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
            service_request: Bytes::from_static(&[0x01, 0x02]),
        };
        let apdu = Apdu::ConfirmedRequest(pdu);
        let encoded = encode_to_vec(&apdu);
        let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
        assert_eq!(apdu, decoded);
    }

    #[test]
    fn error_pdu_truncated_error_class() {
        // Error PDU with invoke_id and service choice but error class tag truncated
        // type=5<<4=0x50, invoke=1, service=12, then truncated tag
        assert!(decode_apdu(Bytes::from_static(&[0x50, 0x01, 0x0C])).is_err());
    }

    #[test]
    fn error_pdu_truncated_error_code() {
        // Error PDU with error class but error code tag truncated
        // type=0x50, invoke=1, service=12, error_class(enum 0, 1byte)=0x91 0x00, then truncated
        let mut buf = BytesMut::with_capacity(16);
        buf.put_u8(0x50); // Error PDU
        buf.put_u8(1); // invoke_id
        buf.put_u8(0x0C); // service_choice (ReadProperty)
        primitives::encode_app_enumerated(&mut buf, 2); // error_class = PROPERTY
        assert!(decode_apdu(buf.freeze()).is_err());
    }

    #[test]
    fn unconfirmed_request_empty_service_data() {
        let pdu = UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::WHO_IS,
            service_request: Bytes::new(),
        };
        let apdu = Apdu::UnconfirmedRequest(pdu);
        let encoded = encode_to_vec(&apdu);
        let decoded = decode_apdu(Bytes::from(encoded)).unwrap();
        assert_eq!(apdu, decoded);
    }
}
