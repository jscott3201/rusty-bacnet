//! BACnet/IP over UDP transport (Annex J).
//!
//! Wraps a tokio UDP socket with BVLL framing. The recv loop decodes
//! incoming BVLL frames and extracts NPDU bytes + source MAC for the
//! network layer. Optionally acts as a BBMD or foreign device.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

use bytes::BytesMut;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use bacnet_types::enums::{BvlcFunction, BvlcResultCode};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;

use crate::bbmd::{BbmdState, BdtEntry};
use crate::bvll::{
    decode_bip_mac, decode_bvll, encode_bip_mac, encode_bvll, encode_bvll_forwarded,
};
use crate::port::{ReceivedNpdu, TransportPort};

/// Default BACnet/IP port (0xBAC0 = 47808).
pub const DEFAULT_BACNET_PORT: u16 = 0xBAC0;

/// Configuration for foreign device registration.
#[derive(Debug, Clone)]
pub struct ForeignDeviceConfig {
    /// BBMD IP address to register with.
    pub bbmd_ip: Ipv4Addr,
    /// BBMD port.
    pub bbmd_port: u16,
    /// Time-to-live in seconds.
    pub ttl: u16,
}

/// BACnet/IP transport over UDP.
pub struct BipTransport {
    interface: Ipv4Addr,
    port: u16,
    broadcast_address: Ipv4Addr,
    local_mac: [u8; 6],
    socket: Option<Arc<UdpSocket>>,
    recv_task: Option<JoinHandle<()>>,
    /// BBMD state (when acting as a BBMD).
    bbmd: Option<Arc<Mutex<BbmdState>>>,
    /// Foreign device config (when registered as a foreign device).
    foreign_device: Option<ForeignDeviceConfig>,
    /// Re-registration timer task.
    registration_task: Option<JoinHandle<()>>,
}

impl BipTransport {
    /// Create a new BACnet/IP transport.
    ///
    /// - `interface`: Local IP to bind (use `0.0.0.0` for all interfaces)
    /// - `port`: UDP port (default 47808 / 0xBAC0)
    /// - `broadcast_address`: Directed broadcast address (e.g., `255.255.255.255`)
    pub fn new(interface: Ipv4Addr, port: u16, broadcast_address: Ipv4Addr) -> Self {
        Self {
            interface,
            port,
            broadcast_address,
            local_mac: [0; 6],
            socket: None,
            recv_task: None,
            bbmd: None,
            foreign_device: None,
            registration_task: None,
        }
    }

    /// Enable BBMD mode with the given initial BDT.
    /// Must be called before `start()`.
    pub fn enable_bbmd(&mut self, bdt: Vec<BdtEntry>) {
        // BbmdState needs local address, which isn't known until start().
        // Store the BDT and create the state in start().
        self.bbmd = Some(Arc::new(Mutex::new(BbmdState::new([0; 4], 0))));
        // We'll set the BDT after we know the local address.
        // Store it temporarily by setting it on a dummy state.
        // Actually, let's just create a proper state in start() and store the BDT config.
        // For now, store the BDT entries for later.
        let state = self.bbmd.as_ref().unwrap();
        let mut state = state.try_lock().unwrap();
        state.set_bdt(bdt).expect("BDT size within limits");
    }

    /// Configure this transport as a foreign device.
    /// Must be called before `start()`.
    pub fn register_as_foreign_device(&mut self, config: ForeignDeviceConfig) {
        self.foreign_device = Some(config);
    }

    /// Get the BBMD state (if BBMD mode is enabled).
    pub fn bbmd_state(&self) -> Option<&Arc<Mutex<BbmdState>>> {
        self.bbmd.as_ref()
    }
}

impl TransportPort for BipTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        // Create socket with socket2 for SO_REUSEADDR and SO_BROADCAST
        let socket2 = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )
        .map_err(Error::Transport)?;

        socket2.set_reuse_address(true).map_err(Error::Transport)?;
        socket2.set_broadcast(true).map_err(Error::Transport)?;
        socket2.set_nonblocking(true).map_err(Error::Transport)?;

        let bind_addr = SocketAddrV4::new(self.interface, self.port);
        socket2.bind(&bind_addr.into()).map_err(Error::Transport)?;

        let std_socket: std::net::UdpSocket = socket2.into();
        let socket = UdpSocket::from_std(std_socket).map_err(Error::Transport)?;

        // Resolve actual local IP if bound to 0.0.0.0
        let local_ip = if self.interface.is_unspecified() {
            resolve_local_ip().unwrap_or(Ipv4Addr::LOCALHOST)
        } else {
            self.interface
        };

        // Resolve actual bound port (in case port was 0)
        let local_port = socket.local_addr().map_err(Error::Transport)?.port();
        self.port = local_port;

        self.local_mac = encode_bip_mac(local_ip.octets(), local_port);

        let socket = Arc::new(socket);
        self.socket = Some(Arc::clone(&socket));

        // Update BBMD state with actual local address
        if let Some(bbmd) = &self.bbmd {
            let mut state = bbmd.lock().await;
            let old_bdt = state.bdt().to_vec();
            *state = BbmdState::new(local_ip.octets(), local_port);
            state.set_bdt(old_bdt).expect("restoring existing BDT");
        }

        let (tx, rx) = mpsc::channel(256);
        let local_mac = self.local_mac;
        let bbmd_for_recv = self.bbmd.clone();
        let broadcast_addr = self.broadcast_address;
        let broadcast_port = self.port;

        // Spawn the receive loop
        let recv_task = tokio::spawn(async move {
            let mut recv_buf = vec![0u8; 2048];
            loop {
                match socket.recv_from(&mut recv_buf).await {
                    Ok((len, addr)) => {
                        let data = &recv_buf[..len];
                        match decode_bvll(data) {
                            Ok(msg) => {
                                let sender_addr = if let std::net::SocketAddr::V4(v4) = addr {
                                    (v4.ip().octets(), v4.port())
                                } else {
                                    continue;
                                };

                                handle_bvll_message(
                                    &msg,
                                    sender_addr,
                                    local_mac,
                                    &socket,
                                    &tx,
                                    &bbmd_for_recv,
                                    broadcast_addr,
                                    broadcast_port,
                                )
                                .await;
                            }
                            Err(e) => {
                                warn!(error = %e, "Failed to decode BVLL frame");
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "UDP recv error");
                        break;
                    }
                }
            }
        });

        self.recv_task = Some(recv_task);

        // If foreign device mode, send initial registration and start timer
        if let Some(fd) = &self.foreign_device {
            let bbmd_addr = SocketAddrV4::new(fd.bbmd_ip, fd.bbmd_port);
            let ttl = fd.ttl;
            let sock = self.socket.as_ref().unwrap().clone();

            // Send initial registration
            send_register_foreign_device(&sock, bbmd_addr, ttl).await;

            // Spawn re-registration timer (re-register at TTL/2 interval)
            let interval = std::time::Duration::from_secs(((ttl as u64) / 2).max(30));
            let reg_task = tokio::spawn(async move {
                let mut ticker = tokio::time::interval(interval);
                ticker.tick().await; // Skip the first immediate tick
                loop {
                    ticker.tick().await;
                    send_register_foreign_device(&sock, bbmd_addr, ttl).await;
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

        let (ip, port) = decode_bip_mac(mac)?;
        let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);

        let mut buf = BytesMut::with_capacity(4 + npdu.len());
        encode_bvll(&mut buf, BvlcFunction::ORIGINAL_UNICAST_NPDU, npdu);

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

        // If registered as a foreign device, use Distribute-Broadcast-To-Network
        if let Some(fd) = &self.foreign_device {
            let bbmd_addr = SocketAddrV4::new(fd.bbmd_ip, fd.bbmd_port);
            let mut buf = BytesMut::with_capacity(4 + npdu.len());
            encode_bvll(
                &mut buf,
                BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
                npdu,
            );
            socket
                .send_to(&buf, bbmd_addr)
                .await
                .map_err(Error::Transport)?;
            return Ok(());
        }

        // Normal broadcast
        let dest = SocketAddrV4::new(self.broadcast_address, self.port);

        let mut buf = BytesMut::with_capacity(4 + npdu.len());
        encode_bvll(&mut buf, BvlcFunction::ORIGINAL_BROADCAST_NPDU, npdu);

        socket.send_to(&buf, dest).await.map_err(Error::Transport)?;

        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }
}

/// Send a Register-Foreign-Device message to a BBMD.
async fn send_register_foreign_device(socket: &UdpSocket, bbmd_addr: SocketAddrV4, ttl: u16) {
    let payload = ttl.to_be_bytes().to_vec();
    let mut buf = BytesMut::with_capacity(6);
    encode_bvll(&mut buf, BvlcFunction::REGISTER_FOREIGN_DEVICE, &payload);
    if let Err(e) = socket.send_to(&buf, bbmd_addr).await {
        warn!(error = %e, "Failed to send Register-Foreign-Device");
    } else {
        debug!(bbmd = %bbmd_addr, ttl = ttl, "Sent Register-Foreign-Device");
    }
}

/// Handle a decoded BVLL message in the recv loop.
#[allow(clippy::too_many_arguments)]
async fn handle_bvll_message(
    msg: &crate::bvll::BvllMessage,
    sender: ([u8; 4], u16),
    local_mac: [u8; 6],
    socket: &Arc<UdpSocket>,
    tx: &mpsc::Sender<ReceivedNpdu>,
    bbmd: &Option<Arc<Mutex<BbmdState>>>,
    broadcast_addr: Ipv4Addr,
    broadcast_port: u16,
) {
    match msg.function {
        f if f == BvlcFunction::ORIGINAL_UNICAST_NPDU => {
            let source_mac = MacAddr::from(encode_bip_mac(sender.0, sender.1));
            if *source_mac == local_mac[..] {
                return;
            }
            let _ = tx
                .send(ReceivedNpdu {
                    npdu: msg.payload.clone(),
                    source_mac,
                    reply_tx: None,
                })
                .await;
        }

        f if f == BvlcFunction::ORIGINAL_BROADCAST_NPDU => {
            let source_mac = MacAddr::from(encode_bip_mac(sender.0, sender.1));
            if *source_mac == local_mac[..] {
                return;
            }

            // Pass NPDU up to network layer
            let _ = tx
                .send(ReceivedNpdu {
                    npdu: msg.payload.clone(),
                    source_mac,
                    reply_tx: None,
                })
                .await;

            // If BBMD, forward as Forwarded-NPDU to BDT peers + FDT entries
            if let Some(bbmd) = bbmd {
                let targets = {
                    let mut state = bbmd.lock().await;
                    state.forwarding_targets(sender.0, sender.1)
                };
                forward_npdu(socket, &msg.payload, sender.0, sender.1, &targets).await;

                // Re-broadcast on local subnet as Forwarded-NPDU per J.4.2.1
                // so local devices receive the originator's B/IP address.
                let dest = SocketAddrV4::new(broadcast_addr, broadcast_port);
                let mut buf = BytesMut::with_capacity(10 + msg.payload.len());
                encode_bvll_forwarded(&mut buf, sender.0, sender.1, &msg.payload);
                let _ = socket.send_to(&buf, dest).await;
            }
        }

        f if f == BvlcFunction::FORWARDED_NPDU => {
            let source_mac =
                if let (Some(ip), Some(port)) = (msg.originating_ip, msg.originating_port) {
                    MacAddr::from(encode_bip_mac(ip, port))
                } else {
                    return;
                };
            if *source_mac == local_mac[..] {
                return;
            }

            // BBMD mode: only accept FORWARDED_NPDU from BDT peers (J.4.2.3)
            if let Some(bbmd) = bbmd {
                let is_bdt_peer = {
                    let state = bbmd.lock().await;
                    state.is_bdt_peer(sender.0, sender.1)
                };
                if !is_bdt_peer {
                    debug!(
                        "Rejecting FORWARDED_NPDU from non-BDT sender {:?}:{}",
                        Ipv4Addr::from(sender.0),
                        sender.1
                    );
                    return;
                }

                // Pass NPDU up to network layer
                let _ = tx
                    .send(ReceivedNpdu {
                        npdu: msg.payload.clone(),
                        source_mac,
                        reply_tx: None,
                    })
                    .await;

                let orig_ip = msg.originating_ip.unwrap();
                let orig_port = msg.originating_port.unwrap();

                // Forward to FDT entries (BDT peers don't need it — they got it directly)
                let fdt_targets = {
                    let mut state = bbmd.lock().await;
                    state.purge_expired();
                    state
                        .fdt()
                        .iter()
                        .filter(|e| !(e.ip == orig_ip && e.port == orig_port))
                        .map(|e| (e.ip, e.port))
                        .collect::<Vec<_>>()
                };
                forward_npdu(socket, &msg.payload, orig_ip, orig_port, &fdt_targets).await;

                // Re-broadcast on local subnet as Forwarded-NPDU
                let dest = SocketAddrV4::new(broadcast_addr, broadcast_port);
                let mut buf = BytesMut::with_capacity(10 + msg.payload.len());
                encode_bvll_forwarded(&mut buf, orig_ip, orig_port, &msg.payload);
                let _ = socket.send_to(&buf, dest).await;
            } else {
                // Non-BBMD: accept all FORWARDED_NPDU (received via local subnet broadcast)
                let _ = tx
                    .send(ReceivedNpdu {
                        npdu: msg.payload.clone(),
                        source_mac,
                        reply_tx: None,
                    })
                    .await;
            }
        }

        f if f == BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK => {
            let source_mac = MacAddr::from(encode_bip_mac(sender.0, sender.1));
            if *source_mac == local_mac[..] {
                return;
            }

            // If BBMD, verify sender is a registered foreign device (J.4.5)
            if let Some(bbmd) = bbmd {
                let is_registered = {
                    let mut state = bbmd.lock().await;
                    state.is_registered_foreign_device(sender.0, sender.1)
                };
                if !is_registered {
                    debug!("Rejecting DISTRIBUTE_BROADCAST_TO_NETWORK from non-registered sender {:?}:{}",
                        Ipv4Addr::from(sender.0), sender.1);
                    send_bvlc_result(
                        socket,
                        sender,
                        BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK,
                    )
                    .await;
                    return;
                }

                // Pass NPDU up to network layer
                let _ = tx
                    .send(ReceivedNpdu {
                        npdu: msg.payload.clone(),
                        source_mac,
                        reply_tx: None,
                    })
                    .await;

                let targets = {
                    let mut state = bbmd.lock().await;
                    state.forwarding_targets(sender.0, sender.1)
                };
                forward_npdu(socket, &msg.payload, sender.0, sender.1, &targets).await;

                // Broadcast locally as Forwarded-NPDU
                let dest = SocketAddrV4::new(broadcast_addr, broadcast_port);
                let mut buf = BytesMut::with_capacity(10 + msg.payload.len());
                encode_bvll_forwarded(&mut buf, sender.0, sender.1, &msg.payload);
                let _ = socket.send_to(&buf, dest).await;
            }
            // Non-BBMD nodes ignore DISTRIBUTE_BROADCAST_TO_NETWORK (only BBMDs handle it)
        }

        // --- BVLC Management Messages ---
        f if f == BvlcFunction::REGISTER_FOREIGN_DEVICE => {
            if let Some(bbmd) = bbmd {
                let ttl = if msg.payload.len() >= 2 {
                    u16::from_be_bytes([msg.payload[0], msg.payload[1]])
                } else {
                    0
                };
                let result = {
                    let mut state = bbmd.lock().await;
                    state.register_foreign_device(sender.0, sender.1, ttl)
                };
                debug!(
                    ip = ?Ipv4Addr::from(sender.0),
                    port = sender.1,
                    ttl = ttl,
                    "Foreign device registered"
                );
                send_bvlc_result(socket, sender, result).await;
            } else {
                send_bvlc_result(socket, sender, BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK).await;
            }
        }

        f if f == BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE => {
            if let Some(bbmd) = bbmd {
                let state = bbmd.lock().await;
                let mut payload = BytesMut::new();
                state.encode_bdt(&mut payload);
                let mut buf = BytesMut::with_capacity(4 + payload.len());
                encode_bvll(
                    &mut buf,
                    BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
                    &payload,
                );
                let dest = SocketAddrV4::new(Ipv4Addr::from(sender.0), sender.1);
                let _ = socket.send_to(&buf, dest).await;
            } else {
                send_bvlc_result(
                    socket,
                    sender,
                    BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK,
                )
                .await;
            }
        }

        f if f == BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE => {
            if let Some(bbmd) = bbmd {
                // Check management ACL before accepting Write-BDT
                let allowed = {
                    let state = bbmd.lock().await;
                    state.is_management_allowed(&sender.0)
                };
                if !allowed {
                    debug!(
                        "Rejecting Write-BDT from non-ACL sender {:?}:{}",
                        Ipv4Addr::from(sender.0),
                        sender.1
                    );
                    send_bvlc_result(
                        socket,
                        sender,
                        BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK,
                    )
                    .await;
                } else {
                    match BbmdState::decode_bdt(&msg.payload) {
                        Ok(entries) => {
                            let mut state = bbmd.lock().await;
                            match state.set_bdt(entries) {
                                Ok(()) => {
                                    send_bvlc_result(
                                        socket,
                                        sender,
                                        BvlcResultCode::SUCCESSFUL_COMPLETION,
                                    )
                                    .await;
                                }
                                Err(_) => {
                                    send_bvlc_result(
                                        socket,
                                        sender,
                                        BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK,
                                    )
                                    .await;
                                }
                            }
                        }
                        Err(_) => {
                            send_bvlc_result(
                                socket,
                                sender,
                                BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK,
                            )
                            .await;
                        }
                    }
                }
            } else {
                send_bvlc_result(
                    socket,
                    sender,
                    BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK,
                )
                .await;
            }
        }

        f if f == BvlcFunction::READ_FOREIGN_DEVICE_TABLE => {
            if let Some(bbmd) = bbmd {
                let mut state = bbmd.lock().await;
                let mut payload = BytesMut::new();
                state.encode_fdt(&mut payload);
                drop(state);
                let mut buf = BytesMut::with_capacity(4 + payload.len());
                encode_bvll(
                    &mut buf,
                    BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
                    &payload,
                );
                let dest = SocketAddrV4::new(Ipv4Addr::from(sender.0), sender.1);
                let _ = socket.send_to(&buf, dest).await;
            } else {
                send_bvlc_result(
                    socket,
                    sender,
                    BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK,
                )
                .await;
            }
        }

        f if f == BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY => {
            if let Some(bbmd) = bbmd {
                // Check management ACL before accepting Delete-FDT-Entry
                let allowed = {
                    let state = bbmd.lock().await;
                    state.is_management_allowed(&sender.0)
                };
                if !allowed {
                    debug!(
                        "Rejecting Delete-FDT-Entry from non-ACL sender {:?}:{}",
                        Ipv4Addr::from(sender.0),
                        sender.1
                    );
                    send_bvlc_result(
                        socket,
                        sender,
                        BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK,
                    )
                    .await;
                } else if msg.payload.len() >= 6 {
                    let ip = [
                        msg.payload[0],
                        msg.payload[1],
                        msg.payload[2],
                        msg.payload[3],
                    ];
                    let port = u16::from_be_bytes([msg.payload[4], msg.payload[5]]);
                    let result = {
                        let mut state = bbmd.lock().await;
                        state.delete_foreign_device(ip, port)
                    };
                    send_bvlc_result(socket, sender, result).await;
                } else {
                    send_bvlc_result(
                        socket,
                        sender,
                        BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK,
                    )
                    .await;
                }
            } else {
                send_bvlc_result(
                    socket,
                    sender,
                    BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK,
                )
                .await;
            }
        }

        f if f == BvlcFunction::BVLC_RESULT => {
            // Response to a management request we sent (e.g., Register-Foreign-Device)
            if msg.payload.len() >= 2 {
                let code =
                    BvlcResultCode::from_raw(u16::from_be_bytes([msg.payload[0], msg.payload[1]]));
                if code != BvlcResultCode::SUCCESSFUL_COMPLETION {
                    warn!(code = ?code, "Received BVLC-Result NAK");
                } else {
                    debug!("Received BVLC-Result: successful");
                }
            }
        }

        _ => {
            debug!(function = msg.function.to_raw(), "Unknown BVLC function");
        }
    }
}

/// Send a BVLC-Result to a destination.
async fn send_bvlc_result(socket: &UdpSocket, dest: ([u8; 4], u16), code: BvlcResultCode) {
    let payload = code.to_raw().to_be_bytes().to_vec();
    let mut buf = BytesMut::with_capacity(6);
    encode_bvll(&mut buf, BvlcFunction::BVLC_RESULT, &payload);
    let addr = SocketAddrV4::new(Ipv4Addr::from(dest.0), dest.1);
    let _ = socket.send_to(&buf, addr).await;
}

/// Forward an NPDU as Forwarded-NPDU to a list of targets.
async fn forward_npdu(
    socket: &UdpSocket,
    npdu: &[u8],
    orig_ip: [u8; 4],
    orig_port: u16,
    targets: &[([u8; 4], u16)],
) {
    let mut buf = BytesMut::with_capacity(10 + npdu.len());
    encode_bvll_forwarded(&mut buf, orig_ip, orig_port, npdu);
    let frame = buf.freeze();

    for &(ip, port) in targets {
        let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);
        if let Err(e) = socket.send_to(&frame, dest).await {
            warn!(error = %e, dest = %dest, "Failed to forward NPDU");
        }
    }
}

/// Resolve the local IPv4 address by connecting a UDP socket to a remote
/// address and reading back the local address. This doesn't actually send
/// any packets.
fn resolve_local_ip() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    // Connect to a public IP — doesn't actually send anything
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()? {
        std::net::SocketAddr::V4(v4) => Some(*v4.ip()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{timeout, Duration};

    #[test]
    fn bip_max_apdu_length() {
        let transport = BipTransport::new(
            std::net::Ipv4Addr::LOCALHOST,
            0,
            std::net::Ipv4Addr::LOCALHOST,
        );
        assert_eq!(transport.max_apdu_length(), 1476);
    }

    #[tokio::test]
    async fn start_stop() {
        let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let _rx = transport.start().await.unwrap();
        assert!(transport.socket.is_some());
        assert!(!transport.local_mac().iter().all(|b| *b == 0));
        transport.stop().await.unwrap();
        assert!(transport.socket.is_none());
    }

    #[tokio::test]
    async fn unicast_loopback() {
        // Two transports on localhost with ephemeral ports
        let mut transport_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let mut transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

        let _rx_a = transport_a.start().await.unwrap();
        let mut rx_b = transport_b.start().await.unwrap();

        let test_npdu = vec![0x01, 0x00, 0xDE, 0xAD, 0xBE, 0xEF];

        // A sends unicast to B
        transport_a
            .send_unicast(&test_npdu, transport_b.local_mac())
            .await
            .unwrap();

        // B should receive it
        let received = timeout(Duration::from_secs(2), rx_b.recv())
            .await
            .expect("Timed out waiting for packet")
            .expect("Channel closed");

        assert_eq!(received.npdu, test_npdu);
        assert_eq!(received.source_mac.as_slice(), transport_a.local_mac());

        transport_a.stop().await.unwrap();
        transport_b.stop().await.unwrap();
    }

    #[tokio::test]
    async fn bbmd_register_foreign_device() {
        // Start a BBMD
        let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        bbmd_transport.enable_bbmd(vec![]);
        let _bbmd_rx = bbmd_transport.start().await.unwrap();
        let bbmd_mac = bbmd_transport.local_mac().to_vec();
        let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();

        // Start a foreign device that registers with the BBMD
        let mut fd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        fd_transport.register_as_foreign_device(ForeignDeviceConfig {
            bbmd_ip: Ipv4Addr::from(bbmd_ip),
            bbmd_port,
            ttl: 60,
        });
        let _fd_rx = fd_transport.start().await.unwrap();

        // Give a moment for the registration to be processed
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify the BBMD has the foreign device in its FDT
        {
            let bbmd_state = bbmd_transport.bbmd_state().unwrap();
            let mut state = bbmd_state.lock().await;
            let fdt = state.fdt();
            assert_eq!(fdt.len(), 1);
            assert_eq!(fdt[0].ttl, 60);
        }

        fd_transport.stop().await.unwrap();
        bbmd_transport.stop().await.unwrap();
    }

    #[tokio::test]
    async fn foreign_device_broadcast_via_bbmd() {
        // BBMD
        let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        bbmd_transport.enable_bbmd(vec![]);
        let mut bbmd_rx = bbmd_transport.start().await.unwrap();
        let bbmd_mac = bbmd_transport.local_mac().to_vec();
        let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();

        // Foreign device
        let mut fd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        fd_transport.register_as_foreign_device(ForeignDeviceConfig {
            bbmd_ip: Ipv4Addr::from(bbmd_ip),
            bbmd_port,
            ttl: 60,
        });
        let _fd_rx = fd_transport.start().await.unwrap();

        // Give time for registration
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Foreign device sends a broadcast (should use Distribute-Broadcast-To-Network)
        let test_npdu = vec![0x01, 0x00, 0xAA, 0xBB];
        fd_transport.send_broadcast(&test_npdu).await.unwrap();

        // BBMD should receive it (as NPDU via Distribute-Broadcast-To-Network)
        let received = timeout(Duration::from_secs(2), bbmd_rx.recv())
            .await
            .expect("BBMD timed out")
            .expect("BBMD channel closed");

        assert_eq!(received.npdu, test_npdu);

        fd_transport.stop().await.unwrap();
        bbmd_transport.stop().await.unwrap();
    }
}
