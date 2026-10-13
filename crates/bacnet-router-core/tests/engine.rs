//! The engine's own contract: configuration, the poll rule, refusals, the
//! sweep's schedule, and what random frames do to it.

mod fixture;

use bacnet_router_core::message::MessageType;
use bacnet_router_core::router::{
    Action, ConfigError, Discard, Event, Frame, Limits, Options, Permissive, Received, SendRefusal,
};
use bacnet_router_core::{Duration, Instant, Mac, PortId, Router, Storage};
use fixture::{
    apdu_to, at, deliver, drain, learn_9_via_r7, options, port_on, router, two_ports, Outcome,
    Recorder, Store, CONFIRMED, NODE_A, OWN_0, OWN_1, R7,
};
use proptest::prelude::*;

/// Storage for firmware: two ports, eight routes, four searches. It can be
/// a `static`, which `new` being `const` allows.
type Small = Storage<2, 8, 8, 4>;
static _SMALL: Small = Small::new();

#[test]
fn new_refuses_what_it_cannot_route() {
    let new = |ports: &[_], options| {
        Router::new(Box::new(Store::new()), ports, options, Instant::ZERO).map(|_| ())
    };
    let hosted = Options::HOSTED;
    assert_eq!(new(&[], hosted), Err(ConfigError::NoPorts));
    let five = [1, 2, 3, 4, 5].map(|network| port_on(network, &[network as u8]));
    assert_eq!(
        new(&five, hosted),
        Err(ConfigError::TooManyPorts {
            count: 5,
            capacity: 4
        })
    );
    assert_eq!(
        new(&[port_on(1, OWN_0), port_on(1, OWN_1)], hosted),
        Err(ConfigError::DuplicateNetwork(1))
    );
    for limits in [
        Limits::both(63),
        Limits::both(1498),
        Limits {
            unicast: 1497,
            broadcast: 10,
        },
    ] {
        let mut ports = two_ports();
        ports[1].max_npdu = limits;
        assert_eq!(new(&ports, hosted), Err(ConfigError::Limit(PortId(1))));
    }
    let mut ports = two_ports();
    ports[0].max_npdu = Limits::both(64);
    assert_eq!(new(&ports, hosted), Ok(()));
    let over = options(|o| o.route_cap = 257);
    assert_eq!(
        new(&two_ports(), over),
        Err(ConfigError::RouteCap {
            cap: 257,
            capacity: 256
        })
    );
    let still = options(|o| o.sweep = Duration::ZERO);
    assert_eq!(new(&two_ports(), still), Err(ConfigError::ZeroSweep));

    let mut storage = Storage::<2, 1, 1>::new();
    let options = options(|o| o.route_cap = 1);
    let refused = Router::new(&mut storage, &two_ports(), options, Instant::ZERO).map(|_| ());
    assert_eq!(refused, Err(ConfigError::TableTooSmall));
}

/// A router built on used storage starts clean.
#[test]
fn new_clears_the_storage_it_is_given() {
    let mut storage = Box::new(Small::new());
    let options = options(|o| o.route_cap = 8);
    let mut router = Router::new(&mut *storage, &two_ports(), options, at(0)).unwrap();
    router
        .table_mut()
        .add_learned(9, PortId(1), Mac::from_slice(R7).unwrap(), at(0));
    let frame = apdu_to(44, &[5], CONFIRMED);
    let received = router.receive(
        Frame {
            port: PortId(0),
            source: NODE_A,
            link_group: false,
            npdu: &frame,
        },
        at(1),
        &mut Permissive,
    );
    assert_eq!(received, Received::Handled);
    let router = Router::new(&mut *storage, &two_ports(), options, at(2)).unwrap();
    assert_eq!(router.table().networks().collect::<Vec<_>>(), [1, 2]);
    assert!(router.discovery().is_empty());
    assert_eq!(router.counters(), Default::default());
    assert_eq!(router.ports(), two_ports());
}

/// The poll rule: a call made before the last call's actions were polled
/// drops them and counts them.
#[test]
fn undrained_actions_are_dropped_and_counted() {
    let mut router = router();
    let mut host = Recorder::default();
    router.start();
    // The startup announcements are never polled.
    let frame = apdu_to(44, &[5], CONFIRMED);
    let _ = router.receive(
        Frame {
            port: PortId(0),
            source: NODE_A,
            link_group: false,
            npdu: &frame,
        },
        at(1),
        &mut host,
    );
    assert_eq!(router.counters().undrained, 1);
    // The search and the reject, polled halfway, then dropped by the next.
    assert!(matches!(router.poll_action(), Action::Send(_)));
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &frame, at(2));
    assert_eq!(router.counters().undrained, 2);
    assert_eq!(sent.len(), 1, "only the second reject");
}

/// A refusal is counted; a port refusing for length counts as an egress
/// limit too.
#[test]
fn refusals_are_counted() {
    let mut router = router();
    let mut host = Recorder::default();
    for why in [
        SendRefusal::Full,
        SendRefusal::TooLong,
        SendRefusal::PortDown,
    ] {
        let frame = apdu_to(2, &[5], CONFIRMED);
        let _ = router.receive(
            Frame {
                port: PortId(0),
                source: NODE_A,
                link_group: false,
                npdu: &frame,
            },
            at(1),
            &mut host,
        );
        assert!(matches!(router.poll_action(), Action::Send(_)));
        router.refused(why);
        assert_eq!(router.poll_action(), Action::Idle);
    }
    let counters = router.counters();
    assert_eq!((counters.refused, counters.egress_too_long), (3, 1));
}

/// The sweep runs when due and not before; a late call runs it once and
/// schedules the next a period after the call.
#[test]
fn the_sweep_keeps_its_schedule() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    assert_eq!(router.next_deadline(), Some(at(60)));
    router.on_deadline(at(59), &mut host);
    assert_eq!(router.next_deadline(), Some(at(60)));
    router.on_deadline(at(60), &mut host);
    assert_eq!(router.next_deadline(), Some(at(120)));
    // 300 s exactly is not past the age; the purge needs more.
    router.on_deadline(at(300), &mut host);
    assert!(router.table().lookup(9).is_some());
    assert_eq!(router.next_deadline(), Some(at(360)));
    // Called long after: one sweep, then a period from now.
    router.on_deadline(at(1_000), &mut host);
    assert!(router.table().lookup(9).is_none());
    assert_eq!(host.events, [Event::Purged { network: 9 }]);
    assert_eq!(router.next_deadline(), Some(at(1_060)));
}

/// A call exactly a period late runs the sweep once and schedules the next
/// a period after the call.
#[test]
fn a_sweep_a_period_late_is_rescheduled_from_the_call() {
    let mut router = router();
    router.on_deadline(at(120), &mut Permissive);
    assert_eq!(router.next_deadline(), Some(at(180)));
}

/// The discovery cooldown is 5 s exactly: a request 4.999 s after a search
/// draws only the reject, and one at 5 s searches again.
#[test]
fn the_discovery_cooldown_is_five_seconds() {
    let mut router = router();
    let mut host = Recorder::default();
    let frame = apdu_to(44, &[5], CONFIRMED);
    let mut solicits = Vec::new();
    for now in [Instant::from_secs(1), Instant::from_millis(5_999), at(6)] {
        let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &frame, now);
        solicits.push(sent.len() == 2);
    }
    assert_eq!(solicits, [true, false, true]);
    assert_eq!(router.counters().solicitations, 2);
}

/// The sweep clears a lapsed busy mark from storage; the forwarding
/// decision never waited for it.
#[test]
fn the_sweep_clears_lapsed_busy_marks() {
    let mut router = router();
    learn_9_via_r7(&mut router, at(0));
    router.table_mut().mark_busy(9, at(30));
    router.on_deadline(at(60), &mut Permissive);
    assert_eq!(router.table().lookup(9).unwrap().busy_until(), None);
}

/// A route written locally on a port the router doesn't have carries
/// nothing, as the hosted router drops such a route's traffic
/// (router/forwarding.rs:76-82).
#[test]
fn a_route_to_a_missing_port_carries_nothing() {
    let mut router = router();
    let mut host = Recorder::default();
    let peer = Mac::from_slice(R7).unwrap();
    assert!(router.table_mut().add_learned(9, PortId(3), peer, at(0)));
    let frame = apdu_to(9, &[5], CONFIRMED);
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &frame, at(1));
    assert_eq!(outcome, Outcome::Discarded(Discard::UnknownPort));
    assert!(sent.is_empty());
    assert_eq!(router.counters().unknown_port, 1);
}

/// Cancelled, discovery searches no more; the reject still goes.
#[test]
fn cancelled_discovery_still_rejects() {
    let mut router = router();
    let mut host = Recorder::default();
    router.cancel_discovery();
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(44, &[5], CONFIRMED),
        at(1),
    );
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent.len(), 1);
    assert_eq!(router.counters().solicitations, 0);
}

/// The discovery cooldown is 5 s per network, and up to `D` networks are
/// searched for at once.
#[test]
fn discovery_is_bounded() {
    let mut storage = Box::new(Small::new());
    let mut router = Router::new(
        &mut *storage,
        &two_ports(),
        options(|o| o.route_cap = 8),
        at(0),
    )
    .unwrap();
    let mut solicited = 0;
    for network in 40..46 {
        let frame = apdu_to(network, &[5], CONFIRMED);
        let _ = router.receive(
            Frame {
                port: PortId(0),
                source: NODE_A,
                link_group: false,
                npdu: &frame,
            },
            at(1),
            &mut Permissive,
        );
        while let Action::Send(send) = router.poll_action() {
            if send.npdu[2] == MessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw() {
                solicited += 1;
            }
        }
    }
    assert_eq!(solicited, 4);
    assert_eq!(router.counters().rejects, 6);
}

/// One frame, as the property test makes them.
fn frame() -> impl Strategy<Value = (u8, Vec<u8>, bool, Vec<u8>)> {
    let source = prop_oneof![
        Just(NODE_A.to_vec()),
        Just(R7.to_vec()),
        proptest::collection::vec(any::<u8>(), 0..20),
    ];
    let npdu = prop_oneof![
        proptest::collection::vec(any::<u8>(), 0..40),
        (any::<u8>(), proptest::collection::vec(any::<u8>(), 0..30)).prop_map(|(kind, data)| {
            let mut out = vec![1, 0x80, kind];
            if kind >= 0x80 {
                out.extend_from_slice(&[0, 7]);
            }
            out.extend(data);
            out
        }),
        (
            prop_oneof![Just(1u16), Just(2), Just(9), Just(44), Just(0xFFFF)],
            any::<u8>(),
            any::<bool>()
        )
            .prop_map(|(dnet, hops, unconfirmed)| {
                let apdu: &[u8] = if unconfirmed {
                    &[0x10, 0x08]
                } else {
                    CONFIRMED
                };
                let dadr: &[u8] = if dnet == 0xFFFF { &[] } else { &[5] };
                let mut out = apdu_to(dnet, dadr, apdu);
                out[2 + 3 + dadr.len()] = hops;
                out
            }),
    ];
    (0u8..3, source, any::<bool>(), npdu)
}

proptest! {
    /// Random frames never panic the engine, every call drains to idle,
    /// the queue never overflows (its debug assertion would fire), and
    /// each drop is counted under its reason.
    #[test]
    fn random_frames_are_handled_and_counted(frames in proptest::collection::vec(frame(), 1..40)) {
        let mut router = router();
        let mut host = Recorder::default();
        learn_9_via_r7(&mut router, at(0));
        router.start();
        drain(&mut router);
        let mut dropped = std::collections::HashMap::new();
        for (index, (port, source, group, npdu)) in frames.iter().enumerate() {
            let now = at(index as u64);
            router.on_deadline(now, &mut host);
            let received = router.receive(
                Frame { port: PortId(*port), source, link_group: *group, npdu },
                now,
                &mut host,
            );
            if let Received::Discarded(reason) = received {
                let key = match reason {
                    Discard::SourceTooLong { .. } | Discard::AddressTooLong { .. } => "address",
                    Discard::Malformed(_) => "malformed",
                    other => match other {
                        Discard::UnknownPort => "port",
                        Discard::GlobalWithDadr => "global",
                        Discard::BroadcastNotUnconfirmed => "broadcast",
                        Discard::GroupDadr => "group",
                        Discard::Denied => "denied",
                        Discard::BadMessage => "bad",
                        Discard::Unroutable => "unroutable",
                        _ => "other",
                    },
                };
                *dropped.entry(key).or_insert(0u64) += 1;
            }
            drain(&mut router);
        }
        let counters = router.counters();
        let count = |key| dropped.get(key).copied().unwrap_or(0);
        prop_assert_eq!(counters.address_length_drops, count("address"));
        prop_assert_eq!(counters.malformed, count("malformed"));
        prop_assert_eq!(counters.unknown_port, count("port"));
        prop_assert_eq!(counters.global_broadcast_dadr_drops, count("global"));
        prop_assert_eq!(counters.broadcast_pdu_type_drops, count("broadcast"));
        prop_assert_eq!(counters.group_dadr_drops, count("group"));
        prop_assert_eq!(counters.denied, count("denied"));
        prop_assert_eq!(counters.bad_messages, count("bad"));
        prop_assert_eq!(counters.unroutable_rejects, count("unroutable"));
        prop_assert_eq!(counters.undrained, 0);
    }
}
