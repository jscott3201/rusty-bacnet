//! The router-core hardening options of #1698: next-hop checks on received
//! rejects and flow control, and duplicate global-broadcast suppression.

mod fixture;

use bacnet_router_core::npdu::{NetworkMessageType, Npdu, RejectMessageReason};
use bacnet_router_core::router::{
    Action, ConfigError, Congestion, Discard, Duplicates, Event, Frame, Groups, Options,
    Permissive, Received, Router,
};
use bacnet_router_core::table::{Reachability, RejectEffect};
use bacnet_router_core::{Duration, Instant, PortId, Storage};
use fixture::{
    addr, at, at_ms, bytes, deliver, deliver_broadcast, learn_9_via_r7, message, networks,
    reject_data, router_with, two_ports, Outcome, Recorder, TestRouter, NODE_A, R7, UNCONFIRMED,
};

/// Another router on port 1, beside R7.
const R8: &[u8] = &[0x08];

fn router(set: impl FnOnce(&mut Options)) -> TestRouter {
    let mut router = router_with(&two_ports(), fixture::options(set));
    learn_9_via_r7(&mut router, Instant::ZERO);
    router
}

/// A reject naming network 9 with `reason`, for the router itself.
fn reject_9(reason: RejectMessageReason) -> Vec<u8> {
    message(
        NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
        &reject_data(reason, 9),
    )
}

fn reachability(router: &TestRouter, network: u16) -> Option<Reachability> {
    router
        .table()
        .lookup(network)
        .map(|route| route.reachability())
}

#[test]
fn by_default_any_senders_reject_updates_the_route() {
    for (port, source) in [(1, R8), (0, NODE_A), (1, R7)] {
        let mut router = router(|_| {});
        let reject = reject_9(RejectMessageReason::NOT_DIRECTLY_CONNECTED);
        let _ = deliver(&mut router, &mut Permissive, port, source, &reject, at(1));
        assert_eq!(
            reachability(&router, 9),
            Some(Reachability::Unreachable),
            "from {source:?} on port {port}"
        );
        assert_eq!(router.counters().not_next_hop, 0);
    }
}

#[test]
fn next_hop_rejects_apply_only_from_the_routes_next_router() {
    for (port, source) in [(1, R8), (0, R7), (0, NODE_A)] {
        let mut router = router(|options| options.next_hop_rejects = true);
        let mut host = Recorder::default();
        for reason in [
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            RejectMessageReason::ROUTER_BUSY,
        ] {
            let (outcome, sent) = deliver(
                &mut router,
                &mut host,
                port,
                source,
                &reject_9(reason),
                at(1),
            );
            // Still for the router itself, as before: only the table is
            // spared.
            assert!(matches!(outcome, Outcome::ToRouter(_)));
            assert!(sent.is_empty());
        }
        assert_eq!(reachability(&router, 9), Some(Reachability::Reachable));
        assert_eq!(router.counters().not_next_hop, 2);
        assert!(host.events.iter().all(|event| matches!(
            event,
            Event::RejectReceived {
                effect: RejectEffect::NotNextHop,
                ..
            }
        )));
    }
    // From R7 on port 1, the next router, it applies.
    let mut router = router(|options| options.next_hop_rejects = true);
    let reject = reject_9(RejectMessageReason::ROUTER_BUSY);
    let _ = deliver(&mut router, &mut Permissive, 1, R7, &reject, at(1));
    assert_eq!(reachability(&router, 9), Some(Reachability::Busy));
    assert_eq!(router.counters().not_next_hop, 0);
    // A direct route no reject changes, and none is counted.
    let reject = message(
        NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
        &reject_data(RejectMessageReason::NOT_DIRECTLY_CONNECTED, 2),
    );
    let _ = deliver(&mut router, &mut Permissive, 1, R8, &reject, at(2));
    assert_eq!(router.counters().not_next_hop, 0);
}

#[test]
fn flow_control_from_another_router_is_counted_and_changes_nothing() {
    for kind in [
        NetworkMessageType::ROUTER_BUSY_TO_NETWORK,
        NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK,
    ] {
        let mut router = router(|_| {});
        // Network 9 is R7's; 2 is direct; 77 is unknown.
        let busy = message(kind, &networks(&[9, 2, 77]));
        let (_, relayed) = deliver(&mut router, &mut Permissive, 1, R8, &busy, at(1));
        assert_eq!(relayed.len(), 1, "passed on either way");
        assert_eq!(reachability(&router, 9), Some(Reachability::Reachable));
        assert_eq!(router.counters().not_next_hop, 1, "only network 9");
    }
    // From R7 it marks, and counts nothing.
    let mut router = router(|_| {});
    let busy = message(NetworkMessageType::ROUTER_BUSY_TO_NETWORK, &networks(&[9]));
    let _ = deliver(&mut router, &mut Permissive, 1, R7, &busy, at(1));
    assert_eq!(reachability(&router, 9), Some(Reachability::Busy));
    assert_eq!(router.counters().not_next_hop, 0);
}

/// A global broadcast of `apdu`, as it reaches the router from `source`
/// on `port`: from a node on that network (no SNET), or relayed with
/// `origin` as its SNET and SADR.
fn global(apdu: &[u8], origin: Option<(u16, &[u8])>) -> Vec<u8> {
    let mut npdu = Npdu::apdu(apdu);
    npdu.destination = Some(addr(0xFFFF, &[]));
    npdu.source = origin.map(|(network, mac)| addr(network, mac));
    bytes(&npdu)
}

fn dup_router() -> TestRouter {
    router(|options| options.duplicates = Some(Duplicates::DEFAULT))
}

#[test]
fn duplicates_are_kept_by_default() {
    let mut router = router(|_| {});
    let first = global(UNCONFIRMED, None);
    let looped = global(UNCONFIRMED, Some((1, NODE_A)));
    for (port, source, frame) in [(0, NODE_A, &first), (1, R7, &looped), (0, NODE_A, &first)] {
        let (outcome, sent) =
            deliver_broadcast(&mut router, &mut Permissive, port, source, frame, at(1));
        assert!(matches!(outcome, Outcome::Local { .. }));
        assert_eq!(sent.len(), 1);
    }
    assert_eq!(router.counters().duplicate_drops, 0);
}

#[test]
fn a_looped_copy_within_the_ttl_is_dropped() {
    let mut router = dup_router();
    // Node A's Who-Is, then the same come back round a loop through R7
    // with node A as its origin.
    let first = global(UNCONFIRMED, None);
    let (outcome, sent) =
        deliver_broadcast(&mut router, &mut Permissive, 0, NODE_A, &first, at(10));
    assert!(matches!(outcome, Outcome::Local { .. }));
    assert_eq!(sent.len(), 1);
    let looped = global(UNCONFIRMED, Some((1, NODE_A)));
    let (outcome, sent) =
        deliver_broadcast(&mut router, &mut Permissive, 1, R7, &looped, at_ms(10_999));
    assert_eq!(outcome, Outcome::Discarded(Discard::Duplicate));
    assert!(sent.is_empty(), "not relayed");
    assert_eq!(router.counters().duplicate_drops, 1);
    // At the time to live it is new again, and remembered from then.
    let (outcome, _) = deliver_broadcast(&mut router, &mut Permissive, 1, R7, &looped, at(11));
    assert!(matches!(outcome, Outcome::Local { .. }));
    let (outcome, _) = deliver_broadcast(
        &mut router,
        &mut Permissive,
        0,
        NODE_A,
        &first,
        at_ms(11_500),
    );
    assert_eq!(outcome, Outcome::Discarded(Discard::Duplicate));
}

#[test]
fn another_origin_or_payload_is_no_duplicate() {
    let mut router = dup_router();
    let other_node = [10, 0, 0, 7, 0xBA, 0xC0];
    let i_am = [0x10, 0x00, 0xC4, 0x02, 0x00, 0x00, 0x05];
    for (index, frame) in [
        global(UNCONFIRMED, None),
        global(&i_am, None),
        global(UNCONFIRMED, Some((5, NODE_A))),
        global(UNCONFIRMED, Some((1, &other_node))),
    ]
    .iter()
    .enumerate()
    {
        let (outcome, _) = deliver_broadcast(&mut router, &mut Permissive, 0, NODE_A, frame, at(1));
        assert!(matches!(outcome, Outcome::Local { .. }), "frame {index}");
    }
    // Only global broadcasts are checked: a local one passes twice.
    let local = bytes(&Npdu::apdu(UNCONFIRMED));
    for _ in 0..2 {
        let (outcome, _) =
            deliver_broadcast(&mut router, &mut Permissive, 0, NODE_A, &local, at(1));
        assert!(matches!(outcome, Outcome::Local { .. }));
    }
    assert_eq!(router.counters().duplicate_drops, 0);
}

#[test]
fn the_record_forgets_its_oldest_when_full() {
    let mut router = dup_router();
    let frames: Vec<Vec<u8>> = (0..33u8).map(|n| global(&[0x10, 0x08, n], None)).collect();
    for frame in &frames {
        let (outcome, _) = deliver_broadcast(&mut router, &mut Permissive, 0, NODE_A, frame, at(1));
        assert!(matches!(outcome, Outcome::Local { .. }));
    }
    // 33 into 32 slots: the first is gone, the last still held.
    let (first, _) = deliver_broadcast(&mut router, &mut Permissive, 0, NODE_A, &frames[0], at(1));
    assert!(matches!(first, Outcome::Local { .. }));
    let (last, _) = deliver_broadcast(&mut router, &mut Permissive, 0, NODE_A, &frames[32], at(1));
    assert_eq!(last, Outcome::Discarded(Discard::Duplicate));
}

#[test]
fn a_record_of_zero_refuses_duplicates_and_keeps_none() {
    type NoRecord = Storage<2, 8, 8, 4, 1497, Groups, 0>;
    let mut storage = NoRecord::new();
    let on = fixture::options(|options| {
        options.route_cap = None;
        options.duplicates = Some(Duplicates::DEFAULT);
    });
    assert_eq!(
        Router::new(&mut storage, &two_ports(), on, Instant::ZERO).err(),
        Some(ConfigError::NoDuplicateRecord)
    );
    // Off, it builds; and the same broadcast passes every time.
    let off = fixture::options(|options| options.route_cap = None);
    let mut router = Router::new(&mut storage, &two_ports(), off, Instant::ZERO).unwrap();
    let frame = global(UNCONFIRMED, None);
    for _ in 0..3 {
        let received = router.receive(
            Frame {
                port: PortId(0),
                source: NODE_A,
                link_group: true,
                npdu: &frame,
            },
            at(1),
            &mut Permissive,
        );
        assert!(matches!(received, Received::Local(_)));
        while router.poll_action() != Action::Idle {}
    }
}

#[test]
fn router_new_refuses_a_congestion_policy_it_cannot_run() {
    let base = Congestion::DEFAULT;
    let mut cases = Vec::new();
    for percent in [0, 100, 255] {
        let mut policy = base;
        policy.drain_below_percent = percent;
        cases.push(policy);
    }
    let mut policy = base;
    policy.repeat = Duration::ZERO;
    cases.push(policy);
    let mut policy = base;
    policy.clog_after = Duration::ZERO;
    cases.push(policy);
    for policy in cases {
        assert!(!policy.is_valid(), "{policy:?}");
        let options = fixture::options(|options| options.congestion = Some(policy));
        let refused = Router::new(
            Box::new(fixture::Store::new()),
            &two_ports(),
            options,
            Instant::ZERO,
        );
        assert_eq!(refused.err(), Some(ConfigError::Congestion), "{policy:?}");
    }
    for percent in [1, 50, 99] {
        let mut policy = base;
        policy.drain_below_percent = percent;
        let options = fixture::options(|options| options.congestion = Some(policy));
        let _ = router_with(&two_ports(), options);
    }
}
