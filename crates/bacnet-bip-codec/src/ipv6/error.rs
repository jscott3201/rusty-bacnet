//! Typed failures emitted by the B/IPv6 BVLL codec.

use core::fmt;

use super::Bvlc6Function;
use crate::SinkFull;

/// A B/IPv6 BVLL message could not be decoded or encoded.
///
/// The variants mirror [`BvllError`](crate::BvllError)'s, for Annex U's
/// type octet, address length and functions; B/IPv6 messages carry no
/// tables. Each one names the direction it comes from. None of them
/// allocates, so the type is `Copy`. Later releases may add variants, so a
/// match outside this crate needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Bvll6Error {
    /// Decode: the datagram is shorter than the 4-octet BVLL header.
    TooShort {
        /// Octets in the datagram.
        length: usize,
    },
    /// Decode: the BVLC Type octet is not 0x82, so the datagram is not a
    /// B/IPv6 BVLL message (Annex U.2).
    NotBip6 {
        /// The type octet received.
        bvlc_type: u8,
    },
    /// Decode: the Length field disagrees with the datagram. It counts the
    /// whole message, header included (Annex U.2), so a datagram cut short
    /// and one with octets after the message both fail here.
    LengthMismatch {
        /// Length the header declares.
        declared: usize,
        /// Octets in the datagram.
        received: usize,
    },
    /// Decode: a function code Annex U.2 doesn't define, so its payload has
    /// no known layout.
    UnknownFunction {
        /// The function octet.
        function: u8,
    },
    /// Decode: the payload doesn't fit its function's layout (see
    /// [`Bvlc6Function::payload_shape`]).
    PayloadLength {
        /// The message's function.
        function: Bvlc6Function,
        /// Octets after the header.
        length: usize,
    },
    /// A B/IPv6 address that is not 18 octets long (Annex U.1.1.2).
    AddressLength {
        /// Octets supplied.
        length: usize,
    },
    /// Encode: the message is longer than its 16-bit Length field can state.
    TooLong {
        /// Octets the message would take.
        length: usize,
    },
    /// Encode: a BVLC-Result whose code Annex U.2.1.1 doesn't define.
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

impl fmt::Display for Bvll6Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { length } => write!(
                f,
                "BVLC6 datagram too short: {length} octets, header needs 4"
            ),
            Self::NotBip6 { bvlc_type } => {
                write!(f, "BVLC type {bvlc_type:#04X} is not B/IPv6 (0x82)")
            }
            Self::LengthMismatch { declared, received } => write!(
                f,
                "BVLC6 length {declared} does not match the {received}-octet datagram"
            ),
            Self::UnknownFunction { function } => {
                write!(f, "unknown BVLC6 function {function:#04X}")
            }
            Self::PayloadLength { function, length } => {
                write!(f, "{function} cannot carry a {length}-octet payload")
            }
            Self::AddressLength { length } => {
                write!(f, "B/IPv6 address must be 18 octets, got {length}")
            }
            Self::TooLong { length } => write!(
                f,
                "BVLC6 message of {length} octets exceeds the 16-bit BVLC6 length field"
            ),
            Self::UndefinedResultCode { code } => {
                write!(f, "BVLC6 result code {code:#06X} is not defined")
            }
            Self::BufferExhausted { needed, available } => {
                write!(f, "buffer exhausted: need {needed}, have {available}")
            }
        }
    }
}

impl core::error::Error for Bvll6Error {}

impl From<SinkFull> for Bvll6Error {
    fn from(full: SinkFull) -> Self {
        Self::BufferExhausted {
            needed: full.needed,
            available: full.available,
        }
    }
}
