//! What a repeated I-Am-Router still teaches, with duplicate suppression
//! on (#2713).
//!
//! The record of relayed messages (#1810) keeps an I-Am-Router by its data
//! alone, so that the copy a loop brings back on another port is known for
//! a repeat. A repeat is never passed on. Until #2713 it wasn't learned
//! from either, so a node that sent a router's list first kept that
//! router's own announcement out for one time to live. Now a repeat is
//! learned from where that keeps a route on its port, and only there: the
//! copy a loop brings back would otherwise move the route round the loop
//! (`loops.rs` has the ring). So a first sender on another port than the
//! router's keeps the route for longer than one time to live; the
//! `by_design_` test pins that known limit.

mod fixture;

use bacnet_router_core::PortId;
use bacnet_router_core::npdu::RejectMessageReason;
use bacnet_router_core::router::{ControlClass, Discard, Duplicates, Event, Options};
use bacnet_router_core::table::{Convergence, Reachability};
use fixture::{
    NODE_A, Outcome, R7, Recorder, TestRouter, at, at_ms, deliver_broadcast, i_am, message,
    options, reject_data, router_with, two_ports,
};

/// A station on port 1, beside R7, that is not the router it claims to be.
const OTHER: &[u8] = &[0x4E];

/// Hosted options with duplicate suppression on and what `set` changes.
fn suppressing(set: impl FnOnce(&mut Options)) -> Options {
    options(|options| {
        options.duplicates = Some(Duplicates::DEFAULT);
        set(options);
    })
}

/// An I-Am-Router for `list` from `sender` on `port` at `millis`: whether
/// it was dropped as a repeat, and how many relays went out.
fn announce(
    router: &mut TestRouter,
    host: &mut Recorder,
    port: u8,
    sender: &[u8],
    list: &[u16],
    millis: u64,
) -> (bool, usize) {
    let (outcome, sent) = deliver_broadcast(router, host, port, sender, &i_am(list), at_ms(millis));
    let repeat = outcome == Outcome::Discarded(Discard::Duplicate);
    assert!(repeat || outcome == Outcome::Handled, "{outcome:?}");
    (repeat, sent.len())
}

/// Where the table has `network`: its port and next router.
fn via(router: &TestRouter, network: u16) -> Option<(u8, Vec<u8>)> {
    let route = router.table().lookup(network)?;
    Some((route.port().0, route.next_hop().as_slice().to_vec()))
}

/// The issue's case: a station that is not the router announces the
/// router's list first. The router's own identical announcement, inside
/// the time to live, is still not passed on, but its claim is learned:
/// the route's next router is the real one.
#[test]
fn a_first_identical_list_does_not_keep_the_real_routers_claim_out() {
    let mut router = router_with(&two_ports(), suppressing(|_| {}));
    let mut host = Recorder::default();
    assert_eq!(
        announce(&mut router, &mut host, 1, OTHER, &[9, 10], 0),
        (false, 1)
    );
    assert_eq!(via(&router, 9), Some((1, OTHER.to_vec())));

    assert_eq!(
        announce(&mut router, &mut host, 1, R7, &[9, 10], 10),
        (true, 0)
    );
    for network in [9, 10] {
        assert_eq!(via(&router, network), Some((1, R7.to_vec())), "{network}");
        let route = router.table().lookup(network).unwrap();
        assert_eq!(route.last_seen(), Some(at_ms(10)));
        assert_eq!(route.flap_count(), 0, "a move within a port is no flap");
    }
    assert_eq!(router.counters().duplicate_drops, 1);
    assert_eq!(router.table().claim_counters().learned_ok, 4);
    let learned = |event: &&Event| matches!(event, Event::Learned { .. });
    assert_eq!(host.events.iter().filter(learned).count(), 4);
    assert_eq!(host.asked.len(), 2, "the host is asked about a repeat too");
}

/// The loop's case, on one router: the same list on another port inside
/// the time to live moves no route there, in either convergence mode, and
/// starts no pending claim; once the time to live has passed it is a fresh
/// announcement, taken by the table's own rules and passed on.
#[test]
fn a_repeat_on_another_port_moves_no_route() {
    for convergence in [Convergence::Standard, Convergence::Hardened] {
        let mut router = router_with(&two_ports(), suppressing(|o| o.convergence = convergence));
        let mut host = Recorder::default();
        assert_eq!(announce(&mut router, &mut host, 1, R7, &[9], 0), (false, 1));
        // Back and forth, as copies going round a loop would come.
        for (millis, port, sender) in [(1, 0, NODE_A), (2, 1, R7), (3, 0, NODE_A), (999, 0, NODE_A)]
        {
            assert_eq!(
                announce(&mut router, &mut host, port, sender, &[9], millis),
                (true, 0),
                "{convergence:?} at {millis} ms"
            );
            assert_eq!(via(&router, 9), Some((1, R7.to_vec())), "{convergence:?}");
        }
        let route = router.table().lookup(9).unwrap();
        assert_eq!(route.flap_count(), 0, "{convergence:?}");
        assert_eq!(route.last_seen(), Some(at_ms(2)), "R7's own repeat");
        assert_eq!(router.table().pending(9), None, "{convergence:?}");
        assert_eq!(router.table().claim_counters().pending_started, 0);
        assert!(
            !host
                .events
                .iter()
                .any(|e| matches!(e, Event::Flapping { .. }))
        );
        assert_eq!(router.counters().duplicate_drops, 4);

        // Past the time to live, the table's rules decide as they always
        // have: at once, or after a second claim.
        assert_eq!(
            announce(&mut router, &mut host, 0, NODE_A, &[9], 1_000),
            (false, 1)
        );
        let moved = convergence == Convergence::Standard;
        let expected = if moved { (0, NODE_A) } else { (1, R7) };
        assert_eq!(via(&router, 9), Some((expected.0, expected.1.to_vec())));
        assert_eq!(router.table().pending(9).is_some(), !moved);
    }
}

/// Entry by entry: in a repeat on another port, a network the table
/// reaches elsewhere stays where it is, and one the table has lost is
/// added, as any claim for a new network is.
#[test]
fn a_repeat_still_adds_a_network_the_table_lacks() {
    let mut router = router_with(&two_ports(), suppressing(|_| {}));
    let mut host = Recorder::default();
    assert_eq!(
        announce(&mut router, &mut host, 1, R7, &[9, 10], 0),
        (false, 1)
    );
    assert!(router.table_mut().remove(10).is_some());

    assert_eq!(
        announce(&mut router, &mut host, 0, NODE_A, &[9, 10], 5),
        (true, 0)
    );
    assert_eq!(via(&router, 9), Some((1, R7.to_vec())), "kept on its port");
    assert_eq!(via(&router, 10), Some((0, NODE_A.to_vec())), "new");
    assert_eq!(
        host.events.last(),
        Some(&Event::Learned {
            network: 10,
            port: PortId(0),
            class: ControlClass::IAm
        })
    );
}

/// What a repeat on the route's own port newly does: it refreshes the
/// route as any claim there does, so it lifts an unreachable mark set
/// since the first copy, each time it is heard, with no relay.
#[test]
fn a_repeat_on_the_routes_port_refreshes_it_each_time() {
    let mut router = router_with(&two_ports(), suppressing(|_| {}));
    let mut host = Recorder::default();
    assert_eq!(announce(&mut router, &mut host, 1, R7, &[9], 0), (false, 1));
    let state =
        |router: &TestRouter, millis| router.table().effective_reachability(9, at_ms(millis));
    for millis in [100, 400, 700] {
        let reject = message(
            bacnet_router_core::npdu::NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
            &reject_data(RejectMessageReason::NOT_DIRECTLY_CONNECTED, 9),
        );
        // From R7, the route's next router, to the router itself.
        let _ = fixture::deliver(&mut router, &mut host, 1, R7, &reject, at_ms(millis));
        let marked = state(&router, millis);
        assert_eq!(
            announce(&mut router, &mut host, 1, R7, &[9], millis + 50),
            (true, 0)
        );
        assert_eq!(state(&router, millis + 50), Some(Reachability::Reachable));
        let route = router.table().lookup(9).unwrap();
        assert_eq!(route.last_seen(), Some(at_ms(millis + 50)));
        // The hold-down a reject starts is lifted by fresh learning, so
        // each reject in turn marks the route again.
        assert_eq!(marked, Some(Reachability::Unreachable), "at {millis} ms");
    }
    assert_eq!(router.counters().duplicate_drops, 3);
}

/// A known limit, kept by decision (#2713): when another station sends a
/// router's list first on one port and the router sends the identical list
/// on a different port inside the time to live, the router's copy is a
/// repeat from another port, which can't be told from a looped copy, so it
/// moves nothing. The route stays with the first sender past the time to
/// live, until the router announces again (here, under standard
/// convergence, it moves at once) or the route goes unconfirmed for the
/// learned age, 300 s, and the sweep purges it.
#[test]
fn by_design_a_first_list_on_another_port_holds_the_route_past_the_time_to_live() {
    let held = || {
        let mut router = router_with(&two_ports(), suppressing(|_| {}));
        let mut host = Recorder::default();
        assert_eq!(
            announce(&mut router, &mut host, 0, NODE_A, &[9], 0),
            (false, 1)
        );
        assert_eq!(announce(&mut router, &mut host, 1, R7, &[9], 10), (true, 0));
        assert_eq!(via(&router, 9), Some((0, NODE_A.to_vec())));
        // Well past the 1 s time to live, the route is still the first
        // sender's: the repeat refreshed nothing and moved nothing.
        router.on_deadline(at(240), &mut host);
        assert_eq!(via(&router, 9), Some((0, NODE_A.to_vec())));
        let route = router.table().lookup(9).unwrap();
        assert_eq!(route.last_seen(), Some(at_ms(0)));
        (router, host)
    };

    // The router announcing again, now a fresh message, moves the route.
    let (mut router, mut host) = held();
    assert_eq!(
        announce(&mut router, &mut host, 1, R7, &[9], 240_000),
        (false, 1)
    );
    assert_eq!(via(&router, 9), Some((1, R7.to_vec())));

    // Without that, the route ages out with the learned age.
    let (mut router, mut host) = held();
    router.on_deadline(at(360), &mut host);
    assert_eq!(via(&router, 9), None);
    assert!(host.events.contains(&Event::Purged { network: 9 }));
}
