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
        let listener = TcpListener::bind(bind_addr)
            .await
            .map_err(|e| bacnet_types::error::Error::Encoding(format!("Hub bind failed: {e}")))?;

        let local_addr = listener.local_addr().map_err(|e| {
            bacnet_types::error::Error::Encoding(format!("Hub could not read local address: {e}"))
        })?;

        debug!("BACnet/SC hub listening on {local_addr}");

        let clients: Clients = Arc::new(Mutex::new(HashMap::new()));

        let task = tokio::spawn(accept_loop(listener, tls_acceptor, hub_vmac, clients));

        Ok(Self {
            hub_vmac,
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
    clients: Clients,
) {
    loop {
        let (tcp_stream, peer_addr) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                warn!("Hub accept error: {e}");
                continue;
            }
        };

        debug!("Hub: new TCP connection from {peer_addr}");

        let acceptor = tls_acceptor.clone();
        let clients = clients.clone();

        tokio::spawn(async move {
            // TLS handshake
            let tls_stream = match acceptor.accept(tcp_stream).await {
                Ok(s) => s,
                Err(e) => {
                    warn!("Hub TLS handshake failed for {peer_addr}: {e}");
                    return;
                }
            };

            // WebSocket upgrade — negotiate the BACnet/SC subprotocol
            // per ASHRAE 135-2020 Annex AB and RFC 6455: only echo the
            // subprotocol if the client offered it.
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

            handle_client(peer_addr, hub_vmac, read, write, clients).await;
        });
    }
}

// ---------------------------------------------------------------------------
// Per-client handler
// ---------------------------------------------------------------------------

async fn handle_client(
    peer_addr: SocketAddr,
    hub_vmac: Vmac,
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
            Ok(_) => continue, // skip non-binary
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
            // --- Connection handshake ---
            ScFunction::ConnectRequest => {
                // Per Annex AB.7.1 the ConnectRequest payload is:
                // VMAC(6) + Max-BVLC-Length(2,BE) + Max-NPDU-Length(2,BE) = 10 bytes.
                let vmac = if sc_msg.payload.len() >= 6 {
                    let mut v = [0u8; 6];
                    v.copy_from_slice(&sc_msg.payload[0..6]);
                    v
                } else {
                    sc_msg.originating_vmac.unwrap_or([0; 6])
                };
                debug!("Hub: ConnectRequest from {peer_addr} vmac={vmac:02x?}");

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
                            originating_vmac: Some(hub_vmac),
                            destination_vmac: Some(vmac),
                            dest_options: Vec::new(),
                            data_options: Vec::new(),
                            // Error payload per Annex AB: originating_function(1)
                            // + error_class(2,BE) + error_code(2,BE) = 5 bytes.
                            payload: Bytes::from(vec![
                                ScFunction::ConnectRequest.to_raw(), // originating function
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
                            originating_vmac: Some(hub_vmac),
                            destination_vmac: Some(vmac),
                            dest_options: Vec::new(),
                            data_options: Vec::new(),
                            payload: Bytes::from(vec![
                                ScFunction::ConnectRequest.to_raw(),
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

                // Send ConnectAccept with 10-byte payload per Annex AB.7.2:
                // VMAC(6) + Max-BVLC-Length(2,BE) + Max-NPDU-Length(2,BE).
                let mut accept_payload = Vec::with_capacity(10);
                accept_payload.extend_from_slice(&hub_vmac);
                accept_payload.extend_from_slice(&1476u16.to_be_bytes()); // Max-BVLC-Length
                accept_payload.extend_from_slice(&1476u16.to_be_bytes()); // Max-NPDU-Length
                let accept = ScMessage {
                    function: ScFunction::ConnectAccept,
                    message_id: sc_msg.message_id,
                    originating_vmac: Some(hub_vmac),
                    destination_vmac: Some(vmac),
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

            // --- Heartbeat ---
            ScFunction::HeartbeatRequest => {
                let ack = ScMessage {
                    function: ScFunction::HeartbeatAck,
                    message_id: sc_msg.message_id,
                    originating_vmac: Some(hub_vmac),
                    destination_vmac: sc_msg.originating_vmac,
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

            // --- Disconnect ---
            ScFunction::DisconnectRequest => {
                debug!("Hub: DisconnectRequest from {peer_addr}");
                let ack = ScMessage {
                    function: ScFunction::DisconnectAck,
                    message_id: sc_msg.message_id,
                    originating_vmac: Some(hub_vmac),
                    destination_vmac: sc_msg.originating_vmac,
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

            // --- NPDU relay ---
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

                // Use raw bytes instead of re-encoding to preserve the
                // original wire format (including any header options).
                let relay_bytes = data;

                if is_broadcast_vmac(&dest) {
                    // Snapshot client sinks then release the map lock before
                    // performing async sends to avoid holding the lock across
                    // awaits.
                    let sinks: Vec<(Vmac, Arc<Mutex<WsSink>>)> = {
                        let map = clients.lock().await;
                        map.iter()
                            .filter(|(vmac, _)| **vmac != sender_vmac)
                            .map(|(vmac, sink)| (*vmac, Arc::clone(sink)))
                            .collect()
                    };
                    // Map lock released here.
                    let mut handles = Vec::with_capacity(sinks.len());
                    for (_vmac, sink) in sinks {
                        let data = relay_bytes.clone();
                        handles.push(tokio::spawn(async move {
                            let mut w = sink.lock().await;
                            if let Err(e) = w.send(Message::Binary(data.into())).await {
                                warn!("Hub: broadcast relay error: {e}");
                            }
                        }));
                    }
                    for handle in handles {
                        let _ = handle.await;
                    }
                } else {
                    // Unicast — snapshot the target sink then release the lock.
                    let target_sink = {
                        let map = clients.lock().await;
                        map.get(&dest).map(Arc::clone)
                    };
                    // Map lock released here.
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

    // Cleanup: remove client from map.
    if let Some(vmac) = client_vmac {
        let mut map = clients.lock().await;
        map.remove(&vmac);
        debug!("Hub: client {peer_addr} (vmac={vmac:02x?}) disconnected");
    }
}
