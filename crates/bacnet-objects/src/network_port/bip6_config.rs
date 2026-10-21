//! Declared configuration for an unbound application-level B/IPv6 port
//! (Annex U). As for B/IP, the mode is not configuration: a registered
//! owner publishes it, with its actual bind.

use crate::common;
use bacnet_types::error::Error;

/// FF05::BAC0, the site-local BACnet group a B/IPv6 transport broadcasts to
/// by default (Annex U.4).
const SITE_LOCAL_GROUP: [u8; 16] = [0xFF, 0x05, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xBA, 0xC0];

/// Complete configured B/IPv6 snapshot. Fixed-size addresses prevent
/// malformed IPv6 values. The unspecified address `::` for the address,
/// gateway or one DNS server means unknown or unconfigured; none of them
/// describes an observed interface or route.
///
/// The object serves this as a port whose addresses are configured by hand:
/// it lists no IPv6_Auto_Addressing_Enable (Table 12-71 footnote 16) or
/// IPv6_Zone_Index (footnote 17) row, nor the optional DHCPv6 rows, and
/// every row is read-only over BACnet.
///
/// `vmac`, and the UDP port the multicast row carries, are placeholders
/// until a registered owner publishes the VMAC it uses and the port it is
/// bound to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bip6PortConfig {
    /// Caller-configured number; zero means unknown, 65535 is invalid.
    pub network_number: u16,
    /// Port APDU_Length, independent of Device Max_APDU_Length_Accepted;
    /// at least 50.
    pub apdu_length: u32,
    /// Configured IPv6 address, most significant octet first: IPv6_Address.
    pub ip_address: [u8; 16],
    /// The subnet prefix length of `ip_address`, 1 to 128:
    /// IPv6_Prefix_Length.
    pub prefix_length: u8,
    /// BACnet_IPv6_UDP_Port. Zero is allowed in this unbound configuration.
    pub udp_port: u16,
    /// Configured gateway, with no assumed OS route: IPv6_Default_Gateway.
    pub default_gateway: [u8; 16],
    /// At least one address; `::` represents unavailable DNS:
    /// IPv6_DNS_Server.
    pub dns_servers: Vec<[u8; 16]>,
    /// The multicast group the port broadcasts to. BACnet_IPv6_Multicast_Address
    /// serves it followed by the port's UDP port, since the transport sends
    /// its broadcasts to the group at its own port.
    pub multicast_address: [u8; 16],
    /// The port's 3-octet VMAC (Annex U.1.3): MAC_Address. A registered
    /// owner publishes the VMAC it actually uses, which replaces this one.
    pub vmac: [u8; 3],
}

impl Default for Bip6PortConfig {
    fn default() -> Self {
        Self {
            network_number: 0,
            // Declared B/IPv6 capacity of this library's transport profile.
            apdu_length: 1476,
            ip_address: [0; 16],
            prefix_length: 64,
            udp_port: 47808,
            default_gateway: [0; 16],
            dns_servers: vec![[0; 16]],
            multicast_address: SITE_LOCAL_GROUP,
            vmac: [0; 3],
        }
    }
}

impl Bip6PortConfig {
    pub(super) fn validate(&self, instance: u32) -> Result<(), Error> {
        // The flat application profile's local Port ID policy, as for B/IP;
        // a prefix length is 1 to 128 (Clause 12.56.41); a broadcast group
        // is an IPv6 multicast address (Annex U.4); an IPv4-mapped address
        // is an IPv4 port, which `new_bip` describes.
        if !(1..=255).contains(&instance)
            || std::net::Ipv6Addr::from(self.ip_address)
                .to_ipv4_mapped()
                .is_some()
            || self.network_number == u16::MAX
            || self.apdu_length < 50
            || self.dns_servers.is_empty()
            || !(1..=128).contains(&self.prefix_length)
            || self.multicast_address[0] != 0xFF
        {
            return Err(common::value_out_of_range_error());
        }
        Ok(())
    }

    /// BACnet_IPv6_Multicast_Address: the group, then the UDP port, each
    /// most significant octet first (Clause 12.56.44).
    pub(super) fn multicast_with_port(&self) -> Vec<u8> {
        let mut value = self.multicast_address.to_vec();
        value.extend_from_slice(&self.udp_port.to_be_bytes());
        value
    }
}

/// What a started B/IPv6 owner publishes for its registered Network Port:
/// the address and UDP port it is bound to, the VMAC it uses and the group
/// it broadcasts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Bip6Bind {
    /// The bound IPv6 address, most significant octet first.
    pub ip: [u8; 16],
    /// The bound UDP port, never zero.
    pub udp: u16,
    /// The VMAC the owner uses (Annex U.1.3).
    pub vmac: [u8; 3],
    /// The multicast group its broadcasts go to.
    pub multicast: [u8; 16],
}
