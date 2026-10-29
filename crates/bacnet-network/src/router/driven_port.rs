//! A router port the test drives, for the suites that start a whole
//! [`BACnetRouter`](super::BACnetRouter) with no sockets.
//!
//! Frames come from a queue the test feeds, and the sends the router asks
//! for land in one the test reads. The port carries [`Driven::limit`]
//! octets each way, refusing a longer send, and publishes changes to it
//! only when the test hands it a watch.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};

use bacnet_npdu::NpduLimits;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::MacAddr;
use bacnet_types::error::Error;
use bytes::Bytes;
use tokio::sync::{mpsc, watch};

/// A send the router asked a [`Driven`] port for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Sent {
    /// The NPDU.
    pub(super) npdu: Vec<u8>,
    /// The destination MAC, or `None` for a broadcast.
    pub(super) to: Option<Vec<u8>>,
    /// Whether the port took it, rather than refusing it.
    pub(super) taken: bool,
}

/// The test's ends of a [`Driven`] port.
pub(super) struct Handle {
    /// Frames for the router, as from the link.
    pub(super) frames: mpsc::Sender<ReceivedNpdu>,
    /// What the router sent.
    pub(super) sent: mpsc::UnboundedReceiver<Sent>,
    /// The longest NPDU the port carries now, each way.
    pub(super) limit: Arc<AtomicU16>,
    /// Whether the port refuses every send.
    pub(super) down: Arc<AtomicBool>,
    /// Whether the router has started the port.
    pub(super) started: Arc<AtomicBool>,
}

impl Handle {
    /// Hand the router a unicast NPDU from `source`.
    pub(super) async fn receive(&self, npdu: Vec<u8>, source: &[u8]) {
        let frame = ReceivedNpdu::unverified(
            Bytes::from(npdu),
            MacAddr::from_slice(source),
            false,
            Vec::new(),
            None,
        );
        self.frames.send(frame).await.unwrap();
    }
}

/// The port itself.
pub(super) struct Driven {
    incoming: Option<mpsc::Receiver<ReceivedNpdu>>,
    mac: [u8; 1],
    limit: Arc<AtomicU16>,
    down: Arc<AtomicBool>,
    started: Arc<AtomicBool>,
    changes: Option<watch::Receiver<NpduLimits>>,
    sent: mpsc::UnboundedSender<Sent>,
}

impl Driven {
    /// A port with `mac` that carries `limit` octets each way, with the
    /// test's ends of it. It publishes no changes to its limit.
    pub(super) fn new(mac: u8, limit: u16) -> (Self, Handle) {
        let (frames, incoming) = mpsc::channel(16);
        let (sent, sent_rx) = mpsc::unbounded_channel();
        let limit = Arc::new(AtomicU16::new(limit));
        let down = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicBool::new(false));
        let port = Self {
            incoming: Some(incoming),
            mac: [mac],
            limit: Arc::clone(&limit),
            down: Arc::clone(&down),
            started: Arc::clone(&started),
            changes: None,
            sent,
        };
        let handle = Handle {
            frames,
            sent: sent_rx,
            limit,
            down,
            started,
        };
        (port, handle)
    }

    /// The same port, publishing the changes to its limit on `changes`.
    pub(super) fn publishing(mut self, changes: watch::Receiver<NpduLimits>) -> Self {
        self.changes = Some(changes);
        self
    }

    /// Record the send, then take it or refuse it.
    fn send(&self, npdu: &[u8], to: Option<&[u8]>) -> Result<(), Error> {
        let refusal = if self.down.load(Ordering::SeqCst) {
            Some(Error::Encoding("the port is down".into()))
        } else if npdu.len() > usize::from(self.limit.load(Ordering::SeqCst)) {
            Some(Error::SendTooLong {
                length: npdu.len(),
                limit: usize::from(self.limit.load(Ordering::SeqCst)),
            })
        } else {
            None
        };
        let sent = Sent {
            npdu: npdu.to_vec(),
            to: to.map(<[u8]>::to_vec),
            taken: refusal.is_none(),
        };
        let _ = self.sent.send(sent);
        refusal.map_or(Ok(()), Err)
    }
}

impl TransportPort for Driven {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.started.store(true, Ordering::SeqCst);
        self.incoming
            .take()
            .ok_or_else(|| Error::Encoding("already started".into()))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.send(npdu, Some(mac))
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.send(npdu, None)
    }

    fn local_mac(&self) -> &[u8] {
        &self.mac
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    fn max_npdu(&self) -> NpduLimits {
        NpduLimits::both(self.limit.load(Ordering::SeqCst))
    }

    fn max_npdu_changes(&self) -> Option<watch::Receiver<NpduLimits>> {
        self.changes.clone()
    }
}
