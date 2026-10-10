//! Typed failures emitted by the frame codec.

use core::fmt;

/// A frame could not be encoded or decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// The input ended before the requested bytes were available.
    TooShort {
        /// Minimum input length for the attempted operation.
        needed: usize,
        /// Input length supplied by the caller.
        available: usize,
    },
    /// The two preamble octets were not `0x55, 0xFF`.
    InvalidPreamble {
        /// First received preamble octet.
        first: u8,
        /// Second received preamble octet.
        second: u8,
    },
    /// The header CRC did not match.
    HeaderCrcMismatch,
    /// A raw type in the reserved COBS/CRC-32K range was received.
    UnsupportedCobsFrame {
        /// Unsupported on-wire frame-type value.
        raw_type: u8,
    },
    /// The source address is not a valid MS/TP master address.
    InvalidSource {
        /// Invalid on-wire source address.
        source: u8,
    },
    /// The data length is above the standard-frame limit.
    Oversize {
        /// Declared payload length.
        length: usize,
        /// Largest supported standard-frame payload.
        maximum: usize,
    },
    /// An encoder was asked to write an unsupported COBS frame type.
    EncodeUnsupportedCobsFrame {
        /// Unsupported frame-type value requested by the caller.
        raw_type: u8,
    },
    /// An encoder was asked to write an oversized payload.
    EncodeOversize {
        /// Payload length requested by the caller.
        length: usize,
        /// Largest supported standard-frame payload.
        maximum: usize,
    },
    /// The header's declared data length disagrees with the borrowed payload.
    HeaderLengthMismatch {
        /// Payload length stored in the header.
        declared: usize,
        /// Borrowed payload length supplied for encoding.
        actual: usize,
    },
    /// The frame ended before its data and CRC were complete.
    Truncated {
        /// Payload length declared by the frame header.
        data_length: usize,
        /// Bytes available after the header.
        available: usize,
    },
    /// The data CRC did not match.
    DataCrcMismatch {
        /// Payload length used to locate the failed CRC.
        data_length: usize,
    },
    /// The destination storage could not hold the encoded frame.
    BufferExhausted {
        /// Capacity required to complete the operation.
        needed: usize,
        /// Capacity supplied by the caller.
        available: usize,
    },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { needed, available } => {
                write!(f, "frame too short: need {needed}, have {available}")
            }
            Self::InvalidPreamble { first, second } => {
                write!(f, "invalid preamble {first:#04X} {second:#04X}")
            }
            Self::HeaderCrcMismatch => f.write_str("header CRC mismatch"),
            Self::UnsupportedCobsFrame { raw_type } => {
                write!(f, "unsupported COBS frame type {raw_type}")
            }
            Self::EncodeUnsupportedCobsFrame { raw_type } => {
                write!(f, "unsupported COBS frame type {raw_type}")
            }
            Self::InvalidSource { source } => write!(f, "invalid source address {source:#04X}"),
            Self::Oversize { length, maximum } => {
                write!(f, "data length {length} exceeds maximum {maximum}")
            }
            Self::EncodeOversize { length, maximum } => {
                write!(f, "data length {length} exceeds maximum {maximum}")
            }
            Self::HeaderLengthMismatch { declared, actual } => {
                write!(
                    f,
                    "header length {declared} does not match payload length {actual}"
                )
            }
            Self::Truncated {
                data_length,
                available,
            } => write!(
                f,
                "truncated data length {data_length}, available {available}"
            ),
            Self::DataCrcMismatch { .. } => f.write_str("data CRC mismatch"),
            Self::BufferExhausted { needed, available } => {
                write!(f, "buffer exhausted: need {needed}, have {available}")
            }
        }
    }
}

#[cfg(feature = "alloc")]
impl From<FrameError> for bacnet_types::error::Error {
    fn from(error: FrameError) -> Self {
        use alloc::format;
        match error {
            FrameError::TooShort { .. } => Self::decoding(0, "MS/TP frame too short"),
            FrameError::InvalidPreamble { first, second } => Self::decoding(
                0,
                format!("MS/TP expected preamble 0x55 0xFF, got 0x{first:02X} 0x{second:02X}"),
            ),
            FrameError::HeaderCrcMismatch => Self::decoding(7, "MS/TP header CRC mismatch"),
            FrameError::UnsupportedCobsFrame { raw_type } => Self::decoding(
                2,
                format!("MS/TP COBS-encoded frame type {raw_type} is unsupported"),
            ),
            FrameError::EncodeUnsupportedCobsFrame { raw_type } => Self::Encoding(format!(
                "MS/TP COBS-encoded frame type {raw_type} is unsupported"
            )),
            FrameError::InvalidSource { source: 0xFF } => {
                Self::decoding(4, "MS/TP source address cannot be broadcast (0xFF)")
            }
            FrameError::InvalidSource { source } => Self::decoding(
                4,
                format!("MS/TP source address 0x{source:02X} exceeds MAX_MASTER (127)"),
            ),
            FrameError::Oversize { length, maximum } => Self::decoding(
                5,
                format!("MS/TP data length {length} exceeds maximum {maximum}"),
            ),
            FrameError::EncodeOversize { length, maximum } => Self::Encoding(format!(
                "MS/TP data length {length} exceeds maximum {maximum}"
            )),
            FrameError::HeaderLengthMismatch { declared, actual } => Self::Encoding(format!(
                "MS/TP header data length {declared} does not match payload length {actual}"
            )),
            FrameError::Truncated {
                data_length,
                available,
            } => Self::decoding(
                8,
                format!(
                    "MS/TP frame truncated: need {} bytes for data+CRC, have {available}",
                    data_length + 2
                ),
            ),
            FrameError::DataCrcMismatch { data_length } => {
                Self::decoding(8 + data_length, "MS/TP data CRC mismatch")
            }
            FrameError::BufferExhausted { needed, available } => {
                Self::buffer_too_short(needed, available)
            }
        }
    }
}
