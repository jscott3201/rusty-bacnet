use std::net::{Ipv6Addr, SocketAddrV6};
use std::sync::Arc;
use std::time::Duration;

use bacnet_encoding::npdu::decode_npdu;
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::port::{ReceivedNpdu, TransportPort};
use crate::udp_metadata::{DestinationReceiver, IpVersion};

use super::frame::destination_vmac_matches;
use super::ingress::{
    forwarded_npdu_is_trusted, forwarded_source_is_usable, is_local_unicast_delivery,
    original_destination_matches,
};
use super::vmac_table::{derive_vmac_from_device_instance, generate_random_vmac, VmacTable};
use super::{
    decode_bvlc6, decode_forwarded_npdu_payload, encode_address_resolution,
    encode_address_resolution_ack, encode_bvlc6, encode_bvlc6_original_broadcast,
    encode_bvlc6_original_unicast, encode_virtual_address_resolution_ack, Bip6Vmac, Bvlc6Function,
    BVLC6_HEADER_LENGTH, BVLC6_UNICAST_HEADER_LENGTH, MAX_VMAC_RETRIES,
};

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
    pub(super) source_vmac: Bip6Vmac,
    pub(super) socket: Option<Arc<UdpSocket>>,
    recv_task: Option<JoinHandle<()>>,
    /// VMAC address table (Clause U.5).
    pub(super) vmac_table: VmacTable,
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
    ///   valid 22-bit device instance (per Clause H.7.2). Otherwise an
    ///   OS-random Random Device Instance VMAC is generated at startup.
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
    ///
    /// A foreign device must also have a configured Device instance. Random
    /// VMAC startup is rejected until BBMD-assisted collision resolution is
    /// implemented.
    /// Must be called before `start()`.
    pub fn register_as_foreign_device(&mut self, config: Bip6ForeignDeviceConfig) {
        self.foreign_device = Some(config);
    }
}

/// Derive a 3-byte VMAC from a 22-bit device instance (Clause H.7.2).
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

impl TransportPort for Bip6Transport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        if self.recv_task.is_some() {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "BIP6 transport already started",
            )));
        }
        if self.device_instance.is_some_and(|id| id > 0x3F_FFFF) {
            return Err(Error::Encoding(
                "BACnet Device instance must be in 0..=4194303".to_string(),
            ));
        }
        if self.foreign_device.is_some() && self.device_instance.is_none() {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "BIP6 foreign devices require a configured Device instance until BBMD-assisted random-VMAC resolution is implemented",
            )));
        }

        let wildcard_bind = self.interface.is_unspecified();
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

        let destination_receiver = DestinationReceiver::configure(&socket2_sock, IpVersion::V6)
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
        let local_unicast_ips = if wildcard_bind {
            crate::local_addresses::ipv6()
        } else {
            vec![local_ip]
        };
        #[cfg(unix)]
        if wildcard_bind && local_unicast_ips.is_empty() {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::AddrNotAvailable,
                "could not enumerate local IPv6 addresses for wildcard ingress",
            )));
        }
        self.local_mac = encode_bip6_mac(local_ip, local_port);

        self.source_vmac = if let Some(id) = self.device_instance {
            derive_vmac_from_device_instance(id)
        } else {
            generate_random_vmac()?
        };

        let if_index =
            crate::local_addresses::ipv6_interface_index(&local_ip).unwrap_or_else(|| {
                warn!("Could not resolve interface index for {local_ip}, using OS default (0)");
                0u32
            });
        let collision_probe_required = self.device_instance.is_none();
        let collision_group = self.broadcast_scope.multicast_addr();
        for group in &[
            BACNET_IPV6_MULTICAST_LINK_LOCAL,
            BACNET_IPV6_MULTICAST_SITE_LOCAL,
            BACNET_IPV6_MULTICAST_ORG_LOCAL,
        ] {
            if let Err(e) = socket.join_multicast_v6(group, if_index) {
                if collision_probe_required && *group == collision_group {
                    return Err(Error::Transport(e));
                }
                warn!("Could not join IPv6 multicast group {group} on interface {if_index}: {e}");
            }
        }

        let socket = Arc::new(socket);

        // VMAC collision detection and resolution
        if self.foreign_device.is_none() {
            let multicast_dest = SocketAddrV6::new(collision_group, self.port, 0, if_index);
            let mut check_buf = vec![0u8; 64];

            for attempt in 0..=MAX_VMAC_RETRIES {
                let ar_msg = encode_address_resolution(&self.source_vmac, &self.source_vmac);
                if let Err(error) = socket.send_to(&ar_msg, multicast_dest).await {
                    if collision_probe_required {
                        return Err(Error::Transport(error));
                    }
                    debug!(error = %error, "Configured-VMAC collision probe send failed");
                }

                let mut collision = false;
                let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
                loop {
                    match tokio::time::timeout_at(
                        deadline,
                        destination_receiver.recv_from(&socket, &mut check_buf),
                    )
                    .await
                    {
                        Ok(Ok(received)) => {
                            if let Ok(frame) = decode_bvlc6(&check_buf[..received.len]) {
                                let is_self_loop = matches!(
                                    received.peer,
                                    std::net::SocketAddr::V6(peer)
                                        if peer.port() == local_port
                                            && (*peer.ip() == local_ip
                                                || local_unicast_ips.contains(peer.ip()))
                                );
                                if frame.function == Bvlc6Function::AddressResolution
                                    && frame.destination_vmac == Some(self.source_vmac)
                                    && matches!(received.destination, std::net::IpAddr::V6(ip) if ip == collision_group)
                                    && received.os_group_delivery != Some(false)
                                    && !is_self_loop
                                {
                                    let ack = encode_address_resolution_ack(
                                        &self.source_vmac,
                                        &frame.source_vmac,
                                    );
                                    if let Err(error) = socket.send_to(&ack, received.peer).await {
                                        if collision_probe_required {
                                            return Err(Error::Transport(error));
                                        }
                                        debug!(error = %error, "Configured-VMAC AR-Ack send failed");
                                    }
                                    if frame.source_vmac == self.source_vmac {
                                        collision = true;
                                        break;
                                    }
                                }
                                // AR-ACK from another node using our VMAC.
                                if frame.function == Bvlc6Function::AddressResolutionAck
                                    && frame.source_vmac == self.source_vmac
                                    && is_local_unicast_delivery(
                                        received.destination,
                                        frame.destination_vmac,
                                        local_ip,
                                        self.source_vmac,
                                        &local_unicast_ips,
                                        wildcard_bind,
                                        received.os_group_delivery,
                                    )
                                {
                                    collision = true;
                                    break;
                                }
                            }
                        }
                        Ok(Err(e)) if e.kind() == std::io::ErrorKind::InvalidData => {
                            debug!(error = %e, "Error during VMAC collision check");
                            continue;
                        }
                        Ok(Err(e)) => {
                            if collision_probe_required {
                                return Err(Error::Transport(e));
                            }
                            debug!(error = %e, "Configured-VMAC collision probe receive failed");
                            break;
                        }
                        Err(_) => break, // timeout elapsed — no collision
                    }
                }

                if !collision {
                    break;
                }

                if self.device_instance.is_some() {
                    return Err(Error::Transport(std::io::Error::new(
                        std::io::ErrorKind::AddrInUse,
                        "configured BACnet Device instance VMAC is already in use",
                    )));
                }

                if attempt < MAX_VMAC_RETRIES {
                    let old_vmac = self.source_vmac;
                    self.source_vmac = generate_random_vmac()?;
                    warn!(
                        old_vmac = ?old_vmac,
                        new_vmac = ?self.source_vmac,
                        attempt = attempt + 1,
                        max_retries = MAX_VMAC_RETRIES,
                        "BIP6 VMAC collision detected, re-deriving new VMAC"
                    );
                } else {
                    return Err(Error::Transport(std::io::Error::new(
                        std::io::ErrorKind::AddrInUse,
                        format!(
                            "random BIP6 VMAC collision persists after {MAX_VMAC_RETRIES} retries"
                        ),
                    )));
                }
            }
        }

        /// NPDU receive channel capacity for high-throughput UDP transports.
        const NPDU_CHANNEL_CAPACITY: usize = 256;

        let (tx, rx) = mpsc::channel(NPDU_CHANNEL_CAPACITY);
        let local_mac = self.local_mac;

        let source_vmac_copy = self.source_vmac;
        let foreign_bbmd = self
            .foreign_device
            .as_ref()
            .map(|fd| (fd.bbmd_ip, fd.bbmd_port));
        let socket_for_recv = Arc::clone(&socket);
        let vmac_table_clone = self.vmac_table.clone();
        let recv_task = tokio::spawn(async move {
            let mut recv_buf = vec![0u8; 2048];
            loop {
                match destination_receiver
                    .recv_from(&socket_for_recv, &mut recv_buf)
                    .await
                {
                    Ok(received) => {
                        let data = &recv_buf[..received.len];
                        match decode_bvlc6(data) {
                            Ok(frame) => {
                                if !destination_vmac_matches(
                                    frame.function,
                                    frame.destination_vmac,
                                    source_vmac_copy,
                                ) {
                                    debug!(
                                        function = frame.function.to_byte(),
                                        destination_vmac = ?frame.destination_vmac,
                                        "Dropping BVLC6 message addressed to another VMAC"
                                    );
                                    continue;
                                }
                                if !original_destination_matches(
                                    frame.function,
                                    received.destination,
                                    frame.destination_vmac,
                                    local_ip,
                                    source_vmac_copy,
                                    &local_unicast_ips,
                                    wildcard_bind,
                                    received.os_group_delivery,
                                ) {
                                    debug!(
                                        function = frame.function.to_byte(),
                                        destination = %received.destination,
                                        "Dropping BVLC6/IP or Destination-VMAC mismatch"
                                    );
                                    continue;
                                }
                                if frame.function == Bvlc6Function::ForwardedNpdu
                                    && !forwarded_npdu_is_trusted(
                                        received.peer,
                                        received.destination,
                                        received.os_group_delivery,
                                        local_ip,
                                        &local_unicast_ips,
                                        wildcard_bind,
                                        foreign_bbmd,
                                    )
                                {
                                    debug!(
                                        peer = %received.peer,
                                        destination = %received.destination,
                                        "Dropping Forwarded-NPDU from an untrusted path"
                                    );
                                    continue;
                                }
                                // Forwarded-NPDU's source VMAC identifies the original
                                // node, not the forwarding BBMD. Its original address is
                                // learned from the function payload below.
                                if matches!(
                                    frame.function,
                                    Bvlc6Function::OriginalUnicast
                                        | Bvlc6Function::OriginalBroadcast
                                        | Bvlc6Function::AddressResolution
                                        | Bvlc6Function::AddressResolutionAck
                                        | Bvlc6Function::VirtualAddressResolution
                                        | Bvlc6Function::VirtualAddressResolutionAck
                                ) {
                                    if let std::net::SocketAddr::V6(v6) = received.peer {
                                        vmac_table_clone.learn(frame.source_vmac, v6).await;
                                    }
                                }

                                match frame.function {
                                    Bvlc6Function::OriginalUnicast
                                    | Bvlc6Function::OriginalBroadcast => {
                                        let source_mac =
                                            if let std::net::SocketAddr::V6(v6) = received.peer {
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
                                                link_layer_group: frame.function
                                                    == Bvlc6Function::OriginalBroadcast,
                                                data_attributes: Vec::new(),
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
                                            Ok((source_addr, npdu_bytes)) => {
                                                if npdu_bytes.is_empty() {
                                                    debug!(
                                                    "ForwardedNpdu with no NPDU payload, ignoring"
                                                );
                                                    continue;
                                                }
                                                if !forwarded_source_is_usable(source_addr) {
                                                    debug!(
                                                        source = %source_addr,
                                                        "Dropping Forwarded-NPDU with unusable origin"
                                                    );
                                                    continue;
                                                }
                                                if decode_npdu(Bytes::copy_from_slice(npdu_bytes))
                                                    .is_err()
                                                {
                                                    debug!(
                                                        "Dropping Forwarded-NPDU with malformed NPDU"
                                                    );
                                                    continue;
                                                }
                                                vmac_table_clone
                                                    .learn(frame.source_vmac, source_addr)
                                                    .await;
                                                let source_mac = encode_bip6_mac(
                                                    *source_addr.ip(),
                                                    source_addr.port(),
                                                );
                                                if tx
                                                    .try_send(ReceivedNpdu {
                                                        npdu: Bytes::copy_from_slice(npdu_bytes),
                                                        source_mac: MacAddr::from_slice(
                                                            &source_mac,
                                                        ),
                                                        link_layer_group: true,
                                                        data_attributes: Vec::new(),
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
                                        // A node receiving VAR at its unicast B/IPv6 address
                                        // answers with its own VMAC and the requester's VMAC.
                                        let ack = encode_virtual_address_resolution_ack(
                                            &source_vmac_copy,
                                            &frame.source_vmac,
                                        );
                                        let _ = socket_for_recv.send_to(&ack, received.peer).await;
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
                                                let _ = socket_for_recv
                                                    .send_to(&ack, received.peer)
                                                    .await;
                                            }
                                        }
                                    }

                                    Bvlc6Function::AddressResolutionAck => {
                                        // AR-ACK: learn the sender's VMAC→address mapping
                                        // (will be used by VMAC table in future)
                                        debug!(
                                            vmac = ?frame.source_vmac,
                                                addr = %received.peer,
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
                    Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                        debug!(error = %e, "Dropping UDP datagram with invalid destination metadata");
                    }
                    Err(e) => {
                        warn!(error = %e, "IPv6 UDP recv error");
                        break;
                    }
                }
            }
        });

        self.recv_task = Some(recv_task);
        self.socket = Some(Arc::clone(&socket));

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
        let mut buf = BytesMut::with_capacity(BVLC6_UNICAST_HEADER_LENGTH + npdu.len());
        // Original-Unicast requires the destination's actual VMAC. The
        // network MAC contains only IPv6+port, so use the latest learned
        // mapping both for that VMAC and for the exact scoped endpoint.
        let (dest_vmac, dest) =
            self.vmac_table
                .resolve_by_addr(ip, port)
                .await
                .ok_or_else(|| {
                    Error::Transport(std::io::Error::new(
                        std::io::ErrorKind::AddrNotAvailable,
                        "BIP6 destination VMAC is unknown; receive a frame from the peer first",
                    ))
                })?;
        encode_bvlc6_original_unicast(&mut buf, &self.source_vmac, &dest_vmac, npdu)?;

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

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        // A B/IPv6 MAC is 16 address octets + 2 port octets. The broadcast
        // spelling is any of the well-known BACnet multicast groups
        // (Clause U.4); the port octets do not decide broadcast-ness.
        if mac.len() != 18 {
            return false;
        }
        [
            BACNET_IPV6_MULTICAST_LINK_LOCAL,
            BACNET_IPV6_MULTICAST_SITE_LOCAL,
            BACNET_IPV6_MULTICAST_ORG_LOCAL,
        ]
        .iter()
        .any(|group| mac[..16] == group.octets())
    }
}
