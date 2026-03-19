//! BACnet/SC Hub — a minimal hub that accepts TLS WebSocket connections
//! from BACnet/SC nodes and relays messages between them.
//!
//! The hub performs three duties:
//! 1. **Connection handshake** — responds to `ConnectRequest` with `ConnectAccept`.
//! 2. **Message relay** — forwards `EncapsulatedNpdu` to the destination VMAC
//!    (unicast) or to all connected nodes (broadcast).
//! 3. **Heartbeat** — responds to `HeartbeatRequest` with `HeartbeatAck`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, warn};

use crate::sc_frame::{
    decode_sc_message, encode_sc_message, is_broadcast_vmac, ScFunction, ScMessage, Vmac,
    BROADCAST_VMAC,
};

type TlsStream = tokio_rustls::server::TlsStream<tokio::net::TcpStream>;
type WsSink = SplitSink<WebSocketStream<TlsStream>, Message>;

/// Shared state for the hub: connected clients keyed by VMAC.
type Clients = Arc<Mutex<HashMap<Vmac, Arc<Mutex<WsSink>>>>>;

/// A minimal BACnet/SC hub.
///
/// Listens on a TLS WebSocket port, accepts SC node connections, performs the
/// Connect-Request/Connect-Accept handshake, and relays `EncapsulatedNpdu`
/// messages between connected nodes.
pub struct ScHub {
    hub_vmac: Vmac,
    /// Device UUID (16 bytes, RFC 4122).
    #[allow(dead_code)]
    hub_uuid: [u8; 16],
    listener_task: Option<JoinHandle<()>>,
    local_addr: Option<SocketAddr>,
}

impl ScHub {
    /// Start the hub, binding to `bind_addr` (e.g. `"127.0.0.1:0"` for a
    /// random port).
    ///
    /// The hub begins accepting TLS WebSocket connections immediately on a
    /// background task.
    pub async fn start(
        bind_addr: &str,
        tls_acceptor: TlsAcceptor,
        hub_vmac: Vmac,
    ) -> Result<Self, bacnet_types::error::Error> {
        Self::start_with_uuid(bind_addr, tls_acceptor, hub_vmac, [0u8; 16]).await
    }

    /// Start the hub with a specific Device UUID.
    pub async fn start_with_uuid(
        bind_addr: &str,
        tls_acceptor: TlsAcceptor,
        hub_vmac: Vmac,
        hub_uuid: [u8; 16],
    ) -> Result<Self, bacnet_types::error::Error> {
        let listener = TcpListener::bind(bind_addr)
            .await
            .map_err(|e| bacnet_types::error::Error::Encoding(format!("Hub bind failed: {e}")))?;

        let local_addr = listener.local_addr().map_err(|e| {
            bacnet_types::error::Error::Encoding(format!("Hub could not read local address: {e}"))
        })?;

        debug!("BACnet/SC hub listening on {local_addr}");

        let clients: Clients = Arc::new(Mutex::new(HashMap::new()));

        let task = tokio::spawn(accept_loop(
            listener,
            tls_acceptor,
            hub_vmac,
            hub_uuid,
            clients,
        ));

        Ok(Self {
            hub_vmac,
            hub_uuid,
            listener_task: Some(task),
            local_addr: Some(local_addr),
        })
    }

    /// The address the hub is listening on (available after [`Self::start`]).
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.local_addr
    }

    /// The hub's own VMAC.
    pub fn hub_vmac(&self) -> Vmac {
        self.hub_vmac
    }

    /// Stop the hub, aborting the listener task.
    pub async fn stop(&mut self) {
        if let Some(task) = self.listener_task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

// ---------------------------------------------------------------------------
// Accept loop
// ---------------------------------------------------------------------------

async fn accept_loop(
    listener: TcpListener,
    tls_acceptor: TlsAcceptor,
    hub_vmac: Vmac,
    hub_uuid: [u8; 16],
    clients: Clients,
) {
    // Track active TCP connections (pre-handshake) to limit DoS surface.
    let active_connections = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    const MAX_ACTIVE_CONNECTIONS: usize = 512;

    loop {
        let (tcp_stream, peer_addr) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                warn!("Hub accept error: {e}");
                continue;
            }
        };

        // Reject if too many pre-handshake connections
        let current = active_connections.load(std::sync::atomic::Ordering::Relaxed);
        if current >= MAX_ACTIVE_CONNECTIONS {
            warn!("Hub: rejecting connection from {peer_addr} — max active connections ({MAX_ACTIVE_CONNECTIONS}) reached");
            drop(tcp_stream);
            continue;
        }
        active_connections.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        debug!("Hub: new TCP connection from {peer_addr}");

        let acceptor = tls_acceptor.clone();
        let clients = clients.clone();
        let conn_counter = active_connections.clone();

        tokio::spawn(async move {
            // Decrement connection counter when this task exits (any path).
            struct ConnGuard(std::sync::Arc<std::sync::atomic::AtomicUsize>);
            impl Drop for ConnGuard {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
            let _guard = ConnGuard(conn_counter);
            // TLS handshake
            let tls_stream = match acceptor.accept(tcp_stream).await {
                Ok(s) => s,
                Err(e) => {
                    warn!("Hub TLS handshake failed for {peer_addr}: {e}");
                    return;
                }
            };

            // WebSocket upgrade — echo the BACnet/SC subprotocol only if the client offered it.
            let ws_stream = match tokio_tungstenite::accept_hdr_async(
                tls_stream,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                 mut response: tokio_tungstenite::tungstenite::handshake::server::Response|
                 -> Result<
                    tokio_tungstenite::tungstenite::handshake::server::Response,
                    tokio_tungstenite::tungstenite::handshake::server::ErrorResponse,
                > {
                    let client_offers = request
                        .headers()
                        .get("Sec-WebSocket-Protocol")
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.split(',').any(|p| p.trim() == "hub.bsc.bacnet.org"))
                        .unwrap_or(false);
                    if client_offers {
                        response.headers_mut().insert(
                            "Sec-WebSocket-Protocol",
                            "hub.bsc.bacnet.org".parse().unwrap(),
                        );
                    }
                    Ok(response)
                },
            )
            .await
            {
                Ok(ws) => ws,
                Err(e) => {
                    warn!("Hub WebSocket upgrade failed for {peer_addr}: {e}");
                    return;
                }
            };

            let (write, read) = ws_stream.split();
            let write = Arc::new(Mutex::new(write));

            handle_client(peer_addr, hub_vmac, hub_uuid, read, write, clients).await;
        });
    }
}

// ---------------------------------------------------------------------------
// Per-client handler
// ---------------------------------------------------------------------------

async fn handle_client(
    peer_addr: SocketAddr,
    hub_vmac: Vmac,
    hub_uuid: [u8; 16],
    mut read: futures_util::stream::SplitStream<WebSocketStream<TlsStream>>,
    write: Arc<Mutex<WsSink>>,
    clients: Clients,
) {
    let mut client_vmac: Option<Vmac> = None;

    while let Some(msg_result) = read.next().await {
        let data = match msg_result {
            Ok(Message::Binary(data)) => data.to_vec(),
            Ok(Message::Close(_)) => {
                debug!("Hub: client {peer_addr} sent close");
                break;
            }
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            Ok(_) => {
                warn!("Hub: non-binary frame from {peer_addr}, closing with 1003");
                let mut w = write.lock().await;
                let _ = w
                    .send(Message::Close(Some(
                        tokio_tungstenite::tungstenite::protocol::CloseFrame {
                            code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Unsupported,
                            reason: "BACnet/SC requires binary frames".into(),
                        },
                    )))
                    .await;
                break;
            }
            Err(e) => {
                warn!("Hub: recv error from {peer_addr}: {e}");
                break;
            }
        };

        let sc_msg = match decode_sc_message(&data) {
            Ok(m) => m,
            Err(e) => {
                warn!("Hub: decode error from {peer_addr}: {e}");
                continue;
            }
        };

        match sc_msg.function {
            ScFunction::ConnectRequest => {
                let vmac = if sc_msg.payload.len() >= 6 {
                    let mut v = [0u8; 6];
                    v.copy_from_slice(&sc_msg.payload[0..6]);
                    v
                } else {
                    sc_msg.originating_vmac.unwrap_or([0; 6])
                };
                debug!("Hub: ConnectRequest from {peer_addr} vmac={vmac:02x?}");

                // Reject reserved VMACs (unknown=0x000000000000, broadcast=0xFFFFFFFFFFFF)
                if vmac == crate::sc_frame::UNKNOWN_VMAC || vmac == BROADCAST_VMAC {
                    warn!("Hub: rejecting reserved VMAC {vmac:02x?} from {peer_addr}");
                    break;
                }

                // Check for VMAC collision and register atomically under a
                // single lock to prevent TOCTOU races.
                const MAX_SC_CLIENTS: usize = 256;
                {
                    let mut map = clients.lock().await;
                    if map.contains_key(&vmac) {
                        warn!("Hub: VMAC collision for {vmac:02x?} from {peer_addr}");
                        drop(map); // release lock before sending
                        let error_result = ScMessage {
                            function: ScFunction::Result,
                            message_id: sc_msg.message_id,
                            originating_vmac: None,
                            destination_vmac: None,
                            dest_options: Vec::new(),
                            data_options: Vec::new(),
                            payload: Bytes::from(vec![
                                ScFunction::ConnectRequest.to_raw(), // result-for function
                                0x01,                                // result code = NAK
                                0x00, // error header marker (no data, type=0)
                                0x00,
                                0x01, // error_class = 1 (communication)
                                0x00,
                                0x01, // error_code = 1 (duplicate vmac)
                            ]),
                        };
                        let mut buf = BytesMut::new();
                        encode_sc_message(&mut buf, &error_result);
                        let mut w = write.lock().await;
                        let _ = w.send(Message::Binary(buf.to_vec().into())).await;
                        break;
                    }
                    if map.len() >= MAX_SC_CLIENTS {
                        warn!("SC Hub: max clients reached, rejecting connection");
                        drop(map);
                        let error_result = ScMessage {
                            function: ScFunction::Result,
                            message_id: sc_msg.message_id,
                            originating_vmac: None,
                            destination_vmac: None,
                            dest_options: Vec::new(),
                            data_options: Vec::new(),
                            payload: Bytes::from(vec![
                                ScFunction::ConnectRequest.to_raw(),
                                0x01, // NAK
                                0x00, // error header marker
                                0x00,
                                0x01, // error_class = 1 (communication)
                                0x00,
                                0x02, // error_code = 2 (other)
                            ]),
                        };
                        let mut buf = BytesMut::new();
                        encode_sc_message(&mut buf, &error_result);
                        let mut w = write.lock().await;
                        let _ = w.send(Message::Binary(buf.to_vec().into())).await;
                        break;
                    }
                    map.insert(vmac, write.clone());
                }
                client_vmac = Some(vmac);

                let mut accept_payload = Vec::with_capacity(26);
                accept_payload.extend_from_slice(&hub_vmac);
                accept_payload.extend_from_slice(&hub_uuid);
                accept_payload.extend_from_slice(&1476u16.to_be_bytes());
                accept_payload.extend_from_slice(&1476u16.to_be_bytes());
                let accept = ScMessage {
                    function: ScFunction::ConnectAccept,
                    message_id: sc_msg.message_id,
                    originating_vmac: None,
                    destination_vmac: None,
                    dest_options: Vec::new(),
                    data_options: Vec::new(),
                    payload: Bytes::from(accept_payload),
                };
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &accept);

                let mut w = write.lock().await;
                if let Err(e) = w.send(Message::Binary(buf.to_vec().into())).await {
                    warn!("Hub: failed to send ConnectAccept to {peer_addr}: {e}");
                    break;
                }
            }

            ScFunction::HeartbeatRequest => {
                let ack = ScMessage {
                    function: ScFunction::HeartbeatAck,
                    message_id: sc_msg.message_id,
                    originating_vmac: None,
                    destination_vmac: None,
                    dest_options: Vec::new(),
                    data_options: Vec::new(),
                    payload: Bytes::new(),
                };
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &ack);

                let mut w = write.lock().await;
                if let Err(e) = w.send(Message::Binary(buf.to_vec().into())).await {
                    warn!("Hub: failed to send HeartbeatAck to {peer_addr}: {e}");
                    break;
                }
            }

            ScFunction::DisconnectRequest => {
                debug!("Hub: DisconnectRequest from {peer_addr}");
                let ack = ScMessage {
                    function: ScFunction::DisconnectAck,
                    message_id: sc_msg.message_id,
                    originating_vmac: None,
                    destination_vmac: None,
                    dest_options: Vec::new(),
                    data_options: Vec::new(),
                    payload: Bytes::new(),
                };
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &ack);

                let mut w = write.lock().await;
                let _ = w.send(Message::Binary(buf.to_vec().into())).await;
                break;
            }

            ScFunction::EncapsulatedNpdu => {
                let Some(registered_vmac) = client_vmac else {
                    warn!("received EncapsulatedNpdu before ConnectRequest — dropping");
                    continue;
                };

                let sender_vmac = sc_msg.originating_vmac.unwrap_or([0; 6]);
                if sender_vmac != registered_vmac {
                    warn!(
                        "originating VMAC {:?} does not match registered {:?} — dropping",
                        sender_vmac, registered_vmac
                    );
                    continue;
                }

                let dest = sc_msg.destination_vmac.unwrap_or(BROADCAST_VMAC);

                // Hub adds Originating Virtual Address; strips Destination for unicast.
                let relay_msg = if is_broadcast_vmac(&dest) {
                    ScMessage {
                        originating_vmac: Some(sender_vmac),
                        ..sc_msg
                    }
                } else {
                    ScMessage {
                        originating_vmac: Some(sender_vmac),
                        destination_vmac: None, // strip for unicast
                        ..sc_msg
                    }
                };
                let mut relay_buf = BytesMut::new();
                encode_sc_message(&mut relay_buf, &relay_msg);
                let relay_bytes: Vec<u8> = relay_buf.to_vec();

                if is_broadcast_vmac(&dest) {
                    // Send sequentially with per-client timeout to avoid
                    // spawning unbounded tasks (one per client per broadcast).
                    let sinks: Vec<(Vmac, Arc<Mutex<WsSink>>)> = {
                        let map = clients.lock().await;
                        map.iter()
                            .filter(|(vmac, _)| **vmac != sender_vmac)
                            .map(|(vmac, sink)| (*vmac, Arc::clone(sink)))
                            .collect()
                    };
                    for (_vmac, sink) in &sinks {
                        let mut w = sink.lock().await;
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            w.send(Message::Binary(relay_bytes.clone().into())),
                        )
                        .await;
                    }
                } else {
                    let target_sink = {
                        let map = clients.lock().await;
                        map.get(&dest).map(Arc::clone)
                    };
                    if let Some(sink) = target_sink {
                        let mut w = sink.lock().await;
                        if let Err(e) = w.send(Message::Binary(relay_bytes.into())).await {
                            warn!("Hub: unicast relay error to {dest:02x?}: {e}");
                        }
                    } else {
                        debug!("Hub: no client with vmac {dest:02x?} for unicast relay");
                    }
                }
            }

            other => {
                debug!("Hub: ignoring function {other:?} from {peer_addr}");
            }
        }
    }

    if let Some(vmac) = client_vmac {
        let mut map = clients.lock().await;
        map.remove(&vmac);
        debug!("Hub: client {peer_addr} (vmac={vmac:02x?}) disconnected");
    }
}
