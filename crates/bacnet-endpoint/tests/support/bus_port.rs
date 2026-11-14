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
//! `max_npdu` keeps its default, 1497 octets each way,
//! which is what a running `BipTransport` reports.
//!
//! The crate's unit tests include this file once, as `session::bus_port`,
//! and integration tests include it by path, like `port_retry.rs`.
use bacnet_transport::bvll::encode_bip_mac;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_transport::test_transport::{SendMode, TestTransport};
use bacnet_types::MacAddr;
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

/// One node's port on the link: it joins on an unused port, takes frames
/// from its own queue, leaves the link when dropped, and reports the limited
/// broadcast as its B/IP broadcast endpoint. A broadcast panics.
pub fn bus_port() -> TestTransport {
    let (tx, rx) = mpsc::channel(QUEUE);
    let mac = {
        let mut nodes = nodes();
        let port = nodes.next_port;
        nodes.next_port = port.checked_add(1).expect("bus ports exhausted");
        let mac = MacAddr::from_slice(&encode_bip_mac([127, 0, 0, 1], port));
        nodes.queues.push((mac.clone(), tx));
        mac
    };
    let (source, leaving) = (mac.clone(), mac.clone());
    TestTransport::builder()
        .local_mac(&mac)
        .inbound(rx)
        .broadcast(SendMode::Panic("BusPort has no broadcast"))
        .on_bip_broadcast_endpoint(|| Some(SocketAddrV4::new(Ipv4Addr::BROADCAST, 47808)))
        .on_send(move |frame| {
            let queue = nodes()
                .queues
                .iter()
                .find(|(node, _)| node.as_slice() == frame.mac.as_slice())
                .map(|(_, queue)| queue.clone());
            // As on a real link, a frame to no node or to a full queue is lost.
            if let Some(queue) = queue {
                let _ = queue.try_send(ReceivedNpdu {
                    direct_response: None,
                    npdu: frame.npdu,
                    source_mac: source.clone(),
                    link_layer_group: false,
                    data_attributes: Vec::new(),
                    provenance: TransportProvenance::unverified(),
                    reply_tx: None,
                });
            }
            async { Ok(()) }
        })
        .on_drop(move || nodes().queues.retain(|(mac, _)| *mac != leaving))
        .build()
}
