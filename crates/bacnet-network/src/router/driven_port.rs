//! A router port the test drives, for the suites that start a whole
//! [`BACnetRouter`](super::BACnetRouter) with no sockets.
//!
//! Frames come from a queue the test feeds, and the sends the router asks
//! for land in one the test reads. The port carries [`Handle::limit`]
//! octets each way, refusing a longer send, and publishes changes to it
//! only when the test hands it a watch.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};

use bacnet_npdu::NpduLimits;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::MacAddr;
use bacnet_types::error::Error;
use bytes::Bytes;
use tokio::sync::{mpsc, watch};

/// A send the router asked a [`driven`] port for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Sent {
    /// The NPDU.
    pub(super) npdu: Vec<u8>,
    /// The destination MAC, or `None` for a broadcast.
    pub(super) to: Option<Vec<u8>>,
    /// Whether the port took it, rather than refusing it.
    pub(super) taken: bool,
}

/// The test's ends of a [`driven`] port.
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

/// A port with `mac` that carries `limit` octets each way, with the test's
/// ends of it. It publishes no changes to its limit.
pub(super) fn driven(mac: u8, limit: u16) -> (TestTransport, Handle) {
    build(mac, limit, None)
}

/// [`driven`], publishing the changes to its limit on `changes`.
pub(super) fn driven_publishing(
    mac: u8,
    limit: u16,
    changes: watch::Receiver<NpduLimits>,
) -> (TestTransport, Handle) {
    build(mac, limit, Some(changes))
}

fn build(
    mac: u8,
    limit: u16,
    changes: Option<watch::Receiver<NpduLimits>>,
) -> (TestTransport, Handle) {
    let (frames, incoming) = mpsc::channel(16);
    let (sent, sent_rx) = mpsc::unbounded_channel();
    let limit = Arc::new(AtomicU16::new(limit));
    let down = Arc::new(AtomicBool::new(false));
    let started = Arc::new(AtomicBool::new(false));
    let mut port = TestTransport::builder()
        .local_mac(&[mac])
        .inbound(incoming)
        .on_start({
            let started = Arc::clone(&started);
            move || started.store(true, Ordering::SeqCst)
        })
        .on_max_npdu({
            let limit = Arc::clone(&limit);
            move || NpduLimits::both(limit.load(Ordering::SeqCst))
        })
        .on_send({
            let (limit, down) = (Arc::clone(&limit), Arc::clone(&down));
            // Record the send, then take it or refuse it.
            move |frame| {
                let limit = usize::from(limit.load(Ordering::SeqCst));
                let refusal = if down.load(Ordering::SeqCst) {
                    Some(Error::Encoding("the port is down".into()))
                } else if frame.npdu.len() > limit {
                    Some(Error::SendTooLong {
                        length: frame.npdu.len(),
                        limit,
                    })
                } else {
                    None
                };
                let _ = sent.send(Sent {
                    npdu: frame.npdu.to_vec(),
                    to: (!frame.broadcast).then(|| frame.mac.to_vec()),
                    taken: refusal.is_none(),
                });
                async move { refusal.map_or(Ok(()), Err) }
            }
        });
    if let Some(changes) = changes {
        port = port.max_npdu_changes(changes);
    }
    let handle = Handle {
        frames,
        sent: sent_rx,
        limit,
        down,
        started,
    };
    (port.build(), handle)
}
