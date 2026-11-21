//! Routing-table queries that arrive from another network (#2699): the
//! answer goes back out of the arrival port toward the requester's SNET and
//! SADR (Clauses 6.5.4 and 6.6.3.8), in chunks the arrival port and a
//! later relay can carry (Clause 6.6.3.9), and the routed path changes no
//! table.

mod fixture;

use bacnet_router_core::PortId;
use bacnet_router_core::message::RoutingEntries;
use bacnet_router_core::npdu::{Address, Mac, NetworkMessageType, Npdu, NpduLimits};
use bacnet_router_core::router::{ControlClass, Discard, Groups, Options, PortConfig, SendRefusal};
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
/// delivered to. The table doesn't change and the host isn't asked.
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
    assert!(host.asked.is_empty(), "a query is never gated");
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
    // Named at the arrival port itself, the router answers as itself
    // there: a node on network 2 needs no SNET to reach it.
    let (_, sent) = deliver(
        &mut router,
        &mut Recorder::default(),
        1,
        R7,
        &query(REQUESTER, Some(addr(2, OWN_1))),
        at(2),
    );
    assert!(!sent.is_empty());
    for ack in &sent {
        assert_eq!(ack.parsed().destination, Some(REQUESTER));
    }
}

/// A long answer is split so that each Ack fits the arrival port's unicast
/// limit, here 128 octets, and still fits 501 octets once a relay adds an
/// SNET of the longest MAC. Every row comes once, ascending.
#[test]
fn routed_chunks_fit_the_arrival_ports_limit() {
    let small = PortConfig::new(
        2,
        Mac::from_slice(OWN_1).unwrap(),
        NpduLimits::both(128),
        Groups::None,
    );
    let mut router = router_with(&[port_on(1, OWN_0), small], Options::HOSTED);
    learn(&mut router, 100..160);
    let (_, sent) = deliver(
        &mut router,
        &mut Recorder::default(),
        1,
        R7,
        &query(REQUESTER, None),
        at(1),
    );
    assert!(sent.len() > 2, "{} Acks", sent.len());
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

/// Today an Initialize-Routing-Table update that arrives from another
/// network (SNET set) goes through the host's `authorize` exactly as a local
/// one does: refused, it changes nothing and draws nothing; allowed, it is
/// applied and acknowledged to the link peer. Pinned as it stands; the
/// default policy for remote updates is a separate decision.
#[test]
fn today_a_remote_table_update_is_authorized_like_a_local_one() {
    let update = bytes(&Npdu {
        source: Some(REQUESTER),
        ..Npdu::message(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            &[1, 0, 44, 1, 0],
        )
    });
    let mut router = router();
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
