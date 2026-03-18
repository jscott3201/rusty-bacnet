//! BACnet/IPv6 BVLC codec per ASHRAE 135-2020 Annex U.
//!
//! Frame format: type(1) + function(1) + length(2) + source-vmac(3) + payload
//! Multicast groups: FF02::BAC0 (link-local), FF05::BAC0 (site-local)

use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{BufMut, Bytes, BytesMut};
use std::collections::hash_map::RandomState;
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
    // 0x0B is removed per Table U-1
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
            // 0x0B removed per Table U-1
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
    /// Source virtual MAC address (3 bytes per Annex U.2).
    pub source_vmac: Bip6Vmac,
    /// Destination virtual MAC address (3 bytes, present in unicast only per U.2.2.1).
    pub destination_vmac: Option<Bip6Vmac>,
    /// Payload after the BVLC-IPv6 header (typically NPDU bytes).
    pub payload: Bytes,
}

/// Encode a BVLC-IPv6 frame into a buffer.
///
/// Wire format: type(1) + function(1) + length(2) + source-vmac(3) + payload.
pub fn encode_bvlc6(
    buf: &mut BytesMut,
    function: Bvlc6Function,
    source_vmac: &Bip6Vmac,
    npdu: &[u8],
) {
    let total_length = BVLC6_HEADER_LENGTH + npdu.len();
    debug_assert!(
        total_length <= u16::MAX as usize,
        "BVLC6 frame length overflow"
    );
    let wire_length = (total_length as u64).min(u16::MAX as u64) as u16;
    buf.reserve(total_length);
    buf.put_u8(BVLC6_TYPE);
    buf.put_u8(function.to_byte());
    buf.put_u16(wire_length);
    buf.put_slice(source_vmac);
    buf.put_slice(npdu);
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

    // U.2.2.1: Original-Unicast-NPDU has Destination-Virtual-Address at bytes [7..10]
    let (destination_vmac, payload_start) = if function == Bvlc6Function::OriginalUnicast {
        if length < BVLC6_UNICAST_HEADER_LENGTH {
            return Err(Error::decoding(
                7,
                "BVLC6 unicast frame too short for destination VMAC",
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
///
/// U.2.2.1: Type(1) + Function(1) + Length(2) + Source-Virtual-Address(3)
///          + Destination-Virtual-Address(3) + NPDU.
pub fn encode_bvlc6_original_unicast(
    buf: &mut BytesMut,
    source_vmac: &Bip6Vmac,
    dest_vmac: &Bip6Vmac,
    npdu: &[u8],
) {
    let total_length = BVLC6_UNICAST_HEADER_LENGTH + npdu.len();
    debug_assert!(
        total_length <= u16::MAX as usize,
        "BVLC6 unicast frame length overflow"
    );
    let wire_length = (total_length as u64).min(u16::MAX as u64) as u16;
    buf.reserve(total_length);
    buf.put_u8(BVLC6_TYPE);
    buf.put_u8(Bvlc6Function::OriginalUnicast.to_byte());
    buf.put_u16(wire_length);
    buf.put_slice(source_vmac);
    buf.put_slice(dest_vmac);
    buf.put_slice(npdu);
}

/// Encode a BVLC-IPv6 Original-Broadcast-NPDU frame.
pub fn encode_bvlc6_original_broadcast(buf: &mut BytesMut, source_vmac: &Bip6Vmac, npdu: &[u8]) {
    encode_bvlc6(buf, Bvlc6Function::OriginalBroadcast, source_vmac, npdu);
}

/// Encode a BVLC-IPv6 Virtual-Address-Resolution frame (Annex U.5).
///
/// The payload is the queried VMAC (3 bytes). The source VMAC in the header
/// is set to the querying node's own VMAC.
pub fn encode_virtual_address_resolution(source_vmac: &Bip6Vmac) -> BytesMut {
    let mut buf = BytesMut::with_capacity(BVLC6_HEADER_LENGTH + 3);
    encode_bvlc6(
        &mut buf,
        Bvlc6Function::VirtualAddressResolution,
        source_vmac,
        source_vmac,
    );
    buf
}

/// Encode a BVLC-IPv6 Virtual-Address-Resolution-Ack frame (Annex U.5).
///
/// Sent in response to a VirtualAddressResolution when the queried VMAC
/// matches our own. The payload is our VMAC (3 bytes).
pub fn encode_virtual_address_resolution_ack(source_vmac: &Bip6Vmac) -> BytesMut {
    let mut buf = BytesMut::with_capacity(BVLC6_HEADER_LENGTH + 3);
    encode_bvlc6(
        &mut buf,
        Bvlc6Function::VirtualAddressResolutionAck,
        source_vmac,
        source_vmac,
    );
    buf
}

/// Extract the NPDU from a ForwardedNpdu payload.
///
/// U.2.9.1: ForwardedNpdu payload (after the 7-byte BVLC header):
///   Original-Source-Virtual-Address(3) + Original-Source-B/IPv6-Address(18) + NPDU.
/// The 18-byte B/IPv6 address is: IPv6(16) + port(2).
/// Returns the originating VMAC, originating B/IPv6 address, and NPDU bytes.
pub fn decode_forwarded_npdu_payload(
    payload: &[u8],
) -> Result<(Bip6Vmac, SocketAddrV6, &[u8]), Error> {
    // Need at least vmac(3) + ipv6_addr(16) + port(2) = 21 bytes
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

/// BACnet/IPv6 transport over UDP (Annex U).
pub struct Bip6Transport {
    interface: Ipv6Addr,
    port: u16,
    device_instance: Option<u32>,
    local_mac: [u8; 18],
    source_vmac: Bip6Vmac,
    socket: Option<Arc<UdpSocket>>,
    recv_task: Option<JoinHandle<()>>,
}

impl Bip6Transport {
    /// Create a new BACnet/IPv6 transport.
    ///
    /// - `interface`: Local IPv6 address to bind (use `::` for all interfaces)
    /// - `port`: UDP port (default 47808 / 0xBAC0)
    /// - `device_instance`: If `Some(id)`, derive the 3-byte VMAC from the
    ///   lower 24 bits of the device instance (per Annex U.5). Otherwise the
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
        }
    }
}

/// Derive a 3-byte VMAC from the lower 24 bits of a device instance (Annex U.5).
fn derive_vmac_from_device_instance(device_instance: u32) -> Bip6Vmac {
    let bytes = device_instance.to_be_bytes(); // [b0, b1, b2, b3]
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
fn resolve_interface_index(addr: &Ipv6Addr) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::ffi::CStr;

        /// RAII guard for `getifaddrs` that calls `freeifaddrs` on drop.
        struct IfAddrsGuard(*mut libc::ifaddrs);
        impl Drop for IfAddrsGuard {
            fn drop(&mut self) {
                unsafe { libc::freeifaddrs(self.0) }
            }
        }

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

        // Derive VMAC: prefer device instance (Annex U.5), fall back to address XOR-fold.
        self.source_vmac = if let Some(id) = self.device_instance {
            derive_vmac_from_device_instance(id)
        } else {
            let local_v6 = SocketAddrV6::new(local_ip, local_port, 0, 0);
            derive_vmac_from_addr(&local_v6)
        };

        // Resolve interface index for multicast. Loopback uses 0 (OS default).
        // For non-loopback addresses, try to find the interface index.
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

        // --- VMAC collision detection and resolution per Annex U.5 ---
        // Send VirtualAddressResolution to link-local multicast, then wait
        // briefly for any VirtualAddressResolutionAck indicating a collision.
        // On collision, generate a new random VMAC and retry up to MAX_VMAC_RETRIES times.
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
                                if frame.function == Bvlc6Function::VirtualAddressResolutionAck
                                    && frame.payload.len() >= 3
                                {
                                    let their_vmac: Bip6Vmac =
                                        frame.payload[..3].try_into().unwrap();
                                    if their_vmac == self.source_vmac {
                                        collision = true;
                                        break;
                                    }
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
                        "BIP6 VMAC collision detected, re-deriving new VMAC (Annex U.5)"
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

        let (tx, rx) = mpsc::channel(256);
        let local_mac = self.local_mac;

        let source_vmac_copy = self.source_vmac;
        let socket_for_recv = Arc::clone(&socket);
        let recv_task = tokio::spawn(async move {
            let mut recv_buf = vec![0u8; 2048];
            loop {
                match socket_for_recv.recv_from(&mut recv_buf).await {
                    Ok((len, addr)) => {
                        let data = &recv_buf[..len];
                        match decode_bvlc6(data) {
                            Ok(frame) => match frame.function {
                                Bvlc6Function::OriginalUnicast
                                | Bvlc6Function::OriginalBroadcast => {
                                    let source_mac = if let std::net::SocketAddr::V6(v6) = addr {
                                        MacAddr::from_slice(&encode_bip6_mac(*v6.ip(), v6.port()))
                                    } else {
                                        continue;
                                    };
                                    if source_mac[..] == local_mac[..] {
                                        continue;
                                    }
                                    let _ = tx
                                        .send(ReceivedNpdu {
                                            npdu: frame.payload.clone(),
                                            source_mac,
                                            reply_tx: None,
                                        })
                                        .await;
                                }

                                // --- ForwardedNpdu: extract NPDU from payload ---
                                // Payload format: originating-VMAC(3) + NPDU bytes.
                                // Use the originating VMAC as source_mac (not the
                                // UDP sender, which is the forwarding BBMD).
                                Bvlc6Function::ForwardedNpdu => {
                                    match decode_forwarded_npdu_payload(&frame.payload) {
                                        Ok((originating_vmac, _source_addr, npdu_bytes)) => {
                                            if npdu_bytes.is_empty() {
                                                debug!(
                                                    "ForwardedNpdu with no NPDU payload, ignoring"
                                                );
                                                continue;
                                            }
                                            let _ = tx
                                                .send(ReceivedNpdu {
                                                    npdu: Bytes::copy_from_slice(npdu_bytes),
                                                    source_mac: MacAddr::from_slice(
                                                        &originating_vmac,
                                                    ),
                                                    reply_tx: None,
                                                })
                                                .await;
                                        }
                                        Err(e) => {
                                            debug!(
                                                error = %e,
                                                "Failed to decode ForwardedNpdu payload"
                                            );
                                        }
                                    }
                                }

                                // --- VirtualAddressResolution: respond if queried
                                //     VMAC matches ours ---
                                Bvlc6Function::VirtualAddressResolution => {
                                    if frame.payload.len() >= 3 {
                                        let query_vmac: Bip6Vmac =
                                            frame.payload[..3].try_into().unwrap();
                                        if query_vmac == source_vmac_copy {
                                            debug!(
                                                vmac = ?source_vmac_copy,
                                                "Received VAR for our VMAC, sending VAR-Ack"
                                            );
                                            let ack = encode_virtual_address_resolution_ack(
                                                &source_vmac_copy,
                                            );
                                            let _ = socket_for_recv.send_to(&ack, addr).await;
                                        }
                                    }
                                }

                                // --- VirtualAddressResolutionAck: collision detection ---
                                Bvlc6Function::VirtualAddressResolutionAck => {
                                    if frame.payload.len() >= 3 {
                                        let their_vmac: Bip6Vmac =
                                            frame.payload[..3].try_into().unwrap();
                                        if their_vmac == source_vmac_copy {
                                            warn!(
                                                vmac = ?source_vmac_copy,
                                                "BIP6 VMAC collision detected! \
                                                 Another node responded with our VMAC."
                                            );
                                        }
                                    }
                                }

                                _ => {
                                    debug!(
                                        function = ?frame.function,
                                        "Unhandled BVLC6 function (not yet implemented)"
                                    );
                                }
                            },
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
        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
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
        // Derive dest VMAC from lower 3 bytes of dest IPv6 address.
        // A full implementation would use a VMAC table (U.5).
        let dest_vmac: Bip6Vmac = [ip.octets()[13], ip.octets()[14], ip.octets()[15]];
        encode_bvlc6_original_unicast(&mut buf, &source_vmac, &dest_vmac, npdu);

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

        let dest = SocketAddrV6::new(BACNET_IPV6_MULTICAST_SITE_LOCAL, self.port, 0, 0);

        let mut buf = BytesMut::with_capacity(BVLC6_HEADER_LENGTH + npdu.len());
        let source_vmac = self.source_vmac;
        encode_bvlc6_original_broadcast(&mut buf, &source_vmac, npdu);

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
        encode_bvlc6_original_unicast(&mut buf, &src_vmac, &dst_vmac, &npdu);
        assert_eq!(buf[0], BVLC6_TYPE);
        assert_eq!(buf[1], Bvlc6Function::OriginalUnicast.to_byte());
        let len = u16::from_be_bytes([buf[2], buf[3]]);
        // U.2.2.1: 4 + src_vmac(3) + dst_vmac(3) + npdu
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
        encode_bvlc6_original_broadcast(&mut buf, &vmac, &npdu);
        assert_eq!(buf[1], Bvlc6Function::OriginalBroadcast.to_byte());
    }

    #[test]
    fn decode_round_trip_unicast() {
        let src_vmac: Bip6Vmac = [0x01, 0x02, 0x03];
        let dst_vmac: Bip6Vmac = [0x0A, 0x0B, 0x0C];
        let npdu = vec![0x01, 0x00, 0xAA, 0xBB];
        let mut buf = BytesMut::new();
        encode_bvlc6_original_unicast(&mut buf, &src_vmac, &dst_vmac, &npdu);
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

        // Verify header
        assert_eq!(buf[0], BVLC6_TYPE);
        assert_eq!(buf[1], Bvlc6Function::VirtualAddressResolution.to_byte());
        let total_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        assert_eq!(total_len, BVLC6_HEADER_LENGTH + 3);
        // Source VMAC in header
        assert_eq!(&buf[4..7], &vmac);
        // Payload = queried VMAC
        assert_eq!(&buf[7..10], &vmac);

        // Round-trip decode
        let frame = decode_bvlc6(&buf).unwrap();
        assert_eq!(frame.function, Bvlc6Function::VirtualAddressResolution);
        assert_eq!(frame.source_vmac, vmac);
        assert_eq!(&frame.payload[..], &vmac);
    }

    #[test]
    fn encode_decode_virtual_address_resolution_ack() {
        let vmac: Bip6Vmac = [0x11, 0x22, 0x33];
        let buf = encode_virtual_address_resolution_ack(&vmac);

        assert_eq!(buf[0], BVLC6_TYPE);
        assert_eq!(buf[1], Bvlc6Function::VirtualAddressResolutionAck.to_byte());
        let total_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        assert_eq!(total_len, BVLC6_HEADER_LENGTH + 3);
        assert_eq!(&buf[4..7], &vmac);
        assert_eq!(&buf[7..10], &vmac);

        let frame = decode_bvlc6(&buf).unwrap();
        assert_eq!(frame.function, Bvlc6Function::VirtualAddressResolutionAck);
        assert_eq!(frame.source_vmac, vmac);
        assert_eq!(&frame.payload[..], &vmac);
    }

    // --- ForwardedNpdu tests ---

    #[test]
    fn decode_forwarded_npdu_extracts_npdu() {
        // U.2.9.1: ForwardedNpdu payload: vmac(3) + B/IPv6-address(18) + NPDU
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
        // Need at least 21 bytes (vmac=3 + ipv6=16 + port=2)
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

        // U.2.9.1: ForwardedNpdu payload: vmac(3) + B/IPv6-addr(18) + NPDU
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
        );

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

        // U.2.9.1: vmac(3) + B/IPv6-addr(18) + NPDU
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
        );

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
        // Verify that receiving a VirtualAddressResolution for our VMAC
        // causes a VirtualAddressResolutionAck response.
        let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(42));
        let _rx = transport.start().await.unwrap();
        let our_vmac = transport.source_vmac;

        // Build a VAR frame querying our VMAC
        let querier_vmac: Bip6Vmac = [0xFF, 0xFE, 0xFD];
        let mut buf = BytesMut::new();
        // The VAR payload is the queried VMAC (our VMAC)
        encode_bvlc6(
            &mut buf,
            Bvlc6Function::VirtualAddressResolution,
            &querier_vmac,
            &our_vmac,
        );

        // Send VAR to the transport
        let checker = UdpSocket::bind("[::1]:0").await.unwrap();
        let (_, transport_port) = decode_bip6_mac(transport.local_mac()).unwrap();
        let dest = SocketAddrV6::new(Ipv6Addr::LOCALHOST, transport_port, 0, 0);
        checker.send_to(&buf, dest).await.unwrap();

        // We should receive a VAR-Ack back
        let mut resp_buf = vec![0u8; 64];
        let result =
            tokio::time::timeout(Duration::from_secs(2), checker.recv_from(&mut resp_buf)).await;

        match result {
            Ok(Ok((len, _))) => {
                let frame = decode_bvlc6(&resp_buf[..len]).unwrap();
                assert_eq!(frame.function, Bvlc6Function::VirtualAddressResolutionAck);
                assert_eq!(frame.source_vmac, our_vmac);
                assert_eq!(&frame.payload[..3], &our_vmac);
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
            // 0x0B removed per Table U-1
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
