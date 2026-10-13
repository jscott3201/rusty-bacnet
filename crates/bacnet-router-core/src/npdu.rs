//! A borrowed view of an NPDU (Clause 6.2), and its writer.
//!
//! [`Npdu::parse`] reads the protocol control information in place and
//! borrows every variable field from the frame: the DADR and SADR, and the
//! payload behind the NPCI, with its offset so a host can slice its own
//! buffer. It accepts and refuses exactly what `bacnet-encoding`'s
//! `decode_npdu` does, and reports the same fields; a property test against
//! that decoder keeps the two equal. As there, a DLEN or SLEN past
//! [`MAX_MAC_LEN`] is refused before its address is read, apart from every
//! other fault ([`ParseError::AddressTooLong`]), with what a router needs to
//! address its reject.
//!
//! [`Npdu::write`] goes the other way, into the caller's buffer, after
//! [`Npdu::encoded_len`] has told the caller how long the result will be.
//! Like the decoder, the view keeps no reserved control bits, so writing a
//! parsed NPDU gives the octets `bacnet-encoding` would encode for it.

mod mac;
mod write;

use core::fmt;

pub use mac::{Mac, MacTooLong};
pub use write::WriteError;

use crate::message::MessageType;
use crate::MAX_MAC_LEN;

/// The protocol version every NPDU starts with.
pub const VERSION: u8 = 1;

/// The DNET of a global broadcast.
pub const GLOBAL_NETWORK: u16 = 0xFFFF;

/// Control octet: a network-layer message, not an APDU.
const NETWORK_MESSAGE: u8 = 0x80;
/// Control octet: DNET, DLEN, DADR and the hop count are present.
const DESTINATION: u8 = 0x20;
/// Control octet: SNET, SLEN and SADR are present.
const SOURCE: u8 = 0x08;
/// Control octet: the sender expects a reply.
const EXPECTING_REPLY: u8 = 0x04;
/// Control octet: the network priority, 0 to 3.
const PRIORITY: u8 = 0x03;

/// A network and a MAC on it: a DNET and DADR, or an SNET and SADR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Address<'a> {
    /// The network number.
    pub network: u16,
    /// The MAC, borrowed from the frame. An empty DADR is a broadcast on
    /// that network.
    pub mac: &'a [u8],
}

/// An NPDU's control information, with its payload, borrowed from a frame.
///
/// The fields are those of `bacnet-encoding`'s `Npdu`, without its owned
/// buffers: a network-layer message is one whose [`message_type`] is set.
///
/// [`message_type`]: Self::message_type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Npdu<'a> {
    /// Whether the sender expects a reply.
    pub expecting_reply: bool,
    /// The network priority, 0 (normal) to 3 (life safety): the control
    /// octet's two low bits. A parsed NPDU never has more than 3, and the
    /// writer writes only those two bits.
    pub priority: u8,
    /// DNET and DADR, for a remote destination or a global broadcast.
    pub destination: Option<Address<'a>>,
    /// SNET and SADR, which a router adds to what it relays.
    pub source: Option<Address<'a>>,
    /// The hop count. Only an NPDU with a destination carries one; without
    /// it this is 255, as in `bacnet-encoding`.
    pub hop_count: u8,
    /// The network-layer message type, or `None` for an APDU.
    pub message_type: Option<MessageType>,
    /// The vendor ID of a proprietary message type
    /// ([`MessageType::is_proprietary`]).
    pub vendor_id: Option<u16>,
    /// The APDU, or the message's data, behind the NPCI.
    pub payload: &'a [u8],
    /// Where [`payload`](Self::payload) starts in the frame. Ignored by the
    /// writer.
    pub payload_offset: usize,
}

/// Which address a [`ParseError::AddressTooLong`] concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressField {
    /// DLEN and DADR.
    Destination,
    /// SLEN and SADR.
    Source,
}

/// Why [`Npdu::parse`] refused a frame.
///
/// It is `#[non_exhaustive]`, so a new refusal isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError<'a> {
    /// A DLEN or SLEN past [`MAX_MAC_LEN`]: an address no data link uses,
    /// which draws reject reason 6 (Clause 6.4.4) if the frame names a DNET.
    AddressTooLong {
        /// Which address's length octet it is.
        field: AddressField,
        /// The length octet as received.
        length: u8,
        /// The DNET, when the frame has one: it precedes both lengths.
        dnet: Option<u16>,
        /// The SNET and SADR behind an over-long DADR, when the frame holds
        /// a complete, valid one after the DADR octets the DLEN announces:
        /// where the reject goes. `None` when the SADR is the long one.
        source: Option<Address<'a>>,
    },
    /// Anything else.
    Malformed(Malformed),
}

/// What made a frame malformed.
///
/// It is `#[non_exhaustive]`, so a new fault isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Malformed {
    /// The frame ends inside a field it announces.
    Truncated,
    /// The first octet is not protocol version 1.
    Version,
    /// A DNET of 0, or an SNET of 0 or 0xFFFF.
    ReservedNetwork,
    /// An SLEN of 0.
    EmptySource,
}

impl From<Malformed> for ParseError<'_> {
    fn from(fault: Malformed) -> Self {
        Self::Malformed(fault)
    }
}

impl fmt::Display for Malformed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Truncated => "NPDU truncated",
            Self::Version => "unsupported BACnet protocol version",
            Self::ReservedNetwork => "NPDU names a reserved network number",
            Self::EmptySource => "NPDU source SLEN=0 is invalid",
        })
    }
}

impl core::error::Error for Malformed {}

impl fmt::Display for ParseError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AddressTooLong { field, length, .. } => {
                let octet = match field {
                    AddressField::Destination => "DLEN",
                    AddressField::Source => "SLEN",
                };
                write!(
                    f,
                    "NPDU {octet}={length} exceeds the {MAX_MAC_LEN}-octet address limit"
                )
            }
            Self::Malformed(fault) => fault.fmt(f),
        }
    }
}

impl core::error::Error for ParseError<'_> {}

impl<'a> Npdu<'a> {
    /// An APDU for the local network: no addresses, normal priority.
    #[must_use]
    pub const fn apdu(payload: &'a [u8]) -> Self {
        Self {
            expecting_reply: false,
            priority: 0,
            destination: None,
            source: None,
            hop_count: 255,
            message_type: None,
            vendor_id: None,
            payload,
            payload_offset: 0,
        }
    }

    /// A network-layer message of a standard type for the local network: no
    /// addresses, normal priority, no vendor ID.
    #[must_use]
    pub const fn message(message_type: MessageType, payload: &'a [u8]) -> Self {
        let mut npdu = Self::apdu(payload);
        npdu.message_type = Some(message_type);
        npdu
    }

    /// Whether it carries a network-layer message rather than an APDU.
    #[must_use]
    pub const fn is_network_message(&self) -> bool {
        self.message_type.is_some()
    }

    /// Whether its DNET is the global broadcast network.
    #[must_use]
    pub fn is_global(&self) -> bool {
        self.destination
            .is_some_and(|destination| destination.network == GLOBAL_NETWORK)
    }

    /// Read the NPDU at the start of `frame`, borrowing from it.
    ///
    /// The checks run in the order the fields come, as in `bacnet-encoding`,
    /// so the two report the same fault for every frame.
    pub fn parse(frame: &'a [u8]) -> Result<Self, ParseError<'a>> {
        let [version, control, ..] = *frame else {
            return Err(Malformed::Truncated.into());
        };
        if version != VERSION {
            return Err(Malformed::Version.into());
        }
        let has_source = control & SOURCE != 0;
        let mut at = 2;

        let mut destination = None;
        if control & DESTINATION != 0 {
            let (network, length) = network_and_length(frame, at)?;
            if network == 0 {
                return Err(Malformed::ReservedNetwork.into());
            }
            at += 3;
            if usize::from(length) > MAX_MAC_LEN {
                return Err(ParseError::AddressTooLong {
                    field: AddressField::Destination,
                    length,
                    dnet: Some(network),
                    source: if has_source {
                        source_behind(frame, at + usize::from(length))
                    } else {
                        None
                    },
                });
            }
            let mac = octets(frame, at, length)?;
            at += mac.len();
            destination = Some(Address { network, mac });
        }

        let mut source = None;
        if has_source {
            let (network, length) = network_and_length(frame, at)?;
            if network == 0 || network == GLOBAL_NETWORK {
                return Err(Malformed::ReservedNetwork.into());
            }
            at += 3;
            if length == 0 {
                return Err(Malformed::EmptySource.into());
            }
            if usize::from(length) > MAX_MAC_LEN {
                return Err(ParseError::AddressTooLong {
                    field: AddressField::Source,
                    length,
                    dnet: destination.map(|destination| destination.network),
                    source: None,
                });
            }
            let mac = octets(frame, at, length)?;
            at += mac.len();
            source = Some(Address { network, mac });
        }

        let mut hop_count = 255;
        if destination.is_some() {
            hop_count = *frame.get(at).ok_or(Malformed::Truncated)?;
            at += 1;
        }

        let mut message_type = None;
        let mut vendor_id = None;
        if control & NETWORK_MESSAGE != 0 {
            let kind = MessageType::from_raw(*frame.get(at).ok_or(Malformed::Truncated)?);
            at += 1;
            if kind.is_proprietary() {
                let vendor = octets(frame, at, 2)?;
                vendor_id = Some(u16::from_be_bytes([vendor[0], vendor[1]]));
                at += 2;
            }
            message_type = Some(kind);
        }

        Ok(Self {
            expecting_reply: control & EXPECTING_REPLY != 0,
            priority: control & PRIORITY,
            destination,
            source,
            hop_count,
            message_type,
            vendor_id,
            payload: &frame[at..],
            payload_offset: at,
        })
    }
}

/// The two-octet network number and the length octet at `at`.
fn network_and_length(frame: &[u8], at: usize) -> Result<(u16, u8), Malformed> {
    match octets(frame, at, 3)? {
        &[high, low, length] => Ok((u16::from_be_bytes([high, low]), length)),
        _ => Err(Malformed::Truncated),
    }
}

/// The `length` octets at `at`, all of them present.
fn octets(frame: &[u8], at: usize, length: u8) -> Result<&[u8], Malformed> {
    frame
        .get(at..at + usize::from(length))
        .ok_or(Malformed::Truncated)
}

/// The SNET and SADR at `at`, behind a DADR too long to read: only one the
/// parser would accept, complete, so a router's reject reaches a real node.
fn source_behind(frame: &[u8], at: usize) -> Option<Address<'_>> {
    let (network, length) = network_and_length(frame, at).ok()?;
    if network == 0 || network == GLOBAL_NETWORK || length == 0 {
        return None;
    }
    if usize::from(length) > MAX_MAC_LEN {
        return None;
    }
    let mac = octets(frame, at + 3, length).ok()?;
    Some(Address { network, mac })
}
