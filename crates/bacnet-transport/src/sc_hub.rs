//! BACnet/SC Hub — a minimal hub that accepts TLS WebSocket connections
//! from BACnet/SC nodes and relays messages between them.
//!
//! The hub performs three duties:
//! 1. **Connection handshake** — responds to `ConnectRequest` with `ConnectAccept`.
//! 2. **Message relay** — forwards `EncapsulatedNpdu`, addressed Unknown functions,
//!    unicast Address-Resolution/ACK, unicast Advertisement/Solicitation,
//!    unicast/broadcast Proprietary-Message, and permitted routed `Result` messages
//!    (for 0x01/0x02/0x03/0x04/0x05/0x0C/Unknown; Results for 0x00/0x06–0x0B stay
//!    dropped). Relayed Results stay opaque with destination-match, no-echo, and
//!    recipient-cap guards.
//!    No node URI parsing, discovery, or direct-connection support is implied.
//! 3. **Heartbeat** — responds to `HeartbeatRequest` with `HeartbeatAck`.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::{Bytes, BytesMut};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
#[cfg(test)]
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, warn};

pub(super) mod admission;
mod advertisement_transit;
mod broadcast_rate;
mod client;
mod connection;
mod deadlines;
mod graceful;
mod handler;
mod heartbeat;
mod helpers;
mod malformed_diag;
mod opaque_relay;
mod outcomes;
mod proprietary_transit;
mod relay;
mod relay_send;
mod resolution_transit;
mod retirement;
mod tasks;
mod timeouts;
mod timing;
mod tls_config;
mod unknown_transit;

pub use admission::{
    ScHubAdmissionDecision, ScHubAdmissionInput, ScHubAdmissionLimits, ScHubAdmissionPolicy,
    ScHubRegistrationKind, ScHubStatus, DEFAULT_MAX_CLIENTS, DEFAULT_MAX_HANDSHAKES,
};
pub use broadcast_rate::{ScHubBroadcastDropCounts, ScHubBroadcastRatePolicy};
pub use graceful::{ScHubGracefulTimeouts, ScHubShutdownOutcome};
pub use outcomes::ScHubOutcomeCounts;
pub use timeouts::ScHubHandshakeTimeouts;
pub use timing::ScHubProbePolicy;
pub use tls_config::ScHubTlsConfig;

use client::HubClient;
use helpers::*;

#[cfg(test)]
use relay::build_hub_relay_message;
use relay::{
    encode_hub_relay_frame, hub_relay_recipient_vmacs, hub_relay_target, relay_result,
    HubRelayReject, HubRelayTarget, ResultRelayDisposition,
};

use crate::sc_frame::{decode_sc_message, encode_sc_message, ScFunction, ScMessage, Vmac};

type TlsStream = tokio_rustls::server::TlsStream<tokio::net::TcpStream>;
type WsSink = SplitSink<WebSocketStream<TlsStream>, Message>;
type DeviceUuid = [u8; 16];

const HUB_MAX_BVLC_LENGTH: u16 = crate::sc_limits::DEFAULT_MAX_BVLC_LENGTH;
const HUB_MAX_NPDU_LENGTH: u16 = 1497;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectRequestVmacDisposition {
    Accept,
    CloseReserved,
    Nak(ErrorClass, ErrorCode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelayLimitDecision {
    Send,
    DropMaxNpdu,
    DropMaxBvlc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HubClientRegistrationDecision {
    Accept,
    Replace { old_vmac: Vmac },
    NakDuplicateVmac,
    NakMaxClients,
}

struct HubRelaySink {
    vmac: Vmac,
    sink: Arc<Mutex<WsSink>>,
    closed: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

/// Shared state for the hub: connected clients keyed by VMAC.
type Clients = Arc<client::ClientRegistry>;

/// A minimal BACnet/SC hub.
///
/// Listens on a TLS WebSocket port, accepts SC node connections, performs the
/// Connect-Request/Connect-Accept handshake, and relays messages between
/// connected nodes.
///
/// Every startup requires the hosting port's VMAC (neither UNKNOWN nor BROADCAST)
/// and the hosting device's nonzero 16-byte UUID. The caller must provision the
/// UUID before deployment and durably reuse it for the device's entire lifetime
/// (AB.1.5.3). The hub neither generates nor persists identity, checks UUID
/// version/variant bits, nor binds it to a certificate. Connect-Accept advertises
/// these exact configured bytes (AB.2.11 and AB.6).
///
/// Dropping the hub requests eventual worker cleanup on a running Tokio runtime.
/// Use [`Self::stop`] to await completion before reusing its resources.
pub struct ScHub {
    hub_vmac: Vmac,
    /// Device UUID (16 bytes, RFC 4122).
    #[allow(dead_code)]
    hub_uuid: DeviceUuid,
    listener_task: Option<JoinHandle<()>>,
    tasks: tasks::Tasks,
    local_addr: Option<SocketAddr>,
    admission: Arc<admission::AdmissionRuntime>,
    clients: Clients,
    active: Arc<AtomicUsize>,
}

impl ScHub {
    /// Compatible alias for [`Self::start_with_uuid_and_timeouts`], with validated
    /// TLS policy, a caller-specified Device UUID, and independent handshake budgets.
    ///
    /// [`ScHubTlsConfig::from_der`] performs credential/configuration validation
    /// without I/O before this method can bind. All public startup methods require
    /// the same constrained policy and share one hub lifecycle. Use [`Self::stop`]
    /// to await worker cleanup. See [`ScHubTlsConfig`] for an executable example.
    pub async fn start_with_tls_config(
        bind_addr: &str,
        tls_config: ScHubTlsConfig,
        hub_vmac: Vmac,
        hub_uuid: DeviceUuid,
        timeouts: ScHubHandshakeTimeouts,
    ) -> Result<Self, bacnet_types::error::Error> {
        Self::start_with_uuid_and_timeouts(bind_addr, tls_config, hub_vmac, hub_uuid, timeouts)
            .await
    }

    /// Start the hub, binding to `bind_addr` (e.g. `"127.0.0.1:0"` for a
    /// random port).
    ///
    /// The hub begins accepting TLS WebSocket connections immediately on a
    /// background task.
    ///
    /// Requires [`ScHubTlsConfig`]: explicit CA trust, mandatory client certificate
    /// verification, and TLS 1.3-only local policy. Uses default handshake budgets.
    /// A zero UUID or reserved local VMAC returns a configuration error before
    /// binding, as on every public startup route. Raw TLS acceptors are not accepted:
    ///
    /// ```compile_fail,E0308
    /// use bacnet_transport::sc_hub::ScHub;
    /// async fn raw(acceptor: tokio_rustls::TlsAcceptor) {
    ///     let _ = ScHub::start("127.0.0.1:0", acceptor, [0x12; 6], [0x34; 16]).await;
    /// }
    /// ```
    pub async fn start(
        bind_addr: &str,
        tls_config: ScHubTlsConfig,
        hub_vmac: Vmac,
        hub_uuid: DeviceUuid,
    ) -> Result<Self, bacnet_types::error::Error> {
        Self::start_with_uuid(bind_addr, tls_config, hub_vmac, hub_uuid).await
    }

    /// Start the hub with a specific Device UUID.
    /// Requires the same constrained TLS policy as [`Self::start`].
    ///
    /// ```compile_fail,E0308
    /// use bacnet_transport::sc_hub::ScHub;
    /// async fn raw(acceptor: tokio_rustls::TlsAcceptor) {
    ///     let _ = ScHub::start_with_uuid("127.0.0.1:0", acceptor, [0x12; 6], [0x34; 16]).await;
    /// }
    /// ```
    pub async fn start_with_uuid(
        bind_addr: &str,
        tls_config: ScHubTlsConfig,
        hub_vmac: Vmac,
        hub_uuid: DeviceUuid,
    ) -> Result<Self, bacnet_types::error::Error> {
        Self::start_with_uuid_and_timeouts(
            bind_addr,
            tls_config,
            hub_vmac,
            hub_uuid,
            ScHubHandshakeTimeouts::default(),
        )
        .await
    }

    /// Start with a Device UUID and validated independent handshake budgets.
    /// Established connections are not governed by these budgets.
    /// Requires the same constrained TLS policy as [`Self::start`].
    ///
    /// ```no_run
    /// # async fn example(tls: bacnet_transport::sc_hub::ScHubTlsConfig) -> Result<(), bacnet_types::error::Error> {
    /// use bacnet_transport::sc_hub::{ScHub, ScHubHandshakeTimeouts};
    /// use std::time::Duration;
    /// let budgets = ScHubHandshakeTimeouts::new(
    ///     Duration::from_secs(5), Duration::from_secs(5), Duration::from_secs(10),
    /// )?;
    /// let mut hub = ScHub::start_with_uuid_and_timeouts(
    ///     "127.0.0.1:0", tls, [0x12; 6], [0x34; 16], budgets,
    /// ).await?;
    /// hub.stop().await;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ```compile_fail,E0308
    /// use bacnet_transport::sc_hub::{ScHub, ScHubHandshakeTimeouts};
    /// async fn raw(acceptor: tokio_rustls::TlsAcceptor) {
    ///     let _ = ScHub::start_with_uuid_and_timeouts(
    ///         "127.0.0.1:0", acceptor, [0x12; 6], [0x34; 16], ScHubHandshakeTimeouts::default(),
    ///     ).await;
    /// }
    /// ```
    pub async fn start_with_uuid_and_timeouts(
        bind_addr: &str,
        tls_config: ScHubTlsConfig,
        hub_vmac: Vmac,
        hub_uuid: DeviceUuid,
        timeouts: ScHubHandshakeTimeouts,
    ) -> Result<Self, bacnet_types::error::Error> {
        // One local-identity enforcement point for all four public startup APIs.
        // This deliberately does not change remote peer admission or UUID shape.
        if hub_uuid == [0; 16] {
            return Err(bacnet_types::error::Error::Encoding(
                "hub device UUID must not be all zero".into(),
            ));
        }
        if hub_vmac == crate::sc_frame::UNKNOWN_VMAC || hub_vmac == crate::sc_frame::BROADCAST_VMAC
        {
            return Err(bacnet_types::error::Error::Encoding(
                "hub VMAC must not be UNKNOWN or BROADCAST".into(),
            ));
        }
        // Admission bounds are validated before binding, like the broadcast
        // policy: every public startup API funnels through here.
        let admission_limits = tls_config.admission_limits();
        admission_limits.validate()?;
        let relay_send_budget = tls_config.relay_send_budget();
        ScHubTlsConfig::validate_relay_send_budget(relay_send_budget)?;
        let probe_policy = tls_config.probe_policy();
        probe_policy.validate()?;
        let graceful_timeouts = tls_config.graceful_timeouts();
        graceful_timeouts.validate()?;
        let admission = Arc::new(admission::AdmissionRuntime::new(
            admission_limits,
            tls_config.admission_policy(),
        ));
        let broadcast = Arc::new(broadcast_rate::HubBudget::new(
            tls_config.broadcast_rate_policy(),
        )?);
        let tls_acceptor = tls_config.into_acceptor();
        let listener = TcpListener::bind(bind_addr)
            .await
            .map_err(|e| bacnet_types::error::Error::Encoding(format!("Hub bind failed: {e}")))?;

        let local_addr = listener.local_addr().map_err(|e| {
            bacnet_types::error::Error::Encoding(format!("Hub could not read local address: {e}"))
        })?;

        debug!("BACnet/SC hub listening on {local_addr}");

        let clients: Clients = Arc::new(client::ClientRegistry::default());
        let active = Arc::new(AtomicUsize::new(0));

        let tasks = tasks::Tasks::new();
        let tasks = tasks.with_broadcast_budget(broadcast);
        let tasks = tasks.with_graceful_timeouts(graceful_timeouts);
        let mut tasks = tasks.with_probe_policy(probe_policy);
        tasks.timing.relay_send_budget = relay_send_budget;
        let task = tokio::spawn(connection::accept_loop_with_counter(
            listener,
            tls_acceptor,
            (hub_vmac, hub_uuid),
            clients.clone(),
            timeouts,
            active.clone(),
            tasks.clone(),
            admission.clone(),
        ));

        Ok(Self {
            hub_vmac,
            hub_uuid,
            listener_task: Some(task),
            tasks,
            local_addr: Some(local_addr),
            admission,
            clients,
            active,
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

    /// Bounded hub snapshot: listener state, handshake/client counts, and
    /// deny/drop counters (see [`ScHubStatus`]).
    ///
    /// Counts and kind labels only — no certificates, keys, VMAC maps, or
    /// payloads. Each counter is atomic; the snapshot is not transactional
    /// while workers are active, and the handshake count is approximate
    /// under replacement churn. Usable after [`Self::stop`].
    ///
    /// ```
    /// use bacnet_transport::sc_hub::{ScHub, ScHubHandshakeTimeouts, ScHubTlsConfig};
    /// use rcgen::{CertificateParams, Issuer, KeyPair};
    /// use rustls::pki_types::PrivatePkcs8KeyDer;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut ca_params = CertificateParams::new(Vec::<String>::new())?;
    /// ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    /// let ca_key = KeyPair::generate()?;
    /// let ca = ca_params.self_signed(&ca_key)?;
    /// let issuer = Issuer::from_params(&ca_params, &ca_key);
    /// let key = KeyPair::generate()?;
    /// let cert = CertificateParams::new(vec!["localhost".into()])?.signed_by(&key, &issuer)?;
    /// let tls = ScHubTlsConfig::from_der(
    ///     vec![ca.der().clone()], vec![cert.der().clone()],
    ///     PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    /// )?;
    /// tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
    ///     let mut hub = ScHub::start(
    ///         "127.0.0.1:0", tls, [0x12; 6], [0x34; 16],
    ///     ).await?;
    ///     let status = hub.status().await;
    ///     assert!(status.listening);
    ///     assert_eq!(status.client_count, 0);
    ///     assert_eq!(status.handshake_count, 0);
    ///     assert_eq!(status.admin_denied, 0);
    ///     assert_eq!((status.limits.max_clients, status.limits.max_handshakes), (256, 256));
    ///     hub.stop().await;
    ///     assert!(!hub.status().await.listening);
    ///     Ok::<_, bacnet_types::error::Error>(())
    /// })?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn status(&self) -> ScHubStatus {
        let client_count = self.clients.lock().await.len();
        let handshake_count = self
            .active
            .load(Ordering::Relaxed)
            .saturating_sub(client_count);
        ScHubStatus {
            listening: self
                .listener_task
                .as_ref()
                .is_some_and(|task| !task.is_finished()),
            limits: self.admission.limits,
            client_count,
            handshake_count,
            admin_denied: self.admission.denied(),
            broadcast_drops: self.tasks.broadcast.drop_counts(),
            outcomes: self.clients.outcomes.snapshot(),
        }
    }

    /// Stop admission, cancel all hub workers, and await their resource cleanup.
    ///
    /// This is forceful local shutdown, not the BACnet Disconnect/reciprocal
    /// WebSocket Close sequence. Cancelling this future leaves shutdown running;
    /// a later call can still await completion. Runtime scheduling is cooperative.
    pub async fn stop(&mut self) {
        self.tasks.request_shutdown();
        if let Some(task) = self.listener_task.as_mut() {
            let _ = task.await;
            self.listener_task = None;
        }
    }

    /// Bounded graceful shutdown alongside the preserved forceful path.
    ///
    /// Seals admission first (same seal as [`Self::stop`], first seal wins
    /// the drain kind), then drives the accepting-peer disconnect exchange
    /// per established peer: hub-initiated Disconnect-Request, awaited
    /// Disconnect-Ack within the configured per-peer ack bound, then the
    /// AB.7.5.5 WebSocket close handshake within the per-close bound.
    /// Half-handshakes get a silent Close, never a Disconnect-Request. The
    /// configured overall bound caps the whole drain with a forceful
    /// fallback; uncooperative peers cannot hang shutdown.
    ///
    /// Corrected role wording follows the official 2024-04-29 errata summary
    /// item 13 (Annex AB.6.2.3 p. 1405): in DISCONNECTING the accepting peer
    /// (this hub) awaits the Disconnect-Ack from the initiating peer (the
    /// node); the redline inserts "initiating" and strikes "accepting".
    /// Adopted close order per peer: Disconnect-Request, awaited
    /// Disconnect-Ack, then the AB.7.5.5 WebSocket close handshake.
    ///
    /// Shutdown ownership stays with the hub: cancelling this future leaves
    /// shutdown running and a later call observes completion (same
    /// cancel-safety pattern as [`Self::stop`]). Closing-but-registered peers
    /// still read as `client_count` in [`Self::status`] until lease cleanup
    /// removes them. Returns [`ScHubShutdownOutcome::Graceful`] only when
    /// every peer Ack was observed and the close handshake completed without
    /// timeout or abort; otherwise [`ScHubShutdownOutcome::Forced`]. A prior
    /// forceful [`Self::stop`] keeps the run forced.
    ///
    /// Bounds come from [`ScHubTlsConfig::with_graceful_timeouts`] at
    /// startup (defaults 5s ack + 5s close within 15s overall).
    pub async fn shutdown_gracefully(&mut self) -> ScHubShutdownOutcome {
        self.tasks.request_graceful();
        if let Some(task) = self.listener_task.as_mut() {
            let _ = task.await;
            self.listener_task = None;
        }
        self.tasks
            .get_outcome()
            .unwrap_or(ScHubShutdownOutcome::Forced)
    }
}

impl Drop for ScHub {
    fn drop(&mut self) {
        // The detached supervisor retains cleanup ownership on a live runtime.
        // Drop requests forceful-seal cleanup only; it never awaits the
        // graceful Disconnect/Ack/Close exchange. Use shutdown_gracefully()
        // (or stop() for forceful await) before drop when the exchange matters.
        self.tasks.request_shutdown();
    }
}

async fn handle_client(
    peer_addr: SocketAddr,
    hub_vmac: Vmac,
    hub_uuid: DeviceUuid,
    read: futures_util::stream::SplitStream<WebSocketStream<TlsStream>>,
    write: Arc<Mutex<WsSink>>,
    clients: Clients,
    expires: tokio::time::Instant,
    admission: Arc<admission::AdmissionRuntime>,
    tls_client_verified: bool,
    graceful: graceful::GracefulCtx,
    timing: timing::HubTiming,
) {
    let deadline = Arc::new(deadlines::ConnectDeadline::new(expires));
    deadlines::serve(
        peer_addr,
        (hub_vmac, hub_uuid),
        read,
        write,
        clients,
        deadline,
        || {},
        admission,
        tls_client_verified,
        graceful,
        timing,
    )
    .await;
}

// Private ACK observer preserves the existing heartbeat test seam.
#[cfg(test)]
async fn handle_client_observed(
    peer_addr: SocketAddr,
    hub_vmac: Vmac,
    hub_uuid: DeviceUuid,
    read: futures_util::stream::SplitStream<WebSocketStream<TlsStream>>,
    write: Arc<Mutex<WsSink>>,
    clients: Clients,
    on_heartbeat_ack: impl Fn() + Send,
) {
    let deadline = Arc::new(deadlines::ConnectDeadline::new(
        tokio::time::Instant::now() + ScHubHandshakeTimeouts::default().connect_request(),
    ));
    // The observed test seam bypasses TLS with no client authentication.
    // Never graceful: a fresh supervisor never fires, preserving the
    // forceful/discard-Ack behavior for heartbeat tests.
    let admission = Arc::new(admission::AdmissionRuntime::default());
    let tasks = tasks::Tasks::new();
    let graceful = tasks.graceful_ctx();
    let timing = heartbeat_test_support::probe_runtime();
    deadlines::serve(
        peer_addr,
        (hub_vmac, hub_uuid),
        read,
        write,
        clients,
        deadline,
        on_heartbeat_ack,
        admission,
        false,
        graceful,
        timing,
    )
    .await;
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod admission_tests;

#[cfg(test)]
mod connect_validation_tests;
#[cfg(test)]
mod deadline_capacity_tests;
#[cfg(test)]
mod deadline_commit_tests;
#[cfg(test)]
mod deadline_test_support;
#[cfg(test)]
mod deadline_tests;
#[cfg(test)]
mod disconnect_validation_tests;
#[cfg(test)]
mod heartbeat_generation_tests;
#[cfg(test)]
mod heartbeat_test_support;
#[cfg(test)]
mod heartbeat_tests;
#[cfg(test)]
mod heartbeat_validation_tests;
#[cfg(test)]
mod peer_uuid_tests;
#[cfg(test)]
mod response_silence_lifecycle_tests;
#[cfg(test)]
mod response_silence_tests;

#[cfg(test)]
mod empty_npdu_tests;

#[cfg(test)]
mod empty_npdu_retirement_tests;

#[cfg(test)]
mod ws_limits_tests;

#[cfg(test)]
mod ws_capacity_tests;
#[cfg(test)]
mod ws_limits_test_support;

#[cfg(test)]
mod shutdown_tests;

#[cfg(test)]
mod graceful_tests;

#[cfg(test)]
mod task_tests;

#[cfg(test)]
mod shutdown_blocked_tests;

#[cfg(test)]
mod unicast_deadline_tests;

#[cfg(test)]
mod retirement_tests;

#[cfg(test)]
mod retirement_lifecycle_tests;

#[cfg(test)]
mod retirement_io_tests;

#[cfg(test)]
mod retirement_capacity_tests;

#[cfg(test)]
mod unknown_transit_lifecycle_tests;
#[cfg(test)]
mod unknown_transit_tests;

#[cfg(test)]
mod advertisement_transit_tests;
#[cfg(test)]
mod proprietary_transit_tests;
#[cfg(test)]
mod resolution_transit_lifecycle_tests;
#[cfg(test)]
mod resolution_transit_tests;

#[cfg(test)]
mod probe_tests;

#[cfg(test)]
mod outcome_tests;

#[cfg(test)]
mod relay_budget_tests;

#[cfg(test)]
mod peer_close_tests;
