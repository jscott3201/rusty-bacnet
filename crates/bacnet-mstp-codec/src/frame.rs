//! Frame headers and one-shot encode/decode operations.

use crate::{
    crc16, crc16_valid, crc8, crc8_valid, FrameError, BROADCAST_MAC, HEADER_LENGTH, MAX_MASTER,
    MAX_STANDARD_MPDU_DATA, PREAMBLE,
};

/// MS/TP frame type octet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    /// Token frame.
    Token = 0x00,
    /// Poll For Master frame.
    PollForMaster = 0x01,
    /// Reply To Poll For Master frame.
    ReplyToPollForMaster = 0x02,
    /// Test Request frame.
    TestRequest = 0x03,
    /// Test Response frame.
    TestResponse = 0x04,
    /// BACnet data expecting a reply.
    BACnetDataExpectingReply = 0x05,
    /// BACnet data not expecting a reply.
    BACnetDataNotExpectingReply = 0x06,
    /// Reply postponed frame.
    ReplyPostponed = 0x07,
    /// A currently unassigned standard frame type.
    Unknown(u8),
}

impl FrameType {
    /// Convert a raw frame type octet to its typed representation.
    pub const fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Self::Token,
            1 => Self::PollForMaster,
            2 => Self::ReplyToPollForMaster,
            3 => Self::TestRequest,
            4 => Self::TestResponse,
            5 => Self::BACnetDataExpectingReply,
            6 => Self::BACnetDataNotExpectingReply,
            7 => Self::ReplyPostponed,
            value => Self::Unknown(value),
        }
    }

    /// Return this frame type's wire value.
    pub const fn to_raw(self) -> u8 {
        match self {
            Self::Token => 0,
            Self::PollForMaster => 1,
            Self::ReplyToPollForMaster => 2,
            Self::TestRequest => 3,
            Self::TestResponse => 4,
            Self::BACnetDataExpectingReply => 5,
            Self::BACnetDataNotExpectingReply => 6,
            Self::ReplyPostponed => 7,
            Self::Unknown(value) => value,
        }
    }

    /// Whether this frame type carries a data payload.
    pub const fn has_data(self) -> bool {
        matches!(
            self,
            Self::TestRequest
                | Self::TestResponse
                | Self::BACnetDataExpectingReply
                | Self::BACnetDataNotExpectingReply
        )
    }

    /// Whether this raw type belongs to the reserved COBS/CRC-32K range.
    pub const fn is_cobs(self) -> bool {
        let raw = self.to_raw();
        raw >= 0x20 && raw <= 0x7F
    }
}

/// The allocation-free header fields of an MS/TP frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// Frame type.
    pub frame_type: FrameType,
    /// Destination station address.
    pub destination: u8,
    /// Source station address.
    pub source: u8,
    /// Number of data bytes following the header.
    pub data_length: usize,
}

impl FrameHeader {
    /// Construct a header for a payload of `data_length` bytes.
    pub const fn new(
        frame_type: FrameType,
        destination: u8,
        source: u8,
        data_length: usize,
    ) -> Self {
        Self {
            frame_type,
            destination,
            source,
            data_length,
        }
    }
}

/// A decoded frame borrowing its data from the input or stream buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame<'a> {
    /// Header fields.
    pub header: FrameHeader,
    /// Borrowed NPDU or test payload.
    pub data: &'a [u8],
}

impl<'a> Frame<'a> {
    /// Construct a borrowed frame and derive its header length from `data`.
    pub const fn new(frame_type: FrameType, destination: u8, source: u8, data: &'a [u8]) -> Self {
        Self {
            header: FrameHeader::new(frame_type, destination, source, data.len()),
            data,
        }
    }

    /// Encode this frame into caller-owned storage.
    pub fn encode_into(self, output: &mut [u8]) -> Result<usize, FrameError> {
        encode_into(output, &self.header, self.data)
    }
}

/// Encode a frame into caller-owned storage, returning its wire length.
pub fn encode_into(
    output: &mut [u8],
    header: &FrameHeader,
    data: &[u8],
) -> Result<usize, FrameError> {
    let data_length = data.len();
    let raw_type = header.frame_type.to_raw();
    if (0x20..=0x7F).contains(&raw_type) {
        return Err(FrameError::EncodeUnsupportedCobsFrame { raw_type });
    }
    if data_length > MAX_STANDARD_MPDU_DATA {
        return Err(FrameError::EncodeOversize {
            length: data_length,
            maximum: MAX_STANDARD_MPDU_DATA,
        });
    }
    if header.data_length != data_length {
        return Err(FrameError::HeaderLengthMismatch {
            declared: header.data_length,
            actual: data_length,
        });
    }
    let total = PREAMBLE.len() + HEADER_LENGTH + data_length + if data_length == 0 { 0 } else { 2 };
    if output.len() < total {
        return Err(FrameError::BufferExhausted {
            needed: total,
            available: output.len(),
        });
    }
    output[..2].copy_from_slice(&PREAMBLE);
    let fields = [
        raw_type,
        header.destination,
        header.source,
        (data_length >> 8) as u8,
        data_length as u8,
    ];
    output[2..7].copy_from_slice(&fields);
    output[7] = crc8(&fields);
    if data_length != 0 {
        output[8..8 + data_length].copy_from_slice(data);
        let crc = crc16(data).to_le_bytes();
        output[8 + data_length..10 + data_length].copy_from_slice(&crc);
    }
    Ok(total)
}

/// Decode one frame from bytes beginning with its preamble.
pub fn decode_frame(data: &[u8]) -> Result<(Frame<'_>, usize), FrameError> {
    if data.len() < 2 + HEADER_LENGTH {
        return Err(FrameError::TooShort {
            needed: 2 + HEADER_LENGTH,
            available: data.len(),
        });
    }
    if data[0] != PREAMBLE[0] || data[1] != PREAMBLE[1] {
        return Err(FrameError::InvalidPreamble {
            first: data[0],
            second: data[1],
        });
    }
    if !crc8_valid(&data[2..8]) {
        return Err(FrameError::HeaderCrcMismatch);
    }
    let raw_type = data[2];
    if (0x20..=0x7F).contains(&raw_type) {
        return Err(FrameError::UnsupportedCobsFrame { raw_type });
    }
    let source = data[4];
    if source > MAX_MASTER {
        return Err(FrameError::InvalidSource { source });
    }
    let data_length = u16::from_be_bytes([data[5], data[6]]) as usize;
    if data_length > MAX_STANDARD_MPDU_DATA {
        return Err(FrameError::Oversize {
            length: data_length,
            maximum: MAX_STANDARD_MPDU_DATA,
        });
    }
    let body_start = 2 + HEADER_LENGTH;
    let total = body_start + if data_length == 0 { 0 } else { data_length + 2 };
    if data.len() < total {
        return Err(FrameError::Truncated {
            data_length,
            available: data.len() - body_start,
        });
    }
    if data_length != 0 && !crc16_valid(&data[body_start..total]) {
        return Err(FrameError::DataCrcMismatch { data_length });
    }
    let frame = Frame {
        header: FrameHeader::new(FrameType::from_raw(raw_type), data[3], source, data_length),
        data: &data[body_start..body_start + data_length],
    };
    Ok((frame, total))
}

/// Return the first offset containing the MS/TP preamble.
pub fn find_preamble(data: &[u8]) -> Option<usize> {
    data.windows(2).position(|pair| pair == PREAMBLE)
}

/// Keep a trailing first preamble octet while discarding other bytes.
pub fn retain_lone_preamble_byte(buffer: &mut [u8], length: &mut usize) {
    if *length != 0 && buffer[*length - 1] == PREAMBLE[0] {
        buffer[0] = PREAMBLE[0];
        *length = 1;
    } else {
        *length = 0;
    }
}

/// Return whether a raw frame type is reserved for COBS/CRC-32K framing.
pub const fn is_cobs_raw(raw: u8) -> bool {
    raw >= 0x20 && raw <= 0x7F
}

/// The broadcast source value is never valid; this helper documents the rule.
pub const fn is_valid_source(source: u8) -> bool {
    source <= MAX_MASTER && source != BROADCAST_MAC
}
