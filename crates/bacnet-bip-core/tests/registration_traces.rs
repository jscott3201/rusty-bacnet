//! A foreign device's registration in virtual time: every action and event,
//! with its instant, for scripted BBMD answers (Annex J.5.2).
//!
//! The settings are the defaults for a 60 s time-to-live: renewal 30 s,
//! response timeout 3 s, backoff. A BBMD's result comes back 10 ms after a
//! request that reaches it, unless the script says otherwise.

mod node_support;

use bacnet_bip_codec::BvlcResultCode;
use bacnet_bip_core::node::{
    Action, Admission, Foreign, Inbound, Node, Permissive, RegistrationEvent, Retry, SendRefusal,
};
use bacnet_bip_core::{Duration, Instant};
use node_support::*;

const NAK: BvlcResultCode = BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
const DBTN_NAK: BvlcResultCode = BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK;

/// What becomes of one registration attempt.
#[derive(Debug, Clone, Copy)]
enum Fate {
    /// Answered with this code, each copy after its delay from the send.
    Answer(BvlcResultCode, &'static [u64]),
    /// Lost.
    Lost,
}

/// Answered with success after 10 ms.
const OK: Fate = Fate::Answer(BvlcResultCode::SUCCESSFUL_COMPLETION, &[10]);

/// One line of a trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    /// Registration attempt `n` (from 1) sent.
    Sent(u32),
    /// A registration event.
    Event(RegistrationEvent),
    /// The held broadcasts released, or discarded.
    Held { released: bool },
    /// A broadcast admitted, or refused.
    Broadcast(Result<Admission, SendRefusal>),
    /// A result that matched nothing.
    Unmatched { code: BvlcResultCode, late: bool },
}

use Entry::*;

fn registered(since: u64, until: u64) -> Entry {
    Event(RegistrationEvent::Registered {
        since: at(since),
        until: at(until),
    })
}

fn renewed(until: u64) -> Entry {
    Event(RegistrationEvent::Renewed { until: at(until) })
}

const UNANSWERED: Entry = Event(RegistrationEvent::Unanswered);
const LAPSED: Entry = Event(RegistrationEvent::Lapsed);

/// A foreign device and its scripted BBMD.
struct Trace {
    node: Node,
    fates: Vec<Fate>,
    /// Results on their way to the node: arrival and code.
    in_flight: Vec<(Instant, BvlcResultCode)>,
    /// Broadcasts the device sends, and what the BBMD answers each with.
    broadcasts: Vec<(Instant, Option<BvlcResultCode>)>,
    attempts: u32,
    log: Vec<(u64, Entry)>,
}

impl Trace {
    fn new(foreign: Foreign, fates: &[Fate]) -> Self {
        let mut trace = Self {
            node: Node::new(foreign_link(foreign), Instant::ZERO).unwrap(),
            fates: fates.to_vec(),
            in_flight: Vec::new(),
            broadcasts: Vec::new(),
            attempts: 0,
            log: Vec::new(),
        };
        trace.act(Instant::ZERO);
        trace
    }

    fn broadcast_at(mut self, millis: u64, reply: Option<BvlcResultCode>) -> Self {
        self.broadcasts.push((at(millis), reply));
        self
    }

    fn note(&mut self, now: Instant, entry: Entry) {
        self.log.push((now.as_nanos() / 1_000_000, entry));
    }

    /// Carry out the node's actions at `now`.
    fn act(&mut self, now: Instant) {
        for action in drain(&mut self.node) {
            match action {
                Action::Register { exchange, .. } => {
                    self.attempts += 1;
                    self.note(now, Sent(self.attempts));
                    assert!(!self.node.exchange_sent(exchange, now));
                    let fate = self
                        .fates
                        .get(self.attempts as usize - 1)
                        .copied()
                        .unwrap_or(OK);
                    if let Fate::Answer(code, delays) = fate {
                        for delay in delays {
                            self.in_flight.push((now + ms(*delay), code));
                        }
                    }
                }
                Action::Registration(event) => self.note(now, Event(event)),
                Action::ReleaseHeld => self.note(now, Held { released: true }),
                Action::DiscardHeld => self.note(now, Held { released: false }),
                Action::ExchangeEnded { .. } | Action::Idle => unreachable!(),
            }
        }
    }

    /// Run until `end`: results first at an instant, as a host's receive
    /// loop serves datagrams before its timer, then broadcasts, then
    /// deadlines.
    fn run(mut self, end: u64) -> Vec<(u64, Entry)> {
        let end = at(end);
        loop {
            let arrival = self.in_flight.iter().map(|(when, _)| *when).min();
            let broadcast = self.broadcasts.iter().map(|(when, _)| *when).min();
            let Some(now) = [arrival, broadcast, self.node.next_deadline()]
                .into_iter()
                .flatten()
                .min()
                .filter(|now| *now <= end)
            else {
                return self.log;
            };
            if arrival == Some(now) {
                let index = self
                    .in_flight
                    .iter()
                    .position(|(when, _)| *when == now)
                    .unwrap();
                let (_, code) = self.in_flight.remove(index);
                let payload = result(code);
                let inbound =
                    self.node
                        .receive(&datagram(&payload, BBMD, UNICAST), now, &mut Permissive);
                if let Inbound::Unmatched { late, .. } = inbound {
                    self.note(now, Unmatched { code, late });
                }
            } else if broadcast == Some(now) {
                let index = self
                    .broadcasts
                    .iter()
                    .position(|(when, _)| *when == now)
                    .unwrap();
                let (_, reply) = self.broadcasts.remove(index);
                let admission = self.node.admit_broadcast(now);
                self.note(now, Broadcast(admission));
                if let (Ok(Admission::Send), Some(code)) = (admission, reply) {
                    self.in_flight.push((now + ms(10), code));
                }
            } else {
                self.node.on_deadline(now);
            }
            self.act(now);
        }
    }
}

/// The loss pattern of the main trace: attempts 3 and 5 to 8 are lost.
const LOSSY: [Fate; 11] = [
    OK,
    OK,
    Fate::Lost,
    OK,
    Fate::Lost,
    Fate::Lost,
    Fate::Lost,
    Fate::Lost,
    OK,
    OK,
    OK,
];

#[test]
fn register_renew_lose_expire_and_register_again() {
    let log = Trace::new(foreign(), &LOSSY).run(180_000);
    assert_eq!(
        log,
        [
            // Register.
            (0, Sent(1)),
            (10, registered(0, 60_000)),
            // Renew.
            (30_000, Sent(2)),
            (30_010, renewed(90_000)),
            // Renew under loss: the lease starts at the unanswered send.
            (60_000, Sent(3)),
            (63_000, UNANSWERED),
            (66_000, Sent(4)),
            (66_010, renewed(120_000)),
            // Expire: three more losses, backing off 3, 6 and 12 s.
            (90_000, Sent(5)),
            (93_000, UNANSWERED),
            (96_000, Sent(6)),
            (99_000, UNANSWERED),
            (105_000, Sent(7)),
            (108_000, UNANSWERED),
            (120_000, LAPSED),
            (120_000, Sent(8)),
            (123_000, UNANSWERED),
            // Register again: the lease starts at attempt 5, the earliest
            // unanswered, so it ends at 150 s; the renewal due at 120 s goes
            // at once. Its success could be a copy of attempt 9's, so that
            // lease starts at attempt 9's send.
            (147_000, Sent(9)),
            (147_010, registered(90_000, 150_000)),
            (147_010, Sent(10)),
            (147_020, renewed(207_000)),
            (177_000, Sent(11)),
            (177_010, renewed(237_000)),
        ]
    );
}

#[test]
fn at_renewal_keeps_todays_schedule_under_the_same_loss() {
    let foreign = with(foreign(), |foreign| foreign.retry = Retry::AtRenewal);
    let log = Trace::new(foreign, &LOSSY).run(300_010);
    assert_eq!(
        log,
        [
            (0, Sent(1)),
            (10, registered(0, 60_000)),
            (30_000, Sent(2)),
            (30_010, renewed(90_000)),
            (60_000, Sent(3)),
            (63_000, UNANSWERED),
            // Today's schedule leans on the BBMD's grace period: the lease
            // from attempt 2 ends as attempt 4 goes.
            (90_000, LAPSED),
            (90_000, Sent(4)),
            (90_010, registered(60_000, 120_000)),
            (120_000, LAPSED),
            (120_000, Sent(5)),
            (123_000, UNANSWERED),
            (150_000, Sent(6)),
            (153_000, UNANSWERED),
            (180_000, Sent(7)),
            (183_000, UNANSWERED),
            (210_000, Sent(8)),
            (213_000, UNANSWERED),
            // Accepted, but the lease it proves (from attempt 5's send)
            // already ended: nothing to report until the next one.
            (240_000, Sent(9)),
            (270_000, Sent(10)),
            (270_010, registered(270_000, 330_000)),
            (300_000, Sent(11)),
            (300_010, renewed(360_000)),
        ]
    );
}

#[test]
fn a_refusal_at_the_first_attempt_discards_what_was_held() {
    let holding = with(foreign(), |foreign| foreign.hold = 2);
    let fates = [Fate::Answer(NAK, &[50])];
    let log = Trace::new(holding, &fates)
        .broadcast_at(1, None)
        .broadcast_at(2, None)
        .broadcast_at(3, None)
        .broadcast_at(30_020, None)
        .run(30_020);
    assert_eq!(
        log,
        [
            (0, Sent(1)),
            (1, Broadcast(Ok(Admission::Hold))),
            (2, Broadcast(Ok(Admission::Hold))),
            (3, Broadcast(Err(SendRefusal::HoldFull))),
            (50, Event(RegistrationEvent::Refused { code: NAK })),
            (50, Held { released: false }),
            (30_000, Sent(2)),
            (30_010, registered(30_000, 90_000)),
            (30_020, Broadcast(Ok(Admission::Send))),
        ]
    );
}

#[test]
fn a_distribute_nak_after_a_bbmd_restart_registers_again_at_once() {
    // The BBMD restarts at 11 s and forgets the device; the device's next
    // broadcast draws the Distribute-Broadcast-To-Network NAK.
    let log = Trace::new(foreign(), &[])
        .broadcast_at(5_000, None)
        .broadcast_at(12_000, Some(DBTN_NAK))
        .run(45_000);
    assert_eq!(
        log,
        [
            (0, Sent(1)),
            (10, registered(0, 60_000)),
            (5_000, Broadcast(Ok(Admission::Send))),
            (12_000, Broadcast(Ok(Admission::Send))),
            (
                12_010,
                Unmatched {
                    code: DBTN_NAK,
                    late: false
                }
            ),
            (12_010, Sent(2)),
            (12_020, renewed(72_010)),
            (42_010, Sent(3)),
            (42_020, renewed(102_010)),
        ]
    );
}

#[test]
fn a_late_result_with_nothing_live_is_counted_late() {
    let fates = [Fate::Answer(SUCCESS, &[4_000])];
    let log = Trace::new(foreign(), &fates).run(6_010);
    assert_eq!(
        log,
        [
            (0, Sent(1)),
            (3_000, UNANSWERED),
            (
                4_000,
                Unmatched {
                    code: SUCCESS,
                    late: true
                }
            ),
            (6_000, Sent(2)),
            // Attempt 1 went unanswered, so the lease starts at its send.
            (6_010, registered(0, 60_000)),
        ]
    );
}

#[test]
fn a_late_result_goes_to_the_newer_attempt_and_anchors_at_the_older() {
    let fates = [Fate::Answer(SUCCESS, &[6_005]), OK];
    let log = Trace::new(foreign(), &fates).run(10_000);
    assert_eq!(
        log,
        [
            (0, Sent(1)),
            (3_000, UNANSWERED),
            (6_000, Sent(2)),
            // Attempt 1's result settles attempt 2, anchored at attempt 1.
            (6_005, registered(0, 60_000)),
            // Attempt 2's own result is then late.
            (
                6_010,
                Unmatched {
                    code: SUCCESS,
                    late: true
                }
            ),
        ]
    );
}

#[test]
fn a_duplicate_result_changes_nothing() {
    let fates = [Fate::Answer(SUCCESS, &[10, 15])];
    let log = Trace::new(foreign(), &fates).run(20_000);
    assert_eq!(
        log,
        [
            (0, Sent(1)),
            (10, registered(0, 60_000)),
            (
                15,
                Unmatched {
                    code: SUCCESS,
                    late: true
                }
            ),
        ]
    );
}

#[test]
fn every_attempt_is_bounded_by_the_renewal_schedule() {
    // All lost, for ten minutes: no attempt comes later than one renewal
    // after the one before, nor sooner than one response timeout.
    let fates = [Fate::Lost; 64];
    let log = Trace::new(foreign(), &fates).run(600_000);
    let starts: Vec<u64> = log
        .iter()
        .filter_map(|(when, entry)| matches!(entry, Sent(_)).then_some(*when))
        .collect();
    assert!(starts.len() > 20);
    for pair in starts.windows(2) {
        let gap = Duration::from_millis(pair[1] - pair[0]);
        assert!(
            gap >= Duration::from_secs(3) && gap <= Duration::from_secs(30),
            "{pair:?}"
        );
    }
    assert!(!log
        .iter()
        .any(|(_, entry)| matches!(entry, Event(RegistrationEvent::Registered { .. }))));
}
