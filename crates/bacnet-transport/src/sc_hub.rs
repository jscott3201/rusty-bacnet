//! BACnet/SC Hub — a minimal hub that accepts TLS WebSocket connections
//! from BACnet/SC nodes and relays messages between them.
//!
//! The hub performs three duties:
//! 1. **Connection handshake** — responds to `ConnectRequest` with `ConnectAccept`.
//! 2. **Message relay** — forwards `EncapsulatedNpdu`, addressed Unknown functions,
//!    unicast Address-Resolution/ACK, unicast Advertisement/Solicitation, and permitted routed `Result` messages.
//!    No node URI parsing, discovery, or direct-connection support is implied.
//! 3. **Heartbeat** — responds to `HeartbeatRequest` with `HeartbeatAck`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

mod advertisement_transit;
mod client;
mod connection;
mod deadlines;
mod handler;
mod heartbeat;
mod helpers;
mod opaque_relay;
mod relay;
mod relay_send;
mod resolution_transit;
mod retirement;
mod tasks;
mod timeouts;
mod tls_config;
mod unknown_transit;

pub use timeouts::ScHubHandshakeTimeouts;
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
type Clients = Arc<Mutex<HashMap<Vmac, HubClient>>>;

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
        let tls_acceptor = tls_config.into_acceptor();
        let listener = TcpListener::bind(bind_addr)
            .await
            .map_err(|e| bacnet_types::error::Error::Encoding(format!("Hub bind failed: {e}")))?;

        let local_addr = listener.local_addr().map_err(|e| {
            bacnet_types::error::Error::Encoding(format!("Hub could not read local address: {e}"))
        })?;

        debug!("BACnet/SC hub listening on {local_addr}");

        let clients: Clients = Arc::new(Mutex::new(HashMap::new()));

        let tasks = tasks::Tasks::new();
        let task = tokio::spawn(connection::accept_loop_with_counter(
            listener,
            tls_acceptor,
            (hub_vmac, hub_uuid),
            clients,
            timeouts,
            Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            tasks.clone(),
        ));

        Ok(Self {
            hub_vmac,
            hub_uuid,
            listener_task: Some(task),
            tasks,
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
}

impl Drop for ScHub {
    fn drop(&mut self) {
        // The detached supervisor retains cleanup ownership on a live runtime.
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
    deadlines::serve(
        peer_addr,
        (hub_vmac, hub_uuid),
        read,
        write,
        clients,
        deadline,
        on_heartbeat_ack,
    )
    .await;
}

#[cfg(test)]
mod tests;

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
mod task_tests;

#[cfg(test)]
mod shutdown_blocked_tests;

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
mod resolution_transit_lifecycle_tests;
#[cfg(test)]
mod resolution_transit_tests;
