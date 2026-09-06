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

/// Maximum NPDU data length per standard MS/TP frame.
pub const MAX_STANDARD_MPDU_DATA: usize = 501;

/// Maximum NPDU data length supported by this non-encoded MS/TP codec.
///
/// This remains as the public compatibility name for
/// [`MAX_STANDARD_MPDU_DATA`]. COBS-encoded frame types 32..=127 are not supported.
pub const MAX_MPDU_DATA: usize = MAX_STANDARD_MPDU_DATA;

const DATA_CRC_LENGTH: usize = 2;

/// Maximum wire length supported by this standard-frame codec.
pub(crate) const MAX_STANDARD_FRAME_LENGTH: usize =
    PREAMBLE.len() + HEADER_LENGTH + MAX_STANDARD_MPDU_DATA + DATA_CRC_LENGTH;

/// Broadcast MAC address.
pub const BROADCAST_MAC: u8 = 0xFF;

/// Maximum master station address.
pub const MAX_MASTER: u8 = 127;

/// MS/TP frame types.
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

fn is_unsupported_cobs_frame_type(raw: u8) -> bool {
    (0x20..=0x7F).contains(&raw)
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
// CRC-8 (Header CRC)
// ---------------------------------------------------------------------------

/// BACnet Clause 9.6 Frame Header CRC — reflected poly `G(x)=x^8+x^7+1` → `0x81`.
/// (Prior incorrect table used `0xE0`, which passes self-round-trip but rejects live trunk frames.)
const CRC8_TABLE: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u8;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0x81;
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

/// Good header-CRC receiver residual (Clause 9.6), including the CRC octet.
pub const HEADER_CRC_RESIDUAL: u8 = 0x55;

/// Compute CRC-8 over the given data. Initial value 0xFF, result ones-complemented.
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0xFF;
    for &b in data {
        crc = CRC8_TABLE[(crc ^ b) as usize];
    }
    !crc
}

/// Running header CRC including the transmitted CRC octet (no final invert).
pub fn crc8_accumulate_all(data_with_crc: &[u8]) -> u8 {
    let mut crc: u8 = 0xFF;
    for &b in data_with_crc {
        crc = CRC8_TABLE[(crc ^ b) as usize];
    }
    crc
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
// CRC-16 (Data CRC)
// ---------------------------------------------------------------------------

/// BACnet Clause 9.6 Data CRC — CRC-16-CCITT reflected poly `0x8408`.
/// (Prior incorrect table used Modbus `0xA001`.)
const CRC16_TABLE: [u16; 256] = {
    let mut table = [0u16; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u16;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0x8408;
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

/// Good data-CRC receiver residual (Clause 9.6), including the CRC octets.
pub const DATA_CRC_RESIDUAL: u16 = 0xF0B8;

/// Compute CRC-16 over the given data. Initial value 0xFFFF, result ones-complemented.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc = (crc >> 8) ^ CRC16_TABLE[((crc ^ b as u16) & 0xFF) as usize];
    }
    !crc
}

/// Running data CRC including the transmitted CRC octets (no final invert).
pub fn crc16_accumulate_all(data_with_crc: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data_with_crc {
        crc = (crc >> 8) ^ CRC16_TABLE[((crc ^ b as u16) & 0xFF) as usize];
    }
    crc
}

/// Verify CRC-16: recomputes CRC over data (excluding last 2 bytes) and compares
/// to the stored CRC (little-endian; LS octet first on the wire).
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
    let frame_type = frame.frame_type.to_raw();
    if is_unsupported_cobs_frame_type(frame_type) {
        return Err(bacnet_types::error::Error::Encoding(format!(
            "MS/TP COBS-encoded frame type {frame_type} is unsupported"
        )));
    }
    if data_len > MAX_STANDARD_MPDU_DATA {
        return Err(bacnet_types::error::Error::Encoding(format!(
            "MS/TP data length {} exceeds maximum {}",
            data_len, MAX_STANDARD_MPDU_DATA
        )));
    }

    // Reserve space
    let total =
        PREAMBLE.len() + HEADER_LENGTH + data_len + if data_len > 0 { DATA_CRC_LENGTH } else { 0 };
    buf.reserve(total);

    // Preamble
    buf.put_slice(&PREAMBLE);

    // Header: frame_type, dest, src, length(2)
    let header = [
        frame_type,
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

/// Result of incremental/streaming MS/TP frame decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamDecode {
    /// A complete, validated frame was decoded.
    Complete { frame: MstpFrame, consumed: usize },
    /// More bytes are required before a decode decision can be made.
    NeedMore,
    /// The buffer contains invalid data; discard at least `discard` bytes and resync.
    Invalid { discard: usize },
}

/// Incrementally decode an MS/TP frame from a receive buffer (starting at the preamble).
///
/// Unlike [`decode_frame`], incomplete input returns [`StreamDecode::NeedMore`] instead of
/// an error. Real corruption (bad CRC, invalid header) returns [`StreamDecode::Invalid`].
pub fn decode_frame_stream(data: &[u8]) -> StreamDecode {
    if data.is_empty() {
        return StreamDecode::NeedMore;
    }

    if data[0] != PREAMBLE[0] {
        return StreamDecode::Invalid { discard: 1 };
    }
    if data.len() < 2 {
        return StreamDecode::NeedMore;
    }
    if data[1] != PREAMBLE[1] {
        return StreamDecode::Invalid { discard: 1 };
    }

    // Preamble(2) + header fields(5) + header_crc(1)
    if data.len() < 2 + HEADER_LENGTH {
        return StreamDecode::NeedMore;
    }

    if !crc8_valid(&data[2..8]) {
        return StreamDecode::Invalid { discard: 1 };
    }

    if is_unsupported_cobs_frame_type(data[2]) {
        return StreamDecode::Invalid { discard: 1 };
    }

    let frame_type = FrameType::from_raw(data[2]);
    let destination = data[3];
    let source = data[4];

    if source > MAX_MASTER && source != BROADCAST_MAC {
        return StreamDecode::Invalid { discard: 1 };
    }
    if source == BROADCAST_MAC {
        return StreamDecode::Invalid { discard: 1 };
    }

    let data_length = ((data[5] as usize) << 8) | (data[6] as usize);
    if data_length > MAX_STANDARD_MPDU_DATA {
        return StreamDecode::Invalid { discard: 1 };
    }

    let mut consumed = 2 + HEADER_LENGTH;

    if data_length > 0 {
        let needed = consumed + data_length + DATA_CRC_LENGTH;
        if data.len() < needed {
            return StreamDecode::NeedMore;
        }

        if !crc16_valid(&data[consumed..consumed + data_length + DATA_CRC_LENGTH]) {
            return StreamDecode::Invalid { discard: needed };
        }

        let payload = Bytes::copy_from_slice(&data[consumed..consumed + data_length]);
        consumed += data_length + DATA_CRC_LENGTH;

        StreamDecode::Complete {
            frame: MstpFrame {
                frame_type,
                destination,
                source,
                data: payload,
            },
            consumed,
        }
    } else {
        StreamDecode::Complete {
            frame: MstpFrame {
                frame_type,
                destination,
                source,
                data: Bytes::new(),
            },
            consumed,
        }
    }
}

/// When no full preamble is found, retain a trailing lone `0x55` for the next chunk.
pub fn retain_lone_preamble_byte(buf: &mut Vec<u8>) {
    if buf.last() == Some(&PREAMBLE[0]) {
        let lone = PREAMBLE[0];
        buf.clear();
        buf.push(lone);
    } else {
        buf.clear();
    }
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

    if is_unsupported_cobs_frame_type(data[2]) {
        return Err(Error::decoding(
            2,
            format!("MS/TP COBS-encoded frame type {} is unsupported", data[2]),
        ));
    }

    let frame_type = FrameType::from_raw(data[2]);
    let destination = data[3];
    let source = data[4];

    // Source address must be a valid master station (0..=MAX_MASTER).
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

    if data_length > MAX_STANDARD_MPDU_DATA {
        return Err(Error::decoding(
            5,
            format!(
                "MS/TP data length {} exceeds maximum {}",
                data_length, MAX_STANDARD_MPDU_DATA
            ),
        ));
    }

    let mut consumed = 2 + HEADER_LENGTH; // 8 bytes for preamble + header + header CRC

    let frame_data = if data_length > 0 {
        // Need data + 2-byte CRC
        let needed = consumed + data_length + DATA_CRC_LENGTH;
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
        if !crc16_valid(&data[consumed..consumed + data_length + DATA_CRC_LENGTH]) {
            return Err(Error::decoding(
                consumed + data_length,
                "MS/TP data CRC mismatch",
            ));
        }

        let payload = Bytes::copy_from_slice(&data[consumed..consumed + data_length]);
        consumed += data_length + DATA_CRC_LENGTH;
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
#[path = "mstp_frame_tests.rs"]
mod tests;
