//! An in-memory link with broadcasts, for tests that run several nodes on
//! paused time (#2617).
//!
//! Like `bus_port.rs`, it carries frames over Tokio channels, so a send
//! wakes its receiver at once and paused time moves only when nothing is in
//! flight. Unlike it, each test makes its own link, a broadcast reaches
//! every other node, and the test can put a frame of its own on the link,
//! or hold back what is addressed to one node and release it later.
use bacnet_transport::bvll::encode_bip_mac;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_transport::test_transport::{TestTransport, TestTransportBuilder};
use bacnet_types::MacAddr;
use bytes::Bytes;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::mpsc;

/// Each node's receive queue, as deep as the loopback transport's.
const QUEUE: usize = 256;

struct Node {
    mac: MacAddr,
    queue: mpsc::Sender<ReceivedNpdu>,
    /// Frames held back while the node's deliveries are on hold.
    held: Option<Vec<ReceivedNpdu>>,
}

/// One link. Clones name the same link.
#[derive(Clone, Default)]
pub struct Wire(Arc<Mutex<Vec<Node>>>);

/// A frame as a node on the link receives it from `source`.
pub fn frame(source: &[u8], npdu: Bytes, broadcast: bool) -> ReceivedNpdu {
    ReceivedNpdu {
        direct_response: None,
        npdu,
        source_mac: MacAddr::from_slice(source),
        link_layer_group: broadcast,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

impl Wire {
    pub fn new() -> Self {
        Self::default()
    }

    fn nodes(&self) -> MutexGuard<'_, Vec<Node>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Node `n`'s MAC: six octets, as a B/IP node on 127.0.0.1 has.
    pub fn mac(n: u8) -> MacAddr {
        MacAddr::from_slice(&encode_bip_mac([127, 0, 0, 1], 47_800 + u16::from(n)))
    }

    /// A transport for node `n`, still a builder so the test can add what
    /// it needs (`number_controls`, say) before `build`. Its unicasts reach
    /// the node that owns the MAC and its broadcasts every other node; a
    /// frame for no node, or for a full queue, is lost, as on a real link.
    pub fn port(&self, n: u8) -> TestTransportBuilder {
        self.port_with_queue(n, QUEUE)
    }

    /// [`Self::port`] with a receive queue `depth` frames deep, for a test
    /// that releases a burst longer than a socket's buffer would hold.
    pub fn port_with_queue(&self, n: u8, depth: usize) -> TestTransportBuilder {
        let (queue, inbound) = mpsc::channel(depth);
        let mac = Self::mac(n);
        self.nodes().push(Node {
            mac: mac.clone(),
            queue,
            held: None,
        });
        let wire = self.clone();
        let source = mac.clone();
        TestTransport::builder()
            .local_mac(&mac)
            .inbound(inbound)
            .on_send(move |sent| {
                if sent.broadcast {
                    wire.broadcast(&source, sent.npdu);
                } else {
                    wire.deliver(&sent.mac, frame(&source, sent.npdu, false));
                }
                async { Ok(()) }
            })
    }

    /// Put `received` in front of the node at `to`, or hold it back while
    /// that node's deliveries are on hold.
    pub fn deliver(&self, to: &[u8], received: ReceivedNpdu) {
        let mut nodes = self.nodes();
        let Some(node) = nodes.iter_mut().find(|node| node.mac.as_slice() == to) else {
            return;
        };
        match &mut node.held {
            Some(held) => held.push(received),
            None => {
                let _ = node.queue.try_send(received);
            }
        }
    }

    /// A broadcast of `npdu` from `source`, to every other node.
    pub fn broadcast(&self, source: &[u8], npdu: Bytes) {
        let others: Vec<MacAddr> = self
            .nodes()
            .iter()
            .filter(|node| node.mac.as_slice() != source)
            .map(|node| node.mac.clone())
            .collect();
        for mac in others {
            self.deliver(&mac, frame(source, npdu.clone(), true));
        }
    }

    /// Hold back every frame for the node at `mac` until [`Self::release`].
    pub fn hold(&self, mac: &[u8]) {
        if let Some(node) = self
            .nodes()
            .iter_mut()
            .find(|node| node.mac.as_slice() == mac)
        {
            node.held.get_or_insert_with(Vec::new);
        }
    }

    /// How many frames are held back for the node at `mac`.
    pub fn held(&self, mac: &[u8]) -> usize {
        self.nodes()
            .iter()
            .find(|node| node.mac.as_slice() == mac)
            .and_then(|node| node.held.as_ref().map(Vec::len))
            .unwrap_or(0)
    }

    /// Deliver what was held back for the node at `mac`, in order, and stop
    /// holding.
    pub fn release(&self, mac: &[u8]) {
        let mut nodes = self.nodes();
        if let Some(node) = nodes.iter_mut().find(|node| node.mac.as_slice() == mac) {
            for received in node.held.take().unwrap_or_default() {
                let _ = node.queue.try_send(received);
            }
        }
    }
}
