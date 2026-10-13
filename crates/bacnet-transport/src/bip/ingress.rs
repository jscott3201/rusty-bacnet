//! The B/IP receive loop, and what it tells the node about one datagram:
//! its source, what its destination address was for this node, and which
//! socket read it.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bacnet_bip_core::node::{Addressed, Datagram, Discard, LocalAddresses, Path};
use tracing::{debug, warn};

use crate::udp_metadata::{DestinationReceiver, IpVersion, ReceivedDatagram};

use super::io::{receive, RecvContext};
use super::receive_counters::{DropReason, ReceiveDrops};
use super::socket::{BipSocket, BoundSockets};

/// Which of a transport's sockets a datagram arrived on (socket.rs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Arrival {
    /// The socket every send leaves from.
    Primary,
    /// A receive-only broadcast listener beside a socket bound to the
    /// interface address, which keeps only broadcasts (#1538).
    BroadcastListener,
}

/// One socket the receive loop reads, with its own buffer.
pub(super) struct Listener {
    socket: Arc<BipSocket>,
    receiver: DestinationReceiver,
    buf: Vec<u8>,
}

impl Listener {
    /// Hand a bound socket to tokio, reading the destination of each
    /// datagram. Must run inside a tokio runtime.
    fn open(socket: socket2::Socket, lease: Option<Arc<()>>) -> io::Result<Self> {
        let receiver = DestinationReceiver::configure(&socket, IpVersion::V4)?;
        let socket = tokio::net::UdpSocket::from_std(socket.into())?;
        Ok(Self {
            socket: Arc::new(BipSocket::new(socket, lease)),
            receiver,
            buf: vec![0; 2048],
        })
    }

    async fn recv(&mut self) -> io::Result<ReceivedDatagram> {
        self.receiver.recv_from(&self.socket, &mut self.buf).await
    }
}

/// A listener's next datagram, or never, once it is closed.
async fn recv_on(listener: Option<&mut Listener>) -> io::Result<ReceivedDatagram> {
    match listener {
        Some(listener) => listener.recv().await,
        None => std::future::pending().await,
    }
}

/// The sockets one receive loop reads: the primary, and in per-address mode
/// up to two broadcast listeners (socket.rs).
pub(super) struct Listeners {
    primary: Listener,
    broadcast: [Option<Listener>; 2],
}

/// Which socket [`Listeners::recv`] read: `None` for the primary, or the
/// index of a broadcast listener.
pub(super) type Source = Option<usize>;

impl Listeners {
    /// Hand the sockets one start bound to tokio. Each keeps `lease` until
    /// it closes.
    pub(super) fn open(bound: BoundSockets, lease: Option<Arc<()>>) -> io::Result<Self> {
        let mut broadcast = [None, None];
        for (slot, socket) in broadcast.iter_mut().zip(bound.listeners) {
            *slot = Some(Listener::open(socket, lease.clone())?);
        }
        let primary = Listener::open(bound.primary, lease)?;
        Ok(Self { primary, broadcast })
    }

    /// The socket every send leaves from.
    pub(super) fn primary(&self) -> &Arc<BipSocket> {
        &self.primary.socket
    }

    /// The next datagram on any socket. Each read is cancel-safe: the
    /// datagram leaves the socket only inside the read that returns it. With
    /// no listener, only the primary is read, in arrival order.
    async fn recv(&mut self) -> (Source, io::Result<ReceivedDatagram>) {
        let Self {
            primary,
            broadcast: [first, second],
        } = self;
        if first.is_none() && second.is_none() {
            return (None, primary.recv().await);
        }
        tokio::select! {
            received = primary.recv() => (None, received),
            received = recv_on(first.as_mut()) => (Some(0), received),
            received = recv_on(second.as_mut()) => (Some(1), received),
        }
    }

    fn data(&self, source: Source, len: usize) -> &[u8] {
        let listener = source
            .and_then(|index| self.broadcast[index].as_ref())
            .unwrap_or(&self.primary);
        &listener.buf[..len]
    }
}

/// Read every datagram the transport's sockets receive, until the primary
/// fails. A failed broadcast listener is closed, and unicast goes on.
pub(super) async fn receive_loop(
    mut listeners: Listeners,
    local: IngressAddresses,
    ctx: RecvContext,
) {
    loop {
        let (source, received) = listeners.recv().await;
        let arrival = match source {
            None => Arrival::Primary,
            Some(_) => Arrival::BroadcastListener,
        };
        match received {
            Ok(received) => {
                let data = listeners.data(source, received.len);
                handle_datagram(data, &received, arrival, &local, &ctx).await;
            }
            Err(e) => match receive_failed(source, &e, &ctx.drops) {
                AfterReceiveError::Continue => {}
                AfterReceiveError::CloseListener(index) => listeners.broadcast[index] = None,
                AfterReceiveError::Stop => break,
            },
        }
    }
}

/// What the receive loop does after a failed read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AfterReceiveError {
    /// Read on: the error cost one datagram.
    Continue,
    /// Close the broadcast listener with this index, and read on.
    CloseListener(usize),
    /// Stop receiving: the primary socket failed.
    Stop,
}

/// Count and log a failed read on `source`, and say what follows. An
/// `InvalidData` error drops one datagram (too large for the buffer, without
/// usable metadata, or on Windows an error reported for an earlier send; see
/// udp_metadata.rs) and reading goes on. Any other error closes the socket
/// it came from.
pub(super) fn receive_failed(
    source: Source,
    error: &io::Error,
    drops: &ReceiveDrops,
) -> AfterReceiveError {
    drops.count(DropReason::ReceiveError);
    if error.kind() == io::ErrorKind::InvalidData {
        debug!(%error, "Dropping UDP datagram with invalid destination metadata");
        return AfterReceiveError::Continue;
    }
    match source {
        None => {
            warn!(%error, "UDP recv error");
            AfterReceiveError::Stop
        }
        Some(index) => {
            warn!(%error, "B/IP broadcast listener failed; closing it");
            AfterReceiveError::CloseListener(index)
        }
    }
}

/// Whether the configured broadcast address is one of this host's own
/// non-loopback addresses, as on a /32 or point-to-point link set up by
/// mistake. Broadcasts sent there reach only this host. Loopback tests use the
/// same setup on purpose, so loopback addresses don't count.
pub(super) fn broadcast_is_own_address(
    broadcast: Ipv4Addr,
    local_ip: Ipv4Addr,
    unicast_ips: &[Ipv4Addr],
) -> bool {
    !broadcast.is_loopback() && (broadcast == local_ip || unicast_ips.contains(&broadcast))
}

/// This node's own addresses, which an inbound datagram's destination is
/// judged against, besides the node's own address and its subnet's
/// broadcast address, which its link holds.
pub(super) struct IngressAddresses {
    /// The host's IPv4 addresses, which a wildcard bind takes unicast to.
    pub(super) unicast_ips: Vec<Ipv4Addr>,
    /// Whether the socket is bound to 0.0.0.0 rather than an interface.
    pub(super) wildcard_bind: bool,
    /// In per-address mode, the index of the interface the transport is
    /// bound to, when the host reports it. The broadcast listeners keep only
    /// what arrived on it.
    pub(super) listener_interface: Option<u32>,
    /// Set by the first broadcast dropped for arriving on another interface,
    /// which is logged as a warning; the rest are logged at debug.
    pub(super) interface_mismatch_seen: AtomicBool,
}

/// Whether a datagram a broadcast listener read came in on another
/// interface than the transport's (#1538). A listener bound to the limited
/// broadcast (Linux) or the wildcard address (macOS and the BSDs) gets the
/// broadcasts of every interface on the port, and one sent on another
/// network is not this network's. With either index unknown (or zero, which
/// names no interface), it stays.
pub(super) fn arrived_elsewhere(listener_interface: Option<u32>, arrival: Option<u32>) -> bool {
    matches!((listener_interface, arrival), (Some(own), Some(arrival)) if arrival != 0 && own != arrival)
}

/// What `destination` was for this node (Annex J.2.11, J.2.12, J.3, J.4.1):
/// its own unicast address, the broadcast address, both or neither. A
/// wildcard bind takes unicast only to one of the host's listed addresses,
/// on every OS (#952), and never counts the broadcast address as its own.
pub(super) fn addressed(
    destination: IpAddr,
    os_group_delivery: Option<bool>,
    local: &IngressAddresses,
    ctx: &RecvContext,
) -> Addressed {
    let IpAddr::V4(destination) = destination else {
        return Addressed::default();
    };
    let link = ctx.node.link();
    let mut addresses = LocalAddresses::new(*link.address.ip(), *link.subnet.broadcast.ip());
    addresses.wildcard = local.wildcard_bind.then_some(local.unicast_ips.as_slice());
    Addressed::classify(destination, os_group_delivery, &addresses)
}

/// Hand one received datagram to the node, and do what it decides. A
/// broadcast listener keeps only broadcasts sent to this link's broadcast
/// address that came in on the transport's own interface (#1538): unicast
/// to a local address no socket on the port is bound to also reaches a
/// wildcard listener (macOS and the BSDs), and a listener gets the
/// broadcasts of every interface on the port. Neither is this transport's,
/// and another node on this port may share the listener, so the node drops
/// such a datagram before its frame is read.
pub(super) async fn handle_datagram(
    data: &[u8],
    received: &ReceivedDatagram,
    arrival: Arrival,
    local: &IngressAddresses,
    ctx: &RecvContext,
) {
    // An IPv4 socket reports only IPv4 peers: any other is unusable
    // metadata, as in `receive_failed`.
    let SocketAddr::V4(source) = received.peer else {
        ctx.drops.count(DropReason::ReceiveError);
        return;
    };
    let path = match arrival {
        Arrival::Primary => Path::Primary,
        Arrival::BroadcastListener => Path::Listener {
            other_interface: arrived_elsewhere(local.listener_interface, received.arrival_index),
        },
    };
    // Every read reports its destination, so the node never sees an unknown
    // one. A datagram the buffer cut short fails the read
    // (`receive_failed`), so none here is truncated.
    let to = addressed(received.destination, received.os_group_delivery, local, ctx);
    let mut datagram = Datagram::new(data, source, Some(to));
    datagram.path = path;
    if let Some(Discard::ListenerMismatch { other_interface }) = receive(&datagram, ctx).await {
        log_listener_mismatch(other_interface, received, local);
    }
}

/// Log a datagram a broadcast listener didn't keep: the first one that
/// arrived on another interface at warn, the rest at debug.
fn log_listener_mismatch(
    other_interface: bool,
    received: &ReceivedDatagram,
    local: &IngressAddresses,
) {
    if !other_interface {
        debug!(
            destination = %received.destination,
            "Dropping a datagram not sent to this link's broadcast address on the B/IP broadcast listener"
        );
        return;
    }
    let (own, arrival) = (local.listener_interface, received.arrival_index);
    if local.interface_mismatch_seen.swap(true, Ordering::Relaxed) {
        debug!(
            ?own,
            ?arrival,
            "Dropping a broadcast that arrived on another interface"
        );
    } else {
        // The index was resolved at start; an interface that came back
        // with a new one needs a restart.
        warn!(
            ?own,
            ?arrival,
            "Dropping a broadcast that arrived on another interface than the B/IP \
             transport's; if its interface changed since start, restart the transport"
        );
    }
}
