//! Why [`decode_npdu`](super::decode_npdu) refused an NPDU.

use core::fmt;

use bacnet_npdu::{AddressField, Malformed, MAX_MAC_LEN};
use bacnet_types::error::{DecodeError, DecodingKind, Error, RangeError};

use super::NpduAddress;

/// Why [`decode_npdu`](super::decode_npdu) refused an NPDU: `bacnet-npdu`'s
/// [`DecodeError`](bacnet_npdu::DecodeError), with the source behind an
/// over-long DADR owned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NpduDecodeError {
    /// The DLEN or SLEN octet is larger than [`MAX_MAC_LEN`] (#1141), so the
    /// address names no node on a standard data link or on one this stack
    /// serves.
    ///
    /// Clause 6.4.4 gives routers Reject-Message-To-Network reason 6
    /// (`ADDRESSING_ERROR`) for an NPDU whose DADR or SADR length is invalid.
    /// [`dnet`](Self::AddressTooLong::dnet) is the network the reject names,
    /// and [`source`](Self::AddressTooLong::source) is where it goes when a
    /// router relayed the refused NPDU (#1158).
    AddressTooLong {
        /// Which address the length octet introduces.
        field: AddressField,
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
    Malformed(Malformed),
}

impl fmt::Display for NpduDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AddressTooLong { field, length, .. } => write!(
                f,
                "NPDU {}={length} exceeds the {MAX_MAC_LEN}-octet address limit",
                field.length_octet(),
            ),
            Self::Malformed(fault) => fault.fmt(f),
        }
    }
}

impl core::error::Error for NpduDecodeError {}

impl From<NpduDecodeError> for Error {
    /// An over-long address becomes [`Error::Range`]: the length octet is
    /// well formed, but its value is past the limit. Its text is the
    /// refusal's own. Any other fault becomes [`Error::Decoding`], with the
    /// fault's text: [`DecodingKind::Missing`] for a frame that ends inside
    /// a field, [`DecodingKind::InvalidEncoding`] for the rest. Neither
    /// knows the offset, so it reads 0.
    fn from(error: NpduDecodeError) -> Self {
        match error {
            NpduDecodeError::AddressTooLong { field, length, .. } => {
                let label = match field {
                    AddressField::Destination => "NPDU DLEN={} exceeds the {}-octet address limit",
                    AddressField::Source => "NPDU SLEN={} exceeds the {}-octet address limit",
                };
                RangeError::new(label)
                    .with_values(length.into(), MAX_MAC_LEN as u64)
                    .into()
            }
            NpduDecodeError::Malformed(fault) => {
                let (kind, label) = match fault {
                    Malformed::Truncated => (DecodingKind::Missing, "NPDU truncated"),
                    Malformed::Version => (
                        DecodingKind::InvalidEncoding,
                        "unsupported BACnet protocol version",
                    ),
                    Malformed::ReservedNetwork => (
                        DecodingKind::InvalidEncoding,
                        "NPDU names a reserved network number",
                    ),
                    Malformed::EmptySource => (
                        DecodingKind::InvalidEncoding,
                        "NPDU source SLEN=0 is invalid",
                    ),
                    // A fault a later bacnet-npdu adds.
                    _ => (DecodingKind::InvalidEncoding, "NPDU malformed"),
                };
                Error::Decoding(DecodeError::new(kind, 0, label))
            }
        }
    }
}
