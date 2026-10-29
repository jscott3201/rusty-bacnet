//! Configured Network Port declarations, independent of a running transport.
use crate::network_port::{Bip6PortConfig, BipPortConfig, NetworkPortObject};
use bacnet_types::{MacAddr, enums::NetworkType, error::Error};
use core::net::{Ipv4Addr, Ipv6Addr};

/// One configured Network-Port entry (see module docs for the profile boundary).
///
/// Provenance: population-rule coverage is Loopback-only except the B/IP +
/// SC entries exercised in the RB-16 real-transport proofs. Field layout
/// follows Clause 12.56; see the module docs for per-transport semantics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkPortEntry {
    /// Network-Port object instance (caller-stable; B/IP=1, SC=2 by convention).
    pub instance: u32,
    /// Clause 12.56 Network_Type (IPV4=5 for B/IP, IPV6=9 for B/IPv6,
    /// VIRTUAL=7 for SC/loopback).
    pub network_type: u32,
    /// BACnet network number for this port.
    pub network_number: u32,
    /// Port MAC (B/IP 6-byte IP+port, B/IPv6 3-byte VMAC, SC VMAC,
    /// loopback MAC). B/IP admission rejects a MAC inconsistent with the
    /// configured IP/UDP.
    pub mac: MacAddr,
    /// IPv4 octets for B/IP; zeros otherwise.
    pub ip: [u8; 4],
    /// IPv6 octets for B/IPv6; zeros otherwise.
    pub ipv6: [u8; 16],
    /// UDP port for B/IP and B/IPv6; 0 for SC/loopback.
    pub udp_port: u16,
}

impl NetworkPortEntry {
    /// B/IP entry with the configured IP/UDP port (possibly unbound).
    ///
    /// Derives the 6-byte B/IP MAC (IP octets + big-endian UDP port) from the
    /// configured socket address. Explicit endpoint registration reconciles the
    /// selected entry with its actual ephemeral port during startup.
    ///
    /// ```
    /// use core::net::Ipv4Addr;
    /// use bacnet_objects::device_identity::NetworkPortEntry;
    ///
    /// let entry = NetworkPortEntry::bip(1, 0, Ipv4Addr::LOCALHOST, 47808);
    /// assert_eq!(entry.instance, 1);
    /// assert_eq!(entry.udp_port, 47808);
    /// ```
    pub fn bip(instance: u32, network_number: u32, ip: Ipv4Addr, udp_port: u16) -> Self {
        let o = ip.octets();
        let mut mac = MacAddr::new();
        mac.extend_from_slice(&o);
        mac.extend_from_slice(&udp_port.to_be_bytes());
        Self {
            instance,
            network_type: NetworkType::IPV4.to_raw(),
            network_number,
            mac,
            ip: o,
            ipv6: [0; 16],
            udp_port,
        }
    }

    /// B/IPv6 entry (Annex U) with the configured address and UDP port
    /// (possibly unbound).
    ///
    /// The VMAC is a zero placeholder: registering the port on a started
    /// `Bip6Transport` publishes the VMAC the transport uses, with its
    /// actual UDP port.
    ///
    /// ```
    /// use core::net::Ipv6Addr;
    /// use bacnet_objects::device_identity::NetworkPortEntry;
    ///
    /// let entry = NetworkPortEntry::bip6(1, 0, Ipv6Addr::LOCALHOST, 47808);
    /// assert_eq!(entry.ipv6, Ipv6Addr::LOCALHOST.octets());
    /// assert_eq!(entry.mac.as_slice(), &[0, 0, 0]);
    /// ```
    pub fn bip6(instance: u32, network_number: u32, ip: Ipv6Addr, udp_port: u16) -> Self {
        Self::bip6_with_vmac(instance, network_number, ip, udp_port, [0; 3])
    }

    /// [`bip6`](Self::bip6) with a known VMAC.
    pub(super) fn bip6_with_vmac(
        instance: u32,
        network_number: u32,
        ip: Ipv6Addr,
        udp_port: u16,
        vmac: [u8; 3],
    ) -> Self {
        Self {
            instance,
            network_type: NetworkType::IPV6.to_raw(),
            network_number,
            mac: MacAddr::from_slice(&vmac),
            ip: [0; 4],
            ipv6: ip.octets(),
            udp_port,
        }
    }

    /// Build the declared configuration; this does not inspect an active port.
    pub(super) fn build_object(&self) -> Result<NetworkPortObject, Error> {
        let network_number = u16::try_from(self.network_number)
            .map_err(|_| Error::Encoding("network number must be 0..65534".into()))?;
        let name = format!("port-{}", self.instance);
        // Configured B/IP/SC capacity, independently of Device62/role limits.
        // Live transport association and capability validation are separate.
        let apdu_length = 1476;
        if self.network_type == NetworkType::IPV4.to_raw() {
            // Public entry fields must describe one configured address. Reject
            // divergent input instead of storing one MAC while deriving another
            // in either database builder.
            let [a, b, c, d] = self.ip;
            let [hi, lo] = self.udp_port.to_be_bytes();
            if self.mac.as_slice() != [a, b, c, d, hi, lo] {
                return Err(Error::Encoding(
                    "B/IP MAC must match configured IPv4 address and UDP port".into(),
                ));
            }
            if self.ipv6 != [0; 16] {
                return Err(Error::Encoding(
                    "a B/IP entry carries no IPv6 address".into(),
                ));
            }
            NetworkPortObject::new_bip(
                self.instance,
                name,
                BipPortConfig {
                    network_number,
                    apdu_length,
                    ip_address: self.ip,
                    udp_port: self.udp_port,
                    ..Default::default()
                },
            )
        } else if self.network_type == NetworkType::IPV6.to_raw() {
            NetworkPortObject::new_bip6(
                self.instance,
                name,
                Bip6PortConfig {
                    network_number,
                    apdu_length,
                    ip_address: self.ipv6,
                    udp_port: self.udp_port,
                    vmac: self.vmac()?,
                    ..Default::default()
                },
            )
        } else {
            NetworkPortObject::new_non_bip(
                self.instance,
                name,
                NetworkType::from_raw(self.network_type),
                network_number,
                self.mac.clone(),
                apdu_length,
            )
        }
    }

    /// SC entry keyed by VMAC (no B/IP socket fields).
    ///
    /// IP octets stay zero and the UDP port stays 0: SC has no B/IP socket.
    /// The VMAC must equal the SC builder's VMAC at hub-dial build time.
    pub fn sc(instance: u32, network_number: u32, vmac: [u8; 6]) -> Self {
        Self {
            instance,
            network_type: NetworkType::VIRTUAL.to_raw(),
            network_number,
            mac: MacAddr::from_slice(&vmac),
            ip: [0; 4],
            ipv6: [0; 16],
            udp_port: 0,
        }
    }

    /// A B/IPv6 entry's VMAC: exactly three octets (Annex U.1.3), and no
    /// IPv4 address beside its IPv6 one.
    pub(super) fn vmac(&self) -> Result<[u8; 3], Error> {
        if self.ip != [0; 4] {
            return Err(Error::Encoding(
                "a B/IPv6 entry carries no IPv4 address".into(),
            ));
        }
        <[u8; 3]>::try_from(self.mac.as_slice())
            .map_err(|_| Error::Encoding("B/IPv6 MAC must be a 3-octet VMAC".into()))
    }
}
