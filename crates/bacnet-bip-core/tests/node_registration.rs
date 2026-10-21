//! A foreign device's registration: lease evidence apart from the last
//! attempt, refusals, the Distribute-Broadcast-To-Network hint, renewal,
//! retries, and the broadcasts held for the first attempt (Annex J.4.5,
//! J.5.2, J.7.5).

mod node_support;

use bacnet_bip_codec::{BdtEntries, BvlcResultCode, Bvll};
use bacnet_bip_core::node::{
    Action, Admission, AttemptOutcome, ExchangeId, ExchangeRefusal, Foreign, Inbound, LocalFailure,
    Mode, Node, NodeCounters, Permissive, Received, RegistrationEvent, Retry, SendRefusal,
};
use bacnet_bip_core::{Duration, Instant};
use node_support::*;

const NAK: BvlcResultCode = BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
const DBTN_NAK: BvlcResultCode = BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK;

/// A foreign device on `foreign`, started at 0.
fn start(foreign: Foreign) -> Node {
    Node::new(foreign_link(foreign), Instant::ZERO).unwrap()
}

/// Drain the node's actions at `now`, sending any registration at once.
/// Returns the attempt sent, if any, and every other action.
fn act(node: &mut Node, now: Instant) -> (Option<ExchangeId>, Vec<Action>) {
    let mut sent = None;
    let mut others = Vec::new();
    for action in drain(node) {
        match action {
            Action::Register { exchange, .. } => {
                assert!(!node.exchange_sent(exchange, now));
                sent = Some(exchange);
            }
            other => others.push(other),
        }
    }
    (sent, others)
}

/// What a BBMD's result did.
#[derive(Debug, PartialEq, Eq)]
enum Said {
    /// It settled an exchange.
    Settled,
    /// It matched nothing.
    Unmatched {
        code: Option<BvlcResultCode>,
        late: bool,
    },
}

/// The BBMD answers with `code` at `now`.
fn bbmd_says(node: &mut Node, code: BvlcResultCode, now: Instant) -> Said {
    let payload = result(code);
    match node.receive(&datagram(&payload, BBMD, UNICAST), now, &mut Permissive) {
        Inbound::Matched { settled: true, .. } => Said::Settled,
        Inbound::Unmatched { code, late, .. } => Said::Unmatched { code, late },
        other => panic!("{other:?}"),
    }
}

fn registered(since: u64, until: u64) -> Action {
    Action::Registration(RegistrationEvent::Registered {
        since: at(since),
        until: at(until),
    })
}

#[test]
fn success_registers() {
    let mut node = start(foreign());
    let (attempt, _) = act(&mut node, at(0));
    assert!(attempt.is_some());
    let view = node.registration(at(5)).unwrap();
    assert!(!view.registered && view.attempt_in_flight);
    assert_eq!(view.last_outcome, Some(AttemptOutcome::Pending));
    bbmd_says(&mut node, SUCCESS, at(10));
    assert_eq!(act(&mut node, at(10)), (None, vec![registered(0, 60_000)]));
    let view = node.registration(at(10)).unwrap();
    assert_eq!(view.lease.map(span), Some((at(0), at(60_000))));
    assert!(view.registered);
    assert_eq!(view.lease_remaining, Some(ms(59_990)));
    assert_eq!(view.next_attempt, Some(at(30_000)));
    assert_eq!(view.last_outcome, Some(AttemptOutcome::Accepted));
    // A look takes nothing; the take drains.
    let looked = node.counters();
    assert_eq!(node.counters(), looked);
    let counters = node.take_counters();
    assert_eq!(counters, looked);
    assert_eq!(node.counters(), NodeCounters::default());
    assert_eq!(counters.registration_attempts, 1);
    assert_eq!(counters.registrations_accepted, 1);
    let attempt = counters.last_registration_attempt.unwrap();
    assert_eq!((attempt.bbmd, attempt.ttl), (BBMD, 60));
    assert_eq!(
        (attempt.outcome, attempt.result),
        (AttemptOutcome::Accepted, Some(SUCCESS))
    );
    // A lease ends at its time-to-live, not 30 s later.
    assert!(!node.registration(at(60_000)).unwrap().registered);
}

#[test]
fn a_refusal_keeps_earlier_evidence() {
    let mut node = start(foreign());
    act(&mut node, at(0));
    bbmd_says(&mut node, SUCCESS, at(10));
    act(&mut node, at(10));
    node.on_deadline(at(30_000));
    act(&mut node, at(30_000));
    bbmd_says(&mut node, NAK, at(30_010));
    assert_eq!(
        act(&mut node, at(30_010)),
        (
            None,
            vec![Action::Registration(RegistrationEvent::Refused {
                code: NAK
            })]
        )
    );
    let view = node.registration(at(30_010)).unwrap();
    assert!(view.registered);
    assert_eq!(view.lease.unwrap().until, at(60_000));
    assert_eq!(view.last_outcome, Some(AttemptOutcome::Refused));
    // As today: the next attempt one renewal after this one started.
    assert_eq!(view.next_attempt, Some(at(60_000)));
    assert_eq!(view.consecutive_failures, 0);
    let counters = node.take_counters();
    assert_eq!(
        (
            counters.registrations_accepted,
            counters.registrations_refused
        ),
        (1, 1)
    );
}

#[test]
fn distribute_nak_pulls_the_renewal_forward() {
    let mut node = start(foreign());
    act(&mut node, at(0));
    // Too soon after the last attempt started: one response timeout after.
    bbmd_says(&mut node, SUCCESS, at(10));
    act(&mut node, at(10));
    assert_eq!(
        bbmd_says(&mut node, DBTN_NAK, at(1_000)),
        Said::Unmatched {
            code: Some(DBTN_NAK),
            late: false
        }
    );
    assert_eq!(node.next_deadline(), Some(at(3_000)));
    node.on_deadline(at(3_000));
    assert!(act(&mut node, at(3_000)).0.is_some(), "pulled forward");
    // While that one is in flight, a hint changes nothing.
    bbmd_says(&mut node, DBTN_NAK, at(3_005));
    assert_eq!(node.next_deadline(), Some(at(6_000)));
    assert_eq!(bbmd_says(&mut node, SUCCESS, at(3_010)), Said::Settled);
    act(&mut node, at(3_010));
    // That success could be a copy of the first one's, 3 s earlier, so
    // the lease still starts at 0.
    assert_eq!(
        node.registration(at(3_010)).unwrap().next_attempt,
        Some(at(30_000))
    );
    // Later on, the hint pulls it forward to now.
    bbmd_says(&mut node, DBTN_NAK, at(20_000));
    assert!(act(&mut node, at(20_000)).0.is_some());
    assert_eq!(node.take_counters().renewal_hints, 3);
    // From anyone but the BBMD, it is no hint.
    let mut node = start(foreign());
    act(&mut node, at(0));
    bbmd_says(&mut node, SUCCESS, at(10));
    let stray = result(DBTN_NAK);
    node.receive(
        &datagram(&stray, PEER, UNICAST),
        at(20_000),
        &mut Permissive,
    );
    assert_eq!(node.next_deadline(), Some(at(30_000)));
    assert_eq!(node.take_counters().renewal_hints, 0);
}

#[test]
fn at_renewal_takes_no_hint_from_a_distribute_nak() {
    // Today's schedule, as the hosted transport keeps it until it backs off.
    let mut node = start(with(foreign(), |f| f.retry = Retry::AtRenewal));
    act(&mut node, at(0));
    bbmd_says(&mut node, SUCCESS, at(10));
    act(&mut node, at(10));
    assert_eq!(
        bbmd_says(&mut node, DBTN_NAK, at(20_000)),
        Said::Unmatched {
            code: Some(DBTN_NAK),
            late: false
        }
    );
    assert!(act(&mut node, at(20_000)).0.is_none());
    assert_eq!(node.next_deadline(), Some(at(30_000)));
    let counters = node.take_counters();
    assert_eq!(
        (counters.renewal_hints, counters.unmatched_responses),
        (0, 1)
    );
    assert_eq!(counters.last_unmatched_result, Some(DBTN_NAK));
}

#[test]
fn renewal_is_independent_of_ttl() {
    // A NAT router may forget a mapping in 30 s or less, whatever the
    // time-to-live the BBMD keeps.
    let foreign = with(Foreign::new(BBMD, 3600), |f| {
        f.renewal = Duration::from_secs(20);
    });
    let mut node = start(foreign);
    let mut starts = Vec::new();
    for second in 0..=60 {
        let now = at(second * 1_000);
        node.on_deadline(now);
        if let (Some(_), _) = act(&mut node, now) {
            starts.push(second);
            bbmd_says(&mut node, SUCCESS, now + ms(10));
        }
    }
    assert_eq!(starts, [0, 20, 40, 60]);
    let attempt = node.take_counters().last_registration_attempt.unwrap();
    assert_eq!(attempt.ttl, 3600);
    assert_eq!(
        node.registration(at(60_010)).unwrap().lease.unwrap().until,
        at(3_660_000)
    );
}

#[test]
fn broadcasts_are_held_until_the_first_attempt_resolves() {
    let holding = with(foreign(), |f| f.hold = 2);
    // Released on success.
    let mut node = start(holding);
    act(&mut node, at(0));
    assert_eq!(node.admit_broadcast(at(1)), Ok(Admission::Hold));
    assert_eq!(node.admit_broadcast(at(2)), Ok(Admission::Hold));
    assert_eq!(node.admit_broadcast(at(3)), Err(SendRefusal::HoldFull));
    assert_eq!(node.registration(at(3)).unwrap().held, 2);
    bbmd_says(&mut node, SUCCESS, at(10));
    assert_eq!(
        act(&mut node, at(10)).1,
        [registered(0, 60_000), Action::ReleaseHeld]
    );
    assert_eq!(node.admit_broadcast(at(11)), Ok(Admission::Send));
    let counters = node.take_counters();
    assert_eq!(
        (counters.broadcasts_held, counters.broadcasts_released),
        (2, 2)
    );
    assert_eq!(
        (
            counters.broadcasts_hold_full,
            counters.broadcasts_unregistered
        ),
        (1, 0)
    );

    // Released on a timeout, then sent, and counted, while unregistered.
    let mut node = start(holding);
    act(&mut node, at(0));
    assert_eq!(node.admit_broadcast(at(1)), Ok(Admission::Hold));
    node.on_deadline(at(3_000));
    assert_eq!(
        act(&mut node, at(3_000)).1,
        [
            Action::Registration(RegistrationEvent::Unanswered),
            Action::ReleaseHeld
        ]
    );
    assert_eq!(node.admit_broadcast(at(3_001)), Ok(Admission::Send));
    assert_eq!(node.take_counters().broadcasts_unregistered, 1);

    // Discarded on a refusal.
    let mut node = start(holding);
    act(&mut node, at(0));
    assert_eq!(node.admit_broadcast(at(1)), Ok(Admission::Hold));
    bbmd_says(&mut node, NAK, at(10));
    assert_eq!(
        act(&mut node, at(10)).1,
        [
            Action::Registration(RegistrationEvent::Refused { code: NAK }),
            Action::DiscardHeld
        ]
    );
    assert_eq!(node.take_counters().broadcasts_discarded, 1);

    // With no room to hold, sent at once.
    let mut node = start(foreign());
    act(&mut node, at(0));
    assert_eq!(node.admit_broadcast(at(1)), Ok(Admission::Send));
    assert_eq!(node.take_counters().broadcasts_unregistered, 1);
    // Other modes always send.
    for mode in [Mode::Normal, Mode::Bbmd] {
        let mut node: Node = Node::new(link(mode), Instant::ZERO).unwrap();
        assert_eq!(node.admit_broadcast(at(1)), Ok(Admission::Send));
        assert!(node.registration(at(1)).is_none());
    }
}

/// Attempt start times over `until` ms when every attempt is lost.
fn starts_when_all_lost(retry: Retry, until: u64) -> Vec<u64> {
    let mut node = start(with(foreign(), |f| f.retry = retry));
    let mut starts = Vec::new();
    let mut now = 0;
    while now <= until {
        node.on_deadline(at(now));
        if act(&mut node, at(now)).0.is_some() {
            starts.push(now);
        }
        now = node.next_deadline().unwrap().as_nanos() / 1_000_000;
    }
    starts
}

#[test]
fn backoff_is_capped_at_the_renewal() {
    // 3 s timeouts, then waits of 3, 6, 12 and 24 s, and 30 s after that,
    // each cut to one renewal after the attempt before.
    assert_eq!(
        starts_when_all_lost(Retry::Backoff, 120_000),
        [0, 6_000, 15_000, 30_000, 57_000, 87_000, 117_000]
    );
    // Today's schedule: one attempt per renewal interval.
    assert_eq!(
        starts_when_all_lost(Retry::AtRenewal, 100_000),
        [0, 30_000, 60_000, 90_000]
    );
}

#[test]
fn a_failed_send_retries_one_timeout_later() {
    let mut node = start(foreign());
    let (attempt, _) = {
        let actions = drain(&mut node);
        let Action::Register { exchange, .. } = actions[0] else {
            panic!()
        };
        (exchange, ())
    };
    node.exchange_failed(attempt, LocalFailure::SendFailed, at(0));
    assert_eq!(
        drain(&mut node),
        [Action::Registration(RegistrationEvent::Unanswered)]
    );
    let view = node.registration(at(0)).unwrap();
    assert_eq!(view.last_outcome, Some(AttemptOutcome::LocalError));
    assert_eq!(view.next_attempt, Some(at(3_000)));
    assert_eq!(node.take_counters().register_foreign_device.local_errors, 1);
}

#[test]
fn a_bbmd_on_another_port_is_answered_from_that_port() {
    let bbmd = addr(10, 0, 0, 1, 47_809);
    let mut node = start(Foreign::new(bbmd, 60));
    let actions = drain(&mut node);
    assert!(matches!(actions[..], [Action::Register { to, .. }] if to == bbmd));
    let Action::Register { exchange, .. } = actions[0] else {
        panic!()
    };
    node.exchange_sent(exchange, at(0));
    let success = result(SUCCESS);
    // The BBMD's IP at the default port is someone else.
    assert!(matches!(
        node.receive(&datagram(&success, BBMD, UNICAST), at(5), &mut Permissive),
        Inbound::Unmatched { .. }
    ));
    assert!(matches!(
        node.receive(&datagram(&success, bbmd, UNICAST), at(10), &mut Permissive),
        Inbound::Matched { settled: true, .. }
    ));
    assert!(node.registration(at(10)).unwrap().registered);
}

#[test]
fn a_foreign_device_refuses_manual_requests_to_its_bbmd() {
    let mut node = start(foreign());
    act(&mut node, at(0));
    bbmd_says(&mut node, SUCCESS, at(10));
    act(&mut node, at(10));
    // The node owns its registration with the BBMD: a manual request a
    // success answers could change the entry behind the lease, whatever
    // its time-to-live, or pass for the registration's answer.
    let owned = [
        Bvll::RegisterForeignDevice { ttl: 0 },
        Bvll::RegisterForeignDevice { ttl: 60 },
        Bvll::DeleteForeignDeviceTableEntry(OWN),
        Bvll::WriteBroadcastDistributionTable(BdtEntries::from_entries(&[])),
    ];
    for request in owned {
        assert_eq!(
            node.begin_exchange(BBMD, request, ms(3_000), at(20))
                .map(|_| ()),
            Err(ExchangeRefusal::ManagedRegistration),
            "{request:?}"
        );
    }
    // Reading its tables is fine, and so is registering elsewhere, which
    // leaves the lease alone.
    for request in [
        Bvll::ReadBroadcastDistributionTable,
        Bvll::ReadForeignDeviceTable,
    ] {
        let (read, _) = node
            .begin_exchange(BBMD, request, ms(3_000), at(20))
            .unwrap();
        node.exchange_failed(read, LocalFailure::Cancelled, at(20));
    }
    let elsewhere = addr(10, 0, 0, 2, PORT);
    let request = Bvll::RegisterForeignDevice { ttl: 0 };
    let (manual, _) = node
        .begin_exchange(elsewhere, request, ms(3_000), at(20))
        .unwrap();
    node.exchange_sent(manual, at(20));
    let success = result(SUCCESS);
    node.receive(
        &datagram(&success, elsewhere, UNICAST),
        at(30),
        &mut Permissive,
    );
    assert!(act(&mut node, at(30)).1.is_empty());
    assert_eq!(
        node.registration(at(30)).unwrap().lease.unwrap().until,
        at(60_000)
    );
    let counters = node.take_counters();
    assert_eq!(counters.register_foreign_device.busy, 0);
    assert_eq!(counters.registration_attempts, 2);
    // A normal node, as the CLI's one-shot registration uses, may send them.
    let mut normal: Node = Node::new(link(Mode::Normal), Instant::ZERO).unwrap();
    for request in owned {
        let (exchange, _) = normal
            .begin_exchange(BBMD, request, ms(3_000), at(1))
            .unwrap();
        normal.exchange_failed(exchange, LocalFailure::Cancelled, at(1));
    }
}

#[test]
fn a_due_attempt_with_no_free_slot_retries_without_an_event() {
    let mut node: Node<1> = Node::new(foreign_link(foreign()), Instant::ZERO).unwrap();
    let Action::Register { exchange, .. } = drain(&mut node)[0] else {
        panic!("a registration");
    };
    node.exchange_sent(exchange, at(0));
    let success = result(SUCCESS);
    node.receive(&datagram(&success, BBMD, UNICAST), at(10), &mut Permissive);
    drain(&mut node);
    // A manual read holds the only slot when the renewal falls due: it
    // counts as busy and retries one response timeout later.
    let request = Bvll::ReadForeignDeviceTable;
    let (manual, _) = node
        .begin_exchange(BBMD, request, ms(3_000), at(29_000))
        .unwrap();
    node.on_deadline(at(30_000));
    assert!(drain(&mut node).is_empty());
    assert_eq!(
        node.registration(at(30_000)).unwrap().next_attempt,
        Some(at(33_000))
    );
    node.exchange_failed(manual, LocalFailure::Cancelled, at(31_000));
    node.on_deadline(at(33_000));
    assert!(matches!(drain(&mut node)[..], [Action::Register { .. }]));
    let counters = node.take_counters();
    assert_eq!(counters.register_foreign_device.busy, 1);
    assert_eq!(counters.registration_attempts, 2);
    let last = counters.last_registration_attempt.unwrap();
    assert_eq!((last.ttl, last.outcome), (60, AttemptOutcome::Pending));
}

/// Run a foreign device whose BBMD answers every attempt with success
/// `after` its send, until `end` or `cap` attempts: their start times.
fn answered_starts(node: &mut Node, after: Duration, end: Instant, cap: usize) -> Vec<Instant> {
    let mut starts = Vec::new();
    let mut answer: Option<Instant> = None;
    let mut now = Instant::ZERO;
    loop {
        if act(node, now).0.is_some() {
            starts.push(now);
            answer = Some(now + after);
        }
        if starts.len() > cap {
            return starts;
        }
        let Some(next) = [answer, node.next_deadline()].into_iter().flatten().min() else {
            return starts;
        };
        if next > end {
            return starts;
        }
        now = next;
        if answer == Some(now) {
            answer = None;
            assert_eq!(bbmd_says(node, SUCCESS, now), Said::Settled);
        } else {
            node.on_deadline(now);
        }
    }
}

#[test]
fn a_short_ttl_renews_once_per_interval() {
    // A 2 s time-to-live: renewal and response timeout 1 s each, so the
    // answer window is the renewal interval. A renewal never counts its
    // predecessor, so each lease starts at its own attempt.
    let every_second: Vec<Instant> = (0..=10).map(|second| at(second * 1_000)).collect();
    // Answered at once too: the attempt one renewal back stays out.
    let mut instant = start(Foreign::new(BBMD, 2));
    assert_eq!(
        answered_starts(&mut instant, ms(0), at(10_010), 40),
        every_second
    );
    let mut node = start(Foreign::new(BBMD, 2));
    let starts = answered_starts(&mut node, ms(10), at(10_010), 40);
    assert_eq!(starts, every_second);
    let lease = node.registration(at(10_010)).unwrap().lease.unwrap();
    assert_eq!(span(lease), (at(10_000), at(12_000)));
}

#[test]
fn a_copy_of_an_answer_never_stretches_a_short_lease() {
    // A 2 s time-to-live: renewal and response timeout 1 s each.
    let mut node = start(Foreign::new(BBMD, 2));
    // Attempt 1, at 0, is lost; attempt 2 goes when it times out.
    act(&mut node, at(0));
    node.on_deadline(at(1_000));
    let (second, events) = act(&mut node, at(1_000));
    assert!(second.is_some());
    assert_eq!(
        events,
        [Action::Registration(RegistrationEvent::Unanswered)]
    );
    // The BBMD takes attempt 2 at 1.1 s, so its entry ends at 3.1 s. The
    // success at 1.2 s could answer attempt 1, so the lease starts at 0,
    // and the renewal it makes due goes at once.
    assert_eq!(bbmd_says(&mut node, SUCCESS, at(1_200)), Said::Settled);
    let (third, events) = act(&mut node, at(1_200));
    assert!(third.is_some());
    assert_eq!(events, [registered(0, 2_000)]);
    // Attempt 3 is lost. A copy of attempt 2's success, 0.75 s after its
    // send, settles it: the lease may start no later than attempt 2.
    assert_eq!(bbmd_says(&mut node, SUCCESS, at(1_750)), Said::Settled);
    assert_eq!(
        act(&mut node, at(1_750)).1,
        [Action::Registration(RegistrationEvent::Renewed {
            until: at(3_000)
        })]
    );
    let entry_end = at(3_100);
    let lease = node.registration(at(1_750)).unwrap().lease.unwrap();
    assert!(lease.until <= entry_end, "{lease:?}");
    assert!(!node.registration(at(3_050)).unwrap().registered);
}

#[test]
fn a_result_as_an_attempt_falls_due_is_not_its_answer() {
    let mut node = start(foreign());
    act(&mut node, at(0));
    bbmd_says(&mut node, SUCCESS, at(10));
    act(&mut node, at(10));
    // At 30 s the renewal falls due, and a stray success arrives before the
    // node is woken for it. The renewal isn't sent yet, so the success
    // can't answer it: it matches nothing.
    assert_eq!(
        bbmd_says(&mut node, SUCCESS, at(30_000)),
        Said::Unmatched {
            code: Some(SUCCESS),
            late: false
        }
    );
    let (attempt, _) = act(&mut node, at(30_000));
    assert!(attempt.is_some());
    assert!(node.registration(at(30_000)).unwrap().attempt_in_flight);
    assert_eq!(bbmd_says(&mut node, SUCCESS, at(30_010)), Said::Settled);
    act(&mut node, at(30_010));
    // The same through the host's split path: decide, then resolve.
    let stray = result(SUCCESS);
    let Received::Response(response) = receive(node.link(), &stray, BBMD, UNICAST) else {
        panic!("a response");
    };
    assert!(matches!(
        node.resolve(response, at(60_000)),
        Inbound::Unmatched { .. }
    ));
    assert!(act(&mut node, at(60_000)).0.is_some());
}
