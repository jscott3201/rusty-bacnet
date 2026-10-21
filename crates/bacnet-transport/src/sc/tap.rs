//! The BACnet/SC side of the frame tap (#957): the hub connection's socket,
//! wrapped so that every BVLC-SC message sent or received over it reaches
//! the transport's [`FrameTap`].
//!
//! The wrapper sits above the [`WebSocketPort`] the application supplies,
//! so a message is taken in plaintext: before a TLS socket encrypts it and
//! after it decrypts it. Every hub socket the transport uses is wrapped
//! when the transport starts: the initial and failover sockets, and those
//! the reconnect and failover connectors dial later, each labelled with the
//! hub it goes to.

use std::sync::Arc;

use bacnet_types::data_link::DataLink;
use bacnet_types::error::Error;

use super::connector::WebSocketConnector;
use super::{ScActiveHub, ScTransport, WebSocketPort};
use crate::tap::{FrameTap, TapDirection, TapPeer};

/// A hub socket that reports what goes over it to a [`FrameTap`].
pub(super) struct TappedSocket<W> {
    inner: W,
    tap: FrameTap,
    hub: ScActiveHub,
}

impl<W> TappedSocket<W> {
    pub(super) fn new(inner: W, tap: &FrameTap, hub: ScActiveHub) -> Self {
        Self {
            inner,
            tap: tap.clone(),
            hub,
        }
    }

    /// The application's socket.
    #[cfg(feature = "sc-tls")]
    pub(super) fn inner(&self) -> &W {
        &self.inner
    }

    fn record(&self, direction: TapDirection, message: &[u8]) {
        let peer = || TapPeer::ScHub(self.hub);
        self.tap.record(direction, DataLink::Sc, peer, message);
    }
}

impl<W: WebSocketPort> WebSocketPort for TappedSocket<W> {
    /// Send, then report the message once the socket has taken it.
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        self.inner.send(data).await?;
        self.record(TapDirection::Outbound, data);
        Ok(())
    }

    /// Receive, reporting the message before handing it on. Reporting
    /// never waits, so this is as cancel safe as the wrapped receive.
    async fn recv(&self) -> Result<Vec<u8>, Error> {
        let data = self.inner.recv().await?;
        self.record(TapDirection::Inbound, &data);
        Ok(data)
    }
}

/// `connector`, with each socket it dials wrapped for `hub`.
pub(super) fn tapped_connector<W: WebSocketPort>(
    connector: &WebSocketConnector<W>,
    tap: &FrameTap,
    hub: ScActiveHub,
) -> WebSocketConnector<TappedSocket<W>> {
    let connector = Arc::clone(connector);
    let tap = tap.clone();
    Arc::new(move || {
        let dial = connector();
        let tap = tap.clone();
        Box::pin(async move { Ok(TappedSocket::new(dial.await?, &tap, hub)) })
    })
}

impl<W: WebSocketPort> ScTransport<W> {
    /// This transport's frame tap: every BVLC-SC message on the hub
    /// connection, Connect-Request and Connect-Accept, heartbeats, address
    /// resolution and Disconnect included, for whoever subscribes.
    ///
    /// Messages are taken in plaintext, before TLS on the way out and after
    /// it on the way in, so the tap shows this node's own traffic only and
    /// is no substitute for a wire capture. Direct connections are not
    /// tapped yet. Subscribe before
    /// [`start`](crate::port::TransportPort::start) to see the handshake.
    /// See [`crate::tap`].
    pub fn frame_tap(&self) -> &FrameTap {
        &self.frame_tap
    }
}

#[cfg(test)]
#[path = "tap_tests.rs"]
mod tests;
