//! A foreign device against a BBMD made of `bbmd::Fdt`, over a network that
//! loses, delays and duplicates: the lease the node reports never outlasts
//! the BBMD's entry without its grace period, and attempts stay between one
//! response timeout and one renewal interval apart (Annex J.5.2).
//!
//! Time-to-live 2, 20 and 60 s. Every copy of a result arrives less than
//! the answer window after its request's send: three response timeouts, at
//! most the renewal interval, so one timeout (1 s) at a 2 s time-to-live
//! and three (9 s) at 20 and 60 s.

mod node_support;

use std::collections::BTreeMap;

use bacnet_bip_codec::BvlcResultCode;
use bacnet_bip_core::bbmd::{Fdt, RegistrationLimits, Reservation};
use bacnet_bip_core::node::{Action, Foreign, Node, Permissive, Retry};
use bacnet_bip_core::{Duration, Instant};
use node_support::*;
use proptest::prelude::*;

/// How long after its request's send a result may still arrive, as the
/// engine counts on: three response timeouts, at most the renewal.
fn answer_window(foreign: &Foreign) -> Duration {
    let timeout = foreign.effective_response_timeout();
    (timeout + timeout + timeout).min(foreign.effective_renewal())
}

/// `share` per mille of `window`, rounded down.
fn part(window: Duration, share: u64) -> Duration {
    Duration::from_nanos(window.as_nanos() / 1_000 * share)
}

const LIMITS: RegistrationLimits = {
    let mut limits = RegistrationLimits::DEFAULT;
    limits.min_ttl = 1;
    limits.max_ttl = u16::MAX;
    limits.max_entries_per_source = 4;
    limits.reserved_capacity = 0;
    limits
};

/// What the network and the BBMD do with one attempt.
#[derive(Debug, Clone)]
struct Plan {
    /// The request never arrives.
    request_lost: bool,
    /// How much of the answer window the request takes, per mille.
    request_share: u64,
    /// The BBMD refuses it (table full, say).
    refused: bool,
    /// Each copy of the result: lost, or its share of what is left, per
    /// mille. Two copies are a duplicate. Below 1000, so every copy
    /// arrives inside the window.
    copies: Vec<(bool, u64)>,
}

fn plan() -> impl Strategy<Value = Plan> {
    (
        prop::bool::weighted(0.3),
        0u64..1_000,
        prop::bool::weighted(0.1),
        prop::collection::vec((prop::bool::weighted(0.25), 0u64..1_000), 1..=2),
    )
        .prop_map(|(request_lost, request_share, refused, copies)| Plan {
            request_lost,
            request_share,
            refused,
            copies,
        })
}

/// Something on its way.
#[derive(Debug, Clone, Copy)]
enum Message {
    /// A Register-Foreign-Device reaches the BBMD; the plan's index.
    Request(usize),
    /// A BVLC-Result reaches the node.
    Result(BvlcResultCode),
}

struct World {
    node: Node,
    fdt: Fdt<2>,
    foreign: Foreign,
    window: Duration,
    plans: Vec<Plan>,
    /// Messages by arrival time, then order sent.
    in_flight: BTreeMap<(Instant, u64), Message>,
    sent: u64,
    starts: Vec<Instant>,
    /// Successes counted since the last attempt started.
    accepted_since_start: u32,
    /// Whether the attempt before the newest was accepted, per start.
    accepted_before: Vec<bool>,
}

impl World {
    fn new(ttl: u16, retry: Retry, plans: Vec<Plan>) -> Self {
        let mut foreign = Foreign::new(BBMD, ttl);
        foreign.retry = retry;
        let mut world = Self {
            node: Node::new(foreign_link(foreign), Instant::ZERO).unwrap(),
            fdt: Fdt::new(),
            foreign,
            window: answer_window(&foreign),
            plans,
            in_flight: BTreeMap::new(),
            sent: 0,
            starts: Vec::new(),
            accepted_since_start: 0,
            accepted_before: Vec::new(),
        };
        world.act(Instant::ZERO);
        world
    }

    fn post(&mut self, when: Instant, message: Message) {
        self.sent += 1;
        self.in_flight.insert((when, self.sent), message);
    }

    fn act(&mut self, now: Instant) {
        // A success comes before the renewal it may start at once.
        self.accepted_since_start += self.node.take_counters().registrations_accepted;
        loop {
            match self.node.poll_action() {
                Action::Idle => break,
                Action::Register { exchange, ttl, .. } => {
                    assert_eq!(ttl, self.foreign.ttl);
                    let index = self.starts.len();
                    self.starts.push(now);
                    self.accepted_before.push(self.accepted_since_start > 0);
                    self.accepted_since_start = 0;
                    assert!(!self.node.exchange_sent(exchange, now));
                    let Some(plan) = self.plans.get(index) else {
                        continue;
                    };
                    if !plan.request_lost {
                        let delay = part(self.window, plan.request_share);
                        self.post(now + delay, Message::Request(index));
                    }
                }
                Action::Registration(_) | Action::ReleaseHeld | Action::DiscardHeld => {}
                Action::ExchangeEnded { .. } => panic!("no manual exchanges here"),
            }
        }
    }

    /// The BBMD takes request `index` at `now` and answers it.
    fn bbmd(&mut self, index: usize, now: Instant) {
        let plan = self.plans[index].clone();
        let ttl = self.foreign.ttl;
        let outcome = self
            .fdt
            .register(OWN, ttl, now, &LIMITS, Reservation::Ordinary, || {
                !plan.refused
            });
        let code = match outcome.result {
            Ok(_) => BvlcResultCode::SUCCESSFUL_COMPLETION,
            Err(refusal) => refusal.result_code(),
        };
        let left = self.window - part(self.window, plan.request_share);
        for (lost, share) in plan.copies {
            if !lost {
                self.post(now + part(left, share), Message::Result(code));
            }
        }
    }

    /// The lease the node reports at `now` must end no later than the
    /// BBMD's entry, grace period left out.
    fn check(&self, now: Instant) -> Result<(), TestCaseError> {
        let view = self.node.registration(now).unwrap();
        if !view.registered {
            return Ok(());
        }
        let lease = view.lease.unwrap();
        let entry = self.fdt.get(OWN);
        prop_assert!(
            entry.is_some(),
            "lease {lease:?} at {now:?}, but no FDT entry"
        );
        let entry = entry.unwrap();
        let entry_end = entry.registered_at + Duration::from_secs(u64::from(entry.ttl));
        prop_assert!(
            lease.until <= entry_end,
            "lease {lease:?} at {now:?} outlasts the entry {entry:?}"
        );
        Ok(())
    }

    /// Run until `end`, checking the lease after every event. No more than
    /// two attempts start per response timeout: one at most, and the one
    /// renewal a success may ask for at once.
    fn run(&mut self, end: Instant) -> Result<(), TestCaseError> {
        let timeout = self.foreign.effective_response_timeout();
        let most = 2 * (end.as_nanos() / timeout.as_nanos()) as usize + 2;
        loop {
            prop_assert!(
                self.starts.len() <= most,
                "{} attempts by {end:?}: {:?}",
                self.starts.len(),
                &self.starts[..8]
            );
            let arrival = self.in_flight.keys().next().map(|(when, _)| *when);
            let Some(now) = [arrival, self.node.next_deadline()]
                .into_iter()
                .flatten()
                .min()
                .filter(|now| *now <= end)
            else {
                return Ok(());
            };
            if arrival == Some(now) {
                let (_, message) = self.in_flight.pop_first().unwrap();
                match message {
                    Message::Request(index) => self.bbmd(index, now),
                    Message::Result(code) => {
                        let payload = result(code);
                        self.node
                            .receive(&datagram(&payload, BBMD, UNICAST), now, &mut Permissive);
                    }
                }
            } else {
                self.node.on_deadline(now);
            }
            self.act(now);
            self.check(now)?;
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn lease_never_outlives_the_bbmd_entry_without_grace(
        ttl in prop_oneof![Just(2u16), Just(20), Just(60)],
        backoff in any::<bool>(),
        plans in prop::collection::vec(plan(), 48),
    ) {
        let retry = if backoff { Retry::Backoff } else { Retry::AtRenewal };
        let mut world = World::new(ttl, retry, plans);
        let renewal = world.foreign.effective_renewal();
        let timeout = world.foreign.effective_response_timeout();
        // Fifty renewal intervals: the 48 planned attempts and the losses
        // after them.
        world.run(Instant::ZERO + Duration::from_nanos(renewal.as_nanos() * 50))?;
        for (pair, accepted) in world.starts.windows(2).zip(&world.accepted_before[1..]) {
            let gap = pair[1] - pair[0];
            // Never later than one renewal after the attempt before.
            prop_assert!(gap <= renewal, "{pair:?}");
            // Never sooner than one response timeout, but for the one
            // renewal a success that proved an old lease asks for at once.
            prop_assert!(gap >= timeout || *accepted, "{pair:?}");
        }
    }
}
