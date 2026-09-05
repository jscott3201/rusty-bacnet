//! Connection acceptance and WebSocket upgrade loop for the BACnet/SC hub.

use std::sync::Arc;

use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_rustls::TlsAcceptor;
use tracing::{debug, warn};

use crate::sc_frame::{Vmac, BACNET_SC_HUB_SUBPROTOCOL};

use super::heartbeat;
use super::helpers::{offers_websocket_subprotocol, websocket_subprotocol_error_response};
use super::{handle_client, Clients, DeviceUuid};

// ---------------------------------------------------------------------------
// Accept loop
// ---------------------------------------------------------------------------

// Closure passed to `accept_hdr_async` returns the upstream tungstenite
// `ErrorResponse`, whose size is fixed by the library. The clippy lint can't
// be addressed without changing the foreign signature.
#[allow(clippy::result_large_err)]
pub(super) async fn accept_loop(
    listener: TcpListener,
    tls_acceptor: TlsAcceptor,
    hub_vmac: Vmac,
    hub_uuid: DeviceUuid,
    clients: Clients,
) {
    // Track active TCP connections (pre-handshake) to limit DoS surface.
    let active_connections = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    const MAX_ACTIVE_CONNECTIONS: usize = 512;

    // Heartbeat sweep: periodically check for idle clients and send HeartbeatRequest.
    // Existing hub-originated liveness probe is a local extension. It does not
    // implement or replace the initiating node's Annex AB.6.3 keepalive duty.
    const HEARTBEAT_CHECK_INTERVAL_SECS: u64 = 30;
    {
        let clients_for_hb = clients.clone();
        let next_msg_id = std::sync::atomic::AtomicU16::new(0x8000); // hub message IDs start high
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                HEARTBEAT_CHECK_INTERVAL_SECS,
            ));
            loop {
                interval.tick().await;
                heartbeat::sweep(&clients_for_hb, &next_msg_id, &heartbeat::SocketIo).await;
            }
        });
    }

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

            // WebSocket upgrade — require and echo the BACnet/SC hub subprotocol.
            let ws_stream = match tokio_tungstenite::accept_hdr_async(
                tls_stream,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                 mut response: tokio_tungstenite::tungstenite::handshake::server::Response|
                 -> Result<
                    tokio_tungstenite::tungstenite::handshake::server::Response,
                    tokio_tungstenite::tungstenite::handshake::server::ErrorResponse,
                > {
                    if !offers_websocket_subprotocol(request, BACNET_SC_HUB_SUBPROTOCOL) {
                        return Err(websocket_subprotocol_error_response());
                    }
                    response.headers_mut().insert(
                        "Sec-WebSocket-Protocol",
                        BACNET_SC_HUB_SUBPROTOCOL.parse().unwrap(),
                    );
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
