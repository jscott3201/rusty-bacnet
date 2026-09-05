//! BACnet/SC Hub — a minimal hub that accepts TLS WebSocket connections
//! from BACnet/SC nodes and relays messages between them.
//!
//! The hub performs three duties:
//! 1. **Connection handshake** — responds to `ConnectRequest` with `ConnectAccept`.
//! 2. **Message relay** — forwards `EncapsulatedNpdu` and routed `Result`
//!    messages to the destination VMAC.
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
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, warn};

mod client;
mod connection;
mod deadlines;
mod handler;
mod heartbeat;
mod helpers;
mod relay;
mod timeouts;

pub use timeouts::ScHubHandshakeTimeouts;

use client::HubClient;
use connection::accept_loop;
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
}

/// Shared state for the hub: connected clients keyed by VMAC.
type Clients = Arc<Mutex<HashMap<Vmac, HubClient>>>;

/// A minimal BACnet/SC hub.
///
/// Listens on a TLS WebSocket port, accepts SC node connections, performs the
/// Connect-Request/Connect-Accept handshake, and relays messages between
/// connected nodes.
pub struct ScHub {
    hub_vmac: Vmac,
    /// Device UUID (16 bytes, RFC 4122).
    #[allow(dead_code)]
    hub_uuid: DeviceUuid,
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
        hub_uuid: DeviceUuid,
    ) -> Result<Self, bacnet_types::error::Error> {
        Self::start_with_uuid_and_timeouts(
            bind_addr,
            tls_acceptor,
            hub_vmac,
            hub_uuid,
            ScHubHandshakeTimeouts::default(),
        )
        .await
    }

    /// Start with a Device UUID and validated independent handshake budgets.
    /// Established connections are not governed by these budgets.
    ///
    /// ```no_run
    /// # async fn example(acceptor: tokio_rustls::TlsAcceptor) -> Result<(), bacnet_types::error::Error> {
    /// use bacnet_transport::sc_hub::{ScHub, ScHubHandshakeTimeouts};
    /// use std::time::Duration;
    /// let budgets = ScHubHandshakeTimeouts::new(
    ///     Duration::from_secs(5), Duration::from_secs(5), Duration::from_secs(10),
    /// )?;
    /// let mut hub = ScHub::start_with_uuid_and_timeouts(
    ///     "127.0.0.1:0", acceptor, [0x12; 6], [0x34; 16], budgets,
    /// ).await?;
    /// hub.stop().await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start_with_uuid_and_timeouts(
        bind_addr: &str,
        tls_acceptor: TlsAcceptor,
        hub_vmac: Vmac,
        hub_uuid: DeviceUuid,
        timeouts: ScHubHandshakeTimeouts,
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
            timeouts,
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
mod ws_limits_tests;

#[cfg(test)]
mod ws_capacity_tests;
#[cfg(test)]
mod ws_limits_test_support;
