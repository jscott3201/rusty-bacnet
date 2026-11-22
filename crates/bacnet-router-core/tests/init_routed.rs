//! Routing-table queries that arrive from another network (#2699): the
//! answer goes back out of the arrival port toward the requester's SNET and
//! SADR (Clauses 6.5.4 and 6.6.3.8), in chunks the arrival port and a
//! later relay can carry (Clause 6.6.3.9), and the routed path changes no
//! table.

mod fixture;

use bacnet_router_core::PortId;
use bacnet_router_core::message::RoutingEntries;
use bacnet_router_core::npdu::{Address, Mac, NetworkMessageType, Npdu, NpduLimits};
use bacnet_router_core::router::{
    ControlClass, Discard, Event, Groups, Options, PortConfig, SendRefusal,
};
use fixture::{
    OWN_0, OWN_1, Outcome, R7, Recorder, Sent, addr, at, bytes, deliver, drain, drain_refusing,
    port_on, router, router_with,
};

/// A requester on network 81, behind R7.
const REQUESTER: Address<'static> = Address {
    network: 81,
    mac: &[0x01],
};

/// A routing-table query (a port count of zero) from `source`, sent to
/// `destination` when given.
fn query(source: Address<'_>, destination: Option<Address<'_>>) -> Vec<u8> {
    bytes(&Npdu {
        source: Some(source),
        destination,
        ..Npdu::message(NetworkMessageType::INITIALIZE_ROUTING_TABLE, &[0])
    })
}

/// Learn networks `networks` through R7 on port 1.
fn learn(router: &mut fixture::TestRouter, networks: core::ops::Range<u16>) {
    let r7 = Mac::from_slice(R7).unwrap();
    for network in networks {
        router
            .table_mut()
            .add_learned(network, PortId(1), r7, at(0));
    }
}

/// The networks an Ack lists.
fn rows(sent: &Sent) -> Vec<u16> {
    let npdu = sent.parsed();
    assert_eq!(
        npdu.message_type,
        Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK)
    );
    RoutingEntries::parse(npdu.payload)
        .unwrap()
        .iter()
        .map(|entry| entry.network)
        .collect()
}

/// Every Ack in `sent` goes back out of port 1 to R7, the link peer that
/// relayed the query, toward the requester with a fresh hop count, and
/// carries `source` as its SNET and SADR.
fn assert_routed_answer(sent: &[Sent], requester: Address<'_>, source: Option<Address<'_>>) {
    assert!(!sent.is_empty());
    for ack in sent {
        assert_eq!((ack.port, ack.to.as_deref()), (1, Some(R7)));
        let npdu = ack.parsed();
        assert_eq!(npdu.destination, Some(requester));
        assert_eq!(npdu.hop_count, 255);
        assert_eq!(npdu.source, source);
    }
}

/// A query relayed by R7 with the requester's SNET and SADR is answered out
/// of the arrival port, unicast to R7, with the requester as DNET and DADR,
/// hop count 255 and no SNET: the router answers as the station R7
/// delivered to. The table doesn't change. The host is asked once, about a
/// query from another network (#2715), and told what the query claims.
#[test]
fn a_relayed_query_is_answered_toward_its_snet_out_of_the_arrival_port() {
    let mut router = router();
    let mut host = Recorder::default();
    let before: Vec<u16> = router.table().networks().collect();
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        1,
        R7,
        &query(REQUESTER, None),
        at(1),
    );
    assert_eq!(outcome, Outcome::Handled);
    assert_routed_answer(&sent, REQUESTER, None);
    assert_eq!(sent.iter().flat_map(rows).collect::<Vec<_>>(), before);
    assert_eq!(router.table().networks().collect::<Vec<_>>(), before);
    let [asked] = &host.asked[..] else {
        panic!("{:?}", host.asked)
    };
    assert_eq!(
        (asked.class, asked.snet, asked.dnet, asked.port),
        (ControlClass::InitQuery, Some(81), None, PortId(1))
    );
    assert_eq!(asked.source, R7, "the link peer, not the SADR");
    assert_eq!(
        asked.message_type,
        NetworkMessageType::INITIALIZE_ROUTING_TABLE
    );
    assert!(asked.targets.is_empty() && !asked.targets_omitted);
    assert_eq!(router.counters().denied, 0);
}

/// A host that refuses the class draws no Ack for a query from another
/// network, whether it names no address, the router at the arrival port or
/// the router at another port; the query is counted as denied and changes
/// nothing. The same host still gets a local query (no SNET) answered, and
/// is not asked about it: only a query whose answer crosses networks is
/// the host's to refuse (#2715).
#[test]
fn a_host_that_refuses_remote_queries_draws_no_ack_and_local_ones_are_unaffected() {
    let mut router = router();
    learn(&mut router, 100..104);
    let before: Vec<u16> = router.table().networks().collect();
    let mut host = Recorder {
        deny: vec![ControlClass::InitQuery],
        ..Recorder::default()
    };
    let named = [None, Some(addr(2, OWN_1)), Some(addr(1, OWN_0))];
    for (index, destination) in named.into_iter().enumerate() {
        let frame = query(REQUESTER, destination);
        let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &frame, at(1));
        assert_eq!(
            outcome,
            Outcome::Discarded(Discard::Denied),
            "{destination:?}"
        );
        assert!(sent.is_empty(), "{destination:?}: {sent:?}");
        assert_eq!(host.asked.len(), index + 1);
        assert_eq!(host.asked[index].class, ControlClass::InitQuery);
        assert_eq!(host.asked[index].dnet, destination.map(|to| to.network));
    }
    assert_eq!(router.counters().denied, 3);
    assert_eq!(router.table().networks().collect::<Vec<_>>(), before);

    // Local: from R7 itself, with no SNET, and the same naming the
    // router's address on its other port.
    let plain = Npdu::message(NetworkMessageType::INITIALIZE_ROUTING_TABLE, &[0]);
    let naming = Npdu {
        destination: Some(addr(1, OWN_0)),
        ..plain
    };
    for (frame, source) in [(plain, None), (naming, Some(addr(1, OWN_0)))] {
        let frame = bytes(&frame);
        let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &frame, at(2));
        assert_eq!(outcome, Outcome::Handled);
        let [ack] = &sent[..] else { panic!("{sent:?}") };
        assert_eq!((ack.port, ack.to.as_deref()), (1, Some(R7)));
        assert_eq!(rows(ack), before);
        assert_eq!(
            (ack.parsed().destination, ack.parsed().source),
            (None, source)
        );
    }
    assert_eq!(host.asked.len(), 3, "a local query is never asked about");
    assert_eq!(router.counters().denied, 3);
}

/// A query from another network with bad data is refused as malformed
/// before the host hears of it: the data is checked first.
#[test]
fn the_host_is_not_asked_about_a_remote_query_with_bad_data() {
    let mut router = router();
    let mut host = Recorder::default();
    let frame = bytes(&Npdu {
        source: Some(REQUESTER),
        ..Npdu::message(NetworkMessageType::INITIALIZE_ROUTING_TABLE, &[0, 9])
    });
    let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &frame, at(1));
    assert_eq!(outcome, Outcome::Discarded(Discard::BadMessage));
    assert!(sent.is_empty() && host.asked.is_empty());
}

/// A query that names the router's address on its other port (DNET 1, DADR
/// [`OWN_0`]), arriving on port 1, is answered with that address as SNET
/// and SADR: the task keeps the port's index and the writer rebuilds the
/// address from the port's configuration.
#[test]
fn a_query_naming_the_router_at_another_port_is_answered_from_that_address() {
    let mut router = router();
    let named = addr(1, OWN_0);
    let (outcome, sent) = deliver(
        &mut router,
        &mut Recorder::default(),
        1,
        R7,
        &query(REQUESTER, Some(named)),
        at(1),
    );
    assert_eq!(outcome, Outcome::Handled);
    assert_routed_answer(&sent, REQUESTER, Some(named));
    // Named at the arrival port itself, the answer carries no SNET, as the
    // answer to a query that names no address does: the relay that carries
    // it on adds the arrival network and the router's MAC there itself.
    let own = addr(2, OWN_1);
    let (_, sent) = deliver(
        &mut router,
        &mut Recorder::default(),
        1,
        R7,
        &query(REQUESTER, Some(own)),
        at(2),
    );
    assert_routed_answer(&sent, REQUESTER, None);
}

/// A node on the arrival network that names the router by that network's
/// number and its MAC there (no SNET in the query) gets what a query with
/// no DNET gets: no DNET and no SNET. Naming the router's address on its
/// other port, it gets that address as SNET and SADR.
#[test]
fn a_local_query_naming_the_arrival_port_is_answered_without_addresses() {
    let mut router = router();
    for (named, source) in [
        (addr(2, OWN_1), None),
        (addr(1, OWN_0), Some(addr(1, OWN_0))),
    ] {
        let frame = bytes(&Npdu {
            destination: Some(named),
            ..Npdu::message(NetworkMessageType::INITIALIZE_ROUTING_TABLE, &[0])
        });
        let (outcome, sent) = deliver(&mut router, &mut Recorder::default(), 1, R7, &frame, at(1));
        assert_eq!(outcome, Outcome::Handled);
        let [ack] = &sent[..] else { panic!("{sent:?}") };
        assert_eq!((ack.port, ack.to.as_deref()), (1, Some(R7)));
        assert_eq!(rows(ack), [1, 2]);
        let npdu = ack.parsed();
        assert_eq!((npdu.destination, npdu.source), (None, source));
    }
}

/// A router whose port 1 carries 128 octets, holding 62 routes: its two
/// ports and networks 100 to 159 through R7.
fn router_with_a_128_octet_port() -> fixture::TestRouter {
    let small = PortConfig::new(
        2,
        Mac::from_slice(OWN_1).unwrap(),
        NpduLimits::both(128),
        Groups::None,
    );
    let mut router = router_with(&[port_on(1, OWN_0), small], Options::HOSTED);
    learn(&mut router, 100..160);
    router
}

/// A long answer is split so that each Ack fits the arrival port's unicast
/// limit, here 128 octets, and still fits 501 octets once a relay adds an
/// SNET of the longest MAC. Every row comes once, ascending.
#[test]
fn routed_chunks_fit_the_arrival_ports_limit() {
    let mut router = router_with_a_128_octet_port();
    let (_, sent) = deliver(
        &mut router,
        &mut Recorder::default(),
        1,
        R7,
        &query(REQUESTER, None),
        at(1),
    );
    // An 8-octet header and the count octet leave 119 of the 128 octets:
    // 29 four-octet rows.
    let counts: Vec<usize> = sent.iter().map(|ack| rows(ack).len()).collect();
    assert_eq!(counts, [29, 29, 4]);
    assert_routed_answer(&sent, REQUESTER, None);
    for ack in &sent {
        assert!(ack.npdu.len() <= 128, "{} octets", ack.npdu.len());
        let relayed = Npdu {
            source: Some(addr(5, &[0x2A; bacnet_router_core::npdu::MAX_MAC_LEN])),
            ..ack.parsed()
        };
        assert!(bytes(&relayed).len() <= 501);
    }
    let all: Vec<u16> = sent.iter().flat_map(rows).collect();
    assert_eq!(all.len(), 62);
    assert!(all.windows(2).all(|pair| pair[0] < pair[1]));
}

/// A long answer to a query that named the router's address on another
/// port carries that address as SNET and SADR on every Ack, and each Ack
/// is sized with it: the 6-octet SADR and its three header octets take
/// the room of two rows from the 128-octet port, and no room is kept back
/// for a relay's SNET, since a relay leaves an SNET that is there alone
/// (Clause 6.5.4). Named at the arrival port, the Acks carry no SNET and are
/// sized as for a query that names none.
#[test]
fn a_named_answer_in_chunks_carries_its_source_on_every_ack() {
    for (named, source, expected) in [
        (addr(1, OWN_0), Some(addr(1, OWN_0)), [27, 27, 8]),
        (addr(2, OWN_1), None, [29, 29, 4]),
    ] {
        let mut router = router_with_a_128_octet_port();
        let (_, sent) = deliver(
            &mut router,
            &mut Recorder::default(),
            1,
            R7,
            &query(REQUESTER, Some(named)),
            at(1),
        );
        let counts: Vec<usize> = sent.iter().map(|ack| rows(ack).len()).collect();
        assert_eq!(counts, expected, "{named:?}");
        assert_routed_answer(&sent, REQUESTER, source);
        // The full Acks are as long as the port allows less a part row.
        let lengths: Vec<usize> = sent.iter().map(|ack| ack.npdu.len()).collect();
        let full = if source.is_some() { 126 } else { 125 };
        assert_eq!(lengths[..2], [full, full], "{named:?}");
        let all: Vec<u16> = sent.iter().flat_map(rows).collect();
        assert_eq!(all.len(), 62);
        assert!(all.windows(2).all(|pair| pair[0] < pair[1]));
    }
}

/// Two requesters behind the same relay each get the whole table, each to
/// its own SNET and SADR.
#[test]
fn two_requesters_each_get_the_whole_table() {
    let mut router = router();
    learn(&mut router, 100..300);
    let other = addr(82, &[0x02, 0x03]);
    let mut host = Recorder::default();
    let (_, first) = deliver(
        &mut router,
        &mut host,
        1,
        R7,
        &query(REQUESTER, None),
        at(1),
    );
    let (_, second) = deliver(&mut router, &mut host, 1, R7, &query(other, None), at(2));
    assert_routed_answer(&first, REQUESTER, None);
    assert_routed_answer(&second, other, None);
    let rows_of = |sent: &[Sent]| sent.iter().flat_map(rows).count();
    assert_eq!((rows_of(&first), rows_of(&second)), (202, 202));
}

/// A full send queue ends a routed answer at the refused Ack: nothing
/// follows it, the router goes idle and counts one refusal.
#[test]
fn a_full_queue_ends_a_routed_answer() {
    let mut router = router();
    learn(&mut router, 100..400);
    let frame = query(REQUESTER, None);
    let received = router.receive(
        bacnet_router_core::router::Frame {
            port: PortId(1),
            source: R7,
            link_group: false,
            npdu: &frame,
        },
        at(1),
        &mut Recorder::default(),
    );
    assert!(matches!(
        received,
        bacnet_router_core::router::Received::Handled
    ));
    let sent = drain_refusing(&mut router, |index, _| {
        (index == 1).then_some(SendRefusal::Full)
    });
    assert_eq!(sent.len(), 2, "the answer ends at the refused Ack");
    assert_eq!(router.counters().refused, 1);
    assert!(drain(&mut router).is_empty());
}

/// An Initialize-Routing-Table update that carries an SNET.
fn remote_update(data: &[u8], destination: Option<Address<'_>>) -> Vec<u8> {
    bytes(&Npdu {
        source: Some(REQUESTER),
        destination,
        ..Npdu::message(NetworkMessageType::INITIALIZE_ROUTING_TABLE, data)
    })
}

/// By default an Initialize-Routing-Table update that arrives from another
/// network (SNET set) is dropped before the host is asked (#2717): it
/// changes nothing and draws no Ack, whether it would add a route or
/// remove one, and whether or not it names the router at the arrival port.
/// It is counted as denied and reported. The same update with no SNET is
/// applied and acknowledged as it always was. Both sets of options start
/// with the default.
#[test]
fn a_table_update_from_another_network_is_refused_by_default() {
    for options in [Options::HOSTED, Options::STANDARD] {
        assert!(!options.remote_table_updates);
        let mut router = router_with(&fixture::two_ports(), options);
        learn(&mut router, 100..101);
        let mut host = Recorder::default();
        let add: &[u8] = &[1, 0, 44, 1, 0];
        let remove: &[u8] = &[1, 0, 100, 0, 0];
        let named = Some(addr(2, OWN_1));
        for (data, destination) in [(add, None), (remove, None), (add, named)] {
            let frame = remote_update(data, destination);
            let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &frame, at(1));
            assert_eq!(outcome, Outcome::Discarded(Discard::Denied));
            assert!(sent.is_empty(), "{sent:?}");
        }
        assert!(router.table().lookup(44).is_none());
        assert!(router.table().lookup(100).is_some());
        assert!(host.asked.is_empty(), "the host is not asked");
        assert_eq!(router.counters().denied, 3);
        let refused = Event::RemoteUpdateRefused {
            port: PortId(1),
            snet: 81,
        };
        assert_eq!(host.events, [refused; 3]);

        // Local: the same data with no SNET.
        let local = fixture::message(NetworkMessageType::INITIALIZE_ROUTING_TABLE, add);
        let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &local, at(2));
        assert_eq!(outcome, Outcome::Handled);
        assert!(router.table().lookup(44).is_some());
        assert_eq!(sent.len(), 1, "the data-less Ack");
        let [asked] = &host.asked[..] else {
            panic!("{:?}", host.asked)
        };
        assert_eq!((asked.class, asked.snet), (ControlClass::InitMgmt, None));
        assert_eq!(router.counters().denied, 3);
    }
}

/// With [`Options::remote_table_updates`] on, an update from another
/// network goes through the host's `authorize` exactly as a local one
/// does, as every update did before #2717: refused, it changes nothing and
/// draws nothing; allowed, it is applied and acknowledged to the link peer.
#[test]
fn with_the_option_a_remote_table_update_is_authorized_like_a_local_one() {
    let update = remote_update(&[1, 0, 44, 1, 0], None);
    let on = fixture::options(|options| options.remote_table_updates = true);
    let mut router = router_with(&fixture::two_ports(), on);
    let mut denying = Recorder {
        deny: vec![ControlClass::InitMgmt],
        ..Recorder::default()
    };
    let (outcome, sent) = deliver(&mut router, &mut denying, 1, R7, &update, at(1));
    assert_eq!(outcome, Outcome::Discarded(Discard::Denied));
    assert!(sent.is_empty());
    assert!(router.table().lookup(44).is_none());
    let [asked] = &denying.asked[..] else {
        panic!("{:?}", denying.asked)
    };
    assert_eq!(
        (asked.class, asked.snet, asked.port),
        (ControlClass::InitMgmt, Some(81), PortId(1))
    );

    let mut allowing = Recorder::default();
    let (outcome, sent) = deliver(&mut router, &mut allowing, 1, R7, &update, at(2));
    assert_eq!(outcome, Outcome::Handled);
    assert!(router.table().lookup(44).is_some());
    assert_eq!(allowing.asked.len(), 1);
    let [ack] = &sent[..] else { panic!("{sent:?}") };
    assert_eq!((ack.port, ack.to.as_deref()), (1, Some(R7)));
    assert_eq!(
        ack.parsed().message_type,
        Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK)
    );
    // Unchanged from a local update: the data-less Ack goes to the link peer
    // with no DNET.
    assert_eq!(ack.parsed().destination, None);
}
