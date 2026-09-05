//! Connection acceptance and WebSocket upgrade loop for the BACnet/SC hub.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::time::Instant;

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

pub(super) async fn accept_loop(
    listener: TcpListener,
    tls_acceptor: TlsAcceptor,
    hub_vmac: Vmac,
    hub_uuid: DeviceUuid,
    clients: Clients,
    timeouts: super::ScHubHandshakeTimeouts,
) {
    accept_loop_with_counter(
        listener,
        tls_acceptor,
        hub_vmac,
        hub_uuid,
        clients,
        timeouts,
        Arc::new(AtomicUsize::new(0)),
    )
    .await;
}

pub(super) async fn accept_loop_with_counter(
    listener: TcpListener,
    tls_acceptor: TlsAcceptor,
    hub_vmac: Vmac,
    hub_uuid: DeviceUuid,
    clients: Clients,
    timeouts: super::ScHubHandshakeTimeouts,
    active_connections: Arc<AtomicUsize>,
) {
    // All active accepted connections count, including established clients.
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

        // Reject when the total active accepted-connection cap is reached
        let current = active_connections.load(std::sync::atomic::Ordering::Relaxed);
        if current >= MAX_ACTIVE_CONNECTIONS {
            warn!("Hub: rejecting connection from {peer_addr} — max active connections ({MAX_ACTIVE_CONNECTIONS}) reached");
            drop(tcp_stream);
            continue;
        }
        let admission = Admission::new(active_connections.clone(), timeouts.tls());

        debug!("Hub: new TCP connection from {peer_addr}");

        let acceptor = tls_acceptor.clone();
        let clients = clients.clone();

        tokio::spawn(serve_connection(
            tcp_stream,
            peer_addr,
            acceptor,
            (hub_vmac, hub_uuid),
            clients,
            timeouts,
            admission,
        ));
    }
}

/// Owned from admission through the entire accepted task, even before first poll.
pub(super) struct Admission {
    counter: Arc<AtomicUsize>,
    tls_deadline: Instant,
}

impl Admission {
    pub(super) fn new(counter: Arc<AtomicUsize>, tls: std::time::Duration) -> Self {
        let tls_deadline = Instant::now() + tls;
        counter.fetch_add(1, Ordering::Relaxed);
        Self {
            counter,
            tls_deadline,
        }
    }
}

impl Drop for Admission {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

// Tungstenite fixes the callback ErrorResponse size.
#[allow(clippy::result_large_err)]
pub(super) async fn serve_connection(
    tcp_stream: tokio::net::TcpStream,
    peer_addr: std::net::SocketAddr,
    acceptor: TlsAcceptor,
    hub: (Vmac, DeviceUuid),
    clients: Clients,
    timeouts: super::ScHubHandshakeTimeouts,
    admission: Admission,
) {
    let (hub_vmac, hub_uuid) = hub;
    let tls_deadline = admission.tls_deadline;
    // TLS handshake
    let tls_stream = match super::deadlines::before(tls_deadline, acceptor.accept(tcp_stream)).await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            warn!("Hub TLS handshake failed for {peer_addr}: {e}");
            return;
        }
        Err(()) => {
            debug!("Hub TLS handshake deadline expired for {peer_addr}");
            return;
        }
    };

    // WebSocket upgrade — require and echo the BACnet/SC hub subprotocol.
    let upgrade_deadline = tokio::time::Instant::now() + timeouts.websocket_upgrade();
    let ws_stream = match super::deadlines::before(
        upgrade_deadline,
        tokio_tungstenite::accept_hdr_async(
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
        ),
    )
    .await
    {
        Ok(Ok(ws)) => ws,
        Ok(Err(e)) => {
            warn!("Hub WebSocket upgrade failed for {peer_addr}: {e}");
            return;
        }
        Err(()) => {
            debug!("Hub WebSocket upgrade deadline expired for {peer_addr}");
            return;
        }
    };

    let connect_deadline = tokio::time::Instant::now() + timeouts.connect_request();
    let (write, read) = ws_stream.split();
    let write = Arc::new(Mutex::new(write));

    handle_client(
        peer_addr,
        hub_vmac,
        hub_uuid,
        read,
        write,
        clients,
        connect_deadline,
    )
    .await;
}
