//! In-process loopback transport for gateway composition.
//!
//! `LoopbackTransport` implements [`TransportPort`] using `tokio::sync::mpsc`
//! channels. A pair of loopback transports can be created with [`LoopbackTransport::pair`],
//! where sending on one side delivers to the other. This enables the gateway's
//! client and server to connect to the router via in-process channels rather
//! than real network sockets.
//!
//! The peer receives every frame, whatever MAC it was sent to.
//! [`LoopbackTransport::record_unicast_destinations`] reports those MACs, so a
//! test can see where each unicast went. The data attributes a frame is sent
//! with are dropped, as on a data link that cannot carry them, unless
//! [`LoopbackTransport::carry_data_attributes`] asks for them to reach the
//! peer. The link has no broadcast MAC unless
//! [`LoopbackTransport::set_broadcast_mac`] names one.
//!
//! Like the real data links, a send never waits for the peer: a frame that
//! finds the peer's 256-frame receive queue full is dropped and counted in
//! the peer's [`LoopbackTransport::receive_drops`] (#1713).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::mpsc;

use crate::port::{DataAttribute, ReceivedNpdu, TransportPort, TransportProvenance};

/// In-process loopback transport backed by mpsc channels.
pub struct LoopbackTransport {
    /// This transport's local MAC address.
    local_mac: MacAddr,
    /// Sender to deliver NPDUs to the peer.
    peer_tx: mpsc::Sender<ReceivedNpdu>,
    /// Receiver for NPDUs from the peer. Taken by `start()`.
    self_rx: Option<mpsc::Receiver<ReceivedNpdu>>,
    /// Frames the peer dropped because this side's queue was full.
    receive_drops: Arc<AtomicU64>,
    /// The peer's count of the same, for frames this side sends.
    peer_drops: Arc<AtomicU64>,
    /// Where each unicast's destination MAC goes, once
    /// [`Self::record_unicast_destinations`] asks for it.
    unicast_destinations: Option<mpsc::UnboundedSender<MacAddr>>,
    /// Held while a recorded unicast is queued for the peer, so concurrent
    /// sends record their MACs in the order the peer receives the frames.
    record_order: Mutex<()>,
    /// Whether the peer gets each frame's data attributes, once
    /// [`Self::carry_data_attributes`] asks for them.
    carries_data_attributes: bool,
    /// The MAC [`TransportPort::is_broadcast_mac`] recognises, once
    /// [`Self::set_broadcast_mac`] names one.
    broadcast_mac: Option<MacAddr>,
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
        let (drops_a, drops_b) = (Arc::default(), Arc::default());

        // A sends to B's rx, and counts B's drops.
        let a = Self::new(mac_a.into(), tx_b, rx_a, &drops_a, &drops_b);
        let b = Self::new(mac_b.into(), tx_a, rx_b, &drops_b, &drops_a);
        (a, b)
    }

    fn new(
        local_mac: MacAddr,
        peer_tx: mpsc::Sender<ReceivedNpdu>,
        self_rx: mpsc::Receiver<ReceivedNpdu>,
        receive_drops: &Arc<AtomicU64>,
        peer_drops: &Arc<AtomicU64>,
    ) -> Self {
        Self {
            local_mac,
            peer_tx,
            self_rx: Some(self_rx),
            receive_drops: Arc::clone(receive_drops),
            peer_drops: Arc::clone(peer_drops),
            unicast_destinations: None,
            record_order: Mutex::new(()),
            carries_data_attributes: false,
            broadcast_mac: None,
        }
    }

    /// Report the destination MAC of each unicast this transport sends from
    /// now on (#1243).
    ///
    /// The returned receiver gets the MAC given to each
    /// [`send_unicast`](TransportPort::send_unicast) that reaches the peer, in
    /// the order the peer receives those frames: the peer's n-th frame with
    /// `link_layer_group` false went to the n-th MAC here. Broadcasts add
    /// nothing. Take the receiver before handing the transport to a router or
    /// network layer.
    ///
    /// The record is unbounded until the receiver is dropped, which stops it.
    /// A later call replaces the earlier receiver.
    pub fn record_unicast_destinations(&mut self) -> mpsc::UnboundedReceiver<MacAddr> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.unicast_destinations = Some(tx);
        rx
    }

    /// Hand the peer the data attributes of each frame this transport sends
    /// from now on (#1289).
    ///
    /// The attributes given to
    /// [`send_unicast_with_data_attributes`](TransportPort::send_unicast_with_data_attributes)
    /// or
    /// [`send_broadcast_with_data_attributes`](TransportPort::send_broadcast_with_data_attributes)
    /// then arrive in the peer's [`ReceivedNpdu::data_attributes`], as they
    /// would over BACnet/SC. Without this call the transport drops them and
    /// the peer's frames carry none. Set it on the side that sends: a test
    /// that feeds a router attributes and checks what the router sends back
    /// sets it on both.
    pub fn carry_data_attributes(&mut self) {
        self.carries_data_attributes = true;
    }

    /// Report `mac` as this link's broadcast MAC from
    /// [`is_broadcast_mac`](TransportPort::is_broadcast_mac), as MS/TP
    /// reports `0xFF` (#1479). Only the answer changes: a unicast to `mac`
    /// still reaches the peer as one frame, not marked as a broadcast.
    pub fn set_broadcast_mac(&mut self, mac: impl Into<MacAddr>) {
        self.broadcast_mac = Some(mac.into());
    }

    /// Frames the peer sent that were dropped because this transport's
    /// receive queue was full.
    pub fn receive_drops(&self) -> u64 {
        self.receive_drops.load(Ordering::Relaxed)
    }

    /// Room for one frame in the peer's queue: `None` when it is full, the
    /// drop counted, as a full data link drops a frame.
    fn reserve(&self) -> Result<Option<mpsc::Permit<'_, ReceivedNpdu>>, Error> {
        match self.peer_tx.try_reserve() {
            Ok(permit) => Ok(Some(permit)),
            Err(mpsc::error::TrySendError::Full(())) => {
                self.peer_drops.fetch_add(1, Ordering::Relaxed);
                Ok(None)
            }
            Err(mpsc::error::TrySendError::Closed(())) => Err(peer_closed()),
        }
    }

    /// The frame the peer receives for `npdu`.
    fn frame(
        &self,
        npdu: &[u8],
        link_layer_group: bool,
        data_attributes: &[DataAttribute],
    ) -> ReceivedNpdu {
        let data_attributes = if self.carries_data_attributes {
            data_attributes.to_vec()
        } else {
            Vec::new()
        };
        ReceivedNpdu {
            direct_response: None,
            npdu: Bytes::copy_from_slice(npdu),
            source_mac: self.local_mac.clone(),
            link_layer_group,
            data_attributes,
            provenance: TransportProvenance::unverified(),
            reply_tx: None,
        }
    }

    async fn unicast(
        &self,
        npdu: &[u8],
        mac: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        // Take room first, then record the MAC and queue the frame under one
        // lock: the record and the peer's queue then share one order. A frame
        // dropped for want of room isn't recorded.
        let Some(permit) = self.reserve()? else {
            return Ok(());
        };
        let msg = self.frame(npdu, false, data_attributes);
        let Some(destinations) = self.unicast_destinations.as_ref() else {
            permit.send(msg);
            return Ok(());
        };
        let _order = self
            .record_order
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // A dropped record receiver only stops the record.
        let _ = destinations.send(MacAddr::from_slice(mac));
        permit.send(msg);
        Ok(())
    }

    async fn broadcast(&self, npdu: &[u8], data_attributes: &[DataAttribute]) -> Result<(), Error> {
        if let Some(permit) = self.reserve()? {
            permit.send(self.frame(npdu, true, data_attributes));
        }
        Ok(())
    }
}

fn peer_closed() -> Error {
    Error::Encoding("loopback peer channel closed".to_string())
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

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.unicast(npdu, mac, &[]).await
    }

    async fn send_unicast_with_data_attributes(
        &self,
        npdu: &[u8],
        mac: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        self.unicast(npdu, mac, data_attributes).await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.broadcast(npdu, &[]).await
    }

    async fn send_broadcast_with_data_attributes(
        &self,
        npdu: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        self.broadcast(npdu, data_attributes).await
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        self.broadcast_mac.as_deref() == Some(mac)
    }

    fn group_destinations(&self) -> crate::port::GroupDestinations {
        let broadcast = self.broadcast_mac.clone();
        crate::port::GroupDestinations::new(move |mac| broadcast.as_deref() == Some(mac))
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
        assert!(!npdu.link_layer_group);
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
        assert!(!npdu.link_layer_group);
    }

    #[tokio::test]
    async fn pair_broadcast() {
        let (mut a, mut b) = LoopbackTransport::pair(vec![0x00, 0x01], vec![0x00, 0x02]);
        let mut rx_b = b.start().await.unwrap();
        let _rx_a = a.start().await.unwrap();

        a.send_broadcast(b"bcast").await.unwrap();
        let npdu = rx_b.recv().await.unwrap();
        assert_eq!(&npdu.npdu[..], b"bcast");
        assert!(npdu.link_layer_group);
    }

    #[tokio::test]
    async fn start_twice_fails() {
        let (mut a, _b) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
        let _rx = a.start().await.unwrap();
        assert!(a.start().await.is_err());
    }

    #[tokio::test]
    async fn recorded_unicast_destinations_follow_the_peers_frames() {
        let (mut a, mut b) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
        let mut destinations = a.record_unicast_destinations();
        let mut rx_b = b.start().await.unwrap();

        // The peer gets all three frames, the first to a MAC not its own.
        a.send_unicast(b"one", &[0x50]).await.unwrap();
        a.send_broadcast(b"two").await.unwrap();
        a.send_unicast(b"three", &[0x02]).await.unwrap();
        for (npdu, group) in [(&b"one"[..], false), (b"two", true), (b"three", false)] {
            let frame = rx_b.recv().await.unwrap();
            assert_eq!(&frame.npdu[..], npdu);
            assert_eq!(frame.link_layer_group, group);
        }
        assert_eq!(destinations.try_recv().unwrap().as_slice(), [0x50]);
        assert_eq!(destinations.try_recv().unwrap().as_slice(), [0x02]);
        assert!(
            destinations.try_recv().is_err(),
            "a broadcast records nothing"
        );

        // Dropping the receiver stops the record; sending carries on.
        drop(destinations);
        a.send_unicast(b"four", &[0x60]).await.unwrap();
        assert_eq!(&rx_b.recv().await.unwrap().npdu[..], b"four");

        // A unicast that never reaches the peer records nothing.
        let mut destinations = a.record_unicast_destinations();
        drop(rx_b);
        assert!(a.send_unicast(b"five", &[0x70]).await.is_err());
        assert!(destinations.try_recv().is_err());
    }

    #[tokio::test]
    async fn concurrent_unicasts_record_destinations_in_delivery_order() {
        let (mut a, mut b) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
        let mut destinations = a.record_unicast_destinations();
        let mut rx_b = b.start().await.unwrap();
        let a = std::sync::Arc::new(a);

        // Four threads send 300 unicasts between them, more than the peer
        // queue holds. Loopback drops a frame when the peer's queue is full
        // (#1713), so wait for every send, then check that each delivered
        // frame matches its recorded MAC in order, and that every frame was
        // either delivered or counted as dropped.
        let threads: Vec<_> = (0..4u16)
            .map(|thread| {
                let a = std::sync::Arc::clone(&a);
                std::thread::spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .build()
                        .unwrap();
                    for i in 0..75 {
                        let id = (thread * 75 + i).to_be_bytes();
                        runtime.block_on(a.send_unicast(&id, &id)).unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let mut delivered = 0u64;
        while let Ok(frame) = rx_b.try_recv() {
            let mac = destinations.try_recv().unwrap();
            assert_eq!(mac.as_slice(), &frame.npdu[..]);
            delivered += 1;
        }
        assert!(
            destinations.try_recv().is_err(),
            "a dropped frame was recorded"
        );
        assert!(delivered > 0);
        assert_eq!(delivered + b.receive_drops(), 300);
    }

    #[tokio::test]
    async fn data_attributes_reach_the_peer_only_when_carried() {
        let (mut a, mut b) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
        let mut rx_b = b.start().await.unwrap();
        let attributes = vec![DataAttribute {
            option_type: 31,
            must_understand: true,
            data: vec![0x12, 0x34],
        }];

        // By default the peer gets the frames without their attributes.
        a.send_unicast_with_data_attributes(b"one", &[0x02], &attributes)
            .await
            .unwrap();
        a.send_broadcast_with_data_attributes(b"two", &attributes)
            .await
            .unwrap();
        for npdu in [&b"one"[..], b"two"] {
            let frame = rx_b.recv().await.unwrap();
            assert_eq!(&frame.npdu[..], npdu);
            assert!(frame.data_attributes.is_empty(), "dropped by default");
        }

        // Once asked, both kinds of frame carry them, also while unicast
        // destinations are recorded, and a frame sent without any has none.
        a.carry_data_attributes();
        let mut destinations = a.record_unicast_destinations();
        a.send_unicast_with_data_attributes(b"three", &[0x50], &attributes)
            .await
            .unwrap();
        a.send_broadcast_with_data_attributes(b"four", &attributes)
            .await
            .unwrap();
        a.send_unicast(b"five", &[0x02]).await.unwrap();
        for (npdu, carried) in [
            (&b"three"[..], &attributes[..]),
            (b"four", &attributes),
            (b"five", &[]),
        ] {
            let frame = rx_b.recv().await.unwrap();
            assert_eq!(&frame.npdu[..], npdu);
            assert_eq!(frame.data_attributes, carried);
        }
        assert_eq!(destinations.try_recv().unwrap().as_slice(), [0x50]);
        assert_eq!(destinations.try_recv().unwrap().as_slice(), [0x02]);
    }

    #[tokio::test]
    async fn local_mac_correct() {
        let (a, b) = LoopbackTransport::pair(vec![0xAA], vec![0xBB]);
        assert_eq!(a.local_mac(), &[0xAA]);
        assert_eq!(b.local_mac(), &[0xBB]);
    }
}
