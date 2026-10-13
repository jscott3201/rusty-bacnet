//! Typed failures emitted by the BVLL codec.

use core::fmt;

use crate::{BvlcFunction, SinkFull};

/// A BVLL message could not be decoded or encoded.
///
/// Each variant names the direction it comes from. None of them allocates,
/// so the type is `Copy`. Later releases may add variants, so a match
/// outside this crate needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BvllError {
    /// Decode: the datagram is shorter than the 4-octet BVLL header.
    TooShort {
        /// Octets in the datagram.
        length: usize,
    },
    /// Decode: the BVLC Type octet is not 0x81, so the datagram is not a
    /// B/IP BVLL message (Annex J.2).
    NotBip {
        /// The type octet received.
        bvlc_type: u8,
    },
    /// Decode: the Length field disagrees with the datagram. It counts the
    /// whole message, header included (Annex J.2), so a datagram cut short
    /// and one with octets after the message both fail here.
    LengthMismatch {
        /// Length the header declares.
        declared: usize,
        /// Octets in the datagram.
        received: usize,
    },
    /// Decode: a function code Annex J.2 doesn't define, so its payload has
    /// no known layout.
    UnknownFunction {
        /// The function octet.
        function: u8,
    },
    /// Decode: the payload doesn't fit its function's layout (see
    /// [`BvlcFunction::payload_shape`]).
    PayloadLength {
        /// The message's function.
        function: BvlcFunction,
        /// Octets after the header.
        length: usize,
    },
    /// A table's octets are not a whole number of entries.
    TableLength {
        /// Octets supplied.
        length: usize,
        /// Octets per entry.
        entry_length: usize,
    },
    /// A B/IP address that is not 6 octets long (Annex J.1.2).
    AddressLength {
        /// Octets supplied.
        length: usize,
    },
    /// Encode: the message is longer than its 16-bit Length field can state.
    TooLong {
        /// Octets the message would take.
        length: usize,
    },
    /// Encode: a BVLC-Result whose code Annex J.2.1.1 doesn't define.
    UndefinedResultCode {
        /// The code supplied.
        code: u16,
    },
    /// Encode: the sink has no room for the message. Nothing was written to
    /// a sink with a fixed capacity.
    BufferExhausted {
        /// Octets the write needed.
        needed: usize,
        /// Room the sink had.
        available: usize,
    },
}

impl fmt::Display for BvllError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { length } => {
                write!(
                    f,
                    "BVLL datagram too short: {length} octets, header needs 4"
                )
            }
            Self::NotBip { bvlc_type } => {
                write!(f, "BVLC type {bvlc_type:#04X} is not B/IP (0x81)")
            }
            Self::LengthMismatch { declared, received } => write!(
                f,
                "BVLC length {declared} does not match the {received}-octet datagram"
            ),
            Self::UnknownFunction { function } => {
                write!(f, "unknown BVLC function {function:#04X}")
            }
            Self::PayloadLength { function, length } => {
                write!(f, "{function} cannot carry a {length}-octet payload")
            }
            Self::TableLength {
                length,
                entry_length,
            } => write!(
                f,
                "table of {length} octets is not a whole number of {entry_length}-octet entries"
            ),
            Self::AddressLength { length } => {
                write!(f, "B/IP address must be 6 octets, got {length}")
            }
            Self::TooLong { length } => write!(
                f,
                "BVLL message of {length} octets exceeds the 16-bit BVLC length field"
            ),
            Self::UndefinedResultCode { code } => {
                write!(f, "BVLC result code {code:#06X} is not defined")
            }
            Self::BufferExhausted { needed, available } => {
                write!(f, "buffer exhausted: need {needed}, have {available}")
            }
        }
    }
}

impl core::error::Error for BvllError {}

impl From<SinkFull> for BvllError {
    fn from(full: SinkFull) -> Self {
        Self::BufferExhausted {
            needed: full.needed,
            available: full.available,
        }
    }
}
