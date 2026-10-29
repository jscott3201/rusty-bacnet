//! Network messages relayed around a loop of routers (#1810, #2020), and a
//! router's own messages coming back round it (#2021).
//!
//! Three routers join networks 1, 2 and 3 in a ring, which the single path
//! of Clauses 4.2 and 6.2.3 rules out but a misconfigured site can have.
//! An I-Am-Router, Router-Busy or Router-Available has no DNET, so no hop
//! count, and each router passes it on whether or not its table changed
//! (Clauses 6.6.3.3, 6.6.3.6, 6.6.3.7); nor has a Who-Is-Router passed on
//! for a network no router knows (Clause 6.6.3.2). Without duplicate
//! suppression one circulates for good. With [`Options::duplicates`] each
//! router takes the first copy and drops the ones that come back.

mod fixture;

use std::collections::VecDeque;

use bacnet_router_core::npdu::{Address, GLOBAL_NETWORK, NetworkMessageType, Npdu};
use bacnet_router_core::router::{Discard, Duplicates, Options, Permissive};
use bacnet_router_core::router::{OriginateError, Request, SourceSelection};
use bacnet_router_core::table::Reachability;
use bacnet_router_core::{Instant, PortId};
use fixture::{
    Outcome, TestRouter, UNCONFIRMED, deliver_broadcast, drain, message, networks, port_on,
    router_with,
};

/// Router X, outside the ring, on network 1.
const X: &[u8] = &[0x99];

/// The ring: each router's two ports, as network and MAC.
const RING: [[(u16, u8); 2]; 3] = [
    [(1, 0x11), (2, 0x12)],
    [(2, 0x21), (3, 0x22)],
    [(3, 0x31), (1, 0x32)],
];

/// Deliveries after which a run counts as unbounded.
const LIMIT: usize = 10_000;

/// What the Busy and Available pair takes before every copy is dropped.
const PAIR_DELIVERIES: usize = 44;

fn ring(options: Options) -> Vec<TestRouter> {
    RING.iter()
        .map(|ports| {
            let ports = ports.map(|(network, mac)| port_on(network, &[mac]));
            router_with(&ports, options)
        })
        .collect()
}

/// What a run around the ring did.
#[derive(Debug)]
struct Run {
    /// Frames delivered to a router port.
    deliveries: usize,
    /// Of them, the ones dropped as repeats.
    duplicates: usize,
    /// Of them, the ones dropped as the receiving router's own.
    own: usize,
    /// Global broadcasts each router delivered to its own application.
    globals: [usize; 3],
}

/// A broadcast on a network: the network, the router and port that sent
/// it (`None` for X), and the NPDU.
type OnWire = (u16, Option<(usize, usize)>, Vec<u8>);

/// X broadcasts `frame` on network 1; every broadcast a router then sends
/// reaches the other router ports on that network, until none is left or
/// [`LIMIT`] deliveries have been made. Each delivery takes a millisecond.
fn circulate(routers: &mut [TestRouter], frame: Vec<u8>) -> Run {
    circulate_all(routers, vec![frame])
}

/// [`circulate`] for several frames X sends one after another.
fn circulate_all(routers: &mut [TestRouter], frames: Vec<Vec<u8>>) -> Run {
    let wire = frames.into_iter().map(|frame| (1, None, frame)).collect();
    pass_round(routers, wire)
}

/// The broadcasts already on `wire`, and every one a router sends on
/// hearing them, delivered as [`circulate`] does.
fn pass_round(routers: &mut [TestRouter], mut wire: VecDeque<OnWire>) -> Run {
    let mut run = Run {
        deliveries: 0,
        duplicates: 0,
        own: 0,
        globals: [0; 3],
    };
    while let Some((network, from, frame)) = wire.pop_front() {
        let source = from.map_or(X.to_vec(), |(router, port)| vec![RING[router][port].1]);
        for (index, router) in routers.iter_mut().enumerate() {
            for (port, &(on, _)) in RING[index].iter().enumerate() {
                if on != network || from == Some((index, port)) {
                    continue;
                }
                if run.deliveries == LIMIT {
                    return run;
                }
                run.deliveries += 1;
                let now = Instant::from_millis(run.deliveries as u64);
                let (outcome, sent) =
                    deliver_broadcast(router, &mut Permissive, port as u8, &source, &frame, now);
                match outcome {
                    Outcome::Discarded(Discard::Duplicate) => run.duplicates += 1,
                    Outcome::Discarded(Discard::OwnSource) => run.own += 1,
                    Outcome::Local { global: true, .. } => run.globals[index] += 1,
                    _ => {}
                }
                for send in sent {
                    assert!(send.is_broadcast());
                    let out = usize::from(send.port);
                    wire.push_back((RING[index][out].0, Some((index, out)), send.npdu));
                }
            }
        }
    }
    run
}

/// The messages that are passed on with no hop count: three with no
/// origin, and a Who-Is-Router for network 9, which no router knows,
/// passed on with X's.
fn relayed() -> [(&'static str, Vec<u8>); 4] {
    [
        ("I-Am-Router", i_am(&[9])),
        ("Router-Busy", busy(&[9])),
        ("Router-Available", available(&[])),
        ("Who-Is-Router", who_is_router(9)),
    ]
}

fn who_is_router(network: u16) -> Vec<u8> {
    message(
        NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK,
        &network.to_be_bytes(),
    )
}

fn i_am(list: &[u16]) -> Vec<u8> {
    message(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &networks(list))
}

fn busy(list: &[u16]) -> Vec<u8> {
    message(NetworkMessageType::ROUTER_BUSY_TO_NETWORK, &networks(list))
}

fn available(list: &[u16]) -> Vec<u8> {
    message(
        NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK,
        &networks(list),
    )
}

#[test]
fn without_suppression_a_relayed_message_circulates_for_good() {
    for (name, frame) in relayed() {
        let mut routers = ring(Options::HOSTED);
        let run = circulate(&mut routers, frame);
        assert_eq!(run.deliveries, LIMIT, "{name}: still going");
    }
}

#[test]
fn with_suppression_each_router_takes_the_first_copy_only() {
    let options = fixture::options(|options| options.duplicates = Some(Duplicates::DEFAULT));
    // An I-Am-Router, kept by its data alone: X's frame reaches the two
    // routers on network 1, each relays it once, and the two copies that
    // come back round are dropped. A Router-Busy or Router-Available,
    // kept by its sender too: each router takes it once from each of its
    // two neighbours, so it goes round once each way before the third
    // copy from one neighbour is dropped. A Who-Is-Router, kept by X as
    // its origin, goes as an I-Am-Router does (#2020).
    for ((name, frame), deliveries) in relayed().into_iter().zip([5, 10, 10, 5]) {
        let mut routers = ring(options);
        let run = circulate(&mut routers, frame);
        assert_eq!(run.deliveries, deliveries, "{name}: {run:?}");
        assert_eq!(run.duplicates, 2, "{name}: {run:?}");
        let dropped: u32 = routers
            .iter()
            .map(|router| router.counters().duplicate_drops)
            .sum();
        assert_eq!(dropped, 2, "{name}");
    }
}

/// The looped copies are neither applied nor passed on: network 9 stays
/// where X's own announcement put it, on each router's port to network 1
/// for the two routers on it, and through network 2 for the third.
#[test]
fn a_looped_announcement_moves_no_route() {
    let options = fixture::options(|options| options.duplicates = Some(Duplicates::DEFAULT));
    let mut routers = ring(options);
    let (_, frame) = relayed().into_iter().next().unwrap();
    let _ = circulate(&mut routers, frame);
    let ports: Vec<u8> = routers
        .iter()
        .map(|router| router.table().lookup(9).expect("learned").port().0)
        .collect();
    // R1's port 0 and R3's port 1 are on network 1; R2 heard it first
    // from R1, on its port to network 2.
    assert_eq!(ports, [0, 0, 1]);
    for router in &routers {
        let state = router
            .table()
            .effective_reachability(9, Instant::from_secs(1));
        assert_eq!(state, Some(Reachability::Reachable));
    }
}

/// One router, off the ring: the record keys a relayed message by its type
/// and data, forgets it after the time to live, and with suppression off
/// passes every copy on, as before.
#[test]
fn relayed_messages_are_keyed_by_type_and_data() {
    let on = fixture::options(|options| options.duplicates = Some(Duplicates::DEFAULT));
    let busy = message(NetworkMessageType::ROUTER_BUSY_TO_NETWORK, &networks(&[9]));
    let available = message(
        NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK,
        &networks(&[9]),
    );
    let i_am_9 = message(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &networks(&[9]));
    let i_am_8 = message(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &networks(&[8]));
    let mut router = router_with(&fixture::two_ports(), on);
    let mut relayed = |frame: &[u8], millis: u64| {
        let now = Instant::from_millis(millis);
        let (outcome, sent) = deliver_broadcast(&mut router, &mut Permissive, 1, X, frame, now);
        (outcome != Outcome::Discarded(Discard::Duplicate)).then_some(sent.len())
    };
    // Same data, another type; another list; then the first again.
    assert_eq!(relayed(&busy, 0), Some(1));
    assert_eq!(relayed(&available, 1), Some(1), "another type");
    assert_eq!(relayed(&i_am_9, 2), Some(1));
    assert_eq!(relayed(&i_am_8, 3), Some(1), "another list");
    assert_eq!(relayed(&i_am_9, 999), None, "within the time to live");
    assert_eq!(relayed(&i_am_9, 1_002), Some(1), "after it");

    let mut router = router_with(&fixture::two_ports(), Options::HOSTED);
    for _ in 0..3 {
        let (outcome, sent) =
            deliver_broadcast(&mut router, &mut Permissive, 1, X, &i_am_9, Instant::ZERO);
        assert_eq!(
            (outcome, sent.len()),
            (Outcome::Handled, 1),
            "off by default"
        );
    }
}

/// A Router-Busy then a Router-Available from X: each accepted message
/// forgets its opposite from the same sender, so without a cap on those
/// resets the pair would pass round the ring for good, each copy the
/// opposite of the one before it on every link. The cap,
/// [`Duplicates::RESET_LIMIT`] resets per sender within one time to live,
/// stops it.
#[test]
fn an_alternating_busy_and_available_pair_is_bounded() {
    let options = fixture::options(|options| options.duplicates = Some(Duplicates::DEFAULT));
    let mut routers = ring(options);
    let run = circulate_all(&mut routers, vec![busy(&[9]), available(&[9])]);
    assert!(run.deliveries < LIMIT, "{run:?}");
    assert_eq!(run.deliveries, PAIR_DELIVERIES, "{run:?}");
}

/// One router, off the ring: a Who-Is-Router it passes on is kept by its
/// origin, SNET and SADR or the arrival network and link source a relay
/// adds, with its type and data (#2020).
#[test]
fn a_relayed_who_is_router_is_keyed_by_origin_type_and_data() {
    let on = fixture::options(|options| options.duplicates = Some(Duplicates::DEFAULT));
    let mut router = router_with(&fixture::two_ports(), on);
    let mut relayed = |source: &[u8], frame: &[u8], millis: u64| {
        let now = Instant::from_millis(millis);
        let (outcome, sent) =
            deliver_broadcast(&mut router, &mut Permissive, 1, source, frame, now);
        (outcome != Outcome::Discarded(Discard::Duplicate)).then_some(sent.len())
    };
    // X asks on network 2 with no SNET; the same query relayed by another
    // router there names X as network 2's node: one origin.
    let relayed_from_x = fixture::bytes(&Npdu {
        source: Some(Address { network: 2, mac: X }),
        ..Npdu::message(
            NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK,
            &9u16.to_be_bytes(),
        )
    });
    assert_eq!(relayed(X, &who_is_router(9), 0), Some(1));
    assert_eq!(relayed(&[0x42], &relayed_from_x, 1), None, "X's, relayed");
    assert_eq!(relayed(X, &who_is_router(8), 2), Some(1), "another network");
    assert_eq!(
        relayed(&[0x42], &who_is_router(9), 3),
        Some(1),
        "another origin"
    );
    assert_eq!(relayed(X, &busy(&[]), 4), Some(1), "another type");
    assert_eq!(
        relayed(X, &who_is_router(9), 999),
        None,
        "within the time to live"
    );
    assert_eq!(relayed(X, &who_is_router(9), 1_000), Some(1), "after it");
    assert_eq!(router.counters().duplicate_drops, 2);

    // Off, every copy is passed on, as before.
    let mut router = router_with(&fixture::two_ports(), Options::HOSTED);
    for _ in 0..3 {
        let frame = who_is_router(9);
        let (outcome, sent) =
            deliver_broadcast(&mut router, &mut Permissive, 1, X, &frame, Instant::ZERO);
        assert_eq!(
            (outcome, sent.len()),
            (Outcome::Handled, 1),
            "off by default"
        );
    }
}

/// What router 0 sent, put on the wire of each port's network.
fn sent_by_first(routers: &mut [TestRouter]) -> VecDeque<OnWire> {
    drain(&mut routers[0])
        .into_iter()
        .map(|send| {
            assert!(send.is_broadcast());
            let port = usize::from(send.port);
            (RING[0][port].0, Some((0, port)), send.npdu)
        })
        .collect()
}

/// Router 0's own global Who-Is, from `source`, round the ring.
fn own_global(routers: &mut [TestRouter], source: SourceSelection) -> Run {
    let destination = Address {
        network: GLOBAL_NETWORK,
        mac: &[],
    };
    let mut request = Request::new(destination, UNCONFIRMED);
    request.source = source;
    routers[0].originate(&request, Instant::ZERO).unwrap();
    let wire = sent_by_first(routers);
    pass_round(routers, wire)
}

/// Router 0's own global broadcast comes back to it round the ring, with
/// SNET and SADR naming it on one of its ports: a router adds them only
/// for the node it heard a message from (Clause 6.5.4). It is dropped,
/// never delivered to router 0's application nor sent round again, with
/// or without duplicate suppression (#2021). Without that drop, router 0
/// passes each copy on until its hop count runs out.
#[test]
fn a_routers_own_global_broadcast_is_dropped_when_it_comes_back() {
    let on = fixture::options(|options| options.duplicates = Some(Duplicates::DEFAULT));
    // From the egress address, each port's copy is bare, so each of the
    // others hears two origins, router 0 on network 1 and on network 2,
    // and takes both. Named from port 0, every copy is from router 0 on
    // network 1, and with suppression the second copy at each of the
    // others is dropped before it reaches router 0.
    for (source, options, deliveries, globals) in [
        (SourceSelection::EgressPort, Options::HOSTED, 6, [0, 2, 2]),
        (SourceSelection::EgressPort, on, 6, [0, 2, 2]),
        (
            SourceSelection::Port(PortId(0)),
            Options::HOSTED,
            6,
            [0, 2, 2],
        ),
        (SourceSelection::Port(PortId(0)), on, 4, [0, 1, 1]),
    ] {
        let mut routers = ring(options);
        let run = own_global(&mut routers, source);
        let case = format!("{source:?}, duplicates {}", options.duplicates.is_some());
        assert_eq!(run.deliveries, deliveries, "{case}: {run:?}");
        assert_eq!(run.globals, globals, "{case}: never to router 0's own");
        let own = if deliveries == 6 { 2 } else { 0 };
        assert_eq!(run.own, own, "{case}: {run:?}");
        assert_eq!(routers[0].counters().own_source_drops, own as u32, "{case}");
    }
}

/// Router 0's own search for network 9, which no router knows, comes back
/// to it named as its origin, and is dropped there with no duplicate
/// record (#2021): bounded in firmware too.
#[test]
fn a_routers_own_search_is_dropped_when_it_comes_back() {
    let mut routers = ring(Options::HOSTED);
    let request = Request::new(
        Address {
            network: 9,
            mac: &[],
        },
        UNCONFIRMED,
    );
    let refused = routers[0].originate(&request, Instant::ZERO);
    assert_eq!(refused, Err(OriginateError::NoRoute));
    let wire = sent_by_first(&mut routers);
    assert_eq!(wire.len(), 2, "a search on both ports");
    let run = pass_round(&mut routers, wire);
    // Each search reaches a neighbour, then the third router, then router
    // 0, which drops it.
    assert_eq!((run.deliveries, run.own), (6, 2), "{run:?}");
    assert_eq!(routers[0].counters().own_source_drops, 2);
}

/// Only the router's exact address, a port's network with its MAC on that
/// port, is its own: its MAC on another of its networks, or another MAC
/// on its network, is some other node, and passes (#2021).
#[test]
fn only_the_routers_exact_address_is_its_own() {
    use fixture::{NODE_A, OWN_0, OWN_1};
    let global = |source| {
        fixture::bytes(&Npdu {
            destination: Some(Address {
                network: GLOBAL_NETWORK,
                mac: &[],
            }),
            source: Some(source),
            ..Npdu::apdu(UNCONFIRMED)
        })
    };
    let search = |source| {
        fixture::bytes(&Npdu {
            source: Some(source),
            ..Npdu::message(
                NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK,
                &9u16.to_be_bytes(),
            )
        })
    };
    let addr = |network, mac| Address { network, mac };
    for (source, own) in [
        (addr(2, OWN_1), true),
        (addr(1, OWN_0), true),
        (addr(1, OWN_1), false),
        (addr(2, OWN_0), false),
        (addr(2, &[0x02]), false),
        (addr(3, OWN_1), false),
    ] {
        for frame in [global(source), search(source)] {
            let mut router = router_with(&fixture::two_ports(), Options::HOSTED);
            let (outcome, sent) = deliver_broadcast(
                &mut router,
                &mut Permissive,
                0,
                NODE_A,
                &frame,
                Instant::ZERO,
            );
            let dropped = outcome == Outcome::Discarded(Discard::OwnSource);
            assert_eq!(dropped, own, "{source:?}: {outcome:?}");
            assert_eq!(sent.is_empty(), own, "{source:?}: passed on unless own");
            assert_eq!(router.counters().own_source_drops, u32::from(own));
        }
    }
}
