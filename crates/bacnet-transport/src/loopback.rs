//! In-process loopback transport for gateway composition.
//!
//! `LoopbackTransport` implements [`TransportPort`] using `tokio::sync::mpsc`
//! channels. A pair of loopback transports can be created with [`LoopbackTransport::pair`],
//! where sending on one side delivers to the other. This enables the gateway's
//! client and server to connect to the router via in-process channels rather
//! than real network sockets.

use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::mpsc;

use crate::port::{ReceivedNpdu, TransportPort};

/// In-process loopback transport backed by mpsc channels.
pub struct LoopbackTransport {
    /// This transport's local MAC address.
    local_mac: MacAddr,
    /// Sender to deliver NPDUs to the peer.
    peer_tx: mpsc::Sender<ReceivedNpdu>,
    /// Receiver for NPDUs from the peer. Taken by `start()`.
    self_rx: Option<mpsc::Receiver<ReceivedNpdu>>,
}

impl LoopbackTransport {
    /// Create a connected pair of loopback transports.
    ///
    /// Each transport has its own MAC address. Sending on transport A delivers
    /// to transport B's receive channel, and vice versa.
    pub fn pair(mac_a: impl Into<MacAddr>, mac_b: impl Into<MacAddr>) -> (Self, Self) {
        /// NPDU receive channel capacity for loopback (matches high-throughput transports).
        const NPDU_CHANNEL_CAPACITY: usize = 256;

        let (tx_a, rx_a) = mpsc::channel(NPDU_CHANNEL_CAPACITY);
        let (tx_b, rx_b) = mpsc::channel(NPDU_CHANNEL_CAPACITY);

        let a = Self {
            local_mac: mac_a.into(),
            peer_tx: tx_b, // A sends to B's rx
            self_rx: Some(rx_a),
        };
        let b = Self {
            local_mac: mac_b.into(),
            peer_tx: tx_a, // B sends to A's rx
            self_rx: Some(rx_b),
        };

        (a, b)
    }
}

impl TransportPort for LoopbackTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.self_rx
            .take()
            .ok_or_else(|| Error::Encoding("loopback transport already started".to_string()))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        // Nothing to clean up — channels drop naturally.
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        let msg = ReceivedNpdu {
            npdu: Bytes::copy_from_slice(npdu),
            source_mac: self.local_mac.clone(),
            reply_tx: None,
        };
        self.peer_tx
            .send(msg)
            .await
            .map_err(|_| Error::Encoding("loopback peer channel closed".to_string()))
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.send_unicast(npdu, &[]).await
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pair_unicast_a_to_b() {
        let (mut a, mut b) = LoopbackTransport::pair(vec![0x00, 0x01], vec![0x00, 0x02]);
        let mut rx_b = b.start().await.unwrap();
        let _rx_a = a.start().await.unwrap();

        a.send_unicast(b"hello", &[0x00, 0x02]).await.unwrap();
        let npdu = rx_b.recv().await.unwrap();
        assert_eq!(&npdu.npdu[..], b"hello");
        assert_eq!(&npdu.source_mac[..], &[0x00, 0x01]);
    }

    #[tokio::test]
    async fn pair_unicast_b_to_a() {
        let (mut a, mut b) = LoopbackTransport::pair(vec![0x00, 0x01], vec![0x00, 0x02]);
        let mut rx_a = a.start().await.unwrap();
        let _rx_b = b.start().await.unwrap();

        b.send_unicast(b"world", &[0x00, 0x01]).await.unwrap();
        let npdu = rx_a.recv().await.unwrap();
        assert_eq!(&npdu.npdu[..], b"world");
        assert_eq!(&npdu.source_mac[..], &[0x00, 0x02]);
    }

    #[tokio::test]
    async fn pair_broadcast() {
        let (mut a, mut b) = LoopbackTransport::pair(vec![0x00, 0x01], vec![0x00, 0x02]);
        let mut rx_b = b.start().await.unwrap();
        let _rx_a = a.start().await.unwrap();

        a.send_broadcast(b"bcast").await.unwrap();
        let npdu = rx_b.recv().await.unwrap();
        assert_eq!(&npdu.npdu[..], b"bcast");
    }

    #[tokio::test]
    async fn start_twice_fails() {
        let (mut a, _b) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
        let _rx = a.start().await.unwrap();
        assert!(a.start().await.is_err());
    }

    #[tokio::test]
    async fn local_mac_correct() {
        let (a, b) = LoopbackTransport::pair(vec![0xAA], vec![0xBB]);
        assert_eq!(a.local_mac(), &[0xAA]);
        assert_eq!(b.local_mac(), &[0xBB]);
    }
}
