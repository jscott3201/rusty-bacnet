//! The target grammar shared by the one-shot commands and the TUI,
//! and the way each transport's MAC addresses are shown so they read back.
//!
//! Resolving a target to something a request can go to is in
//! [`super::destination`].

use std::any::Any;
use std::net::{Ipv4Addr, SocketAddrV6};

use bacnet_transport::bip6::{
    Bip6Transport, DEFAULT_BACNET6_PORT, VMAC_TEXT_PREFIX, parse_vmac_text,
};
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;

/// A parsed target for a BACnet command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A MAC on this network, as written.
    Mac(Vec<u8>),
    /// A B/IPv6 address. The node there is addressed by its VMAC, which
    /// [`bip6_vmac`] finds.
    Bip6(SocketAddrV6),
    /// A device instance, looked up in the client's discovery table.
    Instance(u32),
    /// A device behind a router: its network and device instance.
    Routed {
        /// Remote network number.
        dnet: u16,
        /// Device instance.
        instance: u32,
    },
    /// A station behind a router: its network and its MAC there.
    RoutedMac {
        /// Remote network number.
        dnet: u16,
        /// The station's MAC on that network.
        mac: Vec<u8>,
    },
}

/// Encode an IPv4 address and port as a 6-byte BIP MAC address.
///
/// The format is 4 bytes of IPv4 address followed by 2 bytes of port in
/// big-endian byte order.
pub fn ip_to_bip_mac(ip: Ipv4Addr, port: u16) -> Vec<u8> {
    let mut mac = Vec::with_capacity(6);
    mac.extend_from_slice(&ip.octets());
    mac.extend_from_slice(&port.to_be_bytes());
    mac
}

/// The forms [`parse_target`] takes, for error messages.
pub const TARGET_FORMS: &str = "IP[:port], a 6-octet MAC or SC VMAC \
     (02:00:00:00:00:01 or 020000000001), bip6:INSTANCE, bip6:0xVMAC, [IPv6][:port], \
     mstp:STATION, device instance, dnet:instance or dnet:0xMAC";

/// The prefix of an MS/TP station's address, `mstp:5` or `mstp:0x05`.
pub const MSTP_TEXT_PREFIX: &str = "mstp:";

/// An MS/TP station's MAC from the text after `mstp:`: 0 to 254, in decimal
/// or as `0x` and hex. 255 is the broadcast address, not a station.
fn parse_mstp_station(text: &str) -> Result<u8, String> {
    // Digits only: the integer parsers would also take a leading `+`.
    let value = match text.strip_prefix("0x").or(text.strip_prefix("0X")) {
        Some(hex) if hex.bytes().all(|b| b.is_ascii_hexdigit()) => u8::from_str_radix(hex, 16).ok(),
        None if text.bytes().all(|b| b.is_ascii_digit()) => text.parse::<u8>().ok(),
        _ => None,
    };
    match value {
        Some(station) if station != bacnet_transport::mstp::codec::BROADCAST_MAC => Ok(station),
        _ => Err(format!(
            "invalid MS/TP station '{MSTP_TEXT_PREFIX}{text}': expected 0 to 254, in decimal or 0xHH"
        )),
    }
}

/// Parse a target string into a [`Target`].
///
/// Supported formats:
/// - `dnet:instance`: a routed device (e.g., `2:1234`)
/// - `dnet:0xHEX`: a routed station by its MAC on that network (e.g.,
///   `2:0x05` for MS/TP station 5)
/// - `ip` or `ip:port`: a direct BIP MAC (e.g., `192.168.1.100` or
///   `192.168.1.100:47808`)
/// - six hex octets, `02:00:00:00:00:01`, `02-00-00-00-00-01` or
///   `020000000001`: a 6-octet MAC as written, such as a BACnet/SC VMAC
/// - `bip6:instance` or `bip6:0xHHHHHH`: a B/IPv6 VMAC, a device instance's
///   (Clause H.7.2) or any three octets, as
///   [`format_vmac_text`](bacnet_transport::bip6::format_vmac_text) prints them
/// - `[ipv6]` or `[ipv6]:port`: a B/IPv6 address (port 47808 when absent),
///   whose VMAC [`bip6_vmac`] finds
/// - `mstp:station` or `mstp:0xHH`: an MS/TP station's 1-octet MAC, 0 to 254
/// - `instance`: a device instance number (e.g., `1234`)
pub fn parse_target(s: &str) -> Result<Target, String> {
    if let Some(station) = s.strip_prefix(MSTP_TEXT_PREFIX) {
        return parse_mstp_station(station).map(|mac| Target::Mac(vec![mac]));
    }
    if s.starts_with(VMAC_TEXT_PREFIX) {
        return parse_vmac_text(s)
            .map(|vmac| Target::Mac(vmac.octets().to_vec()))
            .map_err(|error| match error {
                Error::Encoding(message) | Error::OutOfRange(message) => message,
                other => other.to_string(),
            });
    }
    if s.starts_with('[') {
        return parse_bip6_address(s).map(Target::Bip6);
    }
    if let Some(mac) = parse_six_octets(s) {
        return Ok(Target::Mac(mac));
    }

    if let Some((left, right)) = s.split_once(':') {
        if let Ok(dnet) = left.parse::<u16>() {
            let routed =
                right.parse::<u32>().is_ok() || right.starts_with("0x") || right.starts_with("0X");
            // 0 isn't a network number, and 65535 means every network.
            if routed && matches!(dnet, 0 | u16::MAX) {
                return Err(format!(
                    "invalid network {dnet} in routed target '{s}': expected 1 to 65534"
                ));
            }
            // dnet:instance: both parts numeric.
            if let Ok(instance) = right.parse::<u32>() {
                return Ok(Target::Routed { dnet, instance });
            }
            if let Some(hex) = right.strip_prefix("0x").or(right.strip_prefix("0X")) {
                let mac = parse_hex(hex).ok_or_else(|| {
                    format!("invalid MAC in routed target '{s}': expected 0x and hex octets")
                })?;
                return Ok(Target::RoutedMac { dnet, mac });
            }
        }
        // If the left side is not a network number, it might be an IP:port.
        if let Ok(ip) = left.parse::<Ipv4Addr>() {
            let port = right
                .parse::<u16>()
                .map_err(|_| format!("invalid port in target '{s}'"))?;
            return Ok(Target::Mac(ip_to_bip_mac(ip, port)));
        }
    }

    // Try parsing as bare IP address (no port).
    if let Ok(ip) = s.parse::<Ipv4Addr>() {
        return Ok(Target::Mac(ip_to_bip_mac(ip, 0xBAC0)));
    }

    // Try parsing as plain device instance number.
    if let Ok(instance) = s.parse::<u32>() {
        return Ok(Target::Instance(instance));
    }

    Err(format!("invalid target '{s}': expected {TARGET_FORMS}"))
}

/// Six hex octets: twelve digits, or six pairs split by `:` or `-`. Twelve
/// characters can't be an IPv4 address or a device instance, so the forms
/// never collide.
fn parse_six_octets(s: &str) -> Option<Vec<u8>> {
    if s.len() == 12 {
        return parse_hex(s);
    }
    let separator = if s.contains(':') { ':' } else { '-' };
    let parts: Vec<&str> = s.split(separator).collect();
    if parts.len() != 6 || parts.iter().any(|part| part.len() != 2) {
        return None;
    }
    parts
        .iter()
        .map(|part| u8::from_str_radix(part, 16).ok())
        .collect()
}

/// An even, nonzero number of hex digits as octets.
fn parse_hex(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() || !s.len().is_multiple_of(2) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&s[at..at + 2], 16).ok())
        .collect()
}

/// `[ipv6]` or `[ipv6]:port`, port 47808 when absent. A link-local address
/// may carry its interface as a numeric zone, `[fe80::1%3]`.
fn parse_bip6_address(s: &str) -> Result<SocketAddrV6, String> {
    let with_port;
    let text = if s.ends_with(']') {
        with_port = format!("{s}:{DEFAULT_BACNET6_PORT}");
        &with_port
    } else {
        s
    };
    text.parse()
        .map_err(|_| format!("invalid B/IPv6 address '{s}': expected [IPv6] or [IPv6]:port"))
}

/// `transport` as the B/IPv6 transport, when it is one. The commands are
/// generic over the transport; only B/IPv6 maps addresses to VMACs.
pub fn as_bip6<T: TransportPort + 'static>(transport: &T) -> Option<&Bip6Transport> {
    (transport as &dyn Any).downcast_ref()
}

/// True when `transport` is the BACnet/SC transport the CLI builds.
fn is_sc<T: TransportPort + 'static>(transport: &T) -> bool {
    #[cfg(feature = "sc-tls")]
    {
        (transport as &dyn Any)
            .is::<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>>()
    }
    #[cfg(not(feature = "sc-tls"))]
    {
        let _ = transport;
        false
    }
}

/// True when `transport` is MS/TP: the only data link the CLI uses whose
/// MACs are one octet.
pub fn is_mstp<T: TransportPort + 'static>(transport: &T) -> bool {
    transport.local_mac().len() == 1
}

/// [`parse_target`] for a command on `transport`. A `bip6:` VMAC names a
/// B/IPv6 node and `mstp:` an MS/TP station, so any other transport refuses
/// them here, rather than hand its link a MAC it can't send to; B/IPv6
/// refuses a 6-octet MAC, and MS/TP any MAC but a 1-octet one.
pub fn parse_target_on<T: TransportPort + 'static>(
    transport: &T,
    s: &str,
) -> Result<Target, String> {
    let bip6 = as_bip6(transport).is_some();
    let mstp = is_mstp(transport);
    if s.starts_with(VMAC_TEXT_PREFIX) && !bip6 {
        return Err(format!(
            "'{s}' is a B/IPv6 VMAC: bip6: addresses need the --ipv6 transport"
        ));
    }
    if s.starts_with(MSTP_TEXT_PREFIX) && !mstp {
        return Err(format!(
            "'{s}' is an MS/TP station: mstp: addresses need the --mstp transport"
        ));
    }
    let target = parse_target(s)?;
    if let Target::Mac(mac) = &target {
        if bip6 && mac.len() == 6 {
            return Err(format!(
                "'{s}' is a 6-octet MAC; a B/IPv6 node is addressed by its 3-octet VMAC (bip6:...)"
            ));
        }
        if mstp && mac.len() != 1 {
            return Err(format!(
                "'{s}' is a {}-octet MAC; an MS/TP station is addressed by mstp:N",
                mac.len()
            ));
        }
    }
    Ok(target)
}

/// Find the VMAC of the B/IPv6 node at `peer`, the MAC a command sends to.
///
/// A B/IPv6 node is addressed by its VMAC, not its IPv6 address (Clause
/// U.1.3), so the CLI asks the node once, with a Virtual-Address-Resolution
/// (Clause U.5), unless the transport already knows which VMAC is there.
/// Fails on any transport but B/IPv6, and when nothing answers within three
/// seconds.
pub async fn bip6_vmac<T: TransportPort + 'static>(
    transport: &T,
    peer: SocketAddrV6,
) -> Result<Vec<u8>, String> {
    let bip6 = as_bip6(transport)
        .ok_or_else(|| format!("{peer} is a B/IPv6 address: use it with --ipv6"))?;
    let vmac = bip6
        .resolve_virtual_address(peer)
        .await
        .map_err(|e| format!("could not find the VMAC at {peer}: {e}"))?;
    Ok(vmac.octets().to_vec())
}

/// Why a directed Who-Is refuses a device instance or routed target.
pub const TARGET_NOT_AN_ADDRESS: &str =
    "--target requires an address, not a device instance or routed address";

/// The MAC of a target that must be an address, such as a directed
/// Who-Is's: a MAC as written, or the VMAC [`bip6_vmac`] finds for a
/// B/IPv6 address. A device instance or routed target fails with `refusal`.
pub async fn address_mac<T: TransportPort + 'static>(
    transport: &T,
    text: &str,
    refusal: &str,
) -> Result<Vec<u8>, String> {
    match parse_target_on(transport, text)? {
        Target::Mac(mac) => Ok(mac),
        Target::Bip6(peer) => bip6_vmac(transport, peer).await,
        Target::Instance(_) | Target::Routed { .. } | Target::RoutedMac { .. } => {
            Err(refusal.to_string())
        }
    }
}

/// How a transport's MAC addresses are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressStyle {
    /// 4-byte IPv4 address and 2-byte port.
    Bip,
    /// 3-byte B/IPv6 VMAC, as `bip6:0x0004D2` (Clause U.1.3).
    Bip6,
    /// Hex octets (BACnet/SC VMACs and anything else).
    Hex,
    /// A 1-byte MS/TP station, as `mstp:5`.
    Mstp,
}

impl AddressStyle {
    /// The style of `transport`'s addresses.
    pub fn of<T: TransportPort + 'static>(transport: &T) -> Self {
        if as_bip6(transport).is_some() {
            Self::Bip6
        } else if is_mstp(transport) {
            Self::Mstp
        } else if is_sc(transport) {
            Self::Hex
        } else {
            Self::Bip
        }
    }

    /// Format a MAC for display, in a form [`parse_target`] reads back.
    pub fn format(self, mac: &[u8]) -> String {
        match (self, mac) {
            (Self::Mstp, [station]) => format!("{MSTP_TEXT_PREFIX}{station}"),
            (Self::Bip, [_, _, _, _, _, _]) | (Self::Bip6, [_, _, _]) => format_mac(mac),
            _ => hex(mac),
        }
    }

    /// Where a device or a reply is, as the TUI's tables show it. For one
    /// a router passed on, from `routed` (its network and its MAC there):
    /// the routed station as [`routed_station`] writes it, and the router,
    /// the link peer `mac`, in this style. Otherwise, or when the routed
    /// MAC is empty and so names no station, `mac` in this style and no
    /// router.
    pub fn locate(self, mac: &[u8], routed: Option<(u16, &[u8])>) -> (String, Option<String>) {
        match routed.and_then(|(network, station)| routed_station(network, station)) {
            Some(station) => (station, Some(self.format(mac))),
            None => (self.format(mac), None),
        }
    }
}

/// A routed station for display, `NET:0xMAC` with the MAC in upper-case
/// hex (`2:0x7F000001C335`, an MS/TP station `2:0x05`): a station is
/// written in the target syntax, so [`parse_target`] reads it back. An
/// empty MAC names no station (it is a broadcast on that network), so it
/// has no target form and gives `None`; the capture, which shows routed
/// broadcasts, writes those itself.
pub fn routed_station(network: u16, mac: &[u8]) -> Option<String> {
    if mac.is_empty() {
        return None;
    }
    let mut out = format!("{network}:0x");
    for b in mac {
        out.push_str(&format!("{b:02X}"));
    }
    Some(out)
}

/// Format a MAC address as a target the CLI reads back: a BIP MAC (6 bytes:
/// 4 IP + 2 port) as `ip:port`, and a B/IPv6 VMAC (3 bytes, the only
/// 3-byte MAC the CLI's transports use) as `bip6:0x0004D2`. Falls back to
/// hex display for other MACs. A routed station is written with its network
/// by [`routed_station`] instead.
pub fn format_mac(mac: &[u8]) -> String {
    if mac.len() == 6 {
        let ip = Ipv4Addr::new(mac[0], mac[1], mac[2], mac[3]);
        let port = u16::from_be_bytes([mac[4], mac[5]]);
        format!("{ip}:{port}")
    } else if let Ok(vmac) = <[u8; 3]>::try_from(mac) {
        bacnet_transport::bip6::format_vmac_text(bacnet_transport::bip6::Bip6Vmac::new(vmac))
    } else {
        mac.iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":")
    }
}

/// Colon-separated lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(':');
        }
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
#[path = "target_tests.rs"]
mod tests;
