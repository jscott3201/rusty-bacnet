//! BACnet/IP over UDP transport (Annex J).
//!
//! Wraps a tokio UDP socket with BVLL framing. The recv loop decodes
//! incoming BVLL frames and extracts NPDU bytes + source MAC for the
//! network layer. Optionally acts as a BBMD or foreign device.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use bacnet_types::enums::{BvlcFunction, BvlcResultCode};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;

use crate::bbmd::{self, BbmdState, BdtEntry, FdtEntryWire};
use crate::bvll::{
    self, decode_bip_mac, decode_bvll, encode_bip_mac, encode_bvll, encode_bvll_forwarded,
    BvllMessage,
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

/// Pre-start configuration for BBMD mode.
struct BbmdConfig {
    initial_bdt: Vec<BdtEntry>,
    management_acl: Vec<[u8; 4]>,
}

/// BACnet/IP transport over UDP.
pub struct BipTransport {
    interface: Ipv4Addr,
    port: u16,
    broadcast_address: Ipv4Addr,
    local_mac: [u8; 6],
    socket: Option<Arc<UdpSocket>>,
    recv_task: Option<JoinHandle<()>>,
    /// BBMD configuration before start (consumed by `start()`).
    bbmd_config: Option<BbmdConfig>,
    /// BBMD state (when acting as a BBMD, created in `start()`).
    bbmd: Option<Arc<Mutex<BbmdState>>>,
    /// Foreign device config (when registered as a foreign device).
    foreign_device: Option<ForeignDeviceConfig>,
    /// Re-registration timer task.
    registration_task: Option<JoinHandle<()>>,
    /// Shared oneshot channel for routing BVLC management responses back to the caller.
    bvlc_response_tx: Arc<Mutex<Option<oneshot::Sender<BvllMessage>>>>,
    /// Optional path for persisting the BDT across restarts.
    bdt_persist_path: Option<std::path::PathBuf>,
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
            bbmd_config: None,
            bbmd: None,
            foreign_device: None,
            registration_task: None,
            bvlc_response_tx: Arc::new(Mutex::new(None)),
            bdt_persist_path: None,
        }
    }

    /// Enable BBMD mode with the given initial BDT.
    /// Must be called before `start()`.
    pub fn enable_bbmd(&mut self, bdt: Vec<BdtEntry>) {
        self.bbmd_config = Some(BbmdConfig {
            initial_bdt: bdt,
            management_acl: Vec::new(),
        });
    }

    /// Set the path for persisting the BDT across restarts.
    /// Must be called before `start()`. The BDT is stored using the wire encoding
    /// (10 bytes per entry) — no additional serialization dependencies needed.
    pub fn set_bdt_persist_path(&mut self, path: std::path::PathBuf) {
        self.bdt_persist_path = Some(path);
    }

    /// Set the management ACL for BBMD mode.
    /// Must be called after `enable_bbmd()` and before `start()`.
    pub fn set_bbmd_management_acl(&mut self, acl: Vec<[u8; 4]>) {
        if let Some(config) = &mut self.bbmd_config {
            config.management_acl = acl;
        } else {
            // Log a warning if called before `enable_bbmd()` so misconfiguration
            // does not fail silently.
            warn!("set_bbmd_management_acl called before enable_bbmd(); ACL will be ignored");
        }
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

    /// Timeout for BVLC management response waiting.
    const BVLC_RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);

    /// Get the socket, returning an error if not started.
    fn require_socket(&self) -> Result<&Arc<UdpSocket>, Error> {
        self.socket.as_ref().ok_or_else(|| {
            Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "Transport not started",
            ))
        })
    }

    /// Send a raw BVLC management request and await the response.
    async fn bvlc_request(
        &self,
        target: &[u8],
        function: BvlcFunction,
        payload: &[u8],
    ) -> Result<BvllMessage, Error> {
        let socket = self.require_socket()?;
        let (ip, port) = decode_bip_mac(target)?;
        let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);

        let (tx, rx) = oneshot::channel();
        {
            let mut slot = self.bvlc_response_tx.lock().await;
            if slot.is_some() {
                return Err(Error::Encoding(
                    "BVLC management request already in flight".into(),
                ));
            }
            *slot = Some(tx);
        }

        let mut buf = BytesMut::with_capacity(4 + payload.len());
        encode_bvll(&mut buf, function, payload);
        socket.send_to(&buf, dest).await.map_err(Error::Transport)?;

        match tokio::time::timeout(Self::BVLC_RESPONSE_TIMEOUT, rx).await {
            Ok(Ok(msg)) => Ok(msg),
            Ok(Err(_)) => Err(Error::Encoding("BVLC response channel dropped".to_string())),
            Err(_) => {
                let mut slot = self.bvlc_response_tx.lock().await;
                *slot = None;
                Err(Error::Timeout(Self::BVLC_RESPONSE_TIMEOUT))
            }
        }
    }

    /// Send Read-Broadcast-Distribution-Table and return the response entries.
    pub async fn read_bdt(&self, target: &[u8]) -> Result<Vec<BdtEntry>, Error> {
        let msg = self
            .bvlc_request(target, BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE, &[])
            .await?;
        if msg.function == BvlcFunction::BVLC_RESULT {
            let code = if msg.payload.len() >= 2 {
                BvlcResultCode::from_raw(u16::from_be_bytes([msg.payload[0], msg.payload[1]]))
            } else {
                BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK
            };
            return Err(Error::Encoding(format!("BVLC-Result: {code:?}")));
        }
        BbmdState::decode_bdt(&msg.payload)
    }

    /// Send Write-Broadcast-Distribution-Table and return the result code.
    pub async fn write_bdt(
        &self,
        target: &[u8],
        entries: &[BdtEntry],
    ) -> Result<BvlcResultCode, Error> {
        let mut payload = BytesMut::with_capacity(entries.len() * bbmd::BDT_ENTRY_SIZE);
        bbmd::encode_bdt_entries(entries, &mut payload);
        let msg = self
            .bvlc_request(
                target,
                BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE,
                &payload,
            )
            .await?;
        if msg.payload.len() >= 2 {
            Ok(BvlcResultCode::from_raw(u16::from_be_bytes([
                msg.payload[0],
                msg.payload[1],
            ])))
        } else {
            Err(Error::Encoding("BVLC-Result too short".to_string()))
        }
    }

    /// Send Read-Foreign-Device-Table and return the response entries.
    pub async fn read_fdt(&self, target: &[u8]) -> Result<Vec<FdtEntryWire>, Error> {
        let msg = self
            .bvlc_request(target, BvlcFunction::READ_FOREIGN_DEVICE_TABLE, &[])
            .await?;
        if msg.function == BvlcFunction::BVLC_RESULT {
            let code = if msg.payload.len() >= 2 {
                BvlcResultCode::from_raw(u16::from_be_bytes([msg.payload[0], msg.payload[1]]))
            } else {
                BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK
            };
            return Err(Error::Encoding(format!("BVLC-Result: {code:?}")));
        }
        bbmd::decode_fdt(&msg.payload)
    }

    /// Send Delete-Foreign-Device-Table-Entry and return the result code.
    pub async fn delete_fdt_entry(
        &self,
        target: &[u8],
        ip: [u8; 4],
        port: u16,
    ) -> Result<BvlcResultCode, Error> {
        let mut payload = BytesMut::with_capacity(6);
        payload.extend_from_slice(&ip);
        payload.extend_from_slice(&port.to_be_bytes());
        let msg = self
            .bvlc_request(
                target,
                BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
                &payload,
            )
            .await?;
        if msg.payload.len() >= 2 {
            Ok(BvlcResultCode::from_raw(u16::from_be_bytes([
                msg.payload[0],
                msg.payload[1],
            ])))
        } else {
            Err(Error::Encoding("BVLC-Result too short".to_string()))
        }
    }

    /// Send a Register-Foreign-Device BVLC message to a BBMD and return the result code.
    ///
    /// This is a low-level BVLC management operation. It does NOT configure this
    /// transport as a foreign device for broadcast behavior (use
    /// [`register_as_foreign_device`] before `start()` for that).
    pub async fn register_foreign_device_bvlc(
        &self,
        target: &[u8],
        ttl: u16,
    ) -> Result<BvlcResultCode, Error> {
        let payload = ttl.to_be_bytes();
        let msg = self
            .bvlc_request(target, BvlcFunction::REGISTER_FOREIGN_DEVICE, &payload)
            .await?;
        if msg.payload.len() >= 2 {
            Ok(BvlcResultCode::from_raw(u16::from_be_bytes([
                msg.payload[0],
                msg.payload[1],
            ])))
        } else {
            Err(Error::Encoding("BVLC-Result too short".to_string()))
        }
    }
}

impl TransportPort for BipTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
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

        let local_ip = if self.interface.is_unspecified() {
            resolve_local_ip().unwrap_or(Ipv4Addr::LOCALHOST)
        } else {
            self.interface
        };

        let local_port = socket.local_addr().map_err(Error::Transport)?.port();
        self.port = local_port;

        self.local_mac = encode_bip_mac(local_ip.octets(), local_port);

        let socket = Arc::new(socket);
        self.socket = Some(Arc::clone(&socket));

        if let Some(config) = self.bbmd_config.take() {
            let mut state = BbmdState::new(local_ip.octets(), local_port);
            // Try loading persisted BDT; fall back to initial config BDT
            let initial_bdt = if let Some(ref path) = self.bdt_persist_path {
                match std::fs::read(path) {
                    Ok(data) => match BbmdState::decode_bdt(&data) {
                        Ok(entries) => {
                            debug!(
                                path = %path.display(),
                                entries = entries.len(),
                                "Loaded persisted BDT"
                            );
                            entries
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to decode persisted BDT, using config");
                            config.initial_bdt
                        }
                    },
                    Err(_) => config.initial_bdt,
                }
            } else {
                config.initial_bdt
            };
            if let Err(e) = state.set_bdt(initial_bdt) {
                return Err(Error::Encoding(format!("BDT configuration error: {e}")));
            }
            state.set_management_acl(config.management_acl);
            self.bbmd = Some(Arc::new(Mutex::new(state)));
        }

        let (npdu_tx, rx) = mpsc::channel(256);

        let recv_ctx = RecvContext {
            local_mac: self.local_mac,
            socket: Arc::clone(&socket),
            npdu_tx,
            bbmd: self.bbmd.clone(),
            broadcast_addr: self.broadcast_address,
            broadcast_port: self.port,
            bvlc_response: self.bvlc_response_tx.clone(),
            bdt_persist_path: self.bdt_persist_path.clone(),
        };

        let recv_task = tokio::spawn(async move {
            let mut recv_buf = vec![0u8; 2048];
            loop {
                match recv_ctx.socket.recv_from(&mut recv_buf).await {
                    Ok((len, addr)) => {
                        let data = &recv_buf[..len];
                        match decode_bvll(data) {
                            Ok(msg) => {
                                let sender_addr = if let std::net::SocketAddr::V4(v4) = addr {
                                    (v4.ip().octets(), v4.port())
                                } else {
                                    continue;
                                };

                                handle_bvll_message(&msg, sender_addr, &recv_ctx).await;
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

        if let Some(fd) = &self.foreign_device {
            let bbmd_addr = SocketAddrV4::new(fd.bbmd_ip, fd.bbmd_port);
            let ttl = fd.ttl;
            let sock = self.socket.as_ref().unwrap().clone();

            send_register_foreign_device(&sock, bbmd_addr, ttl).await;

            // Re-register at TTL/2 interval
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
        let socket = self.require_socket()?;

        let (ip, port) = decode_bip_mac(mac)?;
        let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);

        let mut buf = BytesMut::with_capacity(4 + npdu.len());
        encode_bvll(&mut buf, BvlcFunction::ORIGINAL_UNICAST_NPDU, npdu);

        socket.send_to(&buf, dest).await.map_err(Error::Transport)?;

        Ok(())
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        let socket = self.require_socket()?;

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

/// Context for the BIP receive loop — holds all shared state needed to
/// process incoming BVLL messages.
struct RecvContext {
    local_mac: [u8; 6],
    socket: Arc<UdpSocket>,
    npdu_tx: mpsc::Sender<ReceivedNpdu>,
    bbmd: Option<Arc<Mutex<BbmdState>>>,
    broadcast_addr: Ipv4Addr,
    broadcast_port: u16,
    bvlc_response: Arc<Mutex<Option<oneshot::Sender<BvllMessage>>>>,
    bdt_persist_path: Option<std::path::PathBuf>,
}

/// Handle a decoded BVLL message in the recv loop.
async fn handle_bvll_message(msg: &bvll::BvllMessage, sender: ([u8; 4], u16), ctx: &RecvContext) {
    match msg.function {
        f if f == BvlcFunction::ORIGINAL_UNICAST_NPDU => {
            let source_mac = MacAddr::from(encode_bip_mac(sender.0, sender.1));
            if *source_mac == ctx.local_mac[..] {
                return;
            }
            let _ = ctx
                .npdu_tx
                .send(ReceivedNpdu {
                    npdu: msg.payload.clone(),
                    source_mac,
                    reply_tx: None,
                })
                .await;
        }

        f if f == BvlcFunction::ORIGINAL_BROADCAST_NPDU => {
            let source_mac = MacAddr::from(encode_bip_mac(sender.0, sender.1));
            if *source_mac == ctx.local_mac[..] {
                return;
            }

            let _ = ctx
                .npdu_tx
                .send(ReceivedNpdu {
                    npdu: msg.payload.clone(),
                    source_mac,
                    reply_tx: None,
                })
                .await;

            // If BBMD, forward as Forwarded-NPDU to BDT peers + FDT entries
            if let Some(bbmd) = &ctx.bbmd {
                let targets = {
                    let mut state = bbmd.lock().await;
                    state.forwarding_targets(sender.0, sender.1)
                };
                forward_npdu(&ctx.socket, &msg.payload, sender.0, sender.1, &targets).await;

                // Re-broadcast on local subnet as Forwarded-NPDU.
                let dest = SocketAddrV4::new(ctx.broadcast_addr, ctx.broadcast_port);
                let mut buf = BytesMut::with_capacity(10 + msg.payload.len());
                encode_bvll_forwarded(&mut buf, sender.0, sender.1, &msg.payload);
                let _ = ctx.socket.send_to(&buf, dest).await;
            }
        }

        f if f == BvlcFunction::FORWARDED_NPDU => {
            // BBMD mode: use originating_ip as source_mac (same subnet, directly reachable).
            // Non-BBMD mode: use actual UDP sender as source_mac (originator may be behind NAT).
            let source_mac =
                if let (Some(ip), Some(port)) = (msg.originating_ip, msg.originating_port) {
                    MacAddr::from(encode_bip_mac(ip, port))
                } else {
                    return;
                };
            if *source_mac == ctx.local_mac[..] {
                return;
            }

            // BBMD mode: only accept FORWARDED_NPDU from BDT peers
            if let Some(bbmd) = &ctx.bbmd {
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

                let _ = ctx
                    .npdu_tx
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
                forward_npdu(&ctx.socket, &msg.payload, orig_ip, orig_port, &fdt_targets).await;

                // Re-broadcast on local subnet as Forwarded-NPDU
                let dest = SocketAddrV4::new(ctx.broadcast_addr, ctx.broadcast_port);
                let mut buf = BytesMut::with_capacity(10 + msg.payload.len());
                encode_bvll_forwarded(&mut buf, orig_ip, orig_port, &msg.payload);
                let _ = ctx.socket.send_to(&buf, dest).await;
            } else {
                // Non-BBMD: use originating address as source_mac (spec J.2.5).
                let _ = ctx
                    .npdu_tx
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
            if *source_mac == ctx.local_mac[..] {
                return;
            }

            // If BBMD, verify sender is a registered foreign device
            if let Some(bbmd) = &ctx.bbmd {
                let is_registered = {
                    let mut state = bbmd.lock().await;
                    state.is_registered_foreign_device(sender.0, sender.1)
                };
                if !is_registered {
                    debug!("Rejecting DISTRIBUTE_BROADCAST_TO_NETWORK from non-registered sender {:?}:{}",
                        Ipv4Addr::from(sender.0), sender.1);
                    send_bvlc_result(
                        &ctx.socket,
                        sender,
                        BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK,
                    )
                    .await;
                    return;
                }

                let _ = ctx
                    .npdu_tx
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
                forward_npdu(&ctx.socket, &msg.payload, sender.0, sender.1, &targets).await;

                // Broadcast locally as Forwarded-NPDU
                let dest = SocketAddrV4::new(ctx.broadcast_addr, ctx.broadcast_port);
                let mut buf = BytesMut::with_capacity(10 + msg.payload.len());
                encode_bvll_forwarded(&mut buf, sender.0, sender.1, &msg.payload);
                let _ = ctx.socket.send_to(&buf, dest).await;
            } else {
                // Non-BBMD: reject with NAK (spec J.4.5)
                send_bvlc_result(
                    &ctx.socket,
                    sender,
                    BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK,
                )
                .await;
            }
        }

        f if f == BvlcFunction::REGISTER_FOREIGN_DEVICE => {
            if let Some(bbmd) = &ctx.bbmd {
                if msg.payload.len() < 2 {
                    send_bvlc_result(
                        &ctx.socket,
                        sender,
                        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK,
                    )
                    .await;
                    return;
                }
                let ttl = u16::from_be_bytes([msg.payload[0], msg.payload[1]]);
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
                send_bvlc_result(&ctx.socket, sender, result).await;
            } else {
                send_bvlc_result(
                    &ctx.socket,
                    sender,
                    BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK,
                )
                .await;
            }
        }

        f if f == BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE => {
            if let Some(bbmd) = &ctx.bbmd {
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
                let _ = ctx.socket.send_to(&buf, dest).await;
            } else {
                send_bvlc_result(
                    &ctx.socket,
                    sender,
                    BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK,
                )
                .await;
            }
        }

        f if f == BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE => {
            if let Some(bbmd) = &ctx.bbmd {
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
                        &ctx.socket,
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
                                    // Persist BDT to disk if configured
                                    if let Some(ref path) = ctx.bdt_persist_path {
                                        let mut buf = BytesMut::new();
                                        state.encode_bdt(&mut buf);
                                        if let Err(e) = std::fs::write(path, &buf) {
                                            warn!(
                                                error = %e,
                                                path = %path.display(),
                                                "Failed to persist BDT"
                                            );
                                        }
                                    }
                                    send_bvlc_result(
                                        &ctx.socket,
                                        sender,
                                        BvlcResultCode::SUCCESSFUL_COMPLETION,
                                    )
                                    .await;
                                }
                                Err(_) => {
                                    send_bvlc_result(
                                        &ctx.socket,
                                        sender,
                                        BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK,
                                    )
                                    .await;
                                }
                            }
                        }
                        Err(_) => {
                            send_bvlc_result(
                                &ctx.socket,
                                sender,
                                BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK,
                            )
                            .await;
                        }
                    }
                }
            } else {
                send_bvlc_result(
                    &ctx.socket,
                    sender,
                    BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK,
                )
                .await;
            }
        }

        f if f == BvlcFunction::READ_FOREIGN_DEVICE_TABLE => {
            if let Some(bbmd) = &ctx.bbmd {
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
                let _ = ctx.socket.send_to(&buf, dest).await;
            } else {
                send_bvlc_result(
                    &ctx.socket,
                    sender,
                    BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK,
                )
                .await;
            }
        }

        f if f == BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY => {
            if let Some(bbmd) = &ctx.bbmd {
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
                        &ctx.socket,
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
                    send_bvlc_result(&ctx.socket, sender, result).await;
                } else {
                    send_bvlc_result(
                        &ctx.socket,
                        sender,
                        BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK,
                    )
                    .await;
                }
            } else {
                send_bvlc_result(
                    &ctx.socket,
                    sender,
                    BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK,
                )
                .await;
            }
        }

        f if f == BvlcFunction::BVLC_RESULT => {
            let sender_opt = {
                let mut slot = ctx.bvlc_response.lock().await;
                slot.take()
            };
            if let Some(response_tx) = sender_opt {
                let _ = response_tx.send(msg.clone());
            } else if msg.payload.len() >= 2 {
                let code =
                    BvlcResultCode::from_raw(u16::from_be_bytes([msg.payload[0], msg.payload[1]]));
                if code != BvlcResultCode::SUCCESSFUL_COMPLETION {
                    warn!(code = ?code, "Received BVLC-Result NAK");
                } else {
                    debug!("Received BVLC-Result: successful");
                }
            }
        }

        f if f == BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK => {
            let sender_opt = {
                let mut slot = ctx.bvlc_response.lock().await;
                slot.take()
            };
            if let Some(response_tx) = sender_opt {
                let _ = response_tx.send(msg.clone());
            } else {
                debug!("Received Read-BDT-ACK with no pending request");
            }
        }

        f if f == BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK => {
            let sender_opt = {
                let mut slot = ctx.bvlc_response.lock().await;
                slot.take()
            };
            if let Some(response_tx) = sender_opt {
                let _ = response_tx.send(msg.clone());
            } else {
                debug!("Received Read-FDT-ACK with no pending request");
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
///
/// Yields between sends for large target lists to avoid starving the recv loop
/// when there are many FDT entries (up to 512).
async fn forward_npdu(
    socket: &UdpSocket,
    npdu: &[u8],
    orig_ip: [u8; 4],
    orig_port: u16,
    targets: &[([u8; 4], u16)],
) {
    if targets.is_empty() {
        return;
    }
    let mut buf = BytesMut::with_capacity(10 + npdu.len());
    encode_bvll_forwarded(&mut buf, orig_ip, orig_port, npdu);
    let frame = buf.freeze();

    for (i, &(ip, port)) in targets.iter().enumerate() {
        let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);
        if let Err(e) = socket.send_to(&frame, dest).await {
            warn!(error = %e, dest = %dest, "Failed to forward NPDU");
        }
        // Yield every 32 sends to let the recv loop process incoming packets
        if i % 32 == 31 {
            tokio::task::yield_now().await;
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
    async fn read_bdt_from_bbmd() {
        // Start a BBMD with a known BDT
        let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let initial_bdt = vec![BdtEntry {
            ip: [10, 0, 0, 1],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 0],
        }];
        bbmd_transport.enable_bbmd(initial_bdt.clone());
        let _bbmd_rx = bbmd_transport.start().await.unwrap();
        let bbmd_mac = bbmd_transport.local_mac().to_vec();

        // Start a second transport (client) to query the BBMD
        let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let _client_rx = client_transport.start().await.unwrap();

        // Read the BDT — includes the configured entry plus the auto-inserted self entry
        let bdt = client_transport.read_bdt(&bbmd_mac).await.unwrap();
        assert_eq!(bdt.len(), 2);
        assert!(bdt
            .iter()
            .any(|e| e.ip == [10, 0, 0, 1] && e.port == 0xBAC0));
        // Self entry is also present (auto-inserted by set_bdt)
        assert!(bdt.len() >= 2);

        client_transport.stop().await.unwrap();
        bbmd_transport.stop().await.unwrap();
    }

    #[tokio::test]
    async fn read_fdt_from_bbmd() {
        // Start a BBMD
        let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        bbmd_transport.enable_bbmd(vec![]);
        let _bbmd_rx = bbmd_transport.start().await.unwrap();
        let bbmd_mac = bbmd_transport.local_mac().to_vec();
        let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();

        // Register a foreign device
        let mut fd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        fd_transport.register_as_foreign_device(ForeignDeviceConfig {
            bbmd_ip: Ipv4Addr::from(bbmd_ip),
            bbmd_port,
            ttl: 120,
        });
        let _fd_rx = fd_transport.start().await.unwrap();
        let fd_mac = fd_transport.local_mac().to_vec();
        let (fd_ip, fd_port) = decode_bip_mac(&fd_mac).unwrap();

        // Wait for registration to be processed
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Start a third transport to query the FDT
        let mut query_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let _query_rx = query_transport.start().await.unwrap();

        let fdt = query_transport.read_fdt(&bbmd_mac).await.unwrap();
        assert_eq!(fdt.len(), 1);
        assert_eq!(fdt[0].ip, fd_ip);
        assert_eq!(fdt[0].port, fd_port);
        assert_eq!(fdt[0].ttl, 120);
        assert!(fdt[0].seconds_remaining <= 150);

        query_transport.stop().await.unwrap();
        fd_transport.stop().await.unwrap();
        bbmd_transport.stop().await.unwrap();
    }

    #[tokio::test]
    async fn write_bdt_to_bbmd() {
        let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        bbmd_transport.enable_bbmd(vec![]);
        let _bbmd_rx = bbmd_transport.start().await.unwrap();
        let bbmd_mac = bbmd_transport.local_mac().to_vec();

        let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let _client_rx = client_transport.start().await.unwrap();

        let new_bdt = vec![BdtEntry {
            ip: [192, 168, 1, 1],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 255],
        }];
        let result = client_transport
            .write_bdt(&bbmd_mac, &new_bdt)
            .await
            .unwrap();
        assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);

        // Verify by reading back — includes written entry plus auto-inserted self
        let bdt = client_transport.read_bdt(&bbmd_mac).await.unwrap();
        assert!(bdt
            .iter()
            .any(|e| e.ip == [192, 168, 1, 1] && e.port == 0xBAC0));

        client_transport.stop().await.unwrap();
        bbmd_transport.stop().await.unwrap();
    }

    #[tokio::test]
    async fn register_foreign_device_via_bvlc() {
        let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        bbmd_transport.enable_bbmd(vec![]);
        let _bbmd_rx = bbmd_transport.start().await.unwrap();
        let bbmd_mac = bbmd_transport.local_mac().to_vec();

        let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let _client_rx = client_transport.start().await.unwrap();

        let result = client_transport
            .register_foreign_device_bvlc(&bbmd_mac, 60)
            .await
            .unwrap();
        assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);

        client_transport.stop().await.unwrap();
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

    #[tokio::test]
    async fn bbmd_management_acl_preserved_after_start() {
        let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        transport.enable_bbmd(vec![]);
        transport.set_bbmd_management_acl(vec![[10, 0, 0, 1]]);
        let _rx = transport.start().await.unwrap();

        {
            let state = transport.bbmd_state().unwrap();
            let s = state.lock().await;
            assert!(s.is_management_allowed(&[10, 0, 0, 1]));
            assert!(!s.is_management_allowed(&[10, 0, 0, 2]));
        }

        transport.stop().await.unwrap();
    }

    #[tokio::test]
    async fn bvlc_request_rejects_concurrent_calls() {
        let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let _rx = transport.start().await.unwrap();

        // Manually install a pending sender to simulate an in-flight request
        {
            let (tx, _rx) = oneshot::channel();
            let mut slot = transport.bvlc_response_tx.lock().await;
            *slot = Some(tx);
        }

        // A second request should fail immediately
        let fake_target = transport.local_mac().to_vec();
        let result = transport.read_bdt(&fake_target).await;
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("already in flight"),
            "expected 'already in flight' error, got: {err}"
        );

        transport.stop().await.unwrap();
    }
}
