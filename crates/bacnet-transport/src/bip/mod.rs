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

use crate::bbmd::{self, BbmdState, BdtEntry, FdtEntryWire};
use crate::bvll::{decode_bip_mac, decode_bvll, encode_bip_mac, encode_bvll, BvllMessage};
use crate::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::{BvlcFunction, BvlcResultCode};
use bacnet_types::error::Error;

mod io;
use io::{handle_bvll_message, resolve_local_ip, send_register_foreign_device, RecvContext};

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
        encode_bvll(&mut buf, function, payload)?;
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
        if self.recv_task.is_some() {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "BIP transport already started",
            )));
        }

        let socket2 = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )
        .map_err(Error::Transport)?;

        socket2.set_reuse_address(true).map_err(Error::Transport)?;
        socket2.set_broadcast(true).map_err(Error::Transport)?;
        socket2.set_nonblocking(true).map_err(Error::Transport)?;

        // Always bind to INADDR_ANY so subnet- and limited-broadcast packets
        // (destination 10.x.y.255 / 255.255.255.255) reach this socket.  A
        // Linux UDP socket bound to a specific interface IP only receives
        // packets whose destination IP matches the bound IP, so binding to
        // self.interface would silently drop every inbound broadcast — see
        // tests::bind_address_is_inaddr_any.  `self.interface` is still used
        // below for the announced local MAC (line 318), so I-Am responses
        // continue to advertise the correct source IP.
        let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, self.port);
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

        /// NPDU receive channel capacity for high-throughput UDP transports.
        const NPDU_CHANNEL_CAPACITY: usize = 256;

        let (npdu_tx, rx) = mpsc::channel(NPDU_CHANNEL_CAPACITY);

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
        encode_bvll(&mut buf, BvlcFunction::ORIGINAL_UNICAST_NPDU, npdu)?;

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
            )?;
            socket
                .send_to(&buf, bbmd_addr)
                .await
                .map_err(Error::Transport)?;
            return Ok(());
        }

        let dest = SocketAddrV4::new(self.broadcast_address, self.port);

        let mut buf = BytesMut::with_capacity(4 + npdu.len());
        encode_bvll(&mut buf, BvlcFunction::ORIGINAL_BROADCAST_NPDU, npdu)?;

        socket.send_to(&buf, dest).await.map_err(Error::Transport)?;

        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }
}

#[cfg(test)]
mod tests;
