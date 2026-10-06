//! Clause 21 value types for the Network Port object's B/IP and B/IPv6
//! broadcast-management properties (Clause 12.56): a host and UDP port, and
//! the rows of a BBMD's BDT and FDT.
//!
//! These are property values. The BVLL messages that read or write the same
//! tables on the wire (Annex J.2) use a different, fixed-width layout, which
//! the transport crate owns. The `bacnet-encoding` crate owns the codecs for
//! the types here.

#[cfg(not(feature = "std"))]
use alloc::string::String;
use core::net::{IpAddr, SocketAddr};

/// `BACnetHostAddress` (Clause 21): a host given by IP address or by name,
/// or no host at all.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BACnetHostAddress {
    /// Context tag 0: no host. Clause 12.56.33 uses it, with port 0, for a
    /// global address that is unknown or not yet configured.
    None,
    /// Context tag 1: an IP address, four octets for B/IP and sixteen for
    /// B/IPv6 on the wire.
    Ip(IpAddr),
    /// Context tag 2: an Internet host name (RFC 1123).
    Name(String),
}

/// `BACnetHostNPort` (Clause 21): a host and a UDP port.
///
/// The Network Port object serves FD_BBMD_Address and
/// BACnet_IP_Global_Address as this type, and each broadcast distribution
/// table entry starts with one (Clause 12.56).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BACnetHostNPort {
    /// Context tag 0: the host.
    pub host: BACnetHostAddress,
    /// Context tag 1: the UDP port.
    pub port: u16,
}

impl BACnetHostNPort {
    /// The host and port of `address`.
    pub fn from_socket_addr(address: SocketAddr) -> Self {
        Self {
            host: BACnetHostAddress::Ip(address.ip()),
            port: address.port(),
        }
    }
}

/// `BACnetBDTEntry` (Clause 21): one row of a BBMD's broadcast distribution
/// table, as BBMD_Broadcast_Distribution_Table serves it (Clause 12.56.34).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BACnetBDTEntry {
    /// Context tag 0: the peer BBMD. Clause 12.56.34 rules out the `None`
    /// host for a table entry; the codec leaves that check to whoever
    /// accepts a written table.
    pub bbmd_address: BACnetHostNPort,
    /// Context tag 1: the broadcast distribution mask, most significant octet
    /// first. Present for B/IP and absent for B/IPv6.
    pub broadcast_mask: Option<[u8; 4]>,
}

/// `BACnetFDTEntry` (Clause 21): one registered foreign device, as
/// BBMD_Foreign_Device_Table serves it (Clause 12.56.36).
///
/// On the wire the address is the registrant's 6-octet B/IP or 18-octet
/// B/IPv6 address: the IP address then the UDP port. An IPv6 flow label or
/// scope ID is not part of it; decoding sets both to zero.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BACnetFDTEntry {
    /// Context tag 0: the registrant's B/IP or B/IPv6 address.
    pub address: SocketAddr,
    /// Context tag 1: the time to live, in seconds, the registrant asked for
    /// when it registered.
    pub time_to_live: u16,
    /// Context tag 2: the seconds left before the entry expires, counting the
    /// grace period the BBMD adds to the time to live (Annex J.5.2.3).
    pub remaining_time_to_live: u16,
}
