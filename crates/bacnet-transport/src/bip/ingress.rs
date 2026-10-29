//! The B/IP receive loop, and what it tells the node about one datagram:
//! its source, what its destination address was for this node, and which
//! socket read it.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bacnet_bip_core::node::{Addressed, Datagram, Discard, LocalAddresses, Path};
use tracing::{debug, warn};

use crate::tap::FrameTap;
use crate::udp_metadata::{
    DestinationReceiver, IpVersion, ReceivedDatagram, request_arrival_stamps,
};

use super::io::{RecvContext, receive};
use super::multicast::GroupSwap;
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
    /// The primary reports each datagram to the tap as it is handed up; a
    /// broadcast listener only what the node keeps ([`handle_datagram`]).
    tap_at_read: bool,
    /// In ordered mode, a datagram read ahead of its turn, whose payload
    /// waits in `buf` ([`Listeners::recv_in_order`]).
    held: Option<Held>,
}

/// A datagram read and not yet handed up, numbered in the order the
/// ordered receive read it.
pub(super) struct Held {
    pub(super) datagram: ReceivedDatagram,
    pub(super) read: u64,
}

impl Held {
    /// Whether this datagram goes up before `other`: by the kernel's
    /// receive time where both have one and they differ, or else by the
    /// order the two were read.
    pub(super) fn before(&self, other: &Held) -> bool {
        match (self.datagram.arrival_stamp, other.datagram.arrival_stamp) {
            (Some(own), Some(theirs)) if own != theirs => own < theirs,
            _ => self.read < other.read,
        }
    }
}

impl Listener {
    /// Hand a bound socket to tokio, reading the destination of each
    /// datagram, and with `arrival_stamp` the time it arrived. Must run inside
    /// a tokio runtime.
    fn open(
        socket: socket2::Socket,
        lease: Option<Arc<()>>,
        tap: &FrameTap,
        tap_at_read: bool,
        arrival_stamp: bool,
    ) -> io::Result<Self> {
        let receiver = DestinationReceiver::configure(&socket, IpVersion::V4)?;
        if arrival_stamp && let Err(error) = request_arrival_stamps(&socket) {
            debug!(%error, "No receive timestamps; datagrams go up in the order they are read");
        }
        let socket = tokio::net::UdpSocket::from_std(socket.into())?;
        Ok(Self {
            socket: Arc::new(BipSocket::new(socket, lease).tapped(tap.clone())),
            receiver,
            buf: vec![0; 2048],
            tap_at_read,
            held: None,
        })
    }

    /// A B/IP-M group listener an activation opened (multicast.rs): set up
    /// as a listener that reads in arrival order, keeping `lease` and
    /// reporting what the node keeps to `tap`. Must run inside a tokio
    /// runtime. Not used on Windows, whose primary socket joins the group.
    #[cfg_attr(windows, allow(dead_code))]
    pub(super) fn open_group(
        socket: socket2::Socket,
        lease: Option<Arc<()>>,
        tap: &FrameTap,
    ) -> io::Result<Self> {
        Self::open(socket, lease, tap, false, true)
    }

    /// The next datagram, which the primary's tap sees as it is read.
    async fn recv(&mut self) -> io::Result<ReceivedDatagram> {
        let received = self.receiver.recv_from(&self.socket, &mut self.buf).await?;
        self.tap_read(&received);
        Ok(received)
    }

    /// The next datagram, unseen by the tap until it is handed up.
    async fn read(&mut self) -> io::Result<ReceivedDatagram> {
        self.receiver.recv_from(&self.socket, &mut self.buf).await
    }

    /// The datagram the kernel already holds for this socket, or
    /// `WouldBlock`.
    fn read_queued(&mut self) -> io::Result<ReceivedDatagram> {
        self.receiver.recv_queued(&self.socket, &mut self.buf)
    }

    fn tap_read(&self, received: &ReceivedDatagram) {
        if self.tap_at_read {
            let datagram = self.buf.get(..received.len).unwrap_or_default();
            self.socket.tap_inbound(datagram, received.peer);
        }
    }
}

/// A listener's next datagram, or never, once it is closed.
async fn read_on(listener: Option<&mut Listener>) -> io::Result<ReceivedDatagram> {
    match listener {
        Some(listener) => listener.read().await,
        None => std::future::pending().await,
    }
}

/// The sockets one receive loop reads: the primary, and in per-address mode
/// up to two broadcast listeners (socket.rs).
pub(super) struct Listeners {
    primary: Listener,
    broadcast: [Option<Listener>; 2],
    /// Whether the start bound broadcast listeners, so that datagrams are
    /// handed up in the order the kernel received them across the sockets
    /// (#1546). Fixed at open: it stays on after a listener closes.
    ordered: bool,
    /// How many datagrams the ordered receive has read, which numbers each.
    reads: u64,
}

/// Which socket [`Listeners::recv`] read: `None` for the primary, or the
/// index of a broadcast listener.
pub(super) type Source = Option<usize>;

impl Listeners {
    /// Hand the sockets one start bound to tokio. Each keeps `lease` until
    /// it closes, and reports its datagrams to `tap`. With broadcast
    /// listeners, every socket asks for receive timestamps.
    pub(super) fn open(
        bound: BoundSockets,
        lease: Option<Arc<()>>,
        tap: &FrameTap,
    ) -> io::Result<Self> {
        let ordered = !bound.listeners.is_empty();
        let mut broadcast = [None, None];
        for (slot, socket) in broadcast.iter_mut().zip(bound.listeners) {
            *slot = Some(Listener::open(socket, lease.clone(), tap, false, ordered)?);
        }
        let primary = Listener::open(bound.primary, lease, tap, true, ordered)?;
        Ok(Self {
            primary,
            broadcast,
            ordered,
            reads: 0,
        })
    }

    /// Read a B/IP-M node's new group listener in place of the old one, or,
    /// with `None`, no listener: an activation moved the group or ended it
    /// (multicast.rs), and opened the listener already, so nothing here can
    /// fail. A member binds no other listener, so the group's is the first.
    /// A datagram the old listener read ahead and still held is dropped
    /// with it: it was sent to the group being left. From here on the
    /// sockets are read in arrival order, as with any listener; the primary
    /// reports no receive stamps unless it started with a listener, and
    /// read order then decides.
    pub(super) fn replace_group(&mut self, listener: Option<Listener>) {
        if listener.is_some() {
            self.ordered = true;
        }
        self.broadcast[0] = listener;
    }

    /// The socket every send leaves from.
    pub(super) fn primary(&self) -> &Arc<BipSocket> {
        &self.primary.socket
    }

    /// The next datagram. Each read is cancel-safe: a datagram leaves its
    /// socket only inside the read that returns it, or is held in its
    /// listener. With no broadcast listener, only the primary is read, in
    /// arrival order, as before #1546.
    pub(super) async fn recv(&mut self) -> (Source, io::Result<ReceivedDatagram>) {
        if !self.ordered {
            return (None, self.primary.recv().await);
        }
        self.recv_in_order().await
    }

    /// The next datagram across every socket, in the order the kernel
    /// received them (#1546). Each socket holds at most one datagram read
    /// ahead. With none held, it waits for any socket; then it reads what
    /// the others already have queued, without waiting, and hands up the
    /// earliest. A datagram found alone goes up at once, so ordering adds a
    /// read attempt per other socket and no delay. One that the kernel
    /// stamped earlier but queued only after that check goes up next. Equal
    /// or missing stamps (coarse ticks, Linux's start-up moment) fall back to
    /// read order, and a step of Linux's realtime clock between two arrivals
    /// can reorder them.
    async fn recv_in_order(&mut self) -> (Source, io::Result<ReceivedDatagram>) {
        let Self {
            primary,
            broadcast: [first, second],
            reads,
            ..
        } = self;
        let mut slots = [
            (None, Some(primary)),
            (Some(0), first.as_mut()),
            (Some(1), second.as_mut()),
        ];
        if slots
            .iter()
            .all(|(_, listener)| listener.as_ref().is_none_or(|l| l.held.is_none()))
        {
            let [(_, primary), (_, first), (_, second)] = &mut slots;
            let (source, received) = tokio::select! {
                received = read_on(primary.as_deref_mut()) => (None, received),
                received = read_on(first.as_deref_mut()) => (Some(0), received),
                received = read_on(second.as_deref_mut()) => (Some(1), received),
            };
            let Some((_, Some(listener))) = slots.iter_mut().find(|(at, _)| *at == source) else {
                unreachable!("the source is one of the slots, and only an open one reads");
            };
            match received {
                Ok(datagram) => {
                    listener.held = Some(Held {
                        datagram,
                        read: next(reads),
                    })
                }
                Err(error) => return (source, Err(error)),
            }
        }
        for (source, listener) in &mut slots {
            let Some(listener) = listener.as_deref_mut().filter(|l| l.held.is_none()) else {
                continue;
            };
            match listener.read_queued() {
                Ok(datagram) => {
                    listener.held = Some(Held {
                        datagram,
                        read: next(reads),
                    })
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return (*source, Err(error)),
            }
        }
        let mut earliest: Option<usize> = None;
        for index in 0..slots.len() {
            let Some(held) = held_in(&slots[index]) else {
                continue;
            };
            if earliest
                .and_then(|e| held_in(&slots[e]))
                .is_none_or(|e| held.before(e))
            {
                earliest = Some(index);
            }
        }
        let Some((source, Some(listener))) = earliest.map(|index| &mut slots[index]) else {
            unreachable!("the wait above leaves a datagram held, or returns");
        };
        let (source, Some(held)) = (*source, listener.held.take()) else {
            unreachable!("only a listener holding a datagram is chosen");
        };
        listener.tap_read(&held.datagram);
        (source, Ok(held.datagram))
    }

    pub(super) fn data(&self, source: Source, len: usize) -> &[u8] {
        let listener = source
            .and_then(|index| self.broadcast[index].as_ref())
            .unwrap_or(&self.primary);
        &listener.buf[..len]
    }
}

/// The datagram a slot's listener holds, if it is open and holds one.
fn held_in<'a>((_, listener): &'a (Source, Option<&mut Listener>)) -> Option<&'a Held> {
    listener.as_ref()?.held.as_ref()
}

/// The next number in `reads`.
fn next(reads: &mut u64) -> u64 {
    *reads += 1;
    *reads
}

/// Read every datagram the transport's sockets receive, until the primary
/// fails. A failed broadcast listener is closed, and unicast goes on. A
/// B/IP-M group listener that `swap` hands over replaces the one read
/// before (multicast.rs).
pub(super) async fn receive_loop(
    mut listeners: Listeners,
    local: IngressAddresses,
    ctx: RecvContext,
    swap: Option<Arc<GroupSwap>>,
) {
    loop {
        // Only a transport that can change its group waits on the
        // hand-off, so the others pay nothing for it. Both arms are
        // cancel-safe and take no lock across an await: the receive holds
        // a datagram it read ahead in its listener.
        let (source, received) = match swap.as_deref() {
            Some(swap) => tokio::select! {
                biased;
                listener = swap.next() => {
                    listeners.replace_group(listener);
                    continue;
                }
                received = listeners.recv() => received,
            },
            None => listeners.recv().await,
        };
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
    // The tap sees only what the node keeps, and sees it before the node
    // acts on it, which may send: the node's own listener verdict decides
    // (#1856).
    if arrival == Arrival::BroadcastListener && ctx.node.link().listener_keeps(&datagram) {
        ctx.socket().tap_inbound(data, received.peer);
    }
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
