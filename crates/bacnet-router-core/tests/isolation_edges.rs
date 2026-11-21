//! Port isolation at its edges (#2684): the route cap, the duplicate
//! record, a route that is busy or unreachable, an origin on the other
//! isolated port's network, a group address, and one isolated port alone.
//!
//! The router has four ports: port 0 on network 1 and port 3 on network 4
//! are ordinary; port 1 on network 2 and port 2 on network 3 are isolated.

mod fixture;

use bacnet_router_core::PortId;
use bacnet_router_core::npdu::{GLOBAL_NETWORK, Mac, NetworkMessageType as Msg, Npdu, NpduLimits};
use bacnet_router_core::router::{
    ControlClass, Discard, Duplicates, Event, Group, GroupClassifier, Kind, Options, Permissive,
    PortConfig,
};
use fixture::*;

/// A node on whichever port a test delivers from.
const NODE: &[u8] = &[0x55];
/// A router on ordinary port 0, the next hop to network 50.
const R9: &[u8] = &[0x09];

fn ports() -> [PortConfig; 4] {
    let mut ports = [
        port_on(1, &[1]),
        port_on(2, &[2]),
        port_on(3, &[3]),
        port_on(4, &[4]),
    ];
    ports[1].isolated = true;
    ports[2].isolated = true;
    ports
}

/// Network 9 behind R7 on isolated port 1, network 50 behind R9 on
/// ordinary port 0.
fn with_routes(options: Options) -> TestRouter {
    let mut router = router_with(&ports(), options);
    learn_9_via_r7(&mut router, at(0));
    let r9 = Mac::from_slice(R9).unwrap();
    assert!(router.table_mut().add_learned(50, PortId(0), r9, at(0)));
    router
}

fn refused(network: u16) -> Event {
    Event::IsolationRefused {
        network,
        port: PortId(1),
        class: ControlClass::IAm,
    }
}

fn egress(sent: &[Sent]) -> Vec<u8> {
    sent.iter().map(|sent| sent.port).collect()
}

/// At the route cap the learning of a message ends at its first new
/// network. A claim after that one which isolation refuses still keeps
/// the whole message from being passed on, and is counted and reported.
#[test]
fn the_route_cap_does_not_hide_a_refused_claim() {
    // Four direct routes and two learned ones: the table is at its cap.
    let mut router = with_routes(options(|options| options.route_cap = Some(6)));
    let mut host = Recorder::default();
    let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &i_am(&[70, 50, 9]), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert!(sent.is_empty(), "{sent:?}");
    assert!(router.table().lookup(70).is_none());
    assert_eq!(router.table().lookup(50).unwrap().port(), PortId(0));
    assert_eq!(
        host.events,
        [
            Event::RouteCap {
                class: ControlClass::IAm
            },
            refused(50)
        ]
    );
    assert_eq!(
        router.counters(),
        router_counts(|counts| counts.isolated_refusals = 1)
    );
    // With nothing to refuse, the capped message is passed on as before.
    let (_, sent) = deliver(&mut router, &mut host, 1, R7, &i_am(&[70, 9]), at(1));
    assert_eq!(egress(&sent), [0, 3]);
}

/// Each list entry is looked at once, on either side of the entry where
/// the route cap ends the learning: a refusal before it is counted by the
/// claim, one after it by the scan of the rest, and none twice.
#[test]
fn refusals_on_both_sides_of_the_cap_break_count_once_each() {
    let mut router = with_routes(options(|options| options.route_cap = Some(6)));
    let mut host = Recorder::default();
    // 50 is on an ordinary port, 70 is new and meets the cap, 4 is
    // directly connected on another port.
    let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &i_am(&[50, 70, 4]), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert!(sent.is_empty(), "{sent:?}");
    assert_eq!(
        host.events,
        [
            refused(50),
            Event::RouteCap {
                class: ControlClass::IAm
            },
            refused(4)
        ]
    );
    assert_eq!(
        router.counters(),
        router_counts(|counts| counts.isolated_refusals = 2)
    );
}

/// The count is per list entry: a refused network listed twice counts
/// twice, and is reported twice.
#[test]
fn a_refused_network_listed_twice_counts_twice() {
    let mut router = with_routes(Options::HOSTED);
    let mut host = Recorder::default();
    let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &i_am(&[50, 50]), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert!(sent.is_empty(), "{sent:?}");
    assert_eq!(host.events, [refused(50), refused(50)]);
    assert_eq!(router.counters().isolated_refusals, 2);
}

/// With one isolated port alone, duplicate suppression is as without
/// isolation: a repeat from that port is dropped as a duplicate, even one
/// that names a network on another port.
#[test]
fn duplicates_with_a_single_isolated_port_are_dropped_as_ever() {
    let mut ports = ports();
    ports[2].isolated = false;
    let mut router = router_with(
        &ports,
        options(|options| options.duplicates = Some(Duplicates::DEFAULT)),
    );
    let r9 = Mac::from_slice(R9).unwrap();
    assert!(router.table_mut().add_learned(50, PortId(0), r9, at(0)));
    let mut host = Recorder::default();
    let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &i_am(&[50, 70]), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(egress(&sent), [0, 2, 3]);
    let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &i_am(&[50, 70]), at(1));
    assert_eq!(outcome, Outcome::Discarded(Discard::Duplicate));
    assert!(sent.is_empty());
    assert_eq!(
        router.counters(),
        router_counts(|counts| counts.duplicate_drops = 1)
    );
}

/// A refused I-Am-Router isn't remembered as a relayed one: the same list
/// from the network's real router, inside the duplicate time to live, is
/// taken and passed on.
#[test]
fn a_refused_claim_does_not_suppress_the_real_routers_announcement() {
    let mut router = with_routes(options(|options| {
        options.duplicates = Some(Duplicates::DEFAULT);
    }));
    let mut host = Recorder::default();
    for count in 1..=2 {
        let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &i_am(&[50]), at(1));
        assert_eq!(outcome, Outcome::Handled);
        assert!(sent.is_empty(), "{sent:?}");
        assert_eq!(router.counters().isolated_refusals, count);
    }
    let (outcome, sent) = deliver(&mut router, &mut host, 0, R9, &i_am(&[50]), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(egress(&sent), [1, 2, 3]);
    for relay in &sent {
        assert!(relay.is_broadcast());
        assert_eq!(relay.npdu, [0x01, 0x80, 0x01, 0x00, 0x32]);
    }
    assert_eq!(router.counters().duplicate_drops, 0);
    // The real router's own repeat is a duplicate, as without isolation.
    let (outcome, sent) = deliver(&mut router, &mut host, 0, R9, &i_am(&[50]), at(1));
    assert_eq!(outcome, Outcome::Discarded(Discard::Duplicate));
    assert!(sent.is_empty());
}

/// A route behind the other isolated port that is busy or unreachable
/// still answers reason 1, never the route's own state: the sender is told
/// nothing about a network it may not see.
#[test]
fn a_busy_or_unreachable_blocked_route_still_draws_reason_1() {
    let mut router = with_routes(Options::HOSTED);
    let reject = [0x01, 0x80, 0x03, 0x01, 0x00, 0x09];
    let frame = apdu_to(9, &[7], CONFIRMED);
    router.table_mut().mark_busy(9, at(100));
    let (outcome, sent) = deliver(&mut router, &mut Permissive, 2, NODE, &frame, at(1));
    assert_eq!(outcome, Outcome::Discarded(Discard::Isolated));
    assert_eq!(sent.len(), 1);
    assert_eq!(
        (sent[0].port, sent[0].to.as_deref(), sent[0].npdu.as_slice()),
        (2, Some(NODE), &reject[..])
    );
    router.table_mut().mark_unreachable(9);
    let (outcome, sent) = deliver(&mut router, &mut Permissive, 2, NODE, &frame, at(1));
    assert_eq!(outcome, Outcome::Discarded(Discard::Isolated));
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].npdu, reject);
    assert_eq!(
        router.counters(),
        router_counts(|counts| {
            counts.isolated_drops = 2;
            counts.rejects = 2;
        })
    );
}

/// A blocked frame whose SNET is a network of the other isolated port
/// draws no reject at all: its origin would be reached through that port.
#[test]
fn an_origin_on_the_other_isolated_ports_network_gets_no_reject() {
    let mut router = with_routes(Options::HOSTED);
    let mut frame = Npdu::apdu(CONFIRMED);
    frame.destination = Some(addr(2, &[7]));
    frame.source = Some(addr(2, &[9]));
    let (outcome, sent) = deliver(&mut router, &mut Permissive, 2, NODE, &bytes(&frame), at(1));
    assert_eq!(outcome, Outcome::Discarded(Discard::Isolated));
    assert!(sent.is_empty(), "{sent:?}");
    assert_eq!(
        router.counters(),
        router_counts(|counts| counts.isolated_drops = 1)
    );
}

/// Port 1's link has a broadcast address and a group it refuses.
#[derive(Clone)]
struct Marked;

impl GroupClassifier for Marked {
    const NONE: Self = Self;

    fn classify(&self, mac: &[u8]) -> Group {
        match mac {
            [0xFF] => Group::Broadcast,
            [0xFE] => Group::Refuse,
            _ => Group::No,
        }
    }
}

/// A group address on the other isolated port is a broadcast there: the
/// blocked frame is dropped without a reject, as a DLEN 0 one is.
#[test]
fn a_blocked_group_address_draws_no_reject() {
    let port = |network, mac: u8, isolated| {
        let mut port = PortConfig::new(
            network,
            Mac::from_array([mac]),
            NpduLimits::both(1497),
            Marked,
        );
        port.isolated = isolated;
        port
    };
    let ports = [port(1, 1, false), port(2, 2, true), port(3, 3, true)];
    let mut router = router_with(&ports, Options::HOSTED);
    for group in [0xFF, 0xFE] {
        let frame = apdu_to(2, &[group], UNCONFIRMED);
        let (outcome, sent) = deliver(&mut router, &mut Permissive, 2, NODE, &frame, at(1));
        assert_eq!(outcome, Outcome::Discarded(Discard::Isolated));
        assert!(sent.is_empty(), "{sent:?}");
    }
    assert_eq!(
        router.counters(),
        router_counts(|counts| counts.isolated_drops = 2)
    );
    // One node there still draws the reject.
    let frame = apdu_to(2, &[7], UNCONFIRMED);
    let (_, sent) = deliver(&mut router, &mut Permissive, 2, NODE, &frame, at(1));
    assert_eq!(sent[0].npdu, [0x01, 0x80, 0x03, 0x01, 0x00, 0x02]);
}

/// One isolated port alone has no peer: the router forwards, answers and
/// relays as one with no isolated port does, and counts nothing.
#[test]
fn a_single_isolated_port_routes_as_an_ordinary_one() {
    let mut ports = ports();
    ports[2].isolated = false;
    let mut router = router_with(&ports, Options::HOSTED);
    learn_9_via_r7(&mut router, at(0));
    // To and from the isolated port, by a direct and a learned route.
    for (from, network, to, mac) in [(1, 3, 2, &[7][..]), (2, 2, 1, &[7][..]), (2, 9, 1, R7)] {
        let frame = apdu_to(network, &[7], CONFIRMED);
        let (outcome, sent) = deliver(&mut router, &mut Permissive, from, NODE, &frame, at(1));
        assert_eq!(outcome, Outcome::Handled);
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].port, sent[0].to.as_deref()), (to, Some(mac)));
        assert_eq!(sent[0].kind, Kind::Forward);
    }
    let global = apdu_to(GLOBAL_NETWORK, &[], UNCONFIRMED);
    let (_, sent) = deliver(&mut router, &mut Permissive, 1, NODE, &global, at(1));
    assert_eq!(egress(&sent), [0, 2, 3]);
    let who = message(Msg::WHO_IS_ROUTER_TO_NETWORK, &networks(&[9]));
    let (outcome, sent) = deliver(&mut router, &mut Permissive, 2, NODE, &who, at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(egress(&sent), [2]);
    assert_eq!(listed(&sent[0]), [9]);
    assert_eq!(router.counters(), router_counts(|_| {}));
}

/// An isolated port's I-Am-Router for a network the table already reaches
/// through that same port, its own directly connected one included, is no
/// move: nothing is refused, and it is passed on.
#[test]
fn a_claim_for_a_network_on_the_same_isolated_port_is_not_refused() {
    let mut router = with_routes(Options::HOSTED);
    let mut host = Recorder::default();
    let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &i_am(&[2, 9]), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(egress(&sent), [0, 3]);
    assert!(router.table().lookup(2).unwrap().is_direct());
    assert_eq!(router.counters().isolated_refusals, 0);
    assert!(
        !host
            .events
            .iter()
            .any(|event| matches!(event, Event::IsolationRefused { .. }))
    );
}
