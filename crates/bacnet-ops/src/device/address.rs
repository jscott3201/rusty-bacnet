//! [`DeviceAddress`]: where a device answers, written in the target
//! grammar ([`crate::target`]).

use std::fmt;
use std::str::FromStr;

use bacnet_client::client::BACnetClient;
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_transport::port::TransportPort;
use bacnet_types::MacAddr;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::error::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::target::{
    AddressStyle, MSTP_TEXT_PREFIX, Target, format_mac, format_sc_mac, parse_target, routed_station,
};

/// Where a device answers: its network (0 for the local one) and its MAC
/// there. Two devices sharing an instance are two addresses, and one device
/// named by its instance or by its address is one, so this is what the
/// request gates, the no-RPM memory and the device handles key on.
///
/// The router a remote station is reached through is not part of it: the
/// low-level client's discovery table keeps that on the station's row.
///
/// It reads and writes as the target grammar's address forms
/// ([`parse_target`]): `192.168.1.10:47808`, `bip6:0x0004D2`, `mstp:5` or
/// six hex octets on the local network, and `2:0x05` behind a router.
/// [`Display`](fmt::Display) writes a local MAC by its length, the forms
/// [`Error::AmbiguousDevice`] prints: one octet as `mstp:N`, three as a
/// B/IPv6 VMAC and six as `ip:port` (also for an SC VMAC or an Ethernet
/// MAC, the same six octets; [`styled`](Self::styled) writes an SC VMAC as
/// hex). Those are the only local MAC lengths the grammar writes, and so
/// the only ones an address takes: every address reads back, and serde
/// uses the same text.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceAddress {
    network: u16,
    mac: MacAddr,
}

impl DeviceAddress {
    /// The station at `mac` on `network`, 0 being the local network.
    ///
    /// # Errors
    ///
    /// Network 65535 (every network), an empty MAC (a broadcast), a MAC
    /// longer than [`BACnetAddress::MAX_MAC_LEN`] octets, a local MAC of a
    /// length the target grammar has no form for (one of 1, 3 or 6
    /// octets), and the MS/TP broadcast `0xFF` as a one-octet MAC name no
    /// one station.
    pub fn new(network: u16, mac: &[u8]) -> Result<Self, String> {
        if network == u16::MAX {
            return Err("network 65535 is every network, not a station's".into());
        }
        if mac.is_empty() || mac.len() > BACnetAddress::MAX_MAC_LEN {
            return Err(format!(
                "a station's MAC has 1 to {} octets, not {}",
                BACnetAddress::MAX_MAC_LEN,
                mac.len()
            ));
        }
        if network == 0 && !matches!(mac.len(), 1 | 3 | 6) {
            return Err(format!(
                "a MAC on the local network has 1 (MS/TP), 3 (B/IPv6) or 6 octets, not {}",
                mac.len()
            ));
        }
        if mac == [0xFF] {
            return Err("MAC 0xFF is the MS/TP broadcast, not a station".into());
        }
        Ok(Self {
            network,
            mac: MacAddr::from_slice(mac),
        })
    }

    /// The station at `mac` on the local network.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn local(mac: &[u8]) -> Result<Self, String> {
        Self::new(0, mac)
    }

    /// Where `device`, a row of the low-level client's discovery table,
    /// answers; `None` when its routing fields name no one station
    /// ([`DiscoveredDevice::address`]).
    pub fn of(device: &DiscoveredDevice) -> Option<Self> {
        let address = device.address()?;
        Self::new(address.network_number, &address.mac_address).ok()
    }

    /// The address of the one row of `client`'s discovery table claiming
    /// `instance`, for a caller that names a device by its instance but
    /// needs the address its gate and requests key on. Sends nothing.
    ///
    /// # Errors
    ///
    /// [`Error::AmbiguousDevice`] when two or more rows claim it, and
    /// [`Error::UnknownDevice`] when none does.
    pub async fn claiming<T: TransportPort + 'static>(
        client: &BACnetClient<T>,
        instance: u32,
    ) -> Result<Self, Error> {
        let found = client.get_device(instance).await?;
        found
            .as_ref()
            .and_then(Self::of)
            .ok_or_else(|| Error::UnknownDevice {
                device: instance.to_string(),
            })
    }

    /// The network: 0 for the local one.
    pub fn network(&self) -> u16 {
        self.network
    }

    /// The MAC on that network: the station's own, never its router's.
    pub fn mac(&self) -> &[u8] {
        &self.mac
    }

    /// True on the local network.
    pub fn is_local(&self) -> bool {
        self.network == 0
    }

    /// The address as a link of `style` shows it: as [`Display`](fmt::Display)
    /// writes it, but six local octets on BACnet/SC are a VMAC, written as
    /// hex (`02:00:00:00:00:01`), as the TUI shows them. Both forms read
    /// back.
    pub fn styled(&self, style: AddressStyle) -> String {
        match (style, self.network, self.mac.as_slice()) {
            (AddressStyle::Hex, 0, mac @ [_, _, _, _, _, _]) => format_sc_mac(mac),
            _ => self.to_string(),
        }
    }
}

impl fmt::Display for DeviceAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.network, self.mac.as_slice()) {
            (0, [station]) => write!(f, "{MSTP_TEXT_PREFIX}{station}"),
            // Three or six octets: `new` takes no other local length.
            (0, mac) => f.write_str(&format_mac(mac)),
            // Never `None`: the MAC isn't empty.
            (network, mac) => f.write_str(&routed_station(network, mac).unwrap_or_default()),
        }
    }
}

impl FromStr for DeviceAddress {
    type Err = String;

    /// The target grammar's address forms only: a device instance
    /// (`1234`, `2:1234`) is no address, and nor is a B/IPv6 address
    /// (`[fe80::1]`), whose VMAC only the node there can say.
    fn from_str(text: &str) -> Result<Self, String> {
        match parse_target(text)? {
            Target::Mac(mac) => Self::local(&mac),
            Target::RoutedMac { dnet, mac } => Self::new(dnet, &mac),
            Target::Instance(_) | Target::Routed { .. } => {
                Err(format!("'{text}' names a device instance, not an address"))
            }
            Target::Bip6(_) => Err(format!(
                "'{text}' is a B/IPv6 address: name the node by its VMAC (bip6:0x...)"
            )),
        }
    }
}

impl From<DeviceAddress> for BACnetAddress {
    fn from(address: DeviceAddress) -> Self {
        BACnetAddress {
            network_number: address.network,
            mac_address: address.mac,
        }
    }
}

impl From<&DeviceAddress> for BACnetAddress {
    fn from(address: &DeviceAddress) -> Self {
        BACnetAddress {
            network_number: address.network,
            mac_address: MacAddr::from_slice(&address.mac),
        }
    }
}

impl Serialize for DeviceAddress {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DeviceAddress {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
#[path = "address_tests.rs"]
mod tests;
