//! Initialize-Routing-Table and its Ack (router/control_messages.rs:448-616
//! and 734-786).

mod fixture;

use bacnet_router_core::message::{MessageType, RoutingEntries};
use bacnet_router_core::router::{ControlClass, Discard, Event, Kind, Options, SendRefusal};
use bacnet_router_core::{Mac, PortId};
use fixture::{
    at, deliver, drain_refusing, learn_9_via_r7, message, port_on, router, router_with, Outcome,
    Recorder, NODE_A, OWN_0, OWN_1, R7,
};

/// An Initialize-Routing-Table with these (network, Port ID) entries, each
/// with `info` octets of Port Info.
fn init_rt(entries: &[(u16, u8)], info: &[u8]) -> Vec<u8> {
    let mut data = vec![entries.len() as u8];
    for (network, port_id) in entries {
        data.extend_from_slice(&network.to_be_bytes());
        data.push(*port_id);
        data.push(info.len() as u8);
        data.extend_from_slice(info);
    }
    message(MessageType::INITIALIZE_ROUTING_TABLE, &data)
}

/// The (network, Port ID) entries of a sent Ack.
fn ack_entries(sent: &fixture::Sent) -> Vec<(u16, u8)> {
    let parsed = sent.parsed();
    assert_eq!(
        parsed.message_type,
        Some(MessageType::INITIALIZE_ROUTING_TABLE_ACK)
    );
    RoutingEntries::parse(parsed.payload)
        .unwrap()
        .iter()
        .map(|entry| (entry.network, entry.port_id))
        .collect()
}

/// Clauses 6.4.7, 6.6.3.8 and 6.6.3.9: an update applies its entries in
/// order, last wins, Port ID 0 deletes, and the sender gets an Ack with no
/// data at all. Reserved networks and unknown Port IDs are skipped, and
/// direct routes are never touched.
#[test]
fn updates_apply_in_order_and_ack_empty() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    let update = init_rt(
        &[
            (44, 2),
            (45, 1),
            (44, 0),
            (9, 0),
            (0, 1),
            (0xFFFF, 1),
            (46, 9),
            (1, 2),
            (47, 2),
        ],
        &[0xEE],
    );
    let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &update, at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(
        router.table().networks().collect::<Vec<_>>(),
        [1, 2, 45, 47]
    );
    let route = router.table().lookup(45).unwrap();
    assert_eq!(
        (route.port(), route.next_hop().as_slice(), route.last_seen()),
        (PortId(0), R7, Some(at(1)))
    );
    assert!(router.table().lookup(1).unwrap().is_direct());

    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (1, Some(R7)));
    assert_eq!(
        sent[0].kind,
        Kind::Answer(MessageType::INITIALIZE_ROUTING_TABLE_ACK)
    );
    assert_eq!(
        sent[0].npdu,
        message(MessageType::INITIALIZE_ROUTING_TABLE_ACK, &[])
    );
    assert_eq!(
        host.events,
        [
            Event::UpdateApplied {
                network: 44,
                port: PortId(1)
            },
            Event::UpdateApplied {
                network: 45,
                port: PortId(0)
            },
            Event::UpdateRemoved { network: 44 },
            Event::UpdateRemoved { network: 9 },
            Event::UnknownPortId {
                network: 46,
                port_id: 9
            },
            Event::UpdateApplied {
                network: 47,
                port: PortId(1)
            },
        ]
    );
    assert_eq!(host.asked[0].class, ControlClass::InitMgmt);
    // Management writes aren't learning: no learning counts.
    assert_eq!(router.table().claim_counters().learned_ok, 0);
}

/// Clauses 6.4.7 and 6.6.3.9: a query (count 0) changes nothing and gets
/// the whole table, ascending, in Acks of at most 124 entries so each fits
/// 501 octets, each network with its port's Port ID (index + 1).
#[test]
fn query_splits_into_bounded_acks() {
    let mut router = router();
    let mut host = Recorder::default();
    let peer = Mac::from_slice(R7).unwrap();
    for network in 100..230 {
        router
            .table_mut()
            .add_learned(network, PortId(1), peer, at(0));
    }
    let before: Vec<u16> = router.table().networks().collect();
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &init_rt(&[], &[]), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent.len(), 2);
    assert!(sent
        .iter()
        .all(|sent| sent.port == 0 && sent.to.as_deref() == Some(NODE_A)));
    let first = ack_entries(&sent[0]);
    let second = ack_entries(&sent[1]);
    assert_eq!((first.len(), second.len()), (124, 8));
    assert_eq!(sent[0].npdu.len(), 3 + 1 + 124 * 4);
    assert!(sent[0].npdu.len() <= 501);
    let all: Vec<(u16, u8)> = first.into_iter().chain(second).collect();
    let expected: Vec<(u16, u8)> = before
        .iter()
        .map(|&network| (network, if network == 1 { 1 } else { 2 }))
        .collect();
    assert_eq!(all, expected);
    assert_eq!(router.table().networks().collect::<Vec<_>>(), before);
    assert!(host.asked.is_empty(), "a query is never gated");
}

/// An empty table answers with one Ack of count 0, which differs from an
/// update's Ack with no data (init_routing_table_tests.rs:104).
#[test]
fn an_empty_table_answers_with_one_count_zero_ack() {
    let mut router = router_with(
        &[port_on(0, OWN_0), port_on(0xFFFF, OWN_1)],
        Options::HOSTED,
    );
    assert!(router.table().is_empty());
    let (_, sent) = deliver(
        &mut router,
        &mut Recorder::default(),
        0,
        NODE_A,
        &init_rt(&[], &[]),
        at(1),
    );
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].npdu,
        message(MessageType::INITIALIZE_ROUTING_TABLE_ACK, &[0])
    );
}

/// A refused Ack ends the answer, as a full queue does today
/// (control_messages.rs:515-522); the table doesn't change.
#[test]
fn a_refused_ack_ends_the_answer() {
    let mut router = router();
    let peer = Mac::from_slice(R7).unwrap();
    for network in 100..230 {
        router
            .table_mut()
            .add_learned(network, PortId(1), peer, at(0));
    }
    let query = init_rt(&[], &[]);
    let received = router.receive(
        bacnet_router_core::router::Frame {
            port: PortId(0),
            source: NODE_A,
            link_group: false,
            npdu: &query,
        },
        at(1),
        &mut Recorder::default(),
    );
    assert!(matches!(
        received,
        bacnet_router_core::router::Received::Handled
    ));
    let sent = drain_refusing(&mut router, |index, _| {
        (index == 0).then_some(SendRefusal::Full)
    });
    assert_eq!(sent.len(), 1);
    assert_eq!(router.counters().refused, 1);
    assert_eq!(router.table().len(), 132);
}

/// A malformed request, or a refused update, changes nothing and draws no
/// Ack; a malformed query isn't answered either.
#[test]
fn malformed_or_refused_updates_change_nothing() {
    let mut router = router();
    let mut host = Recorder {
        deny: vec![ControlClass::InitMgmt],
        ..Recorder::default()
    };
    let truncated = {
        let mut frame = init_rt(&[(44, 1)], &[]);
        frame.pop();
        frame
    };
    let trailing = {
        let mut frame = init_rt(&[], &[]);
        frame.push(0);
        frame
    };
    for bad in [truncated, trailing] {
        let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bad, at(1));
        assert_eq!(outcome, Outcome::Discarded(Discard::BadMessage));
        assert!(sent.is_empty());
    }
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &init_rt(&[(44, 1)], &[]),
        at(1),
    );
    assert_eq!(outcome, Outcome::Discarded(Discard::Denied));
    assert!(sent.is_empty());
    assert!(router.table().lookup(44).is_none());
}

/// Clause 6.6.3.9: an Ack's networks are learned through the arrival port
/// and its sender; its Port IDs, the sender's own, aren't used. An Ack with
/// no data answers an update and says nothing.
#[test]
fn ack_entries_are_learned_through_the_arrival_port() {
    let mut router = router();
    let mut host = Recorder::default();
    let mut data = vec![3];
    for (network, port_id) in [(44u16, 7u8), (0, 1), (45, 0)] {
        data.extend_from_slice(&network.to_be_bytes());
        data.extend_from_slice(&[port_id, 0]);
    }
    let ack = message(MessageType::INITIALIZE_ROUTING_TABLE_ACK, &data);
    let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &ack, at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert!(sent.is_empty());
    for network in [44, 45] {
        let route = router.table().lookup(network).unwrap();
        assert_eq!((route.port(), route.next_hop().as_slice()), (PortId(1), R7));
    }
    assert_eq!(router.table().len(), 4);
    assert_eq!(host.asked[0].targets, [44, 0, 45]);
    assert_eq!(router.table().claim_counters().learned_ok, 2);

    let empty = message(MessageType::INITIALIZE_ROUTING_TABLE_ACK, &[]);
    let (outcome, _) = deliver(&mut router, &mut host, 1, R7, &empty, at(2));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(host.asked.len(), 1, "a data-less Ack asks nothing");
}
