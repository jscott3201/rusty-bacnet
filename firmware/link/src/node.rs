//! The node image's scenario (#1667, #1766).
//!
//! [`run`] drives a foreign device through `bacnet-bip-core`'s port driver,
//! a [`NodePort`] over a [`Node`] with two exchange slots, set up as the
//! hosted B/IP transport sets one up ([`LINK`]): failed attempts retried
//! sooner, backing off; broadcasts held until the first registration
//! attempt resolves (two here, where the transport holds 16, in
//! [`HeldBroadcast`] slots the port borrows); Forwarded-NPDUs taken only
//! from its own BBMD, by unicast. Its send budget is sized to what it
//! sends, the least a link takes ([`BUDGET`]), where the hosted transport
//! sends B/IP's largest NPDU, 1497 octets: the held slots are only as long
//! as the link's broadcasts may be, so the budget is the knob that sizes
//! them ([`HELD_LENGTH`]). The port carries out what the node asks, and
//! every datagram leaves through the scenario's socket, a [`Datagrams`]
//! that encodes it with `bacnet-bip-codec`. Time is a fixed timeline of
//! `bacnet-time` instants, which the bip-core re-exports. It returns a
//! [`Summary`] of what it saw.
//!
//! - **The NAK matrix:** a peer sends each of the six requests only a BBMD
//!   serves ([`REQUESTS`]); each draws its own NAK, which the port sends
//!   back.
//! - **Held broadcasts:** while the first registration attempt is
//!   outstanding, two broadcasts are held and a third is refused. The
//!   BBMD's success registers the node and releases the two, each sent to
//!   the BBMD as a Distribute-Broadcast-To-Network.
//! - **Forwarded-NPDUs:** one from the BBMD, by unicast, is delivered from
//!   its originating address; one from another sender is dropped.
//! - **Register and renew:** the renewal goes one renewal interval into the
//!   lease, and its success moves the lease's end.
//! - **The DBTN hint:** the BBMD answers a broadcast with the
//!   Distribute-Broadcast-To-Network NAK, as a BBMD that has lost the
//!   registration does, and the next attempt starts at once instead of at
//!   the renewal.
//! - **Loss and lapse:** the BBMD stops answering. The attempts time out
//!   and back off until the lease lapses; then a success registers the node
//!   again, and the renewal that lease makes due follows at once.
//!
//! `tests/node.rs` runs it natively and compares the whole summary with
//! [`expected()`]; the `node` image (`src/bin/node.rs`) links it, and its
//! emulation build checks the emulated CPU's summary against the same.
//!
//! # Adapter notes
//!
//! The scenario plays the IP stack, so it knows where each datagram was
//! sent. A real image's adapter must report that for every datagram it
//! reads: [`Datagram::to`] from the packet's destination address, through
//! [`Addressed::classify`], since whether a message came to this node
//! alone or to the whole subnet decides what the node may do with it. An
//! IP stack that can't report the destination per packet passes `None`,
//! and the node delivers nothing from that datagram and drops it as
//! [`Discard::UnknownDestination`], never guessing (#1678).

mod expected;

pub use expected::expected;

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BdtEntries, BdtEntry, BvlcResultCode, Bvll, HEADER_LENGTH};
use bacnet_bip_core::node::{
    Addressed, Admission, AttemptOutcome, Cast, Datagram, Discard, Foreign, ForwardedTrust, Link,
    Mode, Node, NodeCounters, Permissive, RegisterAttempt, RegistrationSnapshot, Retry,
    SendRefusal, MIN_BUDGET,
};
use bacnet_bip_core::port::{Datagrams, HeldBroadcast, Ingress, NodePort};
use bacnet_bip_core::{Duration, Instant, LocalSubnet};

use crate::tally::Sent;

/// Exchange slots: one for the registration, one for a manual request.
pub const EXCHANGES: usize = 2;

/// The node as the image builds it.
pub type ForeignNode = Node<EXCHANGES>;

/// The node's datagram budget: the least a link takes, 64 octets, ample
/// for the scenario's 8-octet Who-Is. The hosted transport's links send
/// B/IP's largest datagram, 1507 octets, with NPDUs of up to 1497; firmware
/// sizes its budget to the NPDUs it sends.
pub const BUDGET: u16 = MIN_BUDGET;

/// The longest broadcast the link sends, so the length of a held slot: the
/// budget less the 4-octet header and the 6-octet originating address a
/// BBMD adds when it forwards the broadcast ([`Link::max_npdu`] for
/// [`Cast::Broadcast`]), 54 octets. [`NodePort::new`] refuses slots shorter
/// than that.
pub const HELD_LENGTH: usize = BUDGET as usize - HEADER_LENGTH - 6;

/// The port over the node, holding broadcasts in slots of [`HELD_LENGTH`]
/// octets.
pub type ForeignPort<'m> = NodePort<'m, EXCHANGES, HELD_LENGTH>;

/// The broadcasts the node holds until its first registration attempt
/// resolves: two, where the hosted transport holds 16, since firmware keeps
/// each one's NPDU in RAM until then.
pub const HOLD: u16 = 2;

/// The time-to-live the node registers with, in seconds.
pub const TTL: u16 = 60;

/// This node.
pub const OWN: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 10), 0xBAC0);
/// Its subnet: the broadcast address, with no mask.
pub const SUBNET: LocalSubnet =
    LocalSubnet::new(SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 255), 0xBAC0));
/// Its BBMD, on another subnet.
pub const BBMD: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 0xBAC0);
/// A peer on its subnet, which sends it the requests only a BBMD serves.
pub const PEER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 20), 0xBAC0);
/// A sender that isn't its BBMD.
pub const STRANGER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(172, 16, 0, 9), 0xBAC0);
/// Where the broadcast the BBMD forwards started.
pub const ORIGIN: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 77), 0xBAC0);

/// The registration, as the hosted transport sets one up (#1622): renewed
/// every half time-to-live, a 3-second response timeout, retries that back
/// off, broadcasts held, and Forwarded-NPDUs only from the BBMD.
pub const FOREIGN: Foreign = {
    let mut foreign = Foreign::new(BBMD, TTL);
    foreign.renewal = Duration::from_secs(30);
    foreign.response_timeout = Duration::from_secs(3);
    foreign.retry = Retry::Backoff;
    foreign.hold = HOLD;
    foreign.trust = ForwardedTrust::ConfiguredBbmd;
    foreign
};

/// The node's link, with a send budget of [`BUDGET`] octets in place of
/// [`Link::new`]'s, B/IP's largest datagram
/// ([`MAX_DATAGRAM`](bacnet_bip_core::node::MAX_DATAGRAM)), which the hosted
/// transport keeps. It is checked when the image is built, and again by
/// [`Node::new`].
pub const LINK: Link = match Link::new(OWN, SUBNET, Mode::Foreign(FOREIGN)) {
    Ok(mut link) => {
        link.budget = Some(BUDGET);
        link
    }
    Err(_) => panic!("the scenario's link is invalid"),
};

/// A global-broadcast Who-Is in an NPDU: what the node broadcasts, and what
/// the BBMD forwards it.
pub const WHO_IS: [u8; 8] = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];

/// The one row of the Write-BDT request.
const WRITTEN_BDT: [BdtEntry; 1] = [BdtEntry::new(PEER, Ipv4Addr::BROADCAST)];

/// The six requests only a BBMD serves, in function-code order, as the peer
/// sends them.
pub const REQUESTS: [Bvll<'static>; 6] = [
    Bvll::WriteBroadcastDistributionTable(BdtEntries::from_entries(&WRITTEN_BDT)),
    Bvll::ReadBroadcastDistributionTable,
    Bvll::RegisterForeignDevice { ttl: TTL },
    Bvll::ReadForeignDeviceTable,
    Bvll::DeleteForeignDeviceTableEntry(STRANGER),
    Bvll::DistributeBroadcastToNetwork(&WHO_IS),
];

/// Octets the scenario encodes into at once; its longest message, the
/// Forwarded-NPDU, takes 18.
const WIRE_LENGTH: usize = 64;

/// Storage the scenario works in besides its stack: a buffer for the
/// datagram received, which the port reads in place, one for the datagram
/// sent, which the socket encodes into, and the slots the port holds
/// broadcasts in, one for each the node holds ([`HOLD`]), each as long as
/// the link's broadcasts may be. The node and its port, a few hundred
/// octets, are built on the stack.
///
/// [`Memory::new`] is all zero bits, so the image keeps it in a static that
/// the linker places in `.bss`.
pub struct Memory {
    /// The datagram received.
    rx: [u8; WIRE_LENGTH],
    /// The datagram sent.
    tx: [u8; WIRE_LENGTH],
    /// The broadcasts held for the first registration.
    held: [HeldBroadcast<HELD_LENGTH>; HOLD as usize],
}

impl Memory {
    /// Empty storage.
    pub const fn new() -> Self {
        Self {
            rx: [0; WIRE_LENGTH],
            tx: [0; WIRE_LENGTH],
            held: [HeldBroadcast::EMPTY; HOLD as usize],
        }
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

/// Room for the registration attempts the scenario makes.
pub const ATTEMPTS: usize = 8;
/// Room for the leases the BBMD's successes prove.
pub const LEASES: usize = 6;
/// Room for the broadcasts it asks to send.
pub const ADMISSIONS: usize = 4;

/// What [`run`] saw. Every field is plain data, so the host test can
/// compare the whole summary at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// The longest NPDU the link sends as a unicast, then as a broadcast.
    pub max_npdu: [usize; 2],
    /// For each of [`REQUESTS`], where its NAK went and its code.
    pub naks: [Option<(SocketAddrV4, BvlcResultCode)>; REQUESTS.len()],
    /// What the port said to each broadcast the scenario asked to send.
    pub admissions: [Option<Result<Admission, SendRefusal>>; ADMISSIONS],
    /// Held broadcasts the port sent when the node released them.
    pub released: u8,
    /// The BBMD's Forwarded-NPDU: the source it was delivered from, and
    /// whether as a broadcast.
    pub forwarded: Option<(SocketAddrV4, bool)>,
    /// Why the other sender's Forwarded-NPDU was dropped.
    pub untrusted: Option<Discard>,
    /// When each registration attempt went out, in order.
    pub attempts: [Option<Instant>; ATTEMPTS],
    /// The lease, as its start and end, after each success the BBMD
    /// answered with, in order.
    pub leases: [Option<(Instant, Instant)>; LEASES],
    /// The registration at the end.
    pub registration: Option<RegistrationSnapshot>,
    /// Everything the node counted, taken once, at the end.
    pub counters: NodeCounters,
    /// Every datagram the port sent.
    pub sent: Sent,
    /// Datagrams of the port's own its socket refused.
    pub refused: u32,
    /// Calls that failed or answered otherwise than the scenario expects.
    /// Zero in a good run.
    pub errors: u8,
}

impl Summary {
    fn new() -> Self {
        Self {
            max_npdu: [0; 2],
            naks: [None; REQUESTS.len()],
            admissions: [None; ADMISSIONS],
            released: 0,
            forwarded: None,
            untrusted: None,
            attempts: [None; ATTEMPTS],
            leases: [None; LEASES],
            registration: None,
            counters: NodeCounters::default(),
            sent: Sent::new(),
            refused: 0,
            errors: 0,
        }
    }

    /// Count a call that failed, or answered otherwise than expected.
    fn error(&mut self) {
        self.errors = self.errors.saturating_add(1);
    }
}

/// What a good run's [`Summary`] holds, as [`expected()`] builds it.
///
/// The node's [`RegistrationSnapshot`] and the counters' newest
/// [`RegisterAttempt`] are `#[non_exhaustive]`, so `expected()` can't build
/// either: it gives their fields, and a summary is compared with it field by
/// field there and whole everywhere else. The comparison happens outside
/// [`run`], so the image links none of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expected {
    /// The summary, with no registration and no newest attempt in its
    /// counters.
    pub summary: Summary,
    /// The registration at the end.
    pub registration: Registration,
    /// The newest registration attempt the counters hold.
    pub last_attempt: Attempt,
}

impl PartialEq<Expected> for Summary {
    fn eq(&self, expected: &Expected) -> bool {
        let mut rest = *self;
        let registration = rest.registration.take().map(Registration::of);
        let last_attempt = rest
            .counters
            .last_registration_attempt
            .take()
            .map(Attempt::of);
        rest == expected.summary
            && registration == Some(expected.registration)
            && last_attempt == Some(expected.last_attempt)
    }
}

/// The node's [`RegistrationSnapshot`], field by field, for [`Expected`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Registration {
    /// The newest proven lease, as its start and end:
    /// [`Lease`](bacnet_bip_core::node::Lease) is
    /// `#[non_exhaustive]` too.
    pub lease: Option<(Instant, Instant)>,
    /// Whether that lease is live.
    pub registered: bool,
    /// Its time left, while live.
    pub lease_remaining: Option<Duration>,
    /// Whether an automatic attempt is waiting for its answer.
    pub attempt_in_flight: bool,
    /// When the next automatic attempt is due.
    pub next_attempt: Option<Instant>,
    /// What the newest automatic attempt came to.
    pub last_outcome: Option<AttemptOutcome>,
    /// Automatic attempts in a row that failed.
    pub consecutive_failures: u32,
    /// Whether the first automatic attempt has ended.
    pub first_resolved: bool,
    /// Broadcasts held for the first attempt.
    pub held: u16,
}

impl Registration {
    fn of(snapshot: RegistrationSnapshot) -> Self {
        Self {
            lease: snapshot.lease.map(|lease| (lease.since, lease.until)),
            registered: snapshot.registered,
            lease_remaining: snapshot.lease_remaining,
            attempt_in_flight: snapshot.attempt_in_flight,
            next_attempt: snapshot.next_attempt,
            last_outcome: snapshot.last_outcome,
            consecutive_failures: snapshot.consecutive_failures,
            first_resolved: snapshot.first_resolved,
            held: snapshot.held,
        }
    }
}

/// The node's newest [`RegisterAttempt`], field by field, for [`Expected`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attempt {
    /// The BBMD it went to.
    pub bbmd: SocketAddrV4,
    /// The time-to-live it asked for.
    pub ttl: u16,
    /// What it came to.
    pub outcome: AttemptOutcome,
    /// The result code that answered it.
    pub result: Option<BvlcResultCode>,
}

impl Attempt {
    fn of(attempt: RegisterAttempt) -> Self {
        Self {
            bbmd: attempt.bbmd,
            ttl: attempt.ttl,
            outcome: attempt.outcome,
            result: attempt.result,
        }
    }
}

/// The scenario's clock: `millis` milliseconds into its timeline.
const fn at(millis: u64) -> Instant {
    Instant::from_millis(millis)
}

/// How long the BBMD takes to answer.
const ROUND_TRIP: Duration = Duration::from_millis(10);

/// The success the BBMD answers a registration with.
const SUCCESS: BvlcResultCode = BvlcResultCode::SUCCESSFUL_COMPLETION;

/// The most deadlines the scenario waits through for the lease to lapse.
const MAX_WAITS: usize = 16;

/// Run the whole scenario in `memory`. The node and its port are built on
/// this function's stack.
///
/// It never panics on a failed call: it counts the failure in
/// [`Summary::errors`] and carries on, or returns early when it can't.
pub fn run(memory: &mut Memory) -> Summary {
    let Memory { rx, tx, held } = memory;
    let mut summary = Summary::new();
    summary.max_npdu = [LINK.max_npdu(Cast::Unicast), LINK.max_npdu(Cast::Broadcast)];
    let Ok(node) = ForeignNode::new(LINK, at(0)) else {
        summary.error();
        return summary;
    };
    let Ok(mut port) = ForeignPort::new(node, held) else {
        summary.error();
        return summary;
    };
    Device {
        port: &mut port,
        rx,
        socket: Socket {
            tx,
            now: at(0),
            result: None,
            summary: &mut summary,
        },
    }
    .run();
    summary
}

/// What became of a received datagram, without its borrow of the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
    /// The port sent this NAK.
    Nak(SocketAddrV4, BvlcResultCode),
    /// An NPDU delivered, from this source, as a broadcast or not.
    Delivered(SocketAddrV4, bool),
    /// Dropped.
    Discarded(Discard),
    /// Handled at the port, with no answer: a response the node took.
    Handled,
}

/// The node's socket: it encodes each datagram into `tx`, keeps the
/// summary's tally of it, and notes what it was.
struct Socket<'m> {
    tx: &'m mut [u8],
    /// The scenario's clock, for the attempts it notes.
    now: Instant,
    /// The NAK the port sent during the call, if any.
    result: Option<(SocketAddrV4, BvlcResultCode)>,
    summary: &'m mut Summary,
}

impl Datagrams for Socket<'_> {
    fn send(&mut self, to: SocketAddrV4, message: &Bvll<'_>) -> Result<(), SendRefusal> {
        let summary = &mut *self.summary;
        if summary.sent.send(self.tx, to, message).is_none() {
            summary.error();
            return Err(SendRefusal::SocketError);
        }
        match *message {
            Bvll::RegisterForeignDevice { ttl: TTL } if to == BBMD => {
                record(&mut summary.attempts, self.now, &mut summary.errors);
            }
            Bvll::BvlcResult(code) => self.result = Some((to, code)),
            // Every broadcast is the one Who-Is, to the BBMD.
            Bvll::DistributeBroadcastToNetwork(npdu) if npdu == WHO_IS && to == BBMD => {}
            _ => summary.error(),
        }
        Ok(())
    }
}

/// The device around the port: it plays the network, and keeps the time.
struct Device<'d, 'm> {
    port: &'d mut ForeignPort<'m>,
    rx: &'d mut [u8],
    socket: Socket<'d>,
}

impl Device<'_, '_> {
    fn run(&mut self) {
        // The first registration attempt is due at once.
        self.port.start(self.socket.now, &mut self.socket);

        // The NAK matrix, from a peer on the subnet.
        self.at(1);
        for (index, request) in REQUESTS.iter().enumerate() {
            let nak = match self.receive(request, PEER) {
                Seen::Nak(to, code) => Some((to, code)),
                _ => None,
            };
            if let Some(slot) = self.socket.summary.naks.get_mut(index) {
                *slot = nak;
            }
        }

        // Three broadcasts before the BBMD answers: two held, one refused.
        for millis in [2, 3, 4] {
            self.at(millis);
            self.broadcast();
        }
        // The BBMD's success registers the node and releases the two.
        self.at(10);
        let before = self.socket.summary.sent.count;
        self.success();
        let released = self.socket.summary.sent.count.saturating_sub(before);
        self.socket.summary.released = released;

        // A forwarded broadcast from the BBMD, and one from a stranger.
        self.at(20);
        let forwarded = Bvll::ForwardedNpdu {
            origin: ORIGIN,
            npdu: &WHO_IS,
        };
        match self.receive(&forwarded, BBMD) {
            Seen::Delivered(source, group) => {
                self.socket.summary.forwarded = Some((source, group));
            }
            _ => self.socket.summary.error(),
        }
        match self.receive(&forwarded, STRANGER) {
            Seen::Discarded(reason) => self.socket.summary.untrusted = Some(reason),
            _ => self.socket.summary.error(),
        }

        // The renewal, one interval into the lease.
        self.wait();
        self.answer();

        // A broadcast the BBMD refuses with the DBTN NAK: the renewal comes
        // at once.
        self.at(39_990);
        self.broadcast();
        self.at(40_000);
        let code = BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK;
        if self.receive(&Bvll::BvlcResult(code), BBMD) != Seen::Handled {
            self.socket.summary.error();
        }
        self.answer();

        // The BBMD stops answering: the attempts back off until the lease
        // lapses, as a new attempt starts.
        for _ in 0..MAX_WAITS {
            self.wait();
            let lapsed = self
                .port
                .node()
                .registration(self.socket.now)
                .is_some_and(|snapshot| !snapshot.registered);
            if lapsed {
                break;
            }
        }
        // It answers again: registered once more, and renewed at once, since
        // that lease started at an attempt it left unanswered.
        self.answer();
        self.answer();

        let now = self.socket.now;
        let registration = self.port.node().registration(now);
        let counters = self.port.node_mut().take_counters();
        let refused = self.port.take_counters().refused;
        let summary = &mut *self.socket.summary;
        summary.registration = registration;
        summary.counters = counters;
        summary.refused = refused;
    }

    /// Let the clock run on to `millis` milliseconds into the timeline.
    fn at(&mut self, millis: u64) {
        self.socket.now = self.socket.now.max(at(millis));
    }

    /// The BBMD answers the attempt in flight with success one round trip
    /// later.
    fn answer(&mut self) {
        self.socket.now += ROUND_TRIP;
        self.success();
    }

    /// The BBMD's success arrives now and settles the attempt in flight:
    /// note the lease it proves.
    fn success(&mut self) {
        if self.receive(&Bvll::BvlcResult(SUCCESS), BBMD) != Seen::Handled {
            self.socket.summary.error();
        }
        let now = self.socket.now;
        match self
            .port
            .node()
            .registration(now)
            .and_then(|view| view.lease)
        {
            Some(lease) => {
                let summary = &mut *self.socket.summary;
                record(
                    &mut summary.leases,
                    (lease.since, lease.until),
                    &mut summary.errors,
                );
            }
            None => self.socket.summary.error(),
        }
    }

    /// Hand the port `message` from `source`, sent to the node's own
    /// address.
    fn receive(&mut self, message: &Bvll<'_>, source: SocketAddrV4) -> Seen {
        let Ok(length) = message.encode_into(self.rx) else {
            self.socket.summary.error();
            return Seen::Handled;
        };
        let Some(payload) = self.rx.get(..length) else {
            self.socket.summary.error();
            return Seen::Handled;
        };
        let to = Addressed {
            node: true,
            broadcast: false,
        };
        let datagram = Datagram::new(payload, source, Some(to));
        self.socket.result = None;
        let now = self.socket.now;
        match self
            .port
            .receive(&datagram, now, &mut Permissive, &mut self.socket)
        {
            Ingress::Npdu(delivery) => {
                if delivery.npdu != WHO_IS {
                    self.socket.summary.error();
                }
                Seen::Delivered(delivery.source, delivery.group)
            }
            Ingress::Discarded(reason) => Seen::Discarded(reason),
            Ingress::Handled => match self.socket.result {
                Some((to, code)) => Seen::Nak(to, code),
                None => Seen::Handled,
            },
            _ => {
                self.socket.summary.error();
                Seen::Handled
            }
        }
    }

    /// Ask the port to broadcast a Who-Is now: sent, held, or refused.
    fn broadcast(&mut self) {
        let now = self.socket.now;
        let admission = self.port.broadcast(&WHO_IS, now, &mut self.socket);
        let summary = &mut *self.socket.summary;
        record(&mut summary.admissions, admission, &mut summary.errors);
    }

    /// Let time run to the node's next deadline, and do what falls due.
    fn wait(&mut self) {
        let Some(deadline) = self.port.next_deadline() else {
            self.socket.summary.error();
            return;
        };
        self.socket.now = self.socket.now.max(deadline);
        let now = self.socket.now;
        self.port.on_deadline(now, &mut self.socket);
    }
}

/// Put `value` in the first free slot of `slots`, or count an error when
/// none is free.
fn record<T: Copy>(slots: &mut [Option<T>], value: T, errors: &mut u8) {
    match slots.iter_mut().find(|slot| slot.is_none()) {
        Some(slot) => *slot = Some(value),
        None => *errors = errors.saturating_add(1),
    }
}
