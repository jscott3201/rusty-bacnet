//! BACnet/IPv6 BVLC codec per ASHRAE 135-2020 Annex U.
//!
//! Frame format: type(1) + function(1) + length(2) + source-vmac(3) + payload
//! Multicast groups: FF02::BAC0 (link-local), FF05::BAC0 (site-local)

use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{BufMut, Bytes, BytesMut};
use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::net::{Ipv6Addr, SocketAddrV6};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::port::{ReceivedNpdu, TransportPort};

/// BVLC type byte for BACnet/IPv6 (Annex U).
pub const BVLC6_TYPE: u8 = 0x82;

/// BIP6 virtual MAC address: 3 bytes per Annex U.2.
pub type Bip6Vmac = [u8; 3];

/// Minimum BVLC-IPv6 header length: type(1) + function(1) + length(2) + source-vmac(3).
pub const BVLC6_HEADER_LENGTH: usize = 7;

/// BVLC-IPv6 unicast header length: type(1) + function(1) + length(2) + source-vmac(3) + dest-vmac(3).
pub const BVLC6_UNICAST_HEADER_LENGTH: usize = 10;

/// Maximum number of VMAC collision resolution retries before giving up (Annex U.5).
pub const MAX_VMAC_RETRIES: u32 = 3;

/// BVLC-IPv6 function codes per Annex U.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bvlc6Function {
    /// BVLC-Result (0x00).
    Result,
    /// Original-Unicast-NPDU (0x01).
    OriginalUnicast,
    /// Original-Broadcast-NPDU (0x02).
    OriginalBroadcast,
    /// Address-Resolution (0x03).
    AddressResolution,
    /// Forwarded-Address-Resolution (0x04).
    ForwardedAddressResolution,
    /// Address-Resolution-Ack (0x05).
    AddressResolutionAck,
    /// Virtual-Address-Resolution (0x06).
    VirtualAddressResolution,
    /// Virtual-Address-Resolution-Ack (0x07).
    VirtualAddressResolutionAck,
    /// Forwarded-NPDU (0x08).
    ForwardedNpdu,
    /// Register-Foreign-Device (0x09).
    RegisterForeignDevice,
    /// Delete-Foreign-Device-Table-Entry (0x0A).
    DeleteForeignDeviceEntry,
    /// Distribute-Broadcast-To-Network (0x0C).
    DistributeBroadcastToNetwork,
    /// Unrecognized function code.
    Unknown(u8),
}

impl Bvlc6Function {
    /// Convert a wire byte to a `Bvlc6Function`.
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            0x00 => Self::Result,
            0x01 => Self::OriginalUnicast,
            0x02 => Self::OriginalBroadcast,
            0x03 => Self::AddressResolution,
            0x04 => Self::ForwardedAddressResolution,
            0x05 => Self::AddressResolutionAck,
            0x06 => Self::VirtualAddressResolution,
            0x07 => Self::VirtualAddressResolutionAck,
            0x08 => Self::ForwardedNpdu,
            0x09 => Self::RegisterForeignDevice,
            0x0A => Self::DeleteForeignDeviceEntry,
            0x0C => Self::DistributeBroadcastToNetwork,
            other => Self::Unknown(other),
        }
    }

    /// Convert a `Bvlc6Function` to its wire byte.
    pub fn to_byte(self) -> u8 {
        match self {
            Self::Result => 0x00,
            Self::OriginalUnicast => 0x01,
            Self::OriginalBroadcast => 0x02,
            Self::AddressResolution => 0x03,
            Self::ForwardedAddressResolution => 0x04,
            Self::AddressResolutionAck => 0x05,
            Self::VirtualAddressResolution => 0x06,
            Self::VirtualAddressResolutionAck => 0x07,
            Self::ForwardedNpdu => 0x08,
            Self::RegisterForeignDevice => 0x09,
            Self::DeleteForeignDeviceEntry => 0x0A,
            Self::DistributeBroadcastToNetwork => 0x0C,
            Self::Unknown(b) => b,
        }
    }
}

/// A decoded BVLC-IPv6 frame.
#[derive(Debug, Clone)]
pub struct Bvlc6Frame {
    /// BVLC-IPv6 function code.
    pub function: Bvlc6Function,
    /// Source virtual MAC address (3 bytes).
    pub source_vmac: Bip6Vmac,
    /// Destination virtual MAC address (3 bytes, present in unicast only).
    pub destination_vmac: Option<Bip6Vmac>,
    /// Payload after the BVLC-IPv6 header (typically NPDU bytes).
    pub payload: Bytes,
}

/// Encode a BVLC-IPv6 frame into a buffer.
pub fn encode_bvlc6(
    buf: &mut BytesMut,
    function: Bvlc6Function,
    source_vmac: &Bip6Vmac,
    npdu: &[u8],
) -> Result<(), Error> {
    let total_length = BVLC6_HEADER_LENGTH + npdu.len();
    if total_length > u16::MAX as usize {
        return Err(Error::Encoding(format!(
            "BVLC6 frame length {total_length} exceeds 16-bit BVLC length field"
        )));
    }
    buf.reserve(total_length);
    buf.put_u8(BVLC6_TYPE);
    buf.put_u8(function.to_byte());
    buf.put_u16(total_length as u16);
    buf.put_slice(source_vmac);
    buf.put_slice(npdu);
    Ok(())
}

/// Decode a BVLC-IPv6 frame from raw bytes.
pub fn decode_bvlc6(data: &[u8]) -> Result<Bvlc6Frame, Error> {
    if data.len() < BVLC6_HEADER_LENGTH {
        return Err(Error::decoding(
            0,
            format!(
                "BVLC6 frame too short: need {} bytes, have {}",
                BVLC6_HEADER_LENGTH,
                data.len()
            ),
        ));
    }

    if data[0] != BVLC6_TYPE {
        return Err(Error::decoding(
            0,
            format!("BVLC6 expected type 0x82, got 0x{:02X}", data[0]),
        ));
    }

    let function = Bvlc6Function::from_byte(data[1]);
    let length = u16::from_be_bytes([data[2], data[3]]) as usize;

    if length < BVLC6_HEADER_LENGTH {
        return Err(Error::decoding(2, "BVLC6 length less than header size"));
    }
    if length > data.len() {
        return Err(Error::decoding(
            2,
            format!("BVLC6 length {} exceeds data length {}", length, data.len()),
        ));
    }

    let mut source_vmac = [0u8; 3];
    source_vmac.copy_from_slice(&data[4..7]);

    // These message types include a 3-byte destination/target VMAC after the source VMAC
    let has_dest_vmac = matches!(
        function,
        Bvlc6Function::OriginalUnicast
            | Bvlc6Function::AddressResolution
            | Bvlc6Function::AddressResolutionAck
            | Bvlc6Function::VirtualAddressResolutionAck
    );

    let (destination_vmac, payload_start) = if has_dest_vmac {
        if length < BVLC6_UNICAST_HEADER_LENGTH {
            return Err(Error::decoding(
                7,
                "BVLC6 frame too short for destination VMAC",
            ));
        }
        let mut dest = [0u8; 3];
        dest.copy_from_slice(&data[7..10]);
        (Some(dest), BVLC6_UNICAST_HEADER_LENGTH)
    } else {
        (None, BVLC6_HEADER_LENGTH)
    };

    let payload = Bytes::copy_from_slice(&data[payload_start..length]);

    Ok(Bvlc6Frame {
        function,
        source_vmac,
        destination_vmac,
        payload,
    })
}

/// Encode a BVLC-IPv6 Original-Unicast-NPDU frame.
pub fn encode_bvlc6_original_unicast(
    buf: &mut BytesMut,
    source_vmac: &Bip6Vmac,
    dest_vmac: &Bip6Vmac,
    npdu: &[u8],
) -> Result<(), Error> {
    let total_length = BVLC6_UNICAST_HEADER_LENGTH + npdu.len();
    if total_length > u16::MAX as usize {
        return Err(Error::Encoding(format!(
            "BVLC6 Original-Unicast-NPDU length {total_length} exceeds 16-bit BVLC length field"
        )));
    }
    buf.reserve(total_length);
    buf.put_u8(BVLC6_TYPE);
    buf.put_u8(Bvlc6Function::OriginalUnicast.to_byte());
    buf.put_u16(total_length as u16);
    buf.put_slice(source_vmac);
    buf.put_slice(dest_vmac);
    buf.put_slice(npdu);
    Ok(())
}

/// Encode a BVLC-IPv6 Original-Broadcast-NPDU frame.
pub fn encode_bvlc6_original_broadcast(
    buf: &mut BytesMut,
    source_vmac: &Bip6Vmac,
    npdu: &[u8],
) -> Result<(), Error> {
    encode_bvlc6(buf, Bvlc6Function::OriginalBroadcast, source_vmac, npdu)
}

/// Encode a BVLC-IPv6 Virtual-Address-Resolution frame (7 bytes, no payload).
///
/// Per spec Clause U.2.7: type(1) + function(1) + length(2) + source_vmac(3) = 7.
pub fn encode_virtual_address_resolution(source_vmac: &Bip6Vmac) -> BytesMut {
    let mut buf = BytesMut::with_capacity(BVLC6_HEADER_LENGTH);
    encode_bvlc6(
        &mut buf,
        Bvlc6Function::VirtualAddressResolution,
        source_vmac,
        &[], // no payload
    )
    .expect("empty Virtual-Address-Resolution frame fits in BVLC6 length field");
    buf
}

/// Encode a BVLC-IPv6 Virtual-Address-Resolution-Ack frame (10 bytes).
///
/// Per spec Clause U.2.7A: includes the requester's VMAC as destination.
/// type(1) + function(1) + length(2) + source_vmac(3) + dest_vmac(3) = 10.
pub fn encode_virtual_address_resolution_ack(
    source_vmac: &Bip6Vmac,
    dest_vmac: &Bip6Vmac,
) -> BytesMut {
    let mut buf = BytesMut::with_capacity(BVLC6_UNICAST_HEADER_LENGTH);
    buf.put_u8(BVLC6_TYPE);
    buf.put_u8(Bvlc6Function::VirtualAddressResolutionAck.to_byte());
    buf.put_u16(BVLC6_UNICAST_HEADER_LENGTH as u16);
    buf.put_slice(source_vmac);
    buf.put_slice(dest_vmac);
    buf
}

/// Encode a BVLC-IPv6 Address-Resolution frame (10 bytes).
///
/// Per spec Clause U.2.4: includes the target VMAC to resolve.
pub fn encode_address_resolution(source_vmac: &Bip6Vmac, target_vmac: &Bip6Vmac) -> BytesMut {
    let mut buf = BytesMut::with_capacity(BVLC6_UNICAST_HEADER_LENGTH);
    buf.put_u8(BVLC6_TYPE);
    buf.put_u8(Bvlc6Function::AddressResolution.to_byte());
    buf.put_u16(BVLC6_UNICAST_HEADER_LENGTH as u16);
    buf.put_slice(source_vmac);
    buf.put_slice(target_vmac);
    buf
}

/// Encode a BVLC-IPv6 Address-Resolution-Ack frame (10 bytes).
///
/// Per spec Clause U.2.5: includes the requester's VMAC as destination.
pub fn encode_address_resolution_ack(source_vmac: &Bip6Vmac, dest_vmac: &Bip6Vmac) -> BytesMut {
    let mut buf = BytesMut::with_capacity(BVLC6_UNICAST_HEADER_LENGTH);
    buf.put_u8(BVLC6_TYPE);
    buf.put_u8(Bvlc6Function::AddressResolutionAck.to_byte());
    buf.put_u16(BVLC6_UNICAST_HEADER_LENGTH as u16);
    buf.put_slice(source_vmac);
    buf.put_slice(dest_vmac);
    buf
}

/// Extract the NPDU from a ForwardedNpdu payload.
///
/// ForwardedNpdu payload layout:
///   Original-Source-Virtual-Address(3) + Original-Source-B/IPv6-Address(18) + NPDU.
/// Returns the originating VMAC, originating B/IPv6 address, and NPDU bytes.
pub fn decode_forwarded_npdu_payload(
    payload: &[u8],
) -> Result<(Bip6Vmac, SocketAddrV6, &[u8]), Error> {
    if payload.len() < 21 {
        return Err(Error::decoding(
            0,
            format!(
                "ForwardedNpdu payload too short: need at least 21 bytes, have {}",
                payload.len()
            ),
        ));
    }
    let mut originating_vmac = [0u8; 3];
    originating_vmac.copy_from_slice(&payload[..3]);

    let mut ipv6_bytes = [0u8; 16];
    ipv6_bytes.copy_from_slice(&payload[3..19]);
    let ipv6_addr = Ipv6Addr::from(ipv6_bytes);
    let port = u16::from_be_bytes([payload[19], payload[20]]);
    let source_addr = SocketAddrV6::new(ipv6_addr, port, 0, 0);

    Ok((originating_vmac, source_addr, &payload[21..]))
}

/// BACnet/IPv6 multicast group (link-local): FF02::BAC0.
pub const BACNET_IPV6_MULTICAST_LINK_LOCAL: Ipv6Addr =
    Ipv6Addr::new(0xFF02, 0, 0, 0, 0, 0, 0, 0xBAC0);

/// BACnet/IPv6 multicast group (site-local): FF05::BAC0.
pub const BACNET_IPV6_MULTICAST_SITE_LOCAL: Ipv6Addr =
    Ipv6Addr::new(0xFF05, 0, 0, 0, 0, 0, 0, 0xBAC0);

/// BACnet/IPv6 multicast group (organization-local): FF08::BAC0.
pub const BACNET_IPV6_MULTICAST_ORG_LOCAL: Ipv6Addr =
    Ipv6Addr::new(0xFF08, 0, 0, 0, 0, 0, 0, 0xBAC0);

/// BACnet/IPv6 multicast group -- alias for link-local (backward compatibility).
pub const BACNET_IPV6_MULTICAST: Ipv6Addr = BACNET_IPV6_MULTICAST_LINK_LOCAL;

/// Default BACnet/IPv6 port (same as BIP: 0xBAC0 = 47808).
pub const DEFAULT_BACNET6_PORT: u16 = 0xBAC0;

/// Encode an IPv6 address + port into an 18-byte MAC.
///
/// Format: `[IPv6 address (16 bytes)][port (2 bytes big-endian)]`
pub fn encode_bip6_mac(ip: Ipv6Addr, port: u16) -> [u8; 18] {
    let mut mac = [0u8; 18];
    mac[..16].copy_from_slice(&ip.octets());
    mac[16..18].copy_from_slice(&port.to_be_bytes());
    mac
}

/// Decode an 18-byte MAC into an IPv6 address + port.
pub fn decode_bip6_mac(mac: &[u8]) -> Result<(Ipv6Addr, u16), Error> {
    if mac.len() != 18 {
        return Err(Error::decoding(
            0,
            format!("BIP6 MAC must be 18 bytes, got {}", mac.len()),
        ));
    }
    let mut ip_bytes = [0u8; 16];
    ip_bytes.copy_from_slice(&mac[..16]);
    let ip = Ipv6Addr::from(ip_bytes);
    let port = u16::from_be_bytes([mac[16], mac[17]]);
    Ok((ip, port))
}

/// VMAC-to-address mapping table per Clause U.5.
/// Updated from incoming frames (learn-on-receive) and AR exchanges.
#[derive(Debug, Clone)]
struct VmacTable {
    entries: Arc<tokio::sync::RwLock<HashMap<Bip6Vmac, SocketAddrV6>>>,
}

impl VmacTable {
    fn new() -> Self {
        Self {
            entries: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    /// Learn a VMAC-to-address mapping from an incoming frame.
    async fn learn(&self, vmac: Bip6Vmac, addr: SocketAddrV6) {
        self.entries.write().await.insert(vmac, addr);
    }

    /// Look up the B/IPv6 address for a VMAC.
    #[allow(dead_code)] // used by AR exchange (future)
    async fn lookup(&self, vmac: &Bip6Vmac) -> Option<SocketAddrV6> {
        self.entries.read().await.get(vmac).copied()
    }

    /// Reverse lookup: find the VMAC for a B/IPv6 address.
    async fn lookup_by_addr(&self, addr: &SocketAddrV6) -> Option<Bip6Vmac> {
        let entries = self.entries.read().await;
        entries
            .iter()
            .find(|(_, a)| *a == addr)
            .map(|(vmac, _)| *vmac)
    }
}

/// BACnet/IPv6 transport over UDP (Annex U).
pub struct Bip6Transport {
    interface: Ipv6Addr,
    port: u16,
    device_instance: Option<u32>,
    local_mac: [u8; 18],
    source_vmac: Bip6Vmac,
    socket: Option<Arc<UdpSocket>>,
    recv_task: Option<JoinHandle<()>>,
    /// VMAC address table (Clause U.5).
    vmac_table: VmacTable,
    /// Broadcast scope for send_broadcast.
    broadcast_scope: Bip6BroadcastScope,
    /// Foreign device BBMD configuration (optional).
    foreign_device: Option<Bip6ForeignDeviceConfig>,
    /// Foreign device re-registration task handle.
    registration_task: Option<JoinHandle<()>>,
}

/// Configuration for BIPv6 foreign device registration.
#[derive(Debug, Clone)]
pub struct Bip6ForeignDeviceConfig {
    /// BBMD IPv6 address to register with.
    pub bbmd_ip: Ipv6Addr,
    /// BBMD port.
    pub bbmd_port: u16,
    /// Time-to-live in seconds.
    pub ttl: u16,
}

/// IPv6 multicast scope for BACnet broadcasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bip6BroadcastScope {
    /// FF02::BAC0 — link-local (single link only)
    LinkLocal,
    /// FF05::BAC0 — site-local (building/campus, default)
    SiteLocal,
    /// FF08::BAC0 — organization-local
    OrganizationLocal,
}

impl Bip6BroadcastScope {
    fn multicast_addr(&self) -> Ipv6Addr {
        match self {
            Self::LinkLocal => BACNET_IPV6_MULTICAST_LINK_LOCAL,
            Self::SiteLocal => BACNET_IPV6_MULTICAST_SITE_LOCAL,
            Self::OrganizationLocal => BACNET_IPV6_MULTICAST_ORG_LOCAL,
        }
    }
}

impl Bip6Transport {
    /// Create a new BACnet/IPv6 transport.
    ///
    /// - `interface`: Local IPv6 address to bind (use `::` for all interfaces)
    /// - `port`: UDP port (default 47808 / 0xBAC0)
    /// - `device_instance`: If `Some(id)`, derive the 3-byte VMAC from the
    ///   lower 22 bits of the device instance (per Clause H.7.2). Otherwise the
    ///   VMAC is derived from the local IPv6 address + port.
    pub fn new(interface: Ipv6Addr, port: u16, device_instance: Option<u32>) -> Self {
        Self {
            interface,
            port,
            device_instance,
            local_mac: [0; 18],
            source_vmac: [0; 3],
            socket: None,
            recv_task: None,
            vmac_table: VmacTable::new(),
            broadcast_scope: Bip6BroadcastScope::SiteLocal,
            foreign_device: None,
            registration_task: None,
        }
    }

    /// Set the broadcast scope for send_broadcast.
    pub fn set_broadcast_scope(&mut self, scope: Bip6BroadcastScope) {
        self.broadcast_scope = scope;
    }

    /// Configure this transport as a foreign device.
    /// Must be called before `start()`.
    pub fn register_as_foreign_device(&mut self, config: Bip6ForeignDeviceConfig) {
        self.foreign_device = Some(config);
    }
}

/// Derive a 3-byte VMAC from the lower 24 bits of a device instance (Annex U.5).
/// Send a Register-Foreign-Device message (Clause U.4.5).
async fn send_register_foreign_device_v6(
    socket: &UdpSocket,
    bbmd_addr: SocketAddrV6,
    ttl: u16,
    source_vmac: &Bip6Vmac,
) {
    let mut buf = BytesMut::with_capacity(BVLC6_HEADER_LENGTH + 2);
    if let Err(e) = encode_bvlc6(
        &mut buf,
        Bvlc6Function::RegisterForeignDevice,
        source_vmac,
        &ttl.to_be_bytes(),
    ) {
        warn!(error = %e, "BIP6: failed to encode Register-Foreign-Device");
        return;
    }
    if let Err(e) = socket.send_to(&buf, bbmd_addr).await {
        warn!(error = %e, "BIP6: failed to send Register-Foreign-Device");
    } else {
        debug!(bbmd = %bbmd_addr, ttl = ttl, "BIP6: sent Register-Foreign-Device");
    }
}

fn derive_vmac_from_device_instance(device_instance: u32) -> Bip6Vmac {
    // Mask to 22 bits per Clause H.7.2 (BACnet device instances are 22-bit)
    let masked = device_instance & 0x3FFFFF;
    let bytes = masked.to_be_bytes(); // [b0, b1, b2, b3]
    [bytes[1], bytes[2], bytes[3]]
}

/// Generate a random 3-byte VMAC for collision resolution (Annex U.5).
pub fn generate_random_vmac() -> Bip6Vmac {
    let h = RandomState::new().build_hasher().finish().to_ne_bytes();
    [h[0], h[1], h[2]]
}

/// Derive a 3-byte VMAC by XOR-folding 16-byte IPv6 + 2-byte port.
fn derive_vmac_from_addr(addr: &SocketAddrV6) -> Bip6Vmac {
    let octets = addr.ip().octets();
    let port_bytes = addr.port().to_be_bytes();
    let mut vmac = [0u8; 3];
    for (i, &b) in octets.iter().chain(port_bytes.iter()).enumerate() {
        vmac[i % 3] ^= b;
    }
    vmac
}

/// Resolve the local IPv6 address by connecting a UDP socket to a link-local
/// multicast address and reading back the local address. No packets are sent.
/// Uses ff02::1 (all-nodes link-local) to avoid any external DNS dependency.
fn resolve_local_ipv6() -> Option<Ipv6Addr> {
    let socket = std::net::UdpSocket::bind("[::]:0").ok()?;
    // Connect to link-local all-nodes multicast on BACnet port -- no packets sent.
    match socket.connect("[ff02::1]:47808") {
        Ok(()) => {}
        Err(_) => {
            warn!("Could not resolve local IPv6 address via ff02::1, falling back to localhost");
            return None;
        }
    }
    match socket.local_addr().ok()? {
        std::net::SocketAddr::V6(v6) => Some(*v6.ip()),
        _ => None,
    }
}

/// Best-effort resolution of IPv6 interface index for the given address.
/// Returns `None` if the interface cannot be determined.
#[allow(unsafe_code)]
fn resolve_interface_index(addr: &Ipv6Addr) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::ffi::CStr;

        /// RAII guard for `getifaddrs` that calls `freeifaddrs` on drop.
        struct IfAddrsGuard(*mut libc::ifaddrs);
        impl Drop for IfAddrsGuard {
            fn drop(&mut self) {
                // SAFETY: `self.0` was returned by `getifaddrs` and stored without modification;
                // `freeifaddrs` is the matching deallocator and is called exactly once on drop.
                unsafe { libc::freeifaddrs(self.0) }
            }
        }

        // SAFETY: this block performs the `getifaddrs`/walk/family-dispatch dance. The
        // returned linked list is owned by `_guard` (calls `freeifaddrs` on drop). All
        // pointer dereferences of `cursor`/`ifa.ifa_addr` are gated on null checks; address
        // family bytes are read after confirming `family == AF_INET6`. `CStr::from_ptr`
        // requires NUL-terminated strings, which the kernel guarantees for `ifa_name`.
        unsafe {
            let mut ifaddrs: *mut libc::ifaddrs = std::ptr::null_mut();
            if libc::getifaddrs(&mut ifaddrs) != 0 {
                return None;
            }
            let _guard = IfAddrsGuard(ifaddrs);
            let mut cursor = ifaddrs;
            while !cursor.is_null() {
                let ifa = &*cursor;
                if !ifa.ifa_addr.is_null() {
                    let family = (*ifa.ifa_addr).sa_family as i32;
                    if family == libc::AF_INET6 {
                        let sockaddr6 = &*(ifa.ifa_addr as *const libc::sockaddr_in6);
                        let ifa_ip = Ipv6Addr::from(sockaddr6.sin6_addr.s6_addr);
                        if ifa_ip == *addr {
                            let name = CStr::from_ptr(ifa.ifa_name);
                            let idx = libc::if_nametoindex(name.as_ptr());
                            if idx != 0 {
                                return Some(idx);
                            }
                        }
                    }
                }
                cursor = ifa.ifa_next;
            }
            None
        }
    }
    #[cfg(not(unix))]
    {
        let _ = addr;
        None
    }
}

impl TransportPort for Bip6Transport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        if self.recv_task.is_some() {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "BIP6 transport already started",
            )));
        }

        let socket2_sock = socket2::Socket::new(
            socket2::Domain::IPV6,
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )
        .map_err(Error::Transport)?;

        socket2_sock.set_only_v6(true).map_err(Error::Transport)?;
        socket2_sock
            .set_reuse_address(true)
            .map_err(Error::Transport)?;
        socket2_sock
            .set_nonblocking(true)
            .map_err(Error::Transport)?;
        socket2_sock
            .set_multicast_loop_v6(true)
            .map_err(Error::Transport)?;

        let bind_addr = SocketAddrV6::new(self.interface, self.port, 0, 0);
        socket2_sock
            .bind(&bind_addr.into())
            .map_err(Error::Transport)?;

        let std_socket: std::net::UdpSocket = socket2_sock.into();
        let socket = UdpSocket::from_std(std_socket).map_err(Error::Transport)?;

        let local_addr = socket.local_addr().map_err(Error::Transport)?;
        let local_port = local_addr.port();
        self.port = local_port;

        let local_ip = if self.interface.is_unspecified() {
            resolve_local_ipv6().unwrap_or(Ipv6Addr::LOCALHOST)
        } else {
            self.interface
        };
        self.local_mac = encode_bip6_mac(local_ip, local_port);

        self.source_vmac = if let Some(id) = self.device_instance {
            derive_vmac_from_device_instance(id)
        } else {
            let local_v6 = SocketAddrV6::new(local_ip, local_port, 0, 0);
            derive_vmac_from_addr(&local_v6)
        };

        let if_index = if local_ip.is_loopback() {
            0u32
        } else {
            resolve_interface_index(&local_ip).unwrap_or_else(|| {
                warn!("Could not resolve interface index for {local_ip}, using OS default (0)");
                0u32
            })
        };
        for group in &[
            BACNET_IPV6_MULTICAST_LINK_LOCAL,
            BACNET_IPV6_MULTICAST_SITE_LOCAL,
            BACNET_IPV6_MULTICAST_ORG_LOCAL,
        ] {
            if let Err(e) = socket.join_multicast_v6(group, if_index) {
                warn!("Could not join IPv6 multicast group {group} on interface {if_index}: {e}");
            }
        }

        let socket = Arc::new(socket);
        self.socket = Some(Arc::clone(&socket));

        // VMAC collision detection and resolution
        {
            let multicast_dest =
                SocketAddrV6::new(BACNET_IPV6_MULTICAST_LINK_LOCAL, self.port, 0, if_index);
            let mut check_buf = vec![0u8; 64];

            for attempt in 0..=MAX_VMAC_RETRIES {
                let var_msg = encode_virtual_address_resolution(&self.source_vmac);
                let _ = socket.send_to(&var_msg, multicast_dest).await;

                let mut collision = false;
                let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
                loop {
                    match tokio::time::timeout_at(deadline, socket.recv_from(&mut check_buf)).await
                    {
                        Ok(Ok((len, _peer))) => {
                            if let Ok(frame) = decode_bvlc6(&check_buf[..len]) {
                                // VAR-ACK from another node using our VMAC
                                if frame.function == Bvlc6Function::VirtualAddressResolutionAck
                                    && frame.source_vmac == self.source_vmac
                                {
                                    collision = true;
                                    break;
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            debug!(error = %e, "Error during VMAC collision check");
                            break;
                        }
                        Err(_) => break, // timeout elapsed — no collision
                    }
                }

                if !collision {
                    break;
                }

                if attempt < MAX_VMAC_RETRIES {
                    let old_vmac = self.source_vmac;
                    self.source_vmac = generate_random_vmac();
                    warn!(
                        old_vmac = ?old_vmac,
                        new_vmac = ?self.source_vmac,
                        attempt = attempt + 1,
                        max_retries = MAX_VMAC_RETRIES,
                        "BIP6 VMAC collision detected, re-deriving new VMAC"
                    );
                } else {
                    warn!(
                        vmac = ?self.source_vmac,
                        "BIP6 VMAC collision persists after {MAX_VMAC_RETRIES} retries, \
                         proceeding with current VMAC"
                    );
                }
            }
        }

        /// NPDU receive channel capacity for high-throughput UDP transports.
        const NPDU_CHANNEL_CAPACITY: usize = 256;

        let (tx, rx) = mpsc::channel(NPDU_CHANNEL_CAPACITY);
        let local_mac = self.local_mac;

        let source_vmac_copy = self.source_vmac;
        let socket_for_recv = Arc::clone(&socket);
        let vmac_table_clone = self.vmac_table.clone();
        let recv_task = tokio::spawn(async move {
            let mut recv_buf = vec![0u8; 2048];
            loop {
                match socket_for_recv.recv_from(&mut recv_buf).await {
                    Ok((len, addr)) => {
                        let data = &recv_buf[..len];
                        match decode_bvlc6(data) {
                            Ok(frame) => {
                                // Learn-on-receive: update VMAC table from every frame
                                if frame.source_vmac != [0; 3] {
                                    if let std::net::SocketAddr::V6(v6) = addr {
                                        vmac_table_clone.learn(frame.source_vmac, v6).await;
                                    }
                                }

                                match frame.function {
                                    Bvlc6Function::OriginalUnicast
                                    | Bvlc6Function::OriginalBroadcast => {
                                        let source_mac = if let std::net::SocketAddr::V6(v6) = addr
                                        {
                                            MacAddr::from_slice(&encode_bip6_mac(
                                                *v6.ip(),
                                                v6.port(),
                                            ))
                                        } else {
                                            continue;
                                        };
                                        if source_mac[..] == local_mac[..] {
                                            continue;
                                        }
                                        if tx
                                            .try_send(ReceivedNpdu {
                                                npdu: frame.payload.clone(),
                                                source_mac,
                                                reply_tx: None,
                                            })
                                            .is_err()
                                        {
                                            warn!(
                                                "BIP6: NPDU channel full, dropping incoming frame"
                                            );
                                        }
                                    }

                                    Bvlc6Function::ForwardedNpdu => {
                                        match decode_forwarded_npdu_payload(&frame.payload) {
                                            Ok((originating_vmac, _source_addr, npdu_bytes)) => {
                                                if npdu_bytes.is_empty() {
                                                    debug!(
                                                    "ForwardedNpdu with no NPDU payload, ignoring"
                                                );
                                                    continue;
                                                }
                                                if tx
                                                    .try_send(ReceivedNpdu {
                                                        npdu: Bytes::copy_from_slice(npdu_bytes),
                                                        source_mac: MacAddr::from_slice(
                                                            &originating_vmac,
                                                        ),
                                                        reply_tx: None,
                                                    })
                                                    .is_err()
                                                {
                                                    warn!("BIP6: NPDU channel full, dropping forwarded frame");
                                                }
                                            }
                                            Err(e) => {
                                                debug!(
                                                    error = %e,
                                                    "Failed to decode ForwardedNpdu payload"
                                                );
                                            }
                                        }
                                    }

                                    Bvlc6Function::VirtualAddressResolution => {
                                        // VAR: sender is checking if anyone else uses their VMAC.
                                        // If the source VMAC matches ours, respond (collision).
                                        if frame.source_vmac == source_vmac_copy {
                                            debug!(
                                                vmac = ?source_vmac_copy,
                                                "Received VAR for our VMAC, sending VAR-Ack"
                                            );
                                            let ack = encode_virtual_address_resolution_ack(
                                                &source_vmac_copy,
                                                &frame.source_vmac,
                                            );
                                            let _ = socket_for_recv.send_to(&ack, addr).await;
                                        }
                                    }

                                    Bvlc6Function::AddressResolution => {
                                        // AR: sender wants to know our B/IPv6 address from our VMAC.
                                        // destination_vmac is the target being resolved.
                                        if let Some(target) = frame.destination_vmac {
                                            if target == source_vmac_copy {
                                                debug!(
                                                    vmac = ?source_vmac_copy,
                                                    "Received AR for our VMAC, sending AR-Ack"
                                                );
                                                let ack = encode_address_resolution_ack(
                                                    &source_vmac_copy,
                                                    &frame.source_vmac,
                                                );
                                                let _ = socket_for_recv.send_to(&ack, addr).await;
                                            }
                                        }
                                    }

                                    Bvlc6Function::AddressResolutionAck => {
                                        // AR-ACK: learn the sender's VMAC→address mapping
                                        // (will be used by VMAC table in future)
                                        debug!(
                                            vmac = ?frame.source_vmac,
                                            addr = %addr,
                                            "Received AR-Ack"
                                        );
                                    }

                                    Bvlc6Function::VirtualAddressResolutionAck => {
                                        // VAR-ACK: someone responded to our collision check
                                        if frame.source_vmac == source_vmac_copy {
                                            warn!(
                                                vmac = ?source_vmac_copy,
                                                "BIP6 VMAC collision detected! \
                                                 Another node responded with our VMAC."
                                            );
                                        }
                                    }

                                    Bvlc6Function::Result => {
                                        // Log BVLC-Result for diagnostics
                                        if frame.payload.len() >= 2 {
                                            let result_code = u16::from_be_bytes([
                                                frame.payload[0],
                                                frame.payload[1],
                                            ]);
                                            if result_code == 0x0000 {
                                                debug!("BIP6: BVLC-Result successful");
                                            } else {
                                                tracing::error!(
                                                    code = result_code,
                                                    "BIP6: BVLC-Result NAK"
                                                );
                                            }
                                        }
                                    }

                                    _ => {
                                        debug!(
                                            function = ?frame.function,
                                            "Unhandled BVLC6 function"
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "Failed to decode BVLC6 frame");
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "IPv6 UDP recv error");
                        break;
                    }
                }
            }
        });

        self.recv_task = Some(recv_task);

        // Start foreign device registration if configured
        if let Some(fd) = &self.foreign_device {
            let bbmd_addr = SocketAddrV6::new(fd.bbmd_ip, fd.bbmd_port, 0, 0);
            let ttl = fd.ttl;
            let sock = Arc::clone(&socket);
            let source_vmac = self.source_vmac;

            // Send initial registration
            send_register_foreign_device_v6(&sock, bbmd_addr, ttl, &source_vmac).await;

            // Re-register at TTL/2 interval
            let interval = std::time::Duration::from_secs(((ttl as u64) / 2).max(30));
            let reg_task = tokio::spawn(async move {
                let mut ticker = tokio::time::interval(interval);
                ticker.tick().await; // Skip first immediate tick
                loop {
                    ticker.tick().await;
                    send_register_foreign_device_v6(&sock, bbmd_addr, ttl, &source_vmac).await;
                }
            });
            self.registration_task = Some(reg_task);
        }

        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        if let Some(task) = self.registration_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.recv_task.take() {
            task.abort();
            let _ = task.await;
        }
        self.socket = None;
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let socket = self.socket.as_ref().ok_or_else(|| {
            Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "Transport not started",
            ))
        })?;

        let (ip, port) = decode_bip6_mac(mac)?;
        let dest = SocketAddrV6::new(ip, port, 0, 0);

        let mut buf = BytesMut::with_capacity(BVLC6_UNICAST_HEADER_LENGTH + npdu.len());
        let source_vmac = self.source_vmac;
        // Look up destination VMAC from table (Clause U.5).
        // Fall back to reverse address lookup, then [0; 3] (unknown).
        let dest_vmac = self
            .vmac_table
            .lookup_by_addr(&dest)
            .await
            .unwrap_or([0; 3]);
        encode_bvlc6_original_unicast(&mut buf, &source_vmac, &dest_vmac, npdu)?;

        socket.send_to(&buf, dest).await.map_err(Error::Transport)?;

        Ok(())
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        let socket = self.socket.as_ref().ok_or_else(|| {
            Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "Transport not started",
            ))
        })?;

        let source_vmac = self.source_vmac;

        // In foreign device mode, use Distribute-Broadcast-To-Network via BBMD
        if let Some(fd) = &self.foreign_device {
            let bbmd_addr = SocketAddrV6::new(fd.bbmd_ip, fd.bbmd_port, 0, 0);
            let mut buf = BytesMut::with_capacity(BVLC6_HEADER_LENGTH + npdu.len());
            encode_bvlc6(
                &mut buf,
                Bvlc6Function::DistributeBroadcastToNetwork,
                &source_vmac,
                npdu,
            )?;
            socket
                .send_to(&buf, bbmd_addr)
                .await
                .map_err(Error::Transport)?;
            return Ok(());
        }

        let dest = SocketAddrV6::new(self.broadcast_scope.multicast_addr(), self.port, 0, 0);
        let mut buf = BytesMut::with_capacity(BVLC6_HEADER_LENGTH + npdu.len());
        encode_bvlc6_original_broadcast(&mut buf, &source_vmac, npdu)?;

        socket.send_to(&buf, dest).await.map_err(Error::Transport)?;

        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_original_unicast() {
        let src_vmac: Bip6Vmac = [0x01, 0x02, 0x03];
        let dst_vmac: Bip6Vmac = [0x0A, 0x0B, 0x0C];
        let npdu = vec![0x01, 0x00, 0xAA];
        let mut buf = BytesMut::new();
        encode_bvlc6_original_unicast(&mut buf, &src_vmac, &dst_vmac, &npdu)
            .expect("valid BVLC6 unicast encoding");
        assert_eq!(buf[0], BVLC6_TYPE);
        assert_eq!(buf[1], Bvlc6Function::OriginalUnicast.to_byte());
        let len = u16::from_be_bytes([buf[2], buf[3]]);
        assert_eq!(len as usize, BVLC6_UNICAST_HEADER_LENGTH + npdu.len());
        assert_eq!(&buf[4..7], &src_vmac);
        assert_eq!(&buf[7..10], &dst_vmac);
        assert_eq!(&buf[10..], &npdu[..]);
    }

    #[test]
    fn encode_original_broadcast() {
        let vmac: Bip6Vmac = [0x01; 3];
        let npdu = vec![0xBB];
        let mut buf = BytesMut::new();
        encode_bvlc6_original_broadcast(&mut buf, &vmac, &npdu)
            .expect("valid BVLC6 broadcast encoding");
        assert_eq!(buf[1], Bvlc6Function::OriginalBroadcast.to_byte());
    }

    #[test]
    fn encode_bvlc6_oversized_payload_errors() {
        let vmac: Bip6Vmac = [0x01; 3];
        let npdu = vec![0; u16::MAX as usize - BVLC6_HEADER_LENGTH + 1];
        let mut buf = BytesMut::new();
        assert!(encode_bvlc6(&mut buf, Bvlc6Function::OriginalBroadcast, &vmac, &npdu).is_err());
    }

    #[test]
    fn encode_bvlc6_unicast_oversized_payload_errors() {
        let src_vmac: Bip6Vmac = [0x01, 0x02, 0x03];
        let dst_vmac: Bip6Vmac = [0x0A, 0x0B, 0x0C];
        let npdu = vec![0; u16::MAX as usize - BVLC6_UNICAST_HEADER_LENGTH + 1];
        let mut buf = BytesMut::new();
        assert!(encode_bvlc6_original_unicast(&mut buf, &src_vmac, &dst_vmac, &npdu).is_err());
    }

    #[test]
    fn decode_round_trip_unicast() {
        let src_vmac: Bip6Vmac = [0x01, 0x02, 0x03];
        let dst_vmac: Bip6Vmac = [0x0A, 0x0B, 0x0C];
        let npdu = vec![0x01, 0x00, 0xAA, 0xBB];
        let mut buf = BytesMut::new();
        encode_bvlc6_original_unicast(&mut buf, &src_vmac, &dst_vmac, &npdu)
            .expect("valid BVLC6 unicast encoding");
        let decoded = decode_bvlc6(&buf).unwrap();
        assert_eq!(decoded.function, Bvlc6Function::OriginalUnicast);
        assert_eq!(decoded.source_vmac, src_vmac);
        assert_eq!(decoded.destination_vmac, Some(dst_vmac));
        assert_eq!(decoded.payload, npdu);
    }

    #[test]
    fn decode_rejects_short_frame() {
        assert!(decode_bvlc6(&[0x82, 0x01]).is_err());
    }

    #[test]
    fn decode_rejects_wrong_type() {
        assert!(decode_bvlc6(&[0x81, 0x01, 0x00, 0x07, 0, 0, 0]).is_err());
    }

    #[test]
    fn function_round_trip() {
        for byte in 0x00..=0x0Cu8 {
            let f = Bvlc6Function::from_byte(byte);
            assert_eq!(f.to_byte(), byte);
        }
    }

    #[test]
    fn bip6_mac_round_trip() {
        let ip = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);
        let port = 47808u16;
        let mac = encode_bip6_mac(ip, port);
        assert_eq!(mac.len(), 18);
        let (decoded_ip, decoded_port) = decode_bip6_mac(&mac).unwrap();
        assert_eq!(decoded_ip, ip);
        assert_eq!(decoded_port, port);
    }

    #[test]
    fn bip6_mac_rejects_wrong_length() {
        assert!(decode_bip6_mac(&[0; 6]).is_err());
        assert!(decode_bip6_mac(&[0; 20]).is_err());
    }

    #[test]
    fn bip6_max_apdu_length() {
        let transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);
        assert_eq!(transport.max_apdu_length(), 1476);
    }

    #[tokio::test]
    async fn bip6_start_stop() {
        let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);
        let _rx = transport.start().await.unwrap();
        assert!(transport.socket.is_some());
        assert_eq!(transport.local_mac().len(), 18);
        transport.stop().await.unwrap();
        assert!(transport.socket.is_none());
    }

    #[tokio::test]
    async fn bip6_unicast_loopback() {
        let mut transport_a = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);
        let mut transport_b = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);

        let _rx_a = transport_a.start().await.unwrap();
        let mut rx_b = transport_b.start().await.unwrap();

        let test_npdu = vec![0x01, 0x00, 0xDE, 0xAD];

        transport_a
            .send_unicast(&test_npdu, transport_b.local_mac())
            .await
            .unwrap();

        let received = tokio::time::timeout(std::time::Duration::from_secs(2), rx_b.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        assert_eq!(received.npdu, test_npdu);
        assert_eq!(received.source_mac.as_slice(), transport_a.local_mac());

        transport_a.stop().await.unwrap();
        transport_b.stop().await.unwrap();
    }

    // --- Virtual Address Resolution tests ---

    #[test]
    fn encode_decode_virtual_address_resolution() {
        let vmac: Bip6Vmac = [0xAA, 0xBB, 0xCC];
        let buf = encode_virtual_address_resolution(&vmac);

        // VAR is 7 bytes: type(1) + function(1) + length(2) + source_vmac(3)
        assert_eq!(buf.len(), BVLC6_HEADER_LENGTH);
        assert_eq!(buf[0], BVLC6_TYPE);
        assert_eq!(buf[1], Bvlc6Function::VirtualAddressResolution.to_byte());
        let total_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        assert_eq!(total_len, BVLC6_HEADER_LENGTH);
        assert_eq!(&buf[4..7], &vmac);

        let frame = decode_bvlc6(&buf).unwrap();
        assert_eq!(frame.function, Bvlc6Function::VirtualAddressResolution);
        assert_eq!(frame.source_vmac, vmac);
        assert!(frame.payload.is_empty());
    }

    #[test]
    fn encode_decode_virtual_address_resolution_ack() {
        let source: Bip6Vmac = [0x11, 0x22, 0x33];
        let dest: Bip6Vmac = [0x44, 0x55, 0x66];
        let buf = encode_virtual_address_resolution_ack(&source, &dest);

        // VAR-ACK is 10 bytes: type(1)+function(1)+length(2)+source(3)+dest(3)
        assert_eq!(buf.len(), BVLC6_UNICAST_HEADER_LENGTH);
        assert_eq!(buf[0], BVLC6_TYPE);
        assert_eq!(buf[1], Bvlc6Function::VirtualAddressResolutionAck.to_byte());
        let total_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        assert_eq!(total_len, BVLC6_UNICAST_HEADER_LENGTH);
        assert_eq!(&buf[4..7], &source);
        assert_eq!(&buf[7..10], &dest);

        let frame = decode_bvlc6(&buf).unwrap();
        assert_eq!(frame.function, Bvlc6Function::VirtualAddressResolutionAck);
        assert_eq!(frame.source_vmac, source);
        assert_eq!(frame.destination_vmac, Some(dest));
        assert!(frame.payload.is_empty());
    }

    #[test]
    fn encode_decode_address_resolution() {
        let source: Bip6Vmac = [0x01, 0x02, 0x03];
        let target: Bip6Vmac = [0x04, 0x05, 0x06];
        let buf = encode_address_resolution(&source, &target);

        assert_eq!(buf.len(), BVLC6_UNICAST_HEADER_LENGTH);
        let frame = decode_bvlc6(&buf).unwrap();
        assert_eq!(frame.function, Bvlc6Function::AddressResolution);
        assert_eq!(frame.source_vmac, source);
        assert_eq!(frame.destination_vmac, Some(target));
    }

    #[test]
    fn encode_decode_address_resolution_ack() {
        let source: Bip6Vmac = [0x0A, 0x0B, 0x0C];
        let dest: Bip6Vmac = [0x0D, 0x0E, 0x0F];
        let buf = encode_address_resolution_ack(&source, &dest);

        assert_eq!(buf.len(), BVLC6_UNICAST_HEADER_LENGTH);
        let frame = decode_bvlc6(&buf).unwrap();
        assert_eq!(frame.function, Bvlc6Function::AddressResolutionAck);
        assert_eq!(frame.source_vmac, source);
        assert_eq!(frame.destination_vmac, Some(dest));
    }

    #[test]
    fn vmac_from_device_instance_masks_to_22_bits() {
        let vmac = derive_vmac_from_device_instance(0x123456);
        assert_eq!(vmac, [0x12, 0x34, 0x56]);
        // Value > 22 bits — upper bits should be masked off
        let vmac = derive_vmac_from_device_instance(0xFFFFFFFF);
        assert_eq!(vmac, [0x3F, 0xFF, 0xFF]);
    }

    // --- ForwardedNpdu tests ---

    #[test]
    fn decode_forwarded_npdu_extracts_npdu() {
        let originating_vmac: Bip6Vmac = [0xDE, 0xAD, 0x01];
        let source_ip = Ipv6Addr::LOCALHOST;
        let source_port: u16 = 47808;
        let npdu_data = vec![0x01, 0x00, 0xFF, 0xEE];
        let mut payload = originating_vmac.to_vec();
        payload.extend_from_slice(&source_ip.octets());
        payload.extend_from_slice(&source_port.to_be_bytes());
        payload.extend_from_slice(&npdu_data);

        let (vmac, addr, npdu) = decode_forwarded_npdu_payload(&payload).unwrap();
        assert_eq!(vmac, originating_vmac);
        assert_eq!(*addr.ip(), source_ip);
        assert_eq!(addr.port(), source_port);
        assert_eq!(npdu, &npdu_data[..]);
    }

    #[test]
    fn decode_forwarded_npdu_rejects_short_payload() {
        assert!(decode_forwarded_npdu_payload(&[0x01; 20]).is_err());
        assert!(decode_forwarded_npdu_payload(&[]).is_err());
    }

    #[test]
    fn decode_forwarded_npdu_vmac_and_addr_only_is_ok() {
        // Exactly 21 bytes = VMAC + B/IPv6 address with empty NPDU
        let mut payload = vec![0x01, 0x02, 0x03]; // vmac
        payload.extend_from_slice(&Ipv6Addr::LOCALHOST.octets()); // 16 bytes
        payload.extend_from_slice(&47808u16.to_be_bytes()); // 2 bytes
        let (vmac, _addr, npdu) = decode_forwarded_npdu_payload(&payload).unwrap();
        assert_eq!(vmac, [0x01, 0x02, 0x03]);
        assert!(npdu.is_empty());
    }

    #[test]
    fn forwarded_npdu_encode_decode_round_trip() {
        // Build a full ForwardedNpdu BVLC6 frame and decode it
        let sender_vmac: Bip6Vmac = [0x10, 0x20, 0x30];
        let originating_vmac: Bip6Vmac = [0xAA, 0xBB, 0xCC];
        let source_ip = Ipv6Addr::LOCALHOST;
        let npdu = vec![0x01, 0x00, 0xDE, 0xAD];

        let mut forwarded_payload = originating_vmac.to_vec();
        forwarded_payload.extend_from_slice(&source_ip.octets());
        forwarded_payload.extend_from_slice(&47808u16.to_be_bytes());
        forwarded_payload.extend_from_slice(&npdu);

        let mut buf = BytesMut::new();
        encode_bvlc6(
            &mut buf,
            Bvlc6Function::ForwardedNpdu,
            &sender_vmac,
            &forwarded_payload,
        )
        .expect("valid BVLC6 encoding");

        let frame = decode_bvlc6(&buf).unwrap();
        assert_eq!(frame.function, Bvlc6Function::ForwardedNpdu);
        assert_eq!(frame.source_vmac, sender_vmac);

        let (orig_vmac, addr, extracted_npdu) =
            decode_forwarded_npdu_payload(&frame.payload).unwrap();
        assert_eq!(orig_vmac, originating_vmac);
        assert_eq!(*addr.ip(), source_ip);
        assert_eq!(extracted_npdu, &npdu[..]);
    }

    #[tokio::test]
    async fn bip6_forwarded_npdu_delivered() {
        // Verify that a ForwardedNpdu sent to a transport is delivered as a ReceivedNpdu
        let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);
        let mut rx = transport.start().await.unwrap();

        // Build a ForwardedNpdu frame from a "BBMD"
        let bbmd_vmac: Bip6Vmac = [0xBB, 0xBB, 0xBB];
        let originating_vmac: Bip6Vmac = [0xAA, 0xAA, 0xAA];
        let test_npdu = vec![0x01, 0x00, 0xCA, 0xFE];

        let mut forwarded_payload = originating_vmac.to_vec();
        forwarded_payload.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
        forwarded_payload.extend_from_slice(&47808u16.to_be_bytes());
        forwarded_payload.extend_from_slice(&test_npdu);

        let mut buf = BytesMut::new();
        encode_bvlc6(
            &mut buf,
            Bvlc6Function::ForwardedNpdu,
            &bbmd_vmac,
            &forwarded_payload,
        )
        .expect("valid BVLC6 encoding");

        // Send directly to the transport's bound address using a separate socket
        let sender = UdpSocket::bind("[::1]:0").await.unwrap();
        let (_, transport_port) = decode_bip6_mac(transport.local_mac()).unwrap();
        let dest = SocketAddrV6::new(Ipv6Addr::LOCALHOST, transport_port, 0, 0);
        sender.send_to(&buf, dest).await.unwrap();

        let received = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        assert_eq!(received.npdu, test_npdu);
        // source_mac must be the originating VMAC (3 bytes), not the UDP sender address
        assert_eq!(received.source_mac.as_slice(), &originating_vmac[..]);

        transport.stop().await.unwrap();
    }

    #[tokio::test]
    async fn bip6_var_response() {
        // Verify that receiving a VAR from a node with the same VMAC
        // causes a VAR-Ack response (collision detection per Clause U.5).
        let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(42));
        let _rx = transport.start().await.unwrap();
        let our_vmac = transport.source_vmac;

        // Build a VAR frame from a node claiming our VMAC (collision scenario)
        let buf = encode_virtual_address_resolution(&our_vmac);

        // Send VAR to the transport
        let checker = UdpSocket::bind("[::1]:0").await.unwrap();
        let (_, transport_port) = decode_bip6_mac(transport.local_mac()).unwrap();
        let dest = SocketAddrV6::new(Ipv6Addr::LOCALHOST, transport_port, 0, 0);
        checker.send_to(&buf, dest).await.unwrap();

        // We should receive a VAR-Ack back (confirming collision)
        let mut resp_buf = vec![0u8; 64];
        let result =
            tokio::time::timeout(Duration::from_secs(2), checker.recv_from(&mut resp_buf)).await;

        match result {
            Ok(Ok((len, _))) => {
                let frame = decode_bvlc6(&resp_buf[..len]).unwrap();
                assert_eq!(frame.function, Bvlc6Function::VirtualAddressResolutionAck);
                assert_eq!(frame.source_vmac, our_vmac);
                // destination_vmac should be the querier's VMAC (same as ours)
                assert_eq!(frame.destination_vmac, Some(our_vmac));
            }
            Ok(Err(e)) => panic!("recv error: {e}"),
            Err(_) => panic!("timeout waiting for VAR-Ack response"),
        }

        transport.stop().await.unwrap();
    }

    /// Verify that local `Bvlc6Function` byte values match `bacnet_types::Bvlc6Function`.
    #[test]
    fn bvlc6_function_codes_match_types_crate() {
        use bacnet_types::enums::Bvlc6Function as TypesBvlc6;

        let expected: &[(u8, &str)] = &[
            (0x00, "BVLC_RESULT"),
            (0x01, "ORIGINAL_UNICAST_NPDU"),
            (0x02, "ORIGINAL_BROADCAST_NPDU"),
            (0x03, "ADDRESS_RESOLUTION"),
            (0x04, "FORWARDED_ADDRESS_RESOLUTION"),
            (0x05, "ADDRESS_RESOLUTION_ACK"),
            (0x06, "VIRTUAL_ADDRESS_RESOLUTION"),
            (0x07, "VIRTUAL_ADDRESS_RESOLUTION_ACK"),
            (0x08, "FORWARDED_NPDU"),
            (0x09, "REGISTER_FOREIGN_DEVICE"),
            (0x0A, "DELETE_FOREIGN_DEVICE_TABLE_ENTRY"),
            (0x0C, "DISTRIBUTE_BROADCAST_TO_NETWORK"),
        ];

        for &(byte, _name) in expected {
            let local = Bvlc6Function::from_byte(byte);
            let types_val = TypesBvlc6::from_raw(byte);
            assert_eq!(
                local.to_byte(),
                types_val.to_raw(),
                "Mismatch at 0x{byte:02X}: bip6.rs={}, enums.rs={}",
                local.to_byte(),
                types_val.to_raw(),
            );
        }

        // Verify 0x0C is Distribute-Broadcast-To-Network (not the old SECURE_BVLL)
        assert_eq!(
            Bvlc6Function::DistributeBroadcastToNetwork.to_byte(),
            TypesBvlc6::DISTRIBUTE_BROADCAST_TO_NETWORK.to_raw(),
        );
        // 0x0B should decode as Unknown since it's removed
        assert!(matches!(
            Bvlc6Function::from_byte(0x0B),
            Bvlc6Function::Unknown(0x0B)
        ));
    }

    #[test]
    fn generate_random_vmac_produces_3_bytes() {
        let vmac = generate_random_vmac();
        assert_eq!(vmac.len(), 3);
    }

    #[test]
    fn generate_random_vmac_is_nondeterministic() {
        // Generate several VMACs — at least two should differ.
        let vmacs: Vec<Bip6Vmac> = (0..10).map(|_| generate_random_vmac()).collect();
        let all_same = vmacs.windows(2).all(|w| w[0] == w[1]);
        assert!(!all_same, "10 random VMACs should not all be identical");
    }

    #[test]
    fn max_vmac_retries_constant() {
        const { assert!(MAX_VMAC_RETRIES >= 1, "must allow at least one retry") };
    }
}
