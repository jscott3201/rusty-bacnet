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

pub(super) async fn accept_loop_with_counter(
    listener: TcpListener,
    tls_acceptor: TlsAcceptor,
    hub: (Vmac, DeviceUuid),
    clients: Clients,
    timeouts: super::ScHubHandshakeTimeouts,
    active_connections: Arc<AtomicUsize>,
    tasks: super::tasks::Tasks,
    admission: Arc<super::admission::AdmissionRuntime>,
) {
    let _abort_on_exit = tasks.abort_on_exit();
    let (hub_vmac, hub_uuid) = hub;
    let mut shutdown = tasks.subscribe();
    // All active accepted connections count, including established clients.
    // The total is the configured sum (defaults 256 + 256 = 512); the
    // handshake bound below counts unregistered connections only.
    let limits = admission.limits;
    let total_active = limits.total_active();

    // Heartbeat sweep: periodically check for idle clients and send HeartbeatRequest.
    // Existing hub-originated liveness probe is a local extension. It does not
    // implement or replace the initiating node's Annex AB.6.3 keepalive duty.
    // Exits cooperatively on shutdown so a graceful drain can complete without
    // aborting this sleep; forceful drain still aborts if needed.
    let timing = tasks.timing;
    {
        let clients_for_hb = clients.clone();
        #[cfg(test)]
        let scans = tasks.probe_scans.clone();
        let mut hb_shutdown = tasks.subscribe();
        let next_msg_id = std::sync::atomic::AtomicU16::new(0x8000); // hub message IDs start high
        tasks.spawner().spawn(async move {
            let mut interval = tokio::time::interval(timing.policy.scan_interval());
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                if *hb_shutdown.borrow() {
                    break;
                }
                tokio::select! {
                    _ = hb_shutdown.changed() => {}
                    _ = interval.tick() => {
                        #[cfg(test)]
                        scans.fetch_add(1, Ordering::Release);
                        heartbeat::sweep(&clients_for_hb, &next_msg_id, &heartbeat::SocketIo(timing)).await;
                    }
                }
            }
        });
    }

    loop {
        let accepted = tokio::select! {
            biased;
            _ = shutdown.wait_for(|requested| *requested) => break,
            _ = tasks.reap() => continue,
            accepted = listener.accept() => accepted,
        };
        let (tcp_stream, peer_addr) = match accepted {
            Ok(v) => v,
            Err(e) => {
                warn!("Hub accept error: {e}");
                continue;
            }
        };

        // Reject when the total active accepted-connection cap is reached
        let current = active_connections.load(std::sync::atomic::Ordering::Relaxed);
        if current >= total_active {
            warn!("Hub: rejecting connection from {peer_addr} — max active connections ({total_active}) reached");
            drop(tcp_stream);
            continue;
        }
        // Reject when the unregistered-handshake cap is reached. Established
        // clients keep their slots, so only handshake pressure is counted
        // here. Both caps are approximate local resource policy under
        // concurrent accepts, not transactional guarantees.
        let registered = clients.lock().await.len();
        if current.saturating_sub(registered) >= limits.max_handshakes {
            warn!(
                "Hub: rejecting connection from {peer_addr} — max handshakes ({}) reached",
                limits.max_handshakes
            );
            drop(tcp_stream);
            continue;
        }
        let admission_permit = Admission::new(active_connections.clone(), timeouts.tls());

        debug!("Hub: new TCP connection from {peer_addr}");

        let acceptor = tls_acceptor.clone();
        let clients = clients.clone();
        let admission_runtime = admission.clone();
        let graceful = tasks.graceful_ctx();

        // Task locals are not inherited by spawn. Explicitly scope every
        // connection to this hub's one aggregate budget; no new worker/lifetime.
        tasks
            .spawner()
            .spawn(super::broadcast_rate::HUB_BUDGET.scope(
                tasks.broadcast.clone(),
                serve_connection(
                    tcp_stream,
                    peer_addr,
                    acceptor,
                    (hub_vmac, hub_uuid),
                    clients,
                    timeouts,
                    admission_permit,
                    admission_runtime,
                    graceful,
                    timing,
                ),
            ));
    }
    drop(listener);
    if tasks.graceful_kind() {
        let overall = tasks.graceful_ctx().timeouts().overall();
        let completed = tasks.drain_gracefully(overall).await;
        let outcome = if completed && !tasks.graceful_failed() {
            super::graceful::ScHubShutdownOutcome::Graceful
        } else {
            super::graceful::ScHubShutdownOutcome::Forced
        };
        tasks.set_outcome(outcome);
    } else {
        tasks.drain().await;
        tasks.set_outcome(super::graceful::ScHubShutdownOutcome::Forced);
    }
    // Cancellation skips per-client tail cleanup. No owned mutator remains.
    clients.lock().await.clear();
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
    runtime: Arc<super::admission::AdmissionRuntime>,
    graceful: super::graceful::GracefulCtx,
    timing: super::timing::HubTiming,
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
    // Boolean channel only: the acceptor already required a CA-verified
    // client certificate for this handshake to succeed. Presence is
    // rechecked here (no subject/fingerprint extraction) so the admission
    // input stays honest if verifier policy ever changes.
    let tls_client_verified = tls_stream
        .get_ref()
        .1
        .peer_certificates()
        .is_some_and(|certs| !certs.is_empty());

    // WebSocket upgrade — require and echo the BACnet/SC hub subprotocol.
    let upgrade_deadline = tokio::time::Instant::now() + timeouts.websocket_upgrade();
    let ws_stream = match super::deadlines::before(
        upgrade_deadline,
        tokio_tungstenite::accept_hdr_async_with_config(
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
            Some(crate::sc_limits::websocket(
                crate::sc_limits::DEFAULT_MAX_BVLC_LENGTH as usize,
            )),
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
        runtime,
        tls_client_verified,
        graceful,
        timing,
    )
    .await;
}
