//! Connection acceptance and WebSocket upgrade loop for the BACnet/SC hub.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::time::Instant;

use futures_util::StreamExt;
use tokio::sync::Mutex;
use tokio_rustls::TlsAcceptor;
use tracing::{debug, warn};

use crate::sc_frame::{Vmac, BACNET_SC_HUB_SUBPROTOCOL};
use crate::sc_tls::boxed;

use super::context::{HubConnectionContext, HubListener, PeerConnection};
use super::heartbeat;
use super::helpers::{offers_websocket_subprotocol, websocket_subprotocol_error_response};
use super::{handle_client, Clients, DeviceUuid};

// ---------------------------------------------------------------------------
// Accept loop
// ---------------------------------------------------------------------------

pub(super) async fn accept_loop_with_counter(
    bound: HubListener,
    hub: (Vmac, DeviceUuid),
    clients: Clients,
    active_connections: Arc<AtomicUsize>,
    tasks: super::tasks::Tasks,
    admission: Arc<super::admission::AdmissionRuntime>,
) {
    let _abort_on_exit = tasks.abort_on_exit();
    let HubListener {
        listener,
        tls_acceptor,
        timeouts,
    } = bound;
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
            super::outcomes::increment(&clients.outcomes.total_active_accept_drops);
            crate::warn_throttled!(
                admission.connection_warning,
                "Hub: rejecting connection from {peer_addr} — max active connections ({total_active}) reached"
            );
            drop(tcp_stream);
            continue;
        }
        // Reject when the unregistered-handshake cap is reached. Established
        // clients keep their slots, so only handshake pressure is counted
        // here. Both caps are approximate local resource policy under
        // concurrent accepts, not transactional guarantees.
        let registered = clients.lock().await.len();
        if current.saturating_sub(registered) >= limits.max_handshakes {
            super::outcomes::increment(&clients.outcomes.handshake_accept_drops);
            crate::warn_throttled!(
                admission.connection_warning,
                "Hub: rejecting connection from {peer_addr} — max handshakes ({}) reached",
                limits.max_handshakes
            );
            drop(tcp_stream);
            continue;
        }
        let admission_permit = Admission::new(active_connections.clone(), timeouts.tls());
        crate::sc_tls::disable_nagle(&tcp_stream);

        debug!("Hub: new TCP connection from {peer_addr}");

        let acceptor = tls_acceptor.clone();
        let clients = clients.clone();
        let context = HubConnectionContext {
            hub,
            clients,
            admission: admission.clone(),
            graceful: tasks.graceful_ctx(),
            timing,
        };

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
                    context,
                    timeouts,
                    admission_permit,
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
//
// Each stage's future is boxed, once per connection: tokio-tungstenite's
// accept handshake takes about 100 KB of stack in a debug build, and unboxed,
// these futures added about 65 KB more to a connection task's stack (#953).
#[allow(clippy::result_large_err)]
pub(super) async fn serve_connection(
    tcp_stream: tokio::net::TcpStream,
    peer_addr: std::net::SocketAddr,
    acceptor: TlsAcceptor,
    ctx: HubConnectionContext,
    timeouts: super::ScHubHandshakeTimeouts,
    admission: Admission,
) {
    let clients = &ctx.clients;
    let tls_deadline = admission.tls_deadline;
    // TLS handshake
    let tls_stream = match boxed(|| {
        super::deadlines::before(tls_deadline, acceptor.accept(tcp_stream).into_fallible())
    })
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err((e, tcp_stream))) => {
            crate::warn_throttled!(
                ctx.admission.connection_warning,
                "Hub TLS handshake failed for {peer_addr}: {e}"
            );
            crate::tls_reject::close_after_alert(tcp_stream, tls_deadline).await;
            return;
        }
        Err(()) => {
            super::outcomes::increment(&clients.outcomes.tls_timeouts);
            debug!("Hub TLS handshake deadline expired for {peer_addr}");
            return;
        }
    };
    let verified_leaf = super::certificate_bindings::VerifiedLeaf::from_verified_chain(
        tls_stream.get_ref().1.peer_certificates(),
    );

    // WebSocket upgrade — require and echo the BACnet/SC hub subprotocol.
    let upgrade_deadline = tokio::time::Instant::now() + timeouts.websocket_upgrade();
    #[cfg(test)]
    ctx.admission
        .upgrade_deadlines_armed
        .fetch_add(1, Ordering::Release);
    let ws_stream = match boxed(|| {
        super::deadlines::before(
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
    })
    .await
    {
        Ok(Ok(ws)) => ws,
        Ok(Err(e)) => {
            crate::warn_throttled!(
                ctx.admission.connection_warning,
                "Hub WebSocket upgrade failed for {peer_addr}: {e}"
            );
            return;
        }
        Err(()) => {
            super::outcomes::increment(&clients.outcomes.websocket_timeouts);
            debug!("Hub WebSocket upgrade deadline expired for {peer_addr}");
            return;
        }
    };

    let connect_deadline = tokio::time::Instant::now() + timeouts.connect_request();
    let (write, read) = ws_stream.split();
    let write = Arc::new(Mutex::new(write));

    boxed(|| {
        handle_client(
            PeerConnection {
                addr: peer_addr,
                read,
                write,
                verified_leaf,
            },
            ctx,
            connect_deadline,
        )
    })
    .await;
}
