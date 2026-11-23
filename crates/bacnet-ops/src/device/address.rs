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
    AddressStyle, MSTP_TEXT_PREFIX, ONE_STATION_SUFFIX, Target, check_one_station, format_mac,
    format_sc_mac, lifted_by_one_station, parse_target, routed_target,
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
/// A station at an address that is a broadcast or group form on some links
/// takes the suffix `:one-station` (`5:0xFF:one-station`, ARCNET station
/// 255; `9:0x0012:one-station`, a gateway's two-octet virtual station),
/// which [`marked`](Self::marked) builds and nothing else does (#2738).
/// There is no field for the mark: no unmarked address exists at such a MAC, so the
/// mark follows from the MAC ([`is_one_station`](Self::is_one_station)),
/// [`Display`](fmt::Display) writes the suffix there and nowhere else, and
/// every address reads back as it was.
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
    /// octets), and the broadcast and group forms the low-level client
    /// refuses a confirmed request to ([`RemoteBroadcast`]: the MS/TP
    /// broadcast `0xFF`, two octets beginning `0x00` or `0xFF`, three
    /// octets of `0xFF`, and six beginning with four) name no one station,
    /// on this network or a remote one (#2709). A remote address that is
    /// one station at such a MAC (#2738), such as an ARCNET station 255 or
    /// a gateway's two-octet virtual station, is refused here too: only
    /// [`marked`](Self::marked) builds one, so that a discovered device's
    /// address ([`of`](Self::of)) can never invent the mark.
    ///
    /// [`RemoteBroadcast`]: bacnet_client::client::RemoteBroadcast
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
        if let Some(form) = bacnet_client::client::RemoteBroadcast::of(mac, false) {
            // On a remote network the form may be one station's address
            // there, which only the one-station mark lets through (#2738).
            let marked = if network != 0 && form.lifted_by_one_station() {
                format!(
                    "; a station at such an address is written with the one-station mark: \
                     {}",
                    routed_target(network, mac, true).unwrap_or_default()
                )
            } else {
                String::new()
            };
            return Err(format!(
                "MAC 0x{} is {form}, not a station{marked}",
                crate::octets::hex_upper(mac)
            ));
        }
        Ok(Self {
            network,
            mac: MacAddr::from_slice(mac),
        })
    }

    /// The station at `mac` on the remote `network`, marked one station
    /// (#2738): the target grammar's `5:0xFF:one-station`. It is the only
    /// way to an address at a MAC that is a broadcast or group form on some
    /// links, and the mark only lifts, so it takes just the MACs it lifts
    /// ([`check_one_station`]): the one octet `0xFF`, and two octets
    /// starting `0x00` or `0xFF` but for `0x0000` and `0xFFFF`. For a
    /// caller that has the operator's word that the MAC is one station.
    ///
    /// # Errors
    ///
    /// The local network (0) and 65535 (every network), and a MAC the mark
    /// lifts nothing at: an ordinary station, `0x0000`, `0xFFFF`, and the
    /// three- and six-octet forms.
    pub fn marked(network: u16, mac: &[u8]) -> Result<Self, String> {
        if network == 0 {
            return Err(format!(
                "':{ONE_STATION_SUFFIX}' lifts nothing on the local network: a local MAC is \
                 judged by its link"
            ));
        }
        if network == u16::MAX {
            return Err("network 65535 is every network, not a station's".into());
        }
        check_one_station(mac)?;
        Ok(Self {
            network,
            mac: MacAddr::from_slice(mac),
        })
    }

    /// Whether the address is marked one station: a remote station at a MAC
    /// the mark lifts ([`marked`](Self::marked)). No address is at such a
    /// MAC unmarked, so this is a read of the MAC; the typed calls send to
    /// it as a marked [`Station`](bacnet_client::client::Station).
    pub fn is_one_station(&self) -> bool {
        self.network != 0 && lifted_by_one_station(&self.mac)
    }

    /// The target text of a routed row `device` whose address is a MAC the
    /// mark lifts, with the suffix: how an operator who knows it is one
    /// station names it (#2738). `None` for any other row. A device that
    /// answered from such an address has no [`DeviceAddress`] of its own
    /// ([`of`](Self::of)), so a caller that finds an instance's only
    /// claimant there uses this to say how to name it.
    pub fn one_station_form(device: &DiscoveredDevice) -> Option<String> {
        let at = device.address()?;
        if at.network_number == 0 || !lifted_by_one_station(&at.mac_address) {
            return None;
        }
        routed_target(at.network_number, &at.mac_address, true)
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

    /// The address with a station on the network numbered `own` written as
    /// the local station at its MAC: the form the request gates, the device
    /// handles and the write ledger compare. `own` is the client's own
    /// network's number, once it knows it
    /// ([`BACnetClient::network_number`]). A MAC the local network has no
    /// form for stays as it is. The discovery table may still hold a routed
    /// row for such a station for a while (#2572): [`row`](Self::row) finds
    /// either.
    pub fn localized(self, own: Option<u16>) -> Self {
        match own {
            Some(own) if own == self.network => Self::local(&self.mac).unwrap_or(self),
            _ => self,
        }
    }

    /// The row of `client`'s discovery table for the station at this
    /// address, looked up in its [`localized`](Self::localized) form. For a
    /// local station, while the client knows its own network's number, also
    /// a routed row on that network the table hasn't folded into the local
    /// one yet (#2572). Sends nothing.
    pub async fn row<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
    ) -> Option<DiscoveredDevice> {
        let own = client.network_number();
        let at = self.clone().localized(own);
        let row = client.get_device_at(at.network(), at.mac()).await;
        match (row, own) {
            (None, Some(own)) if at.is_local() => client.get_device_at(own, at.mac()).await,
            (row, _) => row,
        }
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
            // Never `None`: the MAC isn't empty. The suffix is written
            // exactly where the MAC is one the mark lifts.
            (network, mac) => {
                f.write_str(&routed_target(network, mac, self.is_one_station()).unwrap_or_default())
            }
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
            Target::RoutedMac {
                dnet,
                mac,
                one_station: false,
            } => Self::new(dnet, &mac),
            Target::RoutedMac {
                dnet,
                mac,
                one_station: true,
            } => Self::marked(dnet, &mac),
            Target::Instance(_) | Target::Routed { .. } => {
                Err(format!("'{text}' names a device instance, not an address"))
            }
            Target::Bip6(_) => Err(format!(
                "'{text}' is a B/IPv6 address: name the node by its VMAC (bip6:0x...)"
            )),
        }
    }
}

impl From<DeviceAddress> for bacnet_client::client::Station {
    /// This link at `mac` for a local address, else the remote station,
    /// marked one station for a MAC the mark lifts ([`DeviceAddress::marked`]),
    /// so the client sends a confirmed request there (#2738).
    fn from(address: DeviceAddress) -> Self {
        let one_station = address.is_one_station();
        match address.network {
            0 => Self::Local { mac: address.mac },
            network => Self::Remote {
                network,
                mac: address.mac,
                one_station,
            },
        }
    }
}

impl From<&DeviceAddress> for bacnet_client::client::Station {
    /// As the owned conversion, for a caller that keeps the address.
    fn from(address: &DeviceAddress) -> Self {
        Self::from(address.clone())
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
