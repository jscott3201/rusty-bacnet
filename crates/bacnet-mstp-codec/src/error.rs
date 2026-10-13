//! Typed failures emitted by the frame codec.

use core::fmt;

use crate::SinkFull;

/// A frame could not be encoded or decoded.
///
/// Encode and decode share the variants that mean the same thing in both
/// directions; the caller knows which one it asked for. Later releases may
/// add variants, so a match outside this crate needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameError {
    /// Decode: the input ended before the header was complete.
    TooShort {
        /// Octets needed for the preamble and header.
        needed: usize,
        /// Octets supplied.
        available: usize,
    },
    /// Decode: the input does not start with the preamble `0x55, 0xFF`.
    InvalidPreamble {
        /// First input octet.
        first: u8,
        /// Second input octet.
        second: u8,
    },
    /// Decode: the header CRC does not match.
    HeaderCrcMismatch,
    /// A reserved COBS-encoded frame type, 34-127, which the codec doesn't
    /// implement. The header passed every other check first, source and
    /// Length included, so it is a good header (Clause 9.5.8).
    UnsupportedCobsFrame {
        /// The frame type octet.
        raw_type: u8,
    },
    /// The source address is 255, the broadcast address, which no station may
    /// send from (Clause 9.3).
    BroadcastSource,
    /// A COBS-encoded frame's Length is outside
    /// [`MIN_COBS_LENGTH`](crate::MIN_COBS_LENGTH)..=[`MAX_COBS_LENGTH`](crate::MAX_COBS_LENGTH),
    /// so the header is bad (Clause 9.5.8). On encode, the data is empty or
    /// encodes to more than the largest Length allows.
    InvalidCobsLength {
        /// The Length declared by the header (decode), or the one the data
        /// would need (encode).
        length: usize,
    },
    /// Decode with [`decode_frame`](crate::decode_frame): a good extended
    /// frame header, whose COBS-encoded data can't be borrowed from the
    /// input. It says only "decode this in place": nothing after the header
    /// was checked, and
    /// [`decode_frame_in_place`](crate::decode_frame_in_place) gives the
    /// verdict on the rest of the frame.
    ExtendedFrame {
        /// The frame type octet, 32 or 33.
        raw_type: u8,
    },
    /// The data field of a standard (not COBS-encoded) frame is longer than
    /// one allows.
    Oversize {
        /// Data length declared by the header (decode) or supplied (encode).
        length: usize,
        /// Largest standard data field, [`crate::MAX_STANDARD_MPDU_DATA`].
        maximum: usize,
    },
    /// Encode: data was supplied for a frame type that never carries any (see
    /// [`crate::FrameType::allows_data`]).
    UnexpectedData {
        /// The frame type octet.
        raw_type: u8,
        /// Length of the supplied data.
        length: usize,
    },
    /// Encode: a proprietary frame (type 128-255) has fewer than the two data
    /// octets its vendor identifier takes (Clause 9.3.11).
    MissingVendorId {
        /// The frame type octet.
        raw_type: u8,
        /// Length of the supplied data.
        length: usize,
    },
    /// Decode: the input ended before the frame did: before a standard
    /// frame's data and data CRC, or an extended frame's encoded fields, were
    /// complete.
    Truncated {
        /// The Length declared by the header.
        length: usize,
        /// Octets supplied after the header.
        available: usize,
    },
    /// Decode: the frame's data failed its check. For a standard frame the
    /// data CRC doesn't match. For an extended frame the CRC-32K doesn't, or
    /// a code octet in either encoded field is zero or runs past the field's
    /// end, which Clause 9.10.3 treats as a bad CRC too.
    DataCrcMismatch {
        /// The Length declared by the header.
        length: usize,
    },
    /// Stream decode: a candidate frame is longer than the decoder's whole
    /// storage, so it can never be buffered. See [`crate::StreamDecoder`].
    FrameTooLong {
        /// Octets the candidate needs, from its preamble to its last octet.
        needed: usize,
        /// The decoder's storage capacity.
        capacity: usize,
    },
    /// There was no room for the octets. It comes from three places:
    ///
    /// - Encode, before writing: the sink reports a capacity, and the frame
    ///   doesn't fit it. `needed` is the frame's wire length, `available`
    ///   the sink's remaining capacity, and nothing was written.
    /// - Encode, part way through: one of the sink's own writes failed with
    ///   a [`SinkFull`], as a growable sink's can, since it is never checked
    ///   up front. `needed` and `available` are that write's, as the sink
    ///   reported them, and the octets of the earlier writes stay in the
    ///   sink (see [`FrameSink`](crate::FrameSink)).
    /// - Stream decode: a pushed slice is longer than the decoder's free
    ///   storage. `needed` is the slice less the octets of a frame the
    ///   decoder is skipping, `available` the decoder's capacity less what
    ///   it already buffers, and none of the slice was taken.
    BufferExhausted {
        /// Octets the refused call or write had to store.
        needed: usize,
        /// Room there was for them.
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
            Self::BroadcastSource => f.write_str("source address is broadcast (0xFF)"),
            Self::InvalidCobsLength { length } => {
                write!(f, "COBS frame length {length} is out of range")
            }
            Self::ExtendedFrame { raw_type } => {
                write!(f, "extended frame type {raw_type} must be decoded in place")
            }
            Self::Oversize { length, maximum } => {
                write!(f, "data length {length} exceeds maximum {maximum}")
            }
            Self::UnexpectedData { raw_type, length } => {
                write!(
                    f,
                    "frame type {raw_type} carries no data, got {length} octets"
                )
            }
            Self::MissingVendorId { raw_type, length } => write!(
                f,
                "proprietary frame type {raw_type} needs a 2-octet vendor ID, got {length} octets"
            ),
            Self::Truncated { length, available } => {
                write!(f, "truncated frame length {length}, available {available}")
            }
            Self::DataCrcMismatch { .. } => f.write_str("data CRC mismatch"),
            Self::FrameTooLong { needed, capacity } => write!(
                f,
                "frame needs {needed} octets, decoder storage holds {capacity}"
            ),
            Self::BufferExhausted { needed, available } => {
                write!(f, "buffer exhausted: need {needed}, have {available}")
            }
        }
    }
}

impl core::error::Error for FrameError {}

impl From<SinkFull> for FrameError {
    fn from(full: SinkFull) -> Self {
        Self::BufferExhausted {
            needed: full.needed,
            available: full.available,
        }
    }
}
