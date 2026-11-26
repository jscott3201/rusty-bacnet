//! The target grammar shared by the one-shot commands and the TUI,
//! and the way each transport's MAC addresses are shown so they read back.
//!
//! Resolving a target to something a request can go to is the `bacnet`
//! CLI's (its `destination` module).

use std::net::{Ipv4Addr, SocketAddrV6};

use bacnet_transport::bip6::{
    Bip6Transport, DEFAULT_BACNET6_PORT, VMAC_TEXT_PREFIX, parse_vmac_text,
};
use bacnet_transport::port::TransportPort;
use bacnet_types::data_link::DataLink;
use bacnet_types::error::Error;

use crate::octets::{hex_sep, hex_upper};

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
        /// The target ends in [`ONE_STATION_SUFFIX`]: the operator says the
        /// MAC is one station, though its form is a broadcast or group
        /// address on some links (#2738). Only ever set for a form the
        /// mark lifts ([`check_one_station`]).
        one_station: bool,
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
     mstp:STATION, device instance, dnet:instance, dnet:0xMAC or dnet:0xMAC:one-station";

/// The third field of a routed station, `5:0xFF:one-station`: the operator
/// says the MAC is one station (#2738). The low-level client refuses a
/// confirmed request to a MAC that is a broadcast or group address on some
/// network, as the one octet `0xFF` is on MS/TP and a station on ARCNET;
/// this says which it is, and the request goes. It is written exactly at
/// the MACs the mark lifts, so each address has one spelling.
pub const ONE_STATION_SUFFIX: &str = "one-station";

/// Whether `mac`, a station's MAC on a remote network, is one the
/// [`ONE_STATION_SUFFIX`] lifts (#2738): the one octet `0xFF`, and two
/// octets starting `0x00` or `0xFF` but for `0x0000` and `0xFFFF`. The
/// check the low-level client's unmarked refusal makes, read the other way.
pub fn lifted_by_one_station(mac: &[u8]) -> bool {
    bacnet_client::client::RemoteBroadcast::of(mac, false)
        .is_some_and(bacnet_client::client::RemoteBroadcast::lifted_by_one_station)
}

/// Why `mac` takes no [`ONE_STATION_SUFFIX`], decoded; `Ok` where it
/// does. The mark only lifts, so it is refused wherever it would lift
/// nothing: that keeps the text form canonical.
///
/// # Errors
///
/// A MAC no form of which the client refuses (an ordinary station, or the
/// one octet `0x00`), or a broadcast or group form no mark lifts (two
/// octets `0x0000` and `0xFFFF`, three of `0xFF`, six beginning with four).
pub fn check_one_station(mac: &[u8]) -> Result<(), String> {
    use bacnet_client::client::RemoteBroadcast;
    let hex = hex_upper(mac);
    match RemoteBroadcast::of(mac, false) {
        Some(form) if form.lifted_by_one_station() => Ok(()),
        Some(form) => Err(format!(
            "':{ONE_STATION_SUFFIX}' lifts nothing at MAC 0x{hex}: it is {form}, which no mark \
             makes a station"
        )),
        None => Err(format!(
            "':{ONE_STATION_SUFFIX}' lifts nothing at MAC 0x{hex}: it is an ordinary station \
             without the suffix; write it only for the one octet 0xFF or two octets 0x0001 to \
             0x00FF or 0xFF00 to 0xFFFE, which are otherwise refused as a broadcast or group \
             address"
        )),
    }
}

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
/// - `dnet:0xHEX:one-station`: the same, for a MAC that is one station
///   though it looks like a broadcast or group address (`5:0xFF`, ARCNET
///   station 255; `9:0x0012`, a gateway's two-octet virtual station). Only
///   the MACs the mark lifts take it ([`check_one_station`]), so each
///   address has one spelling; a device instance (`5:1234:one-station`)
///   takes none
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
            .map_err(|error| error.to_string());
    }
    if s.starts_with('[') {
        return parse_bip6_address(s).map(Target::Bip6);
    }
    if let Some(mac) = parse_six_octets(s) {
        return Ok(Target::Mac(mac));
    }

    if let Some((left, right)) = s.split_once(':') {
        if let Ok(dnet) = left.parse::<u16>() {
            // `dnet:0xMAC:one-station`: the third field is the mark.
            let (body, mark) = match right.split_once(':') {
                Some((body, mark)) => (body, Some(mark)),
                None => (right, None),
            };
            let routed =
                body.parse::<u32>().is_ok() || body.starts_with("0x") || body.starts_with("0X");
            // 0 isn't a network number, and 65535 means every network.
            if routed && matches!(dnet, 0 | u16::MAX) {
                return Err(format!(
                    "invalid network {dnet} in routed target '{s}': expected 1 to 65534"
                ));
            }
            // dnet:instance: both parts numeric.
            if let Ok(instance) = body.parse::<u32>() {
                match mark {
                    None => return Ok(Target::Routed { dnet, instance }),
                    Some(ONE_STATION_SUFFIX) => {
                        return Err(format!(
                            "'{s}': ':{ONE_STATION_SUFFIX}' marks a station's MAC, not a device \
                             instance; a device at such an address is named by its address, \
                             {dnet}:0xMAC:{ONE_STATION_SUFFIX}"
                        ));
                    }
                    Some(_) => {}
                }
            }
            if let Some(hex) = body.strip_prefix("0x").or(body.strip_prefix("0X")) {
                let mac = parse_hex(hex).ok_or_else(|| {
                    format!("invalid MAC in routed target '{s}': expected 0x and hex octets")
                })?;
                let one_station = match mark {
                    None => false,
                    Some(ONE_STATION_SUFFIX) => {
                        check_one_station(&mac).map_err(|why| format!("'{s}': {why}"))?;
                        true
                    }
                    Some(other) => {
                        return Err(format!(
                            "invalid suffix ':{other}' in routed target '{s}': the only one is \
                             ':{ONE_STATION_SUFFIX}'"
                        ));
                    }
                };
                return Ok(Target::RoutedMac {
                    dnet,
                    mac,
                    one_station,
                });
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

/// `transport` as the B/IPv6 transport, when it is one or wraps one. The
/// commands are generic over the transport; only B/IPv6 maps addresses to
/// VMACs.
pub fn as_bip6<T: TransportPort>(transport: &T) -> Option<&Bip6Transport> {
    transport.bip6_transport()
}

/// True when `transport` is MS/TP: the only data link the CLI uses whose
/// MACs are one octet.
pub fn is_mstp<T: TransportPort>(transport: &T) -> bool {
    transport.data_link() == Some(DataLink::Mstp)
}

/// Why a link that isn't B/IPv6 refuses a B/IPv6 VMAC. It names no front
/// end's flag or keyword: the CLI, the MCP server and the Python binding
/// all show it, and each adds its own hint where it has one.
pub const NEEDS_BIP6: &str = "only a client on a B/IPv6 link reaches one";

/// Why a link that isn't MS/TP refuses an MS/TP station, as
/// [`NEEDS_BIP6`] does a VMAC.
pub const NEEDS_MSTP: &str = "only a client on an MS/TP link reaches one";

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
        return Err(format!("'{s}' is a B/IPv6 VMAC: {NEEDS_BIP6}"));
    }
    if s.starts_with(MSTP_TEXT_PREFIX) && !mstp {
        return Err(format!("'{s}' is an MS/TP station: {NEEDS_MSTP}"));
    }
    let target = parse_target(s)?;
    if let Target::Mac(mac) = &target {
        check_local_mac(transport, mac, s)?;
    }
    Ok(target)
}

/// Refuse a MAC on the local network that `transport`'s link doesn't
/// address stations by, as [`parse_target_on`] refuses one written for it,
/// with `shown` naming it: B/IPv6 refuses six octets, MS/TP any length but
/// one, and every other link the one octet of an MS/TP station and the
/// three of a B/IPv6 VMAC. `shown` is the address as the caller has it:
/// the text given, or the MAC's octets when it came as octets, so a
/// message never quotes a form nobody wrote.
pub fn check_local_mac<T: TransportPort + 'static>(
    transport: &T,
    mac: &[u8],
    shown: &str,
) -> Result<(), String> {
    let bip6 = as_bip6(transport).is_some();
    let mstp = is_mstp(transport);
    match mac.len() {
        6 if bip6 => Err(format!(
            "'{shown}' is a 6-octet MAC; a B/IPv6 node is addressed by its 3-octet VMAC (bip6:...)"
        )),
        n if mstp && n != 1 => Err(format!(
            "'{shown}' is a {n}-octet MAC; an MS/TP station is addressed by mstp:N"
        )),
        3 if !bip6 => Err(format!(
            "'{shown}' is a 3-octet MAC, a B/IPv6 VMAC: {NEEDS_BIP6}"
        )),
        1 if !mstp => Err(format!(
            "'{shown}' is a 1-octet MAC, an MS/TP station: {NEEDS_MSTP}"
        )),
        _ => Ok(()),
    }
}

/// Why [`bip6_vmac`] found no VMAC.
#[derive(Debug)]
#[non_exhaustive]
pub enum VmacLookup {
    /// The client's link isn't B/IPv6: nothing was asked.
    NotBip6 {
        /// The address given.
        peer: SocketAddrV6,
    },
    /// The node at `peer` didn't say its VMAC.
    NoAnswer {
        /// The address asked.
        peer: SocketAddrV6,
        /// The transport's error.
        error: Error,
    },
}

impl std::fmt::Display for VmacLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotBip6 { peer } => write!(f, "{peer} is a B/IPv6 address: {NEEDS_BIP6}"),
            Self::NoAnswer { peer, error } => {
                write!(f, "could not find the VMAC at {peer}: {error}")
            }
        }
    }
}

impl std::error::Error for VmacLookup {}

/// Find the VMAC of the B/IPv6 node at `peer`, the MAC a command sends to.
///
/// A B/IPv6 node is addressed by its VMAC, not its IPv6 address (Clause
/// U.1.3), so the CLI asks the node once, with a Virtual-Address-Resolution
/// (Clause U.5), unless the transport already knows which VMAC is there.
///
/// # Errors
///
/// [`VmacLookup::NotBip6`] on any transport but B/IPv6, and
/// [`VmacLookup::NoAnswer`] when nothing answers within three seconds.
pub async fn bip6_vmac<T: TransportPort + 'static>(
    transport: &T,
    peer: SocketAddrV6,
) -> Result<Vec<u8>, VmacLookup> {
    let bip6 = as_bip6(transport).ok_or(VmacLookup::NotBip6 { peer })?;
    let vmac = bip6
        .resolve_virtual_address(peer)
        .await
        .map_err(|error| VmacLookup::NoAnswer { peer, error })?;
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
        Target::Bip6(peer)
            if peer.ip().is_unspecified() || peer.ip().is_multicast() || peer.port() == 0 =>
        {
            Err(format!(
                "'{text}' must name a unicast IPv6 station with a nonzero UDP port"
            ))
        }
        Target::Bip6(peer) => bip6_vmac(transport, peer)
            .await
            .map_err(|error| error.to_string()),
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
        transport.data_link().map_or(Self::Bip, Self::of_link)
    }

    /// The style of the addresses on `link`: six octets are `ip:port`
    /// except on BACnet/SC, where they are a VMAC.
    pub fn of_link(link: DataLink) -> Self {
        match link {
            DataLink::Bip6 => Self::Bip6,
            DataLink::Mstp => Self::Mstp,
            DataLink::Sc => Self::Hex,
            _ => Self::Bip,
        }
    }

    /// Format a MAC for display, in a form [`parse_target`] reads back.
    pub fn format(self, mac: &[u8]) -> String {
        match (self, mac) {
            (Self::Mstp, [station]) => format!("{MSTP_TEXT_PREFIX}{station}"),
            (Self::Bip, [_, _, _, _, _, _]) | (Self::Bip6, [_, _, _]) => format_mac(mac),
            _ => hex_sep(mac, ":"),
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
///
/// This names the address and nothing about the one-station mark, which
/// only an operator's say-so can give (#2738): a MAC a router passed on in
/// a capture may be a broadcast. [`routed_target`] writes the mark.
pub fn routed_station(network: u16, mac: &[u8]) -> Option<String> {
    if mac.is_empty() {
        return None;
    }
    Some(format!("{network}:0x{}", hex_upper(mac)))
}

/// A routed station as [`routed_station`] writes it, followed by
/// `:one-station` ([`ONE_STATION_SUFFIX`]) when `one_station` says the
/// address is marked, so [`parse_target`] reads it back as it was.
pub fn routed_target(network: u16, mac: &[u8], one_station: bool) -> Option<String> {
    let station = routed_station(network, mac)?;
    Some(if one_station {
        format!("{station}:{ONE_STATION_SUFFIX}")
    } else {
        station
    })
}

/// Format a MAC address as a target the CLI reads back: a BIP MAC (6 bytes:
/// 4 IP + 2 port) as `ip:port`, and a B/IPv6 VMAC (3 bytes, the only
/// 3-byte MAC the CLI's transports use) as `bip6:0x0004D2`. Falls back to
/// colon-separated hex for other MACs. A routed station is written with its
/// network by [`routed_station`] instead. [`format_sc_mac`] is the same for a
/// BACnet/SC peer, where six octets are a VMAC.
pub fn format_mac(mac: &[u8]) -> String {
    mac_text(mac, true)
}

/// Like [`format_mac`], for a BACnet/SC peer: six octets are a VMAC, not an
/// IPv4 address and port, so they stay colon-separated hex.
pub fn format_sc_mac(mac: &[u8]) -> String {
    mac_text(mac, false)
}

fn mac_text(mac: &[u8], six_is_ip_port: bool) -> String {
    if let (true, [a, b, c, d, hi, lo]) = (six_is_ip_port, mac) {
        let ip = Ipv4Addr::new(*a, *b, *c, *d);
        let port = u16::from_be_bytes([*hi, *lo]);
        format!("{ip}:{port}")
    } else if let Ok(vmac) = <[u8; 3]>::try_from(mac) {
        bacnet_transport::bip6::format_vmac_text(bacnet_transport::bip6::Bip6Vmac::new(vmac))
    } else {
        hex_sep(mac, ":")
    }
}

#[cfg(test)]
#[path = "target_tests.rs"]
mod tests;
