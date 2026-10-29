//! Bundles of hub state threaded through the accept and connection tasks.

use std::sync::Arc;

use futures_util::stream::SplitStream;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::WebSocketStream;

use crate::sc_frame::Vmac;

use super::{
    Clients, DeviceUuid, ScHubHandshakeTimeouts, TlsStream, WsSink, admission::AdmissionRuntime,
    certificate_bindings::VerifiedLeaf, graceful::GracefulCtx, timing::HubTiming,
};

/// Hub-wide state every connection task needs.
pub(super) struct HubConnectionContext {
    /// The hub's own VMAC and device UUID.
    pub(super) hub: (Vmac, DeviceUuid),
    /// Registry of connected clients.
    pub(super) clients: Clients,
    /// Admission limits, policy and counters.
    pub(super) admission: Arc<AdmissionRuntime>,
    /// Graceful-shutdown signal and timeouts.
    pub(super) graceful: GracefulCtx,
    /// Heartbeat and relay timing.
    pub(super) timing: HubTiming,
}

/// One accepted client socket after the WebSocket upgrade.
pub(super) struct PeerConnection {
    /// Remote socket address.
    pub(super) addr: std::net::SocketAddr,
    /// Read half of the upgraded socket.
    pub(super) read: SplitStream<WebSocketStream<TlsStream>>,
    /// Shared write half.
    pub(super) write: Arc<Mutex<WsSink>>,
    /// TLS-verified client leaf certificate, when one was presented.
    pub(super) verified_leaf: Option<VerifiedLeaf>,
}

/// The bound listener with its TLS acceptor and handshake timeouts.
pub(super) struct HubListener {
    /// Bound TCP listener.
    pub(super) listener: TcpListener,
    /// TLS acceptor for new connections.
    pub(super) tls_acceptor: TlsAcceptor,
    /// Per-phase handshake timeouts.
    pub(super) timeouts: ScHubHandshakeTimeouts,
}
