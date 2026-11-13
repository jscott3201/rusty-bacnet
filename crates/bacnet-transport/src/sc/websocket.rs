//! Socket I/O contract shared by hub and direct connection owners.
use std::time::Duration;

use bacnet_types::error::Error;

/// The close status for a connection this node ends on purpose: an RFC 6455
/// normal closure, which the peer maps to `WEBSOCKET_CLOSED_BY_PEER` (Annex
/// AB.7.5.5).
pub(crate) const CLOSE_NORMAL: u16 = 1000;

/// The close status for a peer that sent a data frame that is not binary
/// (Annex AB.7.5.3): `WEBSOCKET_DATA_NOT_ACCEPTED` at the other end.
#[cfg(feature = "sc-tls")]
pub(crate) const CLOSE_DATA_NOT_ACCEPTED: u16 = 1003;

/// The most time one close frame gets before the socket is dropped anyway,
/// so a peer that never reads cannot hold up a stop, a failover or a
/// reconnect. The hub's own closes use it too (#2520).
pub(crate) const CLOSE_GRACE: Duration = Duration::from_secs(1);

/// Abstraction over a WebSocket connection for BACnet/SC.
///
/// Implementations wrap the platform WebSocket driver (e.g. `tokio-tungstenite`).
/// A loopback implementation is provided for testing.
pub trait WebSocketPort: Send + Sync + 'static {
    /// Send a binary WebSocket message.
    fn send(&self, data: &[u8]) -> impl std::future::Future<Output = Result<(), Error>> + Send;
    /// Receive a binary WebSocket message. Blocks until a message is available.
    fn recv(&self) -> impl std::future::Future<Output = Result<Vec<u8>, Error>> + Send;
    /// Send a WebSocket close frame carrying `status`, or one with no status
    /// for `None`, and return once it is written. The transport calls this
    /// when it ends a connection (#2520): 1000 when it means to, such as on
    /// [`stop`](crate::port::TransportPort::stop), and no status when the
    /// peer stopped answering. It bounds the call and drops the socket
    /// next without waiting for the peer's answering close, so an
    /// implementation need not wait for it either.
    ///
    /// The default sends nothing and succeeds, for a port with no close
    /// frame of its own, such as [`LoopbackWebSocket`](super::LoopbackWebSocket).
    fn close(
        &self,
        status: Option<u16>,
    ) -> impl std::future::Future<Output = Result<(), Error>> + Send {
        let _ = status;
        async { Ok(()) }
    }

    /// Whether [`close`](Self::close) sends a close frame. When it does,
    /// the transport gives the peer up to the close grace to answer its
    /// Disconnect-Request before closing, as Annex AB.6.2.2 has the
    /// requester wait; a port without one is dropped without that wait. The
    /// default is `false`, matching the default `close`.
    fn has_close_frame(&self) -> bool {
        false
    }
}

/// Send `ws` a close frame with `status`, giving it at most [`CLOSE_GRACE`].
/// The caller drops the socket next, whether or not the frame went out.
pub(crate) async fn close_within_grace<W: WebSocketPort>(ws: &W, status: Option<u16>) {
    let _ = tokio::time::timeout(CLOSE_GRACE, ws.close(status)).await;
}
