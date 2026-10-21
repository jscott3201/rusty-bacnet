//! Configured Network Port declarations, independent of a running transport.
use crate::network_port::{BipPortConfig, NetworkPortObject};
use bacnet_types::{enums::NetworkType, error::Error, MacAddr};
use std::net::Ipv4Addr;

/// One configured Network-Port entry (see module docs for the profile boundary).
///
/// Provenance: population-rule coverage is Loopback-only except the B/IP +
/// SC entries exercised in the RB-16 real-transport proofs. Field layout
/// follows Clause 12.56; see the module docs for per-transport semantics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkPortEntry {
    /// Network-Port object instance (caller-stable; B/IP=1, SC=2 by convention).
    pub instance: u32,
    /// Clause 12.56 Network_Type (IPV4=5 for B/IP, VIRTUAL=7 for SC/loopback).
    pub network_type: u32,
    /// BACnet network number for this port.
    pub network_number: u32,
    /// Port MAC (B/IP 6-byte IP+port, SC VMAC, loopback MAC).
    /// B/IP admission rejects a MAC inconsistent with the configured IP/UDP.
    pub mac: MacAddr,
    /// IPv4 octets for B/IP; zeros for SC/loopback.
    pub ip: [u8; 4],
    /// UDP port for B/IP; 0 for SC/loopback.
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
    /// use std::net::Ipv4Addr;
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
            udp_port: 0,
        }
    }
}
