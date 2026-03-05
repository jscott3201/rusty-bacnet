//! MS/TP frame encode/decode per ASHRAE 135-2020 Clause 9.
//!
//! Wire format:
//! ```text
//! [0x55] [0xFF] [frame_type] [dest] [src] [len_hi] [len_lo] [header_crc] [data...] [data_crc_lo] [data_crc_hi]
//! ```
//!
//! - Preamble bytes 0x55, 0xFF precede every frame.
//! - Header CRC covers frame_type through length (5 bytes).
//! - Data CRC is present only when data_length > 0.

use bacnet_types::error::Error;
use bytes::{BufMut, Bytes, BytesMut};

/// MS/TP preamble bytes.
pub const PREAMBLE: [u8; 2] = [0x55, 0xFF];

/// Header length after preamble: frame_type(1) + dest(1) + src(1) + length(2) + header_crc(1).
pub const HEADER_LENGTH: usize = 6;

/// Maximum NPDU data length per MS/TP extended frame (Clause 9.2).
/// Standard frames are limited to MAX_STANDARD_MPDU_DATA (501 bytes).
pub const MAX_MPDU_DATA: usize = 1497;

/// Maximum NPDU data length per standard MS/TP frame (Clause 9.1).
/// Legacy devices only support this smaller limit.
pub const MAX_STANDARD_MPDU_DATA: usize = 501;

/// Broadcast MAC address.
pub const BROADCAST_MAC: u8 = 0xFF;

/// Maximum master station address (Clause 9).
pub const MAX_MASTER: u8 = 127;

/// MS/TP frame types (Clause 9.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    /// Token — passed between masters to grant bus access.
    Token = 0x00,
    /// Poll For Master — discover successor stations.
    PollForMaster = 0x01,
    /// Reply To Poll For Master — response to PFM.
    ReplyToPollForMaster = 0x02,
    /// Test Request — data loopback test.
    TestRequest = 0x03,
    /// Test Response — reply to test request.
    TestResponse = 0x04,
    /// BACnet Data Expecting Reply — confirmed service request.
    BACnetDataExpectingReply = 0x05,
    /// BACnet Data Not Expecting Reply — unconfirmed or response.
    BACnetDataNotExpectingReply = 0x06,
    /// Reply Postponed — server needs more time.
    ReplyPostponed = 0x07,
    /// Unknown frame type.
    Unknown(u8),
}

impl FrameType {
    pub fn from_raw(val: u8) -> Self {
        match val {
            0x00 => Self::Token,
            0x01 => Self::PollForMaster,
            0x02 => Self::ReplyToPollForMaster,
            0x03 => Self::TestRequest,
            0x04 => Self::TestResponse,
            0x05 => Self::BACnetDataExpectingReply,
            0x06 => Self::BACnetDataNotExpectingReply,
            0x07 => Self::ReplyPostponed,
            v => Self::Unknown(v),
        }
    }

    pub fn to_raw(self) -> u8 {
        match self {
            Self::Token => 0x00,
            Self::PollForMaster => 0x01,
            Self::ReplyToPollForMaster => 0x02,
            Self::TestRequest => 0x03,
            Self::TestResponse => 0x04,
            Self::BACnetDataExpectingReply => 0x05,
            Self::BACnetDataNotExpectingReply => 0x06,
            Self::ReplyPostponed => 0x07,
            Self::Unknown(v) => v,
        }
    }

    /// True if this frame type carries NPDU data.
    pub fn has_data(self) -> bool {
        matches!(
            self,
            Self::TestRequest
                | Self::TestResponse
                | Self::BACnetDataExpectingReply
                | Self::BACnetDataNotExpectingReply
        )
    }
}

/// A decoded MS/TP frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MstpFrame {
    pub frame_type: FrameType,
    pub destination: u8,
    pub source: u8,
    /// NPDU/data payload (empty for Token, PFM, ReplyToPFM, ReplyPostponed).
    pub data: Bytes,
}

// ---------------------------------------------------------------------------
// CRC-8 (Header CRC) — Clause 9.5.2
// ---------------------------------------------------------------------------
// Polynomial: x^8 + x^2 + x + 1 (generator byte 0x07, reflected 0xE0)
// Calculated per byte using the algorithm from the spec.

/// CRC-8 lookup table per Clause 9 Annex G.
const CRC8_TABLE: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u8;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xE0;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// Compute CRC-8 over the given data. Initial value 0xFF, result inverted.
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0xFF;
    for &b in data {
        crc = CRC8_TABLE[(crc ^ b) as usize];
    }
    !crc
}

/// Verify CRC-8: recomputes CRC over data (excluding last byte) and compares
/// to the stored CRC byte.
pub fn crc8_valid(data_with_crc: &[u8]) -> bool {
    if data_with_crc.is_empty() {
        return false;
    }
    let (data, crc_byte) = data_with_crc.split_at(data_with_crc.len() - 1);
    crc8(data) == crc_byte[0]
}

// ---------------------------------------------------------------------------
// CRC-16 (Data CRC) — Clause 9.5.3
// ---------------------------------------------------------------------------
// Polynomial: x^16 + x^15 + x^2 + 1

/// CRC-16 lookup table per Clause 9 Annex G.
const CRC16_TABLE: [u16; 256] = {
    let mut table = [0u16; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u16;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// Compute CRC-16 over the given data. Initial value 0xFFFF, result inverted.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc = (crc >> 8) ^ CRC16_TABLE[((crc ^ b as u16) & 0xFF) as usize];
    }
    !crc
}

/// Verify CRC-16: recomputes CRC over data (excluding last 2 bytes) and compares
/// to the stored CRC (little-endian).
pub fn crc16_valid(data_with_crc: &[u8]) -> bool {
    if data_with_crc.len() < 3 {
        return false;
    }
    let (data, crc_bytes) = data_with_crc.split_at(data_with_crc.len() - 2);
    let stored = (crc_bytes[0] as u16) | ((crc_bytes[1] as u16) << 8);
    crc16(data) == stored
}

// ---------------------------------------------------------------------------
// Frame encode/decode
// ---------------------------------------------------------------------------

/// Encode an MS/TP frame into the buffer.
///
/// Writes preamble + header + header CRC, then data + data CRC if data is non-empty.
pub fn encode_frame(
    buf: &mut BytesMut,
    frame: &MstpFrame,
) -> Result<(), bacnet_types::error::Error> {
    let data_len = frame.data.len();
    if data_len > MAX_MPDU_DATA {
        return Err(bacnet_types::error::Error::Encoding(format!(
            "MS/TP data length {} exceeds maximum {}",
            data_len, MAX_MPDU_DATA
        )));
    }

    // Reserve space
    let total = 2 + HEADER_LENGTH + data_len + if data_len > 0 { 2 } else { 0 };
    buf.reserve(total);

    // Preamble
    buf.put_slice(&PREAMBLE);

    // Header: frame_type, dest, src, length(2)
    let header = [
        frame.frame_type.to_raw(),
        frame.destination,
        frame.source,
        (data_len >> 8) as u8,
        (data_len & 0xFF) as u8,
    ];
    buf.put_slice(&header);

    // Header CRC (covers the 5 header bytes)
    buf.put_u8(crc8(&header));

    // Data + Data CRC
    if !frame.data.is_empty() {
        buf.put_slice(&frame.data);
        let dcrc = crc16(&frame.data);
        // Data CRC is little-endian
        buf.put_u8(dcrc as u8);
        buf.put_u8((dcrc >> 8) as u8);
    }
    Ok(())
}

/// Decode an MS/TP frame from raw bytes (starting at the preamble).
///
/// Returns the decoded frame and the number of bytes consumed.
pub fn decode_frame(data: &[u8]) -> Result<(MstpFrame, usize), Error> {
    // Minimum: preamble(2) + header(5) + header_crc(1) = 8
    if data.len() < 2 + HEADER_LENGTH {
        return Err(Error::decoding(0, "MS/TP frame too short"));
    }

    // Verify preamble
    if data[0] != PREAMBLE[0] || data[1] != PREAMBLE[1] {
        return Err(Error::decoding(
            0,
            format!(
                "MS/TP expected preamble 0x55 0xFF, got 0x{:02X} 0x{:02X}",
                data[0], data[1]
            ),
        ));
    }

    // Verify header CRC (covers bytes 2..7, CRC at byte 7)
    if !crc8_valid(&data[2..8]) {
        return Err(Error::decoding(7, "MS/TP header CRC mismatch"));
    }

    let frame_type = FrameType::from_raw(data[2]);
    let destination = data[3];
    let source = data[4];

    // Source address must be a valid master station (0..=MAX_MASTER).
    // BROADCAST_MAC (0xFF) is not valid as a source per Clause 9.
    if source > MAX_MASTER && source != BROADCAST_MAC {
        return Err(Error::decoding(
            4,
            format!(
                "MS/TP source address 0x{:02X} exceeds MAX_MASTER ({})",
                source, MAX_MASTER
            ),
        ));
    }
    if source == BROADCAST_MAC {
        return Err(Error::decoding(
            4,
            "MS/TP source address cannot be broadcast (0xFF)",
        ));
    }

    let data_length = ((data[5] as usize) << 8) | (data[6] as usize);

    if data_length > MAX_MPDU_DATA {
        return Err(Error::decoding(
            5,
            format!(
                "MS/TP data length {} exceeds maximum {}",
                data_length, MAX_MPDU_DATA
            ),
        ));
    }

    let mut consumed = 2 + HEADER_LENGTH; // 8 bytes for preamble + header + header CRC

    let frame_data = if data_length > 0 {
        // Need data + 2-byte CRC
        let needed = consumed + data_length + 2;
        if data.len() < needed {
            return Err(Error::decoding(
                consumed,
                format!(
                    "MS/TP frame truncated: need {} bytes for data+CRC, have {}",
                    data_length + 2,
                    data.len() - consumed
                ),
            ));
        }

        // Verify data CRC (covers data bytes + 2 CRC bytes)
        if !crc16_valid(&data[consumed..consumed + data_length + 2]) {
            return Err(Error::decoding(
                consumed + data_length,
                "MS/TP data CRC mismatch",
            ));
        }

        let payload = Bytes::copy_from_slice(&data[consumed..consumed + data_length]);
        consumed += data_length + 2;
        payload
    } else {
        Bytes::new()
    };

    Ok((
        MstpFrame {
            frame_type,
            destination,
            source,
            data: frame_data,
        },
        consumed,
    ))
}

/// Scan for a frame preamble in raw bytes. Returns the offset of the first 0x55 0xFF sequence.
pub fn find_preamble(data: &[u8]) -> Option<usize> {
    data.windows(2).position(|w| w[0] == 0x55 && w[1] == 0xFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // CRC tests
    // -----------------------------------------------------------------------

    #[test]
    fn crc8_known_value() {
        // Token frame header: type=0x00, dest=0x01, src=0x00, len=0x0000
        let header = [0x00, 0x01, 0x00, 0x00, 0x00];
        let crc = crc8(&header);
        // Verify by appending CRC and checking validity
        let mut with_crc = header.to_vec();
        with_crc.push(crc);
        assert!(crc8_valid(&with_crc));
    }

    #[test]
    fn crc8_validate_round_trip() {
        let data = [0x05, 0xFF, 0x03, 0x00, 0x0C];
        let crc = crc8(&data);
        let mut with_crc = data.to_vec();
        with_crc.push(crc);
        assert!(crc8_valid(&with_crc));
    }

    #[test]
    fn crc8_invalid_detects_corruption() {
        let data = [0x05, 0xFF, 0x03, 0x00, 0x0C];
        let crc = crc8(&data);
        let mut with_crc = data.to_vec();
        with_crc.push(crc ^ 0x01); // corrupt
        assert!(!crc8_valid(&with_crc));
    }

    #[test]
    fn crc16_known_value() {
        let data = [0x01, 0x00, 0x10, 0x02];
        let crc = crc16(&data);
        let mut with_crc = data.to_vec();
        with_crc.push(crc as u8);
        with_crc.push((crc >> 8) as u8);
        assert!(crc16_valid(&with_crc));
    }

    #[test]
    fn crc16_validate_round_trip() {
        let data = vec![0xAA; 100];
        let crc = crc16(&data);
        let mut with_crc = data;
        with_crc.push(crc as u8);
        with_crc.push((crc >> 8) as u8);
        assert!(crc16_valid(&with_crc));
    }

    #[test]
    fn crc16_invalid_detects_corruption() {
        let data = [0x01, 0x02, 0x03];
        let crc = crc16(&data);
        let mut with_crc = data.to_vec();
        with_crc.push(crc as u8);
        with_crc.push((crc >> 8) as u8 ^ 0x01); // corrupt
        assert!(!crc16_valid(&with_crc));
    }

    // -----------------------------------------------------------------------
    // Frame encode/decode tests
    // -----------------------------------------------------------------------

    #[test]
    fn token_frame_round_trip() {
        let frame = MstpFrame {
            frame_type: FrameType::Token,
            destination: 1,
            source: 0,
            data: Bytes::new(),
        };
        let mut buf = BytesMut::new();
        encode_frame(&mut buf, &frame).unwrap();

        // Token has no data, so: preamble(2) + header(5) + crc(1) = 8
        assert_eq!(buf.len(), 8);
        assert_eq!(&buf[..2], &PREAMBLE);

        let (decoded, consumed) = decode_frame(&buf).unwrap();
        assert_eq!(consumed, 8);
        assert_eq!(decoded, frame);
    }

    #[test]
    fn poll_for_master_round_trip() {
        let frame = MstpFrame {
            frame_type: FrameType::PollForMaster,
            destination: 42,
            source: 0,
            data: Bytes::new(),
        };
        let mut buf = BytesMut::new();
        encode_frame(&mut buf, &frame).unwrap();

        let (decoded, _) = decode_frame(&buf).unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn data_expecting_reply_round_trip() {
        let npdu = vec![0x01, 0x00, 0x10, 0x02, 0x03, 0x04, 0x05];
        let frame = MstpFrame {
            frame_type: FrameType::BACnetDataExpectingReply,
            destination: 5,
            source: 0,
            data: Bytes::from(npdu.clone()),
        };
        let mut buf = BytesMut::new();
        encode_frame(&mut buf, &frame).unwrap();

        // preamble(2) + header(5) + hcrc(1) + data(7) + dcrc(2) = 17
        assert_eq!(buf.len(), 17);

        let (decoded, consumed) = decode_frame(&buf).unwrap();
        assert_eq!(consumed, 17);
        assert_eq!(decoded.frame_type, FrameType::BACnetDataExpectingReply);
        assert_eq!(decoded.destination, 5);
        assert_eq!(decoded.source, 0);
        assert_eq!(decoded.data, npdu);
    }

    #[test]
    fn data_not_expecting_reply_round_trip() {
        let npdu = vec![0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];
        let frame = MstpFrame {
            frame_type: FrameType::BACnetDataNotExpectingReply,
            destination: BROADCAST_MAC,
            source: 3,
            data: Bytes::from(npdu.clone()),
        };
        let mut buf = BytesMut::new();
        encode_frame(&mut buf, &frame).unwrap();

        let (decoded, _) = decode_frame(&buf).unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn broadcast_destination() {
        let frame = MstpFrame {
            frame_type: FrameType::BACnetDataNotExpectingReply,
            destination: BROADCAST_MAC,
            source: 10,
            data: Bytes::from_static(&[0x01, 0x00]),
        };
        let mut buf = BytesMut::new();
        encode_frame(&mut buf, &frame).unwrap();

        let (decoded, _) = decode_frame(&buf).unwrap();
        assert_eq!(decoded.destination, BROADCAST_MAC);
    }

    #[test]
    fn reply_to_poll_for_master_round_trip() {
        let frame = MstpFrame {
            frame_type: FrameType::ReplyToPollForMaster,
            destination: 0,
            source: 42,
            data: Bytes::new(),
        };
        let mut buf = BytesMut::new();
        encode_frame(&mut buf, &frame).unwrap();

        let (decoded, _) = decode_frame(&buf).unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn test_request_with_data_round_trip() {
        let test_data = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let frame = MstpFrame {
            frame_type: FrameType::TestRequest,
            destination: 5,
            source: 0,
            data: Bytes::from(test_data.clone()),
        };
        let mut buf = BytesMut::new();
        encode_frame(&mut buf, &frame).unwrap();

        let (decoded, _) = decode_frame(&buf).unwrap();
        assert_eq!(decoded.data, test_data);
    }

    #[test]
    fn decode_too_short() {
        assert!(decode_frame(&[0x55, 0xFF, 0x00]).is_err());
    }

    #[test]
    fn decode_bad_preamble() {
        let data = [0x00, 0xFF, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00];
        assert!(decode_frame(&data).is_err());
    }

    #[test]
    fn decode_bad_header_crc() {
        let mut buf = BytesMut::new();
        let frame = MstpFrame {
            frame_type: FrameType::Token,
            destination: 1,
            source: 0,
            data: Bytes::new(),
        };
        encode_frame(&mut buf, &frame).unwrap();
        // Corrupt header CRC (byte 7)
        buf[7] ^= 0xFF;
        assert!(decode_frame(&buf).is_err());
    }

    #[test]
    fn decode_bad_data_crc() {
        let mut buf = BytesMut::new();
        let frame = MstpFrame {
            frame_type: FrameType::BACnetDataNotExpectingReply,
            destination: 1,
            source: 0,
            data: Bytes::from_static(&[0x01, 0x00]),
        };
        encode_frame(&mut buf, &frame).unwrap();
        // Corrupt last byte (data CRC high)
        let last = buf.len() - 1;
        buf[last] ^= 0xFF;
        assert!(decode_frame(&buf).is_err());
    }

    #[test]
    fn decode_truncated_data() {
        let mut buf = BytesMut::new();
        let frame = MstpFrame {
            frame_type: FrameType::BACnetDataExpectingReply,
            destination: 5,
            source: 0,
            data: Bytes::from_static(&[0x01, 0x02, 0x03, 0x04]),
        };
        encode_frame(&mut buf, &frame).unwrap();
        // Truncate: remove data CRC
        buf.truncate(buf.len() - 2);
        assert!(decode_frame(&buf).is_err());
    }

    #[test]
    fn find_preamble_at_start() {
        let data = [0x55, 0xFF, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(find_preamble(&data), Some(0));
    }

    #[test]
    fn find_preamble_with_garbage() {
        let data = [0x00, 0x00, 0x12, 0x55, 0xFF, 0x00, 0x01, 0x00];
        assert_eq!(find_preamble(&data), Some(3));
    }

    #[test]
    fn find_preamble_none() {
        let data = [0x00, 0x55, 0x00, 0xFF, 0x01];
        assert_eq!(find_preamble(&data), None);
    }

    #[test]
    fn frame_type_round_trip() {
        for raw in 0..=0x07 {
            let ft = FrameType::from_raw(raw);
            assert_eq!(ft.to_raw(), raw);
        }
        // Unknown type
        let ft = FrameType::from_raw(0x42);
        assert_eq!(ft.to_raw(), 0x42);
        assert_eq!(ft, FrameType::Unknown(0x42));
    }

    #[test]
    fn frame_type_has_data() {
        assert!(!FrameType::Token.has_data());
        assert!(!FrameType::PollForMaster.has_data());
        assert!(!FrameType::ReplyToPollForMaster.has_data());
        assert!(FrameType::TestRequest.has_data());
        assert!(FrameType::TestResponse.has_data());
        assert!(FrameType::BACnetDataExpectingReply.has_data());
        assert!(FrameType::BACnetDataNotExpectingReply.has_data());
        assert!(!FrameType::ReplyPostponed.has_data());
    }

    #[test]
    fn large_data_frame() {
        // Near-maximum data size
        let npdu = vec![0xAA; 1024];
        let frame = MstpFrame {
            frame_type: FrameType::BACnetDataNotExpectingReply,
            destination: BROADCAST_MAC,
            source: 0,
            data: Bytes::from(npdu.clone()),
        };
        let mut buf = BytesMut::new();
        encode_frame(&mut buf, &frame).unwrap();

        let (decoded, _) = decode_frame(&buf).unwrap();
        assert_eq!(decoded.data, npdu);
    }

    #[test]
    fn encode_oversized_data_returns_error() {
        let frame = MstpFrame {
            frame_type: FrameType::BACnetDataNotExpectingReply,
            destination: 1,
            source: 0,
            data: Bytes::from_static(&[0xAA; MAX_MPDU_DATA + 1]),
        };
        let mut buf = BytesMut::new();
        assert!(encode_frame(&mut buf, &frame).is_err());
    }

    #[test]
    fn decode_rejects_source_above_max_master() {
        // Encode a valid frame then patch the source to 128 (above MAX_MASTER=127)
        let frame = MstpFrame {
            frame_type: FrameType::Token,
            destination: 1,
            source: 0,
            data: Bytes::new(),
        };
        let mut buf = BytesMut::new();
        encode_frame(&mut buf, &frame).unwrap();

        // Patch source byte (offset 4) to 128
        buf[4] = 128;
        // Recompute header CRC (bytes 2..7, CRC at byte 7)
        let header_crc = crc8(&buf[2..7]);
        buf[7] = header_crc;

        assert!(decode_frame(&buf).is_err());
    }

    #[test]
    fn decode_rejects_broadcast_source() {
        // Encode a valid frame then patch the source to BROADCAST_MAC (0xFF)
        let frame = MstpFrame {
            frame_type: FrameType::Token,
            destination: 1,
            source: 0,
            data: Bytes::new(),
        };
        let mut buf = BytesMut::new();
        encode_frame(&mut buf, &frame).unwrap();

        // Patch source byte to 0xFF
        buf[4] = BROADCAST_MAC;
        // Recompute header CRC
        let header_crc = crc8(&buf[2..7]);
        buf[7] = header_crc;

        assert!(decode_frame(&buf).is_err());
    }

    #[test]
    fn decode_accepts_max_master_source() {
        // Source = MAX_MASTER (127) should be valid
        let frame = MstpFrame {
            frame_type: FrameType::Token,
            destination: 1,
            source: MAX_MASTER,
            data: Bytes::new(),
        };
        let mut buf = BytesMut::new();
        encode_frame(&mut buf, &frame).unwrap();

        let (decoded, _) = decode_frame(&buf).unwrap();
        assert_eq!(decoded.source, MAX_MASTER);
    }
}
