//! The node image's scenario (#1667).
//!
//! [`run`] drives a foreign device, a [`Node`] with two exchange slots from
//! `bacnet-bip-core`, set up as the hosted B/IP transport sets one up
//! ([`LINK`]): failed attempts retried sooner, backing off; broadcasts held
//! until the first registration attempt resolves (two here, where the
//! transport holds 16); Forwarded-NPDUs taken only from its own BBMD, by
//! unicast; and a send budget that carries B/IP's largest NPDU, 1497
//! octets. Every datagram goes through `bacnet-bip-codec`, and time is a
//! fixed timeline of `bacnet-time` instants, which the bip-core re-exports.
//! It returns a [`Summary`] of what it saw.
//!
//! - **The NAK matrix:** a peer sends each of the six requests only a BBMD
//!   serves ([`REQUESTS`]); each draws its own NAK, which the node sends
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

use bacnet_bip_codec::{BdtEntries, BdtEntry, BvlcResultCode, Bvll};
use bacnet_bip_core::node::{
    Action, Addressed, Admission, AttemptOutcome, Cast, Datagram, Discard, Foreign, ForwardedTrust,
    Inbound, Link, Mode, Node, NodeCounters, Permissive, RegisterAttempt, RegistrationEvent,
    RegistrationSnapshot, Retry, SendRefusal,
};
use bacnet_bip_core::{Duration, Instant, LocalSubnet};

use crate::tally::Sent;

/// Exchange slots: one for the registration, one for a manual request.
pub const EXCHANGES: usize = 2;

/// The node as the image builds it.
pub type ForeignNode = Node<EXCHANGES>;

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

/// The node's link, with [`Link::new`]'s send budget, B/IP's largest
/// datagram ([`MAX_DATAGRAM`](bacnet_bip_core::node::MAX_DATAGRAM)), so
/// NPDUs of up to 1497 octets go out, unicast or broadcast. It is checked
/// when the image is built.
pub const LINK: Link = match Link::new(OWN, SUBNET, Mode::Foreign(FOREIGN)) {
    Ok(link) => link,
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

/// Storage the scenario works in besides its stack: one buffer for the
/// datagram on its way in or out. The node itself, a few hundred octets, is
/// built on the stack.
///
/// The image keeps it in a static, so the linker counts it in `.bss`.
pub struct Memory {
    /// The datagram being received or sent.
    wire: [u8; WIRE_LENGTH],
}

impl Memory {
    /// Empty storage.
    pub const fn new() -> Self {
        Self {
            wire: [0; WIRE_LENGTH],
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
/// Room for the registration events it sees.
pub const EVENTS: usize = 10;
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
    /// What the node said to each broadcast the scenario asked to send.
    pub admissions: [Option<Result<Admission, SendRefusal>>; ADMISSIONS],
    /// Held broadcasts sent when the node released them.
    pub released: u8,
    /// The BBMD's Forwarded-NPDU: the source it was delivered from, and
    /// whether as a broadcast.
    pub forwarded: Option<(SocketAddrV4, bool)>,
    /// Why the other sender's Forwarded-NPDU was dropped.
    pub untrusted: Option<Discard>,
    /// What the node made of the Distribute-Broadcast-To-Network NAK: the
    /// code of a result that answers none of its exchanges, and whether it
    /// came just after one ended.
    pub hint: Option<(Option<BvlcResultCode>, bool)>,
    /// When each registration attempt started, in order.
    pub attempts: [Option<Instant>; ATTEMPTS],
    /// Each registration event, in order.
    pub events: [Option<RegistrationEvent>; EVENTS],
    /// The registration at the end.
    pub registration: Option<RegistrationSnapshot>,
    /// Everything the node counted, taken once, at the end.
    pub counters: NodeCounters,
    /// Every datagram the node sent.
    pub sent: Sent,
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
            hint: None,
            attempts: [None; ATTEMPTS],
            events: [None; EVENTS],
            registration: None,
            counters: NodeCounters::default(),
            sent: Sent::new(),
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

/// The most actions one call may queue before the scenario counts an error
/// and moves on, so a runaway node can't hang it.
const MAX_ACTIONS: usize = 8;

/// The most deadlines the scenario waits through for the lease to lapse.
const MAX_WAITS: usize = 16;

/// Run the whole scenario in `memory`. The node is built on this function's
/// stack.
///
/// It never panics on a failed call: it counts the failure in
/// [`Summary::errors`] and carries on, or returns early when it can't.
pub fn run(memory: &mut Memory) -> Summary {
    let mut summary = Summary::new();
    summary.max_npdu = [LINK.max_npdu(Cast::Unicast), LINK.max_npdu(Cast::Broadcast)];
    match ForeignNode::new(LINK, at(0)) {
        Ok(mut node) => Device {
            node: &mut node,
            wire: &mut memory.wire,
            now: at(0),
            held: 0,
            summary: &mut summary,
        }
        .run(),
        Err(_) => summary.error(),
    }
    summary
}

/// What became of a received datagram, without its borrow of the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
    /// A NAK to send.
    Nak(SocketAddrV4, BvlcResultCode),
    /// An NPDU delivered, from this source, as a broadcast or not.
    Delivered(SocketAddrV4, bool),
    /// Dropped.
    Discarded(Discard),
    /// The answer to an exchange, which it ended.
    Settled,
    /// A result that answers no exchange: its code, and whether it came
    /// just after one ended.
    Unmatched(Option<BvlcResultCode>, bool),
    /// Anything else.
    Other,
}

/// The device around the node: it sends what the node asks, keeps the held
/// broadcasts, and keeps the time.
struct Device<'m> {
    node: &'m mut ForeignNode,
    wire: &'m mut [u8],
    now: Instant,
    /// Broadcasts the device keeps until the node releases them.
    held: u8,
    summary: &'m mut Summary,
}

impl Device<'_> {
    fn run(&mut self) {
        // The first registration attempt is due at once.
        self.drain();

        // The NAK matrix, from a peer on the subnet.
        self.now = at(1);
        for (index, request) in REQUESTS.iter().enumerate() {
            let nak = match self.receive(request, PEER) {
                Seen::Nak(to, code) => {
                    self.send(to, &Bvll::BvlcResult(code));
                    Some((to, code))
                }
                _ => None,
            };
            if let Some(slot) = self.summary.naks.get_mut(index) {
                *slot = nak;
            }
        }

        // Three broadcasts before the BBMD answers: two held, one refused.
        for millis in [2, 3, 4] {
            self.now = at(millis);
            self.broadcast();
        }
        // The BBMD's success registers the node and releases the two.
        self.now = at(10);
        self.expect(SUCCESS, Seen::Settled);

        // A forwarded broadcast from the BBMD, and one from a stranger.
        self.now = at(20);
        let forwarded = Bvll::ForwardedNpdu {
            origin: ORIGIN,
            npdu: &WHO_IS,
        };
        match self.receive(&forwarded, BBMD) {
            Seen::Delivered(source, group) => self.summary.forwarded = Some((source, group)),
            _ => self.summary.error(),
        }
        match self.receive(&forwarded, STRANGER) {
            Seen::Discarded(reason) => self.summary.untrusted = Some(reason),
            _ => self.summary.error(),
        }

        // The renewal, one interval into the lease.
        self.wait();
        self.answer(SUCCESS);

        // A broadcast the BBMD refuses with the DBTN NAK: the renewal comes
        // at once.
        self.now = at(39_990);
        self.broadcast();
        self.now = at(40_000);
        let code = BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK;
        match self.receive(&Bvll::BvlcResult(code), BBMD) {
            Seen::Unmatched(code, late) => self.summary.hint = Some((code, late)),
            _ => self.summary.error(),
        }
        self.answer(SUCCESS);

        // The BBMD stops answering: the attempts back off until the lease
        // lapses, as a new attempt starts.
        for _ in 0..MAX_WAITS {
            self.wait();
            if self
                .summary
                .events
                .contains(&Some(RegistrationEvent::Lapsed))
            {
                break;
            }
        }
        // It answers again: registered once more, and renewed at once, since
        // that lease started at an attempt it left unanswered.
        self.answer(SUCCESS);
        self.answer(SUCCESS);

        self.summary.registration = self.node.registration(self.now);
        self.summary.counters = self.node.take_counters();
    }

    /// The BBMD answers with `code` one round trip later, settling the
    /// attempt in flight.
    fn answer(&mut self, code: BvlcResultCode) {
        self.now += ROUND_TRIP;
        self.expect(code, Seen::Settled);
    }

    /// The BBMD sends a result with `code` now, which the node should take
    /// as `seen`.
    fn expect(&mut self, code: BvlcResultCode, seen: Seen) {
        if self.receive(&Bvll::BvlcResult(code), BBMD) != seen {
            self.summary.error();
        }
    }

    /// Hand the node `message` from `source`, sent to its own address, and
    /// carry out what it asks.
    fn receive(&mut self, message: &Bvll<'_>, source: SocketAddrV4) -> Seen {
        let Ok(length) = message.encode_into(self.wire) else {
            self.summary.error();
            return Seen::Other;
        };
        let Some(payload) = self.wire.get(..length) else {
            self.summary.error();
            return Seen::Other;
        };
        let to = Addressed {
            node: true,
            broadcast: false,
        };
        let datagram = Datagram::new(payload, source, Some(to));
        let seen = match self.node.receive(&datagram, self.now, &mut Permissive) {
            Inbound::Nak {
                to,
                code,
                malformed: false,
            } => Seen::Nak(to, code),
            Inbound::Deliver(delivery) => {
                if delivery.npdu != WHO_IS {
                    self.summary.error();
                }
                Seen::Delivered(delivery.source, delivery.group)
            }
            Inbound::Discarded(reason) => Seen::Discarded(reason),
            Inbound::Matched {
                accepted: true,
                settled: true,
                ..
            } => Seen::Settled,
            Inbound::Unmatched { code, late, .. } => Seen::Unmatched(code, late),
            _ => Seen::Other,
        };
        self.drain();
        seen
    }

    /// Ask to broadcast a Who-Is now: send it, hold it, or drop it, as the
    /// node says.
    fn broadcast(&mut self) {
        let admission = self.node.admit_broadcast(self.now);
        match self
            .summary
            .admissions
            .iter_mut()
            .find(|slot| slot.is_none())
        {
            Some(slot) => *slot = Some(admission),
            None => self.summary.error(),
        }
        match admission {
            Ok(Admission::Send) => self.send_broadcast(),
            Ok(Admission::Hold) => self.held = self.held.saturating_add(1),
            Err(_) => {}
        }
        self.drain();
    }

    /// Send a Who-Is as the link broadcasts it: to the BBMD, as a
    /// Distribute-Broadcast-To-Network.
    fn send_broadcast(&mut self) {
        match self.node.link().broadcast(&WHO_IS) {
            Ok((outgoing, _)) => self.send(outgoing.to, &outgoing.message),
            Err(_) => self.summary.error(),
        }
    }

    /// Encode `message` and send it to `to`.
    fn send(&mut self, to: SocketAddrV4, message: &Bvll<'_>) {
        if self.summary.sent.send(self.wire, to, message).is_none() {
            self.summary.error();
        }
    }

    /// Let time run to the node's next deadline, and do what falls due.
    fn wait(&mut self) {
        let Some(deadline) = self.node.next_deadline() else {
            self.summary.error();
            return;
        };
        self.now = self.now.max(deadline);
        self.node.on_deadline(self.now);
        self.drain();
    }

    /// Poll the node's actions until it has none, carrying out each one.
    fn drain(&mut self) {
        for _ in 0..MAX_ACTIONS {
            match self.node.poll_action() {
                Action::Idle => return,
                action @ Action::Register { exchange, to, .. } => {
                    record(
                        &mut self.summary.attempts,
                        self.now,
                        &mut self.summary.errors,
                    );
                    match action.message() {
                        Some(message) => self.send(to, &message),
                        None => self.summary.error(),
                    }
                    // Nothing has answered it yet, so nothing settles.
                    if self.node.exchange_sent(exchange, self.now) {
                        self.summary.error();
                    }
                }
                Action::Registration(event) => {
                    record(&mut self.summary.events, event, &mut self.summary.errors);
                }
                Action::ReleaseHeld => {
                    for _ in 0..self.held {
                        self.send_broadcast();
                        self.summary.released = self.summary.released.saturating_add(1);
                    }
                    self.held = 0;
                }
                Action::DiscardHeld | Action::ExchangeEnded { .. } => self.summary.error(),
            }
        }
        self.summary.error();
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
