//! Port isolation applies to transit, route visibility and wire management.
mod fixture;

use bacnet_router_core::message::RoutingEntries;
use bacnet_router_core::npdu::{GLOBAL_NETWORK, Mac, NetworkMessageType as Msg, Npdu};
use bacnet_router_core::router::{
    Backlog, Congestion, Kind, Options, Permissive, Request, SourceSelection,
};
use bacnet_router_core::table::Convergence;
use bacnet_router_core::{Instant, PortId};
use fixture::*;

fn configured(options: Options) -> TestRouter {
    let mut ports = [
        port_on(1, &[1]),
        port_on(2, &[2]),
        port_on(3, &[3]),
        port_on(4, &[4]),
    ];
    ports[1].isolated = true;
    ports[2].isolated = true;
    router_with(&ports, options)
}

fn sends(router: &mut TestRouter, port: u8, npdu: &[u8]) -> Vec<Sent> {
    deliver(router, &mut Permissive, port, R7, npdu, at(1)).1
}

fn egress(sent: &[Sent]) -> Vec<u8> {
    sent.iter().map(|s| s.port).collect()
}

#[test]
fn transit_checks_the_resolved_port_for_unicast_remote_and_global_broadcasts() {
    let mut r = configured(Options::HOSTED);
    learn_9_via_r7(&mut r, at(0));
    for (from, network) in [(1, 3), (2, 2), (2, 9)] {
        for mac in [&[7][..], &[]] {
            assert!(sends(&mut r, from, &apdu_to(network, mac, UNCONFIRMED)).is_empty());
        }
    }
    for (from, network, to) in [(0, 2, 1), (0, 3, 2), (1, 1, 0), (2, 1, 0), (0, 9, 1)] {
        assert_eq!(
            egress(&sends(&mut r, from, &apdu_to(network, &[7], UNCONFIRMED))),
            [to]
        );
    }
    let global = apdu_to(GLOBAL_NETWORK, &[], UNCONFIRMED);
    let (local, sent) = deliver(&mut r, &mut Permissive, 1, R7, &global, at(2));
    assert!(matches!(local, Outcome::Local { global: true, .. }));
    assert_eq!(egress(&sent), [0, 3]);
    assert_eq!(egress(&sends(&mut r, 0, &global)), [1, 2, 3]);

    let mut own = Request::new(addr(GLOBAL_NETWORK, &[]), UNCONFIRMED);
    own.source = SourceSelection::Port(PortId(1));
    r.originate(&own, at(2)).unwrap();
    assert_eq!(egress(&drain(&mut r)), [0, 2, 3, 1]);
}

#[test]
fn discovery_only_searches_and_answers_across_permitted_ports() {
    let mut r = configured(Options::HOSTED);
    learn_9_via_r7(&mut r, at(0));
    for hidden in [2, 9] {
        let who = message(Msg::WHO_IS_ROUTER_TO_NETWORK, &networks(&[hidden]));
        assert!(sends(&mut r, 2, &who).is_empty());
        let sent = sends(&mut r, 0, &who);
        assert_eq!(listed(&sent[0]), [hidden]);
    }
    let who = message(Msg::WHO_IS_ROUTER_TO_NETWORK, &[]);
    let sent = sends(&mut r, 2, &who);
    assert_eq!(listed(&sent[0]), [1, 4]);
    let unknown = message(Msg::WHO_IS_ROUTER_TO_NETWORK, &networks(&[99]));
    assert_eq!(egress(&sends(&mut r, 2, &unknown)), [0, 3]);
    let unknown = apdu_to(99, &[7], UNCONFIRMED);
    let sent = sends(&mut r, 2, &unknown);
    assert_eq!(egress(&sent), [0, 3, 2]);
    assert!(matches!(sent[2].kind, Kind::Reject(_)));
}

#[test]
fn startup_announcements_do_not_expose_other_isolated_ports() {
    let mut r = configured(Options::HOSTED);
    r.start();
    let ads: Vec<_> = drain(&mut r)
        .into_iter()
        .filter(|s| s.kind == Kind::Announce(Msg::I_AM_ROUTER_TO_NETWORK))
        .map(|s| (s.port, listed(&s)))
        .collect();
    assert_eq!(
        ads,
        [
            (0, vec![2, 3, 4]),
            (1, vec![1, 4]),
            (2, vec![1, 4]),
            (3, vec![1, 2, 3])
        ]
    );
    let mut ports = two_ports();
    for port in &mut ports {
        port.isolated = true;
    }
    let mut r = router_with(&ports, Options::HOSTED);
    r.start();
    let sent = drain(&mut r);
    assert_eq!(egress(&sent), [0, 1]);
    assert!(
        sent.iter()
            .all(|s| s.kind == Kind::Announce(Msg::NETWORK_NUMBER_IS))
    );
}

#[test]
fn paged_table_replies_keep_visible_entries_including_the_request_port() {
    let mut r = configured(Options::HOSTED);
    for network in 20..220 {
        let port = if network % 4 == 0 { 1 } else { 0 };
        assert!(
            r.table_mut()
                .add_learned(network, PortId(port), Mac::from_array([7]), at(0))
        );
    }
    let query = message(Msg::INITIALIZE_ROUTING_TABLE, &[0]);
    let sent = sends(&mut r, 2, &query);
    assert_eq!(egress(&sent), [2, 2]);
    let entries: Vec<_> = sent
        .iter()
        .flat_map(|s| {
            RoutingEntries::parse(s.parsed().payload)
                .unwrap()
                .iter()
                .map(|e| e.network)
        })
        .collect();
    let expected: Vec<_> = [1, 3, 4]
        .into_iter()
        .chain((20..220).filter(|n| n % 4 != 0))
        .collect();
    assert_eq!(entries, expected);
}

#[test]
fn each_control_relay_preserves_the_full_list_for_ordinary_ports() {
    for kind in [
        Msg::I_AM_ROUTER_TO_NETWORK,
        Msg::ROUTER_BUSY_TO_NETWORK,
        Msg::ROUTER_AVAILABLE_TO_NETWORK,
    ] {
        let mut r = configured(options(|o| o.convergence = Convergence::Hardened));
        learn_9_via_r7(&mut r, at(0));
        let mixed = message(kind, &networks(&[2, 91, 9, 92]));
        let sent = sends(&mut r, 0, &mixed);
        assert_eq!(egress(&sent), [3, 1, 2]);
        assert_eq!(listed(&sent[0]), [2, 91, 9, 92]);
        for s in &sent[1..] {
            assert_eq!(listed(s), [91, 92]);
        }
        let hidden = message(kind, &networks(&[2, 3]));
        assert_eq!(egress(&sends(&mut r, 0, &hidden)), [3]);
        let relay = message(kind, &networks(&[90]));
        assert_eq!(egress(&sends(&mut r, 1, &relay)), [0, 3]);
        if kind != Msg::I_AM_ROUTER_TO_NETWORK {
            // An originally empty flow-control message covers its sender's routes.
            assert_eq!(egress(&sends(&mut r, 0, &message(kind, &[]))), [1, 2, 3]);
        }
    }
}

#[test]
fn isolated_routes_are_protected_from_peer_management_and_rejects() {
    let mut r = configured(Options::HOSTED);
    learn_9_via_r7(&mut r, at(0));
    let management = message(Msg::INITIALIZE_ROUTING_TABLE, &[2, 0, 9, 0, 0, 0, 88, 2, 0]);
    let _ = sends(&mut r, 2, &management);
    let _ = sends(&mut r, 2, &i_am(&[9]));
    assert_eq!(r.table().lookup(9).unwrap().port(), PortId(1));
    assert!(r.table().lookup(88).is_none());
    let mut reject = Npdu::message(Msg::REJECT_MESSAGE_TO_NETWORK, &[1, 0, 9]);
    reject.destination = Some(addr(2, &[7]));
    assert!(sends(&mut r, 2, &bytes(&reject)).is_empty());
    assert_eq!(
        r.table().lookup(9).unwrap().reachability(),
        bacnet_router_core::table::Reachability::Reachable
    );
    let mut unknown = Npdu::apdu(UNCONFIRMED);
    unknown.destination = Some(addr(99, &[7]));
    unknown.source = Some(addr(2, &[7]));
    let sent = sends(&mut r, 2, &bytes(&unknown));
    assert_eq!(egress(&sent), [0, 3]);
    assert!(sent.iter().all(|s| s.kind == Kind::Solicit));
}

#[test]
fn congestion_is_only_announced_to_ports_that_can_use_the_route() {
    let mut r = configured(options(|o| o.congestion = Some(Congestion::DEFAULT)));
    for (queued, time) in [(256, Instant::ZERO), (256, at(2)), (0, at(3))] {
        r.egress_backlog(PortId(1), Backlog::new(queued, 256), time);
        let sent = drain(&mut r);
        if time != Instant::ZERO {
            assert_eq!(egress(&sent), [0, 3]);
            assert!(sent.iter().all(|s| listed(s) == [2]));
        }
    }
}

/// Only the searches triggered by a transit or local request for DNET 99.
fn search_ports(r: &mut TestRouter, from: Option<u8>, now: Instant) -> Vec<u8> {
    let sent = match from {
        Some(port) => {
            let frame = apdu_to(99, &[7], UNCONFIRMED);
            deliver(r, &mut Permissive, port, R7, &frame, now).1
        }
        None => {
            let request = Request::new(addr(99, &[7]), UNCONFIRMED);
            assert_eq!(
                r.originate(&request, now),
                Err(bacnet_router_core::router::OriginateError::NoRoute)
            );
            drain(r)
        }
    };
    sent.iter()
        .filter(|s| s.kind == Kind::Solicit)
        .map(|s| s.port)
        .collect()
}

#[test]
fn discovery_cooldown_searches_each_newly_reachable_port_once() {
    for (first, second, initial, additional) in [
        (Some(1), Some(0), vec![0, 3], vec![1, 2]),
        (Some(0), Some(1), vec![1, 2, 3], vec![0]),
        (Some(1), None, vec![0, 3], vec![1, 2]),
        (None, Some(1), vec![0, 1, 2, 3], vec![]),
        (Some(0), Some(3), vec![1, 2, 3], vec![0]),
        (Some(1), Some(2), vec![0, 3], vec![]),
    ] {
        let mut r = configured(Options::HOSTED);
        assert_eq!(search_ports(&mut r, first, at(1)), initial);
        assert_eq!(search_ports(&mut r, second, at(2)), additional);
        assert!(search_ports(&mut r, second, at(3)).is_empty());
        let last = if additional.is_empty() { 1 } else { 2 };
        assert_eq!(r.discovery().last_solicited(99), Some(at(last)));
        assert_eq!(search_ports(&mut r, first, at(last + 5)), initial);
    }
}

#[test]
fn an_isolated_request_with_no_egress_leaves_discovery_available_to_local_origin() {
    let mut ports = two_ports();
    for p in &mut ports {
        p.isolated = true;
    }
    let mut r = router_with(&ports, Options::HOSTED);
    assert!(search_ports(&mut r, Some(0), at(1)).is_empty());
    assert!(r.discovery().is_empty());
    assert_eq!(r.counters().solicitations, 0);
    assert_eq!(search_ports(&mut r, None, at(1)), [0, 1]);
}
