//! B/IPv6 addresses and virtual MAC addresses.

use core::fmt;
use core::net::{Ipv6Addr, SocketAddrV6};

use super::{Bvll6Error, BIP6_ADDRESS_LENGTH, VMAC_LENGTH};

/// Octets in the IPv6 address at the front of a B/IPv6 address.
const IPV6_LENGTH: usize = 16;

/// A B/IPv6 address: a 16-octet IPv6 address, then a 2-octet UDP port, both
/// most significant octet first (Annex U.1.1.2).
///
/// It holds exactly the 18 octets that travel. A [`SocketAddrV6`] also has a
/// flow label and a scope ID, neither of which a B/IPv6 address carries:
/// converting from one drops them, and converting to one sets both to zero,
/// so the receiver of a link-local address picks its scope.
///
/// ```
/// use core::net::Ipv6Addr;
/// use bacnet_bip_codec::ipv6::Bip6Address;
///
/// let address = Bip6Address::new(Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0, 1), 47808);
/// let octets = address.to_octets();
/// assert_eq!(octets[..4], [0x20, 0x01, 0x0D, 0xB8]);
/// assert_eq!(octets[16..], [0xBA, 0xC0]);
/// assert_eq!(Bip6Address::from_octets(octets), address);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Bip6Address {
    /// The IPv6 address.
    pub ip: Ipv6Addr,
    /// The UDP port.
    pub port: u16,
}

impl Bip6Address {
    /// Construct an address.
    #[must_use]
    pub const fn new(ip: Ipv6Addr, port: u16) -> Self {
        Self { ip, port }
    }

    /// The address's 18 wire octets: the IPv6 address, then the port.
    #[must_use]
    pub const fn to_octets(self) -> [u8; BIP6_ADDRESS_LENGTH] {
        let mut octets = [0; BIP6_ADDRESS_LENGTH];
        let (ip, port) = octets.split_at_mut(IPV6_LENGTH);
        ip.copy_from_slice(&self.ip.octets());
        port.copy_from_slice(&self.port.to_be_bytes());
        octets
    }

    /// The address 18 wire octets hold.
    #[must_use]
    pub const fn from_octets(octets: [u8; BIP6_ADDRESS_LENGTH]) -> Self {
        let mut ip = [0; IPV6_LENGTH];
        ip.copy_from_slice(octets.split_at(IPV6_LENGTH).0);
        Self {
            ip: Ipv6Addr::from_octets(ip),
            port: u16::from_be_bytes([octets[IPV6_LENGTH], octets[IPV6_LENGTH + 1]]),
        }
    }

    /// The address in `octets`, which must be exactly 18 octets long.
    pub fn from_slice(octets: &[u8]) -> Result<Self, Bvll6Error> {
        match <[u8; BIP6_ADDRESS_LENGTH]>::try_from(octets) {
            Ok(octets) => Ok(Self::from_octets(octets)),
            Err(_) => Err(Bvll6Error::AddressLength {
                length: octets.len(),
            }),
        }
    }
}

/// Keeps the IPv6 address and port; the flow label and scope ID don't
/// travel, so they are dropped.
impl From<SocketAddrV6> for Bip6Address {
    fn from(address: SocketAddrV6) -> Self {
        Self::new(*address.ip(), address.port())
    }
}

/// A socket address with flow label 0 and scope ID 0.
impl From<Bip6Address> for SocketAddrV6 {
    fn from(address: Bip6Address) -> Self {
        SocketAddrV6::new(address.ip, address.port, 0, 0)
    }
}

/// `[ip]:port`, the notation of RFC 5952 that Annex U.1.1.1.1 uses.
impl fmt::Display for Bip6Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}]:{}", self.ip, self.port)
    }
}

/// A B/IPv6 virtual MAC address (VMAC): three octets, which the network
/// layer uses as the node's MAC address (Annex U.1.3).
///
/// A node with a device instance uses that instance, high octet first; one
/// without generates a random-instance VMAC instead (H.7.2). The codec
/// carries any three octets and doesn't judge which kind they are.
///
/// ```
/// use bacnet_bip_codec::ipv6::Bip6Vmac;
///
/// // Device instance 1234 (0x0004D2).
/// let vmac = Bip6Vmac::new([0x00, 0x04, 0xD2]);
/// assert_eq!(vmac.octets(), [0x00, 0x04, 0xD2]);
/// assert_eq!(Bip6Vmac::from([0x00, 0x04, 0xD2]), vmac);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Bip6Vmac([u8; VMAC_LENGTH]);

impl Bip6Vmac {
    /// The VMAC these three octets spell.
    #[must_use]
    pub const fn new(octets: [u8; VMAC_LENGTH]) -> Self {
        Self(octets)
    }

    /// The three wire octets.
    #[must_use]
    pub const fn octets(self) -> [u8; VMAC_LENGTH] {
        self.0
    }
}

impl From<[u8; VMAC_LENGTH]> for Bip6Vmac {
    fn from(octets: [u8; VMAC_LENGTH]) -> Self {
        Self(octets)
    }
}

impl From<Bip6Vmac> for [u8; VMAC_LENGTH] {
    fn from(vmac: Bip6Vmac) -> Self {
        vmac.0
    }
}
