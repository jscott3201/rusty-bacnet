//! Virtual-time traces of the router engine (#1679 design, section 5.2),
//! with the hosted router's options (`Options::HOSTED`). Each step asserts
//! the full list of sends, their octets, and the counters at that instant.
//!
//! Where a step departs from the design's sketch, the hosted router's code
//! decided, and the step says which line.

mod fixture;

use bacnet_router_core::npdu::NetworkPriority;
use bacnet_router_core::npdu::Npdu;
use bacnet_router_core::npdu::{NetworkMessageType, RejectMessageReason};
use bacnet_router_core::router::{Cause, ControlClass, Counters, Event, Kind, Options, Permissive};
use bacnet_router_core::table::{Convergence, Reachability, RejectEffect};
use bacnet_router_core::{Instant, PortId};
use fixture::{
    addr, apdu_to, at, at_ms, bytes, claim_counts, deliver, deliver_broadcast, drain, i_am,
    learn_9_via_r7, message, networks, options, reject_data, router, router_counts, router_with,
    sent_reject, two_ports, Outcome, Recorder, Sent, TestRouter, CONFIRMED, NODE_A, OWN_0, R7,
};

/// A confirmed request from A for node 5 on network 9.
fn from_a_to_9() -> Vec<u8> {
    apdu_to(9, &[5], CONFIRMED)
}

/// The router's counts: `rejects` sent and `solicitations` made, nothing
/// else.
fn counts(rejects: u32, solicitations: u32) -> Counters {
    router_counts(|c| {
        c.rejects = rejects;
        c.solicitations = solicitations;
    })
}

/// The reject the router sends A for that request, with `reason`.
fn reject_to_a(sent: &Sent, why: RejectMessageReason) {
    assert_eq!(sent.port, 0);
    assert_eq!(sent.to.as_deref(), Some(NODE_A));
    assert_eq!(sent.kind, Kind::Reject(why));
    assert_eq!(sent.cause, Cause::Ingress);
    let data = reject_data(why, 9);
    let mut expected = Npdu::message(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK, &data);
    expected.hop_count = 255;
    assert_eq!(sent.npdu, bytes(&expected));
    assert_eq!(sent_reject(sent), (why, 9, None));
}

/// The request as forwarded to R7: DNET 9 and DADR kept, SNET 1 and A's
/// SADR added, the hop count one lower.
fn forwarded_to_r7(sent: &Sent) {
    assert_eq!(sent.port, 1);
    assert_eq!(sent.to.as_deref(), Some(R7));
    assert_eq!(sent.kind, Kind::Forward);
    let mut expected = Npdu::apdu(CONFIRMED);
    expected.destination = Some(addr(9, &[5]));
    expected.source = Some(addr(1, NODE_A));
    expected.hop_count = 254;
    assert_eq!(sent.npdu, bytes(&expected));
}

/// A broadcast on `port` of a message of type `message_type`, sent as
/// `kind`, whose data is `list`: octet for octet.
fn list_broadcast(
    sent: &Sent,
    port: u8,
    kind: Kind,
    message_type: NetworkMessageType,
    list: &[u16],
) {
    assert_eq!(sent.port, port);
    assert!(sent.is_broadcast());
    assert_eq!(sent.kind, kind);
    assert_eq!(sent.priority, NetworkPriority::Normal);
    assert_eq!(sent.npdu, message(message_type, &networks(list)));
}

/// Trace A: discovery, learning and aging.
#[test]
fn a_discovery_learning_and_aging() {
    let mut router = router();
    let mut host = Recorder::default();
    let i_am_type = NetworkMessageType::I_AM_ROUTER_TO_NETWORK;
    let learned = |learned_ok| claim_counts(|c| c.learned_ok = learned_ok);

    // t = 0: port by port, the port's own network number, configured
    // (Clause 6.4.20), then the other port's network (Clause 6.6.2).
    router.start();
    let sent = drain(&mut router);
    assert_eq!(sent.len(), 4);
    let number = Kind::Announce(NetworkMessageType::NETWORK_NUMBER_IS);
    let announce = Kind::Announce(i_am_type);
    assert_eq!((sent[0].port, sent[0].kind), (0, number));
    assert_eq!(
        sent[0].npdu,
        message(NetworkMessageType::NETWORK_NUMBER_IS, &[0, 1, 1])
    );
    list_broadcast(&sent[1], 0, announce, i_am_type, &[2]);
    assert_eq!((sent[2].port, sent[2].kind), (1, number));
    assert_eq!(
        sent[2].npdu,
        message(NetworkMessageType::NETWORK_NUMBER_IS, &[0, 2, 1])
    );
    list_broadcast(&sent[3], 1, announce, i_am_type, &[1]);
    assert!(sent
        .iter()
        .all(|sent| sent.is_broadcast() && sent.cause == Cause::Local));
    assert_eq!(router.next_deadline(), Some(at(60)));
    assert_eq!(router.counters(), counts(0, 0));

    // t = 1 s: no route to 9: a search on port 1, then reason 1 to A.
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &from_a_to_9(), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent.len(), 2);
    assert_eq!((sent[0].port, sent[0].is_broadcast()), (1, true));
    assert_eq!(
        (sent[0].kind, sent[0].cause),
        (Kind::Solicit, Cause::Ingress)
    );
    assert_eq!(
        sent[0].npdu,
        message(
            NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK,
            &networks(&[9])
        )
    );
    reject_to_a(&sent[1], RejectMessageReason::NOT_DIRECTLY_CONNECTED);
    assert_eq!(router.counters(), counts(1, 1));

    // t = 2 s: within the 5 s cooldown: the reject alone.
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &from_a_to_9(), at(2));
    assert_eq!(sent.len(), 1);
    reject_to_a(&sent[0], RejectMessageReason::NOT_DIRECTLY_CONNECTED);
    assert_eq!(router.counters(), counts(2, 1));

    // t = 3 s: R7 answers. 9 is learned through port 1 and R7, and the
    // answer goes on to port 0 as received.
    let (outcome, sent) = deliver_broadcast(&mut router, &mut host, 1, R7, &i_am(&[9]), at(3));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent.len(), 1);
    list_broadcast(&sent[0], 0, Kind::Relay(i_am_type), i_am_type, &[9]);
    let route = router.table().lookup(9).unwrap();
    assert_eq!((route.port(), route.next_hop().as_slice()), (PortId(1), R7));
    assert_eq!(route.last_seen(), Some(at(3)));
    assert_eq!(
        host.events,
        [Event::Learned {
            network: 9,
            port: PortId(1),
            class: ControlClass::IAm
        }]
    );
    assert_eq!(router.counters(), counts(2, 1));
    assert_eq!(router.table().claim_counters(), learned(1));

    // t = 4 s: the request goes to R7.
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &from_a_to_9(), at(4));
    assert_eq!(sent.len(), 1);
    forwarded_to_r7(&sent[0]);
    assert_eq!(router.table().lookup(9).unwrap().last_used(), Some(at(4)));
    assert_eq!(router.counters(), counts(2, 1));

    // t = 60 s: the sweep forgets the search for 9 (59 s old, past 30 s),
    // and nothing else: 9 is 57 s old, and nothing is pending.
    let before = host.events.len();
    router.on_deadline(at(60), &mut host);
    assert_eq!(
        host.events[before..],
        [Event::DiscoveryExpired { network: 9 }]
    );
    assert!(router.discovery().is_empty());
    assert_eq!(router.next_deadline(), Some(at(120)));
    assert_eq!(router.counters(), counts(2, 1));

    // t = 4 s to 300 s: traffic every 10 s moves only `last_used`, and the
    // sweeps up to 300 s keep the route (297 s old is within 300 s).
    let before = host.events.len();
    for second in (14..=294).step_by(10) {
        // The host runs each sweep at its minute; early calls do nothing.
        router.on_deadline(at(second / 60 * 60), &mut host);
        let (_, sent) = deliver(
            &mut router,
            &mut host,
            0,
            NODE_A,
            &from_a_to_9(),
            at(second),
        );
        assert_eq!(sent.len(), 1);
        forwarded_to_r7(&sent[0]);
        assert_eq!(router.counters(), counts(2, 1));
    }
    router.on_deadline(at(300), &mut host);
    assert_eq!(host.events.len(), before, "no sweep removed anything");
    assert_eq!(router.next_deadline(), Some(at(360)));
    let route = router.table().lookup(9).unwrap();
    assert_eq!(route.last_seen(), Some(at(3)));
    assert_eq!(route.last_used(), Some(at(294)));

    // t = 360 s: confirmed last at 3 s, so 357 s old: purged.
    router.on_deadline(at(360), &mut host);
    assert_eq!(host.events[before..], [Event::Purged { network: 9 }]);
    assert!(router.table().lookup(9).is_none());
    assert_eq!(router.counters(), counts(2, 1));

    // t = 361 s: looked for again, and refused again.
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &from_a_to_9(), at(361));
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].kind, Kind::Solicit);
    assert_eq!(
        sent[0].npdu,
        message(
            NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK,
            &networks(&[9])
        )
    );
    reject_to_a(&sent[1], RejectMessageReason::NOT_DIRECTLY_CONNECTED);
    assert_eq!(router.counters(), counts(3, 2));
    assert_eq!(router.table().claim_counters(), learned(1));
}

/// Trace B: flow control from R7, which serves 9 and 10.
///
/// Busy marks are read at their deadline, so the trace never runs the sweep
/// and the mark still lapses on time (router_table.rs:681-692).
#[test]
fn b_flow_control() {
    let mut router = router();
    let mut host = Recorder::default();
    deliver_broadcast(&mut router, &mut host, 1, R7, &i_am(&[9, 10]), at(0));
    let busy_type = NetworkMessageType::ROUTER_BUSY_TO_NETWORK;
    let available_type = NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK;
    let busy = |list: &[u16]| message(busy_type, &networks(list));
    let available = |list: &[u16]| message(available_type, &networks(list));
    let marked = |network, busy| Event::Marked { network, busy };

    // t = 10 s: Busy for 9 marks it until 40 s and goes on to port 0.
    host.events.clear();
    let (outcome, sent) = deliver_broadcast(&mut router, &mut host, 1, R7, &busy(&[9]), at(10));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent.len(), 1);
    list_broadcast(&sent[0], 0, Kind::Relay(busy_type), busy_type, &[9]);
    assert_eq!(router.table().lookup(9).unwrap().busy_until(), Some(at(40)));
    assert_eq!(host.events, [marked(9, true)]);
    assert_eq!(router.counters(), counts(0, 0));

    // t = 11 s: reason 2.
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &from_a_to_9(), at(11));
    assert_eq!(sent.len(), 1);
    reject_to_a(&sent[0], RejectMessageReason::ROUTER_BUSY);
    assert_eq!(router.counters(), counts(1, 0));

    // t = 20 s: Available lifts it; at 21 s the request goes through.
    host.events.clear();
    let (_, sent) = deliver_broadcast(&mut router, &mut host, 1, R7, &available(&[9]), at(20));
    assert_eq!(sent.len(), 1);
    list_broadcast(
        &sent[0],
        0,
        Kind::Relay(available_type),
        available_type,
        &[9],
    );
    assert_eq!(host.events, [marked(9, false)]);
    assert_eq!(router.counters(), counts(1, 0));
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &from_a_to_9(), at(21));
    assert_eq!(sent.len(), 1);
    forwarded_to_r7(&sent[0]);
    assert_eq!(router.counters(), counts(1, 0));

    // t = 30 s: Busy with no list covers everything R7 serves.
    host.events.clear();
    let (_, sent) = deliver_broadcast(&mut router, &mut host, 1, R7, &busy(&[]), at(30));
    assert_eq!(sent.len(), 1);
    list_broadcast(&sent[0], 0, Kind::Relay(busy_type), busy_type, &[]);
    assert_eq!(host.events, [marked(9, true), marked(10, true)]);
    assert_eq!(router.counters(), counts(1, 0));
    let to_10 = apdu_to(10, &[5], CONFIRMED);

    // t = 59.999 s: still busy, both of them.
    let (_, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &from_a_to_9(),
        at_ms(59_999),
    );
    assert_eq!(sent.len(), 1);
    reject_to_a(&sent[0], RejectMessageReason::ROUTER_BUSY);
    assert_eq!(router.counters(), counts(2, 0));
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &to_10, at_ms(59_999));
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent_reject(&sent[0]),
        (RejectMessageReason::ROUTER_BUSY, 10, None)
    );
    assert_eq!(router.counters(), counts(3, 0));

    // t = 60 s: the deadline: forwarded, with no sweep since t = 0.
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &from_a_to_9(), at(60));
    assert_eq!(sent.len(), 1);
    forwarded_to_r7(&sent[0]);
    assert_eq!(
        router.table().lookup(9).unwrap().reachability(),
        Reachability::Busy,
        "the stored mark waits for the sweep; the decision didn't"
    );
    assert_eq!(router.counters(), counts(3, 0));
}

/// Trace C: rejects from R7 about network 9, each addressed to the router.
///
/// A reason-4 reject changes no route (Clause 6.6.3.5): at 31 s and again
/// at 60 s the route stays, and each counts as kept.
#[test]
fn c_received_rejects() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    let reject = |why: RejectMessageReason| {
        message(
            NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
            &reject_data(why, 9),
        )
    };
    let state = |router: &TestRouter| router.table().lookup(9).map(|r| r.reachability());
    let mut step = |router: &mut TestRouter, why: RejectMessageReason, second: u64| {
        let frame = reject(why);
        let (outcome, sent) = deliver(router, &mut host, 1, R7, &frame, at(second));
        assert_eq!(outcome, Outcome::ToRouter(frame), "addressed to the router");
        assert!(sent.is_empty(), "nothing answers a reject");
        assert_eq!(router.counters(), Counters::default());
        match host.events.pop() {
            Some(Event::RejectReceived { effect, .. }) => effect,
            other => panic!("expected a reject event, got {other:?}"),
        }
    };
    // The claim counts after each step: applied, same state, held down,
    // and kept.
    let rejects_kept = |applied, same, held, kept| {
        claim_counts(|c| {
            c.reject_applied = applied;
            c.reject_dampened = same + held;
            c.reject_dampened_same_state = same;
            c.reject_dampened_hold_down = held;
            c.reject_kept = kept;
        })
    };
    let rejects = |applied, same, held| rejects_kept(applied, same, held, 0);

    // t = 0: reason 1 makes 9 unreachable.
    assert_eq!(
        step(&mut router, RejectMessageReason::NOT_DIRECTLY_CONNECTED, 0),
        RejectEffect::Applied
    );
    assert_eq!(state(&router), Some(Reachability::Unreachable));
    assert_eq!(router.table().claim_counters(), rejects(1, 0, 0));
    // t = 10 s: already so.
    assert_eq!(
        step(&mut router, RejectMessageReason::NOT_DIRECTLY_CONNECTED, 10),
        RejectEffect::SameState
    );
    assert_eq!(router.table().claim_counters(), rejects(1, 1, 0));
    // t = 12 s: Available lifts it, and goes on to port 0.
    let available = message(
        NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK,
        &networks(&[9]),
    );
    let (_, sent) = deliver_broadcast(
        &mut router,
        &mut Recorder::default(),
        1,
        R7,
        &available,
        at(12),
    );
    assert_eq!(sent.len(), 1);
    assert_eq!(
        (sent[0].port, sent[0].npdu.as_slice()),
        (0, available.as_slice())
    );
    assert_eq!(state(&router), Some(Reachability::Reachable));
    assert_eq!(router.counters(), Counters::default());
    // t = 15 s: within the hold-down from t = 0.
    assert_eq!(
        step(&mut router, RejectMessageReason::NOT_DIRECTLY_CONNECTED, 15),
        RejectEffect::HeldDown
    );
    assert_eq!(state(&router), Some(Reachability::Reachable));
    assert_eq!(router.table().claim_counters(), rejects(1, 1, 1));
    // t = 30 s: the hold-down is over.
    assert_eq!(
        step(&mut router, RejectMessageReason::NOT_DIRECTLY_CONNECTED, 30),
        RejectEffect::Applied
    );
    assert_eq!(state(&router), Some(Reachability::Unreachable));
    assert_eq!(router.table().claim_counters(), rejects(2, 1, 1));
    // t = 31 s: reason 4 is kept, changing nothing.
    assert_eq!(
        step(&mut router, RejectMessageReason::MESSAGE_TOO_LONG, 31),
        RejectEffect::Kept
    );
    assert_eq!(state(&router), Some(Reachability::Unreachable));
    assert_eq!(router.table().claim_counters(), rejects_kept(2, 1, 1, 1));
    // t = 60 s: reason 4 again, past the hold-down from 30 s: the route
    // still stays.
    assert_eq!(
        step(&mut router, RejectMessageReason::MESSAGE_TOO_LONG, 60),
        RejectEffect::Kept
    );
    assert_eq!(state(&router), Some(Reachability::Unreachable));
    assert_eq!(router.table().claim_counters(), rejects_kept(2, 1, 1, 2));
}

/// Trace D: hardened convergence. B is another router, on port 0.
#[test]
fn d_hardened_convergence() {
    const B: &[u8] = &[10, 0, 0, 7, 0xBA, 0xC0];
    let options = options(|o| o.convergence = Convergence::Hardened);
    let mut router = router_with(&two_ports(), options);
    let mut host = Permissive;
    let claim = |router: &mut TestRouter, port: u8, from: &[u8], second: u64| {
        let frame = i_am(&[9]);
        let mut host = Recorder::default();
        let (outcome, sent) = deliver_broadcast(router, &mut host, port, from, &frame, at(second));
        assert_eq!(outcome, Outcome::Handled);
        // Passed on whether or not it was taken.
        assert_eq!(sent.len(), 1);
        assert_eq!(
            (sent[0].port, sent[0].npdu.as_slice()),
            (1 - port, frame.as_slice())
        );
        assert_eq!(router.counters(), Counters::default());
    };
    // The claim counts: learned, pending started, corroborated, expired.
    let claims = |learned_ok, pending_started, corroborated_applied| {
        claim_counts(|c| {
            c.learned_ok = learned_ok;
            c.pending_started = pending_started;
            c.corroborated_applied = corroborated_applied;
        })
    };
    let route_port = |router: &TestRouter| router.table().lookup(9).unwrap().port();
    let from_port_1 = apdu_to(9, &[5], CONFIRMED);

    // t = 0: a new network is learned at once.
    claim(&mut router, 1, R7, 0);
    assert_eq!(route_port(&router), PortId(1));
    assert_eq!(router.table().claim_counters(), claims(1, 0, 0));
    // t = 1 s: a claim from port 0 waits for a second; traffic still goes to
    // port 1.
    claim(&mut router, 0, B, 1);
    assert_eq!(route_port(&router), PortId(1));
    assert_eq!(router.table().pending(9).unwrap().port, PortId(0));
    assert_eq!(router.table().claim_counters(), claims(1, 1, 0));
    let (_, sent) = deliver(
        &mut router,
        &mut Recorder::default(),
        0,
        NODE_A,
        &from_a_to_9(),
        at(2),
    );
    assert_eq!(sent.len(), 1);
    forwarded_to_r7(&sent[0]);
    // t = 60 s: the sweep keeps the claim (59 s old, inside 60 s).
    router.on_deadline(at(60), &mut host);
    assert!(router.table().pending(9).is_some());
    assert_eq!(router.table().claim_counters(), claims(1, 1, 0));
    // t = 61 s: the repeat comes 60 s after the first: inside the window,
    // which is inclusive, so 9 moves to port 0 and B.
    claim(&mut router, 0, B, 61);
    let route = router.table().lookup(9).unwrap();
    assert_eq!((route.port(), route.next_hop().as_slice()), (PortId(0), B));
    assert_eq!(router.table().claim_counters(), claims(2, 1, 1));
    let (_, sent) = deliver(
        &mut router,
        &mut Recorder::default(),
        1,
        &[0x20],
        &from_port_1,
        at(62),
    );
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (0, Some(B)));
    assert_eq!(router.counters(), Counters::default());

    // Claims alternating between the ports every 10 s never move it back:
    // each of R7's starts a wait, each of B's refreshes the route.
    for (index, second) in (71..=151).step_by(10).enumerate() {
        let (learned, started) = if index % 2 == 0 {
            claim(&mut router, 1, R7, second);
            (2 + index / 2, 2 + index / 2)
        } else {
            claim(&mut router, 0, B, second);
            (3 + index / 2, 2 + index / 2)
        };
        assert_eq!(route_port(&router), PortId(0), "at {second} s");
        assert_eq!(
            router.table().claim_counters(),
            claims(learned as u64, started as u64, 1),
            "at {second} s"
        );
    }
}

/// Trace D's boundary: a repeat one nanosecond past the window is a fresh
/// first claim, not a second.
#[test]
fn d_a_repeat_past_the_window_starts_over() {
    const B: &[u8] = &[10, 0, 0, 7, 0xBA, 0xC0];
    let options = options(|o| o.convergence = Convergence::Hardened);
    let mut router = router_with(&two_ports(), options);
    let mut host = Recorder::default();
    deliver_broadcast(&mut router, &mut host, 1, R7, &i_am(&[9]), at(0));
    deliver_broadcast(&mut router, &mut host, 0, B, &i_am(&[9]), at(1));
    let late = Instant::from_nanos(at(61).as_nanos() + 1);
    deliver_broadcast(&mut router, &mut host, 0, B, &i_am(&[9]), late);
    assert_eq!(router.table().lookup(9).unwrap().port(), PortId(1));
    assert_eq!(router.table().pending(9).unwrap().first_seen, late);
    let counters = router.table().claim_counters();
    assert_eq!((counters.pending_started, counters.pending_expired), (2, 1));
    assert_eq!(counters.corroborated_applied, 0);
}

/// The startup announcements of a one-port router: its number, and no
/// I-Am-Router, since no network lies beyond another port.
#[test]
fn a_one_port_router_announces_only_its_number() {
    let mut router = router_with(&[fixture::port_on(1, OWN_0)], Options::HOSTED);
    router.start();
    let sent = drain(&mut router);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].kind,
        Kind::Announce(NetworkMessageType::NETWORK_NUMBER_IS)
    );
    assert_eq!(
        sent[0].npdu,
        message(NetworkMessageType::NETWORK_NUMBER_IS, &[0, 1, 1])
    );
}
