//! An in-memory link for the endpoint tests (#2293, #2298).
//!
//! Many tests used to talk over real UDP sockets with real-time APDU
//! timeouts, so a test thread stalled on a loaded host failed a read that
//! its peer had answered. Paused Tokio time can't fix that over sockets: the
//! clock jumps ahead whenever every task waits, even while a datagram is
//! still on its way. This link carries frames over Tokio channels instead.
//! A send wakes the receiving task at once, so paused time moves only when
//! nothing is in flight, and every deadline the tests meet is virtual.
//!
//! Each node gets a 6-byte B/IP MAC on 127.0.0.1, so the source Audit route
//! bindings still name it by socket address, and a session reports it as its
//! B/IP address. A unicast reaches the node that owns the MAC. No test here
//! broadcasts, so a broadcast panics rather than pass without its frame
//! reaching anyone.
//!
//! [`TransportPort::max_npdu`] keeps its default, 1497 octets each way,
//! which is what a running `BipTransport` reports.
//!
//! The crate's unit tests include this file once, as `session::bus_port`,
//! and integration tests include it by path, like `port_retry.rs`.
use bacnet_transport::bvll::encode_bip_mac;
use bacnet_transport::port::{ReceivedNpdu, TransportPort, TransportProvenance};
use bacnet_types::MacAddr;
use bacnet_types::error::Error;
use bytes::Bytes;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{Mutex, MutexGuard, PoisonError};
use tokio::sync::mpsc;

/// Each node's receive queue, as deep as the loopback transport's.
const QUEUE: usize = 256;

/// The nodes on the link and the next unused UDP port.
struct Nodes {
    next_port: u16,
    queues: Vec<(MacAddr, mpsc::Sender<ReceivedNpdu>)>,
}

/// One link per test process. Ports are never reused, so tests that share
/// a process (plain `cargo test`) can't reach each other's nodes.
static NODES: Mutex<Nodes> = Mutex::new(Nodes {
    next_port: 40_000,
    queues: Vec::new(),
});

fn nodes() -> MutexGuard<'static, Nodes> {
    NODES.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One node's port on the link.
pub struct BusPort {
    mac: MacAddr,
    rx: Option<mpsc::Receiver<ReceivedNpdu>>,
}

impl BusPort {
    /// Joins the link on an unused port.
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(QUEUE);
        let mut nodes = nodes();
        let port = nodes.next_port;
        nodes.next_port = port.checked_add(1).expect("bus ports exhausted");
        let mac = MacAddr::from_slice(&encode_bip_mac([127, 0, 0, 1], port));
        nodes.queues.push((mac.clone(), tx));
        Self { mac, rx: Some(rx) }
    }
}

impl Drop for BusPort {
    fn drop(&mut self) {
        nodes().queues.retain(|(mac, _)| *mac != self.mac);
    }
}

impl TransportPort for BusPort {
    fn bip_broadcast_endpoint(&self) -> Option<SocketAddrV4> {
        Some(SocketAddrV4::new(Ipv4Addr::BROADCAST, 47808))
    }
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.rx
            .take()
            .ok_or_else(|| Error::Encoding("bus port already started".into()))
    }
    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let queue = nodes()
            .queues
            .iter()
            .find(|(node, _)| node.as_slice() == mac)
            .map(|(_, queue)| queue.clone());
        // As on a real link, a frame to no node or to a full queue is lost.
        if let Some(queue) = queue {
            let _ = queue.try_send(ReceivedNpdu {
                direct_response: None,
                npdu: Bytes::copy_from_slice(npdu),
                source_mac: self.mac.clone(),
                link_layer_group: false,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            });
        }
        Ok(())
    }
    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        panic!("BusPort has no broadcast");
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }
    fn local_mac(&self) -> &[u8] {
        &self.mac
    }
}
