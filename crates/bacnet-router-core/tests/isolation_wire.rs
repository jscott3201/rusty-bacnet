//! Port isolation on the wire (#2684): what a blocked sender and an
//! isolated claimant see, frame by frame, and what the router counts.
//!
//! The router has four ports: port 0 on network 1 and port 3 on network 4
//! are ordinary; port 1 on network 2 and port 2 on network 3 are isolated.

mod fixture;

use bacnet_router_core::PortId;
use bacnet_router_core::message::RoutingEntries;
use bacnet_router_core::npdu::{
    GLOBAL_NETWORK, Mac, NetworkMessageType as Msg, Npdu, RejectMessageReason,
};
use bacnet_router_core::router::{ControlClass, Discard, Event, Kind, Options, Permissive};
use bacnet_router_core::table::Reachability;
use fixture::*;

/// A node on whichever port a test delivers from.
const NODE: &[u8] = &[0x55];
/// A router on an ordinary port, the next hop to network 50.
const R9: &[u8] = &[0x09];

fn ports() -> [bacnet_router_core::router::PortConfig; 4] {
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

fn isolating() -> TestRouter {
    router_with(&ports(), Options::HOSTED)
}

/// The same router with network 9 behind R7 on isolated port 1 and
/// network 50 behind R9 on ordinary port 0.
fn with_routes() -> TestRouter {
    let mut router = isolating();
    learn_9_via_r7(&mut router, at(0));
    learn(&mut router, 50, 0, R9);
    router
}

fn learn(router: &mut TestRouter, network: u16, port: u8, next_hop: &[u8]) {
    let next_hop = Mac::from_slice(next_hop).unwrap();
    assert!(
        router
            .table_mut()
            .add_learned(network, PortId(port), next_hop, at(0))
    );
}

/// Where the table sends for `network`: its port and next hop.
fn route(router: &TestRouter, network: u16) -> (u8, Vec<u8>) {
    let route = router.table().lookup(network).unwrap();
    (route.port().0, route.next_hop().as_slice().to_vec())
}

/// A send as (port, unicast destination, octets).
type Wire<'a> = (u8, Option<&'a [u8]>, &'a [u8]);

fn wire(sent: &[Sent]) -> Vec<Wire<'_>> {
    sent.iter()
        .map(|sent| (sent.port, sent.to.as_deref(), sent.npdu.as_slice()))
        .collect()
}

fn refused(network: u16, port: u8, class: ControlClass) -> Event {
    Event::IsolationRefused {
        network,
        port: PortId(port),
        class,
    }
}

/// One Initialize-Routing-Table entry per (network, wire Port ID).
fn table_update(entries: &[(u16, u8)]) -> Vec<u8> {
    let mut data = vec![entries.len() as u8];
    for (network, port_id) in entries {
        data.extend_from_slice(&network.to_be_bytes());
        data.extend_from_slice(&[*port_id, 0]);
    }
    data
}

/// A message for one node on a network behind another isolated port, by a
/// direct route (network 2) and by a learned one (network 9): the sender
/// gets reject reason 1 naming the network, back out of its own port, and
/// nothing goes anywhere else. No search starts (Clauses 6.4.4, 6.6.3.5).
#[test]
fn blocked_unicast_transit_draws_reason_1_on_the_arrival_port_and_no_search() {
    let mut router = with_routes();
    for (network, reject) in [
        (2, [0x01, 0x80, 0x03, 0x01, 0x00, 0x02]),
        (9, [0x01, 0x80, 0x03, 0x01, 0x00, 0x09]),
    ] {
        for apdu in [UNCONFIRMED, CONFIRMED] {
            let frame = apdu_to(network, &[7], apdu);
            let (outcome, sent) = deliver(&mut router, &mut Permissive, 2, NODE, &frame, at(1));
            assert_eq!(outcome, Outcome::Discarded(Discard::Isolated));
            assert_eq!(wire(&sent), [(2, Some(NODE), &reject[..])]);
            assert_eq!(
                sent[0].kind,
                Kind::Reject(RejectMessageReason::NOT_DIRECTLY_CONNECTED)
            );
        }
    }
    // A sender behind another router gets it through that router: to the
    // link sender, with the sender's SNET and SADR as DNET and DADR.
    let mut routed = Npdu::apdu(UNCONFIRMED);
    routed.destination = Some(addr(2, &[7]));
    routed.source = Some(addr(77, &[5]));
    let (outcome, sent) = deliver(
        &mut router,
        &mut Permissive,
        2,
        NODE,
        &bytes(&routed),
        at(1),
    );
    assert_eq!(outcome, Outcome::Discarded(Discard::Isolated));
    let reject = [
        0x01, 0xA0, 0x00, 0x4D, 0x01, 0x05, 0xFF, 0x03, 0x01, 0x00, 0x02,
    ];
    assert_eq!(wire(&sent), [(2, Some(NODE), &reject[..])]);

    assert_eq!(
        router.counters(),
        router_counts(|counts| {
            counts.isolated_drops = 5;
            counts.rejects = 5;
        })
    );
    assert!(
        router.discovery().is_empty(),
        "no search for a blocked DNET"
    );
    // The route is untouched: an ordinary port still routes to both.
    assert_eq!(
        router.table().lookup(9).unwrap().reachability(),
        Reachability::Reachable
    );
    for (from, network, to) in [(0, 9, R7), (3, 2, &[7][..]), (0, 2, &[7][..])] {
        let frame = apdu_to(network, &[7], UNCONFIRMED);
        let (outcome, sent) = deliver(&mut router, &mut Permissive, from, NODE, &frame, at(2));
        assert_eq!(outcome, Outcome::Handled);
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].port, sent[0].to.as_deref()), (1, Some(to)));
        assert_eq!(sent[0].kind, Kind::Forward);
    }
}

/// Broadcasts from an isolated port reach the ordinary ports and the
/// router's application, never another isolated port, and draw no reject.
#[test]
fn broadcasts_from_an_isolated_port_skip_isolated_ports_without_a_reject() {
    let mut router = with_routes();
    let global = apdu_to(GLOBAL_NETWORK, &[], UNCONFIRMED);
    let (outcome, sent) = deliver(&mut router, &mut Permissive, 1, NODE, &global, at(1));
    assert!(matches!(outcome, Outcome::Local { global: true, .. }));
    let relayed = [
        0x01, 0x28, 0xFF, 0xFF, 0x00, 0x00, 0x02, 0x01, 0x55, 0xFE, 0x10, 0x08,
    ];
    assert_eq!(
        wire(&sent),
        [(0, None, &relayed[..]), (3, None, &relayed[..])]
    );

    // A remote broadcast for an ordinary network goes there.
    let remote = apdu_to(4, &[], UNCONFIRMED);
    let (outcome, sent) = deliver(&mut router, &mut Permissive, 1, NODE, &remote, at(1));
    assert_eq!(outcome, Outcome::Handled);
    let relayed = [0x01, 0x08, 0x00, 0x02, 0x01, 0x55, 0x10, 0x08];
    assert_eq!(wire(&sent), [(3, None, &relayed[..])]);
    assert_eq!(router.counters().isolated_drops, 0);

    // One for a network behind the other isolated port, by a direct or a
    // learned route, is dropped in silence.
    for (from, network) in [(1, 3), (2, 2), (2, 9)] {
        let remote = apdu_to(network, &[], UNCONFIRMED);
        let (outcome, sent) = deliver(&mut router, &mut Permissive, from, NODE, &remote, at(1));
        assert_eq!(outcome, Outcome::Discarded(Discard::Isolated));
        assert!(sent.is_empty(), "{sent:?}");
    }
    assert_eq!(
        router.counters(),
        router_counts(|counts| counts.isolated_drops = 3)
    );
    assert!(router.discovery().is_empty());
}

/// An I-Am-Router from an isolated port can't take a network the table
/// reaches through another port, ordinary or direct: the route stays, the
/// refusal is counted and reported, and the message isn't passed on.
#[test]
fn an_i_am_router_from_an_isolated_port_cannot_move_a_route_and_is_not_relayed() {
    let mut router = with_routes();
    let mut host = Recorder::default();
    // A route learned on an ordinary port; a directly connected network;
    // a route on the other isolated port.
    for (step, network) in [(1, 50), (2, 4), (3, 1), (4, 3)] {
        let before = route(&router, network);
        let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &i_am(&[network]), at(1));
        assert_eq!(outcome, Outcome::Handled);
        assert!(sent.is_empty(), "a refused claim is not relayed: {sent:?}");
        assert_eq!(route(&router, network), before);
        assert_eq!(router.counters().isolated_refusals, step);
        assert_eq!(
            host.events.last(),
            Some(&refused(network, 1, ControlClass::IAm))
        );
    }
    assert_eq!(route(&router, 50), (0, R9.to_vec()));
    assert!(router.table().lookup(4).unwrap().is_direct());

    // One refused claim withholds the whole message; its other networks
    // are still learned here.
    host.events.clear();
    let (_, sent) = deliver(&mut router, &mut host, 1, R7, &i_am(&[60, 50, 61]), at(1));
    assert!(sent.is_empty(), "{sent:?}");
    assert_eq!(route(&router, 60), (1, R7.to_vec()));
    assert_eq!(route(&router, 61), (1, R7.to_vec()));
    assert_eq!(route(&router, 50), (0, R9.to_vec()));
    assert_eq!(router.counters().isolated_refusals, 5);
    assert_eq!(
        host.events
            .iter()
            .filter(|event| matches!(event, Event::IsolationRefused { .. }))
            .collect::<Vec<_>>(),
        [&refused(50, 1, ControlClass::IAm)]
    );
    assert_eq!(router.counters().isolated_drops, 0);
}

/// What an isolated port may still claim, and what other ports may do, is
/// as on a router without isolation.
#[test]
fn allowed_route_claims_still_apply_and_relay() {
    let mut router = with_routes();
    let mut host = Recorder::default();
    // A new network from an isolated port: learned, and relayed to the
    // ordinary ports only.
    let relay = [0x01, 0x80, 0x01, 0x00, 0x3E];
    let (_, sent) = deliver(&mut router, &mut host, 1, R7, &i_am(&[62]), at(1));
    assert_eq!(route(&router, 62), (1, R7.to_vec()));
    assert_eq!(wire(&sent), [(0, None, &relay[..]), (3, None, &relay[..])]);
    // A refresh through the same isolated port, from another router there.
    let (_, sent) = deliver(&mut router, &mut host, 1, &[0x08], &i_am(&[62]), at(2));
    assert_eq!(route(&router, 62), (1, vec![0x08]));
    assert_eq!(wire(&sent), [(0, None, &relay[..]), (3, None, &relay[..])]);
    // Between two ordinary ports.
    let (_, sent) = deliver(&mut router, &mut host, 3, R7, &i_am(&[50]), at(3));
    assert_eq!(route(&router, 50), (3, R7.to_vec()));
    assert_eq!(
        sent.iter().map(|sent| sent.port).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    // From an ordinary port, taking a network off an isolated one.
    let (_, sent) = deliver(&mut router, &mut host, 0, R9, &i_am(&[9]), at(4));
    assert_eq!(route(&router, 9), (0, R9.to_vec()));
    assert_eq!(
        sent.iter().map(|sent| sent.port).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(router.counters().isolated_refusals, 0);
    assert!(
        !host
            .events
            .iter()
            .any(|event| matches!(event, Event::IsolationRefused { .. }))
    );

    // One isolated port alone has no peer to protect: it claims as an
    // ordinary port does.
    let mut ports = ports();
    ports[2].isolated = false;
    let mut alone = router_with(&ports, Options::HOSTED);
    learn(&mut alone, 50, 0, R9);
    let (_, sent) = deliver(&mut alone, &mut host, 1, R7, &i_am(&[50]), at(1));
    assert_eq!(route(&alone, 50), (1, R7.to_vec()));
    assert_eq!(
        sent.iter().map(|sent| sent.port).collect::<Vec<_>>(),
        [0, 2, 3]
    );
    assert_eq!(alone.counters().isolated_refusals, 0);
}

/// A routing table update, or an Ack that carries a table, from an
/// isolated port can't bring to that port a network the table reaches
/// through another one.
#[test]
fn a_table_update_from_an_isolated_port_cannot_move_a_route_onto_it() {
    let mut router = with_routes();
    let mut host = Recorder::default();
    // Port 1's wire Port ID is 2. Network 50 is on an ordinary port, 4 is
    // directly connected, 63 is new.
    let update = message(
        Msg::INITIALIZE_ROUTING_TABLE,
        &table_update(&[(50, 2), (4, 2), (63, 2)]),
    );
    let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &update, at(1));
    assert_eq!(outcome, Outcome::Handled);
    // The update is still acknowledged, with no data (Clause 6.6.3.9).
    assert_eq!(wire(&sent), [(1, Some(R7), &[0x01, 0x80, 0x07][..])]);
    assert_eq!(route(&router, 50), (0, R9.to_vec()));
    assert!(router.table().lookup(4).unwrap().is_direct());
    assert_eq!(route(&router, 4).0, 3);
    assert_eq!(route(&router, 63), (1, R7.to_vec()));
    assert_eq!(router.counters().isolated_refusals, 2);
    let refusals: Vec<_> = host
        .events
        .iter()
        .filter(|event| matches!(event, Event::IsolationRefused { .. }))
        .copied()
        .collect();
    assert_eq!(
        refusals,
        [
            refused(50, 1, ControlClass::InitMgmt),
            refused(4, 1, ControlClass::InitMgmt)
        ]
    );

    // An Ack with a table is a claim for each network through its sender.
    let ack = message(
        Msg::INITIALIZE_ROUTING_TABLE_ACK,
        &table_update(&[(50, 1), (64, 1)]),
    );
    let (_, sent) = deliver(&mut router, &mut host, 1, R7, &ack, at(1));
    assert!(sent.is_empty());
    assert_eq!(route(&router, 50), (0, R9.to_vec()));
    assert_eq!(route(&router, 64), (1, R7.to_vec()));
    assert_eq!(router.counters().isolated_refusals, 3);
    assert_eq!(
        host.events
            .iter()
            .rev()
            .find(|event| matches!(event, Event::IsolationRefused { .. })),
        Some(&refused(50, 1, ControlClass::InitAck))
    );

    // An entry for a route on the other isolated port, or one that names
    // the other isolated port, is refused too.
    let update = message(
        Msg::INITIALIZE_ROUTING_TABLE,
        &table_update(&[(9, 0), (88, 2)]),
    );
    let _ = deliver(&mut router, &mut host, 2, R7, &update, at(1));
    assert_eq!(route(&router, 9), (1, R7.to_vec()));
    assert!(router.table().lookup(88).is_none());
    assert_eq!(router.counters().isolated_refusals, 5);
}

/// Table updates the decision leaves alone: from an ordinary port, and,
/// from an isolated one, a change that brings nothing to an isolated port.
/// By design, isolation is a rule about transit and no access control on
/// updates (decisions of 2026-11-20 and 2026-11-22, #2717): a node on an
/// isolated port can still remove or redirect a route through an ordinary
/// port, and what stops it is the host's authorizer.
#[test]
fn by_design_isolation_is_no_access_control_on_table_updates() {
    let mut router = with_routes();
    let mut host = Recorder::default();
    // From an ordinary port: onto the other ordinary port (wire ID 4),
    // then onto an isolated port (wire ID 2).
    for (port_id, port) in [(4, 3), (2, 1)] {
        let update = message(
            Msg::INITIALIZE_ROUTING_TABLE,
            &table_update(&[(50, port_id)]),
        );
        let _ = deliver(&mut router, &mut host, 0, R9, &update, at(1));
        assert_eq!(route(&router, 50), (port, R9.to_vec()));
    }
    // The route is on isolated port 1 now: that port may refresh it.
    let update = message(Msg::INITIALIZE_ROUTING_TABLE, &table_update(&[(50, 2)]));
    let _ = deliver(&mut router, &mut host, 1, R7, &update, at(1));
    assert_eq!(route(&router, 50), (1, R7.to_vec()));
    // From an isolated port, removing a route that is on an ordinary port
    // still works: only `Host::authorize` decides that.
    learn(&mut router, 51, 0, R9);
    let update = message(Msg::INITIALIZE_ROUTING_TABLE, &table_update(&[(51, 0)]));
    let _ = deliver(&mut router, &mut host, 1, R7, &update, at(1));
    assert!(router.table().lookup(51).is_none());
    assert_eq!(router.counters().isolated_refusals, 0);

    // The control that does stop it: a host that refuses table updates.
    learn(&mut router, 51, 0, R9);
    host.deny.push(ControlClass::InitMgmt);
    let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &update, at(1));
    assert_eq!(outcome, Outcome::Discarded(Discard::Denied));
    assert!(sent.is_empty());
    assert_eq!(route(&router, 51), (0, R9.to_vec()));
}

/// A reject that arrives on an isolated port and names a network reached
/// through an ordinary port. By design it marks that route, as a reject
/// from any port does (Clause 6.6.3.5 names no sender): isolation is no
/// trust level for a port (#2717). The two controls that stop it are the
/// ones the docs point at, `Options::next_hop_rejects` and a host that
/// refuses rejects, and with either the route stays reachable.
#[test]
fn by_design_a_reject_from_an_isolated_port_marks_a_route_through_an_ordinary_port() {
    let reject = message(
        Msg::REJECT_MESSAGE_TO_NETWORK,
        &reject_data(RejectMessageReason::NOT_DIRECTLY_CONNECTED, 50),
    );
    let state = |router: &TestRouter| router.table().effective_reachability(50, at(1));

    let mut router = with_routes();
    let mut host = Recorder::default();
    let _ = deliver(&mut router, &mut host, 1, NODE, &reject, at(1));
    assert_eq!(state(&router), Some(Reachability::Unreachable));
    assert_eq!(router.counters().isolated_refusals, 0);

    let next_hop_only = options(|options| options.next_hop_rejects = true);
    let mut router = router_with(&ports(), next_hop_only);
    learn(&mut router, 50, 0, R9);
    let _ = deliver(&mut router, &mut host, 1, NODE, &reject, at(1));
    assert_eq!(state(&router), Some(Reachability::Reachable));
    assert_eq!(router.counters().not_next_hop, 1);

    let mut router = with_routes();
    host.deny.push(ControlClass::Reject);
    let (outcome, _) = deliver(&mut router, &mut host, 1, NODE, &reject, at(1));
    assert_eq!(outcome, Outcome::Discarded(Discard::Denied));
    assert_eq!(state(&router), Some(Reachability::Reachable));
}

/// A routing table query relayed from another network (#2699) that
/// arrives on an isolated port is answered with the rows that port may
/// see, whichever chunk they fall in.
#[test]
fn a_routed_table_query_on_an_isolated_port_lists_only_visible_rows_across_chunks() {
    let mut router = isolating();
    // Every fourth network is behind the other isolated port, the first
    // and the last of the range among them.
    for network in 20..=260 {
        let port = if network % 4 == 0 { 1 } else { 0 };
        learn(&mut router, network, port, R7);
    }
    let query = bytes(&Npdu {
        source: Some(addr(77, &[5])),
        ..Npdu::message(Msg::INITIALIZE_ROUTING_TABLE, &[0])
    });
    let (outcome, sent) = deliver(&mut router, &mut Permissive, 2, NODE, &query, at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert!(sent.len() >= 2, "{} Acks", sent.len());
    let mut rows = Vec::new();
    for ack in &sent {
        assert_eq!((ack.port, ack.to.as_deref()), (2, Some(NODE)));
        let npdu = ack.parsed();
        assert_eq!(npdu.message_type, Some(Msg::INITIALIZE_ROUTING_TABLE_ACK));
        assert_eq!(npdu.destination, Some(addr(77, &[5])));
        assert_eq!(npdu.source, None);
        let entries = RoutingEntries::parse(npdu.payload).unwrap();
        assert!(entries.count() > 0);
        rows.extend(entries.iter().map(|entry| (entry.network, entry.port_id)));
    }
    // Its own network, both ordinary ones, and the learned routes on the
    // ordinary port: never network 2 nor a route through port 1.
    let expected: Vec<_> = [(1, 1), (3, 3), (4, 4)]
        .into_iter()
        .chain((20..=260).filter(|n| n % 4 != 0).map(|n| (n, 1)))
        .collect();
    assert_eq!(rows, expected);
    assert_eq!(router.counters().isolated_drops, 0);
}

/// The one asymmetry with routed table queries: a query from one isolated
/// port that names the router's own address on another is refused by the
/// route lookup, as any message for that network is, though an APDU for
/// the same address is the router's own and is delivered.
#[test]
fn a_table_query_for_the_routers_address_on_another_isolated_port_is_rejected() {
    let mut router = isolating();
    let own_on_port_2 = addr(3, &[3]);
    let query = bytes(&Npdu {
        destination: Some(own_on_port_2),
        ..Npdu::message(Msg::INITIALIZE_ROUTING_TABLE, &[0])
    });
    let (outcome, sent) = deliver(&mut router, &mut Permissive, 1, NODE, &query, at(1));
    assert_eq!(outcome, Outcome::Discarded(Discard::Isolated));
    let reject = [0x01, 0x80, 0x03, 0x01, 0x00, 0x03];
    assert_eq!(wire(&sent), [(1, Some(NODE), &reject[..])]);

    let frame = apdu_to(3, &[3], CONFIRMED);
    let (outcome, sent) = deliver(&mut router, &mut Permissive, 1, NODE, &frame, at(1));
    assert!(matches!(outcome, Outcome::Local { group: false, .. }));
    assert!(sent.is_empty());
    assert_eq!(
        router.counters(),
        router_counts(|counts| {
            counts.isolated_drops = 1;
            counts.rejects = 1;
        })
    );
}

/// The other places isolation stops a frame or a route change, each with
/// its reason and its count.
#[test]
fn the_remaining_drop_sites_report_and_count() {
    let mut router = with_routes();
    let mut host = Recorder::default();
    // A Who-Is-Router for a network behind the other isolated port: no
    // answer and no search onward.
    for network in [2, 9] {
        let who = message(Msg::WHO_IS_ROUTER_TO_NETWORK, &networks(&[network]));
        let (outcome, sent) = deliver(&mut router, &mut host, 2, NODE, &who, at(1));
        assert_eq!(outcome, Outcome::Discarded(Discard::Isolated));
        assert!(sent.is_empty());
    }
    assert_eq!(router.counters().isolated_drops, 2);

    // A reject from one isolated port on its way to a node behind the
    // other: not passed on. It names a network behind that port too, so
    // its mark on the route is refused as well.
    let mut reject = Npdu::message(Msg::REJECT_MESSAGE_TO_NETWORK, &[1, 0, 9]);
    reject.destination = Some(addr(2, &[7]));
    let (outcome, sent) = deliver(&mut router, &mut host, 2, NODE, &bytes(&reject), at(1));
    assert_eq!(outcome, Outcome::Discarded(Discard::Isolated));
    assert!(sent.is_empty());
    assert_eq!(
        router.table().lookup(9).unwrap().reachability(),
        Reachability::Reachable
    );
    assert_eq!(
        host.events.last(),
        Some(&refused(9, 2, ControlClass::Reject))
    );
    assert_eq!(
        router.counters(),
        router_counts(|counts| {
            counts.isolated_drops = 3;
            counts.isolated_refusals = 1;
        })
    );
    // The same reject toward a node on an ordinary port goes on.
    let mut reject = Npdu::message(Msg::REJECT_MESSAGE_TO_NETWORK, &[3, 0, 77]);
    reject.destination = Some(addr(1, &[7]));
    let (outcome, sent) = deliver(&mut router, &mut host, 2, NODE, &bytes(&reject), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (0, Some(&[7][..])));
    assert_eq!(router.counters().isolated_drops, 3);
}
