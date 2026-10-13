//! Why [`decode_npdu`](super::decode_npdu) refused an NPDU.

#[cfg(not(feature = "std"))]
use alloc::string::ToString;

use core::fmt;

use bacnet_types::error::Error;

use super::NpduAddress;

/// The NPCI address an [`NpduDecodeError::AddressTooLong`] concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NpduAddressField {
    /// DLEN/DADR: the ultimate destination.
    Destination,
    /// SLEN/SADR: the original source.
    Source,
}

impl NpduAddressField {
    /// The NPCI name of this address's length octet: `DLEN` or `SLEN`.
    pub fn length_octet(self) -> &'static str {
        match self {
            Self::Destination => "DLEN",
            Self::Source => "SLEN",
        }
    }
}

impl fmt::Display for NpduAddressField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Destination => "destination",
            Self::Source => "source",
        })
    }
}

/// Why [`decode_npdu`](super::decode_npdu) refused an NPDU.
#[derive(Debug)]
pub enum NpduDecodeError {
    /// The DLEN or SLEN octet is larger than [`NpduAddress::MAX_MAC_LEN`]
    /// (#1141), so the address names no node on a standard data link or on
    /// one this stack serves.
    ///
    /// Clause 6.4.4 gives routers Reject-Message-To-Network reason 6
    /// (`ADDRESSING_ERROR`) for an NPDU whose DADR or SADR length is invalid.
    /// [`dnet`](Self::AddressTooLong::dnet) is the network the reject names,
    /// and [`source`](Self::AddressTooLong::source) is where it goes when a
    /// router relayed the refused NPDU (#1158).
    AddressTooLong {
        /// Which address the length octet introduces.
        field: NpduAddressField,
        /// The DLEN or SLEN octet as received.
        length: u8,
        /// The NPDU's DNET, when its control octet announces one. It precedes
        /// both length octets on the wire, so it is always known here.
        dnet: Option<u16>,
        /// The NPDU's SNET/SADR, when the over-long field is the DADR and a
        /// complete, valid source follows the DLEN octets the frame announces.
        /// It is `None` when the SADR itself is the over-long field, since
        /// that source names no node.
        source: Option<NpduAddress>,
    },
    /// Any other malformation: a short buffer, an unknown protocol version, a
    /// reserved network number, SLEN 0, or a truncated address or field.
    Malformed(Error),
}

impl fmt::Display for NpduDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AddressTooLong { field, length, .. } => write!(
                f,
                "NPDU {}={length} exceeds the {}-octet address limit",
                field.length_octet(),
                NpduAddress::MAX_MAC_LEN
            ),
            Self::Malformed(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for NpduDecodeError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::AddressTooLong { .. } => None,
            Self::Malformed(error) => Some(error),
        }
    }
}

impl From<Error> for NpduDecodeError {
    fn from(error: Error) -> Self {
        Self::Malformed(error)
    }
}

impl From<NpduDecodeError> for Error {
    /// An over-long address becomes [`Error::OutOfRange`]: the length octet
    /// is well formed, but its value is past the limit.
    fn from(error: NpduDecodeError) -> Self {
        match error {
            NpduDecodeError::AddressTooLong { .. } => Error::OutOfRange(error.to_string()),
            NpduDecodeError::Malformed(error) => error,
        }
    }
}
